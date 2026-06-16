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
    BooleanOp, DocChange, Document, DocumentError, EdgeId, FaceId, KernelOp, KernelOpError,
    KernelOpReport, Object, ObjectId, Plane, Point3, SketchEdgeId, SketchId, SketchRegionId,
    Transform, WatertightState,
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
        // UnknownSketch/UnknownObject/NothingTo{Undo,Redo}/InverseFailed carry no
        // separate inner code: the variant name is the code.
        _ => api_err(&e, &e),
    }
}

fn sketch_id(handle: u64) -> SketchId {
    SketchId::from(KeyData::from_ffi(handle))
}

fn object_id(handle: u64) -> ObjectId {
    ObjectId::from(KeyData::from_ffi(handle))
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
}

#[wasm_bindgen]
impl FacePickJs {
    /// Handle of the picked object.
    pub fn object(&self) -> u64 {
        self.object
    }

    /// Handle of the picked face within that object.
    pub fn face(&self) -> u64 {
        self.face
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

    /// Source Object handle, if the snap came from scene geometry.
    pub fn object(&self) -> Option<u64> {
        self.snap.source.map(|s| s.object.data().as_ffi())
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
        for &id in &change.objects_touched {
            self.mesh_cache.remove(id);
            match self.doc.object(id) {
                Some(object) => {
                    self.inference.add_object(id, object, &Transform::IDENTITY);
                }
                None => self.inference.remove_object(id),
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

    /// Render buffers for one Object (cached until its next mutation).
    pub fn object_mesh(&mut self, object: u64) -> Result<MeshJs, ApiError> {
        let id = object_id(object);
        if !self.mesh_cache.contains_key(id) {
            let object = self
                .doc
                .object(id)
                .ok_or_else(|| stale("UnknownObject", "object"))?;
            let mesh = tessellate(object).map_err(|e| api_err(&e, &e))?;
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

    /// Push/pull a face (recorded in the object's undo history).
    pub fn push_pull(
        &mut self,
        object: u64,
        face: u64,
        distance: f64,
    ) -> Result<PushPullJs, ApiError> {
        let op = KernelOp::PushPull {
            face: FaceId::from(KeyData::from_ffi(face)),
            distance,
        };
        match self.apply_op(object, op)? {
            KernelOpReport::PushPull(inner) => Ok(PushPullJs { inner }),
            other => Err(api_err(&other, &"unexpected report kind for push_pull")),
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
    /// `lock_axis` is 0/1/2 for X/Y/Z. Returns `undefined` when nothing
    /// snaps (tools fall back to their own plane intersection).
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
            }),
            // pick_face only ever yields faces; anything else is a bug.
            _ => None,
        }
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
    let object = Object::tetrahedron();
    let mesh = tessellate(&object).expect("the demo tetrahedron is convex, planar, and hole-free");
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
}
