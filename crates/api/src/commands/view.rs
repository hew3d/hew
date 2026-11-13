//! Live-viewport and app-display effects: `hew.view.camera`,
//! `hew.view.zoom_extents`, `hew.view.units` (docs/agents/HEW_API.md §7).
//!
//! Like `hew.view.snapshot` (doc.rs), all three are `Served::Host` — this
//! module owns no effect of its own, only parameter validation and result
//! shaping; the host performs the actual effect (or answers
//! `host_capability_missing` through the normal `CmdError::Refusal` path,
//! host.rs's `unsupported()`).
//!
//! None of the three changes document state: the registry declares
//! `mutates_document = false` for all three (`registry.rs`'s `Solitary`
//! default, un-corrected — contrast `hew.doc.new`/`open`, which the
//! registry explicitly flips back to `true`). A camera move is view
//! state, not a modeled edit; the display-unit format is an app-level
//! preference (`app/src/settings/units.ts`), never serialized into
//! `.hew`. Neither rides the undo log, and `crates/wasm-api`'s
//! `Scene::api_dispatch` never resyncs the document for them (it only
//! resyncs when `mutates_document` is true).
//!
//! The view-state commands below them (docs/agents/HEW_API.md §7,
//! "Selection and view state") are `ReadOnly`, so they ride a transaction
//! and roll back with it: `hew.view.hide`/`show`/`show_all`/`hidden` are
//! kernel-served over the document's user-hidden flags (view state, never
//! undoable); `hew.view.isolate`/`clear_isolation`/`zoom_to` are host
//! effects on the live viewport.
//!
//! `hew.view.enter`/`exit`/`exit_all` are the exception to "view state":
//! they act on the person's editing context (docs/agents/HEW_API.md §6.3).
//! Solitary (a transaction refuses them `-32602`), `mutates_document = true`
//! because a session open or close is a history step, and refused
//! `gesture_in_progress` while a tool gesture is armed.

use super::camera::{RawCamera, parse_camera_or_view};
use super::entity::unknown_entity;
use super::{CmdError, Ctx, Handler};
use crate::host::{IsolationEntry, SelectionEntry, ViewCameraSpec};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{EntityRef, NodeId, Point3};
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.view.camera" => Some(camera),
        "hew.view.zoom_extents" => Some(zoom_extents),
        "hew.view.units" => Some(units),
        "hew.view.hide" => Some(hide),
        "hew.view.show" => Some(show),
        "hew.view.show_all" => Some(show_all),
        "hew.view.hidden" => Some(hidden),
        "hew.view.isolate" => Some(isolate),
        "hew.view.clear_isolation" => Some(clear_isolation),
        "hew.view.zoom_to" => Some(zoom_to),
        "hew.view.enter" => Some(enter),
        "hew.view.exit" => Some(exit),
        "hew.view.exit_all" => Some(exit_all),
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

// ---------------------------------------------------------- hew.view.camera

/// `camera`/`view` are the exact vocabulary `hew.view.snapshot` accepts
/// (`camera.rs`), mutually exclusive; unlike snapshot, exactly one is
/// REQUIRED here — there is no "give neither" default because there is no
/// document camera to fall back to (snapshot's "no camera given" case
/// renders the document's saved working camera or a fitted isometric
/// view; a live camera-set command with neither would be a no-op with no
/// honest meaning).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraParams {
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    view: Option<String>,
}

fn camera(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let CameraParams { camera, view } = parse(params)?;
    let (camera, view) = parse_camera_or_view(camera, view)?;
    let spec = match (camera, view) {
        (Some(c), None) => ViewCameraSpec::Explicit(c),
        (None, Some(v)) => ViewCameraSpec::Standard(v),
        (None, None) => {
            return Err(CmdError::Params(
                "one of camera or view is required".to_string(),
            ));
        }
        (Some(_), Some(_)) => {
            unreachable!("parse_camera_or_view already enforces mutual exclusion")
        }
    };
    ctx.host.set_camera(&spec).map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ---------------------------------------------------- hew.view.zoom_extents

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoomExtentsParams {}

fn zoom_extents(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ZoomExtentsParams {} = parse(params)?;
    ctx.host.zoom_extents().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ----------------------------------------------------------- hew.view.units

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitsParams {
    format: String,
}

fn units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let UnitsParams { format } = parse(params)?;
    crate::commands::doc::check_length_format(&format)?;
    ctx.host
        .set_display_units(&format)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ------------------------------------------- hew.view.hide / show / show_all

/// Hide and show are kernel-served: the user-hidden flags are document
/// view state (`Document::set_node_user_hidden`: persisted, never undoable),
/// so they roll back with an aborted envelope like anything else the
/// envelope wrote. After the write the host is told
/// ([`crate::host::Host::hidden_changed`]) so a live app re-reads them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdsParams {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

/// Whether `node` is a live (not deleted, not an undone creation) node.
fn node_live(doc: &kernel::Document, node: NodeId) -> bool {
    match node {
        NodeId::Object(id) => doc.object(id).is_some(),
        NodeId::Group(id) => doc.group_members(id).is_some(),
        NodeId::Instance(id) => doc.instance_def(id).is_some(),
    }
}

/// Resolves a hide/show id: a live object, group, or component instance.
/// Any other kind refuses `not_a_node`; an id naming nothing live refuses
/// `unknown_entity`.
fn resolve_hideable(ctx: &Ctx, id: &str) -> Result<NodeId, CmdError> {
    let node = match ctx.resolver().resolve(id) {
        None => return Err(unknown_entity(id)),
        Some(EntityRef::Object(o)) => NodeId::Object(o),
        Some(EntityRef::Group(g)) => NodeId::Group(g),
        Some(EntityRef::Instance(i)) => NodeId::Instance(i),
        Some(_) => {
            return Err(CmdError::Refusal(
                Refusal::api(
                    "not_a_node",
                    "Only objects, groups, and component instances can be hidden; tags hide through hew.tag.set_visible.",
                )
                .with_detail(serde_json::json!({ "id": id })),
            ));
        }
    };
    if !node_live(ctx.doc, node) {
        return Err(unknown_entity(id));
    }
    Ok(node)
}

fn node_entity(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(id) => EntityRef::Object(id),
        NodeId::Group(id) => EntityRef::Group(id),
        NodeId::Instance(id) => EntityRef::Instance(id),
    }
}

/// A node's public id, when it carries a stable id.
fn node_public(doc: &kernel::Document, node: NodeId) -> Option<String> {
    let entity = node_entity(node);
    doc.sid_of(&entity)
        .map(|sid| crate::ids::public_id(&entity, sid))
}

/// Every live user-hidden node's public id, in stable-id order.
fn user_hidden_ids(doc: &kernel::Document) -> Vec<String> {
    let mut nodes: Vec<(u64, String)> = doc
        .user_hidden_nodes()
        .into_iter()
        .filter(|n| node_live(doc, *n))
        .filter_map(|n| {
            let entity = node_entity(n);
            doc.sid_of(&entity)
                .map(|sid| (sid, crate::ids::public_id(&entity, sid)))
        })
        .collect();
    nodes.sort();
    nodes.into_iter().map(|(_, id)| id).collect()
}

fn set_user_hidden(ctx: &mut Ctx, params: &Value, hidden: bool) -> Result<Value, CmdError> {
    let IdsParams { ids } = parse(params)?;
    // Every id is checked before any flag moves: a refused call changes
    // nothing.
    let nodes = ids
        .iter()
        .map(|id| resolve_hideable(ctx, id))
        .collect::<Result<Vec<NodeId>, CmdError>>()?;
    for node in nodes {
        ctx.doc.set_node_user_hidden(node, hidden);
    }
    ctx.host.hidden_changed().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({ "hidden": user_hidden_ids(ctx.doc) }))
}

fn hide(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    set_user_hidden(ctx, params, true)
}

fn show(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    set_user_hidden(ctx, params, false)
}

fn show_all(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    for node in ctx.doc.user_hidden_nodes() {
        ctx.doc.set_node_user_hidden(node, false);
    }
    ctx.host.hidden_changed().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({ "hidden": [] }))
}

// ---------------------------------------------------------- hew.view.hidden

/// An isolation entry on the wire: a node's public id, or
/// `{ "object": "obj_x", "instance": "ins_y" }` for a definition member
/// shown through one placing instance. An entry whose entity no longer
/// carries a stable id is left out.
fn isolation_json(doc: &kernel::Document, entries: &[IsolationEntry]) -> Vec<Value> {
    entries
        .iter()
        .filter_map(|e| match *e {
            IsolationEntry::Node(node) => node_public(doc, node).map(Value::String),
            IsolationEntry::Member { object, instance } => {
                let object = node_public(doc, NodeId::Object(object))?;
                let instance = node_public(doc, NodeId::Instance(instance))?;
                Some(serde_json::json!({ "object": object, "instance": instance }))
            }
        })
        .collect()
}

fn hidden(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    let tags_hidden: Vec<String> = ctx
        .doc
        .tag_meta()
        .filter(|(_, hidden)| *hidden)
        .filter_map(|(path, _)| {
            let entity = EntityRef::Tag(path.to_vec());
            ctx.doc
                .sid_of(&entity)
                .map(|sid| crate::ids::public_id(&entity, sid))
        })
        .collect();
    // A host without isolation (a headless one) reports none rather than
    // refusing the whole read.
    let isolated = match ctx.host.isolation() {
        Ok(Some(entries)) => Value::Array(isolation_json(ctx.doc, &entries)),
        Ok(None) | Err(_) => Value::Null,
    };
    Ok(serde_json::json!({
        "hidden": user_hidden_ids(ctx.doc),
        "tags_hidden": tags_hidden,
        "isolated": isolated,
    }))
}

// ------------------------------------------- hew.view.isolate / clear_isolation

/// The most targets one `hew.view.isolate` names.
pub const MAX_ISOLATE_IDS: usize = 10_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IsolateParams {
    ids: Vec<IsolateTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IsolateTarget {
    Node(String),
    Member(MemberTarget),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberTarget {
    object: String,
    instance: String,
}

/// Resolves one isolation target: a live object, group, or instance, or a
/// definition member together with an instance that places it (directly
/// or through nested components).
fn resolve_isolation_target(ctx: &Ctx, target: &IsolateTarget) -> Result<IsolationEntry, CmdError> {
    let resolver = ctx.resolver();
    match target {
        IsolateTarget::Node(id) => {
            let node = match resolver.resolve(id) {
                Some(EntityRef::Object(o)) => NodeId::Object(o),
                Some(EntityRef::Group(g)) => NodeId::Group(g),
                Some(EntityRef::Instance(i)) => NodeId::Instance(i),
                _ => return Err(unknown_entity(id)),
            };
            if !node_live(ctx.doc, node) {
                return Err(unknown_entity(id));
            }
            Ok(IsolationEntry::Node(node))
        }
        IsolateTarget::Member(MemberTarget { object, instance }) => {
            let Some(EntityRef::Instance(i)) = resolver.resolve(instance) else {
                return Err(unknown_entity(instance));
            };
            let Some(def) = ctx.doc.instance_def(i) else {
                return Err(unknown_entity(instance));
            };
            let Some(EntityRef::Object(o)) = resolver.resolve(object) else {
                return Err(unknown_entity(object));
            };
            let placed = ctx.doc.object(o).is_some()
                && ctx
                    .doc
                    .expanded_def_placements(def)
                    .iter()
                    .any(|(member, _)| *member == o);
            if !placed {
                return Err(unknown_entity(object));
            }
            Ok(IsolationEntry::Member {
                object: o,
                instance: i,
            })
        }
    }
}

fn isolate(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let IsolateParams { ids } = parse(params)?;
    if ids.is_empty() || ids.len() > MAX_ISOLATE_IDS {
        return Err(CmdError::Params(format!(
            "ids names {} targets; between 1 and {MAX_ISOLATE_IDS} are allowed",
            ids.len()
        )));
    }
    let entries = ids
        .iter()
        .map(|t| resolve_isolation_target(ctx, t))
        .collect::<Result<Vec<IsolationEntry>, CmdError>>()?;
    ctx.host
        .isolate(entries.clone())
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({ "isolated": isolation_json(ctx.doc, &entries) }))
}

fn clear_isolation(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    ctx.host.clear_isolation().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// --------------------------------------------------------- hew.view.zoom_to

/// `padding`'s default: a tenth of the box's largest extent on each side.
pub const DEFAULT_ZOOM_PADDING: f64 = 0.1;

/// How wide, in meters, `hew.view.zoom_to` frames a box with no extent.
pub const POINT_FRAME_EXTENT: f64 = 1.0;

/// Below this largest extent, in meters, a box counts as a point.
const DEGENERATE_EXTENT: f64 = 1e-9;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoomToParams {
    ids: Vec<String>,
    #[serde(default)]
    padding: Option<f64>,
}

/// Whether `entity` is live, for the kinds that can be deleted or undone
/// away; components, materials, and tags answer by resolving at all.
fn entity_live(doc: &kernel::Document, entity: &EntityRef) -> bool {
    SelectionEntry::from_entity(entity).is_none_or(|e| crate::selection::entry_live(doc, e))
}

fn zoom_to(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ZoomToParams { ids, padding } = parse(params)?;
    let padding = padding.unwrap_or(DEFAULT_ZOOM_PADDING);
    if !(0.0..=2.0).contains(&padding) {
        return Err(CmdError::Params(
            "padding must be a number from 0 to 2".to_string(),
        ));
    }
    let resolver = ctx.resolver();
    let mut boxes: Vec<(Point3, Point3)> = Vec::new();
    for id in &ids {
        let entity = resolver
            .resolve(id)
            .filter(|e| entity_live(ctx.doc, e))
            .ok_or_else(|| unknown_entity(id))?;
        if let Some(b) = crate::locate::entity_bbox(ctx, &entity) {
            boxes.push(b);
        }
    }
    let Some((min, max)) = boxes.into_iter().reduce(|(amin, amax), (bmin, bmax)| {
        (
            Point3::new(amin.x.min(bmin.x), amin.y.min(bmin.y), amin.z.min(bmin.z)),
            Point3::new(amax.x.max(bmax.x), amax.y.max(bmax.y), amax.z.max(bmax.z)),
        )
    }) else {
        return Err(CmdError::Refusal(Refusal::api(
            "nothing_to_frame",
            "Nothing named has an extent to frame (guide lines, materials, and tags have none). Name objects, groups, instances, sketches, guide points, or annotations.",
        )));
    };
    let extent = (max.x - min.x).max(max.y - min.y).max(max.z - min.z);
    // A box with no extent (a lone guide point) frames as a cube
    // `POINT_FRAME_EXTENT` wide around it, padded the same way, rather than
    // putting the camera a hair's breadth from a point.
    let (base, grow) = if extent < DEGENERATE_EXTENT {
        (POINT_FRAME_EXTENT, POINT_FRAME_EXTENT / 2.0)
    } else {
        (extent, 0.0)
    };
    let pad = grow + padding * base;
    let padded_min = Point3::new(min.x - pad, min.y - pad, min.z - pad);
    let padded_max = Point3::new(max.x + pad, max.y + pad, max.z + pad);
    ctx.host
        .zoom_to(padded_min, padded_max, padding)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({
        "min": [min.x, min.y, min.z],
        "max": [max.x, max.y, max.z],
    }))
}

// ------------------------------------------ hew.view.enter / exit / exit_all

/// Refuses while a tool gesture is armed in the user's window: the app's own
/// entry silently does nothing then, so the bus says so instead.
fn refuse_if_gesture_armed(ctx: &Ctx) -> Result<(), CmdError> {
    if ctx.host.gesture_armed() {
        return Err(CmdError::Refusal(Refusal::api(
            "gesture_in_progress",
            "A tool gesture is in progress in the Hew window. Wait for it to finish, then try again.",
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<Vec<String>>,
}

/// The `-32602` for a target that is not an object, group, or instance.
fn not_enterable(id: &str) -> CmdError {
    CmdError::Params(format!(
        "{id} cannot be entered for editing; name an object, group, or component instance"
    ))
}

/// Checks that `id` names a node (object, group, or instance); a sketch,
/// guide, annotation, material, tag, or component definition is a params
/// error, an id that resolves to nothing is `unknown_entity`.
fn check_enterable(ctx: &Ctx, id: &str) -> Result<(), CmdError> {
    match ctx.resolver().resolve(id) {
        Some(EntityRef::Object(_) | EntityRef::Group(_) | EntityRef::Instance(_)) => Ok(()),
        Some(_) => Err(not_enterable(id)),
        None => Err(unknown_entity(id)),
    }
}

fn enter(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let EnterParams { id, path } = parse(params)?;
    let occurrence = match (id, path) {
        (Some(id), None) => {
            check_enterable(ctx, &id)?;
            locate::resolve_world_node(ctx, &id)?
        }
        (None, Some(path)) => {
            if path.is_empty() {
                return Err(CmdError::Params(
                    "path names at least one entity".to_string(),
                ));
            }
            for id in &path {
                check_enterable(ctx, id)?;
            }
            locate::resolve_occurrence_path(ctx, &path)?
        }
        _ => {
            return Err(CmdError::Params(
                "exactly one of id or path is required".to_string(),
            ));
        }
    };
    refuse_if_gesture_armed(ctx)?;
    ctx.host
        .enter_user_context(&occurrence.path, occurrence.node)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

fn exit(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    refuse_if_gesture_armed(ctx)?;
    ctx.host.exit_user_context().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

fn exit_all(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NoParams {} = parse(params)?;
    refuse_if_gesture_armed(ctx)?;
    ctx.host
        .exit_all_user_contexts()
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}
