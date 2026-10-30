//! Composition + context: hew.group.*, hew.component.*, hew.context.*
//! (docs/agents/HEW_API.md §7, §6.3).
//!
//! Context commands are legal only inside a transaction, context-balanced
//! (§6.3) — `transact.rs` already enforces that statically for every
//! envelope (a bare plain request auto-wraps as a one-command transaction,
//! which then fails the balance check). These handlers just open/close
//! the frame the resolved id names.

use super::entity::{resolve_node, unknown_entity};
use super::{CmdError, Ctx, Handler};
use crate::refusal::Refusal;
use kernel::{EntityRef, NodeId, Transform};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.group.create" => group_create,
        "hew.group.explode" => group_explode,
        "hew.group.reparent" => group_reparent,
        "hew.component.create" => component_create,
        "hew.component.place" => component_place,
        "hew.component.make_unique" => component_make_unique,
        "hew.component.explode" => component_explode,
        "hew.component.rebase" => component_rebase,
        "hew.component.flip_instance" => component_flip_instance,
        "hew.context.enter" => context_enter,
        "hew.context.exit" => context_exit,
        _ => return None,
    })
}

// ----------------------------------------------------------------- group

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MembersParams {
    members: Vec<String>,
}

fn group_create(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MembersParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let nodes: Vec<NodeId> = p
        .members
        .iter()
        .map(|id| resolve_node(ctx, id))
        .collect::<Result<_, _>>()?;
    let (group, _) = ctx.doc.group_nodes(&nodes)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Group(group))
        .expect("just created");
    Ok(serde_json::json!({ "group": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: String,
}

fn group_explode(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: IdParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let Some(EntityRef::Group(group)) = ctx.resolver().resolve(&p.id) else {
        return Err(unknown_entity(&p.id));
    };
    ctx.doc.ungroup(group)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReparentParams {
    ids: Vec<String>,
    /// The target group, or `null`/omitted for the top level.
    #[serde(default)]
    parent: Option<String>,
}

/// Moves live world nodes into `parent` (a live group) or out to the top
/// level (`parent: null`), as ONE undo entry — [`kernel::Document::
/// reparent_nodes`], the same op the Outliner's drag-and-drop calls
/// through the wasm boundary. Geometry is untouched: a group is a
/// pose-less container, so this is pure tree bookkeeping. A node already
/// under `parent` is silently skipped.
///
/// # Errors
/// - `empty_ids` — `ids` is empty. The kernel itself has no empty-list
///   guard here (an empty move is a harmless no-op to it), but a silent
///   no-op success would diverge from every sibling entry point for this
///   op: the wasm boundary's `node_ids` refuses an empty list typed
///   (`BadNodeList`), and `hew.group.create` refuses typed
///   (`empty_group`) for the same reason. Named distinctly from the
///   kernel's own `empty_selection` (`DocumentError::EmptySelection`,
///   used by the transform commands) — that one means "everything
///   picked is hidden or gone," a different condition from "nothing was
///   picked at all."
fn group_reparent(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ReparentParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if p.ids.is_empty() {
        return Err(CmdError::Refusal(Refusal::api(
            "empty_ids",
            "Select at least one object, group, or component instance to move.",
        )));
    }
    let nodes: Vec<NodeId> = p
        .ids
        .iter()
        .map(|id| resolve_node(ctx, id))
        .collect::<Result<_, _>>()?;
    let parent = match &p.parent {
        Some(id) => match ctx.resolver().resolve(id) {
            Some(EntityRef::Group(g)) => Some(g),
            _ => return Err(unknown_entity(id)),
        },
        None => None,
    };
    ctx.doc.reparent_nodes(&nodes, parent)?;
    Ok(serde_json::json!({}))
}

// ------------------------------------------------------------- component

fn component_create(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MembersParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let nodes: Vec<NodeId> = p
        .members
        .iter()
        .map(|id| resolve_node(ctx, id))
        .collect::<Result<_, _>>()?;
    let (component, instance, _) = ctx.doc.make_component(&nodes)?;
    let resolver = ctx.resolver();
    let component_public = resolver
        .public_of(ctx.doc, &EntityRef::Component(component))
        .expect("just created");
    let instance_public = resolver
        .public_of(ctx.doc, &EntityRef::Instance(instance))
        .expect("just created");
    Ok(serde_json::json!({ "component": component_public, "instance": instance_public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceParams {
    component: String,
    #[serde(default)]
    pose: Option<Value>,
}

fn parse_pose(pose: &Option<Value>) -> Result<Transform, CmdError> {
    let Some(pose) = pose else {
        return Ok(Transform::IDENTITY);
    };
    if let Some(arr) = pose.as_array() {
        if arr.len() != 12 {
            return Err(CmdError::Params(
                "pose array must hold 12 row-major affine numbers".into(),
            ));
        }
        let mut rows = [0.0_f64; 12];
        for (i, v) in arr.iter().enumerate() {
            rows[i] = v
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| CmdError::Params("pose components must be finite numbers".into()))?;
        }
        return Ok(Transform::from_affine(&rows));
    }
    if let Some(obj) = pose.as_object()
        && let Some(t) = obj.get("translation")
    {
        let p = crate::locate::parse_xyz(t)?;
        return Ok(Transform::translation(p.to_vec()));
    }
    Err(CmdError::Params(
        "pose must be a 12-number affine array or {\"translation\": [x,y,z]}".into(),
    ))
}

fn component_place(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: PlaceParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let Some(EntityRef::Component(component)) = ctx.resolver().resolve(&p.component) else {
        return Err(unknown_entity(&p.component));
    };
    let pose = parse_pose(&p.pose)?;
    let (instance, _) = ctx.doc.place_instance(component, pose)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Instance(instance))
        .expect("just created");
    Ok(serde_json::json!({ "instance": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceParams {
    instance: String,
}

fn component_make_unique(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: InstanceParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let Some(EntityRef::Instance(instance)) = ctx.resolver().resolve(&p.instance) else {
        return Err(unknown_entity(&p.instance));
    };
    let (component, _) = ctx.doc.make_unique(instance)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Component(component))
        .expect("just created");
    Ok(serde_json::json!({ "component": public }))
}

fn component_explode(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: InstanceParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let Some(EntityRef::Instance(instance)) = ctx.resolver().resolve(&p.instance) else {
        return Err(unknown_entity(&p.instance));
    };
    let (objects, _) = ctx.doc.explode_instance(instance)?;
    let resolver = ctx.resolver();
    let public: Vec<String> = objects
        .iter()
        .map(|&o| {
            resolver
                .public_of(ctx.doc, &EntityRef::Object(o))
                .expect("just created")
        })
        .collect();
    Ok(serde_json::json!({ "objects": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RebaseParams {
    id: String,
    origin: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
}

/// `hew.component.rebase` (docs/agents/PLUGINS.md "Setting axes"):
/// `Document::rebase_definition`, with an instance id meaning its
/// definition. `instances_updated` counts every live instance placing the
/// definition, world or nested in another definition.
fn component_rebase(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: RebaseParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let component = match ctx.resolver().resolve(&p.id) {
        Some(EntityRef::Component(c)) => c,
        Some(EntityRef::Instance(i)) => ctx
            .doc
            .instance_def(i)
            .ok_or_else(|| unknown_entity(&p.id))?,
        _ => return Err(unknown_entity(&p.id)),
    };
    let v = |c: [f64; 3]| kernel::Vec3::new(c[0], c[1], c[2]);
    let origin = kernel::Point3::new(p.origin[0], p.origin[1], p.origin[2]);
    ctx.doc
        .rebase_definition(component, origin, v(p.x), v(p.y))?;
    let instances_updated = ctx.doc.instances_of(component).len();
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Component(component))
        .ok_or_else(|| CmdError::Internal("a rebased definition has no public id".into()))?;
    Ok(serde_json::json!({ "component": public, "instances_updated": instances_updated }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FlipParams {
    instance: String,
    axis: FlipAxis,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum FlipAxis {
    X,
    Y,
    Z,
}

/// `hew.component.flip_instance`: `Document::flip_instance` on one world
/// instance. A definition-owned (nested) instance refuses `unknown_entity`,
/// as every world-only instance command does.
fn component_flip_instance(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: FlipParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let Some(EntityRef::Instance(instance)) = ctx.resolver().resolve(&p.instance) else {
        return Err(unknown_entity(&p.instance));
    };
    if !ctx.doc.instance_is_world(instance) {
        return Err(unknown_entity(&p.instance));
    }
    let axis = match p.axis {
        FlipAxis::X => kernel::Axis::X,
        FlipAxis::Y => kernel::Axis::Y,
        FlipAxis::Z => kernel::Axis::Z,
    };
    ctx.doc.flip_instance(instance, axis)?;
    let pose = ctx
        .doc
        .instance_pose(instance)
        .ok_or_else(|| CmdError::Internal("a flipped instance has no pose".into()))?;
    Ok(serde_json::json!({
        "instance": p.instance,
        "pose": pose.to_affine(),
        "mirrored": pose.determinant() < 0.0,
    }))
}

// --------------------------------------------------------------- context

fn context_enter(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: IdParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    match ctx.resolver().resolve(&p.id) {
        Some(EntityRef::Group(g)) => {
            ctx.doc.open_group_session(g)?;
        }
        Some(EntityRef::Instance(i)) => {
            ctx.doc.open_explode_session(i)?;
        }
        _ => return Err(unknown_entity(&p.id)),
    }
    Ok(serde_json::json!({}))
}

fn context_exit(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExitParams {}
    let _: ExitParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    ctx.doc.close_innermost_session()?;
    Ok(serde_json::json!({}))
}
