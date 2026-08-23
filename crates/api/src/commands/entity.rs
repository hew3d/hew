//! Entity lifecycle + transforms: hew.entity.* (docs/agents/HEW_API.md §7).
//!
//! Rename/delete dispatch on the resolved [`kernel::EntityRef`]'s kind;
//! the transform trio (move/rotate/scale) share the pivot/anchor
//! conjugation pattern documented on [`kernel::Transform::then`]: `self`
//! applies first, `second` after, so "rotate about a pivot" is
//! `translate(-pivot).then(rotate).then(translate(pivot))`.

use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{EntityRef, NodeId, SketchId, Transform, Vec3};
use serde_json::Value;
use std::num::NonZeroU32;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.entity.rename" => rename,
        "hew.entity.delete" => delete,
        "hew.entity.move" => move_,
        "hew.entity.rotate" => rotate,
        "hew.entity.scale" => scale,
        _ => return None,
    })
}

/// A public id that does not resolve to a live entity of the required
/// kind (mirrors `locate::unknown_entity`, which is private to that
/// module).
pub(crate) fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(serde_json::json!({ "id": id })),
    )
}

/// Resolves a public id to a tree [`NodeId`] (Object/Group/Instance) —
/// the addressable kind every structural/transform command works over.
pub(crate) fn resolve_node(ctx: &Ctx, id: &str) -> Result<NodeId, CmdError> {
    match ctx.resolver().resolve(id) {
        Some(EntityRef::Object(o)) => Ok(NodeId::Object(o)),
        Some(EntityRef::Group(g)) => Ok(NodeId::Group(g)),
        Some(EntityRef::Instance(i)) => Ok(NodeId::Instance(i)),
        _ => Err(unknown_entity(id)),
    }
}

// ---------------------------------------------------------------- rename

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameParams {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

fn rename(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: RenameParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let entity = ctx
        .resolver()
        .resolve(&p.id)
        .ok_or_else(|| unknown_entity(&p.id))?;
    match entity {
        EntityRef::Object(o) => ctx.doc.set_node_name(NodeId::Object(o), p.name)?,
        EntityRef::Group(g) => ctx.doc.set_node_name(NodeId::Group(g), p.name)?,
        EntityRef::Instance(i) => ctx.doc.set_node_name(NodeId::Instance(i), p.name)?,
        EntityRef::Component(c) => ctx.doc.set_component_name(c, p.name)?,
        EntityRef::Sketch(_) | EntityRef::Material(_) | EntityRef::Guide(_) | EntityRef::Tag(_) => {
            return Err(CmdError::Refusal(Refusal::api(
                "rename_unsupported",
                "Only objects, groups, instances, and component definitions can be renamed today.",
            )));
        }
    };
    Ok(serde_json::json!({}))
}

// ---------------------------------------------------------------- delete

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    id: String,
}

fn sketch_err(e: kernel::SketchError) -> CmdError {
    CmdError::from(kernel::DocumentError::Sketch(e))
}

fn delete(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: DeleteParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let resolver = ctx.resolver();
    let entity = resolver.resolve(&p.id);
    // A sketch edge is a compound id (not a top-level `EntityRef` kind —
    // docs/agents/HEW_API.md §5.2), resolved the same way a region or curve id
    // is: `IdResolver::resolve_edge`, tried only once the ordinary
    // resolve comes up empty.
    let edge = entity
        .is_none()
        .then(|| resolver.resolve_edge(&p.id))
        .flatten();
    let entity = match (entity, edge) {
        (Some(e), _) => e,
        (None, Some((sketch, edge))) => {
            // No silent repair (DEVELOPMENT.md rule 4): a stale edge
            // handle surfaces the kernel's own typed `SketchError`
            // (`unknown_edge`) rather than being treated as already-gone.
            // Orphaned vertices and the regions this edge closed or
            // opened are exactly what `Sketch::remove_edge` itself
            // decides — the API neither second-guesses nor patches that.
            super::run_sketch_gesture(ctx, sketch, |s| s.remove_edge(edge).map_err(sketch_err))?;
            return Ok(serde_json::json!({}));
        }
        (None, None) => return Err(unknown_entity(&p.id)),
    };
    match entity {
        EntityRef::Object(o) => ctx.doc.delete_node(NodeId::Object(o))?,
        EntityRef::Group(g) => ctx.doc.delete_node(NodeId::Group(g))?,
        EntityRef::Instance(i) => ctx.doc.delete_node(NodeId::Instance(i))?,
        EntityRef::Sketch(s) => ctx.doc.delete_sketch(s)?,
        EntityRef::Guide(g) => ctx.doc.delete_guide(g)?,
        EntityRef::Tag(path) => ctx.doc.delete_tag(&path)?,
        EntityRef::Material(_) => {
            return Err(CmdError::Refusal(Refusal::api(
                "delete_unsupported",
                "Materials cannot be deleted from the palette today — the kernel has no material delete.",
            )));
        }
        EntityRef::Component(_) => {
            return Err(CmdError::Refusal(Refusal::api(
                "delete_unsupported",
                "A component definition cannot be deleted directly — it dies with its last instance. Delete or explode its instances instead.",
            )));
        }
    };
    Ok(serde_json::json!({}))
}

// ------------------------------------------------------------------ move

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CopySpec {
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveParams {
    ids: Vec<String>,
    #[serde(default)]
    translation: Option<[f64; 3]>,
    #[serde(default)]
    from: Option<Value>,
    #[serde(default)]
    to: Option<Value>,
    #[serde(default)]
    copy: Option<CopySpec>,
}

/// Splits resolved ids into their `NodeId` and `SketchId` targets,
/// refusing typed on the first id that names neither.
fn split_ids(ctx: &Ctx, ids: &[String]) -> Result<(Vec<NodeId>, Vec<SketchId>), CmdError> {
    let mut nodes = Vec::new();
    let mut sketches = Vec::new();
    for id in ids {
        match ctx.resolver().resolve(id) {
            Some(EntityRef::Object(o)) => nodes.push(NodeId::Object(o)),
            Some(EntityRef::Group(g)) => nodes.push(NodeId::Group(g)),
            Some(EntityRef::Instance(i)) => nodes.push(NodeId::Instance(i)),
            Some(EntityRef::Sketch(s)) => sketches.push(s),
            _ => return Err(unknown_entity(id)),
        }
    }
    Ok((nodes, sketches))
}

fn mixed_selection_refusal() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "mixed_selection_unsupported",
        "This transform's one-envelope-one-undo accounting requires a single sketch OR one or more non-sketch entities, never a mix — and only one sketch at a time. Move them in separate commands.",
    ))
}

fn sketch_copy_refusal() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "sketch_copy_unsupported",
        "Copying a sketch selection is not supported at 1.0: the UI's sketch copy is tool-layer replay through the sticky rules, and the kernel-side sketch duplicate op does not exist yet.",
    ))
}

fn finite_translation(v: [f64; 3]) -> Result<Vec3, CmdError> {
    if v.iter().any(|c| !c.is_finite()) {
        return Err(CmdError::Params(
            "translation components must be finite".into(),
        ));
    }
    Ok(Vec3::new(v[0], v[1], v[2]))
}

fn move_(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MoveParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if p.ids.is_empty() {
        return Err(kernel::DocumentError::EmptySelection.into());
    }
    let has_translation = p.translation.is_some();
    let has_from_to = p.from.is_some() || p.to.is_some();
    if has_translation == has_from_to {
        return Err(CmdError::Params(
            "move needs exactly one of \"translation\" or \"from\"+\"to\"".into(),
        ));
    }
    let offset =
        if let Some(t) = p.translation {
            finite_translation(t)?
        } else {
            let from = p.from.as_ref().ok_or_else(|| {
                CmdError::Params("move's \"to\" needs a matching \"from\"".into())
            })?;
            let to = p.to.as_ref().ok_or_else(|| {
                CmdError::Params("move's \"from\" needs a matching \"to\"".into())
            })?;
            let from = locate::resolve_point(ctx, from)?;
            let to = locate::resolve_point(ctx, to)?;
            to - from
        };
    let t = Transform::translation(offset);

    let (nodes, sketches) = split_ids(ctx, &p.ids)?;
    if !nodes.is_empty() && !sketches.is_empty() {
        return Err(mixed_selection_refusal());
    }
    if !sketches.is_empty() {
        if p.copy.is_some() {
            return Err(sketch_copy_refusal());
        }
        if sketches.len() > 1 {
            return Err(mixed_selection_refusal());
        }
        ctx.doc.transform_sketch(sketches[0], &t)?;
        return Ok(serde_json::json!({}));
    }

    apply_node_transform(ctx, &nodes, &t, p.copy)
}

/// Shared move/rotate execution over resolved nodes: plain transform, or
/// (with `copy`) `duplicate_node`/`duplicate_nodes_array` at the same
/// step (docs/agents/HEW_API.md §7's move semantics, reused by rotate).
fn apply_node_transform(
    ctx: &mut Ctx,
    nodes: &[NodeId],
    t: &Transform,
    copy: Option<CopySpec>,
) -> Result<Value, CmdError> {
    let Some(copy) = copy else {
        ctx.doc.transform_selection(nodes, &[], t)?;
        return Ok(serde_json::json!({}));
    };
    let count = copy.count.unwrap_or(1);
    // Mirrors the wasm boundary's MAX_ARRAY_COUNT trust cap: an agent
    // typo ("count": 1e9) must refuse, not grind the document forever.
    const API_MAX_ARRAY_COUNT: u32 = 1000;
    if count > API_MAX_ARRAY_COUNT {
        return Err(CmdError::Refusal(
            crate::refusal::Refusal::api(
                "array_count_too_large",
                &format!("An array copy places at most {API_MAX_ARRAY_COUNT} copies at once."),
            )
            .with_detail(serde_json::json!({ "max": API_MAX_ARRAY_COUNT })),
        ));
    }
    let count = NonZeroU32::new(count)
        .ok_or_else(|| CmdError::Params("copy count must be positive".into()))?;
    let ids = if nodes.len() == 1 && count.get() == 1 {
        let (root, _) = ctx.doc.duplicate_node(nodes[0], t)?;
        vec![root]
    } else {
        let (roots, _) = ctx.doc.duplicate_nodes_array(nodes, t, count)?;
        roots
    };
    let resolver = ctx.resolver();
    let public: Vec<String> = ids
        .iter()
        .map(|&n| {
            resolver
                .public_of(ctx.doc, &node_entity(n))
                .expect("just created")
        })
        .collect();
    Ok(serde_json::json!({ "ids": public }))
}

fn node_entity(n: NodeId) -> EntityRef {
    match n {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    }
}

// ---------------------------------------------------------------- rotate

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateParams {
    ids: Vec<String>,
    pivot: Value,
    axis: [f64; 3],
    angle: f64,
    #[serde(default)]
    copy: Option<CopySpec>,
}

fn rotate(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: RotateParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if p.ids.is_empty() {
        return Err(kernel::DocumentError::EmptySelection.into());
    }
    if !p.angle.is_finite() {
        return Err(CmdError::Params("angle must be finite".into()));
    }
    if p.axis.iter().any(|c| !c.is_finite()) {
        return Err(CmdError::Params("axis components must be finite".into()));
    }
    let axis = Vec3::new(p.axis[0], p.axis[1], p.axis[2]);
    if axis.length_squared() < 1e-24 {
        return Err(CmdError::Params("axis must be non-zero".into()));
    }
    let pivot = locate::resolve_point(ctx, &p.pivot)?;
    let r = Transform::rotation(axis, p.angle)
        .map_err(|_| CmdError::Params("rotation axis too short to define a direction".into()))?;
    let pivot_v = pivot.to_vec();
    let t = Transform::translation(-pivot_v)
        .then(&r)
        .then(&Transform::translation(pivot_v));

    let (nodes, sketches) = split_ids(ctx, &p.ids)?;
    if !sketches.is_empty() {
        // Rotate is a node-space transform in this registry slice;
        // sketches ride the same one-sketch-per-envelope / no-copy rule
        // as move for consistency, until sketch transforms get their own
        // command-level story.
        if !nodes.is_empty() || sketches.len() > 1 {
            return Err(mixed_selection_refusal());
        }
        if p.copy.is_some() {
            return Err(sketch_copy_refusal());
        }
        ctx.doc.transform_sketch(sketches[0], &t)?;
        return Ok(serde_json::json!({}));
    }
    apply_node_transform(ctx, &nodes, &t, p.copy)
}

// ----------------------------------------------------------------- scale

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleParams {
    ids: Vec<String>,
    anchor: Value,
    factors: [f64; 3],
}

fn scale(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ScaleParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if p.ids.is_empty() {
        return Err(kernel::DocumentError::EmptySelection.into());
    }
    if p.factors.iter().any(|f| !f.is_finite() || *f <= 0.0) {
        return Err(CmdError::Params(
            "scale factors must be finite and positive".into(),
        ));
    }
    let anchor = locate::resolve_point(ctx, &p.anchor)?;
    let anchor_v = anchor.to_vec();
    let s = Transform::scale(Vec3::new(p.factors[0], p.factors[1], p.factors[2]));
    let t = Transform::translation(-anchor_v)
        .then(&s)
        .then(&Transform::translation(anchor_v));

    let (nodes, sketches) = split_ids(ctx, &p.ids)?;
    if !sketches.is_empty() {
        if !nodes.is_empty() || sketches.len() > 1 {
            return Err(mixed_selection_refusal());
        }
        ctx.doc.transform_sketch(sketches[0], &t)?;
        return Ok(serde_json::json!({}));
    }
    ctx.doc.transform_selection(&nodes, &[], &t)?;
    Ok(serde_json::json!({}))
}
