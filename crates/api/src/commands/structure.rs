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
use kernel::{EntityRef, NodeId, Transform};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.group.create" => group_create,
        "hew.group.explode" => group_explode,
        "hew.component.create" => component_create,
        "hew.component.place" => component_place,
        "hew.component.make_unique" => component_make_unique,
        "hew.component.explode" => component_explode,
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
