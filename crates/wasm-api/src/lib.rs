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

use inference::{Axis, ElementRef, InferenceScene, PickRay, SnapKind, SnapLock, SnapQuery};
use kernel::{
    BooleanOp, ComponentId, DocChange, Document, DocumentError, EdgeId, FaceId, GroupId,
    ImageFormat, InstanceId, KernelOp, KernelOpError, KernelOpReport, LoadError, Material,
    MaterialId, NodeId, Object, ObjectId, Plane, Point3, Rgba8, SketchEdgeId, SketchId,
    SketchRegionId, Texture, Transform, WatertightState,
};
use slotmap::{Key, KeyData, SecondaryMap};
use tessellate::{RenderMesh, tessellate};
use wasm_bindgen::prelude::*;

/// Module-init hook: routes Rust panics to `console.error` with the real
/// message and source location. Without it, a panic surfaces only as the
/// opaque wasm "unreachable" trap. (A panic still traps and leaves the Scene
/// unusable — reload to recover — but at least it is now diagnosable.)
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Kernel crate version, for smoke tests and an about box.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ------------------------------------------------------------------ errors

/// Boundary error: stringly `"CODE: message"` per docs/DEVELOPMENT.md B3.
#[derive(Debug)]
pub struct ApiError(String);

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
        DocumentError::Boolean(inner) => api_err(inner, &e),
        DocumentError::Transform(inner) => api_err(inner, &e),
        DocumentError::Op(KernelOpError::PushPull(inner)) => api_err(inner, &e),
        DocumentError::Op(KernelOpError::Sticky(inner)) => api_err(inner, &e),
        // UnknownSketch/UnknownObject/UnknownFace/UnknownMaterial/NothingTo{Undo,Redo}/
        // InverseFailed carry no separate inner code: the variant name is the code.
        _ => api_err(&e, &e),
    }
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
#[derive(Clone)]
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

    /// Line-segment endpoints (xyz pairs), one segment per unique edge.
    pub fn edge_positions(&self) -> Vec<f32> {
        self.mesh.edge_positions.clone()
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

/// What `push_pull` did (mirrors `kernel::PushPullReport`).
#[wasm_bindgen]
pub struct PushPullJs {
    inner: kernel::PushPullReport,
}

#[wasm_bindgen]
impl PushPullJs {
    /// The moved face in its new position (handle may differ from input).
    pub fn face(&self) -> u64 {
        self.inner.face.data().as_ffi()
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

    /// Snap kind for cue styling: "endpoint", "midpoint", "intersection",
    /// "on-edge", "on-face", "on-axis", "parallel", "perpendicular".
    pub fn kind(&self) -> String {
        match self.snap.kind {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Intersection => "intersection",
            SnapKind::OnEdge => "on-edge",
            SnapKind::OnFace => "on-face",
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

    /// Source element handle within the object (see `element_kind`).
    pub fn element(&self) -> Option<u64> {
        self.snap.source.map(|s| match s.element {
            ElementRef::Vertex(v) => v.data().as_ffi(),
            ElementRef::Edge(e) => e.data().as_ffi(),
            ElementRef::Face(f) => f.data().as_ffi(),
        })
    }

    /// "vertex" | "edge" | "face" for interpreting `element`.
    pub fn element_kind(&self) -> Option<String> {
        self.snap.source.map(|s| {
            match s.element {
                ElementRef::Vertex(_) => "vertex",
                ElementRef::Edge(_) => "edge",
                ElementRef::Face(_) => "face",
            }
            .to_string()
        })
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
    /// (replace semantics, mirroring the Document's view). Sketches carry no
    /// inference/cache state today, so `sketches_touched` needs no action here.
    fn reconcile(&mut self, change: &DocChange) {
        // Objects: drop the cached mesh, then (re)register *world* objects with
        // inference at identity, or drop hidden/gone ones. Definition members
        // are not world inference candidates — they reach inference only
        // through their instances below — but their mesh cache is still dropped
        // so a definition edit re-tessellates the shared geometry.
        for &id in &change.objects_touched {
            self.mesh_cache.remove(id);
            if self.doc.is_world_object(id) {
                let object = self.doc.object(id).expect("world object is live");
                self.inference.add_object(id, object, &Transform::IDENTITY);
            } else {
                self.inference.remove_object(id);
            }
        }
        // Instances: re-register each touched instance's definition geometry at
        // its pose (clearing any prior candidates first), or drop hidden/gone
        // ones. A definition edit lands every instance in `instances_touched`,
        // so shared-geometry changes propagate to all placements here.
        for &iid in &change.instances_touched {
            self.inference.remove_instance(iid);
            self.register_instance(iid);
        }
    }

    /// Registers a visible instance's definition members with inference, each
    /// placed at the instance pose. A no-op for a hidden/stale instance (its
    /// candidates were already cleared by the caller).
    fn register_instance(&mut self, iid: InstanceId) {
        let (Some(def), Some(pose)) = (self.doc.instance_def(iid), self.doc.instance_pose(iid))
        else {
            return;
        };
        let Some(members) = self.doc.def_members(def) else {
            return;
        };
        for m in members {
            if let Some(object) = self.doc.object(m) {
                self.inference.add_instance(iid, m, object, &pose);
            }
        }
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
        }
    }

    // ------------------------------------------------------------ sketching

    /// Adds a fresh, empty sketch on the ground plane (M1: the only sketch
    /// surface) and returns its handle. **Additive** — existing sketches are
    /// untouched, so independent coplanar shapes can coexist.
    pub fn begin_ground_sketch(&mut self) -> u64 {
        self.doc.add_sketch(ground_plane()).data().as_ffi()
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
        let s = self
            .doc
            .sketch_mut(sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s
            .add_segment(Point3::new(ax, ay, az), Point3::new(bx, by, bz))
            .map_err(|e| api_err(&e, &e))?;
        Ok(SegmentAddedJs { inner: report })
    }

    /// Removes a sketch edge (eraser tool).
    pub fn sketch_remove_edge(
        &mut self,
        sketch: u64,
        edge: u64,
    ) -> Result<EdgeRemovedJs, ApiError> {
        let s = self
            .doc
            .sketch_mut(sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let report = s
            .remove_edge(SketchEdgeId::from(KeyData::from_ffi(edge)))
            .map_err(|e| api_err(&e, &e))?;
        Ok(EdgeRemovedJs { inner: report })
    }

    /// All sketch edges as xyz line-segment endpoint pairs, for drawing.
    pub fn sketch_lines(&self, sketch: u64) -> Result<Vec<f32>, ApiError> {
        let s = self
            .doc
            .sketch(sketch_id(sketch))
            .ok_or_else(|| stale("UnknownSketch", "sketch"))?;
        let mut out = Vec::with_capacity(s.edges().len() * 6);
        for edge in s.edges().values() {
            for v in [edge.from, edge.to] {
                let p = s.vertices()[v].position;
                out.extend([p.x as f32, p.y as f32, p.z as f32]);
            }
        }
        Ok(out)
    }

    /// Handles of the sketch's current closed regions, excluding any already
    /// extruded into a solid (those are consumed — see `extrude_region`).
    pub fn sketch_regions(&self, sketch: u64) -> Result<Vec<u64>, ApiError> {
        let regions = self
            .doc
            .extrudable_regions(sketch_id(sketch))
            .map_err(doc_err)?;
        Ok(regions.iter().map(|r| r.data().as_ffi()).collect())
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
        let region = SketchRegionId::from(KeyData::from_ffi(region));
        let (id, change) = self
            .doc
            .extrude_region(sketch_id(sketch), region, distance)
            .map_err(doc_err)?;
        self.reconcile(&change);
        Ok(id.data().as_ffi())
    }

    /// Explicit combine (ARCHITECTURE.md): unions/subtracts/intersects two objects,
    /// consuming the operands into the returned result handle. `op` is
    /// 0 = union, 1 = subtract (`a - b`), 2 = intersect. Operands and result
    /// stay stable handles across undo/redo.
    pub fn boolean(&mut self, op: u8, a: u64, b: u64) -> Result<u64, ApiError> {
        let op = match op {
            0 => BooleanOp::Union,
            1 => BooleanOp::Subtract,
            2 => BooleanOp::Intersect,
            _ => return Err(ApiError("BadOp: op must be 0, 1, or 2".to_string())),
        };
        let (id, change) = self
            .doc
            .boolean(op, object_id(a), object_id(b))
            .map_err(doc_err)?;
        self.reconcile(&change);
        Ok(id.data().as_ffi())
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
        Ok(())
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
    /// member node (kind `0` = object, `1` = group); they must be the same
    /// length, name live sibling nodes, and not repeat.
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
        Ok(group.data().as_ffi())
    }

    /// Dissolves a group, returning its members to the group's own parent
    /// (inverse of `group_nodes`). The members keep their geometry and handles.
    pub fn ungroup(&mut self, group: u64) -> Result<(), ApiError> {
        let change = self.doc.ungroup(group_id(group)).map_err(doc_err)?;
        self.reconcile(&change);
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
                Ok(PushPullJs { inner })
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

    /// Push/pull a face (recorded in the object's undo history). A flat imprinted
    /// sub-face (drawn inside an Object) auto-routes to wall-generating
    /// extrude (boss/recess); any other face uses the translate-mode push/pull.
    pub fn push_pull(
        &mut self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<PushPullJs, ApiError> {
        let face_id = FaceId::from(KeyData::from_ffi(face));
        let is_sub = self
            .doc
            .object(object_id(object))
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
                Ok(PushPullJs { inner })
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
        };
        match self.apply_op(object, op)? {
            KernelOpReport::FaceSplitInner(r) => Ok(r.sub_face.data().as_ffi()),
            other => Err(api_err(
                &other,
                &"unexpected report kind for split_face_inner",
            )),
        }
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
        };
        match self.apply_op(object, op)? {
            KernelOpReport::FaceSplit(inner) => Ok(FaceSplitJs { inner }),
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
            KernelOpReport::FaceMerge(inner) => Ok(FaceMergeJs { inner }),
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
    /// per-object op delegates to that object's [`History`].
    pub fn scene_undo(&mut self) -> Result<(), ApiError> {
        let change = self.doc.undo().map_err(doc_err)?;
        self.reconcile(&change);
        Ok(())
    }

    /// Re-applies the most recently undone document action. Object handles are
    /// stable across undo/redo (undone creations are hidden, not deleted), so
    /// redo never has to remap ids.
    pub fn scene_redo(&mut self) -> Result<(), ApiError> {
        let change = self.doc.redo().map_err(doc_err)?;
        self.reconcile(&change);
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
        let source = self.inference.pick_face(&ray)?;
        match source.element {
            ElementRef::Face(f) => Some(FacePickJs {
                object: source.object.data().as_ffi(),
                face: f.data().as_ffi(),
                instance: source.instance.map(|i| i.data().as_ffi()),
            }),
            // pick_face only ever yields faces; anything else is a bug.
            _ => None,
        }
    }

    // ------------------------------------------------------- materials

    /// Add a solid-color material to the palette and return its handle.
    /// Palette additions are not individually undoable — only face assignment
    /// via [`Scene::paint_face`] is.
    pub fn add_material(&mut self, name: String, r: u8, g: u8, b: u8, a: u8) -> u64 {
        let mat = Material::solid(name, Rgba8::rgba(r, g, b, a));
        self.doc.add_material(mat).data().as_ffi()
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
        let mat = Material::textured(name, Rgba8::rgba(r, g, b, a), texture);
        Ok(self.doc.add_material(mat).data().as_ffi())
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
        Ok(())
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
    /// Throws a `"LOAD: <message>"` `JsError` on any parse/validation failure
    /// (bad magic, unsupported version, malformed manifest, dangling reference,
    /// invalid topology).
    pub fn load(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let new_doc =
            Document::load(bytes).map_err(|e: LoadError| JsError::new(&format!("LOAD: {e}")))?;

        // Swap is committed only after successful parse.
        self.doc = new_doc;
        self.mesh_cache = SecondaryMap::new();
        self.inference = InferenceScene::new();

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

    #[test]
    fn empty_scene_has_no_objects_and_rejects_stale_handles() {
        let scene = Scene::new();
        assert!(scene.object_ids().is_empty());
        assert!(scene.object_watertight(42).is_err());
        assert!(!scene.can_scene_undo());
        assert!(!scene.can_scene_redo());
    }

    /// Draws a unit square on the ground sketch and returns
    /// (sketch_handle, region_handle).
    fn ground_unit_square(scene: &mut Scene) -> (u64, u64) {
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
                .sketch_add_segment(sketch, ax, ay, 0.0, bx, by, 0.0)
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
        // The region is consumed: gone from the list, so it can't re-extrude.
        assert!(scene.sketch_regions(sketch).unwrap().is_empty());

        // Undo the creation: the object is hidden (gone from the listing) but
        // its handle is preserved for redo.
        scene.scene_undo().unwrap();
        assert!(scene.object_ids().is_empty());
        assert!(scene.object_watertight(obj).is_err()); // hidden = not live
        assert!(scene.can_scene_redo());

        // Undo also restored the region's extrudability.
        assert_eq!(scene.sketch_regions(sketch).unwrap(), vec![region]);

        // Redo restores the SAME handle and re-consumes the region.
        scene.scene_redo().unwrap();
        assert_eq!(scene.object_ids(), vec![obj]);
        assert!(scene.object_watertight(obj).unwrap());
        assert!(scene.sketch_regions(sketch).unwrap().is_empty());
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
        let (s2, r2) = ground_unit_square(&mut scene);
        let o2 = scene.extrude_region(s2, r2, 1.0).unwrap();

        // Two identical ground boxes share coplanar faces; the boolean now
        // resolves coplanar contact instead of refusing. Union of
        // coincident solids is one box — operands consumed, one object left.
        let result = scene.boolean(0, o1, o2).unwrap();
        assert_eq!(scene.object_ids(), vec![result]);
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

    /// Two top-level boxes group into one node, transform together, and ungroup
    /// back — all non-destructively and undoably across the FFI.
    #[test]
    fn group_transform_ungroup_round_trip() {
        let mut scene = Scene::new();
        let (s1, r1) = ground_unit_square(&mut scene);
        let o1 = scene.extrude_region(s1, r1, 1.0).unwrap();
        let (s2, r2) = ground_unit_square(&mut scene);
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
}
