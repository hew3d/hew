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

mod log;
mod recording;

use dae_import::ImageMap;
use inference::{
    Axis, ElementRef, InferenceScene, PickRay, SketchRegionFace, SnapKind, SnapLock, SnapQuery,
};
use js_sys::{Object as JsObject, Reflect, Uint8Array};
use kernel::{
    BooleanOp, ComponentId, DocChange, Document, DocumentError, EdgeId, FaceId, GroupId, Guide,
    GuideId, ImageFormat, InstanceId, KernelOp, KernelOpError, KernelOpReport, LoadError, Material,
    MaterialId, NodeId, Object, ObjectId, Plane, Point3, Rgba8, SketchEdgeId, SketchId,
    SketchRegionId, Texture, Transform, WatertightState,
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

/// Converts a `u64` handle to a [`MaterialId`], or `None` if the sentinel
/// value `u64::MAX` is given (meaning "default / unpaint").
fn material_id_opt(handle: u64) -> Option<MaterialId> {
    if handle == u64::MAX {
        None
    } else {
        Some(MaterialId::from(KeyData::from_ffi(handle)))
    }
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
}

/// A picked sketch edge: the owning sketch plus the edge itself (see
/// `Scene::pick_sketch_edge`).
#[wasm_bindgen]
pub struct SketchEdgePickJs {
    sketch: u64,
    edge: u64,
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

    /// "vertex" | "edge" | "face" | "sketch-edge" | "sketch-region" for
    /// interpreting `element` / `sketch_region`.
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
    }

    /// The owning sketch handle when this snap derives from a committed sketch
    /// EDGE (`element_kind` == "sketch-edge", `element` is the edge) or a sketch
    /// REGION fill (`element_kind` == "sketch-region", `sketch_region` is the
    /// region); `undefined` otherwise.
    pub fn sketch(&self) -> Option<u64> {
        self.snap
            .sketch_source
            .map(|(s, _)| s.data().as_ffi())
            .or_else(|| {
                self.snap
                    .sketch_region_source
                    .map(|(s, _)| s.data().as_ffi())
            })
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
            self.register_sketch(sid);
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
        let (Some(def), Some(pose)) = (self.doc.instance_def(iid), self.doc.instance_pose(iid))
        else {
            return;
        };
        let Some(members) = self.doc.def_members(def) else {
            return;
        };
        for m in members {
            if !self.inference.has_def_member(m) {
                let Some(object) = self.doc.object(m) else {
                    continue;
                };
                self.inference.set_def_member(m, object);
            }
            self.inference.add_placement(iid, m, &pose);
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
        if let Some(segments) = Self::live_sketch_segments(&self.doc, id) {
            self.inference.add_sketch(id, &segments);
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
            self.inference
                .add_sketch_faces(id, &Self::live_sketch_faces(s));
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
    fn live_sketch_segments(
        doc: &Document,
        id: SketchId,
    ) -> Option<Vec<(SketchEdgeId, Point3, Point3)>> {
        let s = doc.sketch(id)?;
        let mut out = Vec::with_capacity(s.edges().len());
        for (eid, edge) in s.edges() {
            let a = s.vertices()[edge.from].position;
            let b = s.vertices()[edge.to].position;
            out.push((eid, a, b));
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
        }
    }

    // ------------------------------------------------------------ sketching

    /// Adds a fresh, empty sketch on the ground plane (M1: the only sketch
    /// surface) and returns its handle. **Additive** — existing sketches are
    /// untouched, so independent coplanar shapes can coexist.
    pub fn begin_ground_sketch(&mut self) -> u64 {
        recording::record(recording::RecordedCall::BeginGroundSketch);
        self.doc.add_sketch(ground_plane()).data().as_ffi()
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

    /// The analytic definition of curve chain `curve` in `sketch` as
    /// `[cx, cy, cz, radius]`, or `undefined` when the chain carries none
    /// (drawn before geometry capture, or a stale handle).
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
        for (_eid, a, b) in segments {
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

    /// Follow Me along a chain of sketch edges (the follow-me design):
    /// sweeps the closed profile `region` of `sketch` along the path the
    /// `path_edges` of `path_sketch` form (a single connected chain, open or
    /// closed, in any order) into a new watertight Object and returns its
    /// handle. The profile region's scaffolding is consumed exactly as
    /// `extrude_region` consumes its outline (undo restores it); the path
    /// sketch is never touched.
    pub fn follow_me_along_edges(
        &mut self,
        sketch: u64,
        region: u64,
        path_sketch: u64,
        path_edges: Vec<u64>,
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
            .follow_me(sketch_id(sketch), region_id, &path)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAlongEdges {
            sketch,
            region,
            path_sketch,
            path_edges,
        });
        Ok(id.data().as_ffi())
    }

    /// Follow Me around a solid face's outer boundary loop (crown molding
    /// around a tabletop): sweeps the closed profile `region` of `sketch`
    /// around the loop into a new watertight Object and returns its handle.
    /// The path solid is untouched — the sweep is a separate Object the
    /// user unions or subtracts explicitly.
    pub fn follow_me_around_face(
        &mut self,
        sketch: u64,
        region: u64,
        path_object: u64,
        path_face: u64,
    ) -> Result<u64, ApiError> {
        let region_id = SketchRegionId::from(KeyData::from_ffi(region));
        let path = kernel::FollowMePath::FaceLoop {
            object: object_id(path_object),
            face: FaceId::from(KeyData::from_ffi(path_face)),
        };
        let (id, change) = self
            .doc
            .follow_me(sketch_id(sketch), region_id, &path)
            .map_err(doc_err)?;
        self.reconcile(&change);
        recording::record(recording::RecordedCall::FollowMeAroundFace {
            sketch,
            region,
            path_object,
            path_face,
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

    /// The member objects of a definition (definition-local geometry), in order.
    /// Fetch each one's mesh via [`Scene::object_mesh`] and draw it at the
    /// instance pose. Empty if the component is stale/hidden.
    pub fn component_member_objects(&self, component: u64) -> Vec<u64> {
        self.doc
            .def_members(component_id(component))
            .unwrap_or_default()
            .iter()
            .map(|o| o.data().as_ffi())
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
    /// seen by every instance of `component` at once. Like [`Scene::push_pull`],
    /// a flat imprinted sub-face auto-routes to wall-generating extrude. Routed
    /// through the kernel's `apply_def_op`, so it cannot touch world objects.
    pub fn push_pull_in_component(
        &mut self,
        component: u64,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<PushPullJs, ApiError> {
        let oid = object_id(object);
        let face_id = FaceId::from(KeyData::from_ffi(face));
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
        let (report, change) = self
            .doc
            .apply_def_op(component_id(component), oid, op)
            .map_err(doc_err)?;
        self.reconcile(&change);
        match report {
            KernelOpReport::PushPull(inner) | KernelOpReport::ExtrudeSubFace(inner) => {
                recording::record(recording::RecordedCall::PushPullInComponent {
                    component,
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
    pub fn sketch_edge_endpoints(&self, sketch: u64, edge: u64) -> Option<Vec<f64>> {
        let sid = sketch_id(sketch);
        let s = self.doc.sketch(sid)?;
        let eid = SketchEdgeId::from(KeyData::from_ffi(edge));
        let e = s.edges().get(eid)?;
        let a = s.vertices()[e.from].position;
        let b = s.vertices()[e.to].position;
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

    // ------------------------------------------------------------ inference

    /// Resolves one snap query. `anchor` is an optional xyz triple;
    /// `lock_axis` is 0/1/2 for X/Y/Z. `constraint_plane`, when present, is a
    /// 6-tuple `[px,py,pz, nx,ny,nz]` (a point on the active drawing plane plus
    /// its normal): only candidates lying on that plane are considered, so
    /// drawing on a face never snaps through the solid to occluded, off-plane
    /// geometry. Returns `undefined` when nothing snaps (tools fall back to
    /// their own plane intersection).
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
            constraint_plane,
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
            .map(|(sid, eid)| SketchEdgePickJs {
                sketch: sid.data().as_ffi(),
                edge: eid.data().as_ffi(),
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
        let mut best: Option<(f64, SketchId, SketchRegionId)> = None;
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
                if best.is_none_or(|(a, _, _)| area < a) {
                    best = Some((area, sid, rid));
                }
            }
        }
        best.map(|(_, s, r)| SketchRegionPickJs {
            sketch: s.data().as_ffi(),
            region: r.data().as_ffi(),
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

    // --------------------------------------------------------- persistence

    /// Serialise the entire document to a `.hew` zip container (HEW_FILE_FORMAT.md).
    /// The returned bytes are a self-contained file — pass them to
    /// [`Scene::load`] to restore the document exactly.
    ///
    /// wasm-bindgen marshals `Vec<u8>` to a JS `Uint8Array`.
    pub fn save(&self) -> Vec<u8> {
        self.doc.save()
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
        recording::without_capture(|| {
            for call in rec.calls {
                match call {
                    BeginGroundSketch => {
                        self.begin_ground_sketch();
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
                    } => {
                        self.follow_me_along_edges(sketch, region, path_sketch, path_edges)?;
                    }
                    FollowMeAroundFace {
                        sketch,
                        region,
                        path_object,
                        path_face,
                    } => {
                        self.follow_me_around_face(sketch, region, path_object, path_face)?;
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
                    TransformInstance { instance, affine } => {
                        self.transform_instance(instance, &affine)?;
                    }
                    ExplodeInstance { instance } => {
                        self.explode_instance(instance)?;
                    }
                    MakeUnique { instance } => {
                        self.make_unique(instance)?;
                    }
                    PushPullInComponent {
                        component,
                        object,
                        face,
                        distance,
                    } => {
                        self.push_pull_in_component(component, object, face, distance)?;
                    }
                    SplitFace { object, face, path } => {
                        self.split_face(object, face, &path)?;
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
                    RemoveNodeTag { kind, id, path } => {
                        self.remove_node_tag(kind, id, path)?;
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
                    Load { bytes } => {
                        self.load_core(&bytes)?;
                    }
                }
            }
            Ok(())
        })?;
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
            .snap(vx, vy, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None)
            .unwrap()
            .expect("loaded sketch vertex snaps");
        assert_eq!(snap.kind(), "endpoint");
        // …and the drawn circle's exact center snaps as Center.
        let snap = loaded
            .snap(1.0, 1.0, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None)
            .unwrap()
            .expect("loaded circle center snaps");
        assert_eq!(snap.kind(), "center");
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
            .snap(cx + 0.08, cy, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None)
            .unwrap()
            .expect("something snaps at the copy center");
        assert_eq!(snap.kind(), "center");
        assert!((snap.x() - (cx + 0.08)).abs() < 1e-12);
        assert!((snap.y() - cy).abs() < 1e-12);
        // And the original center still snaps too.
        let snap0 = scene
            .snap(cx, cy, 3.0, 0.0, 0.0, -1.0, 0.002, None, None, None)
            .unwrap()
            .expect("original center snaps");
        assert_eq!(snap0.kind(), "center");
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
            ray: PickRay {
                origin: Point3::new(0.11, 0.30, 0.015),
                direction: kernel::Vec3::new(0.0, -1.0, 0.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
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
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.01,
            constraint_plane: None,
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
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
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
            ray: PickRay {
                origin: Point3::new(ox, oy, oz),
                direction: kernel::Vec3::new(dx, dy, dz),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
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

        // Materials.
        let m = scene.add_material("Red".to_string(), 255, 0, 0, 255);
        scene.set_object_material(a, m).unwrap();
        scene.set_material_alpha(m, 128).unwrap();

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
            .push_pull_in_component(comp, member, top, 1.0)
            .unwrap();

        // One document action for the edit; undo restores it, both instances live.
        scene.scene_undo().unwrap();
        assert_eq!(scene.instance_ids().len(), 2);
        assert_eq!(scene.instance_def(inst), Some(comp));
        assert_eq!(scene.instance_def(inst2), Some(comp));
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
                0.05, None, None, None,
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
            .snap(5.0, 5.0, 3.0, 0.0, 0.0, -1.0, 0.05, None, None, None)
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
                    ray.0, ray.1, ray.2, ray.3, ray.4, ray.5, ray.6, None, None, None,
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
            .snap(2.0, 3.0, 5.0, 0.0, 0.0, -1.0, 0.05, None, None, None)
            .unwrap()
            .expect("ray points straight at the guide point");
        assert_eq!(snap.kind(), "endpoint");
    }
}
