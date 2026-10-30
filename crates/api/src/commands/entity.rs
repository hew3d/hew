//! Entity lifecycle + transforms: hew.entity.* (docs/agents/HEW_API.md §7),
//! plus hew.doc.purge_unused — the palette/definition housekeeping
//! counterpart of `delete`, sharing this module's `EntityRef`-kind
//! dispatch and material/component delete paths rather than living in
//! `doc.rs` (which is Host-served document lifecycle only; purge is a
//! plain kernel mutation).
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
        "hew.entity.set_frame" => set_frame,
        "hew.doc.purge_unused" => purge_unused,
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
        EntityRef::Material(m) => {
            // Unlike node display names, a palette material's name is a
            // required `String` (kernel::Material::name), never `Option` —
            // there is no "unnamed, falls back to a positional label"
            // state for a swatch. `name: null` is therefore a params error,
            // not a kernel refusal.
            let name = p.name.ok_or_else(|| {
                CmdError::Params("a material's name cannot be cleared to null".into())
            })?;
            ctx.doc.set_material_name(m, name)?
        }
        EntityRef::Sketch(_)
        | EntityRef::Guide(_)
        | EntityRef::Tag(_)
        | EntityRef::Annotation(_) => {
            return Err(CmdError::Refusal(Refusal::api(
                "rename_unsupported",
                "Only objects, groups, instances, component definitions, and materials can be renamed today.",
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
        EntityRef::Annotation(a) => ctx.doc.delete_annotation(a)?,
        EntityRef::Material(m) => ctx.doc.delete_material(m)?,
        EntityRef::Component(c) => ctx.doc.delete_definition(c)?,
    };
    Ok(serde_json::json!({}))
}

// -------------------------------------------------------- hew.doc.purge_unused

fn purge_unused(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    let _: Empty =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let report = ctx.doc.purge_unused()?;
    Ok(serde_json::json!({
        "materials": report.materials,
        "definitions": report.definitions,
    }))
}

// ------------------------------------------------------------------ move

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CopySpec {
    #[serde(default)]
    count: Option<u32>,
    /// A grid copy (`hew.entity.move` only): 1..=3 axes, each `count`
    /// slots (the original's included) at `step` apart.
    #[serde(default)]
    grid: Option<Vec<GridAxis>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GridAxis {
    count: u32,
    step: [f64; 3],
}

/// Mirrors the wasm boundary's MAX_ARRAY_COUNT trust cap: an agent typo
/// ("count": 1e9) must refuse, not grind the document forever.
const API_MAX_ARRAY_COUNT: u32 = 1000;

fn array_count_too_large() -> CmdError {
    CmdError::Refusal(
        crate::refusal::Refusal::api(
            "array_count_too_large",
            &format!("An array copy places at most {API_MAX_ARRAY_COUNT} copies at once."),
        )
        .with_detail(serde_json::json!({ "max": API_MAX_ARRAY_COUNT })),
    )
}

/// A grid copy's placements: every index tuple `(i0, i1, i2)` in
/// lexicographic order except the original's `(0, 0, 0)`, each the
/// translation `Σ i_a · step_a`. The product of the counts, the original's
/// slot included, is at most [`API_MAX_ARRAY_COUNT`].
fn grid_placements(grid: &[GridAxis]) -> Result<Vec<Transform>, CmdError> {
    if grid.is_empty() || grid.len() > 3 {
        return Err(CmdError::Params("copy.grid takes one to three axes".into()));
    }
    let mut slots: u64 = 1;
    for axis in grid {
        if axis.count == 0 {
            return Err(CmdError::Params(
                "a grid axis's count must be at least 1".into(),
            ));
        }
        if axis.step.iter().any(|c| !c.is_finite()) {
            return Err(CmdError::Params(
                "a grid axis's step components must be finite".into(),
            ));
        }
        slots = slots.saturating_mul(u64::from(axis.count));
    }
    if slots > u64::from(API_MAX_ARRAY_COUNT) {
        return Err(array_count_too_large());
    }
    if slots == 1 {
        return Err(CmdError::Params(
            "a grid copy needs a count above 1 on at least one axis".into(),
        ));
    }
    let mut placements = Vec::with_capacity(slots as usize - 1);
    let mut index = vec![0u32; grid.len()];
    loop {
        // Advance the last axis fastest: lexicographic order.
        let mut a = grid.len();
        loop {
            if a == 0 {
                return Ok(placements);
            }
            a -= 1;
            index[a] += 1;
            if index[a] < grid[a].count {
                break;
            }
            index[a] = 0;
        }
        let mut offset = Vec3::new(0.0, 0.0, 0.0);
        for (i, axis) in index.iter().zip(grid) {
            let s = Vec3::new(axis.step[0], axis.step[1], axis.step[2]);
            offset = offset + s * f64::from(*i);
        }
        placements.push(Transform::translation(offset));
    }
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
    if let Some(grid) = p.copy.as_ref().and_then(|c| c.grid.as_ref()) {
        if p.copy.as_ref().is_some_and(|c| c.count.is_some()) {
            return Err(CmdError::Params(
                "copy takes \"count\" or \"grid\", not both".into(),
            ));
        }
        if has_translation || has_from_to {
            return Err(CmdError::Params(
                "a grid copy takes its steps from \"grid\", not from a translation".into(),
            ));
        }
        let placements = grid_placements(grid)?;
        let (nodes, sketches) = split_ids(ctx, &p.ids)?;
        if !sketches.is_empty() {
            if !nodes.is_empty() {
                return Err(mixed_selection_refusal());
            }
            return Err(sketch_copy_refusal());
        }
        let (roots, _) = ctx.doc.duplicate_nodes_placements(&nodes, &placements)?;
        return Ok(serde_json::json!({ "ids": public_ids(ctx, &roots) }));
    }
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
    if count > API_MAX_ARRAY_COUNT {
        return Err(array_count_too_large());
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
    Ok(serde_json::json!({ "ids": public_ids(ctx, &ids) }))
}

/// The public ids of nodes a copy just created.
fn public_ids(ctx: &Ctx, ids: &[NodeId]) -> Vec<String> {
    let resolver = ctx.resolver();
    ids.iter()
        .map(|&n| {
            resolver
                .public_of(ctx.doc, &node_entity(n))
                .expect("just created")
        })
        .collect()
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
    if p.copy.as_ref().is_some_and(|c| c.grid.is_some()) {
        return Err(CmdError::Params(
            "hew.entity.rotate's copy takes \"count\" only; a grid copy is hew.entity.move's"
                .into(),
        ));
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
    if p.factors.iter().any(|f| !f.is_finite() || *f == 0.0) {
        return Err(CmdError::Params(
            "scale factors must be finite and nonzero".into(),
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

// ------------------------------------------------------------- set_frame

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetFrameParams {
    id: String,
    #[serde(default)]
    origin: Option<[f64; 3]>,
    #[serde(default)]
    x: Option<[f64; 3]>,
    #[serde(default)]
    y: Option<[f64; 3]>,
    #[serde(default)]
    clear: Option<bool>,
}

/// `hew.entity.set_frame`: writes (or clears) the `hew.part` / `frame`
/// override `hew.query.part_frame` reads first, on an object or group
/// (docs/agents/PLUGINS.md "Setting axes"). The values are coordinates in
/// the face frame `part_frame` reports for the part without the override,
/// after its `[length, width, thickness]` swap (`crate::part_frame`), and
/// are stored as given once [`kernel::AxesFrame::new`] accepts them. This
/// is the one first-party writer of `hew.part`, so it bypasses the
/// attribute namespace rule `hew.attr.set` enforces.
fn set_frame(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    use crate::part_frame::{FRAME_KEY, FRAME_NAMESPACE};
    use kernel::{AttrTarget, AttrValue, AxesFrame, Point3};
    let p: SetFrameParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let clear = p.clear.unwrap_or(false);
    let vectors = [p.origin.is_some(), p.x.is_some(), p.y.is_some()];
    let frame = match (clear, vectors) {
        (true, [false, false, false]) => None,
        (false, [true, true, true]) => Some((p.origin.unwrap(), p.x.unwrap(), p.y.unwrap())),
        _ => {
            return Err(CmdError::Params(
                "set_frame takes \"origin\", \"x\", and \"y\" together, or \"clear\": true alone"
                    .into(),
            ));
        }
    };
    let entity = match ctx.resolver().resolve(&p.id) {
        Some(e @ (EntityRef::Object(_) | EntityRef::Group(_))) => e,
        Some(_) => {
            return Err(CmdError::Refusal(Refusal::api(
                "not_a_part",
                "Only objects and groups take a part frame; components use hew.component.rebase.",
            )));
        }
        None => return Err(unknown_entity(&p.id)),
    };
    let target = AttrTarget::Entity(entity);
    let Some((origin, x, y)) = frame else {
        let present = ctx
            .doc
            .attr_get(&target)?
            .and_then(|d| d.get(FRAME_NAMESPACE))
            .is_some_and(|ns| ns.contains_key(FRAME_KEY));
        if present {
            ctx.doc
                .attr_delete(target, FRAME_NAMESPACE, Some(FRAME_KEY))?;
        }
        return Ok(serde_json::json!({ "id": p.id, "source": "face" }));
    };
    let v = |c: [f64; 3]| Vec3::new(c[0], c[1], c[2]);
    AxesFrame::new(Point3::new(origin[0], origin[1], origin[2]), v(x), v(y))
        .map_err(kernel::DocumentError::InvalidFrame)?;
    let list = |c: [f64; 3]| AttrValue::List(c.iter().map(|&n| AttrValue::Float(n)).collect());
    let mut m = std::collections::BTreeMap::new();
    m.insert("origin".to_string(), list(origin));
    m.insert("x".to_string(), list(x));
    m.insert("y".to_string(), list(y));
    ctx.doc
        .attr_set(target, FRAME_NAMESPACE, FRAME_KEY, AttrValue::Map(m))?;
    Ok(serde_json::json!({ "id": p.id, "source": "attribute" }))
}
