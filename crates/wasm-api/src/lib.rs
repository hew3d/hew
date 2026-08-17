// Outside the determinism-critical kernel scope (kernel / inference /
// tessellate / mesh-heal). The WASM boundary holds only session view-state
// (e.g. hidden-object sets) that never feeds the canonical serialization, so
// HashSet/HashMap iteration order cannot perturb kernel output. The workspace
// clippy.toml ban is suppressed here deliberately.
#![allow(clippy::disallowed_types)]

//! The WASM boundary: the only crate allowed to know about JS
//! (DEVELOPMENT.md rule 1).
//!
//! Surface design and decision record: `docs/DEVELOPMENT.md` (rule 8 sign-off
//! trail). Summary: the authoritative model is the kernel [`Document`]; [`Scene`]
//! is its FFI shim, adding only the inference scene and render-mesh caches (the
//! concerns the kernel may not depend on). The UI holds opaque `u64` handles
//! (BigInt in JS), pulls copied buffers after mutations, and receives typed
//! errors as thrown `"CODE: message"` strings.
//!
//! Document-level undo/redo (`scene_undo`/`scene_redo`) orders all mutations
//! and wraps per-Object [`History`]; undoing a creation hides the object
//! rather than deleting it, so handles stay stable across undo/redo.
//! `version()`/`demo_mesh()` remain from M0 until the viewport fully retires
//! the demo path.

mod live;
mod log;
mod recording;
mod scenes_api;

use dae_import::ImageMap;
use inference::{
    ApertureMode, Axis, ElementRef, InferenceScene, PickRay, SketchRegionFace, SnapKind, SnapLock,
    SnapQuery, SnapWeights,
};
use js_sys::{Object as JsObject, Reflect, Uint8Array};
use kernel::{
    Anchor, Annotation, AnnotationId, BooleanOp, CapturedCurve, ComponentId, DocChange, Document,
    DocumentError, EdgeId, FaceId, GroupId, Guide, GuideId, ImageFormat, InstanceId, KernelOp,
    KernelOpError, KernelOpReport, LoadError, Material, MaterialId, MaterialScope, NodeId, Object,
    ObjectId, Plane, Point3, RadialKind, Rgba8, SketchCurveRim, SketchEdgeId, SketchId,
    SketchRegionId, Texture, Transform, UvFrame, Vec3, WatertightState,
};
use slotmap::{Key, KeyData, SecondaryMap};
use tessellate::{RenderMesh, tessellate};
use wasm_bindgen::prelude::*;

/// Pick-cone half-angle (radians) for [`Scene::pick_sketch`]. Unlike `snap`'s
/// caller-supplied, screen-derived aperture, `pick_sketch` mirrors `pick_face`'s
/// parameterless shape — but a sketch edge (unlike a face) has zero thickness,
/// so *some* angular tolerance is unavoidable. `0.02` rad (~1.15°) is in the
/// same neighborhood as the tightest apertures already exercised in the
/// inference test suite (e.g. `aperture: 0.05`), forgiving enough for a
/// deliberate click without competing with nearby solid geometry.
const SKETCH_PICK_APERTURE: f64 = 0.02;

fn segment_ray_depth(origin: Point3, direction: kernel::Vec3, a: Point3, b: Point3) -> f64 {
    let dir = direction.normalized().unwrap_or(direction);
    let seg = b - a;
    let seg_len_sq = seg.length_squared();
    let s = if seg_len_sq <= kernel::tol::NORMALIZE_MIN_LENGTH.powi(2) {
        0.0
    } else {
        let w = origin - a;
        let ray_seg = dir.dot(seg);
        let denom = seg_len_sq - ray_seg * ray_seg;
        if denom.abs() < kernel::tol::NORMALIZE_MIN_LENGTH {
            0.0
        } else {
            ((seg.dot(w) - ray_seg * dir.dot(w)) / denom).clamp(0.0, 1.0)
        }
    };
    ((a + seg * s) - origin).dot(dir)
}

// Persist a panic message where the UI can read it after the wasm instance is
// poisoned. `console_error_panic_hook` writes through a web-sys console binding
// that bypasses the app's `console.error` capture, so a kernel panic was
// invisible to the in-app error surface — we route it to `localStorage` (and
// `console.error`, in a try/catch so a failure here can't re-panic) instead.
#[wasm_bindgen(inline_js = "export function __hew_record_panic(msg) { \
  try { localStorage.setItem('hew:lastPanic', new Date().toISOString() + '\\n' + msg); } catch (e) {} \
  try { console.error(msg); } catch (e) {} \
}")]
extern "C" {
    fn __hew_record_panic(msg: &str);
}

/// Module-init hook: install a panic hook that records the real message +
/// source location to `localStorage['hew:lastPanic']` (and `console.error`).
/// Without it, a kernel panic surfaces only as the opaque wasm "unreachable"
/// trap on the *next* call. (The panic still poisons the instance — reload to
/// recover — but the cause is now diagnosable from the UI.)
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        __hew_record_panic(&info.to_string());
    }));
}

/// Kernel crate version, for smoke tests and an about box.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ------------------------------------------------------- diagnostic logging
//
// The wasm half of the log seam (see `log.rs`): install the kernel-side
// `tracing` subscriber, route its JSON records to a JS drain (Tauri rolling file
// / web ring buffer), and bracket a user gesture with a correlation id.
// Rule-8 surface; recorded in docs/DEVELOPMENT.md.

/// Installs the structured-logging subscriber once and sets the capture level
/// (`"trace"|"debug"|"info"|"warn"|"error"`). Idempotent: a second call only
/// updates the level (the global subscriber can be set just once per process).
/// Until a drain is installed via [`set_log_drain`], records collect in an
/// in-memory ring buffer drainable with [`drain_log_records`].
#[wasm_bindgen]
pub fn init_logging(level: &str) {
    log::set_capture_level(level);
    // `set_global_default` errors if already set; ignore — the level was updated
    // above and the subscriber is already live.
    let _ = tracing::subscriber::set_global_default(log::DrainSubscriber);
}

/// Sets the capture level without (re)installing the subscriber.
#[wasm_bindgen]
pub fn set_log_level(level: &str) {
    log::set_capture_level(level);
}

/// Installs a JS drain: `cb(jsonRecord: string)` is invoked once per log record.
/// Replaces any previous drain and stops buffering (the TS sink owns the tail).
#[wasm_bindgen]
pub fn set_log_drain(cb: js_sys::Function) {
    log::set_drain(Box::new(move |json: &str| {
        // A drain callback must never unwind into the kernel; ignore a throwing
        // JS sink rather than poison the wasm instance.
        let _ = cb.call1(&JsValue::NULL, &JsValue::from_str(json));
    }));
}

/// Removes the JS drain; later records fall back to the in-memory ring buffer.
#[wasm_bindgen]
pub fn clear_log_drain() {
    log::clear_drain();
}

/// Takes and clears the buffered JSON records — the web on-demand download path
/// (no JS drain installed). Each element is one `LogRecord` as a JSON string.
#[wasm_bindgen]
pub fn drain_log_records() -> Vec<String> {
    log::drain_buffer()
}

/// Opens a correlation scope for one user gesture and returns its id; every log
/// record until [`end_gesture`] carries it, so the log filters to one gesture.
#[wasm_bindgen]
pub fn begin_gesture() -> u64 {
    log::begin_gesture()
}

/// Closes the current gesture's correlation scope.
#[wasm_bindgen]
pub fn end_gesture() {
    log::end_gesture()
}

/// Ceiling on [`Scene::duplicate_selection_array`]'s `count`, enforced at
/// this trust boundary (recorded sessions are plain JSON replayed through
/// that method verbatim, so a hand-edited count must fail typed instead of
/// hanging the engine). The single source of truth: the UI reads it via
/// [`Scene::max_array_count`], so the app-side cap cannot drift.
pub const MAX_ARRAY_COUNT: u32 = 1000;

// ------------------------------------------------------------------ errors

/// Boundary error: stringly `"CODE: message"` per docs/DEVELOPMENT.md B3.
#[derive(Debug)]
pub struct ApiError(String);

impl ApiError {
    /// Builds a `"CODE: message"` boundary error directly.
    fn new(code: &str, message: &str) -> ApiError {
        ApiError(format!("{code}: {message}"))
    }
}

impl From<ApiError> for JsValue {
    fn from(e: ApiError) -> JsValue {
        JsValue::from_str(&e.0)
    }
}

/// Builds a `"CODE: message"` error where CODE is the leading identifier of
/// the error's Debug form (its variant name). Pass the INNERMOST typed
/// error, not a wrapper.
fn api_err(code_source: &dyn std::fmt::Debug, message: &dyn std::fmt::Display) -> ApiError {
    let debug = format!("{code_source:?}");
    let code: String = debug
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    ApiError(format!("{code}: {message}"))
}

fn stale(code: &str, what: &str) -> ApiError {
    ApiError(format!("{code}: stale or unknown {what} handle"))
}

/// Maps a [`DocumentError`] to the `"CODE: message"` boundary form, choosing
/// the *innermost* error for CODE so callers see e.g. `DistanceTooSmall` or
/// `UnknownRegion` rather than an opaque wrapper name. The message is the
/// `DocumentError`'s own `Display`, which delegates to the inner error.
fn doc_err(e: DocumentError) -> ApiError {
    match &e {
        DocumentError::Sketch(inner) => api_err(inner, &e),
        DocumentError::Extrude(inner) => api_err(inner, &e),
        DocumentError::FollowMe(inner) => api_err(inner, &e),
        DocumentError::Boolean(inner) => api_err(inner, &e),
        DocumentError::Slice(inner) => api_err(inner, &e),
        DocumentError::Transform(inner) => api_err(inner, &e),
        DocumentError::Op(KernelOpError::PushPull(inner)) => api_err(inner, &e),
        DocumentError::Op(KernelOpError::Sticky(inner)) => api_err(inner, &e),
        DocumentError::InvalidAxesFrame(inner) => api_err(inner, &e),
        // UnknownSketch/UnknownObject/UnknownFace/UnknownMaterial/NothingTo{Undo,Redo}/
        // InverseFailed carry no separate inner code: the variant name is the code.
        _ => api_err(&e, &e),
    }
}

/// Serialize a kernel `ImportReport` to the plain JS object the import UI
/// consumes: `{ objects_created, watertight, leaky, skipped: [{name, reason}],
/// textures_missing: [string], warnings: [string] }`. Shared by `import_dae`,
/// `import_gltf`, and `import_skp`; `warnings` carries each importer's
/// conversion notes (non-manifold split decompositions, plus `.skp` parser
/// recovery notes).
fn import_report_to_js(report: &kernel::ImportReport, warnings: &[String]) -> JsValue {
    let js_report = JsObject::new();
    Reflect::set(
        &js_report,
        &JsValue::from_str("objects_created"),
        &JsValue::from_f64(report.objects_created as f64),
    )
    .unwrap();
    Reflect::set(
        &js_report,
        &JsValue::from_str("watertight"),
        &JsValue::from_f64(report.watertight as f64),
    )
    .unwrap();
    Reflect::set(
        &js_report,
        &JsValue::from_str("leaky"),
        &JsValue::from_f64(report.leaky as f64),
    )
    .unwrap();

    let skipped_arr = js_sys::Array::new();
    for s in &report.skipped {
        let entry = JsObject::new();
        Reflect::set(
            &entry,
            &JsValue::from_str("name"),
            &JsValue::from_str(&s.name),
        )
        .unwrap();
        Reflect::set(
            &entry,
            &JsValue::from_str("reason"),
            &JsValue::from_str(&s.reason),
        )
        .unwrap();
        skipped_arr.push(&entry);
    }
    Reflect::set(&js_report, &JsValue::from_str("skipped"), &skipped_arr).unwrap();

    let missing_arr = js_sys::Array::new();
    for uri in &report.textures_missing {
        missing_arr.push(&JsValue::from_str(uri));
    }
    Reflect::set(
        &js_report,
        &JsValue::from_str("textures_missing"),
        &missing_arr,
    )
    .unwrap();

    let warnings_arr = js_sys::Array::new();
    for w in warnings {
        warnings_arr.push(&JsValue::from_str(w));
    }
    Reflect::set(&js_report, &JsValue::from_str("warnings"), &warnings_arr).unwrap();

    js_report.into()
}

/// Serialize a kernel [`kernel::InsertReport`] to the plain JS object the
/// library-insert UI consumes. Root handles cross as DECIMAL STRINGS (the
/// harness convention — `u64` inside a plain JS object would silently lose
/// precision as a float).
fn insert_report_to_js(report: &kernel::InsertReport) -> JsValue {
    let js = JsObject::new();
    let set_num = |key: &str, v: usize| {
        Reflect::set(&js, &JsValue::from_str(key), &JsValue::from_f64(v as f64)).unwrap();
    };
    let kinds_arr = js_sys::Array::new();
    let ids_arr = js_sys::Array::new();
    for root in &report.roots {
        let (k, id) = match root {
            NodeId::Object(id) => (0u8, id.data().as_ffi()),
            NodeId::Group(id) => (1u8, id.data().as_ffi()),
            NodeId::Instance(id) => (2u8, id.data().as_ffi()),
        };
        kinds_arr.push(&JsValue::from_f64(k as f64));
        ids_arr.push(&JsValue::from_str(&id.to_string()));
    }
    Reflect::set(&js, &JsValue::from_str("rootKinds"), &kinds_arr).unwrap();
    Reflect::set(&js, &JsValue::from_str("rootIds"), &ids_arr).unwrap();
    set_num("definitionsAdded", report.definitions_added);
    set_num("definitionsReused", report.definitions_reused);
    set_num("materialsAdded", report.materials_added);
    set_num("materialsReused", report.materials_reused);
    set_num("objectsAdded", report.objects_added);
    set_num("guidesAdded", report.guides_added);
    set_num("worldSketchesSkipped", report.world_sketches_skipped);
    set_num("annotationsSkipped", report.annotations_skipped);
    js.into()
}

/// Writes a library item's metadata — a JSON object of key → value — into
/// `item`'s DOCUMENT attribute dictionary under the `hew.library`
/// namespace, one key at a time through the validating
/// [`kernel::Document::attr_set`]. A malformed JSON string or a
/// non-representable value (non-finite number, over-deep nesting) is a
/// typed error, never partially applied silently — though keys already
/// written before a failing one do remain on this fresh item document,
/// which the caller then discards.
fn apply_library_meta(item: &mut Document, meta_json: Option<&str>) -> Result<(), ApiError> {
    let Some(raw) = meta_json.filter(|m| !m.trim().is_empty()) else {
        return Ok(());
    };
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| ApiError(format!("BadLibraryMeta: not a JSON object: {e}")))?;
    let serde_json::Value::Object(map) = parsed else {
        return Err(ApiError(
            "BadLibraryMeta: metadata must be a JSON object".to_string(),
        ));
    };
    for (key, value) in map {
        let attr_value = kernel::AttrValue::from_json(&value)
            .map_err(|e| ApiError(format!("BadLibraryMeta: {key}: {e:?}")))?;
        item.attr_set(
            kernel::AttrTarget::Document,
            "hew.library",
            &key,
            attr_value,
        )
        .map_err(doc_err)?;
    }
    Ok(())
}

/// Converts a `u64` handle to a [`MaterialId`], or `None` if the sentinel
/// value `u64::MAX` is given (meaning "default / unpaint").
fn material_id_opt(handle: u64) -> Option<MaterialId> {
    if handle == u64::MAX {
        None
    } else {
        Some(MaterialId::from(KeyData::from_ffi(handle)))
    }
}

/// Flattens a [`recording::UvFrameRecorded`] into the 8-`f64` layout
/// [`Scene::set_face_uv_frame`] accepts (`sx sy sz tx ty tz u0 v0`), so replay
/// can drive the same public setter the original call went through.
fn uv_frame_recorded_to_vec(f: recording::UvFrameRecorded) -> Vec<f64> {
    vec![f.sx, f.sy, f.sz, f.tx, f.ty, f.tz, f.u0, f.v0]
}

fn sketch_id(handle: u64) -> SketchId {
    SketchId::from(KeyData::from_ffi(handle))
}

fn sketch_vertex_id(handle: u64) -> kernel::SketchVertexId {
    kernel::SketchVertexId::from(KeyData::from_ffi(handle))
}

fn object_id(handle: u64) -> ObjectId {
    ObjectId::from(KeyData::from_ffi(handle))
}

fn group_id(handle: u64) -> GroupId {
    GroupId::from(KeyData::from_ffi(handle))
}

fn instance_id(handle: u64) -> InstanceId {
    InstanceId::from(KeyData::from_ffi(handle))
}

fn component_id(handle: u64) -> ComponentId {
    ComponentId::from(KeyData::from_ffi(handle))
}

fn guide_id(handle: u64) -> GuideId {
    GuideId::from(KeyData::from_ffi(handle))
}

fn annotation_id(handle: u64) -> AnnotationId {
    AnnotationId::from(KeyData::from_ffi(handle))
}

/// Decode a 3-float FFI slice into a [`Point3`].
fn point3(p: &[f64]) -> Result<Point3, ApiError> {
    let p: &[f64; 3] = p
        .try_into()
        .map_err(|_| ApiError("BadPoint: point must be 3 floats [x,y,z]".to_string()))?;
    Ok(Point3::new(p[0], p[1], p[2]))
}

/// Decode a 3-float FFI slice into a [`kernel::Vec3`].
fn vec3(v: &[f64]) -> Result<kernel::Vec3, ApiError> {
    let v: &[f64; 3] = v
        .try_into()
        .map_err(|_| ApiError("BadVec: vector must be 3 floats [x,y,z]".to_string()))?;
    Ok(kernel::Vec3::new(v[0], v[1], v[2]))
}

/// Decode a 6-float FFI slice `[px,py,pz,nx,ny,nz]` (a point on the plane and
/// its normal) into a [`Plane`].
fn plane_slice(p: &[f64]) -> Result<Plane, ApiError> {
    let p: &[f64; 6] = p.try_into().map_err(|_| {
        ApiError("BadPlane: plane must be 6 floats [px,py,pz,nx,ny,nz]".to_string())
    })?;
    let point = Point3::new(p[0], p[1], p[2]);
    let normal = kernel::Vec3::new(p[3], p[4], p[5]);
    Plane::from_point_normal(point, normal)
        .map_err(|_| ApiError("DegeneratePlane: plane normal has no direction".to_string()))
}

/// Encode a [`Plane`] back to `[px,py,pz,nx,ny,nz]`.
fn plane_to_slice(plane: &Plane) -> Vec<f64> {
    let p = plane.point();
    let n = plane.normal();
    vec![p.x, p.y, p.z, n.x, n.y, n.z]
}

/// Decode an anchor's `(node_kind, node_id)` FFI pair into `Option<NodeId>`.
/// `node_kind < 0` means a free-floating anchor (no node) — [`node_id`]'s `0`/
/// `1`/`2` convention otherwise applies.
fn anchor_node(node_kind: i8, node_id_: u64) -> Result<Option<NodeId>, ApiError> {
    if node_kind < 0 {
        return Ok(None);
    }
    node_id(node_kind as u8, node_id_).map(Some)
}

/// Encode `Option<NodeId>` back to an FFI node-kind tag: `-1` = no node, else
/// [`node_id`]'s `0`/`1`/`2` convention.
fn anchor_node_kind_out(node: Option<NodeId>) -> i8 {
    match node {
        None => -1,
        Some(NodeId::Object(_)) => 0,
        Some(NodeId::Group(_)) => 1,
        Some(NodeId::Instance(_)) => 2,
    }
}

/// Encode `Option<NodeId>`'s handle, or `None` when the anchor is free.
fn anchor_node_id_out(node: Option<NodeId>) -> Option<u64> {
    match node {
        None => None,
        Some(NodeId::Object(id)) => Some(id.data().as_ffi()),
        Some(NodeId::Group(id)) => Some(id.data().as_ffi()),
        Some(NodeId::Instance(id)) => Some(id.data().as_ffi()),
    }
}

/// Decode a `RadialKind` FFI string: `"radius"` | `"diameter"`.
fn radial_kind(kind: &str) -> Result<RadialKind, ApiError> {
    match kind {
        "radius" => Ok(RadialKind::Radius),
        "diameter" => Ok(RadialKind::Diameter),
        _ => Err(ApiError(
            "BadRadialKind: kind must be \"radius\" or \"diameter\"".to_string(),
        )),
    }
}

fn radial_kind_str(kind: RadialKind) -> String {
    match kind {
        RadialKind::Radius => "radius".to_string(),
        RadialKind::Diameter => "diameter".to_string(),
    }
}

/// Decode a row-major 3×4 affine (12 floats) from the FFI boundary.
fn affine_transform(rows: &[f64]) -> Result<Transform, ApiError> {
    let rows: &[f64; 12] = rows.try_into().map_err(|_| {
        ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
    })?;
    Ok(Transform::from_affine(rows))
}

/// Decode a `(kind, id)` FFI pair into a [`NodeId`]. `kind` is `0` = object,
/// `1` = group, `2` = instance (matching [`NodeJs`]); any other value is
/// rejected.
fn node_id(kind: u8, id: u64) -> Result<NodeId, ApiError> {
    match kind {
        0 => Ok(NodeId::Object(object_id(id))),
        1 => Ok(NodeId::Group(group_id(id))),
        2 => Ok(NodeId::Instance(instance_id(id))),
        _ => Err(ApiError(
            "BadNodeKind: node kind must be 0 (object), 1 (group), or 2 (instance)".to_string(),
        )),
    }
}

/// Decode parallel `(kinds, ids)` FFI lists into [`NodeId`]s — the
/// multi-selection shape the `*_many` tag calls take. Mismatched lengths or
/// an empty list are boundary errors.
fn node_ids(kinds: &[u8], ids: &[u64]) -> Result<Vec<NodeId>, ApiError> {
    if kinds.len() != ids.len() {
        return Err(ApiError(
            "BadNodeList: kinds and ids must have the same length".to_string(),
        ));
    }
    if kinds.is_empty() {
        return Err(ApiError(
            "BadNodeList: at least one node is required".to_string(),
        ));
    }
    kinds
        .iter()
        .zip(ids)
        .map(|(&k, &i)| node_id(k, i))
        .collect()
}

/// The undo label of a multi-node tag batch ("Tag 3 objects" / "Untag 1
/// object").
fn tag_batch_label(verb: &str, n: usize) -> String {
    format!("{verb} {n} {}", if n == 1 { "object" } else { "objects" })
}

/// Decode an xyz `Box<[f64]>` boundary arg into a `[f64; 3]`, rejecting any
/// other length. `name` labels the field in the error (`camera_state`'s
/// `eye`/`target`/`up`, the same boundary shape `Scene::snap`'s `anchor`
/// checks inline).
fn triple(v: &[f64], name: &str) -> Result<[f64; 3], ApiError> {
    match *v {
        [x, y, z] => Ok([x, y, z]),
        _ => Err(ApiError(format!("BadVector: {name} must be an xyz triple"))),
    }
}

// ----------------------------------------------------------- library items
// Free functions: they read ITEM FILE BYTES, independent of any live Scene,
// so the library browser can list, badge, and thumbnail a folder of `.hew`
// files without touching the open document.

/// A manifest-only summary of `.hew` item bytes as a JSON string
/// ([`kernel::read_item_summary`] — entity counts, `hew.library` document
/// attrs, material swatch rows; never opens geometry buffers). Throws the
/// typed load error (`NotAContainer` / `UnsupportedVersion` /
/// `MalformedManifest`) for a file the browser must show as invalid.
#[wasm_bindgen]
pub fn read_item_summary_json(bytes: &[u8]) -> Result<String, JsError> {
    let summary = kernel::read_item_summary(bytes).map_err(|e| JsError::new(&api_err(&e, &e).0))?;
    serde_json::to_string(&summary).map_err(|e| JsError::new(&e.to_string()))
}

/// One named zip entry of `.hew` item bytes, verbatim
/// ([`kernel::read_item_asset`]) — the on-demand fetch for a summary's
/// `texture_asset` paths (a material item's swatch image).
#[wasm_bindgen]
pub fn read_item_asset(bytes: &[u8], path: &str) -> Result<Vec<u8>, JsError> {
    kernel::read_item_asset(bytes, path).map_err(|e| JsError::new(&api_err(&e, &e).0))
}

/// Renders `.hew` item bytes to a square PNG thumbnail from a fitted
/// isometric view ([`softrender::render_document_thumbnail`] — the same
/// deterministic rasterizer behind `hew.view.snapshot`). `undefined` when
/// the item has nothing visible to render (an honest "no thumbnail", e.g. a
/// material item); throws on bytes that don't load as a document.
#[wasm_bindgen]
pub fn render_item_thumbnail(bytes: &[u8], size: u32) -> Result<Option<Vec<u8>>, JsError> {
    let doc = Document::load(bytes).map_err(|e| JsError::new(&api_err(&e, &e).0))?;
    let size = size.clamp(16, 2048);
    Ok(softrender::render_document_thumbnail(&doc, size))
}

/// Rewrites a library item's `hew.library` metadata — the manage flows'
/// rename / re-keyword / collection edits. `meta_json` is a JSON object;
/// each key is written into the item's document attrs under `hew.library`
/// (a `null` value deletes that key), and a non-empty `"name"` also renames
/// the item's own display name (its single definition's, else its single
/// root's) so the file agrees with itself. Returns the item's full new
/// bytes; the input is untouched.
#[wasm_bindgen]
pub fn update_item_meta(bytes: &[u8], meta_json: &str) -> Result<Vec<u8>, JsError> {
    let mut item = Document::load(bytes).map_err(|e| JsError::new(&api_err(&e, &e).0))?;

    let parsed: serde_json::Value = serde_json::from_str(meta_json)
        .map_err(|e| JsError::new(&format!("BadLibraryMeta: not a JSON object: {e}")))?;
    let serde_json::Value::Object(map) = parsed else {
        return Err(JsError::new(
            "BadLibraryMeta: metadata must be a JSON object",
        ));
    };
    for (key, value) in &map {
        if value.is_null() {
            // Deleting a key that was never set is not an error the UI can
            // act on — tolerate it.
            let _ = item.attr_delete(kernel::AttrTarget::Document, "hew.library", Some(key));
            continue;
        }
        let attr_value = kernel::AttrValue::from_json(value)
            .map_err(|e| JsError::new(&format!("BadLibraryMeta: {key}: {e:?}")))?;
        item.attr_set(kernel::AttrTarget::Document, "hew.library", key, attr_value)
            .map_err(|e| JsError::new(&doc_err(e).0))?;
    }
    if let Some(serde_json::Value::String(name)) = map.get("name")
        && !name.trim().is_empty()
    {
        let cids = item.component_ids();
        if cids.len() == 1 {
            let _ = item.set_component_name(cids[0], Some(name.clone()));
        } else if let Some(&root) = item.top_level_nodes().first() {
            let _ = item.set_node_name(root, Some(name.clone()));
        }
    }
    Ok(item.save())
}

/// One flattened ghost mesh of a whole library item — every visible world
/// object and every instance placement baked into item coordinates. The
/// cursor-placement preview's raw material: one geometry, one draw call,
/// origin at the item's own origin.
#[wasm_bindgen]
pub struct GhostMeshJs {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    edge_positions: Vec<f32>,
    bbox: [f64; 6],
}

#[wasm_bindgen]
impl GhostMeshJs {
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    /// HARD edge segment endpoints (soft curved-wall seams excluded), for
    /// the ghost's stroke.
    pub fn edge_positions(&self) -> Vec<f32> {
        self.edge_positions.clone()
    }
    /// `[min_x, min_y, min_z, max_x, max_y, max_z]` in item coordinates.
    pub fn bbox(&self) -> Vec<f64> {
        self.bbox.to_vec()
    }
}

/// Tessellates `.hew` item bytes into one [`GhostMeshJs`] — a FREE function
/// on purpose: it never constructs a `Scene`, so it cannot leak scratch
/// mutations into the global session recording (`recording::record` is
/// process-global; a scratch `Scene::load` would corrupt an active
/// reproducer stream). Instance poses are baked; normals ride the pose's
/// linear part (renormalized — preview fidelity, not analytic truth).
#[wasm_bindgen]
pub fn item_ghost_mesh(bytes: &[u8]) -> Result<GhostMeshJs, JsError> {
    let doc = Document::load(bytes).map_err(|e| JsError::new(&api_err(&e, &e).0))?;
    let items = softrender::document_items(&doc);
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut edge_positions: Vec<f32> = Vec::new();
    for it in &items {
        let base = (positions.len() / 3) as u32;
        let m = &it.mesh;
        for i in (0..m.positions.len()).step_by(3) {
            let p = it.pose.apply_point(kernel::Point3::new(
                m.positions[i] as f64,
                m.positions[i + 1] as f64,
                m.positions[i + 2] as f64,
            ));
            positions.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32]);
            let n = it.pose.apply_vector(kernel::Vec3::new(
                m.normals[i] as f64,
                m.normals[i + 1] as f64,
                m.normals[i + 2] as f64,
            ));
            let n = n.normalized().unwrap_or(kernel::Vec3::new(0.0, 0.0, 1.0));
            normals.extend_from_slice(&[n.x as f32, n.y as f32, n.z as f32]);
        }
        indices.extend(m.indices.iter().map(|&ix| ix + base));
        for i in (0..m.edge_positions.len()).step_by(3) {
            let p = it.pose.apply_point(kernel::Point3::new(
                m.edge_positions[i] as f64,
                m.edge_positions[i + 1] as f64,
                m.edge_positions[i + 2] as f64,
            ));
            edge_positions.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32]);
        }
    }
    let (lo, hi) = softrender::document_bbox(&doc);
    Ok(GhostMeshJs {
        positions,
        normals,
        indices,
        edge_positions,
        bbox: [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z],
    })
}

// ------------------------------------------------------------------- nodes

/// A document-tree node across the FFI: a `kind` tag (`"object"` or
/// `"group"`) plus the opaque `u64` handle. The UI pairs these to address
/// nodes for selection, picking, and grouping without conflating the two
/// handle spaces (object and group slotmaps reuse bit patterns).
#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct NodeJs {
    kind: String,
    id: u64,
}

#[wasm_bindgen]
impl NodeJs {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u64 {
        self.id
    }
}

fn node_js(node: NodeId) -> NodeJs {
    match node {
        NodeId::Object(id) => NodeJs {
            kind: "object".to_string(),
            id: id.data().as_ffi(),
        },
        NodeId::Group(id) => NodeJs {
            kind: "group".to_string(),
            id: id.data().as_ffi(),
        },
        NodeId::Instance(id) => NodeJs {
            kind: "instance".to_string(),
            id: id.data().as_ffi(),
        },
    }
}

// ----------------------------------------------------------------- buffers

/// Flat-shaded render buffers for one Object (copied out per call,
/// docs/DEVELOPMENT.md B4).
#[wasm_bindgen]
pub struct MeshJs {
    mesh: RenderMesh,
    watertight: bool,
}

#[wasm_bindgen]
impl MeshJs {
    /// Triangle vertex positions, xyz per vertex, duplicated per face.
    pub fn positions(&self) -> Vec<f32> {
        self.mesh.positions.clone()
    }

    /// Per-vertex normals, constant across each face.
    pub fn normals(&self) -> Vec<f32> {
        self.mesh.normals.clone()
    }

    /// Triangle indices into `positions`, grouped by material (see `group_*`).
    pub fn indices(&self) -> Vec<u32> {
        self.mesh.indices.clone()
    }

    /// Per-vertex RGB colors (3 floats, range 0–1), parallel to `positions`.
    pub fn colors(&self) -> Vec<f32> {
        self.mesh.colors.clone()
    }

    /// Per-vertex UV coordinates (2 floats), parallel to `positions`.
    pub fn uvs(&self) -> Vec<f32> {
        self.mesh.uvs.clone()
    }

    /// Material handles for each index-buffer group (`u64::MAX` = default).
    pub fn group_material_ids(&self) -> Vec<u64> {
        self.mesh
            .groups
            .iter()
            .map(|g| match g.material {
                Some(id) => id.data().as_ffi(),
                None => u64::MAX,
            })
            .collect()
    }

    /// Start (index offset) of each index-buffer group, parallel to
    /// `group_material_ids`.
    pub fn group_starts(&self) -> Vec<u32> {
        self.mesh.groups.iter().map(|g| g.start).collect()
    }

    /// Triangle-index count of each index-buffer group, parallel to
    /// `group_material_ids`.
    pub fn group_counts(&self) -> Vec<u32> {
        self.mesh.groups.iter().map(|g| g.count).collect()
    }

    /// Line-segment endpoints (xyz pairs), one segment per unique **hard**
    /// edge (everything except interior facet seams of one curved wall).
    pub fn edge_positions(&self) -> Vec<f32> {
        self.mesh.edge_positions.clone()
    }

    /// Line-segment endpoints (xyz pairs) of the **soft** edges: interior
    /// seams between facets claiming the same analytic surface
    /// (the true-curves design stage 4). The viewport suppresses these
    /// — a cylinder reads as one smooth wall; exposed so alternative
    /// renderers or debug views can still draw them.
    pub fn soft_edge_positions(&self) -> Vec<f32> {
        self.mesh.soft_edge_positions.clone()
    }

    /// Whether the source Object encloses a volume.
    pub fn watertight(&self) -> bool {
        self.watertight
    }
}

/// A material palette entry, for the UI swatch panel.
#[wasm_bindgen]
pub struct MaterialJs {
    name: String,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
    has_texture: bool,
    world_w: f64,
    world_h: f64,
}

#[wasm_bindgen]
impl MaterialJs {
    /// Human-facing name.
    pub fn name(&self) -> String {
        self.name.clone()
    }
    /// Red channel (0–255).
    pub fn r(&self) -> u8 {
        self.r
    }
    /// Green channel (0–255).
    pub fn g(&self) -> u8 {
        self.g
    }
    /// Blue channel (0–255).
    pub fn b(&self) -> u8 {
        self.b
    }
    /// Alpha channel (0–255; 255 = opaque).
    pub fn a(&self) -> u8 {
        self.a
    }
    /// Whether this material carries an image texture.
    pub fn has_texture(&self) -> bool {
        self.has_texture
    }
    /// Texture world-size width (meters per tile). Meaningless when
    /// `has_texture` is false.
    pub fn world_w(&self) -> f64 {
        self.world_w
    }
    /// Texture world-size height (meters per tile). Meaningless when
    /// `has_texture` is false.
    pub fn world_h(&self) -> f64 {
        self.world_h
    }
}

/// A face's own material AND its object's base material — the eyedropper's
/// readback (paint-tool design §1). Both use the `u64::MAX` sentinel for
/// `None`, the same convention `paint_face`/`set_object_material` use on the
/// way in.
#[wasm_bindgen]
pub struct FaceMaterialJs {
    face: u64,
    object_default: u64,
}

#[wasm_bindgen]
impl FaceMaterialJs {
    /// The face's own material, or the sentinel if it carries none (in which
    /// case it resolves through `object_default`).
    pub fn face(&self) -> u64 {
        self.face
    }
    /// The face's object's base material, or the sentinel if the object has
    /// none either (in which case the face renders the plain default).
    pub fn object_default(&self) -> u64 {
        self.object_default
    }
}

// ----------------------------------------------------------------- reports

/// What a sketch segment insertion did (mirrors `kernel::SegmentAdded`).
#[wasm_bindgen]
pub struct SegmentAddedJs {
    inner: kernel::SegmentAdded,
}

#[wasm_bindgen]
impl SegmentAddedJs {
    /// Handles of newly created sketch edges.
    pub fn new_edges(&self) -> Vec<u64> {
        self.inner
            .new_edges
            .iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// Handles of regions that came into existence.
    pub fn regions_created(&self) -> Vec<u64> {
        self.inner
            .regions_created
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }

    /// Now-dead handles of regions invalidated by the insertion.
    pub fn regions_removed(&self) -> Vec<u64> {
        self.inner
            .regions_removed
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }
}

/// What a sketch edge removal did (mirrors `kernel::EdgeRemoved`).
#[wasm_bindgen]
pub struct EdgeRemovedJs {
    inner: kernel::EdgeRemoved,
}

#[wasm_bindgen]
impl EdgeRemovedJs {
    /// Handles of regions dissolved by the removal.
    pub fn regions_removed(&self) -> Vec<u64> {
        self.inner
            .regions_removed
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }
}

/// What a circle re-facet did (mirrors `kernel::CurveRefaceted`). The chain's
/// handle and its analytic circle are absent because re-faceting preserves
/// them — there is nothing to report about what did not change.
#[wasm_bindgen]
pub struct CurveRefacetedJs {
    inner: kernel::CurveRefaceted,
}

#[wasm_bindgen]
impl CurveRefacetedJs {
    /// The chain's new facet edges, in RING order (not id order). A caller
    /// holding a now-dead representative edge — a selection — re-points by
    /// feeding any of these to `sketch_curve_chain`, whose first element is
    /// the canonical representative.
    pub fn new_edges(&self) -> Vec<u64> {
        self.inner
            .new_edges
            .iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// The chain's old facet edges, all now dead.
    pub fn removed_edges(&self) -> Vec<u64> {
        self.inner
            .removed_edges
            .iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// Handles of regions that exist now but did not before.
    pub fn regions_created(&self) -> Vec<u64> {
        self.inner
            .regions_created
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }

    /// Handles of regions that existed before but do not now.
    pub fn regions_removed(&self) -> Vec<u64> {
        self.inner
            .regions_removed
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }
}

/// What a region offset did (mirrors `kernel::RegionOffsetAdded`).
#[wasm_bindgen]
pub struct RegionOffsetJs {
    inner: kernel::RegionOffsetAdded,
}

#[wasm_bindgen]
impl RegionOffsetJs {
    /// Handles of the offset loops' newly created sketch edges.
    pub fn new_edges(&self) -> Vec<u64> {
        self.inner
            .new_edges
            .iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// Handles of curve chains minted for the offset loops' analytic runs.
    pub fn new_curves(&self) -> Vec<u64> {
        self.inner
            .new_curves
            .iter()
            .map(|c| c.data().as_ffi())
            .collect()
    }

    /// Handles of regions that came into existence.
    pub fn regions_created(&self) -> Vec<u64> {
        self.inner
            .regions_created
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }

    /// Now-dead handles of regions the insertion invalidated.
    pub fn regions_removed(&self) -> Vec<u64> {
        self.inner
            .regions_removed
            .iter()
            .map(|r| r.data().as_ffi())
            .collect()
    }
}

/// What `push_pull` did. A normal translate carries a [`kernel::PushPullReport`]
/// (`inner`); a **through-cut** ( — push past the opposite wall becomes a
/// subtract) instead carries the resulting object handles in `through`, since
/// the source object was consumed and there is no single "moved face".
#[wasm_bindgen]
pub struct PushPullJs {
    inner: Option<kernel::PushPullReport>,
    through: Vec<u64>,
}

#[wasm_bindgen]
impl PushPullJs {
    /// The moved face in its new position (handle may differ from input). `0`
    /// for a through-cut, which has no moved face — check [`Self::is_through`].
    pub fn face(&self) -> u64 {
        self.inner
            .as_ref()
            .map(|r| r.face.data().as_ffi())
            .unwrap_or(0)
    }

    /// Whether this push/pull became a through-cut subtract: the source
    /// object was consumed and replaced by [`Self::result_objects`].
    pub fn is_through(&self) -> bool {
        self.inner.is_none()
    }

    /// The object handles produced by a through-cut (one normally; two or more
    /// if the cut severed the solid). Empty for a normal translate.
    pub fn result_objects(&self) -> Vec<u64> {
        self.through.clone()
    }
}

/// What `split_face` did (mirrors `kernel::FaceSplitReport`).
#[wasm_bindgen]
pub struct FaceSplitJs {
    inner: kernel::FaceSplitReport,
}

#[wasm_bindgen]
impl FaceSplitJs {
    /// The two faces replacing the input face.
    pub fn new_faces(&self) -> Vec<u64> {
        self.inner
            .new_faces
            .iter()
            .map(|f| f.data().as_ffi())
            .collect()
    }

    /// The cut-path edges; any of them merges the split back.
    pub fn new_edges(&self) -> Vec<u64> {
        self.inner
            .new_edges
            .iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }
}

/// What a document mutation touched (mirrors `kernel::DocChange`). Undo and
/// redo return this so the renderer can rebuild only the affected scene
/// graph nodes instead of falling back to a full-scene refresh.
#[wasm_bindgen]
pub struct DocChangeJs {
    inner: kernel::DocChange,
}

#[wasm_bindgen]
impl DocChangeJs {
    /// Objects whose geometry or visibility may have changed.
    pub fn objects_touched(&self) -> Vec<u64> {
        self.inner
            .objects_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Sketches whose contents or extrudable regions may have changed.
    pub fn sketches_touched(&self) -> Vec<u64> {
        self.inner
            .sketches_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Groups whose membership or visibility may have changed.
    pub fn groups_touched(&self) -> Vec<u64> {
        self.inner
            .groups_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Instances whose pose, definition, or visibility may have changed.
    pub fn instances_touched(&self) -> Vec<u64> {
        self.inner
            .instances_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Component definitions whose membership, geometry, or visibility may
    /// have changed.
    pub fn components_touched(&self) -> Vec<u64> {
        self.inner
            .components_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Guides whose geometry or visibility may have changed.
    pub fn guides_touched(&self) -> Vec<u64> {
        self.inner
            .guides_touched
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }
}

/// What `merge_faces` did (mirrors `kernel::FaceMergeReport`).
#[wasm_bindgen]
pub struct FaceMergeJs {
    inner: kernel::FaceMergeReport,
}

#[wasm_bindgen]
impl FaceMergeJs {
    /// The single face replacing the two inputs.
    pub fn merged_face(&self) -> u64 {
        self.inner.merged_face.data().as_ffi()
    }
}

/// A face picked by ray (mirrors the face `inference::SnapSource`). Used by
/// the push/pull tool, which needs the face under the cursor — not the
/// drawing snap, which prefers nearby endpoints/edges.
#[wasm_bindgen]
pub struct FacePickJs {
    object: u64,
    face: u64,
    instance: Option<u64>,
    depth: f64,
}

#[wasm_bindgen]
impl FacePickJs {
    /// Handle of the picked object (a world object, or the definition member
    /// when `instance` is set — the geometry that owns the face).
    pub fn object(&self) -> u64 {
        self.object
    }

    /// Handle of the picked face within that object.
    pub fn face(&self) -> u64 {
        self.face
    }

    /// Handle of the placing component instance, if the pick hit instanced
    /// geometry; `undefined` for a world object.
    pub fn instance(&self) -> Option<u64> {
        self.instance
    }

    /// Ray-distance (meters) from the ray origin to the hit. Lets a caller
    /// reject a solid beyond its render far plane — the raw world-ray pick
    /// otherwise reaches solids the user cannot see (a drag must never move
    /// off-screen geometry).
    pub fn depth(&self) -> f64 {
        self.depth
    }
}

/// A sketch vertex picked by ray (Phase D per-vertex edit). Carries the owning
/// sketch, the exact vertex handle to drag, and the vertex's world position so
/// the tool can seed the gesture without a second kernel round-trip.
#[wasm_bindgen]
pub struct SketchVertexPickJs {
    sketch: u64,
    vertex: u64,
    x: f64,
    y: f64,
    z: f64,
}

#[wasm_bindgen]
impl SketchVertexPickJs {
    /// Handle of the sketch owning the picked vertex.
    pub fn sketch(&self) -> u64 {
        self.sketch
    }

    /// Handle of the picked vertex within that sketch (pass to `move_sketch_vertex`).
    pub fn vertex(&self) -> u64 {
        self.vertex
    }

    /// Picked vertex X (meters).
    pub fn x(&self) -> f64 {
        self.x
    }

    /// Picked vertex Y (meters).
    pub fn y(&self) -> f64 {
        self.y
    }

    /// Picked vertex Z (meters).
    pub fn z(&self) -> f64 {
        self.z
    }
}

/// A sketch region picked by ray across all live sketches (see
/// [`Scene::pick_sketch_region`]): the owning sketch plus the region handle,
/// ready for `extrude_region`/`region_boundary` without a second round-trip.
#[wasm_bindgen]
pub struct SketchRegionPickJs {
    sketch: u64,
    region: u64,
    depth: f64,
}

#[wasm_bindgen]
impl SketchRegionPickJs {
    /// Handle of the sketch owning the picked region.
    pub fn sketch(&self) -> u64 {
        self.sketch
    }

    /// Handle of the picked region within that sketch.
    pub fn region(&self) -> u64 {
        self.region
    }

    /// Radial distance along the normalized pick ray.
    pub fn depth(&self) -> f64 {
        self.depth
    }
}

/// A picked sketch edge: the owning sketch plus the edge itself (see
/// `Scene::pick_sketch_edge`).
#[wasm_bindgen]
pub struct SketchEdgePickJs {
    sketch: u64,
    edge: u64,
    depth: f64,
}

#[wasm_bindgen]
impl SketchEdgePickJs {
    /// Handle of the sketch owning the picked edge.
    pub fn sketch(&self) -> u64 {
        self.sketch
    }

    /// Handle of the picked edge within that sketch.
    pub fn edge(&self) -> u64 {
        self.edge
    }

    /// Radial distance along the normalized pick ray.
    pub fn depth(&self) -> f64 {
        self.depth
    }
}

/// The camera's working view (mirrors `kernel::CameraState`; docs/design/
/// camera.md §5). Returned by [`Scene::camera_state`].
#[wasm_bindgen]
pub struct CameraStateJs {
    state: kernel::CameraState,
}

#[wasm_bindgen]
impl CameraStateJs {
    /// `"perspective"` or `"parallel"`.
    pub fn projection(&self) -> String {
        match self.state.projection {
            kernel::CameraProjection::Perspective => "perspective",
            kernel::CameraProjection::Parallel => "parallel",
        }
        .to_string()
    }

    /// Perspective vertical field of view, degrees (meaningless but still
    /// present under `"parallel"` — see `kernel::CameraState`).
    pub fn fov_deg(&self) -> f64 {
        self.state.fov_deg
    }

    pub fn eye_x(&self) -> f64 {
        self.state.eye.x
    }
    pub fn eye_y(&self) -> f64 {
        self.state.eye.y
    }
    pub fn eye_z(&self) -> f64 {
        self.state.eye.z
    }
    pub fn target_x(&self) -> f64 {
        self.state.target.x
    }
    pub fn target_y(&self) -> f64 {
        self.state.target.y
    }
    pub fn target_z(&self) -> f64 {
        self.state.target.z
    }
    pub fn up_x(&self) -> f64 {
        self.state.up.x
    }
    pub fn up_y(&self) -> f64 {
        self.state.up.y
    }
    pub fn up_z(&self) -> f64 {
        self.state.up.z
    }
}

/// A resolved snap (mirrors `inference::Snap`).
#[wasm_bindgen]
pub struct SnapJs {
    snap: inference::Snap,
}

#[wasm_bindgen]
impl SnapJs {
    /// Snapped X (meters).
    pub fn x(&self) -> f64 {
        self.snap.position.x
    }

    /// Snapped Y (meters).
    pub fn y(&self) -> f64 {
        self.snap.position.y
    }

    /// Snapped Z (meters).
    pub fn z(&self) -> f64 {
        self.snap.position.z
    }

    /// Snap kind for cue styling: "endpoint", "center", "quadrant",
    /// "midpoint", "intersection", "tangent", "on-edge", "on-face",
    /// "on-guide", "on-axis", "parallel", "perpendicular".
    pub fn kind(&self) -> String {
        match self.snap.kind {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Center => "center",
            SnapKind::Quadrant => "quadrant",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Intersection => "intersection",
            SnapKind::Tangent => "tangent",
            SnapKind::OnEdge => "on-edge",
            SnapKind::OnFace => "on-face",
            SnapKind::OnGuide => "on-guide",
            SnapKind::OnAxis => "on-axis",
            SnapKind::Parallel => "parallel",
            SnapKind::Perpendicular => "perpendicular",
        }
        .to_string()
    }

    /// Source Object handle, if the snap came from scene geometry (a world
    /// object, or a definition member when `instance` is set).
    pub fn object(&self) -> Option<u64> {
        self.snap.source.map(|s| s.object.data().as_ffi())
    }

    /// The placing component instance handle, if the snap came from instanced
    /// geometry; `undefined` otherwise.
    pub fn instance(&self) -> Option<u64> {
        self.snap
            .source
            .and_then(|s| s.instance)
            .map(|i| i.data().as_ffi())
    }

    /// Source element handle: within the object for Object provenance, or
    /// the sketch-edge handle for sketch provenance (see `element_kind`).
    pub fn element(&self) -> Option<u64> {
        self.snap
            .source
            .map(|s| match s.element {
                ElementRef::Vertex(v) => v.data().as_ffi(),
                ElementRef::Edge(e) => e.data().as_ffi(),
                ElementRef::Face(f) => f.data().as_ffi(),
            })
            .or_else(|| self.snap.sketch_source.map(|(_, e)| e.data().as_ffi()))
    }

    /// "vertex" | "edge" | "face" | "sketch-edge" | "sketch-region" |
    /// "sketch-curve" for interpreting `element` / `sketch_region` /
    /// `sketch_curve`.
    ///
    /// "sketch-curve" carries NO `element`: the analytic points of a drawn
    /// curve (a circle's center, its quadrants, an anchored tangent) belong
    /// to the curve CHAIN, not to any one facet edge — a center lies on no
    /// edge at all. Read `sketch_curve` (plus `sketch`) for it.
    pub fn element_kind(&self) -> Option<String> {
        self.snap
            .source
            .map(|s| {
                match s.element {
                    ElementRef::Vertex(_) => "vertex",
                    ElementRef::Edge(_) => "edge",
                    ElementRef::Face(_) => "face",
                }
                .to_string()
            })
            .or_else(|| self.snap.sketch_source.map(|_| "sketch-edge".to_string()))
            .or_else(|| {
                self.snap
                    .sketch_region_source
                    .map(|_| "sketch-region".to_string())
            })
            .or_else(|| {
                self.snap
                    .sketch_curve_source
                    .map(|_| "sketch-curve".to_string())
            })
    }

    /// The owning sketch handle when this snap derives from a committed sketch
    /// EDGE (`element_kind` == "sketch-edge", `element` is the edge), a sketch
    /// REGION fill (`element_kind` == "sketch-region", `sketch_region` is the
    /// region), or a drawn CURVE's analytic point (`element_kind` ==
    /// "sketch-curve", `sketch_curve` is the chain); `undefined` otherwise.
    pub fn sketch(&self) -> Option<u64> {
        self.snap
            .sketch_source
            .map(|(s, _)| s.data().as_ffi())
            .or_else(|| {
                self.snap
                    .sketch_region_source
                    .map(|(s, _)| s.data().as_ffi())
            })
            .or_else(|| {
                self.snap
                    .sketch_curve_source
                    .map(|(s, _)| s.data().as_ffi())
            })
    }

    /// The curve-chain handle when this snap is an analytic point of a drawn
    /// (unextruded) curve — a circle's or arc's exact center, one of its
    /// covered quadrant points, an anchored tangent point, or a regular
    /// polygon's drawn center (`element_kind` == "sketch-curve");
    /// `undefined` otherwise.
    ///
    /// Deliberately not folded into `element`: those points belong to the
    /// chain, and a center belongs to no edge at all, so reporting a
    /// stand-in edge handle would mislead every consumer that treats
    /// `element` as a real sketch edge (Tape Measure's parallel guides read
    /// it as a direction reference). Lets the Select tool resolve a
    /// center/quadrant click to the curve the point describes, matching what
    /// clicking the curve's own rim selects.
    pub fn sketch_curve(&self) -> Option<u64> {
        self.snap
            .sketch_curve_source
            .map(|(_, c)| c.data().as_ffi())
    }

    /// The region handle when this snap is on a drawn sketch region's fill
    /// (`element_kind` == "sketch-region"); `undefined` otherwise. Lets the
    /// Select tool's click resolve a region-fill snap to the exact region the
    /// occlusion-aware hover cue is already showing.
    pub fn sketch_region(&self) -> Option<u64> {
        self.snap
            .sketch_region_source
            .map(|(_, r)| r.data().as_ffi())
    }

    /// Inference direction (xyz) for directional snaps, for guide lines.
    pub fn direction(&self) -> Option<Vec<f64>> {
        self.snap.direction.map(|d| vec![d.x, d.y, d.z])
    }
}

// ------------------------------------------------------------------- scene

/// The WASM boundary shim over the authoritative [`Document`]
/// (docs/DEVELOPMENT.md B1). The model — Sketches, Objects, per-Object undo, and
/// the document command log — lives in the kernel `Document`. `Scene` keeps only
/// what the kernel may not depend on (DEVELOPMENT.md rule 1): the inference scene and
/// per-Object render-mesh caches. Every mutation delegates to `doc`, then
/// `reconcile`s those derived caches from the returned [`DocChange`].
#[wasm_bindgen]
pub struct Scene {
    doc: Document,
    inference: InferenceScene,
    /// Flat-shaded render buffers per Object, rebuilt lazily on demand and
    /// invalidated by `reconcile` when the Object changes or is hidden.
    mesh_cache: SecondaryMap<ObjectId, RenderMesh>,
    /// User-hidden world objects/instances (session-only, app-driven via
    /// [`Scene::set_hidden`]). Hidden geometry is dropped from the inference
    /// scene so `snap`/`pick_face` never report it — hiding makes a solid both
    /// invisible AND non-pickable/non-snappable. Not a kernel/document concept
    /// (not persisted); the kernel's own `hidden` flag is the undo tombstone.
    hidden_objects: std::collections::HashSet<ObjectId>,
    hidden_instances: std::collections::HashSet<InstanceId>,
    /// Definition sketches registered at the active edit instance's WORLD
    /// pose. Only one component placement is editable at a time; keeping this
    /// scoped avoids duplicate/ghost inference at sibling placements.
    active_inference_instance: Option<InstanceId>,
    active_inference_sketches: Vec<SketchId>,
    /// Open live API connections (docs/HEW_API.md §11.2), keyed by the id
    /// [`Scene::api_connection_open`] minted. Each holds its own
    /// `api::Connection` dispatch state (granted profile, hello/attach
    /// lifecycle) — session-only, like `hidden_objects` above, never
    /// persisted and never fed into the canonical serialization.
    api_connections: std::collections::HashMap<u32, api::Connection>,
    /// The next id [`Scene::api_connection_open`] mints. Monotonic for
    /// this `Scene`'s lifetime — wrapping is astronomically unreachable
    /// (2^32 live connections in one session) but wraps rather than
    /// panics if it ever were.
    next_api_conn_id: u32,
    /// A JSON-encoded [`live::ViewDirective`] left by the most recent
    /// [`Scene::api_dispatch`] call whose command was `hew.view.camera`,
    /// `hew.view.zoom_extents`, or `hew.view.units` and succeeded —
    /// `None` otherwise, including for a refused view command (which
    /// changes nothing, view included). [`Scene::take_pending_view_directive`]
    /// is how the live bridge collects it; see that method and
    /// `live::ViewDirective`'s doc comment for why this rides a separate
    /// accessor rather than the JSON-RPC reply itself.
    pending_view_directive: Option<String>,
}

impl Default for Scene {
    fn default() -> Scene {
        Scene::new()
    }
}

fn ground_plane() -> Plane {
    Plane::from_polygon(&[
        Point3::ORIGIN,
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

impl Scene {
    /// Reconciles the inference scene and render caches with the Document after
    /// a mutation. For each touched Object: drop its cached mesh, then register
    /// it with inference if it is now visible or remove it if it is hidden/gone
    /// (replace semantics, mirroring the Document's view). `sketches_touched`
    /// (Phase B): each touched sketch's live segments are re-registered with
    /// inference (replace semantics) so committed sketch geometry becomes
    /// snappable — see [`Scene::register_sketch`]. `guides_touched` (
    ///): each touched guide is re-registered with inference if still
    /// live (visible), or unregistered if hidden/gone — guides are now real
    /// snap targets ([`SnapKind::OnGuide`] / [`SnapKind::Endpoint`] for a
    /// guide point).
    fn reconcile(&mut self, change: &DocChange) {
        // Movable drawing axes (tool-parity design §4): re-push the
        // document's current frame into inference on EVERY reconciled
        // mutation, not just `set_axes` itself — `reconcile` is the
        // universal post-mutation hook (see the comment at the bottom of
        // this method), so this also re-syncs after undo/redo of an axes
        // move. Cheap (a few floats) and always correct, since it just
        // mirrors whatever `Document::axes` already holds.
        self.inference.set_axes_frame(self.doc.axes());

        // Objects: drop the cached mesh, then (re)register *world* objects with
        // inference at identity, or drop hidden/gone ones. Definition members
        // are not world inference candidates — they reach inference only
        // through their instances below — but their mesh cache is still dropped
        // so a definition edit re-tessellates the shared geometry.
        for &id in &change.objects_touched {
            self.mesh_cache.remove(id);
            // A user-hidden object stays out of inference even when a mutation
            // touches it, so snap/pick never resurrect hidden candidates.
            if self.doc.is_world_object(id) && !self.hidden_objects.contains(&id) {
                let object = self.doc.object(id).expect("world object is live");
                self.inference.add_object(id, object, &Transform::IDENTITY);
            } else {
                self.inference.remove_object(id);
                // A touched non-world id may be a definition member whose
                // shared geometry just changed — a definition edit reports
                // the member here and every instance of it in
                // `instances_touched`. Dropping the cached definition-space
                // candidates makes the instance re-registration below
                // re-extract from the new geometry; for a plain hidden or
                // deleted world object this is a no-op.
                self.inference.remove_def_member(id);
            }
        }
        // Instances: re-register each touched instance's definition geometry at
        // its pose (clearing any prior candidates first), or drop hidden/gone
        // ones. A definition edit lands every instance in `instances_touched`,
        // so shared-geometry changes propagate to all placements here.
        for &iid in &change.instances_touched {
            self.inference.remove_instance(iid);
            if !self.hidden_instances.contains(&iid) {
                self.register_instance(iid);
            }
        }
        // Sketches (Phase B): re-register each touched sketch's live segments.
        for &sid in &change.sketches_touched {
            if self.doc.sketch_owner_component(sid).is_some() {
                // Definition sketches have no single world placement. They
                // are picked through the active instance's posed, scoped
                // picker; registering local coordinates globally creates a
                // phantom sketch at the definition origin.
                self.inference.remove_sketch(sid);
            } else {
                self.register_sketch(sid);
            }
        }
        // Guides: re-register if still live, else unregister.
        // Guides have no session-only hidden-set (unlike objects/instances) —
        // `Document::guide` already returns `None` for a hidden/deleted guide.
        for &gid in &change.guides_touched {
            match self.doc.guide(gid) {
                Some(g) => self.inference.add_guide(gid, g),
                None => self.inference.remove_guide(gid),
            }
        }
        self.refresh_active_definition_inference();

        // Stamp the post-op canonical state_hash on the log stream. This
        // fires once per committed mutation (the universal post-mutation hook),
        // so the `kernel::op` event the kernel emitted at the start of the op and
        // this `kernel::cmd` event share a correlation id and bracket the command
        // with its name+params and its resulting state digest (docs/DEVELOPMENT.md).
        // The single full serialization per gesture is negligible vs. per-frame work.
        tracing::info!(
            target: "kernel::cmd",
            state_hash = self.doc.state_hash(),
            objects = change.objects_touched.len(),
        );

        self.torture_self_check(change);
    }

    /// Torture-mode (docs/DEVELOPMENT.md) re-tessellation self-check.
    /// When the Document's torture flag is on, re-tessellate every touched
    /// visible world object after the op and emit a loud `kernel::torture` error
    /// marker if any fails — so a flake surfaces at the **exact** op instead of
    /// as a downstream visual glitch three steps later. The kernel half (the
    /// topology validator on every op even in release WASM) lives in `Document`
    ///; this is the tessellate half, which can't live in the kernel
    /// (rule 1 — the kernel may not depend on `tessellate`). A no-op — one
    /// branch — when torture mode is off, the default.
    fn torture_self_check(&self, change: &DocChange) {
        if !self.doc.torture_mode() {
            return;
        }
        let palette = self.doc.materials();
        for &id in &change.objects_touched {
            if !self.doc.is_world_object(id) {
                continue;
            }
            let Some(object) = self.doc.object(id) else {
                continue;
            };
            if let Err(e) = tessellate(object, palette) {
                tracing::error!(
                    target: "kernel::torture",
                    object = ?id,
                    error = %e,
                    "torture: re-tessellation failed after op (flake surfaced at this op)",
                );
            }
        }
    }

    /// Registers a visible instance's definition members with inference, one
    /// lightweight placement each at the instance pose. A no-op for a
    /// hidden/stale instance (its placements were already cleared by the
    /// caller). The member's shared definition-space geometry is extracted
    /// only if inference doesn't hold it yet — `reconcile` invalidates it
    /// (`remove_def_member`) whenever a touched object turns out to be a
    /// definition member, so "already registered" always means "current",
    /// and registering N instances of one definition extracts its geometry
    /// exactly once.
    fn register_instance(&mut self, iid: InstanceId) {
        // A definition-owned member instance never registers directly: its
        // pose is def-local, not world — its geometry reaches inference
        // through the OUTER world instances' expanded placements below.
        // Touch lists from definition edits legitimately include member
        // instances, so this is a skip, not a bug.
        if !self.doc.instance_is_world(iid) {
            return;
        }
        let (Some(def), Some(pose)) = (self.doc.instance_def(iid), self.doc.instance_pose(iid))
        else {
            return;
        };
        // Nested definitions register every leaf placement at its fully
        // composed pose, all owned by the OUTERMOST instance (so a pick
        // still reports the world instance, and `remove_instance(iid)`
        // still drops the whole family at once). Deterministic expansion
        // order — placement order participates in candidate ranking
        // tie-breaks.
        for (m, local) in self.doc.expanded_def_placements(def) {
            if !self.inference.has_def_member(m) {
                let Some(object) = self.doc.object(m) else {
                    continue;
                };
                self.inference.set_def_member(m, object);
            }
            self.inference.add_placement(iid, m, &local.then(&pose));
        }
    }

    /// Re-registers sketch `id`'s current segments with
    /// inference (replace semantics — see [`inference::InferenceScene::add_sketch`]),
    /// or unregisters it if the sketch is unknown/gone. Shared by `reconcile`
    /// (each `sketches_touched` id) and the wasm-level call sites that mutate
    /// a sketch directly (`sketch_add_segment`/`sketch_remove_edge`), which
    /// bypass `Document::apply_*` and so never produce a `DocChange` —
    /// `sketches_touched` would always be empty for them, so they register
    /// the sketch at the call site instead (see those methods).
    fn register_sketch(&mut self, id: SketchId) {
        self.inference.remove_sketch(id);
        if self.doc.sketch_owner_component(id).is_some() {
            // A definition sketch has no world position of its own. Direct
            // mutation call sites use this helper without a DocChange, so
            // refresh the active instance's posed registration instead of
            // leaking definition-local candidates at the origin.
            self.refresh_active_definition_inference();
            return;
        }
        if let Some(segments) = Self::live_sketch_segments(&self.doc, id) {
            self.inference.add_sketch_edges(id, &segments);
        }
        if let Some(vertices) = Self::live_sketch_vertices(&self.doc, id) {
            self.inference.add_sketch_vertices(id, &vertices);
        }
        // Curve rims: a drawn circle/arc's exact center, quadrants, and
        // tangents snap BEFORE any extrusion (the sketch-level analogue of
        // a solid's analytic rims).
        //
        // Region faces: each closed region registers as a hoverable, occluding
        // face, so the cursor snaps on a drawn region's fill (OnFace) and it
        // hides geometry behind it exactly like a solid's face — instead of the
        // ray passing through to the ground/box beneath.
        if let Some(s) = self.doc.sketch(id) {
            self.inference.add_sketch_curves(id, &s.curve_rims());
            // Polygon centers ride the same re-registration. The two sets are
            // disjoint by construction (`curve_rims` is circles only,
            // `polygon_centers` is polygons only), so no center is offered
            // twice.
            self.inference
                .add_sketch_polygon_centers(id, &s.polygon_centers());
            self.inference
                .add_sketch_faces(id, &Self::live_sketch_faces(s));
        }
    }

    fn refresh_active_definition_inference(&mut self) {
        for sid in self.active_inference_sketches.drain(..) {
            self.inference.remove_sketch(sid);
        }
        let Some(instance) = self.active_inference_instance else {
            return;
        };
        if self.hidden_instances.contains(&instance) {
            return;
        }
        let (Some(component), Some(pose)) = (
            self.doc.instance_def(instance),
            self.doc.instance_pose(instance),
        ) else {
            self.active_inference_instance = None;
            return;
        };
        for sid in self.doc.def_member_sketches(component).unwrap_or_default() {
            let Some(source) = self.doc.sketch(sid) else {
                continue;
            };
            let segments: Vec<_> = source
                .edges()
                .iter()
                .map(|(eid, edge)| {
                    (
                        eid,
                        edge.curve,
                        pose.apply_point(source.vertices()[edge.from].position),
                        pose.apply_point(source.vertices()[edge.to].position),
                    )
                })
                .collect();
            let mut seen = std::collections::HashSet::new();
            let mut vertices = Vec::new();
            for edge in source.edges().values() {
                for vid in [edge.from, edge.to] {
                    if seen.insert(vid) {
                        vertices.push((vid, pose.apply_point(source.vertices()[vid].position)));
                    }
                }
            }
            self.inference.add_sketch_edges(sid, &segments);
            self.inference.add_sketch_vertices(sid, &vertices);
            if let Some(scale) = pose.similarity_scale() {
                let rims: Vec<SketchCurveRim> = source
                    .curve_rims()
                    .into_iter()
                    .filter_map(|rim| {
                        Some(SketchCurveRim {
                            curve: rim.curve,
                            center: pose.apply_point(rim.center),
                            axis: pose.apply_plane(&source.plane()).ok()?.normal(),
                            radius: rim.radius * scale,
                            basis_u: pose.apply_vector(rim.basis_u).normalized().ok()?,
                            basis_v: pose.apply_vector(rim.basis_v).normalized().ok()?,
                            coverage: rim.coverage,
                        })
                    })
                    .collect();
                self.inference.add_sketch_curves(sid, &rims);
            }
            let polygon_centers: Vec<_> = source
                .polygon_centers()
                .into_iter()
                .map(|(curve, center)| (curve, pose.apply_point(center)))
                .collect();
            self.inference
                .add_sketch_polygon_centers(sid, &polygon_centers);
            let Some(plane) = pose.apply_plane(&source.plane()).ok() else {
                continue;
            };
            let faces: Vec<SketchRegionFace> = source
                .regions()
                .iter()
                .map(|(region, loops)| {
                    let point = |vid: &kernel::SketchVertexId| {
                        pose.apply_point(source.vertices()[*vid].position)
                    };
                    SketchRegionFace {
                        region,
                        plane,
                        boundary: loops.outer.iter().map(point).collect(),
                        holes: loops
                            .holes
                            .iter()
                            .map(|hole| hole.iter().map(point).collect())
                            .collect(),
                    }
                })
                .collect();
            self.inference.add_sketch_faces(sid, &faces);
            self.active_inference_sketches.push(sid);
        }
    }

    /// Builds inference face candidates for sketch `s`'s closed regions in
    /// world space — the outer boundary and every hole, on the sketch plane
    /// (see [`inference::InferenceScene::add_sketch_faces`]). Region iteration
    /// is `SlotMap` slot order, which is deterministic (DEVELOPMENT.md §7), so
    /// the registered face order — and thus any OnFace tie-break — is
    /// reproducible.
    fn live_sketch_faces(s: &kernel::Sketch) -> Vec<SketchRegionFace> {
        let plane = s.plane();
        s.regions()
            .iter()
            .map(|(rid, r)| {
                let pos = |vid: &kernel::SketchVertexId| s.vertices()[*vid].position;
                SketchRegionFace {
                    region: rid,
                    plane,
                    boundary: r.outer.iter().map(pos).collect(),
                    holes: r
                        .holes
                        .iter()
                        .map(|h| h.iter().map(pos).collect())
                        .collect(),
                }
            })
            .collect()
    }

    /// Enumerates sketch `id`'s vertices as `(SketchVertexId, world position)`
    /// pairs for the per-vertex edit tool's picking, or `None` if the sketch
    /// is unknown/gone. Every sketch edge is real, visible geometry (Model D
    /// deleted the tombstone machinery), so every edge endpoint is pickable.
    fn live_sketch_vertices(
        doc: &Document,
        id: SketchId,
    ) -> Option<Vec<(kernel::SketchVertexId, Point3)>> {
        let s = doc.sketch(id)?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for edge in s.edges().values() {
            for vid in [edge.from, edge.to] {
                if seen.insert(vid) {
                    out.push((vid, s.vertices()[vid].position));
                }
            }
        }
        Some(out)
    }

    /// Enumerates sketch `id`'s edges as `(SketchEdgeId, world endpoints)`
    /// triples, or `None` if the sketch is unknown/gone. Shared by
    /// [`Scene::register_sketch`] and [`Scene::sketch_lines`];
    /// the edge id becomes snap provenance (Tape Measure parallel guides).
    #[allow(clippy::type_complexity)]
    fn live_sketch_segments(
        doc: &Document,
        id: SketchId,
    ) -> Option<Vec<(SketchEdgeId, Option<kernel::SketchCurveId>, Point3, Point3)>> {
        let s = doc.sketch(id)?;
        let mut out = Vec::with_capacity(s.edges().len());
        for (eid, edge) in s.edges() {
            let a = s.vertices()[edge.from].position;
            let b = s.vertices()[edge.to].position;
            // `edge.curve` is the drawn curve chain this facet belongs to
            // (`None` for a plain line / rectangle side), carried so a facet
            // vertex's Endpoint snap can name its curve for selection.
            out.push((eid, edge.curve, a, b));
        }
        Some(out)
    }

    fn apply_op(&mut self, handle: u64, op: KernelOp) -> Result<KernelOpReport, ApiError> {
        let (report, change) = self
            .doc
            .apply_object_op(object_id(handle), op)
            .map_err(doc_err)?;
        self.reconcile(&change);
        Ok(report)
    }
}

#[wasm_bindgen]
impl Scene {
    /// An empty scene.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Scene {
        Scene {
            doc: Document::new(),
            inference: InferenceScene::new(),
            mesh_cache: SecondaryMap::new(),
            hidden_objects: std::collections::HashSet::new(),
            hidden_instances: std::collections::HashSet::new(),
            active_inference_instance: None,
            active_inference_sketches: Vec::new(),
            api_connections: std::collections::HashMap::new(),
            next_api_conn_id: 0,
            pending_view_directive: None,
        }
    }

    /// Scope definition-sketch inference to the component instance currently
    /// being edited. `None` removes those posed candidates immediately.
    pub fn set_active_inference_instance(&mut self, instance: Option<u64>) {
        self.active_inference_instance = instance.map(instance_id);
        self.refresh_active_definition_inference();
    }

    // -------------------------------------------------------------- live API

    /// Opens a new live API connection against this scene's document,
    /// granted `Profile::App` (docs/HEW_API.md §10, §11.2). Returns the
    /// connection id the caller must pass to every subsequent
    /// [`Scene::api_dispatch`]/[`Scene::api_connection_close`] call. The
    /// Tauri shell opens exactly one of these per accepted socket
    /// connection, after that connection's `hello` token has already
    /// checked out (`crates/wasm-api` never sees the token itself).
    pub fn api_connection_open(&mut self) -> u32 {
        let id = self.next_api_conn_id;
        self.next_api_conn_id = self.next_api_conn_id.wrapping_add(1);
        let identity = format!("live:{id}");
        self.api_connections
            .insert(id, api::Connection::new(api::Profile::App, &identity));
        id
    }

    /// Closes a live API connection and drops its dispatch state. A no-op
    /// for an unknown `conn_id` (already closed, or never opened) — the
    /// Tauri shell's own socket close is the source of truth for when a
    /// connection ends; this just releases what `Scene` was holding for it.
    pub fn api_connection_close(&mut self, conn_id: u32) {
        self.api_connections.remove(&conn_id);
    }

    /// Dispatches one newline-delimited JSON-RPC frame (docs/HEW_API.md
    /// §4, §11.2) against `conn_id`'s connection and this scene's live
    /// document. Returns the serialized JSON-RPC response frame to send
    /// back, or `None` when the frame carried no reply: a malformed-JSON
    /// frame still gets an error response (`id: null`), but a
    /// well-formed client-originated *notification* (no `id`) is dropped
    /// unexecuted per §4.1 and — per JSON-RPC 2.0 — answers nothing at
    /// all, not even an error; an unknown `conn_id` also answers nothing,
    /// since there is no connection to reply through.
    ///
    /// A successful non-read-only dispatch reconciles the scene's derived
    /// caches (inference, mesh cache) exactly as every other `Scene`
    /// mutator does via [`Scene::reconcile`], so the change is visible in
    /// the live viewport immediately — docs/HEW_API.md §12's "in front of
    /// the user's eyes" requirement for `--live`. `crates/api` hands back
    /// no `DocChange` (it stays kernel-only per DEVELOPMENT.md rule 1, so
    /// it cannot know about `inference`/`tessellate` caches), so this
    /// does a full resync ([`Scene::full_resync`]) rather than
    /// `reconcile`'s touched-set-only update — more work, but still cheap
    /// for any Hew-sized document since it only re-registers inference
    /// candidates and never re-tessellates.
    pub fn api_dispatch(&mut self, conn_id: u32, frame: &str) -> Option<String> {
        // Cleared unconditionally up front, so every early-return path
        // below (parse error, unknown connection, a dropped notification)
        // leaves nothing stale for `take_pending_view_directive` to hand
        // out from a PREVIOUS dispatch — only a `hew.view.*` command that
        // both runs and succeeds in this very call sets it again below.
        self.pending_view_directive = None;
        let request: api::Request = match serde_json::from_str(frame) {
            Ok(r) => r,
            Err(e) => {
                let resp = api::Response::err(
                    None,
                    api::codes::PARSE_ERROR,
                    &format!("malformed JSON-RPC frame: {e}"),
                );
                return Some(serde_json::to_string(&resp).expect("Response serializes"));
            }
        };
        let Some(conn) = self.api_connections.get_mut(&conn_id) else {
            // Not a protocol case reachable by any well-behaved client —
            // an unknown conn_id is a Tauri-bridge bug, not a client
            // error. Best-effort typed reply (id echoed when the frame
            // had one) rather than a wasm panic.
            let resp = api::Response::err(
                request.id.clone(),
                api::codes::INTERNAL_FAULT,
                "unknown API connection",
            );
            return Some(serde_json::to_string(&resp).expect("Response serializes"));
        };
        // Snapshot the method's class before `dispatch` consumes the
        // request, so the mutation check below doesn't need `crates/api`
        // to hand anything more back than the JSON-RPC response it
        // already returns.
        let method = request.method.clone();
        // The registry's own answer, not a guess derived from the
        // command class: class governs transaction eligibility, and
        // hew.history.undo/redo are solitary for that reason while
        // plainly changing the document (see
        // `api::CommandDecl::mutates_document`).
        let mutates = conn
            .registry()
            .get(&method)
            .is_some_and(|c| c.mutates_document);
        let mut host = live::LiveHost::default();
        let outcome = conn.dispatch(&mut self.doc, &mut host, request);
        let api::DispatchOutcome::Reply(resp) = outcome else {
            // A dropped client-originated notification: no side effect,
            // no reply (§4.1).
            return None;
        };
        if resp.error.is_none() && mutates {
            // Capture and resync on ONE condition, so a recording can
            // never disagree with the viewport about what changed
            // (recording::RecordedCall::ApiDispatch). Without this, a
            // session co-authored by a user and an agent replays with
            // only half its history — the reproducer silently differs
            // from the document that broke.
            recording::record(recording::RecordedCall::ApiDispatch {
                frame: frame.to_string(),
            });
            self.full_resync();
        }
        if resp.error.is_none() {
            // `hew.view.camera`/`zoom_extents`/`units` are NOT `mutates`
            // (they change no document state — registry.rs's
            // `mutates_document = false`), so they never reach the branch
            // above; this is their own hand-off, unconditional on
            // `mutates` and gated only on success (a refused view command
            // changes nothing, view included, matching every other
            // refusal's untouched-document guarantee).
            self.pending_view_directive = host
                .directive
                .take()
                .map(|d| serde_json::to_string(&d).expect("ViewDirective serializes"));
        }
        Some(serde_json::to_string(&resp).expect("Response serializes"))
    }

    /// Takes (clears) the [`live::ViewDirective`] left by the dispatch this
    /// `Scene` just ran, JSON-encoded — `None` unless that dispatch's
    /// command was `hew.view.camera`, `hew.view.zoom_extents`, or
    /// `hew.view.units` and it succeeded. `app/src/api/liveBridge.ts` calls
    /// this immediately after every [`Scene::api_dispatch`], in the same
    /// synchronous JS task, and applies the directive itself: a `Camera`
    /// directive through the mounted `Viewport`'s existing
    /// `setCamera`/`setStandardView` (the same calls the Camera menu
    /// makes), a `ZoomExtents` directive through `Viewport.zoomExtents()`,
    /// and a `Units` directive through `app/src/settings/units.ts`'s
    /// `setLengthUnit` directly (not a viewport call). See
    /// [`live::ViewDirective`]'s doc comment for why this is a separate
    /// accessor rather than a field riding the JSON-RPC reply itself.
    ///
    /// "Takes" rather than "peeks": a directive is one-shot intent from one
    /// dispatch, not scene state, so leaving it behind would let a LATER,
    /// unrelated `api_dispatch` call (or a caller that simply forgets to
    /// check) accidentally replay it.
    pub fn take_pending_view_directive(&mut self) -> Option<String> {
        self.pending_view_directive.take()
    }

    /// Whether `method` is a document-mutating API command, straight
    /// from the registry (`crates/api`) — the authority on command
    /// class, and the same fact [`Scene::api_dispatch`] itself uses to
    /// decide whether to record and re-sync.
    ///
    /// Exposed so the webview's live bridge answers "does this dispatch
    /// need a re-render?" from the registry instead of guessing from the
    /// method's name. A future mutating command that did not match the
    /// naming convention would otherwise leave the user's viewport
    /// silently stale — showing a document that no longer exists.
    ///
    /// An unknown method answers `false`: it cannot mutate anything, it
    /// can only come back as a method-not-found error.
    pub fn api_method_mutates(&self, method: &str) -> bool {
        api::Registry::protocol_1()
            .get(method)
            .is_some_and(|c| c.mutates_document)
    }

    /// Fully rebuilds `inference` and clears `mesh_cache` from the
    /// Document's current state — the live-API-dispatch analogue of
    /// [`Scene::reconcile`], used by [`Scene::api_dispatch`] after any
    /// successful non-read-only dispatch. Every visible world object is
    /// registered at identity, every visible instance's definition
    /// members at its pose, every guide and sketch re-registered —
    /// respecting the session-only `hidden_objects`/`hidden_instances`
    /// sets, unlike [`Scene::load_core`]'s rebuild (which clears those
    /// sets first, since a whole-document replacement invalidates
    /// session-only view state that `api_dispatch`'s in-place mutations
    /// do not).
    fn full_resync(&mut self) {
        self.mesh_cache = SecondaryMap::new();
        self.inference = InferenceScene::new();
        self.inference.set_axes_frame(self.doc.axes());
        for id in self.doc.visible_object_ids() {
            if self.hidden_objects.contains(&id) {
                continue;
            }
            if let Some(object) = self.doc.object(id) {
                self.inference.add_object(id, object, &Transform::IDENTITY);
            }
        }
        for iid in self.doc.instance_ids() {
            if !self.hidden_instances.contains(&iid) {
                self.register_instance(iid);
            }
        }
        for id in self.doc.guide_ids() {
            if let Some(g) = self.doc.guide(id) {
                self.inference.add_guide(id, g);
            }
        }
        for sid in self.doc.sketch_ids() {
            self.register_sketch(sid);
        }
        self.refresh_active_definition_inference();
    }

    // ------------------------------------------------------------ sketching

    /// Adds a fresh, empty sketch on the ground plane (M1: the only sketch
    /// surface) and returns its handle. **Additive** — existing sketches are
    /// untouched, so independent coplanar shapes can coexist.
    pub fn begin_ground_sketch(&mut self) -> u64 {
        recording::record(recording::RecordedCall::BeginGroundSketch);
        self.doc.add_sketch(ground_plane()).data().as_ffi()
    }

    /// Adds a fresh, empty sketch on the plane through `(px,py,pz)` with
    /// normal `(nx,ny,nz)` — needn't be unit length, it is normalized here —
    /// and returns its handle. **Additive** — existing sketches are
    /// untouched. The one public-surface addition of the sketch-planes
    /// design (§5): mints the sketch an
    /// idle plane lock's first click targets when no cached sketch already
    /// lives on that plane.
    ///
    /// # Errors
    /// - `DegenerateVector` — `(nx,ny,nz)` is shorter than the kernel's
    ///   minimum normalize length (no direction — refuses rather than
    ///   guessing, DEVELOPMENT.md rule 4).
    pub fn begin_sketch_on_plane(
        &mut self,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
    ) -> Result<u64, ApiError> {
        let point = Point3::new(px, py, pz);
        let normal = kernel::Vec3::new(nx, ny, nz);
        let plane = Plane::from_point_normal(point, normal).map_err(|e| api_err(&e, &e))?;
        let handle = self.doc.add_sketch(plane).data().as_ffi();
        recording::record(recording::RecordedCall::BeginSketchOnPlane {
            px,
            py,
            pz,
            nx,
            ny,
            nz,
        });
        Ok(handle)
    }

    /// [`Scene::begin_sketch_on_plane`] for drawing INSIDE a component
    /// (component-edit-parity.md phase K1): `(px,py,pz,nx,ny,nz)` is the
    /// plane in WORLD space (wherever the user clicked/locked through the
    /// instance's rendered pose); the kernel maps it into `instance`'s
    /// definition-local space via the pose⁻¹ itself
    /// ([`kernel::Document::begin_sketch_on_plane_in_instance`]). The new
    /// sketch is shared: its regions extrude under every instance of that
    /// definition, not just this one.
    ///
    /// # Errors
    /// - `UnknownInstance` — `instance` is stale/hidden.
    /// - `DegenerateVector` — `(nx,ny,nz)` is shorter than the kernel's
    ///   minimum normalize length.
    /// - `Singular` — the instance's pose (or the mapped plane) failed to
    ///   invert; unreachable for a live instance in practice (see the kernel
    ///   doc comment).
    // Scalar xyz args are deliberate boundary ergonomics (docs/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn begin_sketch_on_plane_in_instance(
        &mut self,
        instance: u64,
        px: f64,
        py: f64,
        pz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
    ) -> Result<u64, ApiError> {
        let point = Point3::new(px, py, pz);
        let normal = kernel::Vec3::new(nx, ny, nz);
        let plane = Plane::from_point_normal(point, normal).map_err(|e| api_err(&e, &e))?;
        let (sid, _change) = self
            .doc
            .begin_sketch_on_plane_in_instance(instance_id(instance), plane)
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::BeginSketchOnPlaneInInstance {
            instance,
            px,
            py,
            pz,
            nx,
            ny,
            nz,
        });
        Ok(sid.data().as_ffi())
    }

    /// Opens a drawing gesture on `sketch`: everything drawn until
    /// `sketch_end_gesture` (a whole rectangle/circle/arc) lands on the undo
    /// stack as ONE step. The first gesture on a freshly-created sketch folds
    /// the sketch's creation into that step — undoing it removes the sketch.
    /// Tools bracket exactly their commit batch; gestures never nest.
    pub fn sketch_begin_gesture(&mut self, sketch: u64) -> Result<(), ApiError> {
        self.doc
            .begin_sketch_gesture(sketch_id(sketch))
            .map_err(doc_err)?;
        recording::record(recording::RecordedCall::SketchBeginGesture { sketch });
        Ok(())
    }

    /// Closes the open drawing gesture on `sketch`, pushing one undo step if
    /// anything changed (an unchanged gesture records nothing).
    pub fn sketch_end_gesture(&mut self, sketch: u64) -> Result<(), ApiError> {
        let change = self
            .doc
            .end_sketch_gesture(sketch_id(sketch))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SketchEndGesture { sketch });
        Ok(())
    }

    /// Drops the open drawing gesture (if any) without recording an undo step
    /// — the tool-cancel path. Safe to call when no gesture is open.
    pub fn sketch_cancel_gesture(&mut self) {
        if self.doc.cancel_sketch_gesture() {
            recording::record(recording::RecordedCall::SketchCancelGesture);
        }
    }

    /// Handles of every sketch in the document, for rendering all of them.
    pub fn sketch_ids(&self) -> Vec<u64> {
        self.doc
            .sketch_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Inserts a segment with full sticky semantics (see `kernel::sketch`).
    // Scalar xyz args are deliberate boundary ergonomics (docs/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn sketch_add_segment(
        &mut self,
        sketch: u64,
        ax: f64,
        ay: f64,
        az: f64,
        bx: f64,
        by: f64,
        bz: f64,
    ) -> Result<SegmentAddedJs, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s
            .add_segment(Point3::new(ax, ay, az), Point3::new(bx, by, bz))
            .map_err(|e| api_err(&e, &e))?;
        // `Sketch::add_segment` is called directly (not through
        // `Document::apply_*`), so no `DocChange`/`sketches_touched` exists
        // here; re-register with inference at the call site.
        self.register_sketch(sid);
        recording::record(recording::RecordedCall::SketchAddSegment {
            sketch,
            a: [ax, ay, az],
            b: [bx, by, bz],
        });
        Ok(SegmentAddedJs { inner: report })
    }

    /// Removes a sketch edge (eraser tool).
    pub fn sketch_remove_edge(
        &mut self,
        sketch: u64,
        edge: u64,
    ) -> Result<EdgeRemovedJs, ApiError> {
        let sid = sketch_id(sketch);
        let eid = SketchEdgeId::from(KeyData::from_ffi(edge));
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s.remove_edge(eid).map_err(|e| api_err(&e, &e))?;
        // Same rationale as `sketch_add_segment`: no `DocChange` here, so
        // re-register with inference at the call site.
        self.register_sketch(sid);
        recording::record(recording::RecordedCall::SketchRemoveEdge { sketch, edge });
        Ok(EdgeRemovedJs { inner: report })
    }

    /// The Offset tool's sketch commit: offsets `region`'s whole boundary
    /// (outer loop and every hole) by `distance` — positive grows the
    /// material, negative shrinks it — and inserts the offset loops as new
    /// sketch geometry through the ordinary sticky rules, so both the
    /// original and offset regions stay extrudable. Analytic arc runs come
    /// back as true concentric curves (`kernel::Sketch::offset_region`).
    /// Bracket with `sketch_begin_gesture`/`sketch_end_gesture` like any
    /// other drawing commit so the offset is one undo step.
    pub fn sketch_offset_region(
        &mut self,
        sketch: u64,
        region: u64,
        distance: f64,
    ) -> Result<RegionOffsetJs, ApiError> {
        let sid = sketch_id(sketch);
        let rid = SketchRegionId::from(KeyData::from_ffi(region));
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s
            .offset_region(rid, distance)
            .map_err(|e| api_err(&e, &e))?;
        // Same rationale as `sketch_add_segment`: no `DocChange` here, so
        // re-register with inference at the call site.
        self.register_sketch(sid);
        recording::record(recording::RecordedCall::SketchOffsetRegion {
            sketch,
            region,
            distance,
        });
        Ok(RegionOffsetJs { inner: report })
    }

    /// Pure preview of [`Scene::sketch_offset_region`]: the offset loops the
    /// commit would insert, without mutating anything. Encoded as
    /// `[loopCount, n₀, x,y,z × n₀, n₁, x,y,z × n₁, …]` (the outer loop's
    /// image first, then each hole's). Throws the same typed errors the
    /// commit would (`OffsetTooSmall`, `OffsetCollapsed`, `UnknownRegion`),
    /// so the tool can clamp or dim an invalid drag before committing.
    pub fn sketch_offset_region_preview(
        &self,
        sketch: u64,
        region: u64,
        distance: f64,
    ) -> Result<Vec<f64>, ApiError> {
        let s = self
            .doc
            .sketch(sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let rid = SketchRegionId::from(KeyData::from_ffi(region));
        let profile = s.profile(rid).map_err(|e| api_err(&e, &e))?;
        let off = kernel::offset_profile(&profile, distance).map_err(|e| api_err(&e, &e))?;
        let mut out: Vec<f64> = vec![(1 + off.holes.len()) as f64];
        for lp in std::iter::once(&off.outer).chain(off.holes.iter()) {
            out.push(lp.points.len() as f64);
            for p in &lp.points {
                out.extend([p.x, p.y, p.z]);
            }
        }
        Ok(out)
    }

    /// Opens a curve bracket on `sketch`: segments added until
    /// `sketch_end_curve` are ONE curve chain (an arc's or circle's facets),
    /// selected and deleted as a unit. Returns the curve handle.
    pub fn sketch_begin_curve(&mut self, sketch: u64) -> Result<u64, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let id = s.begin_curve();
        recording::record(recording::RecordedCall::SketchBeginCurve { sketch });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::sketch_begin_curve`] with the chain's analytic definition:
    /// the exact circle (center `cx, cy, cz` on the sketch plane, `radius`
    /// in meters) whose facets the bracketed segments approximate. The
    /// geometry is durable — it persists in the file format and survives
    /// sticky splits (the true-curves design). Returns the curve handle.
    pub fn sketch_begin_curve_with(
        &mut self,
        sketch: u64,
        cx: f64,
        cy: f64,
        cz: f64,
        radius: f64,
    ) -> Result<u64, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let id = s
            .begin_curve_with(kernel::CurveGeom {
                center: Point3::new(cx, cy, cz),
                radius,
            })
            .map_err(|e| api_err(&e, &e))?;
        recording::record(recording::RecordedCall::SketchBeginCurveWith {
            sketch,
            center: [cx, cy, cz],
            radius,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::sketch_begin_curve_with`] for a regular POLYGON: the circle
    /// is the polygon's circumcircle, not a curve its sides approximate.
    /// The chain still selects and deletes as one unit and still carries a
    /// durable center — but it offers no quadrant or tangent snaps, does not
    /// offset as concentric arcs, and does not sweep a cylindrical wall on
    /// extrusion, because none of those describe a polygon
    /// (`kernel::SketchCurveKind`). Returns the curve handle.
    pub fn sketch_begin_polygon_with(
        &mut self,
        sketch: u64,
        cx: f64,
        cy: f64,
        cz: f64,
        radius: f64,
    ) -> Result<u64, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let id = s
            .begin_curve_with_kind(
                kernel::CurveGeom {
                    center: Point3::new(cx, cy, cz),
                    radius,
                },
                kernel::SketchCurveKind::Polygon,
            )
            .map_err(|e| api_err(&e, &e))?;
        recording::record(recording::RecordedCall::SketchBeginPolygonWith {
            sketch,
            center: [cx, cy, cz],
            radius,
        });
        Ok(id.data().as_ffi())
    }

    /// Rebuilds drawn circle `curve`'s facets at `segments` density, in
    /// place: the chain keeps its handle and its exact circle, only the
    /// chords approximating it are replaced, starting at the angle the old
    /// ring started at so the circle does not visibly rotate. This is what
    /// editing a circle's segment count in Object Info does — a rebuild of
    /// the existing circle, not a setting for the next one drawn.
    ///
    /// Bracket with `sketch_begin_gesture`/`sketch_end_gesture` like any
    /// other drawing commit so the re-facet is one undo step.
    ///
    /// # Errors
    /// `UnknownSketch`; `UnknownCurve`; `CurveNotAnalytic` (the chain carries
    /// no circle); `CurveNotRefacetable` (not a whole, untouched circle — an
    /// arc, a partially erased circle, a polygon, a circle glued to other
    /// geometry, or one already consumed by an extrusion, which leaves the
    /// chain with no live edges); `SegmentsBelowFloor` / `SegmentsAboveCap`.
    /// On any error the sketch is untouched.
    pub fn sketch_refacet_curve(
        &mut self,
        sketch: u64,
        curve: u64,
        segments: u32,
    ) -> Result<CurveRefacetedJs, ApiError> {
        let sid = sketch_id(sketch);
        let cid = kernel::SketchCurveId::from(KeyData::from_ffi(curve));
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s
            .refacet_curve(cid, segments as usize)
            .map_err(|e| api_err(&e, &e))?;
        // Same rationale as `sketch_add_segment`: the sketch is mutated
        // directly, not through `Document::apply_*`, so no `DocChange` exists
        // — re-register with inference at the call site.
        self.register_sketch(sid);
        recording::record(recording::RecordedCall::SketchRefacetCurve {
            sketch,
            curve,
            segments,
        });
        Ok(CurveRefacetedJs { inner: report })
    }

    /// The analytic definition of curve chain `curve` in `sketch` as
    /// `[cx, cy, cz, radius]`, or `undefined` when the chain carries none
    /// (drawn before geometry capture, or a stale handle).
    ///
    /// **Circles only** — a polygon chain reports `undefined` here, matching
    /// `kernel::Sketch::curve_geom`, which is also how a caller tells the two
    /// apart: a chain with edges but no geom here is a polygon (or predates
    /// geometry capture), and neither is re-facetable.
    pub fn sketch_curve_geom(&self, sketch: u64, curve: u64) -> Option<Vec<f64>> {
        let s = self.doc.sketch(sketch_id(sketch))?;
        let cid = kernel::SketchCurveId::from(KeyData::from_ffi(curve));
        s.curve_geom(cid)
            .map(|g| vec![g.center.x, g.center.y, g.center.z, g.radius])
    }

    /// Closes the open curve bracket on `sketch` (no-op when none is open).
    pub fn sketch_end_curve(&mut self, sketch: u64) -> Result<(), ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch_mut(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        s.end_curve();
        recording::record(recording::RecordedCall::SketchEndCurve { sketch });
        Ok(())
    }

    /// The curve chain `edge` belongs to, or `undefined` for a plain line
    /// (or a stale handle).
    pub fn sketch_edge_curve(&self, sketch: u64, edge: u64) -> Option<u64> {
        let s = self.doc.sketch(sketch_id(sketch))?;
        s.edge_curve(SketchEdgeId::from(KeyData::from_ffi(edge)))
            .map(|c| c.data().as_ffi())
    }

    /// The maximal same-curve run containing `edge`, stopped at junctions
    /// with other geometry — the selection unit for a drawn arc/circle (see
    /// `Sketch::curve_chain_at`). Ascending by id, so the
    /// first element is a stable canonical representative for the chain.
    pub fn sketch_curve_chain(&self, sketch: u64, edge: u64) -> Vec<u64> {
        let sid = sketch_id(sketch);
        let Some(s) = self.doc.sketch(sid) else {
            return Vec::new();
        };
        s.curve_chain_at(SketchEdgeId::from(KeyData::from_ffi(edge)))
            .into_iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// Every edge of `curve` in `sketch`.
    pub fn sketch_curve_edges(&self, sketch: u64, curve: u64) -> Vec<u64> {
        let sid = sketch_id(sketch);
        let Some(s) = self.doc.sketch(sid) else {
            return Vec::new();
        };
        let cid = kernel::SketchCurveId::from(KeyData::from_ffi(curve));
        s.curve_edges(cid)
            .into_iter()
            .map(|e| e.data().as_ffi())
            .collect()
    }

    /// Handles of `sketch`'s islands — the outliner/selection units for
    /// free-standing geometry, in deterministic slotmap order.
    pub fn sketch_island_ids(&self, sketch: u64) -> Vec<u64> {
        let sid = sketch_id(sketch);
        let Some(s) = self.doc.sketch(sid) else {
            return Vec::new();
        };
        s.islands()
            .iter()
            .map(|(id, _)| id.data().as_ffi())
            .collect()
    }

    /// The island `edge` belongs to, or `undefined` for a stale handle.
    pub fn sketch_edge_island(&self, sketch: u64, edge: u64) -> Option<u64> {
        let s = self.doc.sketch(sketch_id(sketch))?;
        s.island_of_edge(SketchEdgeId::from(KeyData::from_ffi(edge)))
            .map(|i| i.data().as_ffi())
    }

    /// The island owning `region` (via any edge of its outer boundary), or
    /// `undefined` for stale handles.
    pub fn sketch_region_island(&self, sketch: u64, region: u64) -> Option<u64> {
        let sid = sketch_id(sketch);
        let s = self.doc.sketch(sid)?;
        let rid = kernel::SketchRegionId::from(KeyData::from_ffi(region));
        let r = s.regions().get(rid)?;
        let (a, b) = (r.outer.first().copied()?, r.outer.get(1).copied()?);
        let eid = s
            .edges()
            .iter()
            .find(|(_, e)| (e.from == a && e.to == b) || (e.from == b && e.to == a))
            .map(|(id, _)| id)?;
        s.island_of_edge(eid).map(|i| i.data().as_ffi())
    }

    /// The edges of `island` in `sketch` — what a per-shape Delete removes.
    pub fn sketch_island_edges(&self, sketch: u64, island: u64) -> Vec<u64> {
        let sid = sketch_id(sketch);
        let Some(s) = self.doc.sketch(sid) else {
            return Vec::new();
        };
        let iid = kernel::SketchIslandId::from(KeyData::from_ffi(island));
        let Some(isl) = s.islands().get(iid) else {
            return Vec::new();
        };
        isl.edges.iter().map(|e| e.data().as_ffi()).collect()
    }

    /// The edges of `island` as xyz segment-endpoint pairs, for the
    /// selection highlight and move ghost (the island analogue of
    /// `sketch_lines`).
    pub fn sketch_island_lines(&self, sketch: u64, island: u64) -> Result<Vec<f32>, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let iid = kernel::SketchIslandId::from(KeyData::from_ffi(island));
        let isl = s
            .islands()
            .get(iid)
            .ok_or_else(|| stale("UnknownIsland", "island"))?;
        let mut out = Vec::new();
        for &eid in &isl.edges {
            let e = s.edges()[eid];
            let a = s.vertices()[e.from].position;
            let b = s.vertices()[e.to].position;
            out.extend([a.x as f32, a.y as f32, a.z as f32]);
            out.extend([b.x as f32, b.y as f32, b.z as f32]);
        }
        Ok(out)
    }

    /// `transform_sketch_island`'s validation without the commit: `true` iff
    /// the move would be accepted AGAINST THE CURRENT STATE (mirrors
    /// [`kernel::Document::validate_transform_sketch_island`], including the
    /// out-of-plane arms). Batch movers validate every island first so one
    /// refusal aborts the whole gesture atomically. (Every island is movable
    /// in principle — extrusion deletes its scaffolding rather than hiding
    /// it, so no island secretly "backs" a solid.)
    ///
    /// This probe promises SUCCESS-equivalence, not MECHANISM-equivalence:
    /// the commit routes between its arms (in-plane bake / whole-sketch
    /// bake / detach) against COMMIT-TIME state, so in a batch that commits
    /// islands one by one, an earlier island's detach can reroute a later
    /// island's out-of-plane commit from "detach" to a whole-sketch bake.
    /// `true` still means "will succeed, soundly and undoably" — it does
    /// not fix WHICH arm runs or which handles stay stable afterwards;
    /// callers re-query handles after commits, as after any reshaping
    /// mutation.
    pub fn can_transform_sketch_island(&self, sketch: u64, island: u64, affine: &[f64]) -> bool {
        let Ok(rows) = <&[f64; 12]>::try_from(affine) else {
            return false;
        };
        let t = Transform::from_affine(rows);
        let sid = sketch_id(sketch);
        let iid = kernel::SketchIslandId::from(KeyData::from_ffi(island));
        self.doc
            .validate_transform_sketch_island(sid, iid, &t)
            .is_ok()
    }

    /// Rigidly move ONE island of a free-standing sketch (per-shape Move /
    /// Rotate / Scale; undoable). In-plane landings bake in place (a landing
    /// that would cross or merge other islands' geometry is refused with a
    /// typed error, never welded). An OUT-OF-PLANE transform — tipping a
    /// drawn shape upright — bakes whole-sketch when the island is the
    /// sketch's only one, and otherwise DETACHES the island into a new
    /// sketch on the transformed plane (a sketch is planar; see
    /// [`kernel::Document::transform_sketch_island`]). Curve chains keep
    /// their analytic identity through every arm. After a detach the island
    /// and its element handles are stale; re-query via `sketch_ids` /
    /// `sketch_island_ids`, as after any reshaping mutation.
    pub fn transform_sketch_island(
        &mut self,
        sketch: u64,
        island: u64,
        affine: &[f64],
    ) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let sid = sketch_id(sketch);
        let iid = kernel::SketchIslandId::from(KeyData::from_ffi(island));
        let change = self
            .doc
            .transform_sketch_island(sid, iid, &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformSketchIsland {
            sketch,
            island,
            affine: *rows,
        });
        Ok(())
    }

    /// Copy a SET of islands of a free-standing sketch onto ONE NEW sketch
    /// with the affine baked in, leaving the SOURCE untouched, and return the
    /// new sketch's handle. This is Move+Alt's OUT-OF-PLANE sketch copy: an
    /// in-plane copy replays into the source sketch (the UI's gesture-replay
    /// path), but a sketch is planar, so islands copied off its plane land on
    /// a sketch of their own — on the transformed plane. Passing every island
    /// of the sketch copies it whole; passing a subset copies just those.
    /// Keeping a sketch's islands together on one call is what preserves a
    /// region's HOLES (a hole boundary is its own island). Curve chains keep
    /// their analytic identity (a copied circle is a true circle,
    /// center-snappable). See [`kernel::Document::copy_sketch_islands`].
    ///
    /// Undoable as a single step regardless of island count: `scene_undo`
    /// hides the copy, `scene_redo` unhides it (the returned handle stays
    /// valid across both). The new sketch's island/element handles are fresh;
    /// re-query via `sketch_island_ids`, as after any reshaping mutation.
    pub fn copy_sketch_islands(
        &mut self,
        sketch: u64,
        islands: &[u64],
        affine: &[f64],
    ) -> Result<u64, ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let sid = sketch_id(sketch);
        let iids: Vec<kernel::SketchIslandId> = islands
            .iter()
            .map(|&i| kernel::SketchIslandId::from(KeyData::from_ffi(i)))
            .collect();
        let (copy, change) = self
            .doc
            .copy_sketch_islands(sid, &iids, &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::CopySketchIslands {
            sketch,
            islands: islands.to_vec(),
            affine: *rows,
        });
        Ok(copy.data().as_ffi())
    }

    /// A sketch's plane as `[px,py,pz, nx,ny,nz]` — a point on the plane
    /// plus its unit normal, the same shape `face_plane` returns — or
    /// `undefined` for a stale or hidden handle. Read-only. Lets a caller
    /// holding a cached sketch handle check WHERE the sketch lies before
    /// reusing it (the draw tools' shared ground-sketch cache: a
    /// whole-sketch transform keeps the handle live while moving the sketch
    /// off the ground plane, so handle liveness alone can't answer "will a
    /// ground point still land on this sketch?").
    pub fn sketch_plane(&self, sketch: u64) -> Option<Vec<f64>> {
        let s = self.doc.sketch(sketch_id(sketch))?;
        let plane = s.plane();
        let p = plane.point();
        let n = plane.normal();
        Some(vec![p.x, p.y, p.z, n.x, n.y, n.z])
    }

    /// All sketch edges as xyz line-segment endpoint pairs, for drawing.
    pub fn sketch_lines(&self, sketch: u64) -> Result<Vec<f32>, ApiError> {
        let segments = Self::live_sketch_segments(&self.doc, sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let mut out = Vec::with_capacity(segments.len() * 6);
        for (_eid, _cid, a, b) in segments {
            out.extend([a.x as f32, a.y as f32, a.z as f32]);
            out.extend([b.x as f32, b.y as f32, b.z as f32]);
        }
        Ok(out)
    }

    /// Handles of the sketch's current closed regions — every closed region
    /// extrudes (Hew's solids interpenetrate freely, so re-extruding occupied
    /// ground is allowed like any other overlap), so this is exactly
    /// `Document::extrudable_regions`.
    pub fn sketch_regions(&self, sketch: u64) -> Result<Vec<u64>, ApiError> {
        let sid = sketch_id(sketch);
        let s = self
            .doc
            .sketch(sid)
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        Ok(s.regions().keys().map(|r| r.data().as_ffi()).collect())
    }

    // --------------------------------------------------------------- solids

    /// THE solid-creating act (ARCHITECTURE.md): extrudes a closed sketch region
    /// into a new watertight Object and returns its handle.
    pub fn extrude_region(
        &mut self,
        sketch: u64,
        region: u64,
        distance: f64,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let (id, change) = self
            .doc
            .extrude_region(sketch_id(sketch), region_id, distance)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ExtrudeRegion {
            sketch,
            region,
            distance,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::extrude_region`] for a sketch owned by `instance`'s
    /// definition (component-edit-parity.md phase K1): births the solid as a
    /// member of the definition (seen by every instance at once) instead of
    /// a world Object. `distance` is a WORLD-space length, mapped through
    /// the instance pose's uniform scale — see
    /// [`kernel::Document::extrude_region_in_instance`] for the exact rule
    /// and its typed refusal under non-uniform scale.
    ///
    /// # Errors
    /// - `UnknownInstance` — `instance` is stale/hidden.
    /// - `UnknownSketch` — `sketch` is stale/hidden or not owned by
    ///   `instance`'s own definition.
    /// - `AmbiguousInstanceScale` — the instance's pose is not a similarity
    ///   (non-uniform scale); the typed `distance` cannot map unambiguously.
    pub fn extrude_region_in_instance(
        &mut self,
        instance: u64,
        sketch: u64,
        region: u64,
        distance: f64,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let (id, change) = self
            .doc
            .extrude_region_in_instance(
                instance_id(instance),
                sketch_id(sketch),
                region_id,
                distance,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ExtrudeRegionInInstance {
            instance,
            sketch,
            region,
            distance,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me along a chain of sketch edges (the follow-me design):
    /// sweeps the closed profile `region` of `sketch` along the path the
    /// `path_edges` of `path_sketch` form (a single connected chain, open or
    /// closed, in any order) into a new watertight Object and returns its
    /// handle. The profile region's scaffolding is consumed exactly as
    /// `extrude_region` consumes its outline (undo restores it); the path
    /// sketch is never touched.
    /// The optional trailing `stop_len` is a PARTIAL sweep: arc length
    /// from the seam at which the sweep is cut and capped (see
    /// [`kernel::Object::from_follow_me_to`]); NEGATIVE sweeps |stop| the
    /// other way around a closed loop (the drag direction);
    /// `undefined`/`None` sweeps the full path exactly as before.
    /// The optional trailing `group` births the swept solid INSIDE that
    /// group (design §2f) — the sweep committed while editing it;
    /// `undefined`/`None` births top-level exactly as before.
    pub fn follow_me_along_edges(
        &mut self,
        sketch: u64,
        region: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        stop_len: Option<f64>,
        group: Option<u64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let edges: Vec<SketchEdgeId> = path_edges
            .iter()
            .map(|&e| SketchEdgeId::from(KeyData::from_ffi(e)))
            .collect();
        let path = kernel::FollowMePath::SketchEdges {
            sketch: sketch_id(path_sketch),
            edges,
        };
        let sid = sketch_id(sketch);
        let (id, change) = match group {
            Some(g) => self.doc.follow_me_grouped(
                sid,
                region_id,
                &path,
                stop_len,
                GroupId::from(KeyData::from_ffi(g)),
            ),
            None => match stop_len {
                None => self.doc.follow_me(sid, region_id, &path),
                Some(stop) => self.doc.follow_me_to(sid, region_id, &path, stop),
            },
        }
        .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAlongEdges {
            sketch,
            region,
            path_sketch,
            path_edges,
            stop_len,
            group,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me around a solid face's outer boundary loop (crown molding
    /// around a tabletop): sweeps the closed profile `region` of `sketch`
    /// around the loop into a new watertight Object and returns its handle.
    /// The path solid is untouched — the sweep is a separate Object the
    /// user unions or subtracts explicitly.
    /// The optional trailing `stop_len` is a PARTIAL sweep, exactly as on
    /// [`Scene::follow_me_along_edges`].
    pub fn follow_me_around_face(
        &mut self,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
        group: Option<u64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let sid = sketch_id(sketch);
        let (id, change) = match group {
            Some(g) => self.doc.follow_me_grouped(
                sid,
                region_id,
                &path,
                stop_len,
                GroupId::from(KeyData::from_ffi(g)),
            ),
            None => match stop_len {
                None => self.doc.follow_me(sid, region_id, &path),
                Some(stop) => self.doc.follow_me_to(sid, region_id, &path, stop),
            },
        }
        .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAroundFace {
            sketch,
            region,
            path_object,
            path_face,
            stop_len,
            group,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::follow_me_around_face`] that MERGES the swept molding with
    /// the path's own solid in one gesture and ONE undo step (design §3b):
    /// a sweep overlapping the solid's interior carves it (Subtract), one
    /// that only rides its surface adds to it (Union) — decided by the
    /// boolean engine on clones. Returns the merged solid's handle; the
    /// path solid is consumed exactly like a boolean operand.
    pub fn follow_me_merged_around_face(
        &mut self,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me_merged(sketch_id(sketch), region_id, &path, stop_len)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeMergedAroundFace {
            sketch,
            region,
            path_object,
            path_face,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me with a solid FACE as the profile (design §3a), along a
    /// chain of sketch edges: the face's boundary (holes become tunnels)
    /// sweeps into a NEW object and the source solid is untouched. The
    /// optional trailing `stop_len` is the partial sweep, exactly as on
    /// [`Scene::follow_me_along_edges`].
    pub fn follow_me_face_along_edges(
        &mut self,
        profile_object: u64,
        profile_face: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let edges: Vec<SketchEdgeId> = path_edges
            .iter()
            .map(|&e| SketchEdgeId::from(KeyData::from_ffi(e)))
            .collect();
        let path = kernel::FollowMePath::SketchEdges {
            sketch: sketch_id(path_sketch),
            edges,
        };
        let (id, change) = self
            .doc
            .follow_me_face(
                object_id(profile_object),
                FaceId::from(KeyData::from_ffi(profile_face)),
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeFaceAlongEdges {
            profile_object,
            profile_face,
            path_sketch,
            path_edges,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me with a solid FACE as the profile around another face's
    /// outer boundary loop — [`Scene::follow_me_face_along_edges`]'s
    /// face-path sibling.
    pub fn follow_me_face_around_face(
        &mut self,
        profile_object: u64,
        profile_face: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me_face(
                object_id(profile_object),
                FaceId::from(KeyData::from_ffi(profile_face)),
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeFaceAroundFace {
            profile_object,
            profile_face,
            path_object,
            path_face,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// Ctrl/Cmd-modified push/pull (design tool-parity §2): straight-
    /// extrudes a solid face's own boundary into a NEW top-level object,
    /// leaving the source untouched — [`Scene::follow_me_face_along_edges`]'s
    /// straight-line sibling with no path/sweep involved. SketchUp's "leave
    /// original face" reinterpreted for Hew's freely-interpenetrating-solids
    /// model: the two solids end up sharing a coincident face, exactly like
    /// re-extruding occupied ground already produces a second coincident
    /// solid.
    ///
    /// # Errors
    /// `UnknownObject` — stale/hidden object, or a component-DEFINITION
    /// member (only world objects have a face to extrude at world scale);
    /// `UnknownFace` — stale face; `Extrude` — a degenerate profile/distance
    /// (matches `extrude_region`'s refusals). The document is untouched on
    /// error.
    pub fn extrude_face_as_new_object(
        &mut self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<u64, ApiError> {
        let (id, change) = self
            .doc
            .extrude_face_as_new_object(
                object_id(object),
                FaceId::from(KeyData::from_ffi(face)),
                distance,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ExtrudeFaceAsNewObject {
            object,
            face,
            distance,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me around a face reached THROUGH a component instance
    /// (design §2e): the definition face's loop rides the instance's pose
    /// into world space and the profile region sweeps around it. The
    /// instance and its definition are untouched; a reflected pose
    /// refuses typed.
    pub fn follow_me_around_instance_face(
        &mut self,
        sketch: u64,
        region: u64,
        instance: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::InstanceFaceLoop {
            instance: InstanceId::from(KeyData::from_ffi(instance)),
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let sid = sketch_id(sketch);
        let (id, change) = match stop_len {
            None => self.doc.follow_me(sid, region_id, &path),
            Some(stop) => self.doc.follow_me_to(sid, region_id, &path, stop),
        }
        .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAroundInstanceFace {
            sketch,
            region,
            instance,
            path_object,
            path_face,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    // ------------------------------------------------ follow me — in a component

    /// [`Scene::follow_me_along_edges`] inside `instance`'s definition
    /// (component-edit-parity.md phase K2): sweeps the closed profile
    /// `region` of a def-owned `sketch` along the chain `path_edges` of
    /// `path_sketch` — both must belong to `instance`'s OWN definition, or
    /// the kernel refuses typed (see
    /// [`kernel::Document::follow_me_in_instance`]). The swept solid is born
    /// as a new member of the definition; every instance sees it at once.
    /// `stop_len` is the same partial-sweep stop as the world variant, in
    /// WORLD units, mapped through the instance's pose.
    pub fn follow_me_along_edges_in_instance(
        &mut self,
        instance: u64,
        sketch: u64,
        region: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let edges: Vec<SketchEdgeId> = path_edges
            .iter()
            .map(|&e| SketchEdgeId::from(KeyData::from_ffi(e)))
            .collect();
        let path = kernel::FollowMePath::SketchEdges {
            sketch: sketch_id(path_sketch),
            edges,
        };
        let (id, change) = self
            .doc
            .follow_me_in_instance(
                instance_id(instance),
                sketch_id(sketch),
                region_id,
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAlongEdgesInInstance {
            instance,
            sketch,
            region,
            path_sketch,
            path_edges,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::follow_me_around_face`] inside `instance`'s definition: the
    /// path is a member face's outer boundary loop — `path_object` must be a
    /// member of the SAME definition as `sketch` (see
    /// [`kernel::Document::follow_me_in_instance`]).
    pub fn follow_me_around_face_in_instance(
        &mut self,
        instance: u64,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me_in_instance(
                instance_id(instance),
                sketch_id(sketch),
                region_id,
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAroundFaceInInstance {
            instance,
            sketch,
            region,
            path_object,
            path_face,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::follow_me_merged_around_face`] inside `instance`'s
    /// definition (see [`kernel::Document::follow_me_merged_in_instance`]):
    /// `path_object` must be a member of the SAME definition, and the swept
    /// molding merges with it in one gesture/undo step.
    pub fn follow_me_merged_around_face_in_instance(
        &mut self,
        instance: u64,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me_merged_in_instance(
                instance_id(instance),
                sketch_id(sketch),
                region_id,
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(
            recording::RecordedCall::FollowMeMergedAroundFaceInInstance {
                instance,
                sketch,
                region,
                path_object,
                path_face,
                stop_len,
            },
        );
        Ok(id.data().as_ffi())
    }

    /// [`Scene::follow_me_face_along_edges`] with the profile face on a
    /// MEMBER of `instance`'s definition (see
    /// [`kernel::Document::follow_me_face_in_instance`]): `path_sketch` must
    /// be a def-owned sketch of the SAME definition as `profile_object`.
    pub fn follow_me_face_along_edges_in_instance(
        &mut self,
        instance: u64,
        profile_object: u64,
        profile_face: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let edges: Vec<SketchEdgeId> = path_edges
            .iter()
            .map(|&e| SketchEdgeId::from(KeyData::from_ffi(e)))
            .collect();
        let path = kernel::FollowMePath::SketchEdges {
            sketch: sketch_id(path_sketch),
            edges,
        };
        let (id, change) = self
            .doc
            .follow_me_face_in_instance(
                instance_id(instance),
                object_id(profile_object),
                FaceId::from(KeyData::from_ffi(profile_face)),
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeFaceAlongEdgesInInstance {
            instance,
            profile_object,
            profile_face,
            path_sketch,
            path_edges,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::follow_me_face_around_face`] with the profile face on a
    /// MEMBER of `instance`'s definition — `path_object` must be a member of
    /// the SAME definition; when it is the profile object itself, the sweep
    /// merges with it exactly like the world variant (design §3b).
    pub fn follow_me_face_around_face_in_instance(
        &mut self,
        instance: u64,
        profile_object: u64,
        profile_face: u64,
        path_object: u64,
        path_face: u64,
        stop_len: Option<f64>,
    ) -> Result<u64, ApiError> {
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me_face_in_instance(
                instance_id(instance),
                object_id(profile_object),
                FaceId::from(KeyData::from_ffi(profile_face)),
                &path,
                stop_len,
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeFaceAroundFaceInInstance {
            instance,
            profile_object,
            profile_face,
            path_object,
            path_face,
            stop_len,
        });
        Ok(id.data().as_ffi())
    }

    /// Explicit combine (ARCHITECTURE.md): unions/subtracts/intersects two objects,
    /// consuming the operands into the returned result handle. `op` is
    /// 0 = union, 1 = subtract (`a - b`), 2 = intersect. Operands and result
    /// stay stable handles across undo/redo.
    pub fn boolean(&mut self, op: u8, a: u64, b: u64) -> Result<u64, ApiError> {
        let bop = match op {
            0 => BooleanOp::Union,
            1 => BooleanOp::Subtract,
            2 => BooleanOp::Intersect,
            _ => return Err(ApiError("BadOp: op must be 0, 1, or 2".to_string())),
        };
        let (id, change) = self
            .doc
            .boolean(bop, object_id(a), object_id(b))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::Boolean { op, a, b });
        Ok(id.data().as_ffi())
    }

    /// Explicit combine of two **tree nodes** — each a plain solid or a whole
    /// group, mixed freely (the group-ops design). `op` is 0 = union,
    /// 1 = subtract (`a - b`), 2 = intersect; `a_kind`/`b_kind` use the
    /// `duplicate_node` convention (0 = object, 1 = group; instances are
    /// refused typed by the kernel). Each operand's solids are composed
    /// (unioned) first, then `op` applies between the composites. Returns the
    /// result root: a single object when the result is one connected solid,
    /// or a result group (named from the operands) holding one object per
    /// disjoint piece. Operands are consumed; everything is one undo step
    /// with stable handles.
    pub fn boolean_nodes(
        &mut self,
        op: u8,
        a_kind: u8,
        a: u64,
        b_kind: u8,
        b: u64,
    ) -> Result<NodeJs, ApiError> {
        let bop = match op {
            0 => BooleanOp::Union,
            1 => BooleanOp::Subtract,
            2 => BooleanOp::Intersect,
            _ => return Err(ApiError("BadOp: op must be 0, 1, or 2".to_string())),
        };
        let na = node_id(a_kind, a)?;
        let nb = node_id(b_kind, b)?;
        let (root, change) = self.doc.boolean_nodes(bop, na, nb).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::BooleanNodes {
            op,
            a_kind,
            a,
            b_kind,
            b,
        });
        Ok(node_js(root))
    }

    /// Slice a watertight solid by a plane into two independent watertight
    /// solids. `plane` is 6 floats `[px,py,pz,nx,ny,nz]` — a point on the
    /// cut plane and its (unnormalized) normal. Returns the two new object
    /// handles `[positive, negative]`, the positive piece on the normal side;
    /// the source is consumed (hidden, undoable). Handles stay stable across
    /// undo/redo. Errors if the object is unknown/hidden, not a solid, or the
    /// cut is degenerate or misses the solid.
    pub fn slice_object(&mut self, object: u64, plane: &[f64]) -> Result<Vec<u64>, ApiError> {
        let p: &[f64; 6] = plane.try_into().map_err(|_| {
            ApiError("BadPlane: slice plane must be 6 floats [px,py,pz,nx,ny,nz]".to_string())
        })?;
        let point = Point3::new(p[0], p[1], p[2]);
        let normal = kernel::Vec3::new(p[3], p[4], p[5]);
        let plane = Plane::from_point_normal(point, normal)
            .map_err(|_| ApiError("DegeneratePlane: slice normal has no direction".to_string()))?;
        let ((a, b), change) = self
            .doc
            .slice_node(object_id(object), &plane)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SliceObject { object, plane: *p });
        Ok(vec![a.data().as_ffi(), b.data().as_ffi()])
    }

    /// Move/rotate/scale an object by baking an affine transform into its
    /// geometry (undoable). `affine` is a row-major 3×4 matrix (12 floats):
    /// `[m00 m01 m02 tx, m10 m11 m12 ty, m20 m21 m22 tz]`. The object handle is
    /// unchanged; the UI re-pulls its mesh afterward.
    pub fn transform_object(&mut self, object: u64, affine: &[f64]) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let change = self
            .doc
            .transform_object(object_id(object), &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformObject {
            object,
            affine: *rows,
        });
        Ok(())
    }

    /// Move/rotate/scale a free-standing sketch by baking an affine into its
    /// geometry (undoable; Phase D). Same row-major 3×4 12-float matrix as
    /// [`Scene::transform_object`]; the `SketchId` is unchanged. A sketch is a
    /// distinct FFI concept from a tree node ('s `NodeId` has no sketch
    /// variant), so this is dedicated rather than routing through a node path.
    ///
    /// # Errors
    /// - `BadAffine` — `affine` is not 12 floats.
    /// - `UnknownSketch` — stale or hidden (deleted) handle.
    /// - `Transform` — singular or orientation-flipping (e.g. negative scale).
    pub fn transform_sketch(&mut self, sketch: u64, affine: &[f64]) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let change = self
            .doc
            .transform_sketch(sketch_id(sketch), &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformSketch {
            sketch,
            affine: *rows,
        });
        Ok(())
    }

    /// Drag one vertex of a free-standing sketch to `(x, y, z)` (Phase D
    /// per-vertex edit; undoable). Topology-preserving — see
    /// [`kernel::Sketch::move_vertex`]: incident edges stretch, nothing splits
    /// or merges. The `SketchId`/`SketchVertexId` are unchanged.
    ///
    /// # Errors
    /// - `UnknownSketch` — stale or hidden (deleted) sketch.
    /// - `Sketch` — the move was refused (off-plane, would collapse an incident
    ///   edge, or would cross/merge geometry); the sketch is left untouched.
    pub fn move_sketch_vertex(
        &mut self,
        sketch: u64,
        vertex: u64,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<(), ApiError> {
        let change = self
            .doc
            .move_sketch_vertex(
                sketch_id(sketch),
                sketch_vertex_id(vertex),
                Point3::new(x, y, z),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::MoveSketchVertex {
            sketch,
            vertex,
            p: [x, y, z],
        });
        Ok(())
    }

    /// Deep-clone a node — Move+Option "copy" — placing the copy under the
    /// same parent, offset by `affine` (the same row-major 3×4 12-float matrix as
    /// [`Scene::transform_object`]). Returns the new node (always the **same kind**
    /// as the source `kind`/`id`): an Object/Group copy bakes the offset into
    /// fresh geometry; an Instance copy shares its definition at the offset pose.
    /// Undoable; the source is left untouched.
    pub fn duplicate_node(
        &mut self,
        kind: u8,
        id: u64,
        affine: &[f64],
    ) -> Result<NodeJs, ApiError> {
        let node = node_id(kind, id)?;
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let (new_node, change) = self.doc.duplicate_node(node, &t).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DuplicateNode {
            kind,
            id,
            affine: *rows,
        });
        Ok(node_js(new_node))
    }

    /// Deep-clone every node of a selection `count` times along `affine` —
    /// the Move tool's **array copy** (a Move+copy commit, or its ×N / /N
    /// refinement). Copy `k` of each node lands at `affine` composed `k`
    /// times, so a pure translation yields evenly spaced copies continuing
    /// along the same vector; a "/N" internal array is expressed by passing
    /// the full offset divided by `count`. Placement per kind matches
    /// [`Scene::duplicate_node`]: an Object/Group copy bakes fresh
    /// independent geometry, an Instance copy shares its definition at the
    /// composed pose. The whole array is **one undoable step**.
    ///
    /// `kinds`/`ids` are parallel arrays naming live sibling-or-not tree
    /// nodes (kind `0` = object, `1` = group, `2` = instance, as in
    /// [`Scene::group_nodes`]); `affine` is the same row-major 3×4 12-float
    /// matrix as [`Scene::transform_object`]. Returns the clone roots in
    /// creation order (every source's copy 1, then copy 2, …).
    ///
    /// # Errors
    /// - `BadNodeList` — `kinds` and `ids` differ in length or name a bad
    ///   kind; or the same node is listed twice (`DuplicateMember`).
    /// - `BadCount` — `count` is zero or exceeds [`MAX_ARRAY_COUNT`]. The cap
    ///   is enforced HERE, at the trust boundary, because recorded sessions
    ///   are plain JSON replayed through this method verbatim — a hand-edited
    ///   or corrupted `count` must fail typed, not hang the engine cloning
    ///   geometry.
    /// - `BadAffine` — `affine` is not 12 floats.
    /// - `EmptySelection` — nothing to duplicate.
    /// - `UnknownObject`/`UnknownGroup`/`UnknownInstance` — a stale or hidden
    ///   handle.
    /// - `Transform` — a singular `affine`, or one that reflects a baked
    ///   target.
    ///
    /// On error the document is untouched (partial clones are rolled back).
    pub fn duplicate_selection_array(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        affine: &[f64],
        count: u32,
    ) -> Result<Vec<NodeJs>, ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        if count > MAX_ARRAY_COUNT {
            return Err(ApiError::new(
                "BadCount",
                &format!("count must be between 1 and {MAX_ARRAY_COUNT}"),
            ));
        }
        let count_nz = std::num::NonZeroU32::new(count).ok_or_else(|| {
            ApiError::new(
                "BadCount",
                &format!("count must be between 1 and {MAX_ARRAY_COUNT}"),
            )
        })?;
        let nodes = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let (roots, change) = self
            .doc
            .duplicate_nodes_array(&nodes, &t, count_nz)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DuplicateSelectionArray {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
            affine: *rows,
            count,
        });
        Ok(roots.into_iter().map(node_js).collect())
    }

    /// The ceiling [`Scene::duplicate_selection_array`] enforces on `count`
    /// ([`MAX_ARRAY_COUNT`]). The UI reads its own pre-check limit from here
    /// so the two caps cannot drift.
    pub fn max_array_count(&self) -> u32 {
        MAX_ARRAY_COUNT
    }

    /// A monotonic token identifying the document's undo-stack state: changes
    /// on every committed mutation, every undo, and every redo — never on
    /// view-state toggles (tag visibility, user-hide). An unchanged value
    /// proves the last action a caller committed is still the top of the
    /// undo stack, so a [`Scene::scene_undo`] will retract exactly it. This
    /// is what the Move tool's array refinement checks before its retracting
    /// undo; [`Scene::state_hash`] cannot stand in (content identity is not
    /// history identity — see [`kernel::Document::history_generation`]).
    pub fn history_generation(&self) -> u64 {
        self.doc.history_generation()
    }

    /// Handles of all currently visible Objects in the scene (undone
    /// creations are hidden, not listed).
    pub fn object_ids(&self) -> Vec<u64> {
        self.doc
            .visible_object_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    // ---------------------------------------------------------------- groups

    /// Non-destructively groups sibling nodes into a new merge group,
    /// returning its handle. Unlike `boolean`, no geometry is welded and no
    /// member is consumed. `kinds`/`ids` are parallel arrays describing each
    /// member node (kind `0` = object, `1` = group, `2` = instance); they
    /// must be the same length, name live sibling nodes, and not repeat.
    pub fn group_nodes(&mut self, kinds: &[u8], ids: &[u64]) -> Result<u64, ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        let members = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let (group, change) = self.doc.group_nodes(&members).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::GroupNodes {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
        });
        Ok(group.data().as_ffi())
    }

    /// Dissolves a group, returning its members to the group's own parent
    /// (inverse of `group_nodes`). The members keep their geometry and handles.
    pub fn ungroup(&mut self, group: u64) -> Result<(), ApiError> {
        let change = self.doc.ungroup(group_id(group)).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::Ungroup { group });
        Ok(())
    }

    /// Removes a whole tree node — Object, Group, or Instance — from the scene
    ///. Tombstone, not a real delete: undoable, and the handle
    /// stays valid for redo. `kind` is `0` = object, `1` = group, `2` =
    /// instance. Deleting a group hides its whole subtree in one step;
    /// deleting an instance never touches its shared definition or sibling
    /// instances. Whole-node delete only — single-face/edge delete and guide
    /// selections are out of scope here ( routes guides to
    /// `delete_guide`/`delete_all_guides`).
    pub fn delete_node(&mut self, kind: u8, id: u64) -> Result<(), ApiError> {
        let node = node_id(kind, id)?;
        let change = self.doc.delete_node(node).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteNode { kind, id });
        Ok(())
    }

    /// Deletes (hides) one free-standing sketch in one undoable step —
    /// whole-sketch granularity, mirroring `delete_guide`. The handle stays
    /// valid for redo. A sketch is a distinct FFI concept from a tree node
    /// ('s `NodeId` has no sketch variant), so this is a dedicated method
    /// rather than routing through `delete_node`.
    ///
    /// # Errors
    /// - `UnknownSketch` — stale, already-hidden, or foreign handle.
    pub fn delete_sketch(&mut self, sketch: u64) -> Result<(), ApiError> {
        let change = self.doc.delete_sketch(sketch_id(sketch)).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteSketch { sketch });
        Ok(())
    }

    /// Move/rotate/scale a group by baking the affine into every leaf object
    /// beneath it (undoable). `affine` is the same row-major 3×4
    /// 12-float matrix as [`Scene::transform_object`].
    pub fn transform_group(&mut self, group: u64, affine: &[f64]) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let change = self
            .doc
            .transform_group(group_id(group), &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformGroup {
            group,
            affine: *rows,
        });
        Ok(())
    }

    /// Move/rotate/scale a whole mixed selection — objects, groups,
    /// instances, and free-standing sketches — as **one undoable step**
    /// (select-all → Move). `kinds`/`ids` are parallel arrays naming the
    /// tree nodes (kind `0` = object, `1` = group, `2` = instance, as in
    /// [`Scene::group_nodes`]); `sketches` lists free-standing sketch
    /// handles (a distinct FFI concept from tree nodes, as in
    /// [`Scene::delete_sketch`]). Objects and group leaves bake the affine;
    /// instances compose it into their pose; nested/duplicate listings
    /// transform once. `affine` is the same row-major 3×4 12-float matrix as
    /// [`Scene::transform_object`].
    ///
    /// # Errors
    /// - `BadNodeList` — `kinds` and `ids` differ in length or name a bad kind.
    /// - `BadAffine` — `affine` is not 12 floats.
    /// - `EmptySelection` — nothing to transform.
    /// - `UnknownObject`/`UnknownGroup`/`UnknownInstance`/`UnknownSketch` — a
    ///   stale or hidden handle; the document is untouched.
    /// - `Transform` — singular affine, or one that reflects a baked target.
    pub fn transform_selection(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        sketches: &[u64],
        affine: &[f64],
    ) -> Result<(), ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        let nodes = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let sketch_ids: Vec<_> = sketches.iter().map(|&s| sketch_id(s)).collect();
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let change = self
            .doc
            .transform_selection(&nodes, &sketch_ids, &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformSelection {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
            sketches: sketches.to_vec(),
            affine: *rows,
        });
        Ok(())
    }

    /// Uniformly rescale the WHOLE document about the world origin (design
    /// tool-parity §3 — the Tape Measure "resize the model" flow): every
    /// world object, every free-standing sketch, every construction guide,
    /// and every component instance's placing pose scale by `factor`;
    /// component DEFINITIONS are never touched (see
    /// [`kernel::Document::rescale_document`] for the full contract). ONE
    /// undo step; undo/redo are bit-exact (never a recomputed `1/factor`).
    ///
    /// # Errors
    /// `InvalidRescaleFactor` — `factor` is non-finite, zero, or negative;
    /// the document is untouched.
    pub fn rescale_document(&mut self, factor: f64) -> Result<(), ApiError> {
        let change = self.doc.rescale_document(factor).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RescaleDocument { factor });
        Ok(())
    }

    /// The current movable drawing axes (tool-parity design §4): 12 floats
    /// `[ox,oy,oz, xx,xy,xz, yx,yy,yz, zx,zy,zz]` — origin, red, green, and
    /// the derived blue direction. World identity
    /// (`[0,0,0, 1,0,0, 0,1,0, 0,0,1]`) until [`Scene::set_axes`] moves it.
    pub fn axes(&self) -> Vec<f64> {
        let frame = self.doc.axes();
        let z = frame.z();
        vec![
            frame.origin.x,
            frame.origin.y,
            frame.origin.z,
            frame.x.x,
            frame.x.y,
            frame.x.z,
            frame.y.x,
            frame.y.y,
            frame.y.z,
            z.x,
            z.y,
            z.z,
        ]
    }

    /// Moves the document's drawing axes to the frame spanned by origin
    /// `(ox,oy,oz)`, red direction `(xx,xy,xz)`, and green direction
    /// `(yx,yy,yz)` (tool-parity design §4 — the Axes tool's three-click
    /// commit; Reset Axes passes world identity's own components:
    /// `(0,0,0, 1,0,0, 0,1,0)`). The blue axis is always derived (`x × y`),
    /// never accepted from the caller. One undo step.
    ///
    /// # Errors
    /// - `NonFinite` — a non-finite origin or direction component.
    /// - `NonOrthonormal` — the red/green directions are not each unit
    ///   length, or not mutually perpendicular. Nothing is silently
    ///   renormalized or reoriented.
    #[allow(clippy::too_many_arguments)]
    pub fn set_axes(
        &mut self,
        ox: f64,
        oy: f64,
        oz: f64,
        xx: f64,
        xy: f64,
        xz: f64,
        yx: f64,
        yy: f64,
        yz: f64,
    ) -> Result<(), ApiError> {
        let change = self
            .doc
            .set_axes(
                Point3::new(ox, oy, oz),
                kernel::Vec3::new(xx, xy, xz),
                kernel::Vec3::new(yx, yy, yz),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetAxes {
            origin: [ox, oy, oz],
            x: [xx, xy, xz],
            y: [yx, yy, yz],
        });
        Ok(())
    }

    /// Handles of all currently visible groups (ungrouped groups are hidden,
    /// not listed).
    pub fn group_ids(&self) -> Vec<u64> {
        self.doc
            .group_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// The direct members of a visible group, in order. Empty if the group is
    /// stale or hidden.
    pub fn group_members(&self, group: u64) -> Vec<NodeJs> {
        self.doc
            .group_members(group_id(group))
            .unwrap_or_default()
            .into_iter()
            .map(node_js)
            .collect()
    }

    /// The visible top-level nodes (parent `None`) — the unit of top-level
    /// selection and picking.
    pub fn top_level_nodes(&self) -> Vec<NodeJs> {
        self.doc
            .top_level_nodes()
            .into_iter()
            .map(node_js)
            .collect()
    }

    /// The containing group handle of a node, or `None` if it is top-level (or
    /// the node handle is stale/hidden). `kind` is `0` = object, `1` = group.
    pub fn node_parent(&self, kind: u8, id: u64) -> Result<Option<u64>, ApiError> {
        let node = node_id(kind, id)?;
        Ok(self.doc.node_parent(node).map(|g| g.data().as_ffi()))
    }

    /// Every visible leaf Object beneath a node (the node itself if it is an
    /// object), recursively — the meshes that move with a group transform and
    /// stay lit when the node is the active editing context. `kind` is
    /// `0` = object, `1` = group.
    pub fn node_leaf_objects(&self, kind: u8, id: u64) -> Result<Vec<u64>, ApiError> {
        let node = node_id(kind, id)?;
        Ok(self
            .doc
            .leaf_objects_under(node)
            .iter()
            .map(|o| o.data().as_ffi())
            .collect())
    }

    // ----------------------------------------------- components & instances

    /// "Make Component": folds a selection of sibling nodes into one
    /// shared definition plus an identity-posed instance in their place, and
    /// returns the **instance** handle (the def is reachable via
    /// [`Scene::instance_def`]). `kinds`/`ids` are parallel arrays (kind `0` =
    /// object, `1` = group, `2` = instance), same as [`Scene::group_nodes`].
    pub fn make_component(&mut self, kinds: &[u8], ids: &[u64]) -> Result<u64, ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        let members = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let (_component, instance, change) = self.doc.make_component(&members).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::MakeComponent {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
        });
        Ok(instance.data().as_ffi())
    }

    /// Stamps another instance of `component` at `affine` (row-major 3×4, 12
    /// floats), returning the new instance handle. Reflection and
    /// non-uniform scale are allowed; a singular pose errors.
    pub fn place_instance(&mut self, component: u64, affine: &[f64]) -> Result<u64, ApiError> {
        let pose = affine_transform(affine)?;
        let (instance, change) = self
            .doc
            .place_instance(component_id(component), pose)
            .map_err(doc_err)?;
        self.reconcile(&change);
        // affine_transform validated the length, so this cannot fail.
        let mut rec_affine = [0.0f64; 12];
        rec_affine.copy_from_slice(affine);
        recording::record(recording::RecordedCall::PlaceInstance {
            component,
            affine: rec_affine,
        });
        Ok(instance.data().as_ffi())
    }

    /// The 3D Text placement pipeline's atomic tail (docs/design/3d-text.md).
    /// Call this immediately after closing (`sketch_end_gesture`) the
    /// sketch-drawing gesture that injected a text run's glyph outlines as
    /// edges into `sketch`, with nothing else committed in between — the
    /// caller contract [`kernel::Document::place_text`] verifies.
    ///
    /// `regions` are the sketch's closed regions to extrude — the FILL
    /// ones the app selected (the font's own nonzero-winding rule decides
    /// which of the resolver's regions are glyph material versus a
    /// counter's own bare interior; see [`Scene::sketch_regions`] and
    /// [`Scene::region_boundary`], and the doc comment on
    /// [`kernel::Document::place_text`] for why "every region" would be
    /// wrong). Each extrudes `distance`; the results fold into ONE new
    /// component definition named `name`, whose one identity-posed instance
    /// handle is returned. `group`, when given, births the placement inside
    /// that group (mirrors `follow_me_along_edges`'s trailing `group`);
    /// `undefined`/`None` births top-level. The whole placement — glyph
    /// injection included — replays as ONE undo/redo step.
    pub fn place_text(
        &mut self,
        sketch: u64,
        regions: Vec<u64>,
        distance: f64,
        name: String,
        group: Option<u64>,
    ) -> Result<u64, ApiError> {
        let region_ids: Vec<SketchRegionId> = regions
            .iter()
            .map(|&r| SketchRegionId::from(KeyData::from_ffi(r)))
            .collect();
        let (_component, instance, change) = self
            .doc
            .place_text(
                sketch_id(sketch),
                &region_ids,
                distance,
                name.clone(),
                group.map(group_id),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::PlaceText {
            sketch,
            regions,
            distance,
            name,
            group,
        });
        Ok(instance.data().as_ffi())
    }

    /// Move/rotate/scale an instance by composing `affine` (row-major 3×4) into
    /// its pose — never baked. Mirror/non-uniform allowed; singular errors.
    pub fn transform_instance(&mut self, instance: u64, affine: &[f64]) -> Result<(), ApiError> {
        let t = affine_transform(affine)?;
        let change = self
            .doc
            .transform_instance(instance_id(instance), &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        // affine_transform validated the length, so this cannot fail.
        let mut rec_affine = [0.0f64; 12];
        rec_affine.copy_from_slice(affine);
        recording::record(recording::RecordedCall::TransformInstance {
            instance,
            affine: rec_affine,
        });
        Ok(())
    }

    /// "Explode": bakes an instance's pose into independent world objects,
    /// returning their handles. A mirrored instance errors (`CannotExplodeReflected`).
    pub fn explode_instance(&mut self, instance: u64) -> Result<Vec<u64>, ApiError> {
        let (created, change) = self
            .doc
            .explode_instance(instance_id(instance))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ExplodeInstance { instance });
        Ok(created.iter().map(|o| o.data().as_ffi()).collect())
    }

    /// "Make Unique": detaches an instance onto a fresh private copy of
    /// its definition, returning the new component handle. Later edits to this
    /// instance's definition no longer affect its former siblings.
    pub fn make_unique(&mut self, instance: u64) -> Result<u64, ApiError> {
        let (new_def, change) = self
            .doc
            .make_unique(instance_id(instance))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::MakeUnique { instance });
        Ok(new_def.data().as_ffi())
    }

    /// Opens an explode session on `instance` (docs/design/explode-session-
    /// prototype.md, a `proto/explode-session` prototype): temporarily bakes
    /// the instance's definition's live members/sketches into WORLD-owned
    /// geometry at the instance's pose (SAME ids — a move, not
    /// `explode_instance`'s copy) and hides every live instance of that
    /// definition, so the app's ordinary, unmodified tool set can edit them.
    /// `close_explode_session` folds them back. Refuses typed if a session
    /// is already open, or if the instance's pose is not a similarity with
    /// positive determinant.
    pub fn open_explode_session(&mut self, instance: u64) -> Result<(), ApiError> {
        let change = self
            .doc
            .open_explode_session(instance_id(instance))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::OpenExplodeSession { instance });
        Ok(())
    }

    /// Closes the open explode session, folding its live members/sketches
    /// back into the definition. Refuses typed if no session is open.
    pub fn close_explode_session(&mut self) -> Result<(), ApiError> {
        let change = self.doc.close_explode_session().map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::CloseExplodeSession);
        Ok(())
    }

    /// The instance an open explode session was entered through, or
    /// `undefined` if no session is open — the app's resync query after
    /// undo/redo crosses a session boundary.
    pub fn explode_session_instance(&self) -> Option<u64> {
        self.doc
            .explode_session_instance()
            .map(|i| i.data().as_ffi())
    }

    /// The definition an open explode session is editing, or `undefined`
    /// if no session is open — the app labels the session from this (the
    /// entered instance is hidden for the session's duration, so the
    /// ordinary instance→definition queries answer `undefined` for it).
    pub fn explode_session_component(&self) -> Option<u64> {
        self.doc
            .explode_session_component()
            .map(|c| c.data().as_ffi())
    }

    /// The live objects inside the open explode session (original members
    /// plus mid-session creations — the same scope the kernel's guards and
    /// the close's fold-in use), or `undefined` when no session is open.
    /// The app scopes picking/selection to exactly this set while a
    /// session is open.
    pub fn explode_session_objects(&self) -> Option<Vec<u64>> {
        self.doc
            .explode_session_objects()
            .map(|v| v.iter().map(|o| o.data().as_ffi()).collect())
    }

    /// [`Scene::explode_session_objects`]'s sketch analog: the live
    /// sketches inside the open explode session, or `undefined` when no
    /// session is open — the app scopes free-standing sketch selection to
    /// this set while a session is open.
    pub fn explode_session_sketches(&self) -> Option<Vec<u64>> {
        self.doc
            .explode_session_sketches()
            .map(|v| v.iter().map(|s| s.data().as_ffi()).collect())
    }

    /// Opens a group editing session on `group`
    /// (docs/design/group-session.md): applies the ungroup posture — the
    /// group's direct members surface at the top level, the group hides —
    /// so the app's ordinary, unmodified tool set (the replacing ops
    /// included) can edit them; `close_group_session` re-homes the
    /// survivors and folds in whatever was created meanwhile. Refuses
    /// typed on a stale/hidden group, a nested group (drill down through
    /// its ancestors), or while a component session is open.
    pub fn open_group_session(&mut self, group: u64) -> Result<(), ApiError> {
        let change = self
            .doc
            .open_group_session(group_id(group))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::OpenGroupSession { group });
        Ok(())
    }

    /// Closes the open group session (the innermost frame must be one),
    /// re-homing survivors and folding in mid-session creations. Refuses
    /// typed otherwise.
    pub fn close_group_session(&mut self) -> Result<(), ApiError> {
        let change = self.doc.close_group_session().map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::CloseGroupSession);
        Ok(())
    }

    /// Closes the INNERMOST open session frame, whichever kind it is —
    /// the app's Escape / double-click-outside gesture. Dispatches to the
    /// specific close, so the recording stays replay-exact.
    pub fn close_innermost_session(&mut self) -> Result<(), ApiError> {
        match self.doc.session_stack().last() {
            Some(NodeId::Instance(_)) => self.close_explode_session(),
            Some(_) => self.close_group_session(),
            None => Err(doc_err(kernel::DocumentError::ExplodeSessionNotOpen)),
        }
    }

    /// The group an open innermost group session is editing, or
    /// `undefined` — the group analog of
    /// [`Scene::explode_session_component`], for the app's session label
    /// and resync-after-undo query.
    pub fn group_session_group(&self) -> Option<u64> {
        self.doc.group_session_group().map(|g| g.data().as_ffi())
    }

    /// The whole open session stack, outermost first: each frame as the
    /// node the user entered — a group, or a component instance for the
    /// (always innermost) explode session. Empty when nothing is open.
    /// The app renders its breadcrumb and "editing" chips from this.
    pub fn session_stack(&self) -> Vec<NodeJs> {
        self.doc.session_stack().into_iter().map(node_js).collect()
    }

    /// The innermost open frame's live direct members — kernel truth for
    /// the app's session-scoped Outliner rows, picking, and dimming
    /// (correct across undo/redo re-entry into any earlier session
    /// bracket, unlike an app-side open-time snapshot). `undefined` when
    /// nothing is open.
    pub fn session_members(&self) -> Option<Vec<NodeJs>> {
        self.doc
            .session_direct_members()
            .map(|v| v.into_iter().map(node_js).collect())
    }

    /// Resizes the contents of the INNERMOST open session frame by
    /// `factor` about the world-space anchor `(ax, ay, az)` — the Tape
    /// Measure's in-context resize (docs/design/group-session.md). The
    /// world outside the session is untouched, so unlike
    /// [`Scene::rescale_document`] the app applies no camera/grid
    /// companion scaling. Refuses typed when no session is open or the
    /// factor/anchor is non-finite.
    pub fn rescale_session(
        &mut self,
        factor: f64,
        ax: f64,
        ay: f64,
        az: f64,
    ) -> Result<(), ApiError> {
        let change = self
            .doc
            .rescale_session(factor, Point3::new(ax, ay, az))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RescaleSession {
            factor,
            anchor: [ax, ay, az],
        });
        Ok(())
    }

    /// Handles of all currently visible component instances.
    pub fn instance_ids(&self) -> Vec<u64> {
        self.doc
            .instance_ids()
            .iter()
            .map(|i| i.data().as_ffi())
            .collect()
    }

    /// The definition an instance places, or `undefined` if the instance is
    /// stale/hidden.
    pub fn instance_def(&self, instance: u64) -> Option<u64> {
        self.doc
            .instance_def(instance_id(instance))
            .map(|c| c.data().as_ffi())
    }

    /// An instance's pose as a row-major 3×4 affine (12 floats) for building the
    /// render node matrix, or `undefined` if the instance is stale/hidden.
    pub fn instance_pose(&self, instance: u64) -> Option<Vec<f64>> {
        self.doc
            .instance_pose(instance_id(instance))
            .map(|t| t.to_affine().to_vec())
    }

    /// The EXPANDED leaf-object placements of an instance's definition —
    /// nested member instances composed, member groups descended — as
    /// parallel arrays: this returns the leaf object handle per placement
    /// (repeats are real: two nested placements of one leaf are two
    /// entries), [`Scene::instance_expanded_local_poses`] returns each
    /// placement's DEF-LOCAL pose. The renderer draws each entry at
    /// `instance_pose × local_pose`. For a flat definition this is exactly
    /// [`Scene::component_member_objects`] with identity local poses.
    pub fn instance_expanded_members(&self, instance: u64) -> Vec<u64> {
        let Some(def) = self.doc.instance_def(instance_id(instance)) else {
            return Vec::new();
        };
        self.doc
            .expanded_def_placements(def)
            .iter()
            .map(|(o, _)| o.data().as_ffi())
            .collect()
    }

    /// The def-local composed pose of each expanded placement, 12 floats
    /// (row-major 3×4) per entry, parallel to
    /// [`Scene::instance_expanded_members`].
    pub fn instance_expanded_local_poses(&self, instance: u64) -> Vec<f64> {
        let Some(def) = self.doc.instance_def(instance_id(instance)) else {
            return Vec::new();
        };
        self.doc
            .expanded_def_placements(def)
            .iter()
            .flat_map(|(_, local)| local.to_affine().to_vec())
            .collect()
    }

    /// Whether a definition has NESTED members (member groups or member
    /// instances). The app gates the in-context editing entry on this —
    /// the flat editing surfaces refuse nested definitions typed.
    pub fn component_has_nested_members(&self, component: u64) -> bool {
        self.doc
            .def_member_nodes(component_id(component))
            .is_some_and(|members| {
                members
                    .iter()
                    .any(|m| !matches!(m, kernel::NodeId::Object(_)))
            })
    }

    /// A visible object's display name, or `undefined` if unnamed/stale. The UI
    /// falls back to a positional label when this is `undefined`.
    pub fn object_name(&self, object: u64) -> Option<String> {
        self.doc.object_name(object_id(object)).map(str::to_string)
    }

    /// A visible group's display name, or `undefined` if unnamed/stale.
    pub fn group_name(&self, group: u64) -> Option<String> {
        self.doc.group_name(group_id(group)).map(str::to_string)
    }

    /// An instance's own display name, or `undefined` if unnamed/stale. An
    /// unnamed instance should display its def's name — see
    /// [`Scene::component_name`] with [`Scene::instance_def`].
    pub fn instance_name(&self, instance: u64) -> Option<String> {
        self.doc
            .instance_name(instance_id(instance))
            .map(str::to_string)
    }

    /// A component definition's display name, or `undefined` if unnamed/stale.
    /// Used as the fallback label for the definition's instances.
    pub fn component_name(&self, component: u64) -> Option<String> {
        self.doc
            .component_name(component_id(component))
            .map(str::to_string)
    }

    /// Rename a component definition (undoable). The definition name is the
    /// shared display label of every instance that places it, so the change
    /// refreshes all of them. `name = None` clears the name (instances fall
    /// back to a positional label). Renaming to the current name is a no-op
    /// (no undo entry). A stale/hidden component errors (`UnknownComponent`).
    pub fn set_component_name(
        &mut self,
        component: u64,
        name: Option<String>,
    ) -> Result<(), ApiError> {
        let change = self
            .doc
            .set_component_name(component_id(component), name)
            .map_err(doc_err)?;
        self.reconcile(&change);
        Ok(())
    }

    // ---------------------------------------------------------- node metadata

    /// Rename a visible tree node (undoable). `name = None` clears the name so
    /// the UI falls back to a positional label. Pass `Some("")` to set an
    /// explicit empty string — the kernel and UI decide how to display it.
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn set_node_name(
        &mut self,
        kind: u8,
        id: u64,
        name: Option<String>,
    ) -> Result<(), ApiError> {
        let node = node_id(kind, id)?;
        let change = self
            .doc
            .set_node_name(node, name.clone())
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetNodeName { kind, id, name });
        Ok(())
    }

    /// Append a tag path to a visible tree node (undoable). `path` is an
    /// ordered list of folder-path segments (root first), e.g. `["Structure",
    /// "Roof"]`. No-op (no undo entry) if the tag is already present.
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn add_node_tag(&mut self, kind: u8, id: u64, path: Vec<String>) -> Result<(), ApiError> {
        let node = node_id(kind, id)?;
        let change = self.doc.add_node_tag(node, path.clone()).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::AddNodeTag { kind, id, path });
        Ok(())
    }

    /// [`Scene::add_node_tag`] across several nodes as ONE undo step (a
    /// labeled compound — the Object Info panel's multi-selection tagging).
    /// `kinds`/`ids` are parallel; nodes already carrying the tag are left
    /// alone; a stale handle anywhere refuses the whole batch with nothing
    /// applied.
    pub fn add_node_tag_many(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        path: Vec<String>,
    ) -> Result<(), ApiError> {
        let nodes = node_ids(kinds, ids)?;
        let label = tag_batch_label("Tag", nodes.len());
        let change = self
            .doc
            .add_node_tag_many(&nodes, path.clone(), &label)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::AddNodeTagMany {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
            path,
        });
        Ok(())
    }

    /// [`Scene::remove_node_tag`] across several nodes as ONE undo step —
    /// the counterpart of [`Scene::add_node_tag_many`].
    pub fn remove_node_tag_many(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        path: Vec<String>,
    ) -> Result<(), ApiError> {
        let nodes = node_ids(kinds, ids)?;
        let label = tag_batch_label("Untag", nodes.len());
        let change = self
            .doc
            .remove_node_tag_many(&nodes, &path, &label)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RemoveNodeTagMany {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
            path,
        });
        Ok(())
    }

    /// Remove the first occurrence of `path` from a visible tree node's tag
    /// list (undoable). No-op (no undo entry) if the path is not present.
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn remove_node_tag(
        &mut self,
        kind: u8,
        id: u64,
        path: Vec<String>,
    ) -> Result<(), ApiError> {
        let node = node_id(kind, id)?;
        let change = self.doc.remove_node_tag(node, &path).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RemoveNodeTag { kind, id, path });
        Ok(())
    }

    /// The tag paths of a visible tree node, encoded as `Vec<String>`.
    ///
    /// Each tag path (a root-first list of folder segments, e.g.
    /// `["Structure","Roof"]`) is joined with `/` into a single string
    /// (e.g. `"Structure/Roof"`). The UI should split on `/` to recover the
    /// segments. **Known limitation**: tag or folder names that themselves
    /// contain `/` will round-trip incorrectly — SketchUp tag names with `/`
    /// are rare in practice and the extra engineering is deferred.
    ///
    /// Returns an empty `Vec` if the node is stale, hidden, or has no tags.
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn node_tags(&self, kind: u8, id: u64) -> Result<Vec<String>, ApiError> {
        let node = node_id(kind, id)?;
        let tags = self
            .doc
            .node_tags(node)
            .iter()
            .map(|segments| segments.join("/"))
            .collect();
        Ok(tags)
    }

    /// The tag metadata registry: every KNOWN tag path (registered by
    /// import or [`Scene::set_tag_hidden`]), `/`-joined and sorted. The
    /// parallel hidden flags come from [`Scene::tag_meta_hidden`] — two
    /// primitive vecs keep the FFI free of ad-hoc JSON. Includes tags no
    /// node carries (an imported `.skp` layer list survives in full).
    ///
    /// Same `/`-join limitation as [`Scene::node_tags`].
    pub fn tag_meta_paths(&self) -> Vec<String> {
        self.doc
            .tag_meta()
            .map(|(segments, _)| segments.join("/"))
            .collect()
    }

    /// Hidden-by-default flags parallel to [`Scene::tag_meta_paths`].
    pub fn tag_meta_hidden(&self) -> Vec<u8> {
        self.doc
            .tag_meta()
            .map(|(_, hidden)| u8::from(hidden))
            .collect()
    }

    /// Sets (registering if unknown) a tag's hidden-by-default flag. `path`
    /// is `/`-joined like [`Scene::node_tags`]. View state, not undoable;
    /// persisted with the document (manifest v5) so hidden `.skp` layers
    /// stay hidden across save/load.
    pub fn set_tag_hidden(&mut self, path: String, hidden: bool) {
        let segments: Vec<String> = path.split('/').map(str::to_string).collect();
        self.doc.set_tag_hidden(segments, hidden);
        recording::record(recording::RecordedCall::SetTagHidden { path, hidden });
    }

    /// Delete the tag `path` — and every registered tag nested under it —
    /// from the whole document (undoable): unregisters it from the tag
    /// metadata (dropping its hidden-by-default flag) and unassigns it from
    /// every node that carries it. Geometry is never deleted or modified.
    /// No-op (no undo entry) for an unknown path. `path` is `/`-joined like
    /// [`Scene::node_tags`].
    pub fn delete_tag(&mut self, path: String) -> Result<(), ApiError> {
        let segments: Vec<String> = path.split('/').map(str::to_string).collect();
        let change = self.doc.delete_tag(&segments).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteTag { path });
        Ok(())
    }

    /// Rename a tag path (and every registered tag nested under it) to
    /// `new_path` — undoable, identity-preserving (same stable id, hidden
    /// flag, and attributes; a Scene's captured hidden tags follow). Both
    /// paths are `/`-joined like [`Scene::delete_tag`]'s. Refuses a
    /// collision (`DuplicateTag`) or an empty / self-nested target
    /// (`InvalidTagPath`).
    pub fn rename_tag(&mut self, path: String, new_path: String) -> Result<(), ApiError> {
        let from: Vec<String> = path
            .split('/')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let to: Vec<String> = new_path
            .split('/')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let change = self.doc.rename_tag(&from, to).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::RenameTag { path, new_path });
        Ok(())
    }

    /// Whether a node is USER-hidden (persisted view state, manifest v6).
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn node_user_hidden(&self, kind: u8, id: u64) -> Result<bool, ApiError> {
        let node = node_id(kind, id)?;
        Ok(self.doc.node_user_hidden(node))
    }

    /// Sets a node's USER-hidden flag (persisted view state, not
    /// undoable — matching [`Scene::set_tag_hidden`]).
    ///
    /// `kind`: 0 = object, 1 = group, 2 = instance.
    pub fn set_node_user_hidden(
        &mut self,
        kind: u8,
        id: u64,
        hidden: bool,
    ) -> Result<(), ApiError> {
        let node = node_id(kind, id)?;
        self.doc.set_node_user_hidden(node, hidden);
        recording::record(recording::RecordedCall::SetNodeUserHidden { kind, id, hidden });
        Ok(())
    }

    /// Every USER-hidden node as parallel kind/id vectors (for seeding the
    /// UI's visibility state on load/import).
    pub fn user_hidden_kinds(&self) -> Vec<u8> {
        self.doc
            .user_hidden_nodes()
            .iter()
            .map(|n| match n {
                kernel::NodeId::Object(_) => 0,
                kernel::NodeId::Group(_) => 1,
                kernel::NodeId::Instance(_) => 2,
            })
            .collect()
    }

    /// Ids parallel to [`Scene::user_hidden_kinds`].
    pub fn user_hidden_ids(&self) -> Vec<u64> {
        self.doc
            .user_hidden_nodes()
            .iter()
            .map(|n| match n {
                kernel::NodeId::Object(id) => id.data().as_ffi(),
                kernel::NodeId::Group(id) => id.data().as_ffi(),
                kernel::NodeId::Instance(id) => id.data().as_ffi(),
            })
            .collect()
    }

    /// Whether `object` is a live, visible, watertight (solid) object.
    ///
    /// Returns `false` if the id is stale, hidden, or the object is leaky/open.
    pub fn object_solid(&self, id: u64) -> bool {
        self.doc.object_solid(object_id(id))
    }

    /// The LIVE member objects of a definition (definition-local geometry), in
    /// order. Fetch each one's mesh via [`Scene::object_mesh`] and draw it at
    /// the instance pose. Empty if the component is stale/hidden.
    ///
    /// [`Document::def_members`] deliberately keeps a hidden (deleted, or
    /// undone-birth) member's id listed — Group parity, and it simplifies
    /// undo — so this filters through [`Document::object`] before returning,
    /// exactly like [`Scene::register_instance`]'s own per-member liveness
    /// check. Without this filter a hidden member's id would reach
    /// [`Scene::object_mesh`], which errors for it (stale/hidden), rather
    /// than simply being absent from the render/pick set.
    pub fn component_member_objects(&self, component: u64) -> Vec<u64> {
        self.doc
            .def_members(component_id(component))
            .unwrap_or_default()
            .iter()
            .filter(|&&o| self.doc.object(o).is_some())
            .map(|o| o.data().as_ffi())
            .collect()
    }

    /// The LIVE member sketches of a definition (component-edit-parity.md
    /// phase K1), in definition-local coordinates — the sketch analog of
    /// [`Scene::component_member_objects`]. Empty if the component is
    /// stale/hidden or has no not-yet-extruded sketches.
    ///
    /// Filtered through [`Document::sketch`] for the same reason
    /// [`Scene::component_member_objects`] filters through [`Document::object`]:
    /// [`Document::def_member_sketches`] keeps a hidden (deleted, or an
    /// undone gesture's now-empty fresh sketch) sketch's id listed, and this
    /// is the render/pick ground truth for "does this sketch still round-trip
    /// through the document" — mirroring [`Scene::pick_sketch_region_in_instance`],
    /// which already applies exactly this filter for picking.
    pub fn component_member_sketches(&self, component: u64) -> Vec<u64> {
        self.doc
            .def_member_sketches(component_id(component))
            .unwrap_or_default()
            .iter()
            .filter(|&&s| self.doc.sketch(s).is_some())
            .map(|s| s.data().as_ffi())
            .collect()
    }

    /// The visible instances that place a definition.
    pub fn instances_of(&self, component: u64) -> Vec<u64> {
        self.doc
            .instances_of(component_id(component))
            .iter()
            .map(|i| i.data().as_ffi())
            .collect()
    }

    /// Push/pull a face of a component's shared geometry — editing *inside* a
    /// component. `object` is the definition member (from
    /// [`Scene::component_member_objects`] or a pick's `.object()`); the edit is
    /// seen by every instance of `instance`'s definition at once. Like
    /// [`Scene::push_pull`], a flat imprinted sub-face auto-routes to
    /// wall-generating extrude. Routed through the kernel's `apply_def_op`, so
    /// it cannot touch world objects.
    ///
    /// `distance` is a WORLD-space length — the ghost preview sweeps the
    /// world drag distance — mapped through `instance`'s pose via
    /// [`kernel::Document::map_instance_world_distance`] exactly like
    /// [`Scene::extrude_region_in_instance`]'s `distance` (see that doc for
    /// the exact rule and its typed refusal under non-uniform scale): a
    /// scaled instance's ghost and its committed geometry must agree, and a
    /// raw (unmapped) distance previously let them diverge.
    ///
    /// # Errors
    /// - `UnknownInstance` — `instance` is stale/hidden.
    /// - `AmbiguousInstanceScale` — the instance's pose is not a similarity
    ///   (non-uniform scale); the typed `distance` cannot map unambiguously.
    /// - `UnknownComponent` / `UnknownObject` — `object` is not a member of
    ///   `instance`'s definition.
    pub fn push_pull_in_component(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<PushPullJs, ApiError> {
        let iid = instance_id(instance);
        let (component, local_distance) = self
            .doc
            .map_instance_world_distance(iid, distance)
            .map_err(doc_err)?;
        let oid = object_id(object);
        let face_id = FaceId::from(KeyData::from_ffi(face));
        if self
            .doc
            .object(oid)
            .is_some_and(|o| o.push_pull_overshoots(face_id, local_distance))
        {
            let (results, change) = self
                .doc
                .push_pull_through_in_component(component, oid, face_id, local_distance)
                .map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::PushPullInComponent {
                instance,
                object,
                face,
                distance,
            });
            return Ok(PushPullJs {
                inner: None,
                through: results.iter().map(|id| id.data().as_ffi()).collect(),
            });
        }
        let is_sub = self
            .doc
            .object(oid)
            .is_some_and(|o| o.is_flat_sub_face(face_id));
        let op = if is_sub {
            KernelOp::ExtrudeSubFace {
                sub_face: face_id,
                distance: local_distance,
            }
        } else {
            KernelOp::PushPull {
                face: face_id,
                distance: local_distance,
            }
        };
        let (report, change) = self.doc.apply_def_op(component, oid, op).map_err(doc_err)?;
        self.reconcile(&change);
        match report {
            KernelOpReport::PushPull(inner) | KernelOpReport::ExtrudeSubFace(inner) => {
                recording::record(recording::RecordedCall::PushPullInComponent {
                    instance,
                    object,
                    face,
                    distance,
                });
                Ok(PushPullJs {
                    inner: Some(inner),
                    through: Vec::new(),
                })
            }
            other => Err(api_err(
                &other,
                &"unexpected report kind for push_pull_in_component",
            )),
        }
    }

    /// Cut a member face along a WORLD-space `path` (xyz triples) — face-mode
    /// drawing inside a component (component-edit-parity.md phase K1).
    /// `object` is the definition member owning `face` (from
    /// [`Scene::component_member_objects`] or a pick's `.object()`); the
    /// kernel maps `path` into definition-local space via `instance`'s
    /// pose⁻¹ (unlike a scalar distance, a point mapping is unambiguous
    /// under any invertible pose — rotation, mirror, or non-uniform scale —
    /// so unlike [`Scene::extrude_region_in_instance`] this never refuses on
    /// scale) and routes through the kernel's `apply_def_op` with the same
    /// `SplitFace` op [`Scene::split_face`] uses for world objects — the
    /// face-cut path already existed; this is the missing instance-aware
    /// wiring. The edit is seen by every instance of the definition at once.
    ///
    /// # Errors
    /// - `UnknownInstance` — `instance` is stale/hidden.
    /// - `Singular` — the instance's pose failed to invert; unreachable for
    ///   a live instance in practice.
    /// - `UnknownComponent` / `UnknownObject` — `object` is not a member of
    ///   `instance`'s definition.
    /// - Whatever [`Scene::split_face`] itself can refuse (a `path` shorter
    ///   than two points, a cut that doesn't resolve on `face`, …).
    pub fn split_face_in_instance(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        path: &[f64],
    ) -> Result<FaceSplitJs, ApiError> {
        if !path.len().is_multiple_of(3) || path.len() < 6 {
            return Err(ApiError(
                "BadPath: path must be at least two xyz triples".to_string(),
            ));
        }
        let iid = instance_id(instance);
        let pose = self
            .doc
            .instance_pose(iid)
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let component = self
            .doc
            .instance_def(iid)
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let pose_inv = pose.inverse().map_err(|e| api_err(&e, &e))?;
        let local_points: Vec<Point3> = path
            .chunks_exact(3)
            .map(|c| pose_inv.apply_point(Point3::new(c[0], c[1], c[2])))
            .collect();
        let op = KernelOp::SplitFace {
            face: FaceId::from(KeyData::from_ffi(face)),
            path: local_points,
            restore: None,
        };
        let (report, change) = self
            .doc
            .apply_def_op(component, object_id(object), op)
            .map_err(doc_err)?;
        self.reconcile(&change);
        match report {
            KernelOpReport::FaceSplit(inner) => {
                recording::record(recording::RecordedCall::SplitFaceInInstance {
                    instance,
                    object,
                    face,
                    path: path.to_vec(),
                });
                Ok(FaceSplitJs { inner })
            }
            other => Err(api_err(
                &other,
                &"unexpected report kind for split_face_in_instance",
            )),
        }
    }

    /// Imprint a closed loop strictly inside a member face's WORLD-space
    /// boundary — the definition-member analog of [`Scene::split_face_inner`]
    /// (component-edit-parity.md phase A2: Rectangle/Circle/Polygon's
    /// face-mode draw needs this loop-imprint shape, not the boundary-to-
    /// boundary cut [`Scene::split_face_in_instance`] covers). `loop_pts` is
    /// mapped into definition-local space through `instance`'s pose⁻¹, same
    /// as `split_face_in_instance`'s `path` — a point mapping is unambiguous
    /// under any invertible pose, so this never refuses on scale. Returns the
    /// new sub-face handle; push/pull it (`push_pull_in_component`) to
    /// boss/recess.
    ///
    /// # Errors
    /// Same family as `split_face_in_instance`: `UnknownInstance`, `Singular`,
    /// `UnknownComponent`/`UnknownObject`, plus whatever
    /// [`Scene::split_face_inner`] itself refuses.
    pub fn split_face_inner_in_instance(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        loop_pts: &[f64],
    ) -> Result<u64, ApiError> {
        self.split_face_inner_in_instance_impl(instance, object, face, loop_pts, None)
    }

    /// [`Scene::split_face_inner_in_instance`] carrying a drawn circle's
    /// analytic identity, mirroring [`Scene::split_face_inner_with_curve`]
    /// for a definition member (CircleTool's face mode inside a component).
    /// `center`/`radius` are WORLD-space, exactly like `loop_pts` — both are
    /// mapped into the instance's definition-local frame (`center` through
    /// `pose⁻¹`, `radius` through `pose`'s uniform scale) before reaching the
    /// kernel's curve-claim validator, which checks the claim against
    /// `loop_pts` in that same local frame.
    ///
    /// # Errors
    /// Same family as `split_face_inner_in_instance`, plus
    /// `AmbiguousInstanceScale` when `instance`'s pose has a non-uniform
    /// scale — a world radius has no single local-frame equivalent then, so
    /// this refuses rather than guess an axis (drag instead of drawing an
    /// exact-radius circle on such an instance).
    pub fn split_face_inner_with_curve_in_instance(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        loop_pts: &[f64],
        center: &[f64],
        radius: f64,
    ) -> Result<u64, ApiError> {
        if center.len() != 3 {
            return Err(ApiError(
                "BadCurve: center must be an xyz triple".to_string(),
            ));
        }
        let curve = kernel::CurveGeom {
            center: Point3::new(center[0], center[1], center[2]),
            radius,
        };
        self.split_face_inner_in_instance_impl(instance, object, face, loop_pts, Some(curve))
    }

    fn split_face_inner_in_instance_impl(
        &mut self,
        instance: u64,
        object: u64,
        face: u64,
        loop_pts: &[f64],
        curve: Option<kernel::CurveGeom>,
    ) -> Result<u64, ApiError> {
        if !loop_pts.len().is_multiple_of(3) || loop_pts.len() < 9 {
            return Err(ApiError(
                "BadLoop: loop needs at least three xyz triples".to_string(),
            ));
        }
        let iid = instance_id(instance);
        let pose = self
            .doc
            .instance_pose(iid)
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let component = self
            .doc
            .instance_def(iid)
            .ok_or_else(|| stale("UnknownInstance", "instance"))?;
        let pose_inv = pose.inverse().map_err(|e| api_err(&e, &e))?;
        let local_points: Vec<Point3> = loop_pts
            .chunks_exact(3)
            .map(|c| pose_inv.apply_point(Point3::new(c[0], c[1], c[2])))
            .collect();
        // `curve` (when present) is the caller's WORLD-space analytic circle
        // identity — center and radius — exactly like `loop_pts` above. The
        // kernel's split-face validator (`ops.rs`) checks every loop vertex's
        // distance to `curve.center` against `curve.radius` in LOCAL space
        // (the frame `local_points` was just mapped into), so the curve must
        // be mapped into that same frame or the claim never matches: a point
        // maps unambiguously under any invertible pose (`pose_inv.apply_point`,
        // same as the loop), but a scalar radius only maps unambiguously
        // under a uniform scale — same rule as `Document::
        // map_world_distance_through_pose`'s typed-distance surfaces — so a
        // non-uniformly-scaled instance refuses `AmbiguousInstanceScale`
        // rather than guess an axis.
        let mapped_curve = curve
            .map(|c| -> Result<kernel::CurveGeom, ApiError> {
                let scale = pose
                    .similarity_scale()
                    .ok_or_else(|| doc_err(DocumentError::AmbiguousInstanceScale))?;
                Ok(kernel::CurveGeom {
                    center: pose_inv.apply_point(c.center),
                    radius: c.radius / scale,
                })
            })
            .transpose()?;
        let op = KernelOp::SplitFaceInner {
            face: FaceId::from(KeyData::from_ffi(face)),
            loop_path: local_points,
            restore: None,
            curve: mapped_curve,
        };
        let (report, change) = self
            .doc
            .apply_def_op(component, object_id(object), op)
            .map_err(doc_err)?;
        self.reconcile(&change);
        match report {
            KernelOpReport::FaceSplitInner(r) => {
                recording::record(recording::RecordedCall::SplitFaceInnerInInstance {
                    instance,
                    object,
                    face,
                    loop_pts: loop_pts.to_vec(),
                    curve: curve.map(|g| [g.center.x, g.center.y, g.center.z, g.radius]),
                });
                Ok(r.sub_face.data().as_ffi())
            }
            other => Err(api_err(
                &other,
                &"unexpected report kind for split_face_inner_in_instance",
            )),
        }
    }

    /// Removes one member Object from a component definition (component-edit-
    /// parity.md phase K1) — the definition-member analog of
    /// [`Scene::delete_node`]. `object` is spliced out of view (hidden, never
    /// erased); refuses typed if it is the definition's last live member
    /// (see [`kernel::Document::delete_def_member`]).
    ///
    /// # Errors
    /// - `UnknownComponent` — `component` is stale/hidden.
    /// - `UnknownObject` — `object` is not a live member of `component`.
    /// - `LastDefinitionMember` — `object` is the definition's only live
    ///   member; delete its instances instead.
    pub fn delete_def_member(&mut self, component: u64, object: u64) -> Result<(), ApiError> {
        let change = self
            .doc
            .delete_def_member(component_id(component), object_id(object))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteDefMember { component, object });
        Ok(())
    }

    /// [`Scene::boolean`] between two members of the SAME component
    /// definition (component-edit-parity.md phase K2): `op` is 0 = union,
    /// 1 = subtract (`a - b`), 2 = intersect, exactly like [`Scene::boolean`].
    /// `a`/`b` must both be members of `component`; the result replaces
    /// them as a new member, seen by every instance of `component` at once.
    /// See [`kernel::Document::boolean_in_component`].
    pub fn boolean_in_component(
        &mut self,
        component: u64,
        op: u8,
        a: u64,
        b: u64,
    ) -> Result<u64, ApiError> {
        let bop = match op {
            0 => BooleanOp::Union,
            1 => BooleanOp::Subtract,
            2 => BooleanOp::Intersect,
            _ => return Err(ApiError("BadOp: op must be 0, 1, or 2".to_string())),
        };
        let (id, change) = self
            .doc
            .boolean_in_component(component_id(component), object_id(a), object_id(b), bop)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::BooleanInComponent {
            component,
            op,
            a,
            b,
        });
        Ok(id.data().as_ffi())
    }

    /// [`Scene::slice_object`] on a member of `instance`'s definition
    /// (component-edit-parity.md phase K2): `plane` is 6 floats
    /// `[px,py,pz,nx,ny,nz]` in WORLD space, mapped into definition-local
    /// space through the instance's pose⁻¹. `object` must be a live member
    /// of `instance`'s own definition. Returns `[positive, negative]`;
    /// both pieces become new members of the same definition. See
    /// [`kernel::Document::slice_def_member`].
    pub fn slice_def_member(
        &mut self,
        instance: u64,
        object: u64,
        plane: &[f64],
    ) -> Result<Vec<u64>, ApiError> {
        let p: &[f64; 6] = plane.try_into().map_err(|_| {
            ApiError("BadPlane: slice plane must be 6 floats [px,py,pz,nx,ny,nz]".to_string())
        })?;
        let point = Point3::new(p[0], p[1], p[2]);
        let normal = kernel::Vec3::new(p[3], p[4], p[5]);
        let plane = Plane::from_point_normal(point, normal)
            .map_err(|_| ApiError("DegeneratePlane: slice normal has no direction".to_string()))?;
        let ((a, b), change) = self
            .doc
            .slice_def_member(instance_id(instance), object_id(object), &plane)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SliceDefMember {
            instance,
            object,
            plane: *p,
        });
        Ok(vec![a.data().as_ffi(), b.data().as_ffi()])
    }

    /// [`Scene::transform_object`] on a member of `instance`'s definition
    /// (component-edit-parity.md phase K2): `affine` is the same row-major
    /// 3×4 WORLD-space matrix as [`Scene::transform_object`], conjugated
    /// through the instance's pose into definition-local space before
    /// baking (see [`kernel::Document::transform_def_member`] for the exact
    /// mapping and its non-uniform-scale posture — a full affine
    /// conjugation is never ambiguous, unlike a scalar distance). `object`
    /// must be a live member of `instance`'s own definition; every instance
    /// of the definition sees the edit at once.
    pub fn transform_def_member(
        &mut self,
        instance: u64,
        object: u64,
        affine: &[f64],
    ) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let t = Transform::from_affine(rows);
        let change = self
            .doc
            .transform_def_member(instance_id(instance), object_id(object), &t)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformDefMember {
            instance,
            object,
            affine: *rows,
        });
        Ok(())
    }

    /// Transform a definition-owned sketch through one instance's world pose.
    pub fn transform_def_sketch(
        &mut self,
        instance: u64,
        sketch: u64,
        affine: &[f64],
    ) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let change = self
            .doc
            .transform_def_sketch(
                instance_id(instance),
                sketch_id(sketch),
                &Transform::from_affine(rows),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformDefSketch {
            instance,
            sketch,
            affine: *rows,
        });
        Ok(())
    }

    /// Validation-only sibling of [`Scene::transform_def_sketch_island`].
    pub fn can_transform_def_sketch_island(
        &self,
        instance: u64,
        sketch: u64,
        island: u64,
        affine: &[f64],
    ) -> bool {
        let Ok(rows) = <&[f64; 12]>::try_from(affine) else {
            return false;
        };
        self.doc
            .validate_transform_def_sketch_island(
                instance_id(instance),
                sketch_id(sketch),
                kernel::SketchIslandId::from(KeyData::from_ffi(island)),
                &Transform::from_affine(rows),
            )
            .is_ok()
    }

    /// Transform one island of a definition-owned sketch through an instance.
    pub fn transform_def_sketch_island(
        &mut self,
        instance: u64,
        sketch: u64,
        island: u64,
        affine: &[f64],
    ) -> Result<(), ApiError> {
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let change = self
            .doc
            .transform_def_sketch_island(
                instance_id(instance),
                sketch_id(sketch),
                kernel::SketchIslandId::from(KeyData::from_ffi(island)),
                &Transform::from_affine(rows),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformDefSketchIsland {
            instance,
            sketch,
            island,
            affine: *rows,
        });
        Ok(())
    }

    /// Atomically transform a mixed selection inside one component definition.
    pub fn transform_def_selection(
        &mut self,
        instance: u64,
        objects: &[u64],
        sketches: &[u64],
        island_sketches: &[u64],
        islands: &[u64],
        affine: &[f64],
    ) -> Result<(), ApiError> {
        if island_sketches.len() != islands.len() {
            return Err(ApiError(
                "BadSelection: island sketch and island arrays must have equal length".to_string(),
            ));
        }
        let rows: &[f64; 12] = affine.try_into().map_err(|_| {
            ApiError("BadAffine: transform must be 12 floats (row-major 3x4)".to_string())
        })?;
        let object_ids: Vec<_> = objects.iter().copied().map(object_id).collect();
        let sketch_ids: Vec<_> = sketches.iter().copied().map(sketch_id).collect();
        let island_ids: Vec<_> = island_sketches
            .iter()
            .copied()
            .zip(islands.iter().copied())
            .map(|(sketch, island)| {
                (
                    sketch_id(sketch),
                    kernel::SketchIslandId::from(KeyData::from_ffi(island)),
                )
            })
            .collect();
        let change = self
            .doc
            .transform_def_selection(
                instance_id(instance),
                &object_ids,
                &sketch_ids,
                &island_ids,
                &Transform::from_affine(rows),
            )
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::TransformDefSelection {
            instance,
            objects: objects.to_vec(),
            sketches: sketches.to_vec(),
            island_sketches: island_sketches.to_vec(),
            islands: islands.to_vec(),
            affine: *rows,
        });
        Ok(())
    }

    /// Render buffers for one Object (cached until its next mutation).
    pub fn object_mesh(&mut self, object: u64) -> Result<MeshJs, ApiError> {
        let id = object_id(object);
        if !self.mesh_cache.contains_key(id) {
            let palette = self.doc.materials();
            let object = self
                .doc
                .object(id)
                .ok_or_else(|| stale("UnknownObject", "object"))?;
            let mesh = tessellate(object, palette).map_err(|e| api_err(&e, &e))?;
            self.mesh_cache.insert(id, mesh);
        }
        let watertight = self
            .doc
            .object(id)
            .ok_or_else(|| stale("UnknownObject", "object"))?
            .watertight()
            == WatertightState::Watertight;
        Ok(MeshJs {
            mesh: self.mesh_cache[id].clone(),
            watertight,
        })
    }

    /// Export tessellation of one Object as a flat triangle soup
    /// (9 floats per triangle, CCW from outside, object-local meters) at a
    /// chosen curve resolution — the true-curves design stage 6.
    ///
    /// `segments_per_turn == 0` exports the stored facets verbatim; any
    /// other value re-facets every pristine stamped cylinder wall at that
    /// resolution (clamped to the tessellator's supported range), keeping
    /// the mesh manifold at any setting; walls that are no longer fully
    /// analytic (boolean seams, bosses) honestly keep their stored facets.
    /// Uncached — export is a one-shot path, not a per-frame one.
    pub fn object_export_triangles(
        &self,
        object: u64,
        segments_per_turn: u32,
    ) -> Result<Vec<f32>, ApiError> {
        let object = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let soup =
            tessellate::export_triangles(object, segments_per_turn).map_err(|e| api_err(&e, &e))?;
        Ok(soup.into_iter().map(|x| x as f32).collect())
    }

    /// Whether an Object encloses a volume (drives the status UI).
    pub fn object_watertight(&self, object: u64) -> Result<bool, ApiError> {
        let object = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        Ok(object.watertight() == WatertightState::Watertight)
    }

    /// World-space outer-loop vertices (flat xyz) of a sketch region — for
    /// rendering the region fill and for client-side region picking. M1
    /// sketches are planar, so this is the region's boundary polygon.
    pub fn region_boundary(&self, sketch: u64, region: u64) -> Result<Vec<f32>, ApiError> {
        let s = self
            .doc
            .sketch(sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let rid = SketchRegionId::from(KeyData::from_ffi(region));
        let region = s
            .regions()
            .get(rid)
            .ok_or_else(|| stale("UnknownRegion", "region"))?;
        let mut out = Vec::with_capacity(region.outer.len() * 3);
        for &vid in &region.outer {
            let p = s.vertices()[vid].position;
            out.extend([p.x as f32, p.y as f32, p.z as f32]);
        }
        Ok(out)
    }

    /// An Object face's outer-loop boundary as a flat `[x,y,z, x,y,z, …]` of
    /// ordered world-space vertices — the face analog of [`Self::region_boundary`].
    /// The push/pull live preview sweeps this polygon along the face
    /// normal to ghost the growing/shrinking solid. Like `face_normal`/`face_plane`
    /// this is the Object's local frame (world Objects are identity-placed, so
    /// local == world). Holes in the face are ignored — the preview only needs
    /// the outer silhouette.
    pub fn face_boundary(&self, object: u64, face: u64) -> Result<Vec<f32>, ApiError> {
        let object = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let fid = FaceId::from(KeyData::from_ffi(face));
        let face = object
            .faces()
            .get(fid)
            .ok_or_else(|| stale("UnknownFace", "face"))?;
        let mut out = Vec::new();
        for p in object.loop_positions(face.outer_loop) {
            out.extend([p.x as f32, p.y as f32, p.z as f32]);
        }
        Ok(out)
    }

    /// The unit normal of an Object face — the axis the push/pull tool drags
    /// along. (Exact, unlike guessing from the snap position.)
    pub fn face_normal(&self, object: u64, face: u64) -> Result<Vec<f64>, ApiError> {
        let object = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let fid = FaceId::from(KeyData::from_ffi(face));
        let face = object
            .faces()
            .get(fid)
            .ok_or_else(|| stale("UnknownFace", "face"))?;
        let n = face.plane.normal();
        Ok(vec![n.x, n.y, n.z])
    }

    /// An Object face's plane as `[px,py,pz, nx,ny,nz]`: a point on the face
    /// (its first outer-loop vertex) plus the unit normal — exactly the
    /// `constraint_plane` shape `snap` accepts. Lets a tool drawing on this face
    /// constrain snapping to the face plane so the cursor never snaps through
    /// the solid to occluded geometry. Like `face_normal`, this is the Object's
    /// local frame (world Objects are identity-placed).
    pub fn face_plane(&self, object: u64, face: u64) -> Result<Vec<f64>, ApiError> {
        let object = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let fid = FaceId::from(KeyData::from_ffi(face));
        let face = object
            .faces()
            .get(fid)
            .ok_or_else(|| stale("UnknownFace", "face"))?;
        let n = face.plane.normal();
        let p = object
            .loop_positions(face.outer_loop)
            .next()
            .ok_or_else(|| stale("DegenerateFace", "face"))?;
        Ok(vec![p.x, p.y, p.z, n.x, n.y, n.z])
    }

    /// A world Object edge's two endpoint positions in world space, as
    /// `[ax,ay,az, bx,by,bz]` — the geometry the Tape Measure tool needs to
    /// build a parallel guide line. `undefined` if `object` isn't a live world
    /// object (world Objects are identity-placed, so local space == world
    /// space) or `edge` is stale; the tool falls back to point-to-point
    /// measuring in that case.
    pub fn edge_endpoints(&self, object: u64, edge: u64) -> Option<Vec<f64>> {
        let oid = object_id(object);
        if !self.doc.is_world_object(oid) {
            return None;
        }
        let object = self.doc.object(oid)?;
        let eid = EdgeId::from(KeyData::from_ffi(edge));
        let (a, b) = object.edge_endpoints(eid)?;
        Some(vec![a.x, a.y, a.z, b.x, b.y, b.z])
    }

    /// World endpoints `[ax, ay, az, bx, by, bz]` of a sketch edge, or
    /// `undefined` if the sketch or edge is stale. The sketch-edge
    /// counterpart of `edge_endpoints`, for tools that use a snapped sketch
    /// edge as a reference (Tape Measure parallel guides).
    ///
    /// World-op guard (component-edit-parity.md phase K1): a def-owned
    /// sketch's edges are DEFINITION-local, not world, so this refuses one
    /// exactly like `edge_endpoints` refuses a definition-member object via
    /// `is_world_object` — a raw def-local pair handed to `add_guide_line`
    /// under this function's promised "world endpoints" contract would place
    /// the guide in the wrong frame. Guides stay world-space-only in v1
    /// (component-edit-parity.md, "Out of scope"). For a def-owned sketch
    /// viewed through a specific instance, see `sketch_edge_endpoints_in_instance`;
    /// for the object-edge analog, see `edge_endpoints_in_instance`.
    pub fn sketch_edge_endpoints(&self, sketch: u64, edge: u64) -> Option<Vec<f64>> {
        let sid = sketch_id(sketch);
        if self.doc.sketch_owner_component(sid).is_some() {
            return None;
        }
        let s = self.doc.sketch(sid)?;
        let eid = SketchEdgeId::from(KeyData::from_ffi(edge));
        let e = s.edges().get(eid)?;
        let a = s.vertices()[e.from].position;
        let b = s.vertices()[e.to].position;
        Some(vec![a.x, a.y, a.z, b.x, b.y, b.z])
    }

    /// World endpoints of a definition-owned sketch edge as viewed through
    /// `instance`. Returns `None` for stale handles or cross-definition input.
    pub fn sketch_edge_endpoints_in_instance(
        &self,
        instance: u64,
        sketch: u64,
        edge: u64,
    ) -> Option<Vec<f64>> {
        let iid = instance_id(instance);
        let component = self.doc.instance_def(iid)?;
        let sid = sketch_id(sketch);
        if self.doc.sketch_owner_component(sid) != Some(component) {
            return None;
        }
        let pose = self.doc.instance_pose(iid)?;
        let s = self.doc.sketch(sid)?;
        let eid = SketchEdgeId::from(KeyData::from_ffi(edge));
        let e = s.edges().get(eid)?;
        let a = pose.apply_point(s.vertices()[e.from].position);
        let b = pose.apply_point(s.vertices()[e.to].position);
        Some(vec![a.x, a.y, a.z, b.x, b.y, b.z])
    }

    /// World endpoints of a definition-owned Object's edge as viewed through
    /// `instance` — the object-edge analog of `sketch_edge_endpoints_in_instance`.
    /// Returns `None` for stale handles, cross-definition input, or a
    /// world-space object (which has no meaningful "instance" to view it
    /// through — use `edge_endpoints` instead).
    pub fn edge_endpoints_in_instance(
        &self,
        instance: u64,
        object: u64,
        edge: u64,
    ) -> Option<Vec<f64>> {
        let iid = instance_id(instance);
        let component = self.doc.instance_def(iid)?;
        let oid = object_id(object);
        if self.doc.object_owner_component(oid) != Some(component) {
            return None;
        }
        let pose = self.doc.instance_pose(iid)?;
        let object = self.doc.object(oid)?;
        let eid = EdgeId::from(KeyData::from_ffi(edge));
        let (a, b) = object.edge_endpoints(eid)?;
        let a = pose.apply_point(a);
        let b = pose.apply_point(b);
        Some(vec![a.x, a.y, a.z, b.x, b.y, b.z])
    }

    /// Push/pull a face (recorded in the object's undo history). A flat imprinted
    /// sub-face (drawn inside an Object) auto-routes to wall-generating
    /// extrude (boss/recess); any other face uses the translate-mode push/pull.
    ///
    /// An inward push that reaches **past the opposite wall** auto-routes to a
    /// through-cut subtract: material is removed (a recess that breaks the
    /// far wall becomes a through-hole) and a cut that severs the solid yields
    /// two objects. The returned report then has [`PushPullJs::is_through`] set
    /// and carries the new object handles in [`PushPullJs::result_objects`].
    pub fn push_pull(
        &mut self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<PushPullJs, ApiError> {
        let face_id = FaceId::from(KeyData::from_ffi(face));
        let oid = object_id(object);

        // Through-cut detection: an inward push past the opposite wall
        // becomes a subtract, not a translate.
        if self
            .doc
            .object(oid)
            .is_some_and(|o| o.push_pull_overshoots(face_id, distance))
        {
            let (results, change) = self
                .doc
                .push_pull_through(oid, face_id, distance)
                .map_err(doc_err)?;
            self.reconcile(&change);
            recording::record(recording::RecordedCall::PushPull {
                object,
                face,
                distance,
            });
            return Ok(PushPullJs {
                inner: None,
                through: results.iter().map(|id| id.data().as_ffi()).collect(),
            });
        }

        let is_sub = self
            .doc
            .object(oid)
            .is_some_and(|o| o.is_flat_sub_face(face_id));
        let op = if is_sub {
            KernelOp::ExtrudeSubFace {
                sub_face: face_id,
                distance,
            }
        } else {
            KernelOp::PushPull {
                face: face_id,
                distance,
            }
        };
        match self.apply_op(object, op)? {
            KernelOpReport::PushPull(inner) | KernelOpReport::ExtrudeSubFace(inner) => {
                recording::record(recording::RecordedCall::PushPull {
                    object,
                    face,
                    distance,
                });
                Ok(PushPullJs {
                    inner: Some(inner),
                    through: Vec::new(),
                })
            }
            other => Err(api_err(&other, &"unexpected report kind for push_pull")),
        }
    }

    /// Imprint a closed loop strictly inside an object's face (within-Object
    /// drawing): the face splits into the loop's sub-face plus the parent (now
    /// holed). `loop_pts` is xyz triples. Returns the new sub-face handle;
    /// push/pull it to boss/recess. Recorded in undo history.
    pub fn split_face_inner(
        &mut self,
        object: u64,
        face: u64,
        loop_pts: &[f64],
    ) -> Result<u64, ApiError> {
        self.split_face_inner_impl(object, face, loop_pts, None)
    }

    /// [`Scene::split_face_inner`] carrying the drawn circle's analytic
    /// identity (`center`, `radius`), so pushing the imprinted face THROUGH
    /// the solid re-attributes the tunnel walls as a smooth cylinder instead
    /// of leaving faceted walls that refuse whole-wall push/pull
    /// (the true-curves design, playtest fix C3). The tool that drew the
    /// circle owns the truth — the kernel never fits a circle to `loop_pts`
    /// and refuses a claim that does not describe them.
    pub fn split_face_inner_with_curve(
        &mut self,
        object: u64,
        face: u64,
        loop_pts: &[f64],
        center: &[f64],
        radius: f64,
    ) -> Result<u64, ApiError> {
        if center.len() != 3 {
            return Err(ApiError(
                "BadCurve: center must be an xyz triple".to_string(),
            ));
        }
        let curve = kernel::CurveGeom {
            center: Point3::new(center[0], center[1], center[2]),
            radius,
        };
        self.split_face_inner_impl(object, face, loop_pts, Some(curve))
    }

    fn split_face_inner_impl(
        &mut self,
        object: u64,
        face: u64,
        loop_pts: &[f64],
        curve: Option<kernel::CurveGeom>,
    ) -> Result<u64, ApiError> {
        if !loop_pts.len().is_multiple_of(3) || loop_pts.len() < 9 {
            return Err(ApiError(
                "BadLoop: loop needs at least three xyz triples".to_string(),
            ));
        }
        let points: Vec<Point3> = loop_pts
            .chunks_exact(3)
            .map(|c| Point3::new(c[0], c[1], c[2]))
            .collect();
        let op = KernelOp::SplitFaceInner {
            face: FaceId::from(KeyData::from_ffi(face)),
            loop_path: points,
            restore: None,
            curve,
        };
        match self.apply_op(object, op)? {
            KernelOpReport::FaceSplitInner(r) => {
                recording::record(recording::RecordedCall::SplitFaceInner {
                    object,
                    face,
                    loop_pts: loop_pts.to_vec(),
                    curve: curve.map(|g| [g.center.x, g.center.y, g.center.z, g.radius]),
                });
                Ok(r.sub_face.data().as_ffi())
            }
            other => Err(api_err(
                &other,
                &"unexpected report kind for split_face_inner",
            )),
        }
    }

    /// The Offset tool's solid-face commit: offsets `face`'s outer boundary
    /// by `distance` in the face plane (negative = into the face — the only
    /// direction that can land on the face) and imprints the offset loop as
    /// a coplanar sub-face, exactly like drawing on the face does. Boundary
    /// arcs recovered from imprinted edge claims or stamped cylinder walls
    /// offset analytically (`kernel::offset_face_boundary`); when the whole
    /// loop is one circle the imprint carries its analytic identity, so a
    /// later push-through yields a smooth cylinder. Returns the new sub-face
    /// handle; push/pull it to boss/recess. Recorded in undo history.
    pub fn offset_face(&mut self, object: u64, face: u64, distance: f64) -> Result<u64, ApiError> {
        let lp = self.offset_face_loop(object, face, distance)?;
        let mut loop_pts: Vec<f64> = Vec::with_capacity(lp.points.len() * 3);
        for p in &lp.points {
            loop_pts.extend([p.x, p.y, p.z]);
        }
        // A single-circle boundary keeps its analytic identity through the
        // imprint; a mixed boundary imprints as plain edges.
        let first = lp.curves.first().copied().flatten();
        let uniform_circle = first.filter(|_| lp.curves.iter().all(|c| *c == first));
        // Delegating to the imprint path records the literal loop
        // (`RecordedCall::SplitFaceInner`), so replay needs no new variant.
        self.split_face_inner_impl(object, face, &loop_pts, uniform_circle)
    }

    /// Pure preview of [`Scene::offset_face`]: the offset loop as xyz
    /// triples, without mutating anything. Throws the commit's typed errors
    /// (`OffsetTooSmall`, `OffsetCollapsed`, `UnknownFace`); note a loop
    /// that computes fine but lies outside the face is only refused at
    /// commit (`LoopNotStrictlyInside`), so the tool treats an outward drag
    /// as guidance, not geometry.
    pub fn offset_face_preview(
        &self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<Vec<f64>, ApiError> {
        let lp = self.offset_face_loop(object, face, distance)?;
        let mut out: Vec<f64> = Vec::with_capacity(lp.points.len() * 3);
        for p in &lp.points {
            out.extend([p.x, p.y, p.z]);
        }
        Ok(out)
    }

    /// Shared boundary-offset computation for [`Scene::offset_face`] and its
    /// preview, mapping kernel errors to boundary codes.
    fn offset_face_loop(
        &self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<kernel::OffsetLoop, ApiError> {
        let obj = self
            .doc
            .object(object_id(object))
            .ok_or_else(|| stale("UnknownObject", "object"))?;
        let fid = FaceId::from(KeyData::from_ffi(face));
        kernel::offset_face_boundary(obj, fid, distance).map_err(|e| match e {
            kernel::FaceOffsetError::UnknownFace => stale("UnknownFace", "face"),
            kernel::FaceOffsetError::Offset(inner) => api_err(&inner, &inner),
        })
    }

    /// Cut a face along `path` (xyz triples), recorded in undo history.
    pub fn split_face(
        &mut self,
        object: u64,
        face: u64,
        path: &[f64],
    ) -> Result<FaceSplitJs, ApiError> {
        if !path.len().is_multiple_of(3) || path.len() < 6 {
            return Err(ApiError(
                "BadPath: path must be at least two xyz triples".to_string(),
            ));
        }
        let points: Vec<Point3> = path
            .chunks_exact(3)
            .map(|c| Point3::new(c[0], c[1], c[2]))
            .collect();
        let op = KernelOp::SplitFace {
            face: FaceId::from(KeyData::from_ffi(face)),
            path: points,
            restore: None,
        };
        match self.apply_op(object, op)? {
            KernelOpReport::FaceSplit(inner) => {
                recording::record(recording::RecordedCall::SplitFace {
                    object,
                    face,
                    path: path.to_vec(),
                });
                Ok(FaceSplitJs { inner })
            }
            other => Err(api_err(&other, &"unexpected report kind for split_face")),
        }
    }

    /// Dissolve the boundary between two coplanar faces, recorded in undo
    /// history.
    pub fn merge_faces(&mut self, object: u64, edge: u64) -> Result<FaceMergeJs, ApiError> {
        let op = KernelOp::MergeFaces {
            edge: EdgeId::from(KeyData::from_ffi(edge)),
        };
        match self.apply_op(object, op)? {
            KernelOpReport::FaceMerge(inner) => {
                recording::record(recording::RecordedCall::MergeFaces { object, edge });
                Ok(FaceMergeJs { inner })
            }
            other => Err(api_err(&other, &"unexpected report kind for merge_faces")),
        }
    }

    // -------------------------------------------------- document undo/redo

    /// True if there is a document-level action to undo.
    pub fn can_scene_undo(&self) -> bool {
        self.doc.can_undo()
    }

    /// True if there is a document-level action to redo.
    pub fn can_scene_redo(&self) -> bool {
        self.doc.can_redo()
    }

    /// Reverses the most recent document action (LIFO across creations and
    /// per-object ops alike). Undoing a creation hides the object; undoing a
    /// per-object op delegates to that object's [`History`]. Returns what the
    /// undo touched so callers can refresh only the affected scene nodes.
    pub fn scene_undo(&mut self) -> Result<DocChangeJs, ApiError> {
        let change = self.doc.undo().map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SceneUndo);
        Ok(DocChangeJs { inner: change })
    }

    /// Re-applies the most recently undone document action. Object handles are
    /// stable across undo/redo (undone creations are hidden, not deleted), so
    /// redo never has to remap ids. Returns what the redo touched so callers
    /// can refresh only the affected scene nodes.
    pub fn scene_redo(&mut self) -> Result<DocChangeJs, ApiError> {
        let change = self.doc.redo().map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SceneRedo);
        Ok(DocChangeJs { inner: change })
    }

    // ---------------------------------------------------------------- camera

    /// The camera's working view at last save (docs/design/camera.md §5), or
    /// `undefined` when none has ever been set — a pre-v13 file, or a
    /// document that never called [`Scene::set_camera_state`]. The app reads
    /// `undefined` as "use today's home framing".
    pub fn camera_state(&self) -> Option<CameraStateJs> {
        self.doc.camera_state().map(|state| CameraStateJs { state })
    }

    /// Records the camera's current working view: `projection` is
    /// `"perspective"` or `"parallel"`; `eye`/`target`/`up` are xyz triples.
    ///
    /// View state, deliberately NOT undoable — matches how SketchUp treats
    /// the camera (mirrors [`Scene::set_tag_hidden`]'s own non-undoable,
    /// still-persisted posture). Saved with the document (manifest v13);
    /// the app calls this on document save and reads it back via
    /// [`Scene::camera_state`] on load.
    pub fn set_camera_state(
        &mut self,
        projection: &str,
        fov_deg: f64,
        eye: Box<[f64]>,
        target: Box<[f64]>,
        up: Box<[f64]>,
    ) -> Result<(), ApiError> {
        let kernel_projection = match projection {
            "perspective" => kernel::CameraProjection::Perspective,
            "parallel" => kernel::CameraProjection::Parallel,
            other => {
                return Err(ApiError(format!(
                    "BadProjection: unknown projection '{other}'"
                )));
            }
        };
        let eye3 = triple(&eye, "eye")?;
        let target3 = triple(&target, "target")?;
        let up3 = triple(&up, "up")?;
        self.doc.set_camera_state(kernel::CameraState {
            projection: kernel_projection,
            fov_deg,
            eye: Point3::new(eye3[0], eye3[1], eye3[2]),
            target: Point3::new(target3[0], target3[1], target3[2]),
            up: kernel::Vec3::new(up3[0], up3[1], up3[2]),
        });
        recording::record(recording::RecordedCall::SetCameraState {
            projection: projection.to_string(),
            fov_deg,
            eye: eye3,
            target: target3,
            up: up3,
        });
        Ok(())
    }

    // ------------------------------------------------------------ inference

    /// Resolves one snap query. `anchor` is an optional xyz triple;
    /// `lock_axis` is 0/1/2 for X/Y/Z. `constraint_plane`, when present, is a
    /// 6-tuple `[px,py,pz, nx,ny,nz]` (a point on the active drawing plane plus
    /// its normal): only candidates lying on that plane are considered, so
    /// drawing on a face never snaps through the solid to occluded, off-plane
    /// geometry. Returns `undefined` when nothing snaps (tools fall back to
    /// their own plane intersection).
    ///
    /// `precision`, when true, selects [`SnapWeights::uniform`] — every snap
    /// kind pulls equally, so ranking falls back to raw angular distance and a
    /// point the default gravity would swallow (a circle facet's endpoint
    /// beside a quadrant) becomes reachable. Omitted/false selects the shipped
    /// gravity profile. It is a plain boolean because the *kernel* owns the
    /// weighting and the *app* owns the gesture that selects it: which key is
    /// held never crosses this boundary (DEVELOPMENT.md rule 1).
    ///
    /// `cylinder`, when true, interprets `aperture` per [`ApertureMode::Cylinder`]
    /// — a constant world-space radius in meters around the ray, rather than
    /// the default [`ApertureMode::Cone`] half-angle in radians. Parallel
    /// (orthographic) projection's apparent size does not shrink with depth,
    /// so its natural pick tolerance is this constant-radius cylinder rather
    /// than perspective's cone (docs/design/camera.md §1); `snapService.ts`
    /// selects it exactly when the active `CameraRig` projection is parallel.
    /// Omitted/false selects `Cone`, matching every caller before this
    /// parameter existed.
    ///
    /// `soft_axis_aperture_scale`, when present, multiplies the fixed
    /// angular tolerance the soft-axis (`OnAxis`, anchor-relative) candidate
    /// is admitted within (see [`SnapQuery::soft_axis_aperture_scale`]).
    /// Omitted/`None` behaves exactly as before. This is how the app's
    /// magnetic-hysteresis release query (`snapService.ts`) reaches a held
    /// soft-axis snap: unlike every other sticky kind, that candidate's
    /// admission cone is NOT `aperture` above, so widening `aperture` alone
    /// (the release-query trick every other sticky kind uses) has no effect
    /// on it.
    ///
    /// `off_plane_points`, when `true`, keeps precise POINT candidates
    /// (endpoint/midpoint/center/quadrant/intersection) that lie off the
    /// `constraint_plane` instead of filtering them (see
    /// [`SnapQuery::off_plane_points`]). Only a tool that can HONOUR an
    /// off-plane point — the Line tool's plane-mode chain, which re-homes
    /// onto a new sketch plane at commit — may pass `true`; tools that
    /// commit into one frozen plane must leave it unset, or they would
    /// report a snap position they cannot commit. Omitted/`None` is `false`
    /// (the pre-existing filter, unchanged).
    // Scalar xyz args are deliberate boundary ergonomics (docs/DEVELOPMENT.md).
    #[allow(clippy::too_many_arguments)]
    pub fn snap(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
        aperture: f64,
        anchor: Option<Box<[f64]>>,
        lock_axis: Option<u8>,
        constraint_plane: Option<Box<[f64]>>,
        precision: Option<bool>,
        cylinder: Option<bool>,
        soft_axis_aperture_scale: Option<f64>,
        off_plane_points: Option<bool>,
    ) -> Result<Option<SnapJs>, ApiError> {
        let anchor = match anchor {
            None => None,
            Some(a) if a.len() == 3 => Some(Point3::new(a[0], a[1], a[2])),
            Some(_) => {
                return Err(ApiError(
                    "BadAnchor: anchor must be an xyz triple".to_string(),
                ));
            }
        };
        let constraint_plane = match constraint_plane {
            None => None,
            Some(p) if p.len() == 6 => {
                let point = Point3::new(p[0], p[1], p[2]);
                let normal = kernel::Vec3::new(p[3], p[4], p[5]);
                match Plane::from_point_normal(point, normal) {
                    Ok(plane) => Some(plane),
                    Err(_) => {
                        return Err(ApiError(
                            "BadPlane: constraint_plane normal is degenerate".to_string(),
                        ));
                    }
                }
            }
            Some(_) => {
                return Err(ApiError(
                    "BadPlane: constraint_plane must be [px,py,pz, nx,ny,nz]".to_string(),
                ));
            }
        };
        let lock = match lock_axis {
            None => None,
            Some(0) => Some(SnapLock::Axis(Axis::X)),
            Some(1) => Some(SnapLock::Axis(Axis::Y)),
            Some(2) => Some(SnapLock::Axis(Axis::Z)),
            Some(_) => {
                return Err(ApiError(
                    "BadAxis: lock_axis must be 0, 1, or 2".to_string(),
                ));
            }
        };
        let query = SnapQuery {
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor,
            lock,
            aperture,
            aperture_mode: if cylinder.unwrap_or(false) {
                ApertureMode::Cylinder
            } else {
                ApertureMode::Cone
            },
            constraint_plane,
            weights: if precision.unwrap_or(false) {
                SnapWeights::uniform()
            } else {
                SnapWeights::default()
            },
            soft_axis_aperture_scale,
            off_plane_points: off_plane_points.unwrap_or(false),
        };
        Ok(self.inference.resolve(&query).map(|snap| SnapJs { snap }))
    }

    /// Picks the nearest Object face the ray passes through (for the push/pull
    /// tool). Unlike `snap`, this ignores the drawing snap-priority model, so
    /// it reliably returns the surface under the cursor rather than a nearby
    /// vertex or edge. `undefined` when the ray hits no face.
    pub fn pick_face(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<FacePickJs> {
        let ray = PickRay {
            origin: Point3::new(ox, oy, oz),
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        let (source, depth) = self.inference.pick_face(&ray)?;
        match source.element {
            ElementRef::Face(f) => Some(FacePickJs {
                object: source.object.data().as_ffi(),
                face: f.data().as_ffi(),
                instance: source.instance.map(|i| i.data().as_ffi()),
                depth,
            }),
            // pick_face only ever yields faces; anything else is a bug.
            _ => None,
        }
    }

    /// Picks the live (non-hidden) free-standing sketch whose
    /// nearest edge the ray passes closest to (for whole-sketch selection,
    ///) — `undefined` when the ray hits no live sketch edge.
    ///
    /// Like `pick_face`, this takes a bare ray with no caller-supplied
    /// aperture: a sketch edge has no thickness, so a fixed pick-cone half-angle
    /// (`SKETCH_PICK_APERTURE`) stands in for screen-derived aperture (the `snap`
    /// convention) — picking a thin line by exact ray intersection alone would
    /// be unreasonably precise to hit.
    pub fn pick_sketch(&self, ox: f64, oy: f64, oz: f64, dx: f64, dy: f64, dz: f64) -> Option<u64> {
        let ray = PickRay {
            origin: Point3::new(ox, oy, oz),
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        self.inference
            .pick_sketch(&ray, SKETCH_PICK_APERTURE)
            .map(|id| id.data().as_ffi())
    }

    /// Picks the nearest live sketch edge under the ray (same aperture and
    /// ranking as `pick_sketch`), returning both the owning sketch and the
    /// edge — the Select tool's per-edge pick. `undefined` on a miss.
    pub fn pick_sketch_edge(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<SketchEdgePickJs> {
        let ray = PickRay {
            origin: Point3::new(ox, oy, oz),
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        self.inference
            .pick_sketch_edge(&ray, SKETCH_PICK_APERTURE)
            .and_then(|(sid, eid)| {
                let sketch = self.doc.sketch(sid)?;
                let edge = sketch.edges().get(eid)?;
                let a = sketch.vertices()[edge.from].position;
                let b = sketch.vertices()[edge.to].position;
                Some(SketchEdgePickJs {
                    sketch: sid.data().as_ffi(),
                    edge: eid.data().as_ffi(),
                    depth: segment_ray_depth(ray.origin, ray.direction, a, b),
                })
            })
    }

    /// Picks the sketch region under the ray across ALL live sketches:
    /// intersects the ray with each sketch's plane and returns the
    /// smallest-area region whose material contains the hit point (nested
    /// regions resolve to the innermost — the same rule the push/pull tool
    /// always used, now kernel-side and multi-sketch). EVERY closed region
    /// participates and every one is extrudable (interpenetration is allowed
    /// everywhere in Hew). Hidden sketches never match (and an extruded
    /// region cannot: its scaffolding was deleted with it); `undefined` when
    /// nothing is hit.
    ///
    /// The "any sketch" targeting primitive: push/pull region targeting,
    /// select-by-interior, and dock hover all resolve through this, replacing
    /// the app's old single-active-sketch bookkeeping.
    pub fn pick_sketch_region(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<SketchRegionPickJs> {
        let origin = Point3::new(ox, oy, oz);
        let dir = kernel::Vec3::new(dx, dy, dz);
        let mut best: Option<(f64, f64, SketchId, SketchRegionId)> = None;
        for sid in self.doc.sketch_ids() {
            let Some(sketch) = self.doc.sketch(sid) else {
                continue;
            };
            let plane = sketch.plane();
            let denom = plane.normal().dot(dir);
            if denom.abs() < kernel::tol::NORMAL_DIRECTION {
                continue; // ray parallel to (or grazing) this sketch plane
            }
            let t = -plane.signed_distance(origin) / denom;
            if t <= 0.0 {
                continue; // plane is behind the ray origin
            }
            let hit = origin + dir * t;
            // ALL closed regions participate; every one is extrudable
            // (interpenetration is allowed everywhere in Hew).
            for rid in sketch.regions().keys() {
                if !sketch.region_contains_point(rid, hit).unwrap_or(false) {
                    continue;
                }
                let area = sketch.region_area(rid).unwrap_or(f64::INFINITY);
                if best.is_none_or(|(best_t, best_area, _, _)| {
                    t < best_t || ((t - best_t).abs() < kernel::tol::PLANE_DIST && area < best_area)
                }) {
                    best = Some((t, area, sid, rid));
                }
            }
        }
        best.map(|(depth, _, s, r)| SketchRegionPickJs {
            sketch: s.data().as_ffi(),
            region: r.data().as_ffi(),
            depth,
        })
    }

    /// [`Scene::pick_sketch_region`], scoped to ONE component INSTANCE's own
    /// definition-owned sketches (component-edit-parity.md phase A2).
    /// `pick_sketch_region` walks only `Document::sketch_ids()`, which
    /// deliberately excludes every definition-owned sketch (the same
    /// world-tree-only boundary `object_ids()` and `sketch_edge_endpoints`
    /// enforce) — so a plane-mode region drawn INSIDE a component's own
    /// definition (`begin_sketch_on_plane_in_instance`) was otherwise
    /// unreachable by a real click, for push/pull's region-extrude or Follow
    /// Me's sketch-region profile pick alike. The ray is WORLD-space, mapped
    /// through the instance's pose⁻¹ before testing; the returned handles are
    /// DEFINITION-LOCAL, exactly like every other `_in_instance`/
    /// `component_member_*` accessor. `None` on a miss OR a stale/singular
    /// instance — a pick never throws.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_sketch_region_in_instance(
        &self,
        instance: u64,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<SketchRegionPickJs> {
        let iid = instance_id(instance);
        let pose = self.doc.instance_pose(iid)?;
        let component = self.doc.instance_def(iid)?;
        let pose_inv = pose.inverse().ok()?;
        let origin = pose_inv.apply_point(Point3::new(ox, oy, oz));
        let dir = pose_inv.apply_vector(kernel::Vec3::new(dx, dy, dz));

        let mut best: Option<(f64, f64, SketchId, SketchRegionId)> = None;
        for sid in self.doc.def_member_sketches(component).unwrap_or_default() {
            let Some(sketch) = self.doc.sketch(sid) else {
                continue;
            };
            let plane = sketch.plane();
            let denom = plane.normal().dot(dir);
            if denom.abs() < kernel::tol::NORMAL_DIRECTION {
                continue; // ray parallel to (or grazing) this sketch plane
            }
            let t = -plane.signed_distance(origin) / denom;
            if t <= 0.0 {
                continue; // plane is behind the ray origin
            }
            let hit = origin + dir * t;
            for rid in sketch.regions().keys() {
                if !sketch.region_contains_point(rid, hit).unwrap_or(false) {
                    continue;
                }
                let area = sketch.region_area(rid).unwrap_or(f64::INFINITY);
                if best.is_none_or(|(best_t, best_area, _, _)| {
                    t < best_t || ((t - best_t).abs() < kernel::tol::PLANE_DIST && area < best_area)
                }) {
                    best = Some((t, area, sid, rid));
                }
            }
        }
        best.map(|(local_depth, _, s, r)| SketchRegionPickJs {
            sketch: s.data().as_ffi(),
            region: r.data().as_ffi(),
            depth: local_depth * kernel::Vec3::new(dx, dy, dz).length(),
        })
    }

    /// [`Scene::pick_sketch_edge`] scoped to the definition-owned sketches
    /// visible through one component instance. Sketch segments are mapped
    /// through the instance pose into world space before applying the same
    /// cone ranking and aperture as the world-sketch picker. Keeping the cone
    /// in world space preserves its screen-angle meaning under non-uniform
    /// instance scale.
    ///
    /// This is the selection counterpart of
    /// [`Scene::pick_sketch_region_in_instance`]. Keeping the query at this
    /// boundary avoids registering one duplicated set of inference candidates
    /// per component placement merely to support a click.
    #[allow(clippy::too_many_arguments)]
    pub fn pick_sketch_edge_in_instance(
        &self,
        instance: u64,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<SketchEdgePickJs> {
        let iid = instance_id(instance);
        let pose = self.doc.instance_pose(iid)?;
        let component = self.doc.instance_def(iid)?;
        let ray = PickRay {
            origin: Point3::new(ox, oy, oz),
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        let mut local = InferenceScene::default();
        for sid in self.doc.def_member_sketches(component).unwrap_or_default() {
            let Some(sketch) = self.doc.sketch(sid) else {
                continue;
            };
            let segments: Vec<_> = sketch
                .edges()
                .iter()
                .map(|(eid, edge)| {
                    (
                        eid,
                        pose.apply_point(sketch.vertices()[edge.from].position),
                        pose.apply_point(sketch.vertices()[edge.to].position),
                    )
                })
                .collect();
            local.add_sketch(sid, &segments);
        }
        local
            .pick_sketch_edge(&ray, SKETCH_PICK_APERTURE)
            .and_then(|(sid, eid)| {
                let sketch = self.doc.sketch(sid)?;
                let edge = sketch.edges().get(eid)?;
                let a = pose.apply_point(sketch.vertices()[edge.from].position);
                let b = pose.apply_point(sketch.vertices()[edge.to].position);
                Some(SketchEdgePickJs {
                    sketch: sid.data().as_ffi(),
                    edge: eid.data().as_ffi(),
                    depth: segment_ray_depth(ray.origin, ray.direction, a, b),
                })
            })
    }

    /// Picks the committed sketch vertex nearest the ray (Phase D per-vertex
    /// edit), for the EditVertex tool. Uses the same fixed `SKETCH_PICK_APERTURE`
    /// as [`Scene::pick_sketch`] (a vertex is a point — exact ray intersection
    /// would be unhittable). Returns the sketch, the vertex handle to drag, and
    /// its world position, or `undefined` if no vertex is within the aperture.
    pub fn pick_sketch_vertex(
        &self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Option<SketchVertexPickJs> {
        let ray = PickRay {
            origin: Point3::new(ox, oy, oz),
            direction: kernel::Vec3::new(dx, dy, dz),
        };
        self.inference
            .pick_sketch_vertex(&ray, SKETCH_PICK_APERTURE)
            .map(|(sid, vid, pos)| SketchVertexPickJs {
                sketch: sid.data().as_ffi(),
                vertex: vid.data().as_ffi(),
                x: pos.x,
                y: pos.y,
                z: pos.z,
            })
    }

    /// Publishes one transient (in-progress) segment as a snap candidate —
    /// e.g. a point the line tool has placed in its current chain but not yet
    /// committed to the kernel sketch. Additive; tools typically call
    /// `clear_transient_segments` then republish the whole current chain
    /// whenever it changes. `snap`/`resolve` stays `&self`, so a one-frame lag
    /// between publishing here and the next `snap` call is expected.
    #[allow(clippy::too_many_arguments)]
    pub fn add_transient_segment(&mut self, ax: f64, ay: f64, az: f64, bx: f64, by: f64, bz: f64) {
        self.inference
            .add_transient_segment(Point3::new(ax, ay, az), Point3::new(bx, by, bz));
    }

    /// Drops every transient segment published via `add_transient_segment`.
    pub fn clear_transient_segments(&mut self) {
        self.inference.clear_transient();
    }

    /// Set the user-hidden world objects and instances (session-only; this
    /// *replaces* the previous sets). Hidden geometry is dropped from the
    /// inference scene, so it is neither snapped to (`snap`) nor pickable
    /// (`pick_face`); showing it again re-registers it. This is the kernel-side
    /// complement to the renderer hiding the meshes — together they make Hide
    /// fully exclude a solid from interaction, so you can snap to / select the
    /// geometry behind it. Not persisted (not a document concept).
    pub fn set_hidden(&mut self, object_ids: &[u64], instance_ids: &[u64]) {
        self.hidden_objects = object_ids.iter().map(|&h| object_id(h)).collect();
        self.hidden_instances = instance_ids.iter().map(|&h| instance_id(h)).collect();

        // Rebuild inference registration to match the new sets: clear every
        // object/instance candidate wholesale, then register only the visible
        // remainder. Per-id removal here would scan the candidate vectors once
        // per registered owner — quadratic on documents with many instances —
        // while the clear makes every re-registration's replace-semantics
        // removal a fast no-op. Guides and sketches are unaffected by
        // visibility sets and survive the clear.
        self.inference.clear_solids();
        for id in self.doc.visible_object_ids() {
            if !self.hidden_objects.contains(&id)
                && let Some(object) = self.doc.object(id)
            {
                self.inference.add_object(id, object, &Transform::IDENTITY);
            }
        }
        for iid in self.doc.instance_ids() {
            if !self.hidden_instances.contains(&iid) {
                self.register_instance(iid);
            }
        }
        self.refresh_active_definition_inference();
    }

    // ------------------------------------------------------- materials

    /// Add a solid-color material to the palette and return its handle.
    /// Palette additions are not individually undoable — only face assignment
    /// via [`Scene::paint_face`] is.
    pub fn add_material(&mut self, name: String, r: u8, g: u8, b: u8, a: u8) -> u64 {
        let mat = Material::solid(name.clone(), Rgba8::rgba(r, g, b, a));
        let id = self.doc.add_material(mat).data().as_ffi();
        recording::record(recording::RecordedCall::AddMaterial { name, r, g, b, a });
        id
    }

    /// Add a textured material to the palette and return its handle.
    /// `image` is the authored encoded bytes (PNG/JPEG); `format` is `0` = PNG,
    /// `1` = JPEG. `world_w`/`world_h` are the real-world meters one tile covers.
    #[allow(clippy::too_many_arguments)]
    pub fn add_texture_material(
        &mut self,
        name: String,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
        image: &[u8],
        format: u8,
        world_w: f64,
        world_h: f64,
    ) -> Result<u64, ApiError> {
        let fmt = match format {
            0 => ImageFormat::Png,
            1 => ImageFormat::Jpeg,
            _ => {
                return Err(ApiError(
                    "BadFormat: image format must be 0 (PNG) or 1 (JPEG)".to_string(),
                ));
            }
        };
        let texture = Texture {
            image: image.to_vec(),
            format: fmt,
            world_size: [world_w, world_h],
        };
        let mat = Material::textured(name.clone(), Rgba8::rgba(r, g, b, a), texture);
        let id = self.doc.add_material(mat).data().as_ffi();
        recording::record(recording::RecordedCall::AddTextureMaterial {
            name,
            r,
            g,
            b,
            a,
            image: image.to_vec(),
            format,
            world_w,
            world_h,
        });
        Ok(id)
    }

    /// Set an existing palette material's opacity (alpha, 0–255, 255 =
    /// opaque). Applies to flat-color and textured materials alike, since
    /// `color`'s alpha modulates both. Undoable; does not invalidate any
    /// object's mesh/inference cache — alpha is resolved live from the
    /// palette at render time (`Scene::material_info`), unlike a face's
    /// material *assignment*, whose grouping is baked into tessellated
    /// geometry.
    ///
    /// # Errors
    /// - `UnknownMaterial` — material handle is not in the palette.
    pub fn set_material_alpha(&mut self, material: u64, alpha: u8) -> Result<(), ApiError> {
        let mid = MaterialId::from(KeyData::from_ffi(material));
        let change = self.doc.set_material_alpha(mid, alpha).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetMaterialAlpha { material, alpha });
        Ok(())
    }

    /// Handles of all palette materials, in unspecified but stable order.
    pub fn material_ids(&self) -> Vec<u64> {
        self.doc
            .material_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// Information about one material, or `undefined` if the handle is stale.
    pub fn material_info(&self, id: u64) -> Option<MaterialJs> {
        let mid = MaterialId::from(KeyData::from_ffi(id));
        let mat = self.doc.material(mid)?;
        let (world_w, world_h) = mat
            .texture
            .as_ref()
            .map(|t| (t.world_size[0], t.world_size[1]))
            .unwrap_or((1.0, 1.0));
        Some(MaterialJs {
            name: mat.name.clone(),
            r: mat.color.r,
            g: mat.color.g,
            b: mat.color.b,
            a: mat.color.a,
            has_texture: mat.has_texture(),
            world_w,
            world_h,
        })
    }

    /// The raw encoded image bytes of a textured material, or `undefined` if
    /// the handle is stale or the material has no texture.
    pub fn material_texture_bytes(&self, id: u64) -> Option<Vec<u8>> {
        let mid = MaterialId::from(KeyData::from_ffi(id));
        let mat = self.doc.material(mid)?;
        mat.texture.as_ref().map(|t| t.image.clone())
    }

    /// The face's own material and its object's base material — the
    /// eyedropper's readback (paint-tool design §1): Alt-click resolves the
    /// effective material as `face`, else `object_default`, else the Default
    /// swatch (both sentinels), and makes it current. `undefined` if `object`
    /// is stale/hidden or `face` is not in it.
    pub fn face_material(&self, object: u64, face: u64) -> Option<FaceMaterialJs> {
        let oid = object_id(object);
        let fid = FaceId::from(KeyData::from_ffi(face));
        let (face_mat, default_mat) = self.doc.face_material_pair(oid, fid)?;
        Some(FaceMaterialJs {
            face: face_mat.map(|id| id.data().as_ffi()).unwrap_or(u64::MAX),
            object_default: default_mat.map(|id| id.data().as_ffi()).unwrap_or(u64::MAX),
        })
    }

    /// Paint `face` of `object` with `material`. Sentinel `u64::MAX`
    /// resets the face to the default (unpainted) material. Painting is
    /// undoable; the kernel records a `PaintFace` document action. Touching a
    /// definition member repaints the face in every instance of that definition.
    ///
    /// # Errors
    /// - `UnknownObject` — stale or hidden object handle.
    /// - `UnknownFace` — face is not in the object.
    /// - `UnknownMaterial` — material handle is not in the palette (and is not
    ///   the sentinel).
    pub fn paint_face(&mut self, object: u64, face: u64, material: u64) -> Result<(), ApiError> {
        let oid = object_id(object);
        let fid = FaceId::from(KeyData::from_ffi(face));
        let mid = material_id_opt(material);
        let change = self.doc.paint_face(oid, fid, mid).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::PaintFace {
            object,
            face,
            material,
        });
        Ok(())
    }

    /// Set `object`'s **base material** ( follow-up). Sentinel `u64::MAX`
    /// clears it to the renderer's default. A face with no explicit material
    /// resolves to the base, so the whole solid — and faces grown later by
    /// extrude/boolean — render consistently; explicitly painted faces still
    /// override. Undoable; invalidates the object's render cache.
    ///
    /// # Errors
    /// - `UnknownObject` — stale or hidden object handle.
    /// - `UnknownMaterial` — material handle is not in the palette (and is not
    ///   the sentinel).
    pub fn set_object_material(&mut self, object: u64, material: u64) -> Result<(), ApiError> {
        let oid = object_id(object);
        let mid = material_id_opt(material);
        let change = self.doc.set_object_material(oid, mid).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetObjectMaterial { object, material });
        Ok(())
    }

    /// Replace every assignment of `from` with `to` in one atomic step — the
    /// Shift-click "replace everywhere" gesture (paint-tool design §2).
    /// `document_wide: true` sweeps every object (world objects and
    /// component-definition members alike); `false` confines the sweep to
    /// `scope_object` (ignored when `document_wide` is true) —
    /// Ctrl/Cmd+Shift-click. Sentinel `u64::MAX` for `from`/`to` means
    /// "default / unpainted", same convention as `paint_face`; a sentinel
    /// `from` fills every genuinely-unpainted face/object (not one merely
    /// *inheriting* a painted base — see `Document::replace_material`).
    /// Undoable as one document action regardless of how many objects it
    /// touches.
    ///
    /// # Errors
    /// - `UnknownObject` — `document_wide` is false and `scope_object` is a
    ///   stale/hidden handle.
    /// - `UnknownMaterial` — `from`/`to` (when not the sentinel) is not in
    ///   the palette.
    pub fn replace_material(
        &mut self,
        document_wide: bool,
        scope_object: u64,
        from: u64,
        to: u64,
    ) -> Result<(), ApiError> {
        let scope = if document_wide {
            MaterialScope::Document
        } else {
            MaterialScope::Object(object_id(scope_object))
        };
        let from_id = material_id_opt(from);
        let to_id = material_id_opt(to);
        let change = self
            .doc
            .replace_material(scope, from_id, to_id)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::ReplaceMaterial {
            document_wide,
            scope_object,
            from,
            to,
        });
        Ok(())
    }

    /// `face`'s explicit UV positioning frame — the Position Texture tool's
    /// readback (paint-tool design §3). `undefined` if `object`/`face` is
    /// stale; an empty array if the face carries no explicit frame (the
    /// planar-projection default); otherwise 8 floats
    /// `[sx, sy, sz, tx, ty, tz, u0, v0]` (`kernel::UvFrame`'s components,
    /// world-space gradients `s`/`t` then the `u0`/`v0` offsets — see
    /// `UvFrame`'s doc comment for the exact `uv = s·p + u0, t·p + v0`
    /// convention the tool's math must match).
    pub fn face_uv_frame(&self, object: u64, face: u64) -> Option<Vec<f64>> {
        let oid = object_id(object);
        let fid = FaceId::from(KeyData::from_ffi(face));
        let frame = self.doc.face_uv_frame(oid, fid)?;
        Some(match frame {
            None => Vec::new(),
            Some(f) => vec![f.s.x, f.s.y, f.s.z, f.t.x, f.t.y, f.t.z, f.u0, f.v0],
        })
    }

    /// Sets `face`'s UV positioning frame — the kernel commit for one
    /// Position Texture gesture (paint-tool design §3). `frame: None` (or the
    /// JS `undefined`) resets the face to the planar-projection default;
    /// `Some` must be exactly 8 floats, same layout as [`Self::face_uv_frame`].
    /// Undoable; the kernel records a `SetFaceUvFrame` document action storing
    /// the exact prior frame (`Option<UvFrame>`), so undo/redo round-trip
    /// `None <-> Some` in either direction. Works on world objects and
    /// component-definition members alike (repositions the texture in every
    /// instance of that definition) — same reach as `paint_face`.
    ///
    /// # Errors
    /// - `BadUvFrame` — `frame` is `Some` but not exactly 8 floats.
    /// - `DegenerateUvFrame` — the 8 floats parse but describe a degenerate
    ///   frame: a non-finite component, a (near-)zero-length gradient, or
    ///   (near-)parallel `s`/`t` gradients.
    /// - `UnknownObject` — stale or hidden object handle.
    /// - `UnknownFace` — face is not in the object.
    pub fn set_face_uv_frame(
        &mut self,
        object: u64,
        face: u64,
        frame: Option<Vec<f64>>,
    ) -> Result<(), ApiError> {
        let oid = object_id(object);
        let fid = FaceId::from(KeyData::from_ffi(face));
        let uv_frame = match &frame {
            None => None,
            Some(v) => {
                let a: &[f64; 8] = v.as_slice().try_into().map_err(|_| {
                    ApiError::new(
                        "BadUvFrame",
                        "frame must be 8 floats (sx sy sz tx ty tz u0 v0)",
                    )
                })?;
                Some(UvFrame::new(
                    Vec3::new(a[0], a[1], a[2]),
                    Vec3::new(a[3], a[4], a[5]),
                    a[6],
                    a[7],
                ))
            }
        };
        let change = self
            .doc
            .set_face_uv_frame(oid, fid, uv_frame)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::SetFaceUvFrame {
            object,
            face,
            frame: uv_frame.map(|f| recording::UvFrameRecorded {
                sx: f.s.x,
                sy: f.s.y,
                sz: f.s.z,
                tx: f.t.x,
                ty: f.t.y,
                tz: f.t.z,
                u0: f.u0,
                v0: f.v0,
            }),
        });
        Ok(())
    }

    /// The vertex range `[base, count]` one Object face occupies in its
    /// tessellated render mesh (`MeshJs::positions`/`uvs`, both duplicated
    /// per face and laid out in face order — see `RenderMesh`'s doc comment).
    /// `undefined` if `object`/`face` is stale. Lets the Position Texture
    /// tool patch just this face's `uv` attribute range in place while
    /// dragging (docs/DEVELOPMENT.md B4-style targeted refresh) instead of
    /// re-tessellating the whole object on every pointer move — the frame
    /// isn't committed to the document until the gesture ends, so there is
    /// nothing for a real re-tessellation to reflect yet anyway. Computed
    /// from the same cached `RenderMesh` `object_mesh` fills (tessellating
    /// on demand if the cache is cold, e.g. this is the first call for the
    /// object), so it costs nothing extra on the common path.
    pub fn face_mesh_range(&mut self, object: u64, face: u64) -> Option<Vec<u32>> {
        let id = object_id(object);
        if !self.mesh_cache.contains_key(id) {
            let palette = self.doc.materials();
            let obj = self.doc.object(id)?;
            let mesh = tessellate(obj, palette).ok()?;
            self.mesh_cache.insert(id, mesh);
        }
        let fid = FaceId::from(KeyData::from_ffi(face));
        let mesh = &self.mesh_cache[id];
        mesh.face_ranges
            .iter()
            .find(|(f, _, _)| *f == fid)
            .map(|&(_, base, count)| vec![base, count])
    }

    // ------------------------------------------------------------- guides

    /// Adds a construction line: infinite, through `(ox, oy, oz)` along
    /// `(dx, dy, dz)` (normalized on store; need not be unit length as given).
    /// Non-solid, non-sketch — never affects watertightness or rendering as
    /// geometry.
    ///
    /// # Errors
    /// - `DegenerateGuide` — a non-finite coordinate, or a zero-length direction.
    #[allow(clippy::too_many_arguments)]
    pub fn add_guide_line(
        &mut self,
        ox: f64,
        oy: f64,
        oz: f64,
        dx: f64,
        dy: f64,
        dz: f64,
    ) -> Result<u64, ApiError> {
        let id = self
            .doc
            .add_guide_line(Point3::new(ox, oy, oz), kernel::Vec3::new(dx, dy, dz))
            .map_err(doc_err)?;
        self.reconcile(&DocChange {
            guides_touched: vec![id],
            ..Default::default()
        });
        recording::record(recording::RecordedCall::AddGuideLine {
            origin: [ox, oy, oz],
            dir: [dx, dy, dz],
        });
        Ok(id.data().as_ffi())
    }

    /// Adds a construction point at `(x, y, z)`.
    ///
    /// # Errors
    /// - `DegenerateGuide` — a non-finite coordinate.
    pub fn add_guide_point(&mut self, x: f64, y: f64, z: f64) -> Result<u64, ApiError> {
        let id = self
            .doc
            .add_guide_point(Point3::new(x, y, z))
            .map_err(doc_err)?;
        self.reconcile(&DocChange {
            guides_touched: vec![id],
            ..Default::default()
        });
        recording::record(recording::RecordedCall::AddGuidePoint { p: [x, y, z] });
        Ok(id.data().as_ffi())
    }

    /// Deletes (hides) one construction guide. Undoable; the handle stays
    /// valid for redo, mirroring object/instance delete semantics.
    ///
    /// # Errors
    /// - `UnknownGuide` — stale, already-hidden, or foreign handle.
    pub fn delete_guide(&mut self, guide: u64) -> Result<(), ApiError> {
        let change = self.doc.delete_guide(guide_id(guide)).map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteGuide { guide });
        Ok(())
    }

    /// Deletes (hides) every currently visible construction guide in one undo
    /// step (Edit ▸ Delete Guide Lines). A no-op (and not a separate undo
    /// entry) when there are no guides.
    pub fn delete_all_guides(&mut self) -> Result<(), ApiError> {
        let change = self.doc.delete_all_guides().map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteAllGuides);
        Ok(())
    }

    /// Enable/disable snapping to construction guides (View ▸ Guides). A hidden
    /// guide must not snap or flash a cue, so toggling its visibility off also
    /// suppresses its inference candidates. The guides stay registered;
    /// only candidate emission is gated, so re-enabling is instant.
    pub fn set_guides_snappable(&mut self, enabled: bool) {
        self.inference.set_guides_enabled(enabled);
    }

    /// Enable/disable snapping to the world origin/axes (View ▸ Axes). As with
    /// guides, hidden axes must not snap or cue.
    pub fn set_axes_snappable(&mut self, enabled: bool) {
        self.inference.set_axes_enabled(enabled);
    }

    /// Handles of every currently visible construction guide.
    pub fn guide_ids(&self) -> Vec<u64> {
        self.doc
            .guide_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// `"line"` | `"point"`, or `undefined` if `guide` is stale/hidden.
    pub fn guide_kind(&self, guide: u64) -> Option<String> {
        match self.doc.guide(guide_id(guide))? {
            Guide::Line { .. } => Some("line".to_string()),
            Guide::Point { .. } => Some("point".to_string()),
        }
    }

    /// `guide`'s geometry, or `undefined` if stale/hidden: a line yields
    /// `[ox, oy, oz, dx, dy, dz]` (origin + unit direction); a point yields
    /// `[x, y, z]`. Check [`Scene::guide_kind`] to know which shape to expect.
    pub fn guide_geometry(&self, guide: u64) -> Option<Vec<f64>> {
        match self.doc.guide(guide_id(guide))? {
            Guide::Line { origin, direction } => Some(vec![
                origin.x,
                origin.y,
                origin.z,
                direction.x,
                direction.y,
                direction.z,
            ]),
            Guide::Point { position } => Some(vec![position.x, position.y, position.z]),
        }
    }

    // -------------------------------------------------------- annotations
    //
    // Dimension / leader-text annotations (docs/design/dimensions-text.md).
    // Kernel owns the geometry (`crates/kernel/src/annotation.rs`); the app
    // computes and lays out the displayed measurement text. An anchor's node
    // crosses the FFI as an `(i8, u64)` pair: `node_kind < 0` means a
    // free-floating anchor (no node), else it's [`node_id`]'s `0`/`1`/`2`
    // convention. Points/vectors cross as 3-float slices, planes as 6-float
    // `[px,py,pz,nx,ny,nz]` slices — [`slice_object`]'s convention, not a
    // scalar-per-component argument list.

    /// Adds a linear dimension between two anchors (`a`/`b`, each an
    /// optionally-node-attached point), with the dimension line offset out of
    /// the `a`-`b` line by `offset` and drawn in `plane`. `text_override`
    /// replaces the app-computed measurement text when present.
    ///
    /// # Errors
    /// - `BadNodeKind` / `BadPoint` / `BadVec` / `BadPlane` /
    ///   `DegeneratePlane` — malformed FFI arguments.
    /// - `UnknownObject` / `UnknownGroup` / `UnknownInstance` — an anchor
    ///   names a stale/hidden node.
    /// - `DegenerateAnnotation` — a non-finite coordinate, or `a`/`b` coincide.
    #[allow(clippy::too_many_arguments)]
    pub fn add_linear_dimension(
        &mut self,
        a_node_kind: i8,
        a_node_id: u64,
        a_point: &[f64],
        b_node_kind: i8,
        b_node_id: u64,
        b_point: &[f64],
        offset: &[f64],
        plane: &[f64],
        text_override: Option<String>,
    ) -> Result<u64, ApiError> {
        let a = Anchor {
            node: anchor_node(a_node_kind, a_node_id)?,
            point: point3(a_point)?,
        };
        let b = Anchor {
            node: anchor_node(b_node_kind, b_node_id)?,
            point: point3(b_point)?,
        };
        let offset_v = vec3(offset)?;
        let plane_v = plane_slice(plane)?;
        let id = self
            .doc
            .add_linear_dimension(a, b, offset_v, plane_v, text_override.clone())
            .map_err(doc_err)?;
        self.reconcile(&DocChange::default());
        recording::record(recording::RecordedCall::AddLinearDimension {
            a_node_kind,
            a_node_id,
            a_point: [a.point.x, a.point.y, a.point.z],
            b_node_kind,
            b_node_id,
            b_point: [b.point.x, b.point.y, b.point.z],
            offset: [offset_v.x, offset_v.y, offset_v.z],
            plane: plane_to_slice(&plane_v).try_into().expect("6 floats"),
            text_override,
        });
        Ok(id.data().as_ffi())
    }

    /// Adds a radius/diameter dimension measuring the analytic circle/arc
    /// captured at creation (`curve_center`/`curve_radius`/`curve_plane` —
    /// the app resolves this from the drawn geometry; the kernel does not
    /// re-derive it). `kind` is `"radius"` | `"diameter"`.
    ///
    /// # Errors
    /// - `BadNodeKind` / `BadPoint` / `BadVec` / `BadPlane` /
    ///   `DegeneratePlane` / `BadRadialKind` — malformed FFI arguments.
    /// - `UnknownObject` / `UnknownGroup` / `UnknownInstance` — `anchor`
    ///   names a stale/hidden node.
    /// - `DegenerateAnnotation` — a non-finite coordinate, or a non-positive
    ///   `curve_radius`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_radial_dimension(
        &mut self,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: &[f64],
        kind: &str,
        curve_center: &[f64],
        curve_radius: f64,
        curve_plane: &[f64],
        leader_dir: &[f64],
        text_override: Option<String>,
    ) -> Result<u64, ApiError> {
        let anchor = Anchor {
            node: anchor_node(anchor_node_kind, anchor_node_id)?,
            point: point3(anchor_point)?,
        };
        let radial_kind_v = radial_kind(kind)?;
        let curve = CapturedCurve {
            center: point3(curve_center)?,
            radius: curve_radius,
            plane: plane_slice(curve_plane)?,
        };
        let leader_dir_v = vec3(leader_dir)?;
        let id = self
            .doc
            .add_radial_dimension(
                anchor,
                radial_kind_v,
                curve,
                leader_dir_v,
                text_override.clone(),
            )
            .map_err(doc_err)?;
        self.reconcile(&DocChange::default());
        recording::record(recording::RecordedCall::AddRadialDimension {
            anchor_node_kind,
            anchor_node_id,
            anchor_point: [anchor.point.x, anchor.point.y, anchor.point.z],
            kind: kind.to_string(),
            curve_center: [curve.center.x, curve.center.y, curve.center.z],
            curve_radius,
            curve_plane: plane_to_slice(&curve.plane).try_into().expect("6 floats"),
            leader_dir: [leader_dir_v.x, leader_dir_v.y, leader_dir_v.z],
            text_override,
        });
        Ok(id.data().as_ffi())
    }

    /// Adds a free-form leader-text annotation: `anchor` is the point the
    /// leader points to, `offset` places the text relative to it.
    ///
    /// # Errors
    /// - `BadNodeKind` / `BadPoint` / `BadVec` — malformed FFI arguments.
    /// - `UnknownObject` / `UnknownGroup` / `UnknownInstance` — `anchor`
    ///   names a stale/hidden node.
    /// - `DegenerateAnnotation` — a non-finite coordinate.
    pub fn add_leader_text(
        &mut self,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: &[f64],
        offset: &[f64],
        text: String,
    ) -> Result<u64, ApiError> {
        let anchor = Anchor {
            node: anchor_node(anchor_node_kind, anchor_node_id)?,
            point: point3(anchor_point)?,
        };
        let offset_v = vec3(offset)?;
        let id = self
            .doc
            .add_leader_text(anchor, offset_v, text.clone())
            .map_err(doc_err)?;
        self.reconcile(&DocChange::default());
        recording::record(recording::RecordedCall::AddLeaderText {
            anchor_node_kind,
            anchor_node_id,
            anchor_point: [anchor.point.x, anchor.point.y, anchor.point.z],
            offset: [offset_v.x, offset_v.y, offset_v.z],
            text,
        });
        Ok(id.data().as_ffi())
    }

    /// Replaces a live linear dimension's anchors/offset/plane/override in
    /// place (the app re-picking geometry to clear a `detached` flag, or a
    /// drag-offset commit, or a `text_override` edit). Clears `detached`
    /// only when an anchor/offset/plane field actually changed — a
    /// `text_override`-only edit leaves an existing `detached` warning in
    /// place (`Document::update_annotation`).
    ///
    /// # Errors
    /// - `BadNodeKind` / `BadPoint` / `BadVec` / `BadPlane` /
    ///   `DegeneratePlane` — malformed FFI arguments.
    /// - `UnknownObject` / `UnknownGroup` / `UnknownInstance` — an anchor
    ///   names a stale/hidden node.
    /// - `UnknownAnnotation` — stale, hidden, or foreign handle.
    /// - `MismatchedAnnotationKind` — `id` does not currently name a linear
    ///   dimension.
    #[allow(clippy::too_many_arguments)]
    pub fn update_linear_dimension(
        &mut self,
        id: u64,
        a_node_kind: i8,
        a_node_id: u64,
        a_point: &[f64],
        b_node_kind: i8,
        b_node_id: u64,
        b_point: &[f64],
        offset: &[f64],
        plane: &[f64],
        text_override: Option<String>,
    ) -> Result<(), ApiError> {
        let a = Anchor {
            node: anchor_node(a_node_kind, a_node_id)?,
            point: point3(a_point)?,
        };
        let b = Anchor {
            node: anchor_node(b_node_kind, b_node_id)?,
            point: point3(b_point)?,
        };
        let offset_v = vec3(offset)?;
        let plane_v = plane_slice(plane)?;
        let new = Annotation::LinearDimension {
            a,
            b,
            offset: offset_v,
            plane: plane_v,
            text_override: text_override.clone(),
        };
        let change = self
            .doc
            .update_annotation(annotation_id(id), new)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::UpdateLinearDimension {
            id,
            a_node_kind,
            a_node_id,
            a_point: [a.point.x, a.point.y, a.point.z],
            b_node_kind,
            b_node_id,
            b_point: [b.point.x, b.point.y, b.point.z],
            offset: [offset_v.x, offset_v.y, offset_v.z],
            plane: plane_to_slice(&plane_v).try_into().expect("6 floats"),
            text_override,
        });
        Ok(())
    }

    /// Replaces a live radial dimension's anchor/kind/curve/leader/override in
    /// place. Clears `detached` only when an anchor/kind/curve/leader field
    /// actually changed — a `text_override`-only edit leaves an existing
    /// `detached` warning in place, same as [`Scene::update_linear_dimension`].
    ///
    /// # Errors
    /// Same as [`Scene::add_radial_dimension`], plus `UnknownAnnotation` /
    /// `MismatchedAnnotationKind` as [`Scene::update_linear_dimension`].
    #[allow(clippy::too_many_arguments)]
    pub fn update_radial_dimension(
        &mut self,
        id: u64,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: &[f64],
        kind: &str,
        curve_center: &[f64],
        curve_radius: f64,
        curve_plane: &[f64],
        leader_dir: &[f64],
        text_override: Option<String>,
    ) -> Result<(), ApiError> {
        let anchor = Anchor {
            node: anchor_node(anchor_node_kind, anchor_node_id)?,
            point: point3(anchor_point)?,
        };
        let radial_kind_v = radial_kind(kind)?;
        let curve = CapturedCurve {
            center: point3(curve_center)?,
            radius: curve_radius,
            plane: plane_slice(curve_plane)?,
        };
        let leader_dir_v = vec3(leader_dir)?;
        let new = Annotation::RadialDimension {
            anchor,
            kind: radial_kind_v,
            curve,
            leader_dir: leader_dir_v,
            text_override: text_override.clone(),
        };
        let change = self
            .doc
            .update_annotation(annotation_id(id), new)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::UpdateRadialDimension {
            id,
            anchor_node_kind,
            anchor_node_id,
            anchor_point: [anchor.point.x, anchor.point.y, anchor.point.z],
            kind: kind.to_string(),
            curve_center: [curve.center.x, curve.center.y, curve.center.z],
            curve_radius,
            curve_plane: plane_to_slice(&curve.plane).try_into().expect("6 floats"),
            leader_dir: [leader_dir_v.x, leader_dir_v.y, leader_dir_v.z],
            text_override,
        });
        Ok(())
    }

    /// Replaces a live leader-text annotation's anchor/offset/text in place.
    /// Clears `detached` only when the anchor/offset actually changed — an
    /// edit to `text` alone leaves an existing `detached` warning in place,
    /// same as [`Scene::update_linear_dimension`].
    ///
    /// # Errors
    /// Same as [`Scene::add_leader_text`], plus `UnknownAnnotation` /
    /// `MismatchedAnnotationKind` as [`Scene::update_linear_dimension`].
    pub fn update_leader_text(
        &mut self,
        id: u64,
        anchor_node_kind: i8,
        anchor_node_id: u64,
        anchor_point: &[f64],
        offset: &[f64],
        text: String,
    ) -> Result<(), ApiError> {
        let anchor = Anchor {
            node: anchor_node(anchor_node_kind, anchor_node_id)?,
            point: point3(anchor_point)?,
        };
        let offset_v = vec3(offset)?;
        let new = Annotation::LeaderText {
            anchor,
            offset: offset_v,
            text: text.clone(),
        };
        let change = self
            .doc
            .update_annotation(annotation_id(id), new)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::UpdateLeaderText {
            id,
            anchor_node_kind,
            anchor_node_id,
            anchor_point: [anchor.point.x, anchor.point.y, anchor.point.z],
            offset: [offset_v.x, offset_v.y, offset_v.z],
            text,
        });
        Ok(())
    }

    /// Deletes (hides) one annotation. Undoable; the handle stays valid for
    /// redo.
    ///
    /// # Errors
    /// - `UnknownAnnotation` — stale, hidden, or foreign handle.
    pub fn delete_annotation(&mut self, id: u64) -> Result<(), ApiError> {
        let change = self
            .doc
            .delete_annotation(annotation_id(id))
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::DeleteAnnotation { id });
        Ok(())
    }

    /// Handles of every currently live (visible) annotation, in kernel order.
    pub fn annotation_ids(&self) -> Vec<u64> {
        self.doc
            .annotation_ids()
            .iter()
            .map(|id| id.data().as_ffi())
            .collect()
    }

    /// `"linear"` | `"radial"` | `"leader"`, or `undefined` if stale/hidden.
    pub fn annotation_kind(&self, id: u64) -> Option<String> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::LinearDimension { .. } => Some("linear".to_string()),
            Annotation::RadialDimension { .. } => Some("radial".to_string()),
            Annotation::LeaderText { .. } => Some("leader".to_string()),
        }
    }

    /// Whether a live annotation is currently `detached` — the warning-color
    /// render cue (docs/design/dimensions-text.md). `undefined` if
    /// stale/hidden.
    pub fn annotation_detached(&self, id: u64) -> Option<bool> {
        self.doc.annotation_detached(annotation_id(id))
    }

    /// The `which`-th anchor's node-kind tag (`-1` = free anchor, else
    /// [`node_id`]'s `0`/`1`/`2` convention): `which` is `0` for
    /// `radial`/`leader`'s single anchor, or linear's `a`; `1` for linear's
    /// `b`. `undefined` if `id`/`which` don't resolve to an anchor.
    pub fn annotation_anchor_node_kind(&self, id: u64, which: u8) -> Option<i8> {
        let node = match (self.doc.annotation(annotation_id(id))?, which) {
            (Annotation::LinearDimension { a, .. }, 0) => a.node,
            (Annotation::LinearDimension { b, .. }, 1) => b.node,
            (Annotation::RadialDimension { anchor, .. }, 0) => anchor.node,
            (Annotation::LeaderText { anchor, .. }, 0) => anchor.node,
            _ => return None,
        };
        Some(anchor_node_kind_out(node))
    }

    /// The `which`-th anchor's node handle, or `undefined` when that anchor
    /// is free-floating or `id`/`which` don't resolve. See
    /// [`Scene::annotation_anchor_node_kind`] for `which`'s convention.
    pub fn annotation_anchor_node_id(&self, id: u64, which: u8) -> Option<u64> {
        let node = match (self.doc.annotation(annotation_id(id))?, which) {
            (Annotation::LinearDimension { a, .. }, 0) => a.node,
            (Annotation::LinearDimension { b, .. }, 1) => b.node,
            (Annotation::RadialDimension { anchor, .. }, 0) => anchor.node,
            (Annotation::LeaderText { anchor, .. }, 0) => anchor.node,
            _ => return None,
        };
        anchor_node_id_out(node)
    }

    /// The `which`-th anchor's world-space point `[x,y,z]`, or `undefined` if
    /// `id`/`which` don't resolve. See [`Scene::annotation_anchor_node_kind`]
    /// for `which`'s convention.
    pub fn annotation_anchor_point(&self, id: u64, which: u8) -> Option<Vec<f64>> {
        let p = match (self.doc.annotation(annotation_id(id))?, which) {
            (Annotation::LinearDimension { a, .. }, 0) => a.point,
            (Annotation::LinearDimension { b, .. }, 1) => b.point,
            (Annotation::RadialDimension { anchor, .. }, 0) => anchor.point,
            (Annotation::LeaderText { anchor, .. }, 0) => anchor.point,
            _ => return None,
        };
        Some(vec![p.x, p.y, p.z])
    }

    /// A linear dimension's placement offset `[x,y,z]` off the `a`-`b` line,
    /// or a leader-text's text placement offset off its anchor. `undefined`
    /// for a radial dimension (see [`Scene::annotation_leader_dir`]) or a
    /// stale/hidden id.
    pub fn annotation_offset(&self, id: u64) -> Option<Vec<f64>> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::LinearDimension { offset, .. } | Annotation::LeaderText { offset, .. } => {
                Some(vec![offset.x, offset.y, offset.z])
            }
            Annotation::RadialDimension { .. } => None,
        }
    }

    /// A linear dimension's dimension-line plane, `[px,py,pz,nx,ny,nz]`.
    /// `undefined` for any other kind, or a stale/hidden id.
    pub fn annotation_plane(&self, id: u64) -> Option<Vec<f64>> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::LinearDimension { plane, .. } => Some(plane_to_slice(plane)),
            _ => None,
        }
    }

    /// A radial dimension's presentation, `"radius"` | `"diameter"`.
    /// `undefined` for any other kind, or a stale/hidden id.
    pub fn annotation_radial_kind(&self, id: u64) -> Option<String> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::RadialDimension { kind, .. } => Some(radial_kind_str(*kind)),
            _ => None,
        }
    }

    /// A radial dimension's captured analytic circle/arc, `[cx, cy, cz,
    /// radius, px, py, pz, nx, ny, nz]` (center, radius, then the circle's
    /// plane). `undefined` for any other kind, or a stale/hidden id.
    pub fn annotation_curve(&self, id: u64) -> Option<Vec<f64>> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::RadialDimension { curve, .. } => {
                let mut out = vec![curve.center.x, curve.center.y, curve.center.z, curve.radius];
                out.extend(plane_to_slice(&curve.plane));
                Some(out)
            }
            _ => None,
        }
    }

    /// A radial dimension's leader-line direction `[x,y,z]` from its anchor.
    /// `undefined` for any other kind, or a stale/hidden id.
    pub fn annotation_leader_dir(&self, id: u64) -> Option<Vec<f64>> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::RadialDimension { leader_dir, .. } => {
                Some(vec![leader_dir.x, leader_dir.y, leader_dir.z])
            }
            _ => None,
        }
    }

    /// A leader-text annotation's text content. `undefined` for any other
    /// kind, or a stale/hidden id.
    pub fn annotation_text(&self, id: u64) -> Option<String> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::LeaderText { text, .. } => Some(text.clone()),
            _ => None,
        }
    }

    /// A linear/radial dimension's `text_override`, or `undefined` when
    /// unset (or the id is stale/hidden, or names a leader-text — see
    /// [`Scene::annotation_text`] for that kind's content).
    pub fn annotation_text_override(&self, id: u64) -> Option<String> {
        match self.doc.annotation(annotation_id(id))? {
            Annotation::LinearDimension { text_override, .. }
            | Annotation::RadialDimension { text_override, .. } => text_override.clone(),
            Annotation::LeaderText { .. } => None,
        }
    }

    // ------------------------------------------------------------ import

    /// Import COLLADA bytes (+ host-resolved images) into the current document.
    /// Additive: existing geometry is untouched. Returns the `ImportReport` as a
    /// JS object with fields:
    ///   `{ objects_created, watertight, leaky, skipped: [{name, reason}],
    ///      textures_missing: [string], warnings: [string] }`.
    /// `warnings` carries conversion notes — non-manifold meshes import as
    /// open shells split at their non-manifold edges, said out loud.
    ///
    /// `images` is a JS object shaped:
    ///   `{ "<uri>": { bytes: Uint8Array, format: "png" | "jpeg" } }`
    /// Pass `null` / `undefined` when there are no images to resolve.
    ///
    /// # Errors
    /// Throws a `"DAE: <message>"` `JsError` on parse failure.
    pub fn import_dae(&mut self, dae_bytes: &[u8], images: JsValue) -> Result<JsValue, JsError> {
        // ── 1. Parse the images JS object into an ImageMap ────────────────────
        let mut image_map: ImageMap = ImageMap::new();
        if !images.is_null() && !images.is_undefined() {
            let obj = JsObject::from(images.clone());
            let keys = JsObject::keys(&obj);
            for i in 0..keys.length() {
                let key = keys.get(i);
                let uri = key.as_string().unwrap_or_default();
                let entry = Reflect::get(&obj, &key).unwrap_or(JsValue::UNDEFINED);
                if entry.is_undefined() || entry.is_null() {
                    continue;
                }
                // entry = { bytes: Uint8Array, format: "png"|"jpeg" }
                let bytes_val =
                    Reflect::get(&entry, &JsValue::from_str("bytes")).unwrap_or(JsValue::UNDEFINED);
                let format_val = Reflect::get(&entry, &JsValue::from_str("format"))
                    .unwrap_or(JsValue::UNDEFINED);
                if bytes_val.is_undefined() || bytes_val.is_null() {
                    continue;
                }
                let arr = Uint8Array::from(bytes_val);
                let raw: Vec<u8> = arr.to_vec();
                let format = match format_val.as_string().as_deref() {
                    Some("jpeg") | Some("jpg") => ImageFormat::Jpeg,
                    _ => ImageFormat::Png,
                };
                image_map.insert(uri, (raw, format));
            }
        }

        // ── 2. Parse + ingest + reconcile + record (shared with replay) ───────
        let (report, warnings) = self
            .import_dae_core(dae_bytes, &image_map)
            .map_err(|e| JsError::new(&e.0))?;

        // ── 3. Serialize the ImportReport to a plain JS object ────────────────
        Ok(import_report_to_js(&report, &warnings))
    }

    /// [`Scene::import_dae`] minus the JS-value plumbing: parse, ingest
    /// (additive), reconcile, and record. The replay arm re-issues imports
    /// through this (no `JsValue`, so it also runs in native tests).
    fn import_dae_core(
        &mut self,
        dae_bytes: &[u8],
        image_map: &ImageMap,
    ) -> Result<(kernel::ImportReport, Vec<String>), ApiError> {
        let out =
            dae_import::import(dae_bytes, image_map).map_err(|e| ApiError(format!("DAE: {e}")))?;

        let (report, change) = self
            .doc
            .ingest(out.scene, out.textures_missing)
            .map_err(|e| ApiError(format!("DAE: {e}")))?;

        // Reconcile caches (additive — do NOT clear like `load`).
        self.reconcile(&change);

        // Imports push DocAction::Imported and extend the saved document, so
        // they are recorded like any other committed mutation — with the file
        // (and image) bytes embedded, keeping the recording self-contained.
        recording::record(recording::RecordedCall::ImportDae {
            bytes: dae_bytes.to_vec(),
            images: image_map
                .iter()
                .map(|(uri, (bytes, format))| recording::RecordedImage {
                    uri: uri.clone(),
                    bytes: bytes.clone(),
                    format: match format {
                        ImageFormat::Jpeg => 1,
                        _ => 0,
                    },
                })
                .collect(),
        });
        Ok((report, out.warnings))
    }

    /// Import glTF 2.0 / GLB bytes into the current document. Additive: existing
    /// geometry is untouched. Returns the same `ImportReport` JS shape as
    /// [`Scene::import_dae`].
    ///
    /// Resources must be embedded (GLB binary chunk or `data:` URIs); external
    /// file URIs cannot be fetched here and are surfaced in `textures_missing`.
    ///
    /// # Errors
    /// Throws a `"glTF: <message>"` `JsError` on parse failure.
    pub fn import_gltf(&mut self, gltf_bytes: &[u8]) -> Result<JsValue, JsError> {
        let (report, warnings) = self
            .import_gltf_core(gltf_bytes)
            .map_err(|e| JsError::new(&e.0))?;
        Ok(import_report_to_js(&report, &warnings))
    }

    /// [`Scene::import_gltf`] minus the JS-value plumbing (see
    /// [`Scene::import_dae_core`]).
    fn import_gltf_core(
        &mut self,
        gltf_bytes: &[u8],
    ) -> Result<(kernel::ImportReport, Vec<String>), ApiError> {
        let out = gltf_import::import(gltf_bytes).map_err(|e| ApiError(format!("glTF: {e}")))?;

        let (report, change) = self
            .doc
            .ingest(out.scene, out.missing)
            .map_err(|e| ApiError(format!("glTF: {e}")))?;

        // Additive — do NOT clear caches like `load`.
        self.reconcile(&change);

        recording::record(recording::RecordedCall::ImportGltf {
            bytes: gltf_bytes.to_vec(),
        });
        Ok((report, out.warnings))
    }

    /// Import SketchUp 2017 `.skp` bytes into the current document (
    /// clean-room OpenSKP reader —). Additive: existing
    /// geometry is untouched. Returns the same `ImportReport` JS shape as
    /// [`Scene::import_dae`], plus `warnings: [string]` — parser recovery
    /// notes (non-empty means the reader resynced inside a malformed section
    /// and content may be missing; clean SketchUp 2017 files produce none).
    ///
    /// Textures are embedded in the `.skp` container, so there is no images
    /// argument; ones without embedded bytes surface in `textures_missing`.
    ///
    /// # Errors
    /// Throws a `"SKP: <message>"` `JsError` on parse failure. Unsupported
    /// versions (anything but 2017) throw with the file's own version and
    /// "Save As ▸ SketchUp Version 2017" guidance baked into the message.
    pub fn import_skp(&mut self, skp_bytes: &[u8]) -> Result<JsValue, JsError> {
        let (report, warnings) = self
            .import_skp_core(skp_bytes)
            .map_err(|e| JsError::new(&e.0))?;
        Ok(import_report_to_js(&report, &warnings))
    }

    /// [`Scene::import_skp`] minus the JS-value plumbing (see
    /// [`Scene::import_dae_core`]).
    fn import_skp_core(
        &mut self,
        skp_bytes: &[u8],
    ) -> Result<(kernel::ImportReport, Vec<String>), ApiError> {
        let out = skp_import::import(skp_bytes).map_err(|e| ApiError(format!("SKP: {e}")))?;

        let (report, change) = self
            .doc
            .ingest(out.scene, out.textures_missing)
            .map_err(|e| ApiError(format!("SKP: {e}")))?;

        // Additive — do NOT clear caches like `load`.
        self.reconcile(&change);

        recording::record(recording::RecordedCall::ImportSkp {
            bytes: skp_bytes.to_vec(),
        });
        Ok((report, out.warnings))
    }

    /// Import STL bytes into the current document. Additive: existing
    /// geometry is untouched. Returns the same `ImportReport` JS shape as
    /// [`Scene::import_dae`], plus `warnings: [string]` — non-manifold split
    /// notices and a leaky-piece summary (STL never carries units or materials,
    /// so there is nothing else to report missing).
    ///
    /// `unit_scale` is meters-per-STL-unit: STL carries no units of its own,
    /// so the UI's units-chooser prompt decides (millimeters — `0.001` — is
    /// the maker-community default).
    ///
    /// `name` names the imported Objects — the UI passes the picked file's
    /// stem (`bunny.stl` → `"bunny"`, `"bunny (2)"`, …). `None`/blank falls
    /// back to `"Imported"`. STL has no internal object names, so this is the
    /// only source.
    ///
    /// # Errors
    /// Throws a `"STL: <message>"` `JsError` on parse failure or a file with
    /// no usable triangles.
    pub fn import_stl(
        &mut self,
        stl_bytes: &[u8],
        unit_scale: f64,
        name: Option<String>,
    ) -> Result<JsValue, JsError> {
        let (report, warnings) = self
            .import_stl_core(stl_bytes, unit_scale, name)
            .map_err(|e| JsError::new(&e.0))?;
        Ok(import_report_to_js(&report, &warnings))
    }

    /// [`Scene::import_stl`] minus the JS-value plumbing (see
    /// [`Scene::import_dae_core`]).
    fn import_stl_core(
        &mut self,
        stl_bytes: &[u8],
        unit_scale: f64,
        name: Option<String>,
    ) -> Result<(kernel::ImportReport, Vec<String>), ApiError> {
        let out = stl_import::import(stl_bytes, unit_scale, name.as_deref())
            .map_err(|e| ApiError(format!("STL: {e}")))?;

        let (report, change) = self
            .doc
            .ingest(out.scene, out.missing)
            .map_err(|e| ApiError(format!("STL: {e}")))?;

        // Additive — do NOT clear caches like `load`.
        self.reconcile(&change);

        recording::record(recording::RecordedCall::ImportStl {
            bytes: stl_bytes.to_vec(),
            unit_scale,
            name,
        });
        Ok((report, out.warnings))
    }

    // --------------------------------------------------------- persistence

    /// Serialise the entire document to a `.hew` zip container (HEW_FILE_FORMAT.md).
    /// The returned bytes are a self-contained file — pass them to
    /// [`Scene::load`] to restore the document exactly.
    ///
    /// While an explode session is open, the bytes are transparently those
    /// of the document AS IF the session had been closed
    /// ([`kernel::Document::save_for_persistence`]): the user's session
    /// stays open, nothing is recorded anywhere, and the app never has to
    /// interrupt an edit to save — autosave included.
    ///
    /// wasm-bindgen marshals `Vec<u8>` to a JS `Uint8Array`.
    pub fn save(&self) -> Vec<u8> {
        self.doc.save_for_persistence()
    }

    // ------------------------------------------------------------- library

    /// Extracts a selection (`kinds`/`ids` parallel arrays, kind `0` =
    /// object, `1` = group, `2` = instance — [`Scene::make_component`]'s
    /// convention) into standalone `.hew` library-item bytes
    /// ([`kernel::Document::extract_item`]). Read-only on this document.
    ///
    /// `wrap_as_component` wraps a single bare object as a definition plus
    /// an identity instance (the component-item shape). A non-empty `name`
    /// becomes the item's own display name — the single definition's, else
    /// the single root node's. `meta_json`, when given, is a JSON object
    /// written verbatim into the item's document attrs under the
    /// `hew.library` namespace (id, category, keywords, collection, …).
    pub fn extract_item(
        &self,
        kinds: &[u8],
        ids: &[u64],
        wrap_as_component: bool,
        name: Option<String>,
        meta_json: Option<String>,
    ) -> Result<Vec<u8>, ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        let nodes = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let mut item = self
            .doc
            .extract_item(&nodes, wrap_as_component)
            .map_err(doc_err)?;
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            let cids = item.component_ids();
            if cids.len() == 1 {
                let _ = item.set_component_name(cids[0], Some(n));
            } else if let Some(&root) = item.top_level_nodes().first() {
                let _ = item.set_node_name(root, Some(n));
            }
        }
        apply_library_meta(&mut item, meta_json.as_deref())?;
        Ok(item.save())
    }

    /// The whole document as a library "model item": a plain save (open
    /// sessions transparently closed, exactly like [`Scene::save`]) with
    /// `meta_json` stamped into the `hew.library` document attrs.
    pub fn extract_document_item(&self, meta_json: Option<String>) -> Result<Vec<u8>, ApiError> {
        let mut item = Document::load(&self.doc.save_for_persistence())
            .map_err(|e: LoadError| api_err(&e, &e))?;
        apply_library_meta(&mut item, meta_json.as_deref())?;
        Ok(item.save())
    }

    /// One palette material as a library "material item": a `.hew` with an
    /// empty scene and a one-entry palette (texture bytes ride along in the
    /// container as always), with `meta_json` stamped like every item.
    pub fn extract_material_item(
        &self,
        material: u64,
        meta_json: Option<String>,
    ) -> Result<Vec<u8>, ApiError> {
        let mid = material_id_opt(material).ok_or_else(|| {
            ApiError("UnknownMaterial: the default material is not an entry".into())
        })?;
        let mat = self
            .doc
            .material(mid)
            .ok_or_else(|| ApiError("UnknownMaterial: no such palette entry".into()))?
            .clone();
        let mut item = Document::new();
        item.add_material(mat);
        apply_library_meta(&mut item, meta_json.as_deref())?;
        Ok(item.save())
    }

    /// Inserts `.hew` item bytes into the current document at `affine`
    /// (row-major 3×4) — [`kernel::Document::insert_document`]: lossless,
    /// one undo step, materials content-deduplicated. `source_id` +
    /// `content_hash` (both or neither) are the item's library provenance:
    /// with them, a definition this document already carries from the same
    /// item version is REUSED (idempotent re-insert), and created
    /// definitions/roots are stamped for future matches and for
    /// [`Scene::library_placements_json`].
    ///
    /// Returns `{ rootKinds: number[], rootIds: string[] (decimal u64),
    /// definitionsAdded, definitionsReused, materialsAdded, materialsReused,
    /// objectsAdded, guidesAdded, worldSketchesSkipped, annotationsSkipped }`.
    pub fn insert_item(
        &mut self,
        bytes: &[u8],
        affine: &[f64],
        source_id: Option<String>,
        content_hash: Option<String>,
    ) -> Result<JsValue, JsError> {
        let report = self
            .insert_item_core(bytes, affine, source_id, content_hash)
            .map_err(|e| JsError::new(&e.0))?;
        Ok(insert_report_to_js(&report))
    }

    /// [`Scene::insert_item`] minus the JS-value plumbing: load, insert
    /// (additive), reconcile, and record. The replay arm re-issues inserts
    /// through this (no `JsValue`, so it also runs in native tests).
    fn insert_item_core(
        &mut self,
        bytes: &[u8],
        affine: &[f64],
        source_id: Option<String>,
        content_hash: Option<String>,
    ) -> Result<kernel::InsertReport, ApiError> {
        let item = Document::load(bytes).map_err(|e: LoadError| api_err(&e, &e))?;
        let pose = affine_transform(affine)?;
        let provenance = match (&source_id, &content_hash) {
            (Some(s), Some(h)) => Some(kernel::LibraryProvenance {
                source_id: s.clone(),
                content_hash: h.clone(),
            }),
            _ => None,
        };
        let (report, change) = self
            .doc
            .insert_document(&item, &kernel::InsertOptions { pose, provenance })
            .map_err(doc_err)?;
        // Reconcile caches (additive — do NOT clear like `load`).
        self.reconcile(&change);
        // affine_transform validated the length, so this cannot fail.
        let mut rec_affine = [0.0f64; 12];
        rec_affine.copy_from_slice(affine);
        recording::record(recording::RecordedCall::InsertItem {
            bytes: bytes.to_vec(),
            affine: rec_affine,
            source_id,
            content_hash,
        });
        Ok(report)
    }

    /// Copies a material item's palette into the document's, content-
    /// deduplicated ([`kernel::Document::insert_palette`]) — the "Add to
    /// palette" / "Paint with this" action. Returns the resolved material
    /// handles in the item's palette order (existing entries when
    /// content-equal, fresh ones otherwise). Not undoable on its own,
    /// matching [`Scene::add_material`].
    pub fn insert_item_palette(&mut self, bytes: &[u8]) -> Result<Vec<u64>, ApiError> {
        self.insert_item_palette_core(bytes)
    }

    /// [`Scene::insert_item_palette`] minus nothing — split so the replay
    /// arm names the same core path the import cores use.
    fn insert_item_palette_core(&mut self, bytes: &[u8]) -> Result<Vec<u64>, ApiError> {
        let item = Document::load(bytes).map_err(|e: LoadError| api_err(&e, &e))?;
        let ids = self.doc.insert_palette(&item);
        recording::record(recording::RecordedCall::InsertItemPalette {
            bytes: bytes.to_vec(),
        });
        Ok(ids.iter().map(|m| m.data().as_ffi()).collect())
    }

    /// For each material in an item's palette, the handle of a content-equal
    /// entry already in THIS document's palette — the browser's "in palette"
    /// badge. JSON array, item palette order: decimal handle strings, `null`
    /// where nothing matches. Read-only.
    pub fn palette_matches_json(&self, bytes: &[u8]) -> Result<String, ApiError> {
        let item = Document::load(bytes).map_err(|e: LoadError| api_err(&e, &e))?;
        let matches = self.doc.palette_matches(&item);
        let rows: Vec<serde_json::Value> = matches
            .iter()
            .map(|m| match m {
                Some(id) => serde_json::Value::String(id.data().as_ffi().to_string()),
                None => serde_json::Value::Null,
            })
            .collect();
        Ok(serde_json::Value::Array(rows).to_string())
    }

    /// Marks the SOURCE of a just-saved library item
    /// ([`kernel::Document::stamp_library_source`]): the saved selection —
    /// and, for an instance, its definition (`def_sid` = the definition's
    /// stable id in the item file, decimal) — gets the item's `hew.library`
    /// provenance, so "in this model" counts it immediately and a later
    /// insert of the item reuses the definition it was saved from. Not an
    /// undo step (bookkeeping, not a model edit), but it IS a recorded,
    /// serialized mutation.
    pub fn stamp_library_source(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        source_id: &str,
        content_hash: &str,
        def_sid: Option<String>,
    ) -> Result<(), ApiError> {
        self.stamp_library_source_core(kinds, ids, source_id, content_hash, def_sid)
    }

    /// [`Scene::stamp_library_source`]'s body — the replay arm re-issues
    /// stamps through this.
    fn stamp_library_source_core(
        &mut self,
        kinds: &[u8],
        ids: &[u64],
        source_id: &str,
        content_hash: &str,
        def_sid: Option<String>,
    ) -> Result<(), ApiError> {
        if kinds.len() != ids.len() {
            return Err(ApiError(
                "BadNodeList: kinds and ids must be the same length".to_string(),
            ));
        }
        let nodes = kinds
            .iter()
            .zip(ids)
            .map(|(&k, &i)| node_id(k, i))
            .collect::<Result<Vec<_>, _>>()?;
        let prov = kernel::LibraryProvenance {
            source_id: source_id.to_string(),
            content_hash: content_hash.to_string(),
        };
        let def_sid_num =
            match def_sid.as_deref() {
                None => None,
                Some(raw) => Some(raw.parse::<u64>().map_err(|_| {
                    ApiError(format!("BadLibraryMeta: def_sid is not a u64: {raw:?}"))
                })?),
            };
        self.doc.stamp_library_source(&nodes, &prov, def_sid_num);
        recording::record(recording::RecordedCall::StampLibrarySource {
            kinds: kinds.to_vec(),
            ids: ids.to_vec(),
            source_id: source_id.to_string(),
            content_hash: content_hash.to_string(),
            def_sid,
        });
        Ok(())
    }

    /// Every live palette entry's content hash
    /// ([`kernel::material_content_hash`]) as a JSON array of decimal
    /// strings — pushed to the Library window so its "in palette" badge can
    /// compare against item summaries without a live palette handle.
    pub fn palette_content_hashes_json(&self) -> String {
        let hashes: Vec<serde_json::Value> = self
            .doc
            .materials()
            .values()
            .map(|m| serde_json::Value::String(kernel::material_content_hash(m).to_string()))
            .collect();
        serde_json::Value::Array(hashes).to_string()
    }

    /// How many live placements of each library item this document holds —
    /// the browser's "in this model" badge and scope filter. JSON object,
    /// `source_id` → count. A placement is: a live instance whose own
    /// `hew.library` stamp or whose definition's stamp names the source,
    /// or a live stamped group/object (each counted once, instances first).
    pub fn library_placements_json(&self) -> String {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let source_of = |target: &kernel::AttrTarget| -> Option<String> {
            let dict = self.doc.attr_get(target).ok().flatten()?;
            match dict.get("hew.library")?.get("source_id")? {
                kernel::AttrValue::Text(s) => Some(s.clone()),
                _ => None,
            }
        };
        for iid in self.doc.instance_ids() {
            let own = source_of(&kernel::AttrTarget::Entity(kernel::EntityRef::Instance(
                iid,
            )));
            let via_def = own.or_else(|| {
                let def = self.doc.instance_def(iid)?;
                source_of(&kernel::AttrTarget::Entity(kernel::EntityRef::Component(
                    def,
                )))
            });
            if let Some(s) = via_def {
                *counts.entry(s).or_default() += 1;
            }
        }
        for gid in self.doc.group_ids() {
            if let Some(s) = source_of(&kernel::AttrTarget::Entity(kernel::EntityRef::Group(gid))) {
                *counts.entry(s).or_default() += 1;
            }
        }
        for oid in self.doc.visible_object_ids() {
            if let Some(s) = source_of(&kernel::AttrTarget::Entity(kernel::EntityRef::Object(oid)))
            {
                *counts.entry(s).or_default() += 1;
            }
        }
        serde_json::to_string(&counts).expect("string→usize map is always JSON")
    }

    /// Exports the document to `format` (`"stl" | "3mf" | "glb" | "gltf" |
    /// "usdz"`) via `crates/mesh-export` — the same writers `hew-cli` and the
    /// `hew.doc.export` live-API command use, now the desktop app's own
    /// File > Export path too (the app's former TypeScript exporters,
    /// `app/src/io/exporters/{stlExport,threeMfExport,gltfExport}.ts`, are
    /// retired). `segments_per_turn` is the curve re-facet resolution (0 =
    /// stored facets).
    ///
    /// Returns `None` — never a refusal — when nothing survives the
    /// traversal (an empty document, or every object collapsing to
    /// degenerate triangles): the same "nothing to export" the app's own
    /// retired writers reported, now [`mesh_export::ExportError::NothingToExport`]
    /// underneath. Passes `solids_only: false` (`crates/mesh-export`'s
    /// module doc, "Non-solid inclusion"): the app's own solid-gating
    /// dialog (`collectNonSolidObjects`, `app/src/App.tsx`) warns about
    /// non-solid objects before an STL/3MF export, but the export itself
    /// has always included them regardless — this keeps that exact
    /// behavior rather than starting to silently drop geometry.
    ///
    /// wasm-bindgen marshals `Option<Vec<u8>>` to `Uint8Array | undefined`.
    pub fn export(
        &self,
        format: &str,
        segments_per_turn: u32,
    ) -> Result<Option<Vec<u8>>, ApiError> {
        match mesh_export::export(&self.doc, format, segments_per_turn, false) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(mesh_export::ExportError::NothingToExport) => Ok(None),
            Err(e) => Err(ApiError::new(e.name(), &e.message())),
        }
    }

    /// A canonical, deterministic digest of the document's live state
    /// ([`Document::state_hash`],  / docs/DEVELOPMENT.md). Read-only — the oracle
    /// for record/replay, the diagnostic-log op stamps, and the determinism
    /// guard. Two scenes share a hash iff they serialize identically.
    ///
    /// wasm-bindgen marshals `u64` to a JS `BigInt`.
    pub fn state_hash(&self) -> u64 {
        self.doc.state_hash()
    }

    /// Enable/disable kernel **torture mode** (docs/DEVELOPMENT.md): the
    /// Debug Mode toggle (Settings) flips it. When on, the topology validator
    /// runs on every visible object after every op **even in release WASM**
    /// (where the debug `check_invariants` compiles out —), and this Scene
    /// additionally re-tessellates every touched object after each op (the
    /// [`Scene::reconcile`] self-check). Together they surface a flake at the
    /// exact op with a precise log marker. Off by default — interactive cost is
    /// real, so it stays opt-in.
    pub fn set_torture_mode(&mut self, on: bool) {
        self.doc.set_torture_mode(on);
    }

    /// Whether torture mode is currently enabled (see [`Scene::set_torture_mode`]).
    pub fn torture_mode(&self) -> bool {
        self.doc.torture_mode()
    }

    // -------------------------------------------------- recording / replay

    /// Begins recording the committed `Scene` command stream as replayable typed
    /// calls (docs/DEVELOPMENT.md). Begin on a fresh `Scene` so the recording
    /// replays from `Scene::new`. Replaces any prior in-progress recording.
    /// See `docs/DIAGNOSTICS.md`.
    pub fn start_recording(&self) {
        recording::start();
    }

    /// Stops recording; the accumulated calls remain available to
    /// [`Scene::take_recording`].
    pub fn stop_recording(&self) {
        recording::stop();
    }

    /// Whether a recording is currently active.
    pub fn is_recording(&self) -> bool {
        recording::is_active()
    }

    /// Takes the recording so far as a JSON [`Recording`] artifact
    /// (`docs/DIAGNOSTICS.md`): the captured calls plus this document's
    /// current `state_hash` as the replay golden. Clears the recorder's buffer.
    /// The reproducer you attach to a bug and, once fixed, freeze as a CI replay
    /// fixture.
    ///
    /// [`Recording`]: recording::Recording
    pub fn take_recording(&self) -> String {
        let rec = recording::Recording {
            version: recording::RECORDING_FORMAT_VERSION,
            calls: recording::take_calls(),
            golden_hash: self.doc.state_hash(),
        };
        serde_json::to_string(&rec).unwrap_or_else(|_| "{}".to_string())
    }

    /// Replays a [`Recording`] JSON (`docs/DIAGNOSTICS.md`) by re-issuing
    /// each captured call verbatim into **this** scene, then returns the final
    /// `state_hash`. Run on a fresh `Scene` and compare the result to the
    /// recording's `golden_hash`: equality is the regression guarantee.
    ///
    /// Re-issued calls are not themselves recorded. A malformed artifact or a
    /// call that fails to apply surfaces as a thrown error (`REPLAY: …`).
    ///
    /// [`Recording`]: recording::Recording
    pub fn replay(&mut self, json: &str) -> Result<u64, ApiError> {
        use recording::RecordedCall::*;
        let rec: recording::Recording = serde_json::from_str(json)
            .map_err(|e| ApiError::new("REPLAY", &format!("malformed recording: {e}")))?;
        if rec.version != recording::RECORDING_FORMAT_VERSION {
            return Err(ApiError::new(
                "REPLAY",
                &format!(
                    "recording format v{} != supported v{}",
                    rec.version,
                    recording::RECORDING_FORMAT_VERSION
                ),
            ));
        }
        // Opened on the first recorded API envelope, if any — a
        // recording with none pays nothing.
        let mut replay_conn: Option<u32> = None;
        recording::without_capture(|| {
            for call in rec.calls {
                match call {
                    BeginGroundSketch => {
                        self.begin_ground_sketch();
                    }
                    BeginSketchOnPlane {
                        px,
                        py,
                        pz,
                        nx,
                        ny,
                        nz,
                    } => {
                        self.begin_sketch_on_plane(px, py, pz, nx, ny, nz)?;
                    }
                    SketchAddSegment { sketch, a, b } => {
                        self.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])?;
                    }
                    SketchRemoveEdge { sketch, edge } => {
                        self.sketch_remove_edge(sketch, edge)?;
                    }
                    SketchBeginGesture { sketch } => {
                        self.sketch_begin_gesture(sketch)?;
                    }
                    SketchBeginCurve { sketch } => {
                        self.sketch_begin_curve(sketch)?;
                    }
                    SketchBeginCurveWith {
                        sketch,
                        center,
                        radius,
                    } => {
                        self.sketch_begin_curve_with(
                            sketch, center[0], center[1], center[2], radius,
                        )?;
                    }
                    SketchBeginPolygonWith {
                        sketch,
                        center,
                        radius,
                    } => {
                        self.sketch_begin_polygon_with(
                            sketch, center[0], center[1], center[2], radius,
                        )?;
                    }
                    SketchRefacetCurve {
                        sketch,
                        curve,
                        segments,
                    } => {
                        self.sketch_refacet_curve(sketch, curve, segments)?;
                    }
                    SketchEndCurve { sketch } => {
                        self.sketch_end_curve(sketch)?;
                    }
                    SketchEndGesture { sketch } => {
                        self.sketch_end_gesture(sketch)?;
                    }
                    SketchCancelGesture => {
                        self.sketch_cancel_gesture();
                    }
                    ExtrudeRegion {
                        sketch,
                        region,
                        distance,
                    } => {
                        self.extrude_region(sketch, region, distance)?;
                    }
                    FollowMeAlongEdges {
                        sketch,
                        region,
                        path_sketch,
                        path_edges,
                        stop_len,
                        group,
                    } => {
                        self.follow_me_along_edges(
                            sketch,
                            region,
                            path_sketch,
                            path_edges,
                            stop_len,
                            group,
                        )?;
                    }
                    FollowMeAroundFace {
                        sketch,
                        region,
                        path_object,
                        path_face,
                        stop_len,
                        group,
                    } => {
                        self.follow_me_around_face(
                            sketch,
                            region,
                            path_object,
                            path_face,
                            stop_len,
                            group,
                        )?;
                    }
                    FollowMeAroundInstanceFace {
                        sketch,
                        region,
                        instance,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_around_instance_face(
                            sketch,
                            region,
                            instance,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    FollowMeMergedAroundFace {
                        sketch,
                        region,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_merged_around_face(
                            sketch,
                            region,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    FollowMeFaceAlongEdges {
                        profile_object,
                        profile_face,
                        path_sketch,
                        path_edges,
                        stop_len,
                    } => {
                        self.follow_me_face_along_edges(
                            profile_object,
                            profile_face,
                            path_sketch,
                            path_edges,
                            stop_len,
                        )?;
                    }
                    FollowMeFaceAroundFace {
                        profile_object,
                        profile_face,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_face_around_face(
                            profile_object,
                            profile_face,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    SketchOffsetRegion {
                        sketch,
                        region,
                        distance,
                    } => {
                        self.sketch_offset_region(sketch, region, distance)?;
                    }
                    Boolean { op, a, b } => {
                        self.boolean(op, a, b)?;
                    }
                    BooleanNodes {
                        op,
                        a_kind,
                        a,
                        b_kind,
                        b,
                    } => {
                        self.boolean_nodes(op, a_kind, a, b_kind, b)?;
                    }
                    GroupNodes { kinds, ids } => {
                        self.group_nodes(&kinds, &ids)?;
                    }
                    DuplicateNode { kind, id, affine } => {
                        self.duplicate_node(kind, id, &affine)?;
                    }
                    SliceObject { object, plane } => {
                        self.slice_object(object, &plane)?;
                    }
                    TransformObject { object, affine } => {
                        self.transform_object(object, &affine)?;
                    }
                    TransformSelection {
                        kinds,
                        ids,
                        sketches,
                        affine,
                    } => {
                        self.transform_selection(&kinds, &ids, &sketches, &affine)?;
                    }
                    RescaleDocument { factor } => {
                        self.rescale_document(factor)?;
                    }
                    DeleteNode { kind, id } => {
                        self.delete_node(kind, id)?;
                    }
                    DuplicateSelectionArray {
                        kinds,
                        ids,
                        affine,
                        count,
                    } => {
                        self.duplicate_selection_array(&kinds, &ids, &affine, count)?;
                    }
                    SplitFaceInner {
                        object,
                        face,
                        loop_pts,
                        curve,
                    } => match curve {
                        Some(c) => {
                            self.split_face_inner_with_curve(
                                object,
                                face,
                                &loop_pts,
                                &c[..3],
                                c[3],
                            )?;
                        }
                        None => {
                            self.split_face_inner(object, face, &loop_pts)?;
                        }
                    },
                    PushPull {
                        object,
                        face,
                        distance,
                    } => {
                        self.push_pull(object, face, distance)?;
                    }
                    ExtrudeFaceAsNewObject {
                        object,
                        face,
                        distance,
                    } => {
                        self.extrude_face_as_new_object(object, face, distance)?;
                    }
                    SceneUndo => {
                        self.scene_undo()?;
                    }
                    SceneRedo => {
                        self.scene_redo()?;
                    }
                    TransformSketch { sketch, affine } => {
                        self.transform_sketch(sketch, &affine)?;
                    }
                    TransformSketchIsland {
                        sketch,
                        island,
                        affine,
                    } => {
                        self.transform_sketch_island(sketch, island, &affine)?;
                    }
                    CopySketchIslands {
                        sketch,
                        islands,
                        affine,
                    } => {
                        self.copy_sketch_islands(sketch, &islands, &affine)?;
                    }
                    MoveSketchVertex { sketch, vertex, p } => {
                        self.move_sketch_vertex(sketch, vertex, p[0], p[1], p[2])?;
                    }
                    Ungroup { group } => {
                        self.ungroup(group)?;
                    }
                    DeleteSketch { sketch } => {
                        self.delete_sketch(sketch)?;
                    }
                    TransformGroup { group, affine } => {
                        self.transform_group(group, &affine)?;
                    }
                    MakeComponent { kinds, ids } => {
                        self.make_component(&kinds, &ids)?;
                    }
                    PlaceInstance { component, affine } => {
                        self.place_instance(component, &affine)?;
                    }
                    PlaceText {
                        sketch,
                        regions,
                        distance,
                        name,
                        group,
                    } => {
                        self.place_text(sketch, regions, distance, name, group)?;
                    }
                    TransformInstance { instance, affine } => {
                        self.transform_instance(instance, &affine)?;
                    }
                    ExplodeInstance { instance } => {
                        self.explode_instance(instance)?;
                    }
                    MakeUnique { instance } => {
                        self.make_unique(instance)?;
                    }
                    OpenExplodeSession { instance } => {
                        self.open_explode_session(instance)?;
                    }
                    CloseExplodeSession => {
                        self.close_explode_session()?;
                    }
                    OpenGroupSession { group } => {
                        self.open_group_session(group)?;
                    }
                    CloseGroupSession => {
                        self.close_group_session()?;
                    }
                    RescaleSession { factor, anchor } => {
                        self.rescale_session(factor, anchor[0], anchor[1], anchor[2])?;
                    }
                    PushPullInComponent {
                        instance,
                        object,
                        face,
                        distance,
                    } => {
                        self.push_pull_in_component(instance, object, face, distance)?;
                    }
                    SplitFace { object, face, path } => {
                        self.split_face(object, face, &path)?;
                    }
                    BeginSketchOnPlaneInInstance {
                        instance,
                        px,
                        py,
                        pz,
                        nx,
                        ny,
                        nz,
                    } => {
                        self.begin_sketch_on_plane_in_instance(instance, px, py, pz, nx, ny, nz)?;
                    }
                    ExtrudeRegionInInstance {
                        instance,
                        sketch,
                        region,
                        distance,
                    } => {
                        self.extrude_region_in_instance(instance, sketch, region, distance)?;
                    }
                    SplitFaceInInstance {
                        instance,
                        object,
                        face,
                        path,
                    } => {
                        self.split_face_in_instance(instance, object, face, &path)?;
                    }
                    SplitFaceInnerInInstance {
                        instance,
                        object,
                        face,
                        loop_pts,
                        curve,
                    } => match curve {
                        Some(c) => {
                            self.split_face_inner_with_curve_in_instance(
                                instance,
                                object,
                                face,
                                &loop_pts,
                                &c[..3],
                                c[3],
                            )?;
                        }
                        None => {
                            self.split_face_inner_in_instance(instance, object, face, &loop_pts)?;
                        }
                    },
                    DeleteDefMember { component, object } => {
                        self.delete_def_member(component, object)?;
                    }
                    MergeFaces { object, edge } => {
                        self.merge_faces(object, edge)?;
                    }
                    SetNodeName { kind, id, name } => {
                        self.set_node_name(kind, id, name)?;
                    }
                    AddNodeTag { kind, id, path } => {
                        self.add_node_tag(kind, id, path)?;
                    }
                    RenameTag { path, new_path } => {
                        self.rename_tag(path, new_path)?;
                    }
                    RemoveNodeTag { kind, id, path } => {
                        self.remove_node_tag(kind, id, path)?;
                    }
                    AddNodeTagMany { kinds, ids, path } => {
                        self.add_node_tag_many(&kinds, &ids, path)?;
                    }
                    RemoveNodeTagMany { kinds, ids, path } => {
                        self.remove_node_tag_many(&kinds, &ids, path)?;
                    }
                    SetTagHidden { path, hidden } => {
                        self.set_tag_hidden(path, hidden);
                    }
                    DeleteTag { path } => {
                        self.delete_tag(path)?;
                    }
                    SetNodeUserHidden { kind, id, hidden } => {
                        self.set_node_user_hidden(kind, id, hidden)?;
                    }
                    SetCameraState {
                        projection,
                        fov_deg,
                        eye,
                        target,
                        up,
                    } => {
                        self.set_camera_state(
                            &projection,
                            fov_deg,
                            Box::new(eye),
                            Box::new(target),
                            Box::new(up),
                        )?;
                    }
                    SetSectionPlane {
                        origin,
                        normal,
                        active,
                    } => {
                        self.set_section_plane(
                            origin[0], origin[1], origin[2], normal[0], normal[1], normal[2],
                            active,
                        )?;
                    }
                    ClearSectionPlane => {
                        self.clear_section_plane();
                    }
                    AddScene {
                        name,
                        props,
                        camera_json,
                        display_json,
                        after,
                    } => {
                        self.add_scene(name, props, camera_json, display_json, after)?;
                    }
                    UpdateScene {
                        sid,
                        props,
                        camera_json,
                        display_json,
                    } => {
                        self.update_scene(sid, props, camera_json, display_json)?;
                    }
                    SetSceneProps {
                        sid,
                        props,
                        camera_json,
                        display_json,
                    } => {
                        self.set_scene_props(sid, props, camera_json, display_json)?;
                    }
                    RenameScene { sid, name } => {
                        self.rename_scene(sid, name)?;
                    }
                    SetSceneDescription { sid, text } => {
                        self.set_scene_description(sid, text)?;
                    }
                    MoveScene { sid, index } => {
                        self.move_scene(sid, index)?;
                    }
                    RemoveScene { sid } => {
                        self.remove_scene(sid)?;
                    }
                    ApplyScene { sid } => {
                        self.apply_scene(sid)?;
                    }
                    AddMaterial { name, r, g, b, a } => {
                        self.add_material(name, r, g, b, a);
                    }
                    AddTextureMaterial {
                        name,
                        r,
                        g,
                        b,
                        a,
                        image,
                        format,
                        world_w,
                        world_h,
                    } => {
                        self.add_texture_material(
                            name, r, g, b, a, &image, format, world_w, world_h,
                        )?;
                    }
                    SetMaterialAlpha { material, alpha } => {
                        self.set_material_alpha(material, alpha)?;
                    }
                    PaintFace {
                        object,
                        face,
                        material,
                    } => {
                        self.paint_face(object, face, material)?;
                    }
                    SetObjectMaterial { object, material } => {
                        self.set_object_material(object, material)?;
                    }
                    ReplaceMaterial {
                        document_wide,
                        scope_object,
                        from,
                        to,
                    } => {
                        self.replace_material(document_wide, scope_object, from, to)?;
                    }
                    SetFaceUvFrame {
                        object,
                        face,
                        frame,
                    } => {
                        self.set_face_uv_frame(object, face, frame.map(uv_frame_recorded_to_vec))?;
                    }
                    AddGuideLine { origin, dir } => {
                        self.add_guide_line(
                            origin[0], origin[1], origin[2], dir[0], dir[1], dir[2],
                        )?;
                    }
                    AddGuidePoint { p } => {
                        self.add_guide_point(p[0], p[1], p[2])?;
                    }
                    DeleteGuide { guide } => {
                        self.delete_guide(guide)?;
                    }
                    DeleteAllGuides => {
                        self.delete_all_guides()?;
                    }
                    AddLinearDimension {
                        a_node_kind,
                        a_node_id,
                        a_point,
                        b_node_kind,
                        b_node_id,
                        b_point,
                        offset,
                        plane,
                        text_override,
                    } => {
                        self.add_linear_dimension(
                            a_node_kind,
                            a_node_id,
                            &a_point,
                            b_node_kind,
                            b_node_id,
                            &b_point,
                            &offset,
                            &plane,
                            text_override,
                        )?;
                    }
                    AddRadialDimension {
                        anchor_node_kind,
                        anchor_node_id,
                        anchor_point,
                        kind,
                        curve_center,
                        curve_radius,
                        curve_plane,
                        leader_dir,
                        text_override,
                    } => {
                        self.add_radial_dimension(
                            anchor_node_kind,
                            anchor_node_id,
                            &anchor_point,
                            &kind,
                            &curve_center,
                            curve_radius,
                            &curve_plane,
                            &leader_dir,
                            text_override,
                        )?;
                    }
                    AddLeaderText {
                        anchor_node_kind,
                        anchor_node_id,
                        anchor_point,
                        offset,
                        text,
                    } => {
                        self.add_leader_text(
                            anchor_node_kind,
                            anchor_node_id,
                            &anchor_point,
                            &offset,
                            text,
                        )?;
                    }
                    UpdateLinearDimension {
                        id,
                        a_node_kind,
                        a_node_id,
                        a_point,
                        b_node_kind,
                        b_node_id,
                        b_point,
                        offset,
                        plane,
                        text_override,
                    } => {
                        self.update_linear_dimension(
                            id,
                            a_node_kind,
                            a_node_id,
                            &a_point,
                            b_node_kind,
                            b_node_id,
                            &b_point,
                            &offset,
                            &plane,
                            text_override,
                        )?;
                    }
                    UpdateRadialDimension {
                        id,
                        anchor_node_kind,
                        anchor_node_id,
                        anchor_point,
                        kind,
                        curve_center,
                        curve_radius,
                        curve_plane,
                        leader_dir,
                        text_override,
                    } => {
                        self.update_radial_dimension(
                            id,
                            anchor_node_kind,
                            anchor_node_id,
                            &anchor_point,
                            &kind,
                            &curve_center,
                            curve_radius,
                            &curve_plane,
                            &leader_dir,
                            text_override,
                        )?;
                    }
                    UpdateLeaderText {
                        id,
                        anchor_node_kind,
                        anchor_node_id,
                        anchor_point,
                        offset,
                        text,
                    } => {
                        self.update_leader_text(
                            id,
                            anchor_node_kind,
                            anchor_node_id,
                            &anchor_point,
                            &offset,
                            text,
                        )?;
                    }
                    DeleteAnnotation { id } => {
                        self.delete_annotation(id)?;
                    }
                    InsertItem {
                        bytes,
                        affine,
                        source_id,
                        content_hash,
                    } => {
                        self.insert_item_core(&bytes, &affine, source_id, content_hash)?;
                    }
                    InsertItemPalette { bytes } => {
                        self.insert_item_palette_core(&bytes)?;
                    }
                    StampLibrarySource {
                        kinds,
                        ids,
                        source_id,
                        content_hash,
                        def_sid,
                    } => {
                        self.stamp_library_source_core(
                            &kinds,
                            &ids,
                            &source_id,
                            &content_hash,
                            def_sid,
                        )?;
                    }
                    ImportDae { bytes, images } => {
                        let mut image_map: ImageMap = ImageMap::new();
                        for img in images {
                            let format = if img.format == 1 {
                                ImageFormat::Jpeg
                            } else {
                                ImageFormat::Png
                            };
                            image_map.insert(img.uri, (img.bytes, format));
                        }
                        self.import_dae_core(&bytes, &image_map)?;
                    }
                    ImportGltf { bytes } => {
                        self.import_gltf_core(&bytes)?;
                    }
                    ImportSkp { bytes } => {
                        self.import_skp_core(&bytes)?;
                    }
                    ImportStl {
                        bytes,
                        unit_scale,
                        name,
                    } => {
                        self.import_stl_core(&bytes, unit_scale, name)?;
                    }
                    SetAxes { origin, x, y } => {
                        self.set_axes(
                            origin[0], origin[1], origin[2], x[0], x[1], x[2], y[0], y[1], y[2],
                        )?;
                    }
                    Load { bytes } => {
                        self.load_core(&bytes)?;
                    }
                    FollowMeAlongEdgesInInstance {
                        instance,
                        sketch,
                        region,
                        path_sketch,
                        path_edges,
                        stop_len,
                    } => {
                        self.follow_me_along_edges_in_instance(
                            instance,
                            sketch,
                            region,
                            path_sketch,
                            path_edges,
                            stop_len,
                        )?;
                    }
                    FollowMeAroundFaceInInstance {
                        instance,
                        sketch,
                        region,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_around_face_in_instance(
                            instance,
                            sketch,
                            region,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    FollowMeMergedAroundFaceInInstance {
                        instance,
                        sketch,
                        region,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_merged_around_face_in_instance(
                            instance,
                            sketch,
                            region,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    FollowMeFaceAlongEdgesInInstance {
                        instance,
                        profile_object,
                        profile_face,
                        path_sketch,
                        path_edges,
                        stop_len,
                    } => {
                        self.follow_me_face_along_edges_in_instance(
                            instance,
                            profile_object,
                            profile_face,
                            path_sketch,
                            path_edges,
                            stop_len,
                        )?;
                    }
                    FollowMeFaceAroundFaceInInstance {
                        instance,
                        profile_object,
                        profile_face,
                        path_object,
                        path_face,
                        stop_len,
                    } => {
                        self.follow_me_face_around_face_in_instance(
                            instance,
                            profile_object,
                            profile_face,
                            path_object,
                            path_face,
                            stop_len,
                        )?;
                    }
                    BooleanInComponent {
                        component,
                        op,
                        a,
                        b,
                    } => {
                        self.boolean_in_component(component, op, a, b)?;
                    }
                    SliceDefMember {
                        instance,
                        object,
                        plane,
                    } => {
                        self.slice_def_member(instance, object, &plane)?;
                    }
                    TransformDefMember {
                        instance,
                        object,
                        affine,
                    } => {
                        self.transform_def_member(instance, object, &affine)?;
                    }
                    TransformDefSketch {
                        instance,
                        sketch,
                        affine,
                    } => {
                        self.transform_def_sketch(instance, sketch, &affine)?;
                    }
                    TransformDefSketchIsland {
                        instance,
                        sketch,
                        island,
                        affine,
                    } => {
                        self.transform_def_sketch_island(instance, sketch, island, &affine)?;
                    }
                    TransformDefSelection {
                        instance,
                        objects,
                        sketches,
                        island_sketches,
                        islands,
                        affine,
                    } => {
                        self.transform_def_selection(
                            instance,
                            &objects,
                            &sketches,
                            &island_sketches,
                            &islands,
                            &affine,
                        )?;
                    }
                    ApiDispatch { frame } => {
                        // Replayed through a real connection, so the
                        // envelope takes exactly the path it took live —
                        // including the transaction bracket that makes it
                        // one compound undo entry. The connection is
                        // opened lazily and primed once: only mutating
                        // frames are recorded, and the handshake that
                        // precedes them is read-only, so it is never in
                        // the stream and has to be synthesized here.
                        let conn = match replay_conn {
                            Some(id) => id,
                            None => {
                                let id = self.api_connection_open();
                                self.api_dispatch(
                                    id,
                                    r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
                                );
                                self.api_dispatch(
                                    id,
                                    r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
                                );
                                replay_conn = Some(id);
                                id
                            }
                        };
                        let reply = self.api_dispatch(conn, &frame);
                        // A frame is only ever recorded once it has
                        // succeeded, so a refusal here means the replay
                        // has already diverged from the session — fail
                        // loudly rather than hand back a wrong hash.
                        if let Some(reply) = reply
                            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&reply)
                            && let Some(err) = value.get("error")
                        {
                            return Err(ApiError::new(
                                "REPLAY",
                                &format!("recorded API envelope was refused on replay: {err}"),
                            ));
                        }
                    }
                }
            }
            Ok(())
        })?;
        if let Some(id) = replay_conn {
            self.api_connection_close(id);
        }
        Ok(self.doc.state_hash())
    }

    /// Replace this scene's document with one deserialized from `bytes` (a
    /// `.hew` container produced by [`Scene::save`]).
    ///
    /// On success the derived caches are fully rebuilt:
    /// - `mesh_cache` is cleared (every object will re-tessellate on demand).
    /// - `inference` is rebuilt from scratch: every visible world object is
    ///   added at identity, every visible instance is registered at its pose.
    ///
    /// On failure the scene is left **unchanged** — the new document is built
    /// first, and the swap only happens after a successful parse.
    ///
    /// # Errors
    /// Throws a `"CODE: message"` `JsError` on any parse/validation failure,
    /// where CODE is the [`LoadError`] variant name (`NotAContainer`,
    /// `UnsupportedVersion`, `MalformedManifest`, `DanglingReference`,
    /// `MissingAsset`, `Geometry`) — the same boundary convention every
    /// other typed error uses, so the UI's plain-language copy table can
    /// key on it.
    pub fn load(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.load_core(bytes).map_err(|e| JsError::new(&e.0))
    }

    /// [`Scene::load`] minus the JS-error plumbing: parse, swap, rebuild
    /// caches, and record. The replay arm re-issues loads through this.
    fn load_core(&mut self, bytes: &[u8]) -> Result<(), ApiError> {
        let new_doc = Document::load(bytes).map_err(|e: LoadError| api_err(&e, &e))?;

        // Swap is committed only after successful parse.
        self.doc = new_doc;
        self.mesh_cache = SecondaryMap::new();
        self.inference = InferenceScene::new();
        // Hidden sets key by dense ids the new document reuses; drop them so a
        // stale id can't keep a fresh object out of inference.
        self.hidden_objects.clear();
        self.hidden_instances.clear();
        self.active_inference_instance = None;
        self.active_inference_sketches.clear();

        // Movable drawing axes (tool-parity design §4): the fresh
        // `InferenceScene` starts at world identity — sync it to whatever
        // the loaded document's axes actually are before anything snaps.
        self.inference.set_axes_frame(self.doc.axes());

        // Register every visible world object.
        for id in self.doc.visible_object_ids() {
            if let Some(object) = self.doc.object(id) {
                self.inference.add_object(id, object, &Transform::IDENTITY);
            }
        }
        // Register every visible instance's definition members at their poses.
        for iid in self.doc.instance_ids() {
            self.register_instance(iid);
        }
        // Register every visible construction guide.
        for id in self.doc.guide_ids() {
            if let Some(g) = self.doc.guide(id) {
                self.inference.add_guide(id, g);
            }
        }
        // Register every sketch — segments, vertices, and curve rims. This
        // was missing (only objects/instances/guides were registered), so a
        // freshly loaded drawing offered no sketch snaps until its first
        // mutation happened to re-register it.
        for sid in self.doc.sketch_ids() {
            self.register_sketch(sid);
        }

        // A mid-session load replaces the entire saved document, so the
        // recording embeds the bytes: everything after this call stays
        // replayable from a fresh `Scene`.
        recording::record(recording::RecordedCall::Load {
            bytes: bytes.to_vec(),
        });
        Ok(())
    }
}

// --------------------------------------------------------------- M0 demo

/// Render buffers for the M0 demo tetrahedron. Retires when the viewport
/// migrates to `Scene::object_mesh` (pre-approved in docs/DEVELOPMENT.md).
#[wasm_bindgen]
pub struct DemoMesh {
    mesh: RenderMesh,
    watertight: bool,
}

#[wasm_bindgen]
impl DemoMesh {
    /// Triangle vertex positions (xyz per vertex, duplicated per face).
    pub fn positions(&self) -> Vec<f32> {
        self.mesh.positions.clone()
    }

    /// Per-vertex normals, constant across each face.
    pub fn normals(&self) -> Vec<f32> {
        self.mesh.normals.clone()
    }

    /// Triangle indices into `positions`.
    pub fn indices(&self) -> Vec<u32> {
        self.mesh.indices.clone()
    }

    /// Line-segment endpoints (xyz pairs), one segment per unique edge.
    pub fn edge_positions(&self) -> Vec<f32> {
        self.mesh.edge_positions.clone()
    }

    /// Whether the source Object encloses a volume.
    pub fn watertight(&self) -> bool {
        self.watertight
    }
}

/// Builds the M0 demo geometry: a kernel tetrahedron run through tessellate.
#[wasm_bindgen]
pub fn demo_mesh() -> DemoMesh {
    use kernel::MaterialPalette;
    let object = Object::tetrahedron();
    let empty_palette = MaterialPalette::default();
    let mesh = tessellate(&object, &empty_palette)
        .expect("the demo tetrahedron is convex, planar, and hole-free");
    DemoMesh {
        mesh,
        watertight: object.watertight() == WatertightState::Watertight,
    }
}

#[cfg(test)]
mod tests {
    /// Loading a document must re-register its SKETCHES with inference —
    /// segments, vertices, and curve rims alike. This was silently missing
    /// (objects, instances, and guides were registered; sketches were not),
    /// so a freshly loaded drawing offered no sketch snaps until the first
    /// mutation happened to touch it.
    #[test]
    fn load_registers_sketches_with_inference() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        scene.sketch_begin_gesture(sketch).unwrap();
        scene
            .sketch_begin_curve_with(sketch, 1.0, 1.0, 0.0, 0.1)
            .unwrap();
        let n = 24usize;
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / (n as f64);
            (1.0 + 0.1 * a.cos(), 1.0 + 0.1 * a.sin())
        };
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();
        let bytes = scene.save();

        let mut loaded = Scene::new();
        loaded.load_core(&bytes).unwrap();

        // A facet vertex snaps as Endpoint…
        let (vx, vy) = p(0);
        let snap = loaded
            .snap(
                vx, vy, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("loaded sketch vertex snaps");
        assert_eq!(snap.kind(), "endpoint");
        // …and the drawn circle's exact center snaps as Center.
        let snap = loaded
            .snap(
                1.0, 1.0, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("loaded circle center snaps");
        assert_eq!(snap.kind(), "center");
    }

    /// `Scene::export` reaches the exact same shared writer
    /// `crates/mesh-export` gives every host — mirrors
    /// `crates/wasm-api/src/live.rs`'s `export_document_hands_back_bytes_
    /// matching_the_shared_writer`, but for the desktop app's own
    /// interactive path (`Scene::export`) rather than the `hew.doc.export`
    /// dispatch (`LiveHost::export_document`) — the two differ in exactly
    /// one argument (`solids_only`), pinned here.
    #[test]
    fn export_reaches_the_shared_writer_with_solids_only_false() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        draw_rect(&mut scene, sketch, 0.0, 0.0, 1.0, 1.0);
        let region = scene.sketch_regions(sketch).unwrap()[0];
        scene.extrude_region(sketch, region, 1.0).unwrap();

        let bytes = scene
            .export("stl", 0)
            .expect("a solid box exports")
            .expect("bytes come back, not a nothing-to-export null");
        let direct = mesh_export::export(&scene.doc, "stl", 0, false)
            .expect("the same writer, called directly with the same solids_only");
        assert_eq!(
            bytes, direct,
            "Scene::export matches mesh_export::export(..., solids_only: false) byte for byte"
        );

        // USDZ reaches the same shared writer too — the newest format on
        // the same dispatch match arm.
        let usdz_bytes = scene
            .export("usdz", 0)
            .expect("a solid box exports to usdz")
            .expect("bytes come back, not a nothing-to-export null");
        let usdz_direct = mesh_export::export(&scene.doc, "usdz", 0, false)
            .expect("the same writer, called directly with the same solids_only");
        assert_eq!(usdz_bytes, usdz_direct);

        // An unrecognized format still refuses, typed.
        let err = scene.export("obj", 0).unwrap_err();
        assert!(format!("{err:?}").contains("host_capability_missing"));

        // Nothing at all: None, not a refusal.
        let empty = Scene::new();
        assert_eq!(empty.export("stl", 0).unwrap(), None);
    }

    /// Move+Alt's sketch copy is a translated replay through the drawing
    /// surface (one gesture, curve bracket re-opened with the shifted
    /// analytic definition). Even when the copy OVERLAPS its source — so
    /// the sticky rules split both circles at the crossings — both chains
    /// stay true circles: each exact center still snaps as Center.
    #[test]
    fn replay_copied_circle_keeps_center_snaps_even_overlapping() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        let (cx, cy, r, n) = (1.0f64, 1.0f64, 0.1f64, 24usize);
        // Draw the original circle inside a gesture, like CircleTool.
        scene.sketch_begin_gesture(sketch).unwrap();
        scene
            .sketch_begin_curve_with(sketch, cx, cy, 0.0, r)
            .unwrap();
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / (n as f64);
            (cx + r * a.cos(), cy + r * a.sin())
        };
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();

        // Replay-copy translated +0.08 X (overlapping), like duplicateSketchSelection.
        scene.sketch_begin_gesture(sketch).unwrap();
        scene
            .sketch_begin_curve_with(sketch, cx + 0.08, cy, 0.0, r)
            .unwrap();
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax + 0.08, ay, 0.0, bx + 0.08, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();

        // Snap straight down at the copy's center.
        let snap = scene
            .snap(
                cx + 0.08,
                cy,
                3.0,
                0.0,
                0.0,
                -1.0,
                0.002,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
            .expect("something snaps at the copy center");
        assert_eq!(snap.kind(), "center");
        assert!((snap.x() - (cx + 0.08)).abs() < 1e-12);
        assert!((snap.y() - cy).abs() < 1e-12);
        // And the original center still snaps too.
        let snap0 = scene
            .snap(
                cx, cy, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("original center snaps");
        assert_eq!(snap0.kind(), "center");
    }

    /// Regression (the reported bug): selecting a drawn circle to edit its
    /// segment count worked only about half the time — a click that resolved
    /// to a Center or Quadrant snap carried no selectable provenance, so the
    /// app fell through to a ray re-probe and selected the sketch's island
    /// (no Segments field) instead of the curve. This walks a real drawn
    /// circle's rim BY ANGLE through the real `Scene::snap` boundary and
    /// asserts EVERY resolved snap names something the selection resolver can
    /// turn into the curve: a facet's `sketch-edge`, or the curve's own
    /// `sketch-curve` (center / quadrant / tangent). None may be
    /// provenance-less, and the center and every quadrant specifically must
    /// report `sketch-curve` with a real curve handle — the exact cases that
    /// used to fall through.
    #[test]
    fn drawn_circle_rim_snaps_all_select_the_curve() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        let (cx, cy, r, n) = (1.0f64, 2.0f64, 0.5f64, 24usize);
        scene.sketch_begin_gesture(sketch).unwrap();
        scene
            .sketch_begin_curve_with(sketch, cx, cy, 0.0, r)
            .unwrap();
        // Half-facet phase, like CircleTool: no facet vertex lands on a
        // cardinal, so a Quadrant candidate is never shadowed by an Endpoint.
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / (n as f64);
            (cx + r * a.cos(), cy + r * a.sin())
        };
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();

        // The one curve chain's handle — what a correct selection must name.
        let curve = {
            let s = scene.doc.sketch(sketch_id(sketch)).unwrap();
            let rim = s.curve_rims().into_iter().next().unwrap();
            rim.curve.data().as_ffi()
        };

        // Center: the marquee case. Straight down onto the exact center.
        let c = scene
            .snap(
                cx, cy, 3.0, 0.0, 0.0, -1.0, 0.004, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("center snaps");
        assert_eq!(c.kind(), "center");
        assert_eq!(c.element_kind().as_deref(), Some("sketch-curve"));
        assert_eq!(c.sketch(), Some(sketch));
        assert_eq!(c.sketch_curve(), Some(curve));
        assert_eq!(c.element(), None, "a center lies on no edge");

        // Walk the rim by world angle. A modest aperture lets Center/Quadrant
        // gravity win near a cardinal, exactly as the app runs it.
        let steps = 180usize;
        let mut quadrant_hits = 0usize;
        let mut endpoint_hits = 0usize;
        for k in 0..steps {
            let a = 2.0 * std::f64::consts::PI * (k as f64) / (steps as f64);
            let (wx, wy) = (cx + r * a.cos(), cy + r * a.sin());
            let snap = scene
                .snap(
                    wx, wy, 3.0, 0.0, 0.0, -1.0, 0.01, None, None, None, None, None, None, None,
                )
                .unwrap()
                .unwrap_or_else(|| panic!("a point on the rim always snaps (angle {a})"));
            let ek = snap.element_kind();
            // Every rim snap must name the curve, one way or the other.
            assert!(
                matches!(ek.as_deref(), Some("sketch-edge") | Some("sketch-curve")),
                "rim snap at angle {a} is provenance-less ({ek:?}, kind {}) — it would fall \
                 through to the ray re-probe and select the island",
                snap.kind()
            );
            assert_eq!(snap.sketch(), Some(sketch));
            match snap.kind().as_str() {
                // The regressed analytic point: a quadrant names the curve.
                "quadrant" => {
                    quadrant_hits += 1;
                    assert_eq!(snap.element_kind().as_deref(), Some("sketch-curve"));
                    assert_eq!(snap.sketch_curve(), Some(curve));
                    assert_eq!(snap.element(), None);
                }
                // The vertex-side of the same gap: a facet vertex names the
                // curve via `sketch_curve`, and carries NO `element`
                // (`sketch_source`) — so Tape Measure never mistakes a vertex
                // for a direction reference.
                "endpoint" => {
                    endpoint_hits += 1;
                    assert_eq!(snap.element_kind().as_deref(), Some("sketch-curve"));
                    assert_eq!(snap.sketch_curve(), Some(curve));
                    assert_eq!(snap.element(), None);
                }
                // A facet edge keeps its edge provenance (a real direction
                // reference) — and still resolves to the curve via its chain.
                "on-edge" | "midpoint" => {
                    assert_eq!(snap.element_kind().as_deref(), Some("sketch-edge"));
                    assert!(snap.element().is_some());
                }
                other => panic!("unexpected rim snap kind {other} at angle {a}"),
            }
        }
        assert!(
            quadrant_hits >= 4,
            "a full circle walk must pass all four quadrants (saw {quadrant_hits})"
        );
        assert!(
            endpoint_hits > 0,
            "a fine rim walk must land on facet vertices too (saw {endpoint_hits})"
        );
    }

    /// Companion to the circle walk, for the OTHER analytic point that used to
    /// fall through: a drawn regular polygon's centre. A polygon has no rim
    /// (no quadrants, no tangents), so its centre rides its own scene channel
    /// (`add_sketch_polygon_centers`) — and this drives the REAL production
    /// wiring end to end: build the polygon through the `Scene` API (whose
    /// per-segment reconcile re-registers the sketch exactly as the app does),
    /// then snap its centre and confirm it names the polygon's chain, so a
    /// click on it selects the polygon rather than the island. Reverting the
    /// `register_sketch` polygon-centre line (which passes
    /// `s.polygon_centers()` to `add_sketch_polygon_centers`) makes the
    /// centre snap provenance-less and this test fail.
    #[test]
    fn drawn_polygon_center_selects_the_polygon_through_production_wiring() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        let (cx, cy, r, n) = (2.0f64, 1.0f64, 0.6f64, 6usize);
        scene.sketch_begin_gesture(sketch).unwrap();
        let curve = scene
            .sketch_begin_polygon_with(sketch, cx, cy, 0.0, r)
            .unwrap();
        // A hexagon's six sides, drawn corner to corner exactly like the tool.
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            (cx + r * a.cos(), cy + r * a.sin())
        };
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();

        // A polygon offers no rim, so the centre is its sole analytic point.
        let c = scene
            .snap(
                cx, cy, 3.0, 0.0, 0.0, -1.0, 0.004, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("polygon center snaps");
        assert_eq!(c.kind(), "center");
        assert_eq!(
            c.element_kind().as_deref(),
            Some("sketch-curve"),
            "the polygon centre must name its curve, not fall through to the island"
        );
        assert_eq!(c.sketch(), Some(sketch));
        assert_eq!(c.sketch_curve(), Some(curve));
        assert_eq!(c.element(), None, "a centre lies on no edge");
    }

    use super::*;

    #[test]
    fn demo_mesh_has_tetrahedron_buffers() {
        let demo = demo_mesh();
        assert_eq!(demo.positions().len(), 36);
        assert_eq!(demo.normals().len(), 36);
        assert_eq!(demo.indices().len(), 12);
        assert_eq!(demo.edge_positions().len(), 36);
        assert!(demo.watertight());
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    /// Draw one axis-aligned ground rectangle into `sketch` (4 segments).
    fn draw_rect(scene: &mut Scene, sketch: u64, x0: f64, y0: f64, x1: f64, y1: f64) {
        for (a, b) in [
            ([x0, y0], [x1, y0]),
            ([x1, y0], [x1, y1]),
            ([x1, y1], [x0, y1]),
            ([x0, y1], [x0, y0]),
        ] {
            scene
                .sketch_add_segment(sketch, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
    }

    /// The whole drawn rectangle — and the sketch's creation — undo and redo
    /// as ONE scene-level step (the M-sketch-interactability Cmd+Z contract).
    #[test]
    fn drawn_rectangle_is_one_scene_undo_step() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        assert!(!scene.can_scene_undo(), "an empty sketch is not a step");

        scene.sketch_begin_gesture(sketch).unwrap();
        draw_rect(&mut scene, sketch, 0.0, 0.0, 1.0, 1.0);
        scene.sketch_end_gesture(sketch).unwrap();

        assert!(scene.can_scene_undo());
        assert_eq!(scene.sketch_regions(sketch).unwrap().len(), 1);

        scene.scene_undo().unwrap();
        assert!(
            scene.sketch_ids().is_empty(),
            "one undo removes the rectangle AND the sketch it created"
        );
        assert!(!scene.can_scene_undo());
        assert!(scene.can_scene_redo());

        scene.scene_redo().unwrap();
        assert_eq!(scene.sketch_ids(), vec![sketch]);
        assert_eq!(scene.sketch_regions(sketch).unwrap().len(), 1);
    }

    /// `pick_sketch_region` targets ANY live sketch's regions — not just the
    /// most recently drawn one — and resolves nested regions to the
    /// innermost. (An extruded region cannot match: its scaffolding was
    /// deleted with it.)
    #[test]
    fn pick_sketch_region_targets_any_sketch() {
        let mut scene = Scene::new();
        let s1 = scene.begin_ground_sketch();
        draw_rect(&mut scene, s1, 0.0, 0.0, 1.0, 1.0);
        let s2 = scene.begin_ground_sketch();
        draw_rect(&mut scene, s2, 2.0, 0.0, 3.0, 1.0);

        // A downward ray over each rectangle finds its own sketch — including
        // s1, which is NOT the most recent.
        let p1 = scene
            .pick_sketch_region(0.5, 0.5, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert_eq!(p1.sketch(), s1);
        assert_eq!(vec![p1.region()], scene.sketch_regions(s1).unwrap());
        let p2 = scene
            .pick_sketch_region(2.5, 0.5, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert_eq!(p2.sketch(), s2);

        // Empty space and a sideways (plane-parallel) ray both miss.
        assert!(
            scene
                .pick_sketch_region(10.0, 10.0, 5.0, 0.0, 0.0, -1.0)
                .is_none()
        );
        assert!(
            scene
                .pick_sketch_region(0.5, 0.5, 5.0, 1.0, 0.0, 0.0)
                .is_none()
        );

        // An extruded region stops matching: its scaffolding was deleted.
        let r2 = scene.sketch_regions(s2).unwrap()[0];
        scene.extrude_region(s2, r2, 1.0).unwrap();
        assert!(
            scene
                .pick_sketch_region(2.5, 0.5, 5.0, 0.0, 0.0, -1.0)
                .is_none()
        );

        // Nested regions resolve to the innermost (smallest outer area).
        draw_rect(&mut scene, s1, 0.25, 0.25, 0.75, 0.75);
        let inner = scene
            .pick_sketch_region(0.5, 0.5, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert_eq!(inner.sketch(), s1);
        let inner_area_pick = scene
            .pick_sketch_region(0.1, 0.5, 5.0, 0.0, 0.0, -1.0)
            .unwrap();
        assert_ne!(
            inner.region(),
            inner_area_pick.region(),
            "a point between the squares picks the outer ring, not the inner"
        );
    }

    /// The `precision` flag on `Scene::snap` is the ONLY thing that crosses
    /// the boundary for snap gravity — the weighting itself stays in the
    /// kernel, and which key selects it stays in the app (DEVELOPMENT.md
    /// rule 1). This pins that the flag is actually wired: the same ray, the
    /// same aperture, two different answers.
    #[test]
    fn the_precision_flag_selects_uniform_snap_weights() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        let (cx, cy, r, n) = (1.0f64, 1.0f64, 0.1f64, 24usize);
        scene.sketch_begin_gesture(sketch).unwrap();
        scene
            .sketch_begin_curve_with(sketch, cx, cy, 0.0, r)
            .unwrap();
        // Half-facet phase: no vertex lands on a cardinal, so the +X quadrant
        // and the facet vertex beside it are distinct competing candidates.
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / (n as f64);
            (cx + r * a.cos(), cy + r * a.sin())
        };
        for i in 0..n {
            let (ax, ay) = p(i);
            let (bx, by) = p(i + 1);
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_curve(sketch).unwrap();
        scene.sketch_end_gesture(sketch).unwrap();

        // Aim 65% of the way from the +X quadrant to the facet vertex beside
        // it — NEARER the vertex, and both inside the plain aperture.
        let (qx, qy) = (cx + r, cy);
        let (vx, vy) = p(0);
        let (ax, ay) = (qx + 0.65 * (vx - qx), qy + 0.65 * (vy - qy));

        let default_snap = scene
            .snap(
                ax, ay, 3.0, 0.0, 0.0, -1.0, 0.004, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("something snaps");
        assert_eq!(
            default_snap.kind(),
            "quadrant",
            "default gravity: the quadrant out-pulls the nearer facet vertex"
        );

        let precise = scene
            .snap(
                ax,
                ay,
                3.0,
                0.0,
                0.0,
                -1.0,
                0.004,
                None,
                None,
                None,
                Some(true),
                None,
                None,
                None,
            )
            .unwrap()
            .expect("something snaps");
        assert_eq!(
            precise.kind(),
            "endpoint",
            "precision: uniform weights, so the nearer facet vertex wins"
        );
        assert!((precise.x() - vx).abs() < 1e-9 && (precise.y() - vy).abs() < 1e-9);

        // An explicit `false` must behave exactly like an omitted flag.
        let explicit_off = scene
            .snap(
                ax,
                ay,
                3.0,
                0.0,
                0.0,
                -1.0,
                0.004,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
            )
            .unwrap()
            .expect("something snaps");
        assert_eq!(explicit_off.kind(), "quadrant");
    }

    /// FIX A, against the maintainer's real file (`follow-me-2.hew`): hovering
    /// the fill of a standing sketch region resolves to an `OnFace` snap ON the
    /// region's plane, instead of the ray passing through to the ground/box
    /// behind it. Before the fix a sketch region registered no face, so this
    /// same hover snapped to whatever lay beneath (an `Endpoint`/`OnFace` at
    /// y≈0), never the perpendicular shape.
    #[test]
    fn standing_sketch_region_is_a_hoverable_face() {
        use inference::{PickRay, SnapKind, SnapQuery};
        let bytes = include_bytes!("../tests/fixtures/follow-me-2.hew");
        let mut scene = Scene::new();
        scene.load_core(bytes).expect("load");

        // The standing rectangle (sketch id 1) lies on the plane y≈0.14442,
        // spanning x∈[0.10,0.12], z∈[-0.005,0.035]. Hover its centre from the
        // +Y side, close enough that its own edges fall outside the aperture
        // cone (the maintainer's zoom) — so only a face candidate can win.
        let q = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(0.11, 0.30, 0.015),
                direction: kernel::Vec3::new(0.0, -1.0, 0.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        };
        let snap = scene.inference.resolve(&q).expect("a snap over the fill");
        assert_eq!(
            snap.kind,
            SnapKind::OnFace,
            "hovering the fill must snap ON the region's face, not through it"
        );
        // The snap lands on the rectangle's plane (y≈0.14442), NOT on geometry
        // behind it (y≈0): the region occludes what's beneath, like a solid.
        assert!(
            (snap.position.y - 0.14441909951924115).abs() < 1e-9,
            "snap landed at y={}, expected the region plane y≈0.14442",
            snap.position.y
        );

        // Both standing regions are registered as pickable regions (the
        // circle, sketch id 2, resolves through the same primitive Follow Me
        // and Select use).
        let circ = scene
            .pick_sketch_region(0.10, -1.0, 0.015, 0.0, 1.0, 0.0)
            .expect("the circle's fill resolves to its region");
        assert_eq!(circ.sketch(), scene.doc.sketch_ids()[1].data().as_ffi());
    }

    /// FINDING 1 (shared partition edge stays selectable): a rectangle split
    /// by a partition into two adjacent regions has region-interior on BOTH
    /// sides of the partition line — so the earlier region-before-edge chain
    /// made the partition permanently unselectable (region always Some).
    /// The occlusion-aware `resolve` the hover cue uses ranks the edge ABOVE
    /// the region fill, so a click ON the partition resolves to the EDGE,
    /// keeping the "draw a partition, delete it to merge" workflow working.
    #[test]
    fn finding1_shared_partition_edge_resolves_to_the_edge() {
        use inference::{PickRay, SnapKind, SnapQuery};
        let mut scene = Scene::new();
        let s = scene.begin_ground_sketch();
        // A 2×1 outer rectangle plus a partition at x=1 → two 1×1 regions
        // sharing that edge.
        for (a, b) in [
            ([0.0, 0.0], [2.0, 0.0]),
            ([2.0, 0.0], [2.0, 1.0]),
            ([2.0, 1.0], [0.0, 1.0]),
            ([0.0, 1.0], [0.0, 0.0]),
            ([1.0, 0.0], [1.0, 1.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        assert_eq!(
            scene.sketch_regions(s).unwrap().len(),
            2,
            "the partition splits the rectangle into two regions"
        );

        // A point ON the partition line (not its midpoint/endpoint): both a
        // region AND an edge are under the ray — the coordinator's repro.
        let (ox, oy, oz, dx, dy, dz) = (1.0, 0.3, 5.0, 0.0, 0.0, -1.0);
        assert!(
            scene.pick_sketch_region(ox, oy, oz, dx, dy, dz).is_some(),
            "region-interior on both sides of the partition"
        );
        assert!(
            scene.pick_sketch_edge(ox, oy, oz, dx, dy, dz).is_some(),
            "the partition edge is also under the ray"
        );

        // The hover-consistent resolve ranks OnEdge above OnFace, so the click
        // (which now routes through resolve) selects the partition EDGE.
        let q = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.01,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        };
        let snap = scene
            .inference
            .resolve(&q)
            .expect("a snap on the partition");
        assert_eq!(
            snap.kind,
            SnapKind::OnEdge,
            "the partition edge wins over the region fill"
        );
        assert!(
            snap.sketch_source.is_some(),
            "and carries the sketch-edge provenance the Select tool selects"
        );
    }

    /// FINDING 2 (a region in front of a solid): `pick_face` walks only solid
    /// faces, so with a sketch region nearer than a solid along the ray it
    /// returns the SOLID — while the occlusion-aware `resolve` the hover cue
    /// uses returns the nearer REGION. Routing the Select click through
    /// `resolve` makes the click match the cue and select the region.
    #[test]
    fn finding2_region_in_front_of_solid_resolves_to_the_region() {
        use inference::{PickRay, SnapKind, SnapQuery};
        let mut scene = Scene::new();
        // A solid box on the ground (a face somewhere along z ≤ 1).
        let s1 = scene.begin_ground_sketch();
        draw_rect(&mut scene, s1, 0.0, 0.0, 2.0, 2.0);
        let r1 = scene.sketch_regions(s1).unwrap()[0];
        let box_obj = scene.extrude_region(s1, r1, 1.0).unwrap();

        // A sketch region lifted to z = 2 — in FRONT of the box along a
        // downward ray.
        let s2 = scene.begin_ground_sketch();
        draw_rect(&mut scene, s2, 0.5, 0.5, 1.5, 1.5);
        scene
            .transform_sketch(
                s2,
                &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0],
            )
            .unwrap();
        let r2 = scene.sketch_regions(s2).unwrap()[0];

        let (ox, oy, oz, dx, dy, dz) = (1.0, 1.0, 5.0, 0.0, 0.0, -1.0);
        // pick_face is blind to sketch regions → returns the solid behind.
        let pf = scene
            .pick_face(ox, oy, oz, dx, dy, dz)
            .expect("pick_face hits the solid");
        assert_eq!(
            pf.object(),
            box_obj,
            "pick_face returns the solid, not the region"
        );

        // The occlusion-aware resolve returns the nearer region.
        let q = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        };
        let snap = scene.inference.resolve(&q).expect("a snap over the region");
        assert_eq!(snap.kind, SnapKind::OnFace);
        let (sk, rg) = snap
            .sketch_region_source
            .expect("the region's provenance, not the solid's");
        assert_eq!(sk.data().as_ffi(), s2);
        assert_eq!(rg.data().as_ffi(), r2);
        assert!(
            snap.source.is_none(),
            "a region snap has no Object source — the solid did not win"
        );
    }

    /// A provenance-less snap must not clear a solid selection: the world
    /// ORIGIN (like a guide point or axis) registers as an Endpoint — the
    /// STRONGEST kind — so `resolve` returns it, carrying NO selectable
    /// provenance, even over a solid's face. `pick_face` still finds the solid
    /// under the ray, which is what the Select tool's fallback selects (so a
    /// dead-centre click on a pit whose top face sits on the origin selects the
    /// pit rather than clearing).
    #[test]
    fn origin_over_a_solid_snaps_provenance_less_yet_pick_face_finds_the_object() {
        use inference::{PickRay, SnapKind, SnapQuery};
        let mut scene = Scene::new();
        // A rectangle spanning the origin, extruded into a pit — its top face
        // lies on z = 0 through the origin, unoccluded from above.
        let s = scene.begin_ground_sketch();
        draw_rect(&mut scene, s, -1.0, -1.0, 1.0, 1.0);
        let r = scene.sketch_regions(s).unwrap()[0];
        let pit = scene.extrude_region(s, r, -1.0).unwrap();

        let (ox, oy, oz, dx, dy, dz) = (0.0, 0.0, 5.0, 0.0, 0.0, -1.0);
        let q = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        };
        let snap = scene.inference.resolve(&q).expect("a snap at the origin");
        assert_eq!(snap.kind, SnapKind::Endpoint, "the origin wins on kind");
        assert!(
            snap.source.is_none()
                && snap.sketch_source.is_none()
                && snap.sketch_region_source.is_none(),
            "the origin snap carries no selectable provenance"
        );

        // The Select fallback's target: the solid actually under the ray.
        let pf = scene
            .pick_face(ox, oy, oz, dx, dy, dz)
            .expect("pick_face hits the pit");
        assert_eq!(pf.object(), pit);
        // The reported depth (ray origin z=5 to the pit's top face at z=0) lets
        // the drag arm reject a hit beyond its render far plane.
        assert!(
            (pf.depth() - 5.0).abs() < 1e-9,
            "pick_face reports the ray-distance to the hit"
        );
    }

    /// End-to-end: a real kernel `Document` op emits its `kernel::op`
    /// event through the wasm `DrainSubscriber`, stamped with the active gesture
    /// correlation id — proving the kernel→drain seam across the crate boundary.
    #[test]
    fn kernel_op_event_reaches_the_drain_with_correlation() {
        use kernel::{Document, Plane, Point3};
        use tracing::subscriber::with_default;

        log::reset();
        with_default(log::DrainSubscriber, || {
            begin_gesture();
            let mut doc = Document::new();
            let plane = Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ])
            .unwrap();
            let s = doc.add_sketch(plane);
            {
                let sk = doc.sketch_mut(s).unwrap();
                for (a, b) in [
                    (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
                    (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
                    (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
                    (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
                ] {
                    sk.add_segment(a, b).unwrap();
                }
            }
            let r = doc.extrudable_regions(s).unwrap()[0];
            doc.extrude_region(s, r, 2.0).unwrap();
            end_gesture();
        });
        let records: Vec<serde_json::Value> = log::drain_buffer()
            .into_iter()
            .map(|s| serde_json::from_str(&s).unwrap())
            .collect();

        let extrude = records
            .iter()
            .find(|r| r["fields"]["op"] == "extrude_region")
            .expect("the kernel extrude_region event reached the drain");
        assert_eq!(extrude["target"], "kernel::op");
        assert_eq!(extrude["fields"]["distance"], 2.0);
        assert!(
            extrude["corr"].as_u64().unwrap() > 0,
            "the event carries the active gesture correlation id"
        );
    }

    /// A Scene mutation emits the post-op `kernel::cmd` event carrying the
    /// canonical `state_hash` (the reconcile stamp).
    #[test]
    fn scene_mutation_stamps_state_hash_on_the_log() {
        use tracing::subscriber::with_default;

        log::reset();
        with_default(log::DrainSubscriber, || {
            let mut scene = Scene::new();
            scene
                .add_guide_point(1.0, 2.0, 3.0)
                .expect("add guide point");
        });
        let cmd = log::drain_buffer()
            .into_iter()
            .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap())
            .find(|r| r["target"] == "kernel::cmd")
            .expect("a committed Scene mutation emits a kernel::cmd event");
        assert!(
            cmd["fields"]["state_hash"].as_u64().is_some(),
            "the cmd event carries a numeric post-op state_hash"
        );
    }

    /// Torture mode: the wasm accessor forwards to the kernel flag, a
    /// normal op still commits with it on, and a valid op emits **no**
    /// `kernel::torture` error marker (the re-tessellation self-check passes —
    /// the marker fires only on a genuine flake).
    #[test]
    fn torture_mode_runs_the_self_check_without_false_positives() {
        use tracing::subscriber::with_default;

        log::reset();
        with_default(log::DrainSubscriber, || {
            let mut scene = Scene::new();
            assert!(!scene.torture_mode(), "off by default");
            scene.set_torture_mode(true);
            assert!(scene.torture_mode());

            let (s, r) = ground_unit_square(&mut scene);
            scene
                .extrude_region(s, r, 2.0)
                .expect("extrude commits with torture on");

            scene.set_torture_mode(false);
            assert!(!scene.torture_mode());
        });
        let torture_failures = log::drain_buffer()
            .into_iter()
            .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap())
            .filter(|r| r["target"] == "kernel::torture")
            .count();
        assert_eq!(
            torture_failures, 0,
            "a valid op produces no torture self-check failure marker"
        );
    }

    #[test]
    fn empty_scene_has_no_objects_and_rejects_stale_handles() {
        let scene = Scene::new();
        assert!(scene.object_ids().is_empty());
        assert!(scene.object_watertight(42).is_err());
        assert!(!scene.can_scene_undo());
        assert!(!scene.can_scene_redo());
    }

    /// End-to-end: record a real multi-op Scene session, then replay the
    /// artifact verbatim into a *fresh* Scene and assert the final `state_hash`
    /// matches the recorded golden — the regression guarantee, and empirical
    /// proof that deterministic handles survive verbatim replay (no remap).
    #[test]
    fn record_then_replay_reproduces_the_golden_state_hash() {
        recording::reset();

        // Record: two boxes, union them, slice the result.
        let mut scene = Scene::new();
        scene.start_recording();
        assert!(scene.is_recording());

        let (s1, r1) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s1, r1, 2.0).unwrap();
        // b is drawn offset and moved to (0.5, 0.5) so it overlaps a, then
        // union.
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let b = scene.extrude_region(s2, r2, 1.0).unwrap();
        scene
            .transform_object(
                b,
                &[1.0, 0.0, 0.0, -1.5, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0],
            )
            .unwrap();
        let _u = scene.boolean(0, a, b).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        // The artifact reports its golden and is the right format version.
        let rec: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(rec["version"], recording::RECORDING_FORMAT_VERSION);
        assert_eq!(rec["golden_hash"].as_u64().unwrap(), golden);
        assert!(
            rec["calls"].as_array().unwrap().len() >= 10,
            "the full call stream (sketch segments + extrudes + transform + boolean) was captured"
        );

        // Replay into a fresh scene: same final state_hash, byte-identical save.
        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying the recording reproduces the golden state_hash"
        );
        assert_eq!(
            replayed.save(),
            scene.save(),
            "replay reproduces byte-identical document bytes"
        );
        // Replaying must not itself record.
        assert!(!replayed.is_recording());
    }

    /// A library insert is a committed document mutation: it must be
    /// captured with the item bytes, pose, and provenance embedded, so a
    /// recorded session replays self-contained and reproduces the same
    /// definition reuse-vs-copy decision — plus a later undo against the
    /// same stack shape. Also pins the extract half: extracting is
    /// read-only and records nothing.
    #[test]
    fn library_insert_records_and_replays() {
        recording::reset();

        // Author an item in a scratch scene: one box, wrapped as a
        // component item.
        let mut author = Scene::new();
        let (s, r) = ground_unit_square(&mut author);
        let obj = author.extrude_region(s, r, 1.0).unwrap();
        let item_bytes = author
            .extract_item(
                &[0u8],
                &[obj],
                true,
                Some("Chair".to_string()),
                Some(r#"{"id":"lib-1","category":"component"}"#.to_string()),
            )
            .unwrap();

        // Record a session that inserts the item twice with the same
        // provenance (idempotent path exercised), then undoes once.
        let mut scene = Scene::new();
        scene.start_recording();
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let shifted = [1.0, 0.0, 0.0, 3.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let first = scene
            .insert_item_core(
                &item_bytes,
                &identity,
                Some("lib-1".into()),
                Some("hash-1".into()),
            )
            .unwrap();
        assert_eq!(first.definitions_added, 1);
        let second = scene
            .insert_item_core(
                &item_bytes,
                &shifted,
                Some("lib-1".into()),
                Some("hash-1".into()),
            )
            .unwrap();
        assert_eq!(second.definitions_reused, 1, "same item version → reuse");
        scene.scene_undo().unwrap();
        scene.stop_recording();

        // Placement counts see the one remaining placement.
        let counts: serde_json::Value =
            serde_json::from_str(&scene.library_placements_json()).unwrap();
        assert_eq!(counts["lib-1"], serde_json::json!(1));

        let golden = scene.state_hash();
        let json = scene.take_recording();
        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(final_hash, golden, "insert+undo replays to the golden hash");
        assert_eq!(replayed.save(), scene.save());
    }

    /// The UI routes EVERY boolean through `boolean_nodes` — plain
    /// object–object subtracts included — so the recorder must capture it,
    /// or the bug-report bundle silently loses the whole boolean feature and
    /// a session with a later undo replays against a differently-shaped undo
    /// stack (adversarial review, critical). Red-checks by removing the
    /// `RecordedCall::BooleanNodes` capture — replay then diverges.
    #[test]
    fn ui_boolean_route_records_and_replays() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        // extrude → extrude → subtract (plain objects, the UI route) → undo →
        // redo. The undo/redo pin the stack shape: if the boolean is missing
        // from the stream, the replayed undo pops a different action.
        let (s1, r1) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s1, r1, 2.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let b = scene.extrude_region(s2, r2, 1.0).unwrap();
        scene
            .transform_object(
                b,
                &[1.0, 0.0, 0.0, -1.5, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.25],
            )
            .unwrap();
        let sub = scene.boolean_nodes(1, 0, a, 0, b).unwrap();
        assert_eq!(sub.kind(), "object");
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let live_count = scene.object_ids().len();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            replayed.object_ids().len(),
            live_count,
            "replay reproduces the live object count"
        );
        assert_eq!(
            final_hash, golden,
            "replaying a UI boolean session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save());
    }

    /// Push/Pull's Ctrl-extrude modifier and the Tape Measure rescale
    /// (tool-parity §2/§3) both record and replay — an undo/redo pair pins
    /// the stack shape for each, the way `ui_boolean_route_records_and_replays`
    /// pins the boolean route.
    #[test]
    fn ctrl_extrude_and_rescale_record_and_replay() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        let boss = scene.extrude_face_as_new_object(obj, top, 1.0).unwrap();
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        scene.rescale_document(2.0).unwrap();
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let live_ids = scene.object_ids();
        assert!(live_ids.contains(&obj));
        assert!(live_ids.contains(&boss));
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying a Ctrl-extrude + rescale session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save());
    }

    /// A whole group session — group, duplicate the group, group-boolean —
    /// records and replays to the exact same state. Covers the structural
    /// calls (`group_nodes`, `duplicate_node`, `boolean_nodes`) added to the
    /// recording set together (adversarial review; the first two were a
    /// pre-existing gap).
    #[test]
    fn group_session_records_and_replays() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (s1, r1) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let b = scene.extrude_region(s2, r2, 1.0).unwrap();
        let g = scene.group_nodes(&[0, 0], &[a, b]).unwrap();
        let copy = scene
            .duplicate_node(
                1,
                g,
                &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 0.0, 0.0, 1.0, 0.0],
            )
            .unwrap();
        assert_eq!(copy.kind(), "group");
        // Union the source group with its copy: all four boxes are disjoint,
        // so the result is a result group of four solids.
        let root = scene.boolean_nodes(0, 1, g, 1, copy.id()).unwrap();
        assert_eq!(root.kind(), "group");
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let live_count = scene.object_ids().len();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(replayed.object_ids().len(), live_count);
        assert_eq!(
            final_hash, golden,
            "replaying a group session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save());
    }

    /// The Move tool's array copy across the FFI: `duplicate_selection_array`
    /// creates `count` copies in creation order as ONE scene-level undo step,
    /// and rejects bad input with typed codes, the document untouched.
    #[test]
    fn duplicate_selection_array_is_one_undo_step() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();

        // 3 copies at +2 m, +4 m, +6 m along X.
        let step = [
            1.0, 0.0, 0.0, 2.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let roots = scene
            .duplicate_selection_array(&[0], &[o], &step, 3)
            .unwrap();
        assert_eq!(roots.len(), 3);
        assert!(roots.iter().all(|n| n.kind == "object"));
        assert_eq!(scene.object_ids().len(), 4, "source + three copies");

        // ONE undo removes the whole array; ONE redo restores it.
        scene.scene_undo().unwrap();
        assert_eq!(scene.object_ids(), vec![o]);
        scene.scene_redo().unwrap();
        assert_eq!(scene.object_ids().len(), 4);

        // Typed rejections, document untouched.
        let err = scene
            .duplicate_selection_array(&[0], &[o], &step, 0)
            .unwrap_err();
        assert!(err.0.starts_with("BadCount"), "got {}", err.0);
        let err = scene
            .duplicate_selection_array(&[0, 0], &[o], &step, 1)
            .unwrap_err();
        assert!(err.0.starts_with("BadNodeList"), "got {}", err.0);
        let err = scene
            .duplicate_selection_array(&[0], &[o], &step[..7], 1)
            .unwrap_err();
        assert!(err.0.starts_with("BadAffine"), "got {}", err.0);
        let err = scene
            .duplicate_selection_array(&[], &[], &step, 1)
            .unwrap_err();
        assert!(err.0.starts_with("EmptySelection"), "got {}", err.0);
        assert_eq!(scene.object_ids().len(), 4, "refusals mutate nothing");
    }

    /// The count cap is enforced at the trust boundary: exactly
    /// `MAX_ARRAY_COUNT` copies succeed, one more refuses typed with the
    /// document untouched — and a hand-edited recording carrying an absurd
    /// count fails its replay loudly instead of hanging the engine.
    #[test]
    fn duplicate_selection_array_bounds_count_at_the_boundary() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let step = [
            1.0, 0.0, 0.0, 2.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];

        // cap + 1 refuses typed, document untouched.
        let err = scene
            .duplicate_selection_array(&[0], &[o], &step, MAX_ARRAY_COUNT + 1)
            .unwrap_err();
        assert!(err.0.starts_with("BadCount"), "got {}", err.0);
        assert_eq!(scene.object_ids().len(), 1, "refusal mutates nothing");
        // No stray action was pushed: undoing once retracts the EXTRUDE.
        scene.scene_undo().unwrap();
        assert!(
            scene.object_ids().is_empty(),
            "refusal pushed no undo entry"
        );
        scene.scene_redo().unwrap();

        // Exactly the cap succeeds.
        let roots = scene
            .duplicate_selection_array(&[0], &[o], &step, MAX_ARRAY_COUNT)
            .unwrap();
        assert_eq!(roots.len(), MAX_ARRAY_COUNT as usize);
        assert_eq!(scene.object_ids().len(), 1 + MAX_ARRAY_COUNT as usize);

        // A recording with an absurd count fails loudly on replay.
        let rogue = format!(
            r#"{{"version":{},"calls":[{{"method":"begin_ground_sketch"}},{{"method":"duplicate_selection_array","kinds":[0],"ids":[1],"affine":[1,0,0,2,0,1,0,0,0,0,1,0],"count":4000000}}],"golden_hash":0}}"#,
            recording::RECORDING_FORMAT_VERSION
        );
        let err = Scene::new().replay(&rogue).unwrap_err();
        assert!(err.0.starts_with("BadCount"), "got {}", err.0);
    }

    /// `history_generation` crosses the FFI with the kernel's semantics: it
    /// bumps on a committed mutation, on undo, and on redo — and stays put
    /// across the non-undoable view-state toggles the eye icons drive.
    #[test]
    fn history_generation_crosses_the_ffi() {
        let mut scene = Scene::new();
        let g0 = scene.history_generation();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let g1 = scene.history_generation();
        assert!(g1 > g0, "a committed mutation bumps");

        scene.scene_undo().unwrap();
        let g2 = scene.history_generation();
        assert!(g2 > g1, "undo bumps");
        scene.scene_redo().unwrap();
        let g3 = scene.history_generation();
        assert!(g3 > g2, "redo bumps");

        scene.set_node_user_hidden(0, o, true).unwrap();
        scene.set_node_user_hidden(0, o, false).unwrap();
        scene.set_tag_hidden("walls".to_string(), true);
        assert_eq!(
            scene.history_generation(),
            g3,
            "view-state toggles leave the generation untouched"
        );
    }

    /// Copies are part of the replay contract: a session that Move+Option
    /// copies one node and then array-copies a selection replays into a fresh
    /// Scene to the exact golden state_hash (both calls are recorded — the
    /// single-copy path used to be a silent recording gap).
    #[test]
    fn copy_and_array_copy_record_and_replay() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let mv = [
            1.0, 0.0, 0.0, 3.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let copy = scene.duplicate_node(0, o, &mv).unwrap();
        assert_eq!(copy.kind, "object");
        let step = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 2.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene
            .duplicate_selection_array(&[0, 0], &[o, copy.id], &step, 2)
            .unwrap();
        assert_eq!(scene.object_ids().len(), 6);

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(json.contains("\"method\":\"duplicate_node\""));
        assert!(json.contains("\"method\":\"duplicate_selection_array\""));

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying a copy + array-copy session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// Draw-on-face with an analytic circle, then push the disk THROUGH the
    /// solid, records and replays to the exact same state (adversarial review,
    /// major): a C3 session must not diverge on replay. Red-checks by removing
    /// the imprint's recording — replay then diverges.
    #[test]
    fn draw_on_face_circle_then_through_cut_records_and_replays() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();

        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        // The +Z top face at z = 1.
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        // Imprint a circle carrying its identity, strictly inside the unit top.
        let (cx, cy, cz, r, n) = (0.5, 0.5, 1.0, 0.3, 24usize);
        let mut loop_pts = Vec::with_capacity(n * 3);
        for i in 0..n {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            loop_pts.push(cx + r * a.cos());
            loop_pts.push(cy + r * a.sin());
            loop_pts.push(cz);
        }
        let disk = scene
            .split_face_inner_with_curve(obj, top, &loop_pts, &[cx, cy, cz], r)
            .unwrap();
        // Push the disk straight down through the whole box.
        scene.push_pull(obj, disk, -2.0).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        // The imprint call was captured WITH its circle, and the push recorded.
        let rec: serde_json::Value = serde_json::from_str(&json).unwrap();
        let calls = rec["calls"].as_array().unwrap();
        assert!(
            calls
                .iter()
                .any(|c| c["method"] == "split_face_inner" && c["curve"].is_array()),
            "the draw-on-face imprint carrying its circle was recorded"
        );
        assert!(
            calls.iter().any(|c| c["method"] == "push_pull"),
            "the through-cut push was recorded"
        );

        // Replay into a fresh scene reproduces the exact state.
        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying the C3 session reproduces the golden state_hash"
        );
        assert_eq!(
            replayed.save(),
            scene.save(),
            "replay reproduces byte-identical document bytes (the tunnel's cylinder refs included)"
        );
    }

    /// The eraser's commit (`sketch_remove_edge`) is captured and replayed:
    /// a session that deletes a line diverges without it (the merged
    /// regions and the surviving edge set differ), so the golden state
    /// hash is the proof it round-trips.
    #[test]
    fn record_then_replay_captures_the_eraser() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        // Two wall-sharing squares, then erase the shared wall (one
        // gesture, like the app), then extrude the surviving merged
        // region's neighbor… keep it simple: erase, then extrude the
        // remaining closed region after redrawing the wall.
        let (s, _r) = ground_unit_square(&mut scene);
        for (a, b) in [
            ([1.0, 0.0], [2.0, 0.0]),
            ([2.0, 0.0], [2.0, 1.0]),
            ([2.0, 1.0], [1.0, 1.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        let wall = scene
            .pick_sketch_edge(1.0, 0.5, 5.0, 0.0, 0.0, -1.0)
            .expect("shared wall is pickable");
        scene.sketch_begin_gesture(s).unwrap();
        scene
            .sketch_remove_edge(wall.sketch(), wall.edge())
            .unwrap();
        scene.sketch_end_gesture(s).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(
            json.contains("\"method\":\"sketch_remove_edge\""),
            "the eraser commit is in the call stream"
        );

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying an eraser session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// Undo/redo are committed mutations like any other: a session that
    /// leans on them — where Model D's subtle behavior lives (extrusion
    /// undo RE-INSERTS scaffolding, merging with later edits) — must
    /// capture and replay them, or the recorder cannot reproduce exactly
    /// the bugs it exists for. A FAILED redo attempt commits nothing and
    /// is not recorded.
    #[test]
    fn record_then_replay_captures_undo_redo() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        // Draw → extrude → undo (outline re-inserted) → redo (re-deleted
        // by geometry) → undo again.
        let (s, r) = ground_unit_square(&mut scene);
        scene.extrude_region(s, r, 1.0).unwrap();
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();
        scene.scene_undo().unwrap();

        // Draw more into the restored sketch, bracketed as one gesture the
        // way tools commit (recording a SketchGesture step clears redo)…
        scene.sketch_begin_gesture(s).unwrap();
        for (a, b) in [
            ([2.0, 0.0], [3.0, 0.0]),
            ([3.0, 0.0], [3.0, 1.0]),
            ([3.0, 1.0], [2.0, 1.0]),
            ([2.0, 1.0], [2.0, 0.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        scene.sketch_end_gesture(s).unwrap();
        // …so this redo attempt fails: nothing committed, nothing recorded.
        assert!(scene.scene_redo().is_err());

        // Eraser: open the first square, then extrude the second.
        let edge = scene
            .pick_sketch_edge(0.5, 0.0, 5.0, 0.0, 0.0, -1.0)
            .expect("first square's bottom edge");
        scene
            .sketch_remove_edge(edge.sketch(), edge.edge())
            .unwrap();
        let regions = scene.sketch_regions(s).unwrap();
        assert_eq!(regions.len(), 1, "only the second square still closes");
        scene.extrude_region(s, regions[0], 1.0).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert_eq!(
            json.matches("\"method\":\"scene_undo\"").count(),
            2,
            "both undos are in the call stream"
        );
        assert_eq!(
            json.matches("\"method\":\"scene_redo\"").count(),
            1,
            "the successful redo is recorded; the failed attempt is not"
        );

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying an undo/redo session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// A curve bracket carrying its analytic circle records and replays:
    /// the replayed document is byte-identical, so the curve geometry
    /// (persisted in manifest v10) survived the round trip.
    #[test]
    fn analytic_curve_bracket_records_and_replays() {
        let mut scene = Scene::new();
        scene.start_recording();

        let sketch = scene.begin_ground_sketch();
        scene
            .sketch_begin_curve_with(sketch, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        // Two facets of the unit circle.
        scene
            .sketch_add_segment(sketch, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0)
            .unwrap();
        scene
            .sketch_add_segment(sketch, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0)
            .unwrap();
        scene.sketch_end_curve(sketch).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(
            json.contains("sketch_begin_curve_with"),
            "the analytic bracket is captured as its own call"
        );

        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
        assert_eq!(replayed.save(), scene.save());
    }

    /// The empirically-proven divergence class the recording audit closes:
    /// `delete_tag` (and the other tag/metadata mutators) pushes onto the
    /// SAME document undo stack as recorded ops but was invisible to session
    /// recording, while `scene_undo` IS recorded — so a recorded session
    /// containing a tag mutation plus an undo replayed a DIFFERENT action
    /// off a differently-shaped stack: the real session's undo reverted the
    /// tag delete; the replayed undo reverted the extrude, leaving zero
    /// objects and a state-hash mismatch.
    #[test]
    fn record_then_replay_covers_tag_ops_and_their_undo() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(s, r, 2.0).unwrap();
        scene.add_node_tag(0, obj, vec!["A".to_string()]).unwrap();
        scene.set_tag_hidden("A".to_string(), true);
        scene.delete_tag("A".to_string()).unwrap();
        // Undoes the TAG DELETE — not the extrude. If delete_tag were not
        // recorded, the replayed undo would hit the extrude instead.
        scene.scene_undo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            replayed.object_ids().len(),
            scene.object_ids().len(),
            "replay kept the extruded object — the recorded undo hit the tag delete"
        );
        assert_eq!(
            final_hash, golden,
            "replaying a tag-op session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// `set_camera_state` is not undoable but IS persisted (manifest v13) —
    /// mirrors `record_then_replay_covers_tag_ops_and_their_undo` for the
    /// camera view instead of tags: a trailing `scene_undo` must hit the
    /// Scenes + the section plane (docs/design/scenes.md) are persisted
    /// view state like the camera: every recorded Scene edit and apply must
    /// replay to the same bytes and state hash, and the JSON shapes crossing
    /// the boundary must round-trip.
    #[test]
    fn record_then_replay_covers_scenes_and_section_plane() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(s, r, 2.0).unwrap();
        scene
            .add_node_tag(0, obj, vec!["Hardware".to_string()])
            .unwrap();
        scene.set_tag_hidden("Hardware".to_string(), true);
        scene
            .set_section_plane(0.5, 0.5, 1.0, 0.0, 0.0, 1.0, true)
            .unwrap();
        let cam = r#"{"projection":"perspective","fovDeg":45,"eye":[4,-6,3],"target":[0,0,0],"up":[0,0,1]}"#;
        let disp = r#"{"grid":true,"axes":false,"guides":true}"#;
        let a = scene
            .add_scene(
                None,
                31,
                Some(cam.to_string()),
                Some(disp.to_string()),
                None,
            )
            .unwrap();
        scene.rename_scene(a, "Assembled".to_string()).unwrap();
        scene
            .set_scene_description(a, "Everything.".to_string())
            .unwrap();
        // Drift, then a second Scene without the section, then apply the first.
        scene.set_tag_hidden("Hardware".to_string(), false);
        scene.clear_section_plane();
        let b = scene
            .add_scene(
                Some("Cam only".to_string()),
                1,
                Some(cam.to_string()),
                None,
                Some(a),
            )
            .unwrap();
        scene.move_scene(b, 0).unwrap();
        scene.set_scene_props(a, 31 - 16, None, None).unwrap();
        scene.update_scene(a, 4, None, None).unwrap(); // re-capture visible tags (now none hidden)
        scene.set_tag_hidden("Hardware".to_string(), true);
        scene.update_scene(a, 4, None, None).unwrap(); // hidden again
        scene.set_tag_hidden("Hardware".to_string(), false);
        let resolved = scene.apply_scene(a).unwrap();
        assert!(resolved.has_hidden());
        assert_eq!(resolved.hidden_object_ids(), vec![obj]);
        assert_eq!(resolved.hidden_tag_paths(), vec!["Hardware".to_string()]);
        assert!(resolved.has_section());
        assert!(resolved.section_json().unwrap().contains("\"active\":true"));
        assert!(resolved.display_json().is_none(), "display was uncaptured");
        assert!(scene.doc.tag_hidden(&["Hardware".to_string()]));

        let c = scene.add_scene(None, 31, None, None, None).unwrap();
        scene.remove_scene(c).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying a Scenes session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");

        let list: serde_json::Value = serde_json::from_str(&replayed.scenes_json()).unwrap();
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "Cam only");
        assert_eq!(arr[0]["props"], 1);
        assert_eq!(arr[1]["name"], "Assembled");
        assert_eq!(arr[1]["description"], "Everything.");
        assert_eq!(arr[1]["props"], 15);
        assert_eq!(arr[1]["section"]["active"], true);
        assert!(arr[1].get("display").is_none());
        assert_eq!(arr[1]["camera"]["projection"], "perspective");
        assert!(
            replayed
                .section_plane_json()
                .unwrap()
                .contains("\"active\":true")
        );

        let drift: serde_json::Value = serde_json::from_str(
            &replayed
                .scene_drift(a, Some(cam.to_string()), None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(drift["camera"], false);
        assert_eq!(drift["hiddenTags"], false);
        assert_eq!(drift["staleRefs"], 0);
        assert!(replayed.next_scene_name() == "Scene 1");
    }

    /// Malformed boundary JSON is a typed refusal, never a silent default.
    #[test]
    fn scene_json_inputs_are_validated() {
        let mut scene = Scene::new();
        assert!(
            scene
                .add_scene(None, 1, Some("{not json".to_string()), None, None)
                .is_err()
        );
        assert!(scene
            .add_scene(
                None,
                1,
                Some(r#"{"projection":"iso","fovDeg":45,"eye":[0,0,0],"target":[0,0,0],"up":[0,0,1]}"#.to_string()),
                None,
                None
            )
            .is_err());
        assert!(
            scene
                .add_scene(None, 16, None, Some(r#"{"grid":1}"#.to_string()), None)
                .is_err()
        );
        assert!(
            scene
                .set_section_plane(0.0, 0.0, 0.0, 0.0, 0.0, 2.0, true)
                .is_err()
        );
        assert!(scene.rename_scene(42, "x".to_string()).is_err());
        let sid = scene
            .add_scene(Some("A".to_string()), 0, None, None, None)
            .unwrap();
        assert!(
            scene
                .add_scene(Some("A".to_string()), 0, None, None, None)
                .is_err()
        );
        assert!(scene.rename_scene(sid, "  ".to_string()).is_err());
    }

    /// Renaming a tag is undoable, keeps its stable id, and replays.
    #[test]
    fn record_then_replay_covers_rename_tag() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (s1, r1) = ground_unit_square_at(&mut scene, 0.0, 0.0);
        let a = scene.extrude_region(s1, r1, 1.0).unwrap();
        scene
            .add_node_tag(0, a, vec!["Hardware".to_string(), "Screws".to_string()])
            .unwrap();
        scene.set_tag_hidden("Hardware/Screws".to_string(), true);
        let sid_before = scene
            .doc
            .sid_of(&kernel::EntityRef::Tag(vec![
                "Hardware".to_string(),
                "Screws".to_string(),
            ]))
            .unwrap();
        let depth = scene.doc.undo_depth();
        scene
            .rename_tag("Hardware".to_string(), "Fixings".to_string())
            .unwrap();
        assert_eq!(scene.doc.undo_depth(), depth + 1);
        assert_eq!(
            scene.node_tags(0, a).unwrap(),
            vec!["Fixings/Screws".to_string()]
        );
        assert_eq!(
            scene.doc.sid_of(&kernel::EntityRef::Tag(vec![
                "Fixings".to_string(),
                "Screws".to_string()
            ])),
            Some(sid_before)
        );
        assert!(
            scene
                .rename_tag("Fixings".to_string(), "".to_string())
                .is_err()
        );
        scene.scene_undo().unwrap();
        assert_eq!(
            scene.node_tags(0, a).unwrap(),
            vec!["Hardware/Screws".to_string()]
        );
        scene.scene_redo().unwrap();
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
        assert_eq!(replayed.save(), scene.save());
    }

    /// Multi-selection tagging is one compound undo step, refuses a stale
    /// handle wholesale, and replays.
    #[test]
    fn record_then_replay_covers_bulk_tags() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();
        let (s1, r1) = ground_unit_square_at(&mut scene, 0.0, 0.0);
        let a = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 3.0, 0.0);
        let b = scene.extrude_region(s2, r2, 1.0).unwrap();
        let depth = scene.doc.undo_depth();
        scene
            .add_node_tag_many(&[0, 0], &[a, b], vec!["Hardware".to_string()])
            .unwrap();
        assert_eq!(scene.doc.undo_depth(), depth + 1);
        assert_eq!(scene.node_tags(0, a).unwrap(), vec!["Hardware".to_string()]);
        assert_eq!(scene.node_tags(0, b).unwrap(), vec!["Hardware".to_string()]);
        assert!(
            scene
                .add_node_tag_many(&[0], &[a, b], vec!["X".to_string()])
                .is_err()
        );
        assert!(
            scene
                .add_node_tag_many(&[], &[], vec!["X".to_string()])
                .is_err()
        );
        scene
            .remove_node_tag_many(&[0, 0], &[a, b], vec!["Hardware".to_string()])
            .unwrap();
        scene.scene_undo().unwrap(); // both tagged again
        assert_eq!(scene.node_tags(0, b).unwrap(), vec!["Hardware".to_string()]);
        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        let mut replayed = Scene::new();
        assert_eq!(replayed.replay(&json).unwrap(), golden);
        assert_eq!(replayed.save(), scene.save());
    }

    /// extrude, not unwind the camera (there is nothing to unwind), and the
    /// replayed document must carry the same camera state.
    #[test]
    fn record_then_replay_covers_camera_state() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        scene.extrude_region(s, r, 2.0).unwrap();
        scene
            .set_camera_state(
                "parallel",
                62.5,
                Box::new([1.0, 2.0, 3.0]),
                Box::new([-1.0, 0.5, 0.0]),
                Box::new([0.0, 0.0, 1.0]),
            )
            .unwrap();
        // Undoes the EXTRUDE — set_camera_state left no undo entry.
        scene.scene_undo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            replayed.object_ids().len(),
            0,
            "the recorded undo hit the extrude — no objects survive"
        );
        assert_eq!(
            final_hash, golden,
            "replaying a camera-state session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");

        let cam = replayed
            .camera_state()
            .expect("the replayed document carries the recorded camera state");
        assert_eq!(cam.projection(), "parallel");
        assert!((cam.fov_deg() - 62.5).abs() < 1e-12);
        assert!((cam.eye_x() - 1.0).abs() < 1e-12);
        assert!((cam.eye_y() - 2.0).abs() < 1e-12);
        assert!((cam.eye_z() - 3.0).abs() < 1e-12);
        assert!((cam.target_x() - (-1.0)).abs() < 1e-12);
        assert!((cam.target_y() - 0.5).abs() < 1e-12);
        assert!((cam.target_z() - 0.0).abs() < 1e-12);
        assert!((cam.up_z() - 1.0).abs() < 1e-12);
    }

    /// `Scene::camera_state` is `undefined` until a camera has been set, and
    /// `set_camera_state` rejects an unknown projection token typed rather
    /// than silently defaulting.
    #[test]
    fn camera_state_getter_and_bad_projection() {
        let mut scene = Scene::new();
        assert!(scene.camera_state().is_none());

        assert!(
            scene
                .set_camera_state(
                    "isometric",
                    45.0,
                    Box::new([0.0, 0.0, 5.0]),
                    Box::new([0.0, 0.0, 0.0]),
                    Box::new([0.0, 0.0, 1.0]),
                )
                .is_err()
        );
        assert!(scene.camera_state().is_none(), "the bad call set nothing");

        scene
            .set_camera_state(
                "perspective",
                45.0,
                Box::new([0.0, 0.0, 5.0]),
                Box::new([0.0, 0.0, 0.0]),
                Box::new([0.0, 0.0, 1.0]),
            )
            .unwrap();
        assert_eq!(scene.camera_state().unwrap().projection(), "perspective");
    }

    /// A broad structural/metadata session — naming, duplication, grouping,
    /// component + instance lifecycle, materials, guides, persisted view
    /// state — records and replays to the exact same state. Every op here
    /// previously pushed the shared document undo stack (or mutated saved
    /// state) while being invisible to recording, so the trailing undo
    /// replayed off a differently-shaped stack.
    #[test]
    fn record_then_replay_covers_structural_and_metadata_ops() {
        recording::reset();

        const SHIFT_Y3: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 1.0, 0.0];
        const SHIFT_X6: [f64; 12] = [1.0, 0.0, 0.0, 6.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        const SHIFT_HALF_X: [f64; 12] =
            [1.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        const SHIFT_Y1: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];

        let mut scene = Scene::new();
        scene.start_recording();

        let (s1, r1) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 3.0, 0.0);
        let b = scene.extrude_region(s2, r2, 1.0).unwrap();

        // Naming, duplication, persisted user-hide.
        scene.set_node_name(0, a, Some("Base".to_string())).unwrap();
        let dup = scene.duplicate_node(0, b, &SHIFT_Y3).unwrap();
        scene.set_node_user_hidden(0, dup.id(), true).unwrap();

        // Group the originals, move the group, dissolve it.
        let g = scene.group_nodes(&[0, 0], &[a, b]).unwrap();
        scene.transform_group(g, &SHIFT_HALF_X).unwrap();
        scene.ungroup(g).unwrap();

        // Component + instance lifecycle.
        let inst = scene.make_component(&[0], &[b]).unwrap();
        let def = scene.instance_def(inst).unwrap();
        let placed = scene.place_instance(def, &SHIFT_X6).unwrap();
        scene.transform_instance(placed, &SHIFT_Y1).unwrap();
        scene.make_unique(placed).unwrap();
        scene.explode_instance(placed).unwrap();

        // Materials. `paint_face`/`replace_material` were not previously
        // exercised by this record/replay coverage sweep — closing that gap
        // alongside `replace_material`'s own new coverage. `a` sits at
        // x=[0.5,1.5] y=[0,1] z=[0,1] by now (the earlier `SHIFT_HALF_X`
        // group transform moved it); (1.0, 0.5, 10.0) straight down lands
        // safely inside its top face, well clear of any edge — the
        // `pick.object()` assert makes that assumption self-checking.
        let m = scene.add_material("Red".to_string(), 255, 0, 0, 255);
        let m2 = scene.add_material("Blue".to_string(), 0, 0, 255, 255);
        scene.set_object_material(a, m).unwrap();
        scene.set_material_alpha(m, 128).unwrap();
        let pick = scene.pick_face(1.0, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a, "picked object `a`'s top face as expected");
        scene.paint_face(pick.object(), pick.face(), m2).unwrap();
        scene.replace_material(true, u64::MAX, m2, m).unwrap();

        // Position Texture (paint-tool design §3): set an explicit frame,
        // then reset it back to the planar default — exercises both
        // directions (`None -> Some` and `Some -> None`) through record/replay.
        scene
            .set_face_uv_frame(
                pick.object(),
                pick.face(),
                Some(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, -0.5]),
            )
            .unwrap();
        scene
            .set_face_uv_frame(pick.object(), pick.face(), None)
            .unwrap();

        // Guides.
        scene.add_guide_line(0.0, 0.0, 0.0, 1.0, 0.0, 0.0).unwrap();
        let gp = scene.add_guide_point(1.0, 2.0, 3.0).unwrap();
        scene.delete_guide(gp).unwrap();
        scene.add_guide_point(4.0, 5.0, 6.0).unwrap();
        scene.delete_all_guides().unwrap();

        // An undo at the end exercises the (now identically-shaped) stack.
        scene.scene_undo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        assert_eq!(
            replayed.replay(&json).unwrap(),
            golden,
            "replaying a structural/metadata session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// The byte-embedding arms replay: a session containing a glTF import
    /// (file bytes embedded in the recording) and a texture-material
    /// addition (encoded image bytes embedded) reproduces object counts,
    /// state hash, and saved bytes on replay. Drives `import_gltf_core`,
    /// the exact body the public method and the replay arm share (the
    /// JsValue report wrapper cannot run natively); the dae/skp arms use
    /// the identical embed-and-reissue mechanism.
    #[test]
    fn record_then_replay_covers_byte_embedding_calls() {
        recording::reset();

        let glb: &[u8] = include_bytes!("../../gltf-import/tests/fixtures/box.glb");
        // The palette stores encoded image bytes verbatim (no decode on
        // add), so a PNG-magic-prefixed stub is a faithful payload.
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05,
        ];

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        scene.import_gltf_core(glb).unwrap();
        let m = scene
            .add_texture_material("Wood".to_string(), 200, 180, 150, 255, png, 0, 1.0, 1.0)
            .unwrap();
        scene.set_object_material(a, m).unwrap();
        // An undo at the end: the import pushed DocAction::Imported onto the
        // shared stack, so replay diverges if the embed-and-reissue is wrong.
        scene.scene_undo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(
            json.contains("\"method\":\"import_gltf\""),
            "the import is in the call stream"
        );
        assert!(
            json.contains("\"method\":\"add_texture_material\""),
            "the texture addition is in the call stream"
        );

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            replayed.object_ids().len(),
            scene.object_ids().len(),
            "object counts match after replaying an import session"
        );
        assert_eq!(
            final_hash, golden,
            "replaying a byte-embedding session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// `RecordedCall::ImportStl` — the merge hazard the STL importer's design
    /// specifically calls out: a dropped match arm anywhere in the
    /// record/serialize/replay-dispatch chain builds locally but breaks
    /// replay determinism. This proves the whole chain, exactly like
    /// [`record_then_replay_covers_byte_embedding_calls`] does for glTF:
    /// `unit_scale` AND the file-stem `name` are embedded too (STL carries
    /// neither units nor object names of its own, so a replay that lost either
    /// would reproduce the wrong-sized or wrong-named geometry).
    ///
    /// Crucially the imported Object is left VISIBLE through the golden-state
    /// capture (no trailing `scene_undo`): undo sets `hidden = true`, and
    /// `save`/`state_hash` filter hidden objects out — so an undone import
    /// would compare byte-identical no matter WHICH `name`/`unit_scale` the
    /// replay arm forwarded, masking a dropped field at exactly this hazard.
    /// The Object's name and its exported-triangle bounding box are compared
    /// directly on both scenes while it is visible: a lost `name` renames it,
    /// a lost `unit_scale` rescales it 1000×.
    #[test]
    fn record_then_replay_covers_stl_import() {
        recording::reset();

        let stl: &[u8] = include_bytes!("../../stl-import/tests/fixtures/cube_binary.stl");

        // Name + exported-triangle bbox of the imported Object (the one NOT
        // named like the extruded box below), for cross-scene comparison.
        fn imported_probe(scene: &Scene) -> (String, [f64; 6]) {
            let id = scene
                .object_ids()
                .into_iter()
                .find(|&id| scene.object_name(id).as_deref() == Some("bunny"))
                .expect("the imported Object named 'bunny' exists and is visible");
            let name = scene.object_name(id).unwrap();
            let tris = scene.object_export_triangles(id, 0).unwrap();
            let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
            for v in tris.chunks_exact(3) {
                for a in 0..3 {
                    b[a] = b[a].min(v[a]);
                    b[a + 3] = b[a + 3].max(v[a]);
                }
            }
            (name, b.map(|x| x as f64))
        }

        let mut scene = Scene::new();
        scene.start_recording();

        let (s, r) = ground_unit_square(&mut scene);
        scene.extrude_region(s, r, 1.0).unwrap();
        scene
            .import_stl_core(stl, 0.001, Some("bunny".to_string()))
            .unwrap();

        // Golden state is captured with the import VISIBLE (see the doc note) —
        // no undo, so the hash/save are actually sensitive to name + unit_scale.
        scene.stop_recording();
        let golden = scene.state_hash();
        let (golden_name, golden_bbox) = imported_probe(&scene);
        let json = scene.take_recording();
        assert!(
            json.contains("\"method\":\"import_stl\""),
            "the import is in the call stream"
        );
        assert!(
            json.contains("\"unit_scale\":0.001"),
            "the unit scale is embedded, not just the bytes"
        );
        assert!(
            json.contains("\"name\":\"bunny\""),
            "the file-stem name is embedded so the Objects replay identically"
        );

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            replayed.object_ids().len(),
            scene.object_ids().len(),
            "object counts match after replaying an STL import session"
        );

        // The imported Object's NAME and GEOMETRY match after replay — this is
        // what catches a dropped `name` or `unit_scale` at the RecordedCall arm.
        let (replay_name, replay_bbox) = imported_probe(&replayed);
        assert_eq!(
            replay_name, golden_name,
            "the imported Object's name replays"
        );
        for a in 0..6 {
            assert!(
                (replay_bbox[a] - golden_bbox[a]).abs() < 1e-9,
                "the imported Object's bbox (unit-scale-dependent) replays: axis {a}, \
                 golden {} vs replay {}",
                golden_bbox[a],
                replay_bbox[a]
            );
        }

        assert_eq!(
            final_hash, golden,
            "replaying an STL-import session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// A mid-session File ▸ Open (`load`) replaces the whole document. The
    /// recording embeds the `.hew` bytes so the session — including work
    /// done AFTER the open — still replays from a fresh `Scene`.
    #[test]
    fn record_then_replay_covers_a_mid_session_load() {
        recording::reset();

        // A saved document to open mid-session.
        let saved = {
            let mut base = Scene::new();
            let (s, r) = ground_unit_square(&mut base);
            base.extrude_region(s, r, 2.0).unwrap();
            base.save()
        };

        let mut scene = Scene::new();
        scene.start_recording();
        let (s, r) = ground_unit_square(&mut scene);
        scene.extrude_region(s, r, 1.0).unwrap();
        assert!(scene.load(&saved).is_ok(), "mid-session open");
        // Keep working in the loaded document.
        let (s2, r2) = ground_unit_square_at(&mut scene, 3.0, 0.0);
        scene.extrude_region(s2, r2, 1.0).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(
            json.contains("\"method\":\"load\""),
            "the open is in the call stream"
        );

        let mut replayed = Scene::new();
        assert_eq!(
            replayed.replay(&json).unwrap(),
            golden,
            "replaying a session that spans a File ▸ Open reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// A degenerate analytic bracket is refused with a typed error and
    /// leaves no bracket open.
    #[test]
    fn analytic_curve_bracket_rejects_degenerate_radius() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        let err = scene
            .sketch_begin_curve_with(sketch, 0.0, 0.0, 0.0, 0.0)
            .unwrap_err();
        assert!(err.0.starts_with("DegenerateCurve:"), "got: {}", err.0);
    }

    /// `face_uv_frame`/`set_face_uv_frame` round-trip an explicit frame, and
    /// `None` resets it back to the planar-default empty-vec reading
    /// (paint-tool design §3).
    #[test]
    fn face_uv_frame_round_trips_through_set() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let pick = scene.pick_face(0.5, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a);
        let (object, face) = (pick.object(), pick.face());

        assert_eq!(
            scene.face_uv_frame(object, face),
            Some(Vec::new()),
            "unset reads back as the planar-default empty vec"
        );

        let frame = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, -0.5];
        scene
            .set_face_uv_frame(object, face, Some(frame.clone()))
            .unwrap();
        assert_eq!(scene.face_uv_frame(object, face), Some(frame));

        scene.set_face_uv_frame(object, face, None).unwrap();
        assert_eq!(scene.face_uv_frame(object, face), Some(Vec::new()));
    }

    /// A malformed frame (not exactly 8 floats) is refused typed, not
    /// silently truncated/padded.
    #[test]
    fn set_face_uv_frame_rejects_malformed_length() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let pick = scene.pick_face(0.5, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a);

        let err = scene
            .set_face_uv_frame(pick.object(), pick.face(), Some(vec![1.0, 2.0]))
            .unwrap_err();
        assert!(err.0.starts_with("BadUvFrame:"), "got: {}", err.0);
    }

    /// An 8-float frame that IS well-formed length-wise but geometrically
    /// degenerate (non-finite, zero-gradient, or singular/parallel `s`/`t`)
    /// is refused typed through the kernel's own validation
    /// (`DocumentError::DegenerateUvFrame`), not silently stored — same
    /// no-silent-repair posture as the malformed-length case above, one
    /// layer deeper.
    #[test]
    fn set_face_uv_frame_rejects_degenerate_frame() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let pick = scene.pick_face(0.5, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a);
        let (object, face) = (pick.object(), pick.face());

        let non_finite = vec![f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let err = scene
            .set_face_uv_frame(object, face, Some(non_finite))
            .unwrap_err();
        assert!(err.0.starts_with("DegenerateUvFrame:"), "got: {}", err.0);

        let parallel = vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0];
        let err = scene
            .set_face_uv_frame(object, face, Some(parallel))
            .unwrap_err();
        assert!(err.0.starts_with("DegenerateUvFrame:"), "got: {}", err.0);

        assert_eq!(
            scene.face_uv_frame(object, face),
            Some(Vec::new()),
            "both refusals left the face at its unset planar default"
        );
    }

    /// Stale object/face handles are refused typed, matching `paint_face`'s
    /// posture.
    #[test]
    fn set_face_uv_frame_rejects_unknown_inputs() {
        let mut scene = Scene::new();
        let err = scene
            .set_face_uv_frame(u64::MAX, u64::MAX, None)
            .unwrap_err();
        assert!(err.0.starts_with("UnknownObject:"), "got: {}", err.0);
    }

    /// `set_face_uv_frame` is undoable through the normal `scene_undo`/
    /// `scene_redo` path, same as `paint_face`.
    #[test]
    fn set_face_uv_frame_is_undoable_via_scene() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let pick = scene.pick_face(0.5, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a);
        let (object, face) = (pick.object(), pick.face());

        let frame = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        scene
            .set_face_uv_frame(object, face, Some(frame.clone()))
            .unwrap();
        assert_eq!(scene.face_uv_frame(object, face), Some(frame.clone()));

        scene.scene_undo().unwrap();
        assert_eq!(
            scene.face_uv_frame(object, face),
            Some(Vec::new()),
            "undo restores the planar default"
        );

        scene.scene_redo().unwrap();
        assert_eq!(scene.face_uv_frame(object, face), Some(frame));
    }

    /// `face_mesh_range` gives a `[base, count]` vertex range that actually
    /// indexes into `object_mesh`'s `uvs`/`positions` — the Position Texture
    /// tool's live-preview patch target (paint-tool design §3).
    #[test]
    fn face_mesh_range_indexes_into_the_object_mesh() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let pick = scene.pick_face(0.5, 0.5, 10.0, 0.0, 0.0, -1.0).unwrap();
        assert_eq!(pick.object(), a);
        let (object, face) = (pick.object(), pick.face());

        let range = scene.face_mesh_range(object, face).expect("live face");
        let (base, count) = (range[0] as usize, range[1] as usize);
        assert!(count >= 3, "a face has at least 3 vertices");

        let mesh = scene.object_mesh(object).unwrap();
        let uvs = mesh.uvs();
        let positions = mesh.positions();
        assert!(
            (base + count) * 2 <= uvs.len(),
            "range must fit inside the mesh's uv buffer"
        );
        assert!(
            (base + count) * 3 <= positions.len(),
            "range must fit inside the mesh's position buffer"
        );

        // Stale object/face reads back undefined, not a stale range.
        assert_eq!(scene.face_mesh_range(u64::MAX, face), None);
    }

    /// A session co-authored by the user and an agent — UI gestures
    /// interleaved with live-API envelopes — replays WHOLE. Before
    /// `RecordedCall::ApiDispatch`, the API half was invisible to the
    /// recorder, so a reproducer built from such a session silently
    /// reconstructed a different document: the agent's geometry simply
    /// missing, the golden hash unmatchable. That is the exact failure
    /// this pins.
    #[test]
    fn a_session_mixing_ui_gestures_and_live_api_envelopes_replays_whole() {
        let mut scene = Scene::new();
        recording::start();

        // UI half: draw and extrude a square the ordinary way.
        let (sketch, region) = ground_unit_square(&mut scene);
        scene.extrude_region(sketch, region, 0.5).expect("extrude");

        // Agent half: a second solid, built entirely over the live API
        // inside one transaction ($ref chaining, exactly as an MCP client
        // drives it).
        let conn = scene.api_connection_open();
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
        );
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
        );
        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":2,"method":"hew.doc.transact","params":{"label":"Agent box","commands":[
                    {"method":"hew.sketch.draw_rect","as":"s1","params":{"plane":{"origin":[5,0,0],"normal":[0,0,1]},"corner_a":[5,0,0],"corner_b":[6,1,0]}},
                    {"method":"hew.solid.extrude","params":{"region":{"$ref":"s1#/region_id"},"distance":0.25}}]}}"#,
            )
            .expect("transact replies");
        assert!(
            !reply.contains("\"error\""),
            "the agent's transaction must succeed: {reply}"
        );
        scene.api_connection_close(conn);

        let golden = scene.state_hash();
        recording::stop();
        let calls = recording::take_calls();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, recording::RecordedCall::ApiDispatch { .. })),
            "the API envelope must have been captured"
        );

        let artifact = serde_json::to_string(&recording::Recording {
            version: recording::RECORDING_FORMAT_VERSION,
            calls,
            golden_hash: golden,
        })
        .expect("recording serializes");

        let mut fresh = Scene::new();
        let replayed = fresh.replay(&artifact).expect("replay succeeds");
        assert_eq!(
            replayed, golden,
            "replaying both halves must reproduce the session's document exactly"
        );
    }

    /// `hew.history.undo` over the live API is a document change and must
    /// be recorded like any other. It is classed `Solitary` — it cannot
    /// ride inside the very history it moves — so deriving "did this
    /// mutate?" from the command class skipped it: the recording lost the
    /// undo, and the viewport kept showing geometry the agent had just
    /// removed. Found by adversarial review.
    #[test]
    fn a_live_api_undo_is_recorded_and_replays() {
        let mut scene = Scene::new();
        recording::start();
        let (sketch, region) = ground_unit_square(&mut scene);
        scene.extrude_region(sketch, region, 0.5).expect("extrude");

        let conn = scene.api_connection_open();
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
        );
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
        );
        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":2,"method":"hew.history.undo","params":{}}"#,
            )
            .expect("undo replies");
        assert!(!reply.contains("\"error\""), "undo must succeed: {reply}");
        scene.api_connection_close(conn);

        let golden = scene.state_hash();
        recording::stop();
        let calls = recording::take_calls();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, recording::RecordedCall::ApiDispatch { frame } if frame.contains("history.undo"))),
            "the agent's undo must be in the recording: {calls:?}"
        );

        let artifact = serde_json::to_string(&recording::Recording {
            version: recording::RECORDING_FORMAT_VERSION,
            calls,
            golden_hash: golden,
        })
        .expect("recording serializes");
        let mut fresh = Scene::new();
        assert_eq!(
            fresh.replay(&artifact).expect("replay succeeds"),
            golden,
            "a session ending in an API undo must replay to the same document"
        );
    }

    /// A read-only envelope is NOT recorded — an agent polling
    /// `hew.query.scene` must not bloat every recording with traffic that
    /// changed nothing.
    #[test]
    fn read_only_api_traffic_is_not_recorded() {
        let mut scene = Scene::new();
        recording::start();
        let conn = scene.api_connection_open();
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
        );
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
        );
        scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":2,"method":"hew.query.scene","params":{}}"#,
        );
        recording::stop();
        let calls = recording::take_calls();
        assert!(
            calls.is_empty(),
            "read-only API traffic must not be recorded: {calls:?}"
        );
    }

    /// The bridge's "does this need a re-render?" question is answered by
    /// the registry, not by a method-name convention.
    #[test]
    fn api_method_mutates_reads_the_registry() {
        let scene = Scene::new();
        assert!(scene.api_method_mutates("hew.solid.extrude"));
        assert!(scene.api_method_mutates("hew.doc.transact"));
        assert!(!scene.api_method_mutates("hew.query.scene"));
        assert!(
            !scene.api_method_mutates("hew.doc.attach"),
            "attach is connection state, not a document change"
        );
        assert!(
            scene.api_method_mutates("hew.history.undo"),
            "undo is solitary because it cannot ride inside the history it moves — \
             but it absolutely changes the document, and a host that skipped it \
             would leave the viewport showing geometry that is no longer there"
        );
        assert!(scene.api_method_mutates("hew.history.redo"));
        assert!(
            !scene.api_method_mutates("hew.not.a.command"),
            "an unknown method cannot mutate — it can only come back as method-not-found"
        );
    }

    /// A version mismatch in a recording artifact is rejected, not mis-replayed.
    #[test]
    fn replay_rejects_a_wrong_format_version() {
        let mut scene = Scene::new();
        let err = scene
            .replay(r#"{"version":999,"calls":[],"golden_hash":0}"#)
            .unwrap_err();
        assert!(err.0.starts_with("REPLAY:"), "got: {}", err.0);
    }

    /// Draws a unit square on the ground sketch and returns
    /// (sketch_handle, region_handle).
    fn ground_unit_square(scene: &mut Scene) -> (u64, u64) {
        ground_unit_square_at(scene, 0.0, 0.0)
    }

    /// [`ground_unit_square`] at an (x, y) offset — for tests that need a
    /// second solid drawn clear of the first (its position is otherwise
    /// incidental; overlapping regions extrude directly now).
    fn ground_unit_square_at(scene: &mut Scene, x: f64, y: f64) -> (u64, u64) {
        let sketch = scene.begin_ground_sketch();
        let corners = [
            (0.0, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ];
        let mut region = None;
        for (ax, ay, bx, by) in corners {
            let report = scene
                .sketch_add_segment(sketch, x + ax, y + ay, 0.0, x + bx, y + by, 0.0)
                .unwrap();
            if let Some(&r) = report.inner.regions_created.first() {
                region = Some(r.data().as_ffi());
            }
        }
        (sketch, region.expect("closing the square creates a region"))
    }

    #[test]
    fn extrude_then_scene_undo_redo_hides_and_restores_the_object() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);

        // region_boundary returns the square's 4 corners.
        let boundary = scene.region_boundary(sketch, region).unwrap();
        assert_eq!(boundary.len(), 12);

        // Before extrusion the region is listed and extrudable.
        assert_eq!(scene.sketch_regions(sketch).unwrap(), vec![region]);

        let obj = scene.extrude_region(sketch, region, 2.0).unwrap();
        assert_eq!(scene.object_ids(), vec![obj]);
        assert!(scene.object_watertight(obj).unwrap());
        assert!(scene.can_scene_undo());
        // The region is consumed: its scaffolding was deleted and the
        // emptied sketch itself ceased to exist.
        assert!(scene.sketch_regions(sketch).is_err(), "sketch is gone");
        assert!(!scene.sketch_ids().contains(&sketch));

        // Undo the creation: the object is hidden (gone from the listing) but
        // its handle is preserved for redo.
        scene.scene_undo().unwrap();
        assert!(scene.object_ids().is_empty());
        assert!(scene.object_watertight(obj).is_err()); // hidden = not live
        assert!(scene.can_scene_redo());

        // Undo also restored the outline (fresh region handle — the
        // scaffolding is re-inserted, not snapshot-restored).
        assert_eq!(scene.sketch_regions(sketch).unwrap().len(), 1);
        assert!(
            !scene.sketch_lines(sketch).unwrap().is_empty(),
            "sketch lines must reappear after undoing the extrusion"
        );

        // Redo restores the SAME handle and re-consumes the sketch.
        scene.scene_redo().unwrap();
        assert_eq!(scene.object_ids(), vec![obj]);
        assert!(scene.object_watertight(obj).unwrap());
        assert!(
            scene.sketch_regions(sketch).is_err(),
            "the sketch is gone again after redo"
        );
    }

    #[test]
    fn push_pull_is_scene_undoable_after_extrude() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();

        // The top face has normal +Z; find it via face_normal over the mesh's
        // faces is indirect, so just confirm face_normal works on some face.
        // Pull the top face up by 1 (translate mode keeps 6 faces).
        // Find the top face: object_mesh doesn't expose face ids, so we drive
        // push_pull through a known face by scanning normals.
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        // face_normal returns +Z for the top face.
        let n = scene.face_normal(obj, top).unwrap();
        assert!((n[2] - 1.0).abs() < 1e-9);

        scene.push_pull(obj, top, 1.0).unwrap();
        // Two document actions now: create, then push/pull.
        scene.scene_undo().unwrap(); // undo push/pull
        assert!(scene.object_ids().contains(&obj)); // object still here
        scene.scene_undo().unwrap(); // undo create
        assert!(scene.object_ids().is_empty());
    }

    /// Push/Pull's Ctrl/Cmd modifier: straight-extrudes the clicked face's
    /// boundary into a NEW top-level object, leaving the source untouched —
    /// undoable/redoable and replayable exactly like `push_pull`.
    #[test]
    fn extrude_face_as_new_object_births_a_new_object_and_undoes() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };

        let boss = scene.extrude_face_as_new_object(obj, top, 1.0).unwrap();
        assert_ne!(boss, obj);
        assert!(scene.object_ids().contains(&obj), "source untouched");
        assert!(scene.object_ids().contains(&boss));
        assert!(scene.object_watertight(boss).unwrap());

        scene.scene_undo().unwrap(); // undo the Ctrl-extrude
        assert!(scene.object_ids().contains(&obj));
        assert!(!scene.object_ids().contains(&boss));
        scene.scene_redo().unwrap();
        assert!(scene.object_ids().contains(&boss));
    }

    /// Bad inputs refuse typed and never touch the document.
    #[test]
    fn extrude_face_as_new_object_rejects_unknown_object_or_face() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        assert!(scene.extrude_face_as_new_object(999_999, top, 1.0).is_err());
        assert!(scene.extrude_face_as_new_object(obj, 999_999, 1.0).is_err());
    }

    /// `rescale_document` scales the model about the origin, refuses bad
    /// factors typed, and undoes/redoes bit-exact.
    #[test]
    fn rescale_document_scales_the_model_and_undoes() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let before = scene.doc.object(object_id(obj)).unwrap().to_polygons();

        assert!(scene.rescale_document(0.0).is_err());
        assert!(scene.rescale_document(-1.0).is_err());
        assert!(scene.rescale_document(f64::NAN).is_err());

        scene.rescale_document(2.0).unwrap();
        let after = scene.doc.object(object_id(obj)).unwrap().to_polygons();
        assert_ne!(before, after, "geometry scaled");

        scene.scene_undo().unwrap();
        assert_eq!(
            scene.doc.object(object_id(obj)).unwrap().to_polygons(),
            before,
            "undo restores the exact pre-scale geometry"
        );
        scene.scene_redo().unwrap();
        assert_eq!(
            scene.doc.object(object_id(obj)).unwrap().to_polygons(),
            after,
            "redo reproduces the exact original post-scale geometry"
        );
    }

    #[test]
    fn set_axes_moves_the_frame_reorients_inference_and_undoes() {
        let mut scene = Scene::new();
        assert_eq!(
            scene.axes(),
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            "a fresh scene's axes are world identity"
        );

        // Non-orthonormal candidates are refused typed and change nothing.
        assert!(
            scene
                .set_axes(0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0)
                .is_err()
        );
        assert_eq!(
            scene.axes()[3],
            1.0,
            "the refused frame left axes at identity"
        );

        scene
            .set_axes(1.0, 2.0, 3.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0)
            .expect("valid orthonormal frame");
        assert_eq!(
            scene.axes(),
            vec![1.0, 2.0, 3.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            "origin/x/y set, z derived as x cross y"
        );

        // Inference's axis-lock now resolves through the new frame, not
        // world X/Y/Z (see `InferenceScene::set_axes_frame`).
        let snap = scene
            .snap(
                1.4,
                2.7,
                8.0,
                0.0,
                0.0,
                -1.0,
                0.3,
                Some(vec![1.0, 2.0, 3.0].into_boxed_slice()),
                Some(0), // lock_axis 0 = X
                None,
                None,
                None,
                None,
                None,
            )
            .expect("snap call succeeds")
            .expect("axis lock with an anchor always resolves");
        assert_eq!(
            snap.direction(),
            Some(vec![0.0, 1.0, 0.0]),
            "the X lock now follows the frame's red axis (world +Y)"
        );

        scene.scene_undo().unwrap();
        assert_eq!(
            scene.axes(),
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            "undo restores world identity"
        );
        scene.scene_redo().unwrap();
        assert_eq!(
            scene.axes()[3],
            0.0,
            "redo reproduces the moved frame's x.x"
        );
    }

    #[test]
    fn set_axes_records_and_replays() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();

        scene
            .set_axes(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0)
            .unwrap();
        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replay reproduces the axes-move state hash"
        );
        assert_eq!(replayed.axes(), scene.axes());
    }

    #[test]
    fn face_plane_returns_point_on_face_plus_normal() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        let pn = scene.face_plane(obj, top).unwrap();
        assert_eq!(pn.len(), 6);
        // Normal is +Z.
        assert!((pn[5] - 1.0).abs() < 1e-9 && pn[3].abs() < 1e-9 && pn[4].abs() < 1e-9);
        // The point lies on the top plane (z = 1).
        assert!((pn[2] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sketch_plane_tracks_transforms_and_is_undefined_for_stale_handles() {
        let mut scene = Scene::new();
        let (sketch, _region) = ground_unit_square(&mut scene);

        let pn = scene.sketch_plane(sketch).expect("live sketch");
        assert_eq!(pn.len(), 6);
        // Ground: contains the origin, normal +Z.
        assert!(pn[2].abs() < 1e-9);
        assert!((pn[5] - 1.0).abs() < 1e-9 && pn[3].abs() < 1e-9 && pn[4].abs() < 1e-9);

        // Stand the sketch upright (90 degrees about the X axis through the
        // origin): the reported plane follows the bake.
        #[rustfmt::skip]
        let rot_x_90: [f64; 12] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 0.0, -1.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
        ];
        scene.transform_sketch(sketch, &rot_x_90).unwrap();
        let pn = scene.sketch_plane(sketch).expect("still live");
        assert!(
            pn[4].abs() > 0.99 && pn[3].abs() < 1e-9 && pn[5].abs() < 1e-9,
            "upright plane's normal is +/-Y, got {pn:?}"
        );

        // A deleted (hidden) sketch reads as undefined, like any stale handle.
        scene.delete_sketch(sketch).unwrap();
        assert!(scene.sketch_plane(sketch).is_none());
        assert!(scene.sketch_plane(u64::MAX).is_none());
    }

    #[test]
    fn set_hidden_excludes_object_from_pick_and_snap() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 2.0).unwrap();

        // A ray straight down through the box hits the top face while visible,
        // and the box's 8 vertices / 12 edges / 6 faces are inference candidates.
        assert!(
            scene.pick_face(0.5, 0.5, 5.0, 0.0, 0.0, -1.0).is_some(),
            "visible box is pickable"
        );
        assert_eq!(
            scene.inference.candidate_counts(),
            (8, 12, 6),
            "visible box registers its full geometry with inference"
        );

        // Hiding the object drops every candidate AND makes it unpickable, so a
        // click/hover reaches whatever lies behind it instead of the hidden solid.
        scene.set_hidden(&[obj], &[]);
        assert_eq!(
            scene.inference.candidate_counts(),
            (0, 0, 0),
            "hidden box contributes no snap candidates"
        );
        assert!(
            scene.pick_face(0.5, 0.5, 5.0, 0.0, 0.0, -1.0).is_none(),
            "hidden box must not be pickable"
        );

        // Showing it again re-registers the full geometry.
        scene.set_hidden(&[], &[]);
        assert_eq!(scene.inference.candidate_counts(), (8, 12, 6));
        assert!(
            scene.pick_face(0.5, 0.5, 5.0, 0.0, 0.0, -1.0).is_some(),
            "shown box is pickable again"
        );
    }

    #[test]
    fn hidden_object_stays_hidden_across_a_mutation() {
        // reconcile must not resurrect a hidden object's inference candidates
        // when a later mutation touches it.
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 2.0).unwrap();
        scene.set_hidden(&[obj], &[]);

        // A push/pull on the box touches it (objects_touched), driving reconcile.
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        scene.push_pull(obj, top, 1.0).unwrap();

        assert_eq!(
            scene.inference.candidate_counts(),
            (0, 0, 0),
            "a mutation must not re-register a hidden object with inference"
        );
        assert!(
            scene.pick_face(0.5, 0.5, 5.0, 0.0, 0.0, -1.0).is_none(),
            "the hidden box stays unpickable after the push/pull"
        );
    }

    #[test]
    fn imprint_then_push_bosses_a_subface_and_undoes() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let top = {
            let object = scene.doc.object(object_id(obj)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        // Imprint a rectangle inside the top, then push it (auto-routes to
        // extrude_sub_face since it's a flat sub-face).
        let rect = [
            0.25, 0.25, 1.0, 0.75, 0.25, 1.0, 0.75, 0.75, 1.0, 0.25, 0.75, 1.0,
        ];
        let sub = scene.split_face_inner(obj, top, &rect).unwrap();
        scene.push_pull(obj, sub, 0.5).unwrap();
        assert_eq!(scene.object_ids(), vec![obj], "still one object");
        // Three document actions: create, imprint, boss — each undoes.
        scene.scene_undo().unwrap(); // undo boss
        scene.scene_undo().unwrap(); // undo imprint
        assert!(scene.object_ids().contains(&obj));
        scene.scene_undo().unwrap(); // undo create
        assert!(scene.object_ids().is_empty());
    }

    #[test]
    fn ground_sketches_are_additive_and_coexist() {
        let mut scene = Scene::new();
        let first = scene.begin_ground_sketch();
        assert_eq!(scene.sketch_lines(first).unwrap().len(), 0);
        let second = scene.begin_ground_sketch();
        // : beginning a new sketch is additive — the first handle stays live,
        // so independent coplanar sketches coexist.
        assert_ne!(first, second);
        assert!(scene.sketch_lines(first).is_ok());
        assert!(scene.sketch_regions(second).unwrap().is_empty());
        assert_eq!(scene.sketch_ids().len(), 2);
    }

    #[test]
    fn extruded_object_mesh_is_cached_and_served() {
        let mut scene = Scene::new();
        let (sketch, region) = ground_unit_square(&mut scene);
        let obj = scene.extrude_region(sketch, region, 1.0).unwrap();
        let mesh = scene.object_mesh(obj).unwrap();
        // A box: 6 quad faces -> 24 duplicated corners, 12 triangles.
        assert_eq!(mesh.positions().len(), 24 * 3);
        // Cache fills and serves the second pull identically.
        assert_eq!(scene.object_mesh(obj).unwrap().indices().len(), 12 * 3);
    }

    #[test]
    fn boolean_of_two_coplanar_boxes_unions_into_one() {
        let mut scene = Scene::new();
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        // The second box is drawn offset and moved into coincidence with o1.
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();
        scene
            .transform_object(
                o2,
                &[1.0, 0.0, 0.0, -2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            )
            .unwrap();

        // Two identical ground boxes share coplanar faces; the boolean now
        // resolves coplanar contact instead of refusing. Union of
        // coincident solids is one box — operands consumed, one object left.
        let result = scene.boolean(0, o1, o2).unwrap();
        assert_eq!(scene.object_ids(), vec![result]);
    }

    #[test]
    fn boolean_nodes_group_operand_returns_result_node() {
        let mut scene = Scene::new();
        // Three disjoint unit boxes; the first two grouped.
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 4.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();
        let (s3, r3) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let o3 = scene.extrude_region(s3, r3, 1.0).unwrap();
        let g = scene.group_nodes(&[0, 0], &[o1, o2]).unwrap();

        // Union of a group of two disjoint solids with a third disjoint
        // solid: three connected volumes → a result group of three objects.
        let root = scene.boolean_nodes(0, 1, g, 0, o3).unwrap();
        assert_eq!(root.kind(), "group");
        assert_eq!(scene.group_members(root.id()).len(), 3);
        assert_eq!(scene.object_ids().len(), 3, "one object per volume");
        assert!(
            !scene.object_ids().contains(&o1) && !scene.object_ids().contains(&o3),
            "operands consumed"
        );

        // One undo restores the operands (stable handles).
        scene.scene_undo().unwrap();
        let ids = scene.object_ids();
        assert!(ids.contains(&o1) && ids.contains(&o2) && ids.contains(&o3));
        assert_eq!(scene.group_ids(), vec![g], "the operand group is back");
    }

    #[test]
    fn boolean_rejects_bad_op_code() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let err = scene.boolean(9, o, o).unwrap_err();
        assert!(err.0.starts_with("BadOp"), "got {}", err.0);
    }

    #[test]
    fn transform_object_moves_and_is_undoable() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        // Row-major 3x4: identity linear, translate +X by 5.
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene.transform_object(o, &affine).unwrap();
        assert_eq!(scene.object_ids(), vec![o], "same handle after transform");
        scene.scene_undo().unwrap();
        assert_eq!(scene.object_ids(), vec![o], "still there after undo");
    }

    #[test]
    fn transform_object_rejects_bad_affine_and_reflection() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();

        let short = [1.0, 0.0, 0.0];
        assert!(
            scene
                .transform_object(o, &short)
                .unwrap_err()
                .0
                .starts_with("BadAffine")
        );
        // Negative scale on every axis flips orientation → refused.
        let reflect = [
            -1.0, 0.0, 0.0, 0.0, //
            0.0, -1.0, 0.0, 0.0, //
            0.0, 0.0, -1.0, 0.0,
        ];
        let err = scene.transform_object(o, &reflect).unwrap_err();
        assert!(err.0.starts_with("Reflection"), "got {}", err.0);
    }

    /// After extruding one of two wall-sharing squares, the surviving
    /// island holds only the neighbor's real edges — no invisible
    /// scaffolding backs a solid — so it validates and moves freely (Z5).
    #[test]
    fn island_move_works_after_neighbor_extrusion() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        // A second square sharing the first's right wall.
        for (a, b) in [
            ([1.0, 0.0], [2.0, 0.0]),
            ([2.0, 0.0], [2.0, 1.0]),
            ([2.0, 1.0], [1.0, 1.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        scene.extrude_region(s, r, 1.0).unwrap();

        // The extruded square's exclusive edges are gone; what remains is
        // ONE island: the neighbor square (closed by the shared wall).
        let islands = scene.sketch_island_ids(s);
        assert_eq!(islands.len(), 1);
        assert_eq!(scene.sketch_island_edges(s, islands[0]).len(), 4);
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        assert!(
            scene.can_transform_sketch_island(s, islands[0], &affine),
            "nothing invisible backs the surviving island"
        );
        scene
            .transform_sketch_island(s, islands[0], &affine)
            .unwrap();
        assert_eq!(
            scene.sketch_island_edges(s, islands[0]).len(),
            4,
            "exactly the visible shape moved"
        );
    }

    /// Move+Alt's out-of-plane sketch copy across the FFI: copying a ground
    /// island straight up Z lands a NEW sketch on the lifted plane, leaves
    /// the source in place, and is ONE undo step that hides just the copy.
    #[test]
    fn copy_sketch_island_out_of_plane_lands_a_new_sketch_and_undoes() {
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        let islands = scene.sketch_island_ids(s);
        assert_eq!(islands.len(), 1);

        let up = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.1,
        ];
        let copy = scene.copy_sketch_islands(s, &islands, &up).unwrap();
        assert_ne!(copy, s, "the copy is its own sketch");

        // Both sketches are live; the source is unchanged on the ground.
        let ids = scene.sketch_ids();
        assert!(ids.contains(&s) && ids.contains(&copy));
        assert_eq!(scene.sketch_island_edges(s, islands[0]).len(), 4);
        let src_plane = scene.sketch_plane(s).unwrap();
        assert!(
            src_plane[2].abs() < 1e-9,
            "source origin still on the ground"
        );
        let copy_plane = scene.sketch_plane(copy).unwrap();
        assert!(
            (copy_plane[2] - 0.1).abs() < 1e-9,
            "copy plane lifted to z=0.1"
        );

        // ONE undo step removes only the copy; the source stays.
        scene.scene_undo().unwrap();
        let ids = scene.sketch_ids();
        assert!(
            ids.contains(&s) && !ids.contains(&copy),
            "only the copy is gone"
        );
        assert_eq!(scene.sketch_island_edges(s, islands[0]).len(), 4);

        // Redo brings the copy back with the same handle.
        scene.scene_redo().unwrap();
        assert!(scene.sketch_ids().contains(&copy));
    }

    /// Deleting the wall an extruded solid's base shared with a live square
    /// simply OPENS the neighbor (the extruded side's edges were deleted at
    /// extrusion — there is nothing left to merge with). Deleting the solid
    /// resurrects nothing; redrawing the wall closes the neighbor again and
    /// it extrudes freely (adjacent to the solid, not under it).
    #[test]
    fn deleting_the_shared_wall_opens_the_neighbor_and_resurrects_nothing() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        for (a, b) in [
            ([1.0, 0.0], [2.0, 0.0]),
            ([2.0, 0.0], [2.0, 1.0]),
            ([2.0, 1.0], [1.0, 1.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        let obj = scene.extrude_region(s, r, 1.0).unwrap();

        // The shared wall survives the extrusion (the neighbor needs it).
        let wall = scene
            .pick_sketch_edge(1.0, 0.5, 5.0, 0.0, 0.0, -1.0)
            .expect("shared wall is pickable");
        scene.sketch_begin_gesture(s).unwrap();
        scene
            .sketch_remove_edge(wall.sketch(), wall.edge())
            .unwrap();
        scene.sketch_end_gesture(s).unwrap();

        assert_eq!(
            scene.sketch_regions(s).unwrap().len(),
            0,
            "removing the wall opened the neighbor — no region closes"
        );
        scene.delete_node(0, obj).unwrap();
        assert_eq!(
            scene.sketch_regions(s).unwrap().len(),
            0,
            "deleting the solid resurrects nothing"
        );

        // Redraw the wall: the neighbor closes and extrudes freely.
        scene.sketch_begin_gesture(s).unwrap();
        scene
            .sketch_add_segment(s, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0)
            .unwrap();
        scene.sketch_end_gesture(s).unwrap();
        let regions = scene.sketch_regions(s).unwrap();
        assert_eq!(regions.len(), 1);
        scene.extrude_region(s, regions[0], 1.0).unwrap();
    }

    /// Tipping an island out of its sketch plane commits instead of
    /// refusing: sole island → whole-sketch bake (handle stable); island of
    /// a shared sketch → detach into a new sketch. This is the Rotate-tool
    /// "stand a drawn profile upright" path.
    #[test]
    fn island_rotates_out_of_plane_via_bake_or_detach() {
        // Rotate 90 deg about the X axis through the origin (row-major 3x4).
        let rot_x_90 = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, -1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0,
        ];

        // Sole island: the sketch itself tips upright; no new sketch.
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        let islands = scene.sketch_island_ids(s);
        assert_eq!(islands.len(), 1);
        assert!(
            scene.can_transform_sketch_island(s, islands[0], &rot_x_90),
            "out-of-plane rotation validates"
        );
        scene
            .transform_sketch_island(s, islands[0], &rot_x_90)
            .unwrap();
        assert_eq!(
            scene.sketch_ids(),
            vec![s],
            "whole-sketch bake, same handle"
        );
        scene.scene_undo().unwrap();

        // Shared sketch: a second island forces the detach arm.
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        for (a, b) in [
            ([3.0, 0.0], [4.0, 0.0]),
            ([4.0, 0.0], [4.0, 1.0]),
            ([4.0, 1.0], [3.0, 1.0]),
            ([3.0, 1.0], [3.0, 0.0]),
        ] {
            scene
                .sketch_add_segment(s, a[0], a[1], 0.0, b[0], b[1], 0.0)
                .unwrap();
        }
        let islands = scene.sketch_island_ids(s);
        assert_eq!(islands.len(), 2);
        let target = *islands
            .iter()
            .find(|&&i| {
                scene
                    .sketch_island_lines(s, i)
                    .unwrap()
                    .iter()
                    .step_by(3)
                    .all(|&x| x < 2.0)
            })
            .expect("the unit square island");
        assert!(scene.can_transform_sketch_island(s, target, &rot_x_90));
        scene.transform_sketch_island(s, target, &rot_x_90).unwrap();
        let ids = scene.sketch_ids();
        assert_eq!(ids.len(), 2, "the island detached into its own sketch");
        assert!(ids.contains(&s));
        assert_eq!(
            scene.sketch_island_ids(s).len(),
            1,
            "the source keeps only its other island"
        );
        // Undo restores the shared sketch and hides the detached one.
        scene.scene_undo().unwrap();
        assert_eq!(scene.sketch_ids(), vec![s]);
        assert_eq!(scene.sketch_island_ids(s).len(), 2);
    }

    #[test]
    fn transform_sketch_moves_and_is_undoable() {
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        // Row-major 3x4: identity linear, translate +X by 5.
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene.transform_sketch(s, &affine).unwrap();
        assert!(
            scene.sketch_ids().contains(&s),
            "the sketch is still live and visible after transform"
        );
        scene.scene_undo().unwrap();
        assert!(
            scene.sketch_ids().contains(&s),
            "still there after undo, same handle"
        );
    }

    #[test]
    fn transform_sketch_rejects_bad_affine_and_reflection() {
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);

        let short = [1.0, 0.0, 0.0];
        assert!(
            scene
                .transform_sketch(s, &short)
                .unwrap_err()
                .0
                .starts_with("BadAffine")
        );
        let reflect = [
            -1.0, 0.0, 0.0, 0.0, //
            0.0, -1.0, 0.0, 0.0, //
            0.0, 0.0, -1.0, 0.0,
        ];
        let err = scene.transform_sketch(s, &reflect).unwrap_err();
        assert!(err.0.starts_with("Reflection"), "got {}", err.0);
    }

    #[test]
    fn pick_and_move_sketch_vertex_is_undoable() {
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        // Ray straight down onto the (1,1) corner picks that exact vertex.
        let pick = scene
            .pick_sketch_vertex(1.0, 1.0, 5.0, 0.0, 0.0, -1.0)
            .expect("a vertex sits under the (1,1) ray");
        assert_eq!(pick.sketch(), s);
        assert!((pick.x() - 1.0).abs() < 1e-9 && (pick.y() - 1.0).abs() < 1e-9);

        // Nudge it; topology preserved, so the sketch stays live and undoable.
        scene
            .move_sketch_vertex(s, pick.vertex(), 1.4, 0.8, 0.0)
            .unwrap();
        assert!(scene.sketch_ids().contains(&s));
        // The vertex is now pickable at its new spot, not the old one.
        assert!(
            scene
                .pick_sketch_vertex(1.0, 1.0, 5.0, 0.0, 0.0, -1.0)
                .is_none()
        );
        assert!(
            scene
                .pick_sketch_vertex(1.4, 0.8, 5.0, 0.0, 0.0, -1.0)
                .is_some()
        );

        scene.scene_undo().unwrap();
        assert!(
            scene
                .pick_sketch_vertex(1.0, 1.0, 5.0, 0.0, 0.0, -1.0)
                .is_some()
        );
    }

    #[test]
    fn move_sketch_vertex_rejects_a_retopologizing_drag() {
        let mut scene = Scene::new();
        let (s, _r) = ground_unit_square(&mut scene);
        let pick = scene
            .pick_sketch_vertex(0.0, 0.0, 5.0, 0.0, 0.0, -1.0)
            .expect("a vertex sits under the (0,0) ray");
        // Drag corner (0,0) across to (2, 0.5): its edges sweep over the far
        // side → refused as a typed Sketch error, sketch untouched.
        let err = scene
            .move_sketch_vertex(s, pick.vertex(), 2.0, 0.5, 0.0)
            .unwrap_err();
        assert!(err.0.starts_with("WouldRetopologize"), "got {}", err.0);
        assert!(
            scene
                .pick_sketch_vertex(0.0, 0.0, 5.0, 0.0, 0.0, -1.0)
                .is_some()
        );
    }

    /// Two top-level boxes group into one node, transform together, and ungroup
    /// back — all non-destructively and undoably across the FFI.
    #[test]
    fn group_transform_ungroup_round_trip() {
        let mut scene = Scene::new();
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();

        // Group both objects (kind 0 = object).
        let g = scene.group_nodes(&[0, 0], &[o1, o2]).unwrap();
        // Both objects stay visible (non-destructive), and the group is the
        // sole top-level node listing them both.
        assert_eq!(scene.object_ids().len(), 2, "members stay visible");
        let top = scene.top_level_nodes();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].kind(), "group");
        assert_eq!(top[0].id(), g);
        let members = scene.group_members(g);
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|m| m.kind() == "object"));
        // Both objects flatten as the group's leaves.
        let mut leaves = scene.node_leaf_objects(1, g).unwrap();
        leaves.sort_unstable();
        let mut expected = vec![o1, o2];
        expected.sort_unstable();
        assert_eq!(leaves, expected);
        assert_eq!(scene.node_parent(0, o1).unwrap(), Some(g));

        // Transform the group: bakes into both leaves, one undoable step.
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene.transform_group(g, &affine).unwrap();
        assert!(scene.can_scene_undo());
        scene.scene_undo().unwrap();

        // Ungroup: members return to the top level, group disappears.
        scene.ungroup(g).unwrap();
        assert!(scene.group_ids().is_empty());
        assert_eq!(scene.top_level_nodes().len(), 2);
        assert_eq!(scene.node_parent(0, o1).unwrap(), None);
    }

    /// A mixed selection — a bare object, a group, and a free sketch —
    /// transforms across the FFI as one undoable step, and bad inputs are
    /// refused with typed codes.
    #[test]
    fn transform_selection_round_trips_and_rejects_bad_input() {
        let mut scene = Scene::new();
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 2.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();
        let g = scene.group_nodes(&[0], &[o2]).unwrap();
        let free = scene.begin_ground_sketch();

        let hash_before = scene.state_hash();
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene
            .transform_selection(&[0, 1], &[o1, g], &[free], &affine)
            .unwrap();
        assert_ne!(scene.state_hash(), hash_before, "the selection moved");

        // One undo restores the whole act.
        scene.scene_undo().unwrap();
        assert_eq!(scene.state_hash(), hash_before, "one undo restores all");

        let err = scene
            .transform_selection(&[0, 0], &[o1], &[], &affine)
            .unwrap_err();
        assert!(err.0.starts_with("BadNodeList"), "got {}", err.0);
        let err = scene
            .transform_selection(&[], &[], &[], &affine)
            .unwrap_err();
        assert!(err.0.starts_with("EmptySelection"), "got {}", err.0);
        let err = scene
            .transform_selection(&[0], &[o1], &[], &affine[..7])
            .unwrap_err();
        assert!(err.0.starts_with("BadAffine"), "got {}", err.0);
    }

    #[test]
    fn group_rejects_mismatched_node_lists_and_bad_kinds() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();

        let err = scene.group_nodes(&[0, 0], &[o]).unwrap_err();
        assert!(err.0.starts_with("BadNodeList"), "got {}", err.0);

        let err = scene.group_nodes(&[7], &[o]).unwrap_err();
        assert!(err.0.starts_with("BadNodeKind"), "got {}", err.0);
    }

    /// Make a component, stamp a second instance, and confirm they share one
    /// definition with the pose round-tripping across the FFI.
    #[test]
    fn make_component_place_and_share_a_definition() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();

        // Make a component from the one object → returns the instance handle;
        // the object drops out of the world set (it is now a definition member).
        let inst = scene.make_component(&[0], &[o]).unwrap();
        assert!(!scene.object_ids().contains(&o));
        assert_eq!(scene.instance_ids(), vec![inst]);
        let comp = scene.instance_def(inst).unwrap();
        assert_eq!(scene.component_member_objects(comp), vec![o]);
        // The shared member mesh is still fetchable (drawn at each pose).
        assert!(scene.object_mesh(o).is_ok());

        // Stamp a second instance shifted in X; both share the definition.
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let inst2 = scene.place_instance(comp, &affine).unwrap();
        assert_eq!(scene.instance_ids().len(), 2);
        assert_eq!(scene.instances_of(comp).len(), 2);
        assert_eq!(scene.instance_def(inst2), Some(comp));
        // The pose round-trips as a 3×4 affine.
        assert_eq!(scene.instance_pose(inst2).unwrap(), affine.to_vec());
    }

    /// `component_member_objects` must filter out a member whose birth was
    /// undone — `Document::def_members` deliberately keeps the id listed
    /// (Group parity, simpler undo bookkeeping; see the kernel doc comment),
    /// so the wasm-api getter is the one place that must turn that into "not
    /// live" before a renderer ever hands the id to `object_mesh` (which
    /// errors for a hidden object rather than quietly drawing nothing).
    #[test]
    fn component_member_objects_excludes_a_member_whose_birth_was_undone() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o1]).unwrap();
        let comp = scene.instance_def(inst).unwrap();

        // Give the definition a second member via a def-owned sketch, so
        // deleting the first still leaves a live member behind
        // (`delete_def_member` refuses to delete a definition's last one).
        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        let mut def_region = None;
        for (ax, ay, bx, by) in [
            (3.0, 0.0, 4.0, 0.0),
            (4.0, 0.0, 4.0, 1.0),
            (4.0, 1.0, 3.0, 1.0),
            (3.0, 1.0, 3.0, 0.0),
        ] {
            let report = scene
                .sketch_add_segment(def_sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
            if let Some(&r) = report.inner.regions_created.first() {
                def_region = Some(r.data().as_ffi());
            }
        }
        let o2 = scene
            .extrude_region_in_instance(inst, def_sketch, def_region.expect("closed region"), 1.0)
            .unwrap();
        assert_eq!(
            scene.component_member_objects(comp).len(),
            2,
            "both members live before the delete"
        );

        scene.delete_def_member(comp, o1).unwrap();
        assert_eq!(
            scene.component_member_objects(comp),
            vec![o2],
            "the deleted member's id must not reach a renderer"
        );

        // Undo restores it — the tombstone-not-erase contract.
        scene.scene_undo().unwrap();
        let members = scene.component_member_objects(comp);
        assert_eq!(members.len(), 2);
        assert!(members.contains(&o1));
    }

    /// `component_member_sketches` mirrors the same filter for sketches:
    /// undoing the gesture that created a fresh def-owned sketch hides it
    /// (`Document::def_member_sketches` keeps its id listed regardless, same
    /// as the object case above) — the getter used to render/pick a
    /// definition's sketches must not hand back an id that no longer
    /// round-trips through the document.
    #[test]
    fn component_member_sketches_excludes_a_sketch_whose_creation_was_undone() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();

        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene.sketch_begin_gesture(def_sketch).unwrap();
        for (ax, ay, bx, by) in [
            (2.0, 2.0, 2.5, 2.0),
            (2.5, 2.0, 2.5, 2.5),
            (2.5, 2.5, 2.0, 2.5),
            (2.0, 2.5, 2.0, 2.0),
        ] {
            scene
                .sketch_add_segment(def_sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_gesture(def_sketch).unwrap();
        assert_eq!(scene.component_member_sketches(comp), vec![def_sketch]);

        // Undo the gesture: it created the sketch, so undoing hides it.
        scene.scene_undo().unwrap();
        assert_eq!(
            scene.component_member_sketches(comp),
            Vec::<u64>::new(),
            "an undone def sketch must not reach a renderer or a pick"
        );
    }

    #[test]
    fn active_component_context_registers_definition_sketch_inference_at_its_pose_only() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .transform_instance(
                inst,
                &[
                    1.0, 0.0, 0.0, 5.0, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.0,
                ],
            )
            .unwrap();
        scene.set_active_inference_instance(Some(inst));
        // Direct sketch mutation happens after the instance is posed. This
        // exercises the mutation-time refresh path, before a gesture commit
        // can reconcile the scene.
        scene
            .sketch_add_segment(sketch, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0)
            .unwrap();

        let posed = scene
            .snap(
                7.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("posed definition endpoint must snap");
        assert_eq!(posed.kind(), "endpoint");
        assert!((posed.x() - 7.0).abs() < kernel::tol::POINT_MERGE);

        scene.set_hidden(&[], &[inst]);
        let hidden = scene
            .snap(
                7.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap();
        assert_ne!(
            hidden.as_ref().map(|s| s.kind()).as_deref(),
            Some("endpoint"),
            "hiding the active instance removes its posed sketch inference"
        );
        scene.set_hidden(&[], &[]);
        let shown = scene
            .snap(
                7.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("showing the active instance restores its posed sketch inference");
        assert_eq!(shown.kind(), "endpoint");

        scene.set_active_inference_instance(None);
        let after_exit = scene
            .snap(
                7.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap();
        assert_ne!(
            after_exit.as_ref().map(|s| s.kind()).as_deref(),
            Some("endpoint"),
            "exiting the context removes the posed definition endpoint"
        );
        let local = scene
            .snap(
                2.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap();
        assert_ne!(
            local.as_ref().map(|s| s.kind()).as_deref(),
            Some("endpoint"),
            "definition-local coordinates never leak into world inference"
        );
    }

    #[test]
    fn active_component_context_registers_mirrored_definition_sketch_inference() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .sketch_add_segment(sketch, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0)
            .unwrap();
        scene
            .transform_instance(
                inst,
                &[
                    -1.0, 0.0, 0.0, 5.0, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.0,
                ],
            )
            .unwrap();
        scene.set_active_inference_instance(Some(inst));

        let mirrored = scene
            .snap(
                3.0, 0.0, 5.0, 0.0, 0.0, -1.0, 0.02, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("mirrored definition endpoint must snap");
        assert_eq!(mirrored.kind(), "endpoint");
        assert!((mirrored.x() - 3.0).abs() < kernel::tol::POINT_MERGE);
    }

    #[test]
    fn loading_a_document_clears_active_component_inference_context() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        scene.set_active_inference_instance(Some(inst));

        let replacement = Scene::new().save();
        scene.load_core(&replacement).unwrap();

        assert_eq!(scene.active_inference_instance, None);
        assert!(scene.active_inference_sketches.is_empty());
    }

    /// Draws an 'O'-like glyph outline (an outer square loop plus a smaller
    /// concentric "counter" square loop) through the exact FFI sequence the
    /// 3D Text tool uses — `begin_ground_sketch` → gesture-bracketed
    /// `sketch_add_segment` → `sketch_regions`/`region_boundary` to pick the
    /// fill region (the larger-extent one; the app's real selection uses the
    /// nesting-depth-parity rule this mirrors) — then `place_text`. Proves
    /// the whole wasm surface end-to-end: only the ring extrudes (not the
    /// counter's own disk), the fold names the definition, and the result is
    /// watertight (the counter closed as a real tunnel, not a leak).
    #[test]
    fn place_text_o_glyph_via_full_gesture_pipeline_is_watertight() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        scene.sketch_begin_gesture(sketch).unwrap();
        let outer = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        for i in 0..4 {
            let (ax, ay) = outer[i];
            let (bx, by) = outer[(i + 1) % 4];
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        let inner = [(-0.4, -0.4), (0.4, -0.4), (0.4, 0.4), (-0.4, 0.4)];
        for i in 0..4 {
            let (ax, ay) = inner[i];
            let (bx, by) = inner[(i + 1) % 4];
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_gesture(sketch).unwrap();

        let region_ids = scene.sketch_regions(sketch).unwrap();
        assert_eq!(region_ids.len(), 2, "two nested loops trace as ring + disk");
        let extent = |r: u64| -> f64 {
            let pts = scene.region_boundary(sketch, r).unwrap();
            pts.chunks(3)
                .map(|p| ((p[0] * p[0] + p[1] * p[1]) as f64).sqrt())
                .fold(0.0_f64, f64::max)
        };
        let (ring, disk) = if extent(region_ids[0]) > extent(region_ids[1]) {
            (region_ids[0], region_ids[1])
        } else {
            (region_ids[1], region_ids[0])
        };
        let _ = disk; // the counter's own interior — deliberately never extruded

        let instance = scene
            .place_text(sketch, vec![ring], 0.5, "3D Text \"O\"".to_string(), None)
            .unwrap();
        let comp = scene.instance_def(instance).unwrap();
        let members = scene.component_member_objects(comp);
        assert_eq!(members.len(), 1, "only the ring's fill region extruded");
        assert!(
            scene.object_watertight(members[0]).unwrap(),
            "the counter closes as a tunnel, not a leak"
        );
    }

    /// `place_text` replays as ONE undo step across the FFI: the gesture and
    /// the extrude+fold it bundles reproduce the golden state hash and
    /// byte-identical saved document.
    #[test]
    fn record_then_replay_covers_place_text() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();

        let sketch = scene.begin_ground_sketch();
        scene.sketch_begin_gesture(sketch).unwrap();
        let outer = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        for i in 0..4 {
            let (ax, ay) = outer[i];
            let (bx, by) = outer[(i + 1) % 4];
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        let inner = [(-0.4, -0.4), (0.4, -0.4), (0.4, 0.4), (-0.4, 0.4)];
        for i in 0..4 {
            let (ax, ay) = inner[i];
            let (bx, by) = inner[(i + 1) % 4];
            scene
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }
        scene.sketch_end_gesture(sketch).unwrap();
        let region_ids = scene.sketch_regions(sketch).unwrap();
        let extent = |r: u64| -> f64 {
            let pts = scene.region_boundary(sketch, r).unwrap();
            pts.chunks(3)
                .map(|p| ((p[0] * p[0] + p[1] * p[1]) as f64).sqrt())
                .fold(0.0_f64, f64::max)
        };
        let ring = if extent(region_ids[0]) > extent(region_ids[1]) {
            region_ids[0]
        } else {
            region_ids[1]
        };
        scene
            .place_text(sketch, vec![ring], 0.5, "3D Text \"O\"".to_string(), None)
            .unwrap();

        // An undo at the end exercises the compound stack shape too.
        scene.scene_undo().unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        assert_eq!(
            replayed.replay(&json).unwrap(),
            golden,
            "replaying a place_text session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    /// Transform composes into the pose; explode bakes to a world object and
    /// undoes; make_unique detaches a sibling — all across the FFI.
    #[test]
    fn transform_explode_and_make_unique_round_trip() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();

        let mv = [
            1.0, 0.0, 0.0, 2.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene.transform_instance(inst, &mv).unwrap();
        assert_eq!(scene.instance_pose(inst).unwrap(), mv.to_vec());

        // Explode → one independent world object; the instance is gone.
        let created = scene.explode_instance(inst).unwrap();
        assert_eq!(created.len(), 1);
        assert!(scene.object_ids().contains(&created[0]));
        assert!(scene.instance_ids().is_empty());

        // Undo explode → the instance returns, the world object disappears.
        scene.scene_undo().unwrap();
        assert_eq!(scene.instance_ids(), vec![inst]);
        assert!(!scene.object_ids().contains(&created[0]));

        // make_unique detaches a placed sibling onto its own definition.
        let inst2 = scene.place_instance(comp, &mv).unwrap();
        let new_comp = scene.make_unique(inst2).unwrap();
        assert_ne!(new_comp, comp);
        assert_eq!(scene.instance_def(inst2), Some(new_comp));
        assert_eq!(scene.instance_def(inst), Some(comp));
    }

    /// `sketch_edge_endpoints` promises WORLD endpoints, but a def-owned
    /// sketch's edges are DEFINITION-local — a raw answer would hand a
    /// Tape-Measure-style caller coordinates in the wrong frame (the same bug
    /// class as an unguarded follow-me path sketch). It must refuse exactly
    /// like `edge_endpoints` refuses a definition-member object.
    #[test]
    fn sketch_edge_endpoints_refuses_a_def_owned_sketch() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();

        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .sketch_add_segment(def_sketch, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();
        let edge = scene
            .doc
            .sketch(sketch_id(def_sketch))
            .unwrap()
            .edges()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();

        assert_eq!(
            scene.sketch_edge_endpoints(def_sketch, edge),
            None,
            "a def-owned sketch edge must not answer as world endpoints"
        );
    }

    /// `edge_endpoints_in_instance` is the object-edge analog of
    /// `sketch_edge_endpoints_in_instance`: a posed instance's member-object
    /// edge maps into world space through that instance's pose.
    #[test]
    fn edge_endpoints_in_instance_maps_a_posed_member_edge_to_world_space() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(comp)[0];

        let edge = scene
            .doc
            .object(object_id(member))
            .unwrap()
            .edges()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();
        let (local_a, local_b) = scene
            .doc
            .object(object_id(member))
            .unwrap()
            .edge_endpoints(EdgeId::from(KeyData::from_ffi(edge)))
            .unwrap();

        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 2.0, //
            0.0, 0.0, 1.0, 3.0,
        ];
        scene.transform_instance(inst, &affine).unwrap();

        let got = scene
            .edge_endpoints_in_instance(inst, member, edge)
            .expect("a posed member edge resolves to world endpoints");
        assert_eq!(
            got,
            vec![
                local_a.x + 5.0,
                local_a.y + 2.0,
                local_a.z + 3.0,
                local_b.x + 5.0,
                local_b.y + 2.0,
                local_b.z + 3.0,
            ]
        );
    }

    /// A world-space (non-definition) object has no meaningful "instance" to
    /// view it through, so `edge_endpoints_in_instance` refuses it — the
    /// caller falls back to `edge_endpoints` for world objects.
    #[test]
    fn edge_endpoints_in_instance_refuses_a_world_space_object() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();

        let (s2, r2) = ground_unit_square_at(&mut scene, 5.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o2]).unwrap();

        let edge = scene
            .doc
            .object(object_id(o))
            .unwrap()
            .edges()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();

        assert_eq!(
            scene.edge_endpoints_in_instance(inst, o, edge),
            None,
            "a world-space object must not resolve through an unrelated instance"
        );
    }

    /// An object owned by a *different* component's definition must not
    /// resolve through an instance of another definition — the object-side
    /// analog of `sketch_edge_endpoints_in_instance`'s cross-definition guard.
    #[test]
    fn edge_endpoints_in_instance_refuses_a_different_definitions_object() {
        let mut scene = Scene::new();
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        let inst1 = scene.make_component(&[0], &[o1]).unwrap();

        let (s2, r2) = ground_unit_square_at(&mut scene, 5.0, 0.0);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();
        let inst2 = scene.make_component(&[0], &[o2]).unwrap();
        let comp2 = scene.instance_def(inst2).unwrap();
        let member2 = scene.component_member_objects(comp2)[0];

        let edge = scene
            .doc
            .object(object_id(member2))
            .unwrap()
            .edges()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();

        assert_eq!(
            scene.edge_endpoints_in_instance(inst1, member2, edge),
            None,
            "an object owned by a different component's definition must not resolve"
        );
    }

    /// A stale/nonexistent instance handle must not resolve, regardless of
    /// whether `object`/`edge` are otherwise live.
    #[test]
    fn edge_endpoints_in_instance_refuses_a_stale_instance_handle() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(comp)[0];

        let edge = scene
            .doc
            .object(object_id(member))
            .unwrap()
            .edges()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();

        assert_eq!(
            scene.edge_endpoints_in_instance(u64::MAX, member, edge),
            None,
            "a stale instance handle must not resolve"
        );
    }

    /// Editing inside a component (push/pull a shared member face) succeeds via
    /// `push_pull_in_component` and is one undoable document action; both
    /// instances keep referencing the definition.
    #[test]
    fn editing_inside_a_component_is_undoable() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let inst2 = scene.place_instance(comp, &affine).unwrap();

        // Pull the shared member's +Z (top) face up.
        let member = scene.component_member_objects(comp)[0];
        let top = {
            let object = scene.doc.object(object_id(member)).unwrap();
            object
                .faces()
                .iter()
                .find(|(_, f)| {
                    f.plane.normal().approx_eq(
                        kernel::Vec3::new(0.0, 0.0, 1.0),
                        kernel::tol::NORMAL_DIRECTION,
                    )
                })
                .map(|(fid, _)| fid.data().as_ffi())
                .unwrap()
        };
        scene
            .push_pull_in_component(inst, member, top, 1.0)
            .unwrap();

        // One document action for the edit; undo restores it, both instances live.
        scene.scene_undo().unwrap();
        assert_eq!(scene.instance_ids().len(), 2);
        assert_eq!(scene.instance_def(inst), Some(comp));
        assert_eq!(scene.instance_def(inst2), Some(comp));
    }

    #[test]
    fn component_push_pull_automatically_routes_an_overshoot_to_through_cut() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));
        let loop_pts = [
            0.2, 0.2, 1.0, //
            0.8, 0.2, 1.0, //
            0.8, 0.8, 1.0, //
            0.2, 0.8, 1.0,
        ];
        let sub = scene
            .split_face_inner_in_instance(inst, member, top, &loop_pts)
            .unwrap();
        let before_cut = scene.state_hash();
        let report = scene
            .push_pull_in_component(inst, member, sub, -1.5)
            .expect("overshoot becomes a subtracting hole");
        assert!(report.is_through());
        assert_eq!(report.result_objects().len(), 1);
        assert!(!scene.component_member_objects(comp).contains(&member));
        scene.scene_undo().unwrap();
        assert_eq!(scene.state_hash(), before_cut);
        scene.scene_redo().unwrap();
        assert_eq!(
            scene.component_member_objects(comp),
            report.result_objects()
        );
    }

    /// The face of `obj` whose outward normal most nearly matches `dir` —
    /// the wasm-level counterpart of `component_edit_k2_specs.rs`'s
    /// `face_matching`.
    fn wasm_face_matching(scene: &Scene, obj: u64, dir: kernel::Vec3) -> u64 {
        let object = scene.doc.object(object_id(obj)).unwrap();
        object
            .faces()
            .iter()
            .max_by(|(_, a), (_, b)| {
                a.plane
                    .normal()
                    .dot(dir)
                    .partial_cmp(&b.plane.normal().dot(dir))
                    .unwrap()
            })
            .map(|(fid, _)| fid.data().as_ffi())
            .unwrap()
    }

    /// A full component-edit-parity.md **phase K2** session — every new
    /// surface's wasm export exercised at least once (Follow Me's three
    /// in-instance variants, `boolean_in_component`, `slice_def_member`,
    /// `transform_def_member`), mixed with undo/redo — records and replays
    /// to the exact same state. The real risk this pins down is the
    /// `RecordedCall` enum/replay-match wiring itself (the "enum trap"): the
    /// underlying kernel surfaces already carry their own executable specs
    /// in `component_edit_k2_specs.rs`, so the shapes here are copied
    /// verbatim from proven-safe specs there rather than re-derived.
    #[test]
    fn component_edit_k2_session_records_and_replays() {
        recording::reset();
        let mut scene = Scene::new();
        scene.start_recording();

        // A component with member A (1x1x1 at the origin).
        let (s, r) = ground_unit_square(&mut scene);
        let a = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[a]).unwrap();
        let comp = scene.instance_def(inst).unwrap();

        // Member B, offset in x, through the SAME instance
        // (`extrude_region_in_instance`) — general position for the boolean.
        let gb = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        let mut rb = None;
        for (ax, ay, bx, by) in [
            (2.0, 0.0, 3.0, 0.0),
            (3.0, 0.0, 3.0, 1.0),
            (3.0, 1.0, 2.0, 1.0),
            (2.0, 1.0, 2.0, 0.0),
        ] {
            let report = scene
                .sketch_add_segment(gb, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
            if let Some(&rr) = report.inner.regions_created.first() {
                rb = Some(rr.data().as_ffi());
            }
        }
        let b = scene
            .extrude_region_in_instance(inst, gb, rb.unwrap(), 1.0)
            .unwrap();

        // `boolean_in_component`: union A and B into one member.
        let unioned = scene.boolean_in_component(comp, 0, a, b).unwrap();

        // `slice_def_member`: cut it at x = 2.5 (a world-space plane through
        // the instance's — here identity — pose).
        let pieces = scene
            .slice_def_member(inst, unioned, &[2.5, 0.5, 0.0, 1.0, 0.0, 0.0])
            .unwrap();
        let piece = pieces[0];

        // `transform_def_member`: bake a small world-space translation into it.
        scene
            .transform_def_member(
                inst,
                piece,
                &[
                    1.0, 0.0, 0.0, 0.1, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, 0.2,
                ],
            )
            .unwrap();

        // Member C, a 4x2x1 box far away, dedicated to the Follow Me family
        // (the `component_edit_k2_specs.rs` `boxed_component` shape, offset
        // +10 in x).
        let gc = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        let mut rc = None;
        for (ax, ay, bx, by) in [
            (10.0, 0.0, 14.0, 0.0),
            (14.0, 0.0, 14.0, 2.0),
            (14.0, 2.0, 10.0, 2.0),
            (10.0, 2.0, 10.0, 0.0),
        ] {
            let report = scene
                .sketch_add_segment(gc, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
            if let Some(&rr) = report.inner.regions_created.first() {
                rc = Some(rr.data().as_ffi());
            }
        }
        let c = scene
            .extrude_region_in_instance(inst, gc, rc.unwrap(), 1.0)
            .unwrap();

        // `follow_me_along_edges_in_instance`: a small square on a vertical
        // plane, swept along an L-shaped path — both def-owned sketches,
        // fully disconnected from every other member (the
        // `follow_me_commits_like_extrusion...` shape).
        let path_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 20.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .sketch_add_segment(path_sketch, 20.0, 0.0, 0.0, 22.0, 0.0, 0.0)
            .unwrap();
        scene
            .sketch_add_segment(path_sketch, 22.0, 0.0, 0.0, 22.0, 2.0, 0.0)
            .unwrap();
        let path_edges: Vec<u64> = scene
            .doc
            .sketch(sketch_id(path_sketch))
            .unwrap()
            .edges()
            .keys()
            .map(|e| e.data().as_ffi())
            .collect();
        let profile_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 20.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();
        let mut profile_region = None;
        for (ay, az, by, bz) in [
            (-0.3, -0.3, 0.3, -0.3),
            (0.3, -0.3, 0.3, 0.3),
            (0.3, 0.3, -0.3, 0.3),
            (-0.3, 0.3, -0.3, -0.3),
        ] {
            let report = scene
                .sketch_add_segment(profile_sketch, 20.0, ay, az, 20.0, by, bz)
                .unwrap();
            if let Some(&rr) = report.inner.regions_created.first() {
                profile_region = Some(rr.data().as_ffi());
            }
        }
        let d = scene
            .follow_me_along_edges_in_instance(
                inst,
                profile_sketch,
                profile_region.unwrap(),
                path_sketch,
                path_edges,
                None,
            )
            .unwrap();

        // `follow_me_around_face_in_instance`: a small profile straddling
        // C's top rim, swept around it — C is untouched (non-merging).
        let top_c = wasm_face_matching(&scene, c, kernel::Vec3::new(0.0, 0.0, 1.0));
        let molding_profile = |scene: &mut Scene, x: f64| -> u64 {
            let sk = scene
                .begin_sketch_on_plane_in_instance(inst, x, 0.0, 0.0, 1.0, 0.0, 0.0)
                .unwrap();
            let mut region = None;
            for (ay, az, by, bz) in [
                (-0.3, 0.9, -0.05, 0.9),
                (-0.05, 0.9, -0.05, 1.15),
                (-0.05, 1.15, -0.3, 1.15),
                (-0.3, 1.15, -0.3, 0.9),
            ] {
                let report = scene.sketch_add_segment(sk, x, ay, az, x, by, bz).unwrap();
                if let Some(&rr) = report.inner.regions_created.first() {
                    region = Some(rr.data().as_ffi());
                }
            }
            let _ = region;
            sk
        };
        let ps1 = molding_profile(&mut scene, 10.5);
        let pr1 = scene
            .doc
            .sketch(sketch_id(ps1))
            .unwrap()
            .regions()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();
        let e = scene
            .follow_me_around_face_in_instance(inst, ps1, pr1, c, top_c, None)
            .unwrap();

        // `follow_me_merged_around_face_in_instance`: the SAME rim shape,
        // this time consuming C into the merged result (the
        // `follow_me_merged_in_instance_carves_the_path_member_in_one_step`
        // shape).
        let ps2 = molding_profile(&mut scene, 12.0);
        let pr2 = scene
            .doc
            .sketch(sketch_id(ps2))
            .unwrap()
            .regions()
            .keys()
            .next()
            .unwrap()
            .data()
            .as_ffi();
        let merged = scene
            .follow_me_merged_around_face_in_instance(inst, ps2, pr2, c, top_c, None)
            .unwrap();

        // `follow_me_face_along_edges_in_instance`: a face of D as the
        // profile, swept along a fresh path sketch — non-merging.
        let d_side = wasm_face_matching(&scene, d, kernel::Vec3::new(1.0, 0.0, 0.0));
        let path2 = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .sketch_add_segment(path2, 30.0, 0.0, 0.0, 32.0, 0.0, 0.0)
            .unwrap();
        let path2_edges: Vec<u64> = scene
            .doc
            .sketch(sketch_id(path2))
            .unwrap()
            .edges()
            .keys()
            .map(|e| e.data().as_ffi())
            .collect();
        let g = scene
            .follow_me_face_along_edges_in_instance(inst, d, d_side, path2, path2_edges, None)
            .unwrap();

        // `follow_me_face_around_face_in_instance`: a face of G swept around
        // a face of `merged` — non-merging (different objects).
        let g_face = wasm_face_matching(&scene, g, kernel::Vec3::new(0.0, 0.0, 1.0));
        let merged_face = wasm_face_matching(&scene, merged, kernel::Vec3::new(0.0, 0.0, -1.0));
        let _h = scene
            .follow_me_face_around_face_in_instance(inst, g, g_face, merged, merged_face, None)
            .unwrap();

        scene.scene_undo().unwrap();
        scene.scene_redo().unwrap();

        // A definition-owned sketch transform records as one compound call,
        // exercising the replay arm that keeps object/sketch selections
        // atomic.
        let ds = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        scene
            .sketch_add_segment(ds, 6.0, 0.0, 0.0, 6.5, 0.0, 0.0)
            .unwrap();
        let move_sketch = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.5, //
            0.0, 0.0, 1.0, 0.0,
        ];
        scene
            .transform_def_selection(inst, &[], &[ds], &[], &[], &move_sketch)
            .unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying a full K2 session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save());
        let _ = e;
    }

    /// `split_face_inner_in_instance` maps a WORLD-space loop through the
    /// instance's pose⁻¹ before imprinting — proven with a genuinely
    /// non-identity (translated) pose, unlike `component_edit_k2_session_
    /// records_and_replays`'s identity-posed instance, where local and world
    /// coordinates coincide and a mapping bug would go unnoticed. The
    /// imprinted sub-face's boundary (`face_boundary`, definition-local) must
    /// equal the loop's LOCAL coordinates, not the world ones passed in.
    #[test]
    fn split_face_inner_in_instance_imprints_through_a_translated_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        // Re-place the ONLY instance at a translation so its pose is
        // genuinely non-identity (make_component's own instance keeps the
        // object's original, identity placement).
        scene.delete_node(2, inst).unwrap();
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 2.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let inst = scene.place_instance(comp, &affine).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        // A small square strictly inside the top face, in WORLD coordinates
        // (the translated instance's placement) — local coordinates would be
        // (0.2,0.2)-(0.8,0.2)-(0.8,0.8)-(0.2,0.8) at z=1.
        let world_loop = [
            5.2, 2.2, 1.0, //
            5.8, 2.2, 1.0, //
            5.8, 2.8, 1.0, //
            5.2, 2.8, 1.0,
        ];
        let sub_face = scene
            .split_face_inner_in_instance(inst, member, top, &world_loop)
            .unwrap();

        // The imprinted sub-face's boundary is DEFINITION-local — it must
        // match the loop mapped through pose⁻¹, not the world points given.
        let boundary = scene.face_boundary(member, sub_face).unwrap();
        assert_eq!(
            boundary.len(),
            12,
            "a 4-point loop imprints a 4-vertex sub-face"
        );
        let mut xs: Vec<f64> = (0..4).map(|i| boundary[i * 3] as f64).collect();
        let mut ys: Vec<f64> = (0..4).map(|i| boundary[i * 3 + 1] as f64).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        assert!((xs[0] - 0.2).abs() < 1e-6 && (xs[3] - 0.8).abs() < 1e-6);
        assert!((ys[0] - 0.2).abs() < 1e-6 && (ys[3] - 0.8).abs() < 1e-6);

        // Every instance of the definition sees the new sub-face at once.
        assert_eq!(scene.instances_of(comp), vec![inst]);
    }

    /// Same imprint, through a ROTATED + uniformly-SCALED pose — a
    /// translation-only pose leaves local axes parallel to world axes, which
    /// would hide an inverse-transpose/scale mistake that only shows up once
    /// the pose actually rotates or scales (the coverage gap the
    /// `CurveClaimOffLoop` finding traced back to this exact test family
    /// using only translation/identity poses).
    #[test]
    fn split_face_inner_in_instance_imprints_through_a_rotated_and_scaled_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        let pose = kernel::Transform::rotation(
            kernel::Vec3::new(0.0, 0.0, 1.0),
            std::f64::consts::FRAC_PI_2,
        )
        .unwrap()
        .then(&kernel::Transform::scale(kernel::Vec3::new(2.0, 2.0, 2.0)))
        .then(&kernel::Transform::translation(kernel::Vec3::new(
            5.0, 2.0, 0.0,
        )));
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        // The same local square as the translated-pose sibling spec, mapped
        // to WORLD through this rotated+scaled pose — exactly what a real
        // draw tool sends.
        let local_corners = [
            Point3::new(0.2, 0.2, 1.0),
            Point3::new(0.8, 0.2, 1.0),
            Point3::new(0.8, 0.8, 1.0),
            Point3::new(0.2, 0.8, 1.0),
        ];
        let world_loop: Vec<f64> = local_corners
            .iter()
            .flat_map(|&p| {
                let w = pose.apply_point(p);
                [w.x, w.y, w.z]
            })
            .collect();

        let sub_face = scene
            .split_face_inner_in_instance(inst, member, top, &world_loop)
            .unwrap();

        let boundary = scene.face_boundary(member, sub_face).unwrap();
        assert_eq!(boundary.len(), 12);
        let mut xs: Vec<f64> = (0..4).map(|i| boundary[i * 3] as f64).collect();
        let mut ys: Vec<f64> = (0..4).map(|i| boundary[i * 3 + 1] as f64).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        assert!((xs[0] - 0.2).abs() < 1e-6 && (xs[3] - 0.8).abs() < 1e-6);
        assert!((ys[0] - 0.2).abs() < 1e-6 && (ys[3] - 0.8).abs() < 1e-6);
    }

    /// Same imprint, through a MIRRORED pose (negative determinant, uniform
    /// |scale| = 1) — the design doc's own called-out highest-scrutiny case
    /// ("pose⁻¹ correctness is most visible on mirrors").
    #[test]
    fn split_face_inner_in_instance_imprints_through_a_mirrored_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        let pose = kernel::Transform::scale(kernel::Vec3::new(-1.0, 1.0, 1.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(7.0, 4.0, 0.0)),
        );
        assert!(pose.determinant() < 0.0, "sanity: this pose mirrors");
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        let local_corners = [
            Point3::new(0.2, 0.2, 1.0),
            Point3::new(0.8, 0.2, 1.0),
            Point3::new(0.8, 0.8, 1.0),
            Point3::new(0.2, 0.8, 1.0),
        ];
        let world_loop: Vec<f64> = local_corners
            .iter()
            .flat_map(|&p| {
                let w = pose.apply_point(p);
                [w.x, w.y, w.z]
            })
            .collect();

        let sub_face = scene
            .split_face_inner_in_instance(inst, member, top, &world_loop)
            .unwrap();

        let boundary = scene.face_boundary(member, sub_face).unwrap();
        assert_eq!(boundary.len(), 12);
        let mut xs: Vec<f64> = (0..4).map(|i| boundary[i * 3] as f64).collect();
        let mut ys: Vec<f64> = (0..4).map(|i| boundary[i * 3 + 1] as f64).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        assert!((xs[0] - 0.2).abs() < 1e-6 && (xs[3] - 0.8).abs() < 1e-6);
        assert!((ys[0] - 0.2).abs() < 1e-6 && (ys[3] - 0.8).abs() < 1e-6);
    }

    /// `split_face_inner_with_curve_in_instance` — the curve-carrying sibling
    /// used by CircleTool's face mode inside a component. Only checks the
    /// instance-aware plumbing (the call succeeds and produces a face); the
    /// curve-identity-enables-a-cylinder-wall semantics themselves are
    /// already proven world-side.
    #[test]
    fn split_face_inner_with_curve_in_instance_succeeds() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        // An octagon standing in for a circle of radius 0.3 around (0.5,0.5,1)
        // — identity-posed instance, so world coordinates are local ones.
        let mut world_loop = Vec::new();
        for i in 0..8 {
            let theta = std::f64::consts::TAU * (i as f64) / 8.0;
            world_loop.push(0.5 + 0.3 * theta.cos());
            world_loop.push(0.5 + 0.3 * theta.sin());
            world_loop.push(1.0);
        }
        let sub_face = scene
            .split_face_inner_with_curve_in_instance(
                inst,
                member,
                top,
                &world_loop,
                &[0.5, 0.5, 1.0],
                0.3,
            )
            .unwrap();
        assert!(scene.face_boundary(member, sub_face).unwrap().len() >= 24);
    }

    /// The exact `CurveClaimOffLoop` regression this finding traced: the
    /// identity-posed sibling above coincidentally makes world == local, so
    /// it cannot catch a curve center/radius that isn't mapped through the
    /// instance's pose. Here the pose genuinely rotates and uniformly
    /// scales — before the fix, this failed outright every time with
    /// `CurveClaimOffLoop` (the curve claim never matching the mapped
    /// loop); after it, `center` maps through `pose⁻¹` and `radius` divides
    /// by the pose's uniform scale, exactly like `loop_pts`/the loop-only
    /// sibling spec.
    #[test]
    fn split_face_inner_with_curve_in_instance_maps_the_circle_through_a_rotated_and_scaled_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        let pose = kernel::Transform::rotation(
            kernel::Vec3::new(0.0, 0.0, 1.0),
            std::f64::consts::FRAC_PI_2,
        )
        .unwrap()
        .then(&kernel::Transform::scale(kernel::Vec3::new(2.0, 2.0, 2.0)))
        .then(&kernel::Transform::translation(kernel::Vec3::new(
            5.0, 2.0, 0.0,
        )));
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        let local_center = Point3::new(0.5, 0.5, 1.0);
        let local_radius = 0.3_f64;
        let mut world_loop = Vec::new();
        for i in 0..8 {
            let theta = std::f64::consts::TAU * (i as f64) / 8.0;
            let local_pt = Point3::new(
                local_center.x + local_radius * theta.cos(),
                local_center.y + local_radius * theta.sin(),
                1.0,
            );
            let w = pose.apply_point(local_pt);
            world_loop.extend([w.x, w.y, w.z]);
        }
        let world_center = pose.apply_point(local_center);
        let world_radius = local_radius * pose.similarity_scale().unwrap();

        let sub_face = scene
            .split_face_inner_with_curve_in_instance(
                inst,
                member,
                top,
                &world_loop,
                &[world_center.x, world_center.y, world_center.z],
                world_radius,
            )
            .expect(
                "center mapped through pose⁻¹ and radius through the uniform scale must match \
                 the mapped loop",
            );
        assert!(scene.face_boundary(member, sub_face).unwrap().len() >= 24);
    }

    /// Same, through a MIRRORED pose (negative determinant, uniform
    /// |scale| = 1) — the design doc's own called-out highest-scrutiny case.
    #[test]
    fn split_face_inner_with_curve_in_instance_maps_the_circle_through_a_mirrored_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        let pose = kernel::Transform::scale(kernel::Vec3::new(-1.0, 1.0, 1.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(7.0, 4.0, 0.0)),
        );
        assert!(pose.determinant() < 0.0, "sanity: this pose mirrors");
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        let local_center = Point3::new(0.5, 0.5, 1.0);
        let local_radius = 0.3_f64;
        let mut world_loop = Vec::new();
        for i in 0..8 {
            let theta = std::f64::consts::TAU * (i as f64) / 8.0;
            let local_pt = Point3::new(
                local_center.x + local_radius * theta.cos(),
                local_center.y + local_radius * theta.sin(),
                1.0,
            );
            let w = pose.apply_point(local_pt);
            world_loop.extend([w.x, w.y, w.z]);
        }
        let world_center = pose.apply_point(local_center);
        let world_radius = local_radius * pose.similarity_scale().unwrap();

        let sub_face = scene
            .split_face_inner_with_curve_in_instance(
                inst,
                member,
                top,
                &world_loop,
                &[world_center.x, world_center.y, world_center.z],
                world_radius,
            )
            .expect("a mirrored (still uniform-scale) pose must map the curve claim, not refuse");
        assert!(scene.face_boundary(member, sub_face).unwrap().len() >= 24);
    }

    /// A NON-uniformly-scaled instance has no single local-frame radius for
    /// a world-space scalar length: unlike a point (which always maps
    /// unambiguously under any invertible affine — the loop-only sibling
    /// `split_face_inner_in_instance` never needs to refuse), the curve
    /// claim's `radius` must refuse `AmbiguousInstanceScale` rather than
    /// guess an axis, matching `Document::map_world_distance_through_pose`'s
    /// rule for every other typed world-space scalar.
    #[test]
    fn split_face_inner_with_curve_in_instance_refuses_a_non_uniformly_scaled_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        // Non-uniform: ×2 in X, ×1 in Y/Z.
        let pose = kernel::Transform::scale(kernel::Vec3::new(2.0, 1.0, 1.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(5.0, 2.0, 0.0)),
        );
        assert!(
            pose.similarity_scale().is_none(),
            "sanity: this pose is non-uniform"
        );
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        let world_loop = [
            pose.apply_point(Point3::new(0.2, 0.2, 1.0)),
            pose.apply_point(Point3::new(0.8, 0.2, 1.0)),
            pose.apply_point(Point3::new(0.8, 0.8, 1.0)),
            pose.apply_point(Point3::new(0.2, 0.8, 1.0)),
        ]
        .iter()
        .flat_map(|p| [p.x, p.y, p.z])
        .collect::<Vec<f64>>();

        // The loop-only surface still succeeds — a point always maps
        // unambiguously, regardless of scale uniformity.
        assert!(
            scene
                .split_face_inner_in_instance(inst, member, top, &world_loop)
                .is_ok(),
            "a plain loop imprint never needs the scale to be uniform"
        );

        // But the curve-carrying surface must refuse: its radius has no
        // single local-frame equivalent under a non-uniform scale.
        let world_center = pose.apply_point(Point3::new(0.5, 0.5, 1.0));
        let err = scene
            .split_face_inner_with_curve_in_instance(
                inst,
                member,
                top,
                &world_loop,
                &[world_center.x, world_center.y, world_center.z],
                0.3,
            )
            .expect_err("a non-uniform scale must refuse the curve claim, not guess an axis");
        assert!(
            format!("{err:?}").contains("AmbiguousInstanceScale"),
            "got {err:?}"
        );
    }

    /// Delta-review fix on component-edit-parity.md phase A2:
    /// `push_pull_in_component`'s `distance` is a WORLD-space length — the
    /// ghost preview sweeps the world drag distance — so it must map
    /// through the instance's pose exactly like
    /// `extrude_region_in_instance`'s `distance`, not commit raw (the two
    /// previously disagreed on a scaled instance: the ghost showed one
    /// height, the commit landed another). A uniformly 2x-scaled instance:
    /// pushing the top face by a WORLD 4.0 must move the DEFINITION-local
    /// face by 2.0 (world effect stays 4.0), not 4.0 raw. This assertion
    /// fails against the pre-fix behavior (local face lands at z=5.0).
    #[test]
    fn push_pull_in_component_maps_world_distance_through_a_uniformly_scaled_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap(); // 1x1x1 box, top face at local z=1
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        // Uniform 2x scale — isolates the scale mapping without a rotation.
        let pose = kernel::Transform::scale(kernel::Vec3::new(2.0, 2.0, 2.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(5.0, 2.0, 0.0)),
        );
        assert!(
            pose.similarity_scale().is_some(),
            "sanity: this pose IS uniform"
        );
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        scene
            .push_pull_in_component(inst, member, top, 4.0)
            .expect("a uniformly-scaled instance must map, not refuse");

        let object = scene.doc.object(object_id(member)).unwrap();
        let max_z = object
            .vertices()
            .values()
            .map(|v| v.position.z)
            .fold(f64::MIN, f64::max);
        assert!(
            (max_z - 3.0).abs() < 1e-9,
            "world distance 4.0 through a 2x-scaled pose must move the definition-local \
             face by 2.0 (to local z=3.0, from the box's top at z=1.0) — got local z={max_z}"
        );
    }

    /// The typed refusal counterpart: a non-uniformly-scaled instance has no
    /// single local-frame equivalent for a world-space scalar distance —
    /// same rule `extrude_region_in_instance` and every other `_in_instance`
    /// typed scalar already enforce
    /// (`Document::map_world_distance_through_pose`, reused here through
    /// `Document::map_instance_world_distance`).
    #[test]
    fn push_pull_in_component_refuses_a_non_uniformly_scaled_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        // Non-uniform: ×2 in X, ×1 in Y/Z.
        let pose = kernel::Transform::scale(kernel::Vec3::new(2.0, 1.0, 1.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(5.0, 2.0, 0.0)),
        );
        assert!(
            pose.similarity_scale().is_none(),
            "sanity: this pose is non-uniform"
        );
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();
        let member = scene.component_member_objects(comp)[0];
        let top = wasm_face_matching(&scene, member, kernel::Vec3::new(0.0, 0.0, 1.0));

        // `PushPullJs` (the `Ok` payload) has no `Debug` impl, so
        // `expect_err`/`unwrap_err` (which require `T: Debug` to format a
        // panic message on the non-error branch) don't fit — match instead.
        let err = match scene.push_pull_in_component(inst, member, top, 4.0) {
            Err(e) => e,
            Ok(_) => panic!("a non-uniform scale must refuse rather than guess an axis"),
        };
        assert!(
            format!("{err:?}").contains("AmbiguousInstanceScale"),
            "got {err:?}"
        );
    }

    /// `pick_sketch_region` walks only `Document::sketch_ids()` — a
    /// definition-owned sketch's region is invisible to it no matter how
    /// squarely the ray lands, exactly like `sketch_edge_endpoints` refusing
    /// a def-owned sketch (K1's boundary). `pick_sketch_region_in_instance`
    /// exists precisely because of this: without it, a plane-mode region
    /// drawn inside a component's own definition could never be found by a
    /// real click for push/pull's region-extrude or Follow Me's sketch-
    /// region profile pick.
    #[test]
    fn pick_sketch_region_is_blind_to_a_def_owned_sketch_but_the_in_instance_sibling_finds_it() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();

        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        for (ax, ay, bx, by) in [
            (2.0, 2.0, 2.5, 2.0),
            (2.5, 2.0, 2.5, 2.5),
            (2.5, 2.5, 2.0, 2.5),
            (2.0, 2.5, 2.0, 2.0),
        ] {
            scene
                .sketch_add_segment(def_sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }

        // Straight down through the region's centre — identity-posed
        // instance, so world and local coordinates coincide.
        assert!(
            scene
                .pick_sketch_region(2.25, 2.25, 5.0, 0.0, 0.0, -1.0)
                .is_none(),
            "pick_sketch_region must not see a definition-owned region"
        );
        let found = scene
            .pick_sketch_region_in_instance(inst, 2.25, 2.25, 5.0, 0.0, 0.0, -1.0)
            .expect("pick_sketch_region_in_instance must find the def-owned region");
        assert_eq!(found.sketch(), def_sketch);

        // A stale/unknown instance is a miss, never a throw.
        assert!(
            scene
                .pick_sketch_region_in_instance(999_999, 2.25, 2.25, 5.0, 0.0, 0.0, -1.0)
                .is_none()
        );
    }

    /// Same pick, through a genuinely non-identity (translated) instance
    /// pose — proves the ray is actually mapped through pose⁻¹, not just
    /// coincidentally correct under an identity placement.
    #[test]
    fn pick_sketch_region_in_instance_maps_the_ray_through_a_translated_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();
        let affine = [
            1.0, 0.0, 0.0, 5.0, //
            0.0, 1.0, 0.0, 2.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let inst = scene.place_instance(comp, &affine).unwrap();

        // A def-owned sketch region at LOCAL (0.2,0.2)-(0.8,0.8) — WORLD
        // (5.2,2.2)-(5.8,2.8) through the translated pose.
        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        for (ax, ay, bx, by) in [
            (0.2, 0.2, 0.8, 0.2),
            (0.8, 0.2, 0.8, 0.8),
            (0.8, 0.8, 0.2, 0.8),
            (0.2, 0.8, 0.2, 0.2),
        ] {
            scene
                .sketch_add_segment(def_sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }

        // A ray straight down through the region's WORLD position must find
        // it; the SAME ray through the region's LOCAL (unmapped) position
        // must miss — proof the pose is actually applied, not skipped.
        let found = scene
            .pick_sketch_region_in_instance(inst, 5.5, 2.5, 5.0, 0.0, 0.0, -1.0)
            .expect("world ray through the posed region must hit");
        assert_eq!(found.sketch(), def_sketch);
        assert!(
            scene
                .pick_sketch_region_in_instance(inst, 0.5, 0.5, 5.0, 0.0, 0.0, -1.0)
                .is_none(),
            "the region's LOCAL position is not where it sits in WORLD space"
        );
    }

    /// Same pick, through a MIRRORED pose (negative determinant) — closes
    /// the coverage gap this finding traced: every prior `_in_instance` ray/
    /// point test used only translation/identity poses, the same class of
    /// gap that let the curve-claim mapping bug go unnoticed.
    #[test]
    fn pick_sketch_region_in_instance_maps_the_ray_through_a_mirrored_pose() {
        let mut scene = Scene::new();
        let (s, r) = ground_unit_square(&mut scene);
        let o = scene.extrude_region(s, r, 1.0).unwrap();
        let inst = scene.make_component(&[0], &[o]).unwrap();
        let comp = scene.instance_def(inst).unwrap();
        scene.delete_node(2, inst).unwrap();

        let pose = kernel::Transform::scale(kernel::Vec3::new(-1.0, 1.0, 1.0)).then(
            &kernel::Transform::translation(kernel::Vec3::new(7.0, 4.0, 0.0)),
        );
        assert!(pose.determinant() < 0.0, "sanity: this pose mirrors");
        let inst = scene.place_instance(comp, &pose.to_affine()).unwrap();

        let def_sketch = scene
            .begin_sketch_on_plane_in_instance(inst, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0)
            .unwrap();
        for (ax, ay, bx, by) in [
            (0.2, 0.2, 0.8, 0.2),
            (0.8, 0.2, 0.8, 0.8),
            (0.8, 0.8, 0.2, 0.8),
            (0.2, 0.8, 0.2, 0.2),
        ] {
            scene
                .sketch_add_segment(def_sketch, ax, ay, 0.0, bx, by, 0.0)
                .unwrap();
        }

        let world_center = pose.apply_point(Point3::new(0.5, 0.5, 0.0));
        let found = scene
            .pick_sketch_region_in_instance(
                inst,
                world_center.x,
                world_center.y,
                5.0,
                0.0,
                0.0,
                -1.0,
            )
            .expect("world ray through the mirrored region must hit");
        assert_eq!(found.sketch(), def_sketch);
        assert!(
            scene
                .pick_sketch_region_in_instance(inst, 0.5, 0.5, 5.0, 0.0, 0.0, -1.0)
                .is_none(),
            "the region's LOCAL position is not where it sits under the mirrored pose"
        );
    }

    // -------------------------------------------------------------------
    // : guides as inference snap targets
    // -------------------------------------------------------------------

    /// After `add_guide_line`, a ray crossing it snaps with kind `"on-guide"`.
    #[test]
    fn add_guide_line_is_snappable_as_on_guide() {
        let mut scene = Scene::new();
        scene.add_guide_line(5.0, 0.0, 0.0, 0.0, 1.0, 0.0).unwrap();

        let snap = scene
            .snap(
                5.0, 5.0, 3.0, // ray origin
                0.0, 0.0, -1.0, // ray direction (-Z)
                0.05, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("ray crosses the guide line");
        assert_eq!(snap.kind(), "on-guide");
    }

    /// After `delete_all_guides`, the same ray no longer snaps.
    #[test]
    fn delete_all_guides_unregisters_them_from_inference() {
        let mut scene = Scene::new();
        scene.add_guide_line(5.0, 0.0, 0.0, 0.0, 1.0, 0.0).unwrap();
        scene.delete_all_guides().unwrap();

        let snap = scene
            .snap(
                5.0, 5.0, 3.0, 0.0, 0.0, -1.0, 0.05, None, None, None, None, None, None, None,
            )
            .unwrap();
        assert!(
            snap.is_none(),
            "deleted guide must no longer snap, got {:?}",
            snap.map(|s| s.kind())
        );
    }

    /// Undoing a guide creation unregisters it from inference; redoing
    /// re-registers it. Mirrors `extrude_then_scene_undo_redo_hides_and_restores_the_object`.
    #[test]
    fn guide_creation_undo_redo_round_trips_through_inference() {
        let mut scene = Scene::new();
        scene.add_guide_line(5.0, 0.0, 0.0, 0.0, 1.0, 0.0).unwrap();

        let ray = (5.0, 5.0, 3.0, 0.0, 0.0, -1.0, 0.05);
        let snaps = |scene: &Scene| {
            scene
                .snap(
                    ray.0, ray.1, ray.2, ray.3, ray.4, ray.5, ray.6, None, None, None, None, None,
                    None, None,
                )
                .unwrap()
        };

        assert_eq!(
            snaps(&scene).map(|s| s.kind()),
            Some("on-guide".to_string())
        );

        scene.scene_undo().unwrap();
        assert!(
            snaps(&scene).is_none(),
            "undone guide creation must unregister from inference"
        );

        scene.scene_redo().unwrap();
        assert_eq!(
            snaps(&scene).map(|s| s.kind()),
            Some("on-guide".to_string()),
            "redone guide creation must re-register with inference"
        );
    }

    /// A guide point registers as an `"endpoint"` snap (Endpoint-tier, like a
    /// real vertex) through the same wasm `snap` surface.
    #[test]
    fn add_guide_point_is_snappable_as_endpoint() {
        let mut scene = Scene::new();
        scene.add_guide_point(2.0, 3.0, 0.0).unwrap();

        let snap = scene
            .snap(
                2.0, 3.0, 5.0, 0.0, 0.0, -1.0, 0.05, None, None, None, None, None, None, None,
            )
            .unwrap()
            .expect("ray points straight at the guide point");
        assert_eq!(snap.kind(), "endpoint");
    }

    /// The `cylinder` flag on `Scene::snap` is the ONLY thing that crosses
    /// the boundary for parallel-projection pick tolerance (docs/design/
    /// camera.md §1) — the cone/cylinder math itself stays in `inference`,
    /// and which projection is active stays in the app (DEVELOPMENT.md rule
    /// 1). This pins that the flag is actually wired: a guide point 1 m off
    /// the ray axis at 5 m depth is OUTSIDE a 0.05 rad cone (angle ≈ 0.197
    /// rad) but INSIDE a 1.5 m cylinder — same ray, same call, only
    /// `cylinder` and the (now meters, not radians) `aperture` differ.
    #[test]
    fn the_cylinder_flag_selects_a_constant_world_radius_tolerance() {
        let mut scene = Scene::new();
        scene.add_guide_point(2.0, 4.0, 0.0).unwrap();

        let cone_miss = scene
            .snap(
                2.0, 3.0, 5.0, 0.0, 0.0, -1.0, 0.05, None, None, None, None, None, None, None,
            )
            .unwrap();
        assert!(
            cone_miss.is_none(),
            "1 m off-axis at 5 m depth is well outside a 0.05 rad cone"
        );

        let cylinder_hit = scene
            .snap(
                2.0,
                3.0,
                5.0,
                0.0,
                0.0,
                -1.0,
                1.5,
                None,
                None,
                None,
                None,
                Some(true),
                None,
                None,
            )
            .unwrap()
            .expect("1 m off-axis is inside a 1.5 m cylinder, regardless of depth");
        assert_eq!(cylinder_hit.kind(), "endpoint");
    }

    // ---------------------------------------------------------------- annotations

    /// A free-floating (no node) linear dimension round-trips its anchors,
    /// offset, plane, and query surface; `text_override` starts unset.
    #[test]
    fn add_linear_dimension_round_trips_and_queries() {
        let mut scene = Scene::new();
        let id = scene
            .add_linear_dimension(
                -1,
                0,
                &[0.0, 0.0, 0.0],
                -1,
                0,
                &[3.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                None,
            )
            .unwrap();

        assert_eq!(scene.annotation_kind(id).as_deref(), Some("linear"));
        assert_eq!(scene.annotation_detached(id), Some(false));
        assert_eq!(scene.annotation_anchor_node_kind(id, 0), Some(-1));
        assert_eq!(
            scene.annotation_anchor_point(id, 0),
            Some(vec![0.0, 0.0, 0.0])
        );
        assert_eq!(
            scene.annotation_anchor_point(id, 1),
            Some(vec![3.0, 0.0, 0.0])
        );
        assert_eq!(scene.annotation_offset(id), Some(vec![0.0, 1.0, 0.0]));
        assert_eq!(scene.annotation_text_override(id), None);
        assert!(scene.annotation_ids().contains(&id));

        // Setting a text_override, then clearing it, round-trips through
        // update_linear_dimension — the SketchUp `<>` re-pick semantics.
        scene
            .update_linear_dimension(
                id,
                -1,
                0,
                &[0.0, 0.0, 0.0],
                -1,
                0,
                &[3.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                Some("3m even".to_string()),
            )
            .unwrap();
        assert_eq!(
            scene.annotation_text_override(id).as_deref(),
            Some("3m even")
        );

        scene.delete_annotation(id).unwrap();
        assert_eq!(
            scene.annotation_kind(id),
            None,
            "deleted annotation is hidden"
        );
    }

    /// A radial dimension captures its analytic circle and presents as
    /// radius/diameter via `kind`.
    #[test]
    fn add_radial_dimension_round_trips_curve_and_kind() {
        let mut scene = Scene::new();
        let id = scene
            .add_radial_dimension(
                -1,
                0,
                &[5.0, 0.0, 0.0],
                "radius",
                &[0.0, 0.0, 0.0],
                5.0,
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                &[1.0, 0.0, 0.0],
                None,
            )
            .unwrap();

        assert_eq!(scene.annotation_kind(id).as_deref(), Some("radial"));
        assert_eq!(scene.annotation_radial_kind(id).as_deref(), Some("radius"));
        assert_eq!(
            scene.annotation_curve(id),
            Some(vec![0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(scene.annotation_leader_dir(id), Some(vec![1.0, 0.0, 0.0]));

        // Tab-toggle to diameter is an update with the SAME curve.
        scene
            .update_radial_dimension(
                id,
                -1,
                0,
                &[5.0, 0.0, 0.0],
                "diameter",
                &[0.0, 0.0, 0.0],
                5.0,
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                &[1.0, 0.0, 0.0],
                None,
            )
            .unwrap();
        assert_eq!(
            scene.annotation_radial_kind(id).as_deref(),
            Some("diameter")
        );
    }

    /// Leader text stores its content and is distinguishable from a
    /// dimension's `text_override` field.
    #[test]
    fn add_leader_text_round_trips_content() {
        let mut scene = Scene::new();
        let id = scene
            .add_leader_text(
                -1,
                0,
                &[1.0, 2.0, 3.0],
                &[0.5, 0.5, 0.0],
                "Note".to_string(),
            )
            .unwrap();
        assert_eq!(scene.annotation_kind(id).as_deref(), Some("leader"));
        assert_eq!(scene.annotation_text(id).as_deref(), Some("Note"));
        assert_eq!(scene.annotation_text_override(id), None);

        scene
            .update_leader_text(
                id,
                -1,
                0,
                &[1.0, 2.0, 3.0],
                &[0.5, 0.5, 0.0],
                "Edited".to_string(),
            )
            .unwrap();
        assert_eq!(scene.annotation_text(id).as_deref(), Some("Edited"));
    }

    /// An anchor naming a live object round-trips its node kind/id, and
    /// deleting the object detaches the annotation (rather than silently
    /// keeping a stale point) — the re-anchoring contract
    /// (docs/design/dimensions-text.md).
    #[test]
    fn linear_dimension_anchored_to_object_detaches_on_delete() {
        let mut scene = Scene::new();
        let sketch = scene.begin_ground_sketch();
        scene
            .sketch_add_segment(sketch, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();
        scene
            .sketch_add_segment(sketch, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0)
            .unwrap();
        scene
            .sketch_add_segment(sketch, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0)
            .unwrap();
        let report = scene
            .sketch_add_segment(sketch, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0)
            .unwrap();
        let region = report.inner.regions_created[0].data().as_ffi();
        let object = scene.extrude_region(sketch, region, 1.0).unwrap();

        let id = scene
            .add_linear_dimension(
                0,
                object,
                &[0.0, 0.0, 0.0],
                -1,
                0,
                &[1.0, 0.0, 0.0],
                &[0.0, -1.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                None,
            )
            .unwrap();
        assert_eq!(scene.annotation_anchor_node_kind(id, 0), Some(0));
        assert_eq!(scene.annotation_anchor_node_id(id, 0), Some(object));
        assert_eq!(scene.annotation_detached(id), Some(false));

        scene.delete_node(0, object).unwrap();
        assert_eq!(
            scene.annotation_detached(id),
            Some(true),
            "deleting the anchored object detaches the dimension"
        );
    }

    /// Recording + replaying a session that creates, updates, and deletes
    /// annotations reproduces the golden `state_hash` — the same regression
    /// guarantee every other mutating call already carries. Covers every
    /// annotation-mutating call (including `update_linear_dimension` /
    /// `update_radial_dimension`, not just `update_leader_text`) AND every
    /// anchor shape: free (`node: None`, the original coverage), a live
    /// OBJECT node, and a live INSTANCE node — `instance`'s pose is MIRRORED
    /// (negative determinant) before the LINEAR dimension anchors to it, so
    /// this shape covers anchoring onto an already-transformed instance, not
    /// a reanchor triggered during the session: `transform_instance` here
    /// precedes `add_linear_dimension`, and by `Transform::similarity_scale`'s
    /// definition a pure mirror IS a similarity anyway (its columns stay
    /// orthonormal — only the sign flips), so `reanchor_touched`'s
    /// non-similarity branch never runs for it.
    ///
    /// The genuine non-similarity case — `Document::reanchor_touched`'s doc
    /// comment singles out a captured `RadialDimension` circle possibly not
    /// surviving the map — needs a transform that is NOT a similarity
    /// (squash/shear, not mirror) applied AFTER the annotation anchors, so
    /// `reanchor_touched` actually runs mid-session. `instance2`/
    /// `radial_on_instance2` below cover exactly that: anchored while
    /// `instance2` is still identity-posed, then squashed non-uniformly,
    /// flipping `detached` to `true` — asserted immediately, and again after
    /// replay, so the flag itself (not just the aggregate hash) is proven to
    /// round-trip.
    #[test]
    fn record_then_replay_reproduces_annotation_session() {
        let mut scene = Scene::new();
        scene.start_recording();

        // A plain solid (for the object-anchored annotation) and a second
        // solid folded into a mirrored component instance (for the
        // instance-anchored one).
        let (s1, r1) = ground_unit_square(&mut scene);
        let object = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square_at(&mut scene, 3.0, 0.0);
        let base = scene.extrude_region(s2, r2, 1.0).unwrap();
        let instance = scene.make_component(&[0], &[base]).unwrap();
        // Reflect across X (determinant < 0) — `transform_instance` allows a
        // mirrored pose (only explode/make_unique refuse one).
        const MIRROR_X: [f64; 12] = [-1.0, 0.0, 0.0, 6.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(instance, &MIRROR_X).unwrap();

        let id = scene
            .add_linear_dimension(
                -1,
                0,
                &[0.0, 0.0, 0.0],
                -1,
                0,
                &[2.0, 0.0, 0.0],
                &[0.0, 1.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                None,
            )
            .unwrap();
        let radial = scene
            .add_radial_dimension(
                -1,
                0,
                &[5.0, 0.0, 0.0],
                "radius",
                &[0.0, 0.0, 0.0],
                5.0,
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                &[1.0, 0.0, 0.0],
                None,
            )
            .unwrap();
        let leader = scene
            .add_leader_text(
                -1,
                0,
                &[1.0, 1.0, 0.0],
                &[0.5, 0.5, 0.0],
                "Note".to_string(),
            )
            .unwrap();
        scene
            .update_leader_text(
                leader,
                -1,
                0,
                &[1.0, 1.0, 0.0],
                &[0.5, 0.5, 0.0],
                "Edited".to_string(),
            )
            .unwrap();

        // Node-anchored annotations: one on the live object, one on the
        // mirrored instance.
        let object_anchored = scene
            .add_leader_text(
                0,
                object,
                &[0.5, 0.5, 1.0],
                &[0.2, 0.2, 0.0],
                "On object".to_string(),
            )
            .unwrap();
        let instance_anchored = scene
            .add_linear_dimension(
                2,
                instance,
                &[3.0, 0.0, 0.0],
                -1,
                0,
                &[4.0, 0.0, 0.0],
                &[0.0, -1.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                None,
            )
            .unwrap();
        assert_eq!(scene.annotation_detached(object_anchored), Some(false));
        assert_eq!(scene.annotation_detached(instance_anchored), Some(false));

        // The genuine non-similarity reanchor case: a THIRD solid, folded
        // into its own component instance, anchors a RADIAL dimension while
        // `instance2` is still identity-posed — unlike `instance` above
        // (already mirrored before anything anchored to it), so the
        // transform below is what actually runs `reanchor_touched` during
        // this recorded session.
        let (s3, r3) = ground_unit_square_at(&mut scene, 6.0, 0.0);
        let base2 = scene.extrude_region(s3, r3, 1.0).unwrap();
        let instance2 = scene.make_component(&[0], &[base2]).unwrap();
        let radial_on_instance2 = scene
            .add_radial_dimension(
                2,
                instance2,
                &[6.5, 0.5, 1.0],
                "radius",
                &[6.5, 0.5, 1.0],
                0.5,
                &[6.5, 0.5, 1.0, 0.0, 0.0, 1.0],
                &[1.0, 0.0, 0.0],
                None,
            )
            .unwrap();
        assert_eq!(scene.annotation_detached(radial_on_instance2), Some(false));

        // Scale x by 2, leave y/z alone: `similarity_scale` requires every
        // column of the linear part to share one magnitude, which this
        // violates (unlike MIRROR_X's orthonormal-but-flipped columns
        // above) — a genuine non-similarity map.
        const SQUASH_X: [f64; 12] = [2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scene.transform_instance(instance2, &SQUASH_X).unwrap();
        assert_eq!(
            scene.annotation_detached(radial_on_instance2),
            Some(true),
            "a non-similarity transform on the anchored instance detaches the radial dimension"
        );

        // `update_linear_dimension`/`update_radial_dimension` themselves —
        // the original session only ever exercised `update_leader_text`.
        scene
            .update_linear_dimension(
                id,
                -1,
                0,
                &[0.0, 0.0, 0.0],
                -1,
                0,
                &[2.0, 0.0, 0.0],
                &[0.0, 2.0, 0.0],
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                Some("2m even".to_string()),
            )
            .unwrap();
        scene
            .update_radial_dimension(
                radial,
                -1,
                0,
                &[5.0, 0.0, 0.0],
                "diameter",
                &[0.0, 0.0, 0.0],
                5.0,
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                &[1.0, 0.0, 0.0],
                None,
            )
            .unwrap();

        scene.delete_annotation(id).unwrap();

        let recording_json = scene.take_recording();
        let golden = scene.state_hash();

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&recording_json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying an annotation session reproduces the golden state_hash"
        );
        // The genuine non-similarity case, checked directly rather than only
        // through the aggregate hash: `radial_on_instance2`'s `detached: true`
        // (set mid-session by the squash above) round-trips through
        // record/replay, not just the annotation's other fields.
        assert_eq!(
            replayed.annotation_detached(radial_on_instance2),
            Some(true),
            "the non-similarity detach round-trips through record/replay"
        );
    }

    // ---------------------------------------------------------- begin_sketch_on_plane

    /// `begin_sketch_on_plane` creates a live sketch whose `sketch_plane`
    /// round-trips the requested point/normalized normal. `(2,0,0)` with
    /// normal `(1,0,0)` is deliberately its OWN perpendicular foot (the
    /// point `Plane::point()` returns), so the round-trip is an exact
    /// component match rather than merely "some point on the plane".
    #[test]
    fn begin_sketch_on_plane_round_trips_point_and_normal() {
        let mut scene = Scene::new();
        let sketch = scene
            .begin_sketch_on_plane(2.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();

        let plane = scene
            .sketch_plane(sketch)
            .expect("freshly minted sketch is live");
        assert_eq!(plane, vec![2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

        // A non-unit normal is normalized, not rejected — the point is
        // already its own perpendicular foot here too (3 = 1*3 along the
        // normalized (1,0,0) direction).
        let sketch2 = scene
            .begin_sketch_on_plane(3.0, 0.0, 0.0, 2.0, 0.0, 0.0)
            .unwrap();
        let plane2 = scene.sketch_plane(sketch2).unwrap();
        assert_eq!(
            plane2,
            vec![3.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "normal normalized to unit length"
        );
    }

    /// Drawing a closed region on a vertical plane (normal +X through
    /// `(2,0,0)`) via `sketch_add_segment` in WORLD coordinates, then
    /// `extrude_region`, yields a solid — the kernel's plane-generic sketch
    /// math (the sketch-planes design §2) exercised through the new
    /// entry point rather than only through `begin_ground_sketch`.
    #[test]
    fn begin_sketch_on_plane_draws_and_extrudes_a_vertical_solid() {
        let mut scene = Scene::new();
        let sketch = scene
            .begin_sketch_on_plane(2.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();

        let corners = [
            (0.0, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ];
        let mut region = None;
        for (ay, az, by, bz) in corners {
            let report = scene
                .sketch_add_segment(sketch, 2.0, ay, az, 2.0, by, bz)
                .unwrap();
            if let Some(&r) = report.inner.regions_created.first() {
                region = Some(r.data().as_ffi());
            }
        }
        let region = region.expect("closing the loop creates a region");

        let object = scene.extrude_region(sketch, region, 1.0).unwrap();
        assert!(
            scene.object_ids().contains(&object),
            "extrusion produced a live solid"
        );
    }

    /// A degenerate (zero) normal is a typed `DegenerateVector` refusal
    /// (DEVELOPMENT.md rule 4 — no silent repair, no guessed direction) and
    /// creates no sketch.
    #[test]
    fn begin_sketch_on_plane_rejects_degenerate_normal() {
        let mut scene = Scene::new();
        let err = scene
            .begin_sketch_on_plane(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            .unwrap_err();
        assert!(err.0.starts_with("DegenerateVector"), "got {}", err.0);
        assert!(
            scene.sketch_ids().is_empty(),
            "the refused call created nothing"
        );
    }

    /// A recording containing `begin_sketch_on_plane` replays: the same
    /// pattern as `copy_and_array_copy_record_and_replay`, exercised on the
    /// one new public wasm-api call this phase adds.
    #[test]
    fn begin_sketch_on_plane_records_and_replays() {
        recording::reset();

        let mut scene = Scene::new();
        scene.start_recording();

        let sketch = scene
            .begin_sketch_on_plane(2.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .unwrap();
        let corners = [
            (0.0, 0.0, 1.0, 0.0),
            (1.0, 0.0, 1.0, 1.0),
            (1.0, 1.0, 0.0, 1.0),
            (0.0, 1.0, 0.0, 0.0),
        ];
        let mut region = None;
        for (ay, az, by, bz) in corners {
            let report = scene
                .sketch_add_segment(sketch, 2.0, ay, az, 2.0, by, bz)
                .unwrap();
            if let Some(&r) = report.inner.regions_created.first() {
                region = Some(r.data().as_ffi());
            }
        }
        scene.extrude_region(sketch, region.unwrap(), 1.0).unwrap();

        scene.stop_recording();
        let golden = scene.state_hash();
        let json = scene.take_recording();
        assert!(json.contains("\"method\":\"begin_sketch_on_plane\""));

        let mut replayed = Scene::new();
        let final_hash = replayed.replay(&json).unwrap();
        assert_eq!(
            final_hash, golden,
            "replaying a begin_sketch_on_plane session reproduces the golden state_hash"
        );
        assert_eq!(replayed.save(), scene.save(), "byte-identical document");
    }

    // ------------------------------------------------------------ live API

    fn hello_and_attach(scene: &mut Scene, conn: u32) {
        assert!(
            scene
                .api_dispatch(
                    conn,
                    r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
                )
                .unwrap()
                .contains("\"profile\":\"app\"")
        );
        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
            )
            .unwrap();
        assert!(reply.contains("\"result\""), "attach succeeds: {reply}");
    }

    #[test]
    fn api_dispatch_runs_a_mutating_command_and_it_shows_up_live() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        hello_and_attach(&mut scene, conn);

        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":2,"method":"hew.sketch.draw_rect",
                    "params":{"plane":{"ground":true},"corner_a":[0.0,0.0,0.0],"corner_b":[1.0,1.0,0.0]}}"#,
            )
            .expect("a request always gets a reply");
        assert!(reply.contains("\"result\""), "draw_rect succeeds: {reply}");
        assert_eq!(
            scene.sketch_ids().len(),
            1,
            "the sketch dispatched over the live API is the SAME document the viewport reads"
        );
    }

    #[test]
    fn api_dispatch_malformed_json_answers_a_parse_error_with_null_id() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        let reply = scene.api_dispatch(conn, "not json").unwrap();
        assert!(reply.contains("\"id\":null"), "got {reply}");
        assert!(reply.contains("-32700"), "PARSE_ERROR code: {reply}");
    }

    #[test]
    fn api_dispatch_unknown_connection_answers_typed_instead_of_panicking() {
        let mut scene = Scene::new();
        let reply = scene
            .api_dispatch(
                999,
                r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
            )
            .unwrap();
        assert!(reply.contains("-32003"), "INTERNAL_FAULT code: {reply}");
    }

    #[test]
    fn api_dispatch_notification_is_dropped_with_no_reply() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        hello_and_attach(&mut scene, conn);
        let reply = scene.api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","method":"hew.sketch.draw_rect",
                "params":{"plane":{"ground":true},"corner_a":[0.0,0.0,0.0],"corner_b":[1.0,1.0,0.0]}}"#,
        );
        assert!(reply.is_none(), "a notification (no id) gets no reply");
        assert!(
            scene.sketch_ids().is_empty(),
            "a dropped notification must not execute (§4.1)"
        );
    }

    #[test]
    fn api_connection_close_forgets_the_connection() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        hello_and_attach(&mut scene, conn);
        scene.api_connection_close(conn);
        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":5,"method":"hew.query.scene","params":{}}"#,
            )
            .unwrap();
        assert!(
            reply.contains("-32003"),
            "a closed connection is unknown again: {reply}"
        );
    }

    /// `hew.doc.new` is host-served and `LiveHost` refuses it typed
    /// (crates/wasm-api/src/live.rs) — a remote connection must not be
    /// able to wipe the user's live document out from under them.
    #[test]
    fn api_dispatch_refuses_doc_new_and_leaves_the_document_untouched() {
        let mut scene = Scene::new();
        let conn = scene.api_connection_open();
        hello_and_attach(&mut scene, conn);
        scene.begin_ground_sketch();

        let reply = scene
            .api_dispatch(
                conn,
                r#"{"jsonrpc":"2.0","id":2,"method":"hew.doc.new","params":{}}"#,
            )
            .unwrap();
        assert!(reply.contains("host_capability_missing"), "got {reply}");
        assert_eq!(
            scene.sketch_ids().len(),
            1,
            "the refused hew.doc.new left the live document untouched"
        );
    }
}
