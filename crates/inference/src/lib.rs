//! Snapping/inference engine: pure geometry queries over the whole scene.
//!
//! The SketchUp-feel half of the product (ARCHITECTURE.md, risk #2): tools ask
//! "the cursor is along this ray — where does the user *mean*?", and this
//! crate answers with a [`Snap`]: a position, why it snapped, and to what.
//!
//! Boundaries (DEVELOPMENT.md rule 1): UI-free, I/O-free, renderer-free. The UI
//! converts pixels to a [`PickRay`] + cone aperture and draws the cues; this
//! crate only does geometry. It reads kernel Objects but NEVER mutates —
//! inference queries against the whole scene, while geometry merging stays
//! within the active Object (ARCHITECTURE.md).
//!
//! # Priority model
//!
//! When several candidates fall inside the pick cone, the strongest
//! [`SnapKind`] wins (the enum's declaration order IS the priority order,
//! strongest first — `SnapKind`'s `Ord` reflects it and tools may rely on
//! that). Among candidates of equal kind, the one nearest the ray wins; ties
//! break toward the one nearest the ray origin (closest to camera).
//!
//! # Occlusion
//!
//! The pick cone is a screen-space projection with no depth buffer, so on its
//! own it "sees through" solids: a hidden back edge or vertex (which outranks a
//! face on [`SnapKind`] priority) would beat the visible front face under the
//! cursor. After ranking, `resolve` therefore walks the sorted list and returns
//! the first candidate that is *visible* — not hidden behind an opaque face
//! along the ray to it (see `is_occluded`). Only what you can see can snap,
//! matching SketchUp. A tool-supplied `constraint_plane` is a separate, additive
//! filter (it restricts candidates to the active drawing plane).
//!
//! # Locking
//!
//! A [`SnapLock`] (shift-lock or arrow keys in SketchUp terms) constrains the
//! result to a line through the query's `anchor`. With a lock active, every
//! candidate is projected onto the locked line before ranking, and the
//! returned snap keeps the candidate's `kind`/`source` so the UI can still
//! say *why* (e.g. "on axis, from endpoint").
//!
//! M1 status: point/segment/face candidates are pruned through a lazily
//! rebuilt AABB BVH (see `index`) before the exact per-candidate tests run;
//! constant-count candidates (guides, world axes/origin) and gesture-scoped
//! ones (sketch/transient segments) stay on a linear walk. The API
//! deliberately hides the storage so the index strategy can change without
//! touching callers. Intersection snaps (`SnapKind::Intersection`) are M2 —
//! the variant exists but `resolve` does not emit it yet.

use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeSet;

use kernel::{
    EdgeId, FaceId, Guide, GuideId, InstanceId, Object, ObjectId, Plane, Point3, SketchId,
    SketchVertexId, Transform, Vec3, VertexId, tol,
};

mod index;

use index::SceneIndex;

/// A picking ray in world space (UI derives it from the camera + cursor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickRay {
    /// Ray start (the camera/near-plane point).
    pub origin: Point3,
    /// Ray direction; need not be normalized.
    pub direction: Vec3,
}

/// Why a position snapped. **Declaration order is priority order, strongest
/// first**; `Ord` follows it (smaller = stronger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// Exactly on an existing vertex.
    Endpoint,
    /// On the midpoint of an edge.
    Midpoint,
    /// On the apparent intersection of two edges.
    Intersection,
    /// Anywhere along an edge.
    OnEdge,
    /// Anywhere on a face.
    OnFace,
    /// On a construction guide: a line or point the user placed
    /// deliberately as a drawing aid. Beats the ambient world axes (it's a
    /// real, user-placed reference) but loses to actual solid geometry
    /// (faces/edges/vertices) — a guide is an aid, not material.
    OnGuide,
    /// On a model axis (or locked direction) through the anchor.
    OnAxis,
    /// Direction parallel to a reference edge (M2; needs a reference).
    Parallel,
    /// Direction perpendicular to a reference edge (M2; needs a reference).
    Perpendicular,
}

/// The scene element a snap derives from, for highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRef {
    /// A kernel vertex.
    Vertex(VertexId),
    /// A kernel edge.
    Edge(EdgeId),
    /// A kernel face.
    Face(FaceId),
}

/// Which Object (and which element of it) produced a snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapSource {
    /// The geometry-owning Object: a world solid, or — when `instance` is set —
    /// the component definition member the element belongs to (so kernel ops
    /// route correctly via [`kernel::Document::apply_def_op`]).
    pub object: ObjectId,
    /// The element within that Object.
    pub element: ElementRef,
    /// The placing component instance, if this candidate came from one:
    /// `None` for a plain world object, `Some` for instanced geometry. Lets two
    /// instances of one definition coexist without colliding, and tells the UI
    /// which placement was hit (for selection/highlight).
    pub instance: Option<InstanceId>,
}

/// A resolved snap: where the cursor should land and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    /// The snapped position in world space.
    pub position: Point3,
    /// What kind of snap this is (drives the cue color/glyph).
    pub kind: SnapKind,
    /// Provenance for highlighting; `None` for pure-direction snaps like
    /// [`SnapKind::OnAxis`].
    pub source: Option<SnapSource>,
    /// The inference direction for directional snaps (axis / parallel /
    /// perpendicular), for drawing the dashed guide line.
    pub direction: Option<Vec3>,
}

/// The three model axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// +X (red in SketchUp tradition).
    X,
    /// +Y (green).
    Y,
    /// +Z (blue).
    Z,
}

impl Axis {
    /// The unit vector of this axis.
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::new(1.0, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, 1.0, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, 1.0),
        }
    }
}

/// A direction constraint for the current tool gesture (see module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapLock {
    /// Locked to a model axis through the anchor.
    Axis(Axis),
    /// Locked to an arbitrary direction through the anchor (e.g. "hold to
    /// keep this inference").
    Direction(Vec3),
}

/// One inference request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapQuery {
    /// The pick ray under the cursor.
    pub ray: PickRay,
    /// The tool's fixed point (e.g. a line's first endpoint). Required for
    /// axis/parallel/perpendicular inference; point snaps work without it.
    pub anchor: Option<Point3>,
    /// Active direction lock, if any.
    pub lock: Option<SnapLock>,
    /// Pick-cone half-angle in radians. The UI computes it from its snap
    /// radius in pixels and the camera FOV, keeping this crate
    /// screen-agnostic.
    pub aperture: f64,
    /// Active drawing-plane constraint. When `Some`, only candidates whose
    /// position lies on this plane (within [`tol::PLANE_DIST`]) are considered —
    /// drawing on a face must not "see through" the solid and snap to occluded,
    /// off-plane geometry (hidden edges/midpoints/vertices). `OnFace` candidates
    /// then naturally collapse to the coplanar (active) face. `None` keeps the
    /// unconstrained behavior (free-space / ground drawing).
    pub constraint_plane: Option<Plane>,
}

/// A snappable point with provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScenePoint {
    /// World-space position.
    pub position: Point3,
    /// Where it came from.
    pub source: SnapSource,
}

/// A snappable segment (kernel edge in world space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSegment {
    /// One endpoint.
    pub a: Point3,
    /// The other endpoint.
    pub b: Point3,
    /// Where it came from.
    pub source: SnapSource,
}

/// A snappable planar face region (kernel face in world space).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFace {
    /// The supporting plane.
    pub plane: Plane,
    /// Outer boundary in cycle order (containment tests happen against it).
    pub boundary: Vec<Point3>,
    /// Inner loops (holes) in cycle order. A ray that lands inside any of
    /// these is NOT on the face — it passes through the hole (e.g. the annular
    /// parent of an imprinted sub-face). Empty for ordinary faces.
    pub holes: Vec<Vec<Point3>>,
    /// Where it came from.
    pub source: SnapSource,
}

/// A construction guide registered with the inference engine, in world
/// space. Guides carry no topology and have no `SnapSource` — like the world
/// origin/axes, they snap with `source: None` (see [`SnapKind::OnGuide`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct SceneGuide {
    id: GuideId,
    geom: SceneGuideGeom,
}

/// A bare endpoint-pair segment with no `SnapSource` provenance — used for
/// sketch and transient candidates (Part 1, Phase B), which aren't yet
/// selectable kernel elements. Endpoints/midpoints derived from these snap
/// exactly like [`SceneSegment`]'s, just with `source: None` (mirroring how a
/// guide or the world axes snap with no provenance).
#[derive(Debug, Clone, Copy, PartialEq)]
struct BareSegment {
    a: Point3,
    b: Point3,
}

/// The geometry of a [`SceneGuide`], mirroring [`kernel::Guide`] in world
/// space (guides carry no placement/instance — they're authored directly in
/// world coordinates, unlike Object geometry).
#[derive(Debug, Clone, Copy, PartialEq)]
enum SceneGuideGeom {
    /// An infinite construction line through `origin` along unit `direction`.
    Line { origin: Point3, direction: Vec3 },
    /// A single construction point.
    Point { position: Point3 },
}

/// The engine's view of the scene: world-space snap candidates extracted
/// from every Object, refreshed incrementally as Objects change.
///
/// The spatial index lives behind this type; the public API exposes
/// only candidates and queries so the indexing strategy stays swappable.
#[derive(Debug, Clone)]
pub struct InferenceScene {
    points: Vec<ScenePoint>,
    segments: Vec<SceneSegment>,
    faces: Vec<SceneFace>,
    guides: Vec<SceneGuide>,
    /// Persistent sketch candidates (committed sketch edges, not yet kernel
    /// Objects): keyed by `SketchId` so a caller can replace one sketch's
    /// segments without touching another's. No `SnapSource` provenance —
    /// sketch elements aren't selectable in this phase.
    sketch_segments: Vec<(SketchId, BareSegment)>,
    /// Committed sketch *vertices*, keyed by `SketchId`, carrying their
    /// `SketchVertexId` so the per-vertex edit tool (Phase D) can pick an exact
    /// vertex to drag. Registered/cleared alongside `sketch_segments`.
    sketch_vertices: Vec<(SketchId, SketchVertexId, Point3)>,
    /// Transient (in-progress) segments — e.g. the line tool's current
    /// rubber-band chain — published every frame and never persisted. Cleared
    /// wholesale by [`InferenceScene::clear_transient`], not per-id.
    transient_segments: Vec<BareSegment>,
    /// When `false`, guide candidates are suppressed (View ▸ Guides off): a
    /// hidden guide must not snap or flash a cue. Defaults to `true`.
    guides_enabled: bool,
    /// When `false`, the world-origin/axis candidates are suppressed (View ▸
    /// Axes off): hidden axes must not snap or flash a cue. Defaults to `true`.
    axes_enabled: bool,
    /// Lazily rebuilt spatial index over `points`/`segments`/`faces`; `None`
    /// means dirty (a mutator ran since the last build). Interior mutability
    /// because the hot pointer-move queries (`resolve`, `pick_face`) take
    /// `&self` while a rebuild must write the cache. Panic-free by
    /// construction: the only `borrow_mut` lives in
    /// [`InferenceScene::spatial_index`], whose build reads the candidate
    /// Vecs and calls nothing that touches this cell again (no reentrancy);
    /// `RefCell` additionally makes the scene `!Sync`, so a future threaded
    /// caller fails to compile instead of racing.
    spatial: RefCell<Option<SceneIndex>>,
    /// Cumulative count of exact ray-vs-face occlusion tests (see
    /// [`InferenceScene::occlusion_face_tests`]). `Cell` because queries
    /// take `&self`; single-threaded for the same reason as `spatial`.
    occlusion_tests: Cell<u64>,
    /// World-object ids currently registered via
    /// [`InferenceScene::add_object`] — exactly the ids for which candidates
    /// with `instance == None` can exist (`register` only emits such
    /// candidates on the `add_object` path). Lets
    /// [`InferenceScene::remove_object`] answer "nothing to remove" in
    /// O(log owners) instead of three O(scene) retain passes: bulk registration (document
    /// load, undo/redo re-registration) calls the replace-semantics `add_*`
    /// once per object, and paying a full-scene scan for each never-present
    /// id made that accidentally quadratic.
    world_owners: BTreeSet<ObjectId>,
    /// Instance ids currently registered via
    /// [`InferenceScene::add_instance`] — exactly the ids for which
    /// candidates with `instance == Some(id)` can exist (across every
    /// definition member the instance places). Same fast path for
    /// [`InferenceScene::remove_instance`].
    instance_owners: BTreeSet<InstanceId>,
    /// Cumulative count of candidates walked by the removal retain passes
    /// (see [`InferenceScene::removal_candidates_visited`]). Plain `u64`
    /// (not `Cell`) because removal takes `&mut self`.
    removal_visits: u64,
}

impl Default for InferenceScene {
    fn default() -> Self {
        InferenceScene {
            points: Vec::new(),
            segments: Vec::new(),
            faces: Vec::new(),
            guides: Vec::new(),
            sketch_segments: Vec::new(),
            sketch_vertices: Vec::new(),
            transient_segments: Vec::new(),
            guides_enabled: true,
            axes_enabled: true,
            spatial: RefCell::new(None),
            occlusion_tests: Cell::new(0),
            world_owners: BTreeSet::new(),
            instance_owners: BTreeSet::new(),
            removal_visits: 0,
        }
    }
}

impl InferenceScene {
    /// An empty scene.
    pub fn new() -> InferenceScene {
        InferenceScene::default()
    }

    /// Enable/disable guide snapping (View ▸ Guides). Hidden guides must not
    /// snap or flash a cue; the registered guides are kept, only their
    /// candidate emission is gated.
    pub fn set_guides_enabled(&mut self, enabled: bool) {
        self.guides_enabled = enabled;
    }

    /// Enable/disable world-origin/axis snapping (View ▸ Axes).
    pub fn set_axes_enabled(&mut self, enabled: bool) {
        self.axes_enabled = enabled;
    }

    /// Candidate counts as (points, segments, faces) — cheap introspection
    /// for tests and debug overlays.
    pub fn candidate_counts(&self) -> (usize, usize, usize) {
        (self.points.len(), self.segments.len(), self.faces.len())
    }

    /// Cumulative number of exact ray-vs-face tests performed by occlusion
    /// culling across all queries so far — cheap introspection for tests and
    /// debug overlays, like [`InferenceScene::candidate_counts`]. The
    /// spatial index exists to keep the per-query delta far below the total
    /// face count; the perf-sanity spec asserts exactly that.
    pub fn occlusion_face_tests(&self) -> u64 {
        self.occlusion_tests.get()
    }

    /// Cumulative number of candidates walked by the retain passes of
    /// [`InferenceScene::remove_object`] and
    /// [`InferenceScene::remove_instance`] across all calls so far — cheap
    /// introspection for tests and debug overlays, like
    /// [`InferenceScene::occlusion_face_tests`]. The owner-set fast path
    /// exists to make removal of a never-registered id visit zero candidates
    /// (bulk registration calls the replace-semantics `add_*` once per
    /// object, so anything else is accidentally quadratic in scene size);
    /// the removal perf-sanity spec asserts exactly that.
    pub fn removal_candidates_visited(&self) -> u64 {
        self.removal_visits
    }

    /// The spatial index, rebuilding it first if a mutator marked it dirty.
    ///
    /// Rebuild cost is O(n log n) in the candidate count and amortizes
    /// across the many pointer-move queries between committed mutations —
    /// mutators only invalidate, they never rebuild. Panic-free: this holds
    /// the crate's only `borrow_mut`, and [`SceneIndex::build`] reads the
    /// candidate Vecs without re-entering the cell (see the `spatial` field
    /// docs).
    fn spatial_index(&self) -> Ref<'_, SceneIndex> {
        if self.spatial.borrow().is_none() {
            *self.spatial.borrow_mut() =
                Some(SceneIndex::build(&self.points, &self.segments, &self.faces));
        }
        Ref::map(self.spatial.borrow(), |slot| {
            slot.as_ref()
                .expect("built above; nothing can mutate the scene through &self in between")
        })
    }

    /// Extracts snap candidates from `object` (vertices, edges with
    /// midpoints derived at query time, faces) transformed by `placement`
    /// into world space, replacing any candidates previously registered for
    /// `id`.
    ///
    /// Cost model: linear in the Object's elements; called on Object
    /// creation and after each committed mutation, never per-frame.
    pub fn add_object(&mut self, id: ObjectId, object: &Object, placement: &Transform) {
        // Replace semantics: drop any prior candidates for this id first.
        self.remove_object(id);
        // Record the owner before registering: `id` is now the one world
        // object whose candidates carry `instance == None`, which is exactly
        // what `remove_object`'s fast path keys on.
        self.world_owners.insert(id);
        self.register(object, placement, id, None);
    }

    /// Registers the candidates of one component instance: `object` is the
    /// definition `member` geometry, placed by the instance `pose`; every
    /// candidate is tagged with `instance` so two instances of one definition
    /// never collide and a pick knows which placement it hit. **Additive** — the
    /// caller clears an instance's prior candidates with
    /// [`InferenceScene::remove_instance`] before re-registering its members.
    pub fn add_instance(
        &mut self,
        instance: InstanceId,
        member: ObjectId,
        object: &Object,
        pose: &Transform,
    ) {
        // Record the owner (idempotent across the instance's members):
        // candidates carrying `instance == Some(instance)` now exist, which
        // is exactly what `remove_instance`'s fast path keys on.
        self.instance_owners.insert(instance);
        self.register(object, pose, member, Some(instance));
    }

    /// Extracts `object`'s vertices/edges/faces into world-space candidates owned
    /// by `owner`, each optionally tagged with the placing `instance`. Shared by
    /// [`InferenceScene::add_object`] (world solids, `instance == None`) and
    /// [`InferenceScene::add_instance`] (instanced geometry).
    fn register(
        &mut self,
        object: &Object,
        placement: &Transform,
        owner: ObjectId,
        instance: Option<InstanceId>,
    ) {
        // The candidate Vecs are about to change shape: drop the spatial
        // index and let the next query rebuild it. Invalidation is
        // per-committed-op (this is never called per-frame), so the rebuild
        // amortizes across the many pointer-move queries in between.
        *self.spatial.get_mut() = None;

        // --- Vertices -> ScenePoint (Endpoint source) ---
        for (vid, vertex) in object.vertices() {
            self.points.push(ScenePoint {
                position: placement.apply_point(vertex.position),
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Vertex(vid),
                    instance,
                },
            });
        }

        // --- Edges -> SceneSegment (midpoints derived at query time) ---
        for (eid, edge) in object.edges() {
            // Each edge references a half-edge; get its two endpoint vertices.
            let half_edges = object.half_edges();
            let he = &half_edges[edge.half_edge];
            let origin_vid = he.origin;
            let dest_vid = half_edges[he.next].origin;
            let a = placement.apply_point(object.vertices()[origin_vid].position);
            let b = placement.apply_point(object.vertices()[dest_vid].position);
            self.segments.push(SceneSegment {
                a,
                b,
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Edge(eid),
                    instance,
                },
            });
        }

        // --- Faces -> SceneFace (plane + outer-loop boundary) ---
        for (fid, face) in object.faces() {
            // Apply placement to the plane via apply_plane (handles normals
            // under non-uniform scale via inverse-transpose — KERNEL_GUIDE trap).
            let world_plane = match placement.apply_plane(&face.plane) {
                Ok(p) => p,
                Err(_) => continue, // singular placement: skip this face
            };
            // Boundary: outer loop positions transformed into world space.
            let boundary: Vec<Point3> = object
                .loop_positions(face.outer_loop)
                .map(|p| placement.apply_point(p))
                .collect();
            // Holes: each inner loop transformed into world space.
            let holes: Vec<Vec<Point3>> = face
                .inner_loops
                .iter()
                .map(|&lid| {
                    object
                        .loop_positions(lid)
                        .map(|p| placement.apply_point(p))
                        .collect()
                })
                .collect();
            self.faces.push(SceneFace {
                plane: world_plane,
                boundary,
                holes,
                source: SnapSource {
                    object: owner,
                    element: ElementRef::Face(fid),
                    instance,
                },
            });
        }
    }

    /// Drops all **world-object** candidates registered for `id` (instanced
    /// candidates are keyed by instance, see [`InferenceScene::remove_instance`]).
    /// Unknown ids are a no-op — removal must be idempotent so document undo can
    /// call it freely, and the no-op never scans candidates (see
    /// [`InferenceScene::removal_candidates_visited`]).
    pub fn remove_object(&mut self, id: ObjectId) {
        // Owner-set fast path: candidates matching `world` below exist only
        // for ids in `world_owners` (`register` emits `instance == None`
        // candidates solely on the `add_object` path, which inserts). For a
        // never-registered id the retain passes would walk every candidate
        // to remove nothing, so idempotent callers — document load and
        // undo/redo re-register N objects, each `add_*` starting with this
        // removal — went accidentally quadratic. Nothing is removed here, no
        // index shifts, so the spatial index stays valid too: skip the dirty.
        if !self.world_owners.remove(&id) {
            return;
        }
        // The retain passes shift every index behind the removed candidates,
        // so the whole spatial index is stale: mark it dirty for a lazy full
        // rebuild on the next query (per-committed-op, never per-frame).
        *self.spatial.get_mut() = None;
        self.removal_visits += (self.points.len() + self.segments.len() + self.faces.len()) as u64;
        let world = |s: &SnapSource| s.object == id && s.instance.is_none();
        self.points.retain(|p| !world(&p.source));
        self.segments.retain(|s| !world(&s.source));
        self.faces.retain(|f| !world(&f.source));
    }

    /// Drops all candidates registered for `instance` (across every definition
    /// member it places). Idempotent, so document undo can call it freely —
    /// and the unknown-id no-op never scans candidates (see
    /// [`InferenceScene::removal_candidates_visited`]).
    pub fn remove_instance(&mut self, instance: InstanceId) {
        // Owner-set fast path, mirroring `remove_object`: candidates with
        // `instance == Some(instance)` exist only for ids in
        // `instance_owners` (`add_instance` inserts before registering), so
        // a never-registered id has nothing to remove — return without
        // scanning, and without dirtying the still-valid spatial index.
        if !self.instance_owners.remove(&instance) {
            return;
        }
        // Same index-shifting retain passes as `remove_object`: dirty the
        // spatial index for a lazy rebuild.
        *self.spatial.get_mut() = None;
        self.removal_visits += (self.points.len() + self.segments.len() + self.faces.len()) as u64;
        let key = Some(instance);
        self.points.retain(|p| p.source.instance != key);
        self.segments.retain(|s| s.source.instance != key);
        self.faces.retain(|f| f.source.instance != key);
    }

    /// Drops every object- and instance-sourced candidate at once, leaving
    /// guides, sketches, and transient segments registered. For bulk
    /// visibility rebuilds (e.g. applying a whole hidden set): removing N
    /// registered owners one at a time scans the candidate vectors once per
    /// owner, while clearing and re-registering the visible remainder is one
    /// linear pass in total — each re-registration's replace-semantics
    /// removal hits the empty-owner fast path.
    pub fn clear_solids(&mut self) {
        *self.spatial.get_mut() = None;
        self.points.clear();
        self.segments.clear();
        self.faces.clear();
        self.world_owners.clear();
        self.instance_owners.clear();
    }

    /// Registers (or re-registers) one construction guide as a
    /// snap target. Replace semantics, mirroring [`InferenceScene::add_object`]:
    /// drops any prior candidate for `id` first, so callers can call this on
    /// every guide creation/edit without tracking whether it's new.
    pub fn add_guide(&mut self, id: GuideId, guide: &Guide) {
        self.remove_guide(id);
        let geom = match *guide {
            Guide::Line { origin, direction } => SceneGuideGeom::Line { origin, direction },
            Guide::Point { position } => SceneGuideGeom::Point { position },
        };
        self.guides.push(SceneGuide { id, geom });
    }

    /// Drops the candidate registered for guide `id`. Unknown ids are a
    /// no-op — removal must be idempotent so document undo/redo can call it
    /// freely (mirroring [`InferenceScene::remove_object`]).
    pub fn remove_guide(&mut self, id: GuideId) {
        self.guides.retain(|g| g.id != id);
    }

    /// Number of guides currently registered — cheap introspection for tests
    /// (kept separate from [`InferenceScene::candidate_counts`] so that
    /// existing callers of the points/segments/faces tuple are unaffected).
    pub fn guide_count(&self) -> usize {
        self.guides.len()
    }

    /// Registers (or re-registers) the committed segments of sketch `id` as
    /// snap candidates: each segment's endpoints and derived midpoint resolve
    /// exactly like a kernel edge's, but with `source: None` (sketch elements
    /// aren't selectable in this phase — no `ElementRef` variant exists for
    /// them yet). Replace semantics, mirroring [`InferenceScene::add_object`]:
    /// drops any prior candidates for `id` first, so callers can call this on
    /// every sketch mutation (add/remove segment, extrude) without tracking
    /// whether `id` was already registered.
    pub fn add_sketch(&mut self, id: SketchId, segments: &[(Point3, Point3)]) {
        self.remove_sketch(id);
        self.sketch_segments
            .extend(segments.iter().map(|&(a, b)| (id, BareSegment { a, b })));
    }

    /// Registers (or re-registers) the committed *vertices* of sketch `id` as
    /// pickable targets for the per-vertex edit tool (Phase D), carrying each
    /// `SketchVertexId`. Replace semantics like [`InferenceScene::add_sketch`]:
    /// drops any prior vertices for `id` first. Callers register vertices and
    /// segments together on every sketch mutation.
    pub fn add_sketch_vertices(&mut self, id: SketchId, vertices: &[(SketchVertexId, Point3)]) {
        self.sketch_vertices.retain(|(sid, _, _)| *sid != id);
        self.sketch_vertices
            .extend(vertices.iter().map(|&(vid, p)| (id, vid, p)));
    }

    /// Drops all candidates registered for sketch `id`. Unknown ids are a
    /// no-op — removal must be idempotent (mirroring
    /// [`InferenceScene::remove_object`]) so callers can remove-then-add
    /// freely.
    pub fn remove_sketch(&mut self, id: SketchId) {
        self.sketch_segments.retain(|(sid, _)| *sid != id);
        self.sketch_vertices.retain(|(sid, _, _)| *sid != id);
    }

    /// Publishes one transient (in-progress) segment as a snap candidate —
    /// e.g. a just-placed point in the line tool's current chain, which never
    /// touches the kernel sketch until the gesture commits. Additive; callers
    /// typically call [`InferenceScene::clear_transient`] then re-publish the
    /// whole current chain each time it changes (a one-frame lag between
    /// publish and the next `resolve` is expected — see wasm-api docs).
    pub fn add_transient_segment(&mut self, a: Point3, b: Point3) {
        self.transient_segments.push(BareSegment { a, b });
    }

    /// Drops every transient segment. Idempotent.
    pub fn clear_transient(&mut self) {
        self.transient_segments.clear();
    }

    /// Answers one inference query (see the module docs for the priority and
    /// locking model). Returns `None` when nothing falls inside the pick
    /// cone and no lock/anchor produces a directional snap — the tool then
    /// uses its own fallback (e.g. ground-plane intersection).
    ///
    /// Must be cheap enough to call on every mouse-move at interactive
    /// rates: the spatial index (lazily rebuilt after committed mutations)
    /// prunes the point/segment/face candidates to a conservative superset
    /// before the exact tests run.
    pub fn resolve(&self, query: &SnapQuery) -> Option<Snap> {
        let index = self.spatial_index();
        self.resolve_impl(query, Some(&index))
    }

    /// Reference implementation of [`resolve`](Self::resolve) with the
    /// spatial index bypassed — an honest full linear scan, kept so the
    /// executable specs and property tests can assert the indexed path
    /// returns byte-for-byte identical snaps (DEVELOPMENT.md rule 3). Not
    /// part of the supported API.
    #[doc(hidden)]
    pub fn resolve_linear(&self, query: &SnapQuery) -> Option<Snap> {
        self.resolve_impl(query, None)
    }

    /// Shared body of `resolve`/`resolve_linear`: with `index == None` every
    /// candidate is scanned; with `Some` only the index's conservative
    /// superset is, in ascending-index (= linear emission) order, so the
    /// exact tests, ranking, tie-breaks, and occlusion behave identically on
    /// both paths.
    fn resolve_impl(&self, query: &SnapQuery, index: Option<&SceneIndex>) -> Option<Snap> {
        // Normalize the ray direction; degenerate direction -> None.
        let dir = match query.ray.direction.normalized() {
            Ok(d) => d,
            Err(_) => return None,
        };
        let origin = query.ray.origin;
        let aperture = query.aperture;

        // tan(aperture) bounds the cone's radius growth per unit depth for
        // the index's conservative node test. At or past a 90° half-angle
        // the cone covers the whole front half-space, so the radius prune is
        // disabled (`None`); FRAC_PI_2 is a domain bound, not a tolerance.
        // The cutoff backs off by tol::CONE_SLACK: within that band of π/2
        // the tangent is so ill-conditioned that the node test's guard band
        // (see `Aabb::maybe_in_cone`) could no longer provably cover the
        // exact test's rounding, so those cones are treated as the whole
        // front half-space too. Only pruning strength is affected — the
        // exact tests always use `aperture` itself.
        let tan_aperture =
            (aperture < std::f64::consts::FRAC_PI_2 - tol::CONE_SLACK).then(|| aperture.tan());

        // Candidate index sets. The spatial index prunes to a conservative
        // superset (the exact tests below re-filter); the linear reference
        // takes everything.
        let (point_ids, segment_ids, face_ids) = match index {
            Some(ix) => (
                ix.points_in_cone(origin, dir, tan_aperture),
                ix.segments_in_cone(origin, dir, tan_aperture),
                ix.faces_crossing_ray(origin, dir),
            ),
            None => (
                (0..self.points.len()).collect::<Vec<_>>(),
                (0..self.segments.len()).collect::<Vec<_>>(),
                (0..self.faces.len()).collect::<Vec<_>>(),
            ),
        };

        // Collect all candidates that fall inside the pick cone.
        // Tuple: (kind, angular_dist, depth, position, source, direction)
        let mut candidates: Vec<Candidate> = Vec::new();

        // --- Endpoint candidates: from ScenePoints ---
        for &pi in &point_ids {
            let sp = &self.points[pi];
            if let Some((ang, depth)) = cone_test(origin, dir, sp.position, aperture) {
                candidates.push((
                    SnapKind::Endpoint,
                    ang,
                    depth,
                    sp.position,
                    Some(sp.source),
                    None,
                ));
            }
        }

        // --- Segment candidates: Midpoint and OnEdge ---
        for &si in &segment_ids {
            let seg = &self.segments[si];
            let mid = midpoint(seg.a, seg.b);

            // Midpoint candidate: emitted when the midpoint itself is in the cone.
            if let Some((ang, depth)) = cone_test(origin, dir, mid, aperture) {
                candidates.push((SnapKind::Midpoint, ang, depth, mid, Some(seg.source), None));
            }

            // OnEdge candidate: the closest point on the segment to the ray,
            // if it lies within the cone. Emit even when the midpoint is also
            // in the cone — priority ranking handles "Midpoint beats OnEdge".
            if let Some((pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture) {
                // Skip if this is the same point as the midpoint (it would be
                // a duplicate; the Midpoint candidate already covers it with
                // the stronger kind).
                if !pos.approx_eq(mid, tol::POINT_MERGE) {
                    candidates.push((SnapKind::OnEdge, ang, depth, pos, Some(seg.source), None));
                }
            }
        }

        // --- Sketch and transient segment candidates: Endpoint, Midpoint,
        //     and OnEdge, all with source: None (no ElementRef provenance in
        //     this phase — sketch/transient elements aren't selectable). ---
        let bare_segments = self
            .sketch_segments
            .iter()
            .map(|(_, seg)| seg)
            .chain(self.transient_segments.iter());
        for seg in bare_segments {
            for endpoint in [seg.a, seg.b] {
                if let Some((ang, depth)) = cone_test(origin, dir, endpoint, aperture) {
                    candidates.push((SnapKind::Endpoint, ang, depth, endpoint, None, None));
                }
            }

            let mid = midpoint(seg.a, seg.b);
            if let Some((ang, depth)) = cone_test(origin, dir, mid, aperture) {
                candidates.push((SnapKind::Midpoint, ang, depth, mid, None, None));
            }

            if let Some((pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture)
                && !pos.approx_eq(mid, tol::POINT_MERGE)
            {
                candidates.push((SnapKind::OnEdge, ang, depth, pos, None, None));
            }
        }

        // --- Face candidates: OnFace ---
        for &fi in &face_ids {
            let face = &self.faces[fi];
            if let Some((pos, ang, depth)) = face_cone_hit(
                origin,
                dir,
                &face.plane,
                &face.boundary,
                &face.holes,
                aperture,
            ) {
                candidates.push((SnapKind::OnFace, ang, depth, pos, Some(face.source), None));
            }
        }

        // --- World-origin and world-axis candidates ---
        // The origin snaps as a strong Endpoint; the three world axes snap as
        // OnAxis (weakest meaningful kind, so object geometry still wins).
        // Suppressed when axes are hidden (View ▸ Axes off) so a hidden axis
        // never snaps or flashes a cue.
        if self.axes_enabled {
            if let Some((ang, depth)) = cone_test(origin, dir, Point3::ORIGIN, aperture) {
                candidates.push((SnapKind::Endpoint, ang, depth, Point3::ORIGIN, None, None));
            }
            for axis in [Axis::X, Axis::Y, Axis::Z] {
                let adir = axis.unit();
                let pos = closest_point_on_line_to_ray(Point3::ORIGIN, adir, origin, dir);
                if let Some((ang, depth)) = cone_test(origin, dir, pos, aperture) {
                    candidates.push((SnapKind::OnAxis, ang, depth, pos, None, Some(adir)));
                }
            }
        }

        // --- Construction guide candidates ---
        // A guide point is a precise snap (Endpoint-tier, like the world
        // origin); a guide line snaps as OnGuide, carrying its direction for
        // the dashed cue exactly like an axis snap. Source is always None —
        // guides carry no topology to highlight. Suppressed when guides are
        // hidden (View ▸ Guides off) so a hidden guide never snaps or cues.
        if self.guides_enabled {
            for guide in &self.guides {
                match guide.geom {
                    SceneGuideGeom::Point { position } => {
                        if let Some((ang, depth)) = cone_test(origin, dir, position, aperture) {
                            candidates.push((SnapKind::Endpoint, ang, depth, position, None, None));
                        }
                    }
                    SceneGuideGeom::Line {
                        origin: go,
                        direction: gd,
                    } => {
                        let pos = closest_point_on_line_to_ray(go, gd, origin, dir);
                        if let Some((ang, depth)) = cone_test(origin, dir, pos, aperture) {
                            candidates.push((SnapKind::OnGuide, ang, depth, pos, None, Some(gd)));
                        }
                    }
                }
            }
        }

        // --- Constrain to the active drawing plane, if any. ---
        // Drawing on a face must not snap to occluded, off-plane geometry: the
        // pick cone is a screen-space projection that "sees through" the solid,
        // so without this a hidden bottom-edge midpoint (which outranks OnFace)
        // would win. Keep only candidates lying on the plane; this also collapses
        // OnFace to the coplanar (active) face. The lock-fallback line below is
        // intentionally NOT constrained — it's a directional inference, not a
        // candidate snap.
        if let Some(plane) = query.constraint_plane {
            candidates.retain(|c| plane.signed_distance(c.3).abs() <= tol::PLANE_DIST);
        }

        // --- Rank: strongest SnapKind first, then smallest angular distance,
        //     then nearest ray origin (smallest depth). ---
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        });

        // --- Occlusion cull: walk the ranked list and take the first candidate
        //     that isn't hidden behind an opaque face. A solid must not let a
        //     draw/select snap "see through" it to a higher-priority back edge
        //     or vertex — only what's visible from the eye should snap. Lazy by
        //     design: usually the top candidate is visible, so this costs one
        //     visibility test. The lock-fallback line below is a *directional*
        //     inference, not a candidate, so it is intentionally never culled. ---
        let winner = candidates
            .iter()
            .copied()
            .find(|c| !self.is_occluded(origin, c.3, index));

        // TRACE only — `resolve` runs on every pointer move, so this is a
        // firehose filtered out by default; raise the capture level to debug a
        // bad snap (the inference winner + candidate count,  / docs/DEVELOPMENT.md).
        tracing::trace!(
            target: "inference::resolve",
            candidates = candidates.len(),
            winner = ?winner.as_ref().map(|c| c.0),
        );

        // --- Handle locking ---
        match (query.lock, query.anchor) {
            (Some(lock), Some(anchor)) => {
                // Build the normalized lock direction.
                let lock_dir_raw = match lock {
                    SnapLock::Axis(axis) => axis.unit(),
                    SnapLock::Direction(v) => v,
                };
                let lock_dir = match lock_dir_raw.normalized() {
                    Ok(d) => d,
                    Err(_) => return None,
                };

                if let Some((kind, _ang, _depth, pos, source, _cdir)) = winner.as_ref() {
                    // A candidate snapped: project its position onto the locked line.
                    let projected = project_onto_line(anchor, lock_dir, *pos);
                    Some(Snap {
                        position: projected,
                        kind: *kind,
                        source: *source,
                        direction: Some(lock_dir),
                    })
                } else {
                    // Nothing snapped: intersect the locked line with the pick ray
                    // (closest point between the two lines).
                    let locked_pos = closest_point_on_line_to_ray(anchor, lock_dir, origin, dir);
                    Some(Snap {
                        position: locked_pos,
                        kind: SnapKind::OnAxis,
                        source: None,
                        direction: Some(lock_dir),
                    })
                }
            }
            _ => {
                // No lock (or lock with no anchor): return the top-ranked
                // candidate that is actually visible (see the occlusion cull above).
                winner.map(|(kind, _ang, _depth, pos, source, snap_dir)| Snap {
                    position: pos,
                    kind,
                    source,
                    direction: snap_dir,
                })
            }
        }
    }

    /// Picks the nearest face the ray passes *through* — face selection for
    /// tools like push/pull, distinct from [`resolve`](Self::resolve).
    ///
    /// Unlike `resolve`, this ignores the snap-priority model and the pick
    /// cone entirely: a face is a candidate iff the ray actually crosses its
    /// boundary polygon (in front of the origin), and the nearest such face
    /// wins. The drawing snap prefers endpoints/edges, so it is the wrong tool
    /// for "what surface is under the cursor"; this is the right one. Returns
    /// the face's [`SnapSource`], or `None` if the ray hits no face.
    pub fn pick_face(&self, ray: &PickRay) -> Option<SnapSource> {
        let index = self.spatial_index();
        self.pick_face_impl(ray, Some(&index))
    }

    /// Reference implementation of [`pick_face`](Self::pick_face) with the
    /// spatial index bypassed, mirroring
    /// [`resolve_linear`](Self::resolve_linear) (DEVELOPMENT.md rule 3). Not
    /// part of the supported API.
    #[doc(hidden)]
    pub fn pick_face_linear(&self, ray: &PickRay) -> Option<SnapSource> {
        self.pick_face_impl(ray, None)
    }

    /// Shared body of `pick_face`/`pick_face_linear`. The indexed superset
    /// is scanned in ascending order with the same strict `<` depth
    /// comparison, so equal-depth ties resolve to the lowest candidate
    /// index on both paths.
    fn pick_face_impl(&self, ray: &PickRay, index: Option<&SceneIndex>) -> Option<SnapSource> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        let face_ids: Vec<usize> = match index {
            Some(ix) => ix.faces_crossing_ray(origin, dir),
            None => (0..self.faces.len()).collect(),
        };
        let mut best: Option<(f64, SnapSource)> = None;
        for &fi in &face_ids {
            let face = &self.faces[fi];
            // `face_cone_hit` ignores its aperture arg for faces (a face hit
            // is pure ray-polygon containment), so any value works here.
            if let Some((_pos, _ang, depth)) =
                face_cone_hit(origin, dir, &face.plane, &face.boundary, &face.holes, 0.0)
                && best.as_ref().is_none_or(|(d, _)| depth < *d)
            {
                best = Some((depth, face.source));
            }
        }
        best.map(|(_, source)| source)
    }

    /// Visibility test for snap occlusion: is `pos` hidden behind an opaque
    /// face, as seen from `origin`?
    ///
    /// The pick cone is a screen-space projection that "sees through" solids,
    /// so without this a snap candidate on the *far* side of a solid (a hidden
    /// back edge/vertex, or the interior of a back face) — which can outrank
    /// the visible front face on [`SnapKind`] priority alone — would win. We
    /// cast the ray `origin -> pos` and treat `pos` as occluded iff some face
    /// crosses that ray strictly nearer than `pos` itself.
    ///
    /// A [`tol::OCCLUSION_REL`] skin keeps the face `pos` lies on (and faces
    /// sharing its edge) from self-occluding it: those are hit at depth ≈ the
    /// distance to `pos`, not nearer. A ray that passes through a face's *hole*
    /// is not occluded by that face (`face_cone_hit` already rejects holes), so
    /// snaps seen through an imprinted opening stay visible.
    fn is_occluded(&self, origin: Point3, pos: Point3, index: Option<&SceneIndex>) -> bool {
        let to_pos = pos - origin;
        let dist = to_pos.length();
        let dir = match to_pos.normalized() {
            Ok(d) => d,
            Err(_) => return false, // candidate at the eye — nothing can occlude it
        };
        let near_threshold = dist * (1.0 - tol::OCCLUSION_REL);
        // The exact hole-aware test, shared verbatim by the indexed and
        // linear paths. The counter feeds `occlusion_face_tests` — the
        // introspection the perf-sanity spec uses to prove the index prunes.
        let occludes = |face: &SceneFace| {
            self.occlusion_tests.set(self.occlusion_tests.get() + 1);
            // `face_cone_hit` ignores its aperture arg for faces (pure
            // ray-polygon containment); 0.0 is fine.
            face_cone_hit(origin, dir, &face.plane, &face.boundary, &face.holes, 0.0)
                .is_some_and(|(_pos, _ang, depth)| depth < near_threshold)
        };
        match index {
            // Early-out walk: only subtrees whose boxes the ray enters
            // nearer than the threshold can hold an occluder, and the walk
            // stops at the first face that actually occludes.
            Some(ix) => {
                ix.any_face_hit_before(origin, dir, near_threshold, |fi| occludes(&self.faces[fi]))
            }
            None => self.faces.iter().any(occludes),
        }
    }

    /// Picks the live sketch whose nearest edge is closest to the ray, for
    /// whole-sketch selection of a free-standing (not-yet-extruded) sketch.
    ///
    /// Unlike `pick_face`, a sketch edge has no thickness, so this uses the
    /// same pick-cone model as [`InferenceScene::resolve`]: a sketch is a
    /// candidate iff some point on one of its edges falls within `aperture`
    /// radians of the ray axis, and among candidates the one with the
    /// smallest angular distance wins (depth breaks ties, nearest to the ray
    /// origin first) — mirroring `OnEdge` ranking in `resolve`. Registered
    /// transient segments (no owning sketch) are not candidates here.
    /// Returns `None` if the ray hits no live sketch edge within `aperture`.
    pub fn pick_sketch(&self, ray: &PickRay, aperture: f64) -> Option<SketchId> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        let mut best: Option<(f64, f64, SketchId)> = None; // (angular_dist, depth, id)
        for &(id, seg) in &self.sketch_segments {
            if let Some((_pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture)
                && best.as_ref().is_none_or(|&(a, d, _)| (ang, depth) < (a, d))
            {
                best = Some((ang, depth, id));
            }
        }
        best.map(|(_, _, id)| id)
    }

    /// Picks the committed sketch *vertex* nearest the ray (Phase D per-vertex
    /// edit). Uses the same pick-cone model as [`InferenceScene::pick_sketch`]
    /// but tests vertex points (via [`cone_test`]) rather than edges, returning
    /// the owning sketch, the exact `SketchVertexId`, and its world position.
    /// Smallest angular distance wins; depth breaks ties (nearest first).
    /// Returns `None` if no registered sketch vertex falls within `aperture`.
    pub fn pick_sketch_vertex(
        &self,
        ray: &PickRay,
        aperture: f64,
    ) -> Option<(SketchId, SketchVertexId, Point3)> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        // (angular_dist, depth, id, vertex, position)
        let mut best: Option<(f64, f64, SketchId, SketchVertexId, Point3)> = None;
        for &(id, vid, pos) in &self.sketch_vertices {
            if let Some((ang, depth)) = cone_test(origin, dir, pos, aperture)
                && best
                    .as_ref()
                    .is_none_or(|&(a, d, _, _, _)| (ang, depth) < (a, d))
            {
                best = Some((ang, depth, id, vid, pos));
            }
        }
        best.map(|(_, _, id, vid, pos)| (id, vid, pos))
    }
}

/// Internal candidate tuple used inside `resolve`:
/// `(kind, angular_dist, depth, position, source, direction)`.
type Candidate = (SnapKind, f64, f64, Point3, Option<SnapSource>, Option<Vec3>);

// ---------------------------------------------------------------------------
// Geometry helpers (crate-private)
// ---------------------------------------------------------------------------

/// Returns `(angular_distance_radians, depth)` if `point` is inside the pick
/// cone (in front of the ray and within `aperture` radians of the ray axis),
/// otherwise `None`.
///
/// `dir` must already be normalized.
fn cone_test(origin: Point3, dir: Vec3, point: Point3, aperture: f64) -> Option<(f64, f64)> {
    let to_point = point - origin;
    let depth = to_point.dot(dir); // signed distance along ray
    if depth <= 0.0 {
        return None; // behind the ray origin
    }
    let dist_sq = to_point.length_squared();
    if dist_sq < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        // Point is essentially at the ray origin; treat as angle 0.
        return Some((0.0, depth));
    }
    // cos(angle) = depth / dist; angle = acos(depth / dist).
    let cos_angle = (depth / dist_sq.sqrt()).min(1.0);
    let angle = cos_angle.acos();
    if angle <= aperture {
        Some((angle, depth))
    } else {
        None
    }
}

/// Returns the midpoint of a segment.
fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5)
}

/// Finds the closest point on the segment [a, b] to the pick ray, and returns
/// `(position, angular_distance, depth)` if that point lies within `aperture`.
///
/// `dir` must already be normalized.
fn segment_cone_hit(
    origin: Point3,
    dir: Vec3,
    a: Point3,
    b: Point3,
    aperture: f64,
) -> Option<(Point3, f64, f64)> {
    // Closest point between two lines (ray and segment-as-line), then clamp
    // to the segment [0, 1].
    //
    // Ray:     P(t) = origin + t * dir        (t >= 0 for in-front)
    // Segment: Q(s) = a + s * seg_dir         (s in [0, 1])
    let seg_dir = b - a;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        // Degenerate segment (endpoints coincide); treat as a point.
        return cone_test(origin, dir, a, aperture).map(|(ang, depth)| (a, ang, depth));
    }

    let w = origin - a;
    let b_coef = dir.dot(seg_dir); // dot(ray_dir, seg_dir)

    // denom = |dir|^2 * |seg_dir|^2 - (dir . seg_dir)^2, but |dir|=1 so:
    //       = seg_len_sq - b_coef^2
    let denom = seg_len_sq - b_coef * b_coef;

    // s on the segment line (unclamped), then clamped to [0, 1].
    let s_unclamped = if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        // Lines are parallel; closest point is at s=0 (endpoint a).
        0.0_f64
    } else {
        let e = dir.dot(w); // dir · (origin - a)
        let f = seg_dir.dot(w); // seg_dir · (origin - a)
        // Segment parameter of the closest point between the (unit-direction)
        // ray and the segment line: s = (f - (dir·seg_dir)(dir·w)) / denom.
        // (The earlier `seg_len_sq * e - b_coef * f` form was the ray
        // parameter's numerator and clamped to the wrong endpoint — caught by
        // segment_closest_point_clamps_to_endpoints.)
        (f - b_coef * e) / denom
    };
    let s = s_unclamped.clamp(0.0, 1.0);

    let closest_on_seg = a + seg_dir * s;
    cone_test(origin, dir, closest_on_seg, aperture)
        .map(|(ang, depth)| (closest_on_seg, ang, depth))
}

/// Ray-face intersection: returns `(position, angular_distance, depth)` if
/// the ray hits the face plane in front of the origin and the hit point lies
/// inside the boundary polygon.
///
/// `dir` must already be normalized.
fn face_cone_hit(
    origin: Point3,
    dir: Vec3,
    plane: &Plane,
    boundary: &[Point3],
    holes: &[Vec<Point3>],
    _aperture: f64,
) -> Option<(Point3, f64, f64)> {
    let n = plane.normal();
    let denom = n.dot(dir);
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None; // ray is parallel to the plane
    }
    // t = (offset - n·origin) / (n·dir)
    let t = -plane.signed_distance(origin) / denom;
    if t <= 0.0 {
        return None; // intersection is behind the ray origin
    }
    let hit = origin + dir * t;

    // Point-in-polygon test: project to plane's local 2D axes.
    if !point_in_polygon(hit, boundary, n) {
        return None;
    }
    // Reject hits that land inside a hole: the ray passes through the opening,
    // not the face material (e.g. the annular parent of an imprinted sub-face).
    if holes.iter().any(|hole| point_in_polygon(hit, hole, n)) {
        return None;
    }

    // The depth is t (already the ray parameter with normalized dir).
    // For an on-face snap, the angular distance from the ray axis is 0
    // (the ray goes through the hit point by definition). Use t as depth.
    Some((hit, 0.0, t))
}

/// 2D point-in-polygon test using ray casting.
///
/// Projects `point` and all `boundary` vertices onto the plane defined by
/// `normal`, using an orthonormal basis derived from `normal`, then runs the
/// standard ray-casting test.
fn point_in_polygon(point: Point3, boundary: &[Point3], normal: Vec3) -> bool {
    if boundary.len() < 3 {
        return false;
    }
    // Build a local 2D basis on the plane.
    let (u, v) = plane_basis(normal);

    // Project into 2D.
    let to2d = |p: Point3| -> (f64, f64) {
        let pv = p.to_vec();
        (u.dot(pv), v.dot(pv))
    };

    let (px, py) = to2d(point);
    let verts: Vec<(f64, f64)> = boundary.iter().map(|&p| to2d(p)).collect();
    let n = verts.len();

    // Standard ray-casting: count crossings of a ray from (px, py) in +x.
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        // Edge i->j crosses the horizontal ray from (px, py) if one endpoint
        // is above and the other at or below py, and the crossing x > px.
        if (yi > py) != (yj > py) {
            let cross_x = xj + (py - yj) * (xi - xj) / (yi - yj);
            if px < cross_x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Constructs an orthonormal basis (u, v) on a plane with unit normal `n`,
/// such that u × v = n (right-handed).
fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    // Pick a helper vector not parallel to n.
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = helper
        .cross(n)
        .normalized()
        .expect("helper is never parallel to a unit normal");
    let v = n.cross(u);
    (u, v)
}

/// Projects `point` onto the line `anchor + t * dir` (dir must be unit).
/// Returns the projected point, which lies exactly on the line.
fn project_onto_line(anchor: Point3, dir: Vec3, point: Point3) -> Point3 {
    let t = (point - anchor).dot(dir);
    anchor + dir * t
}

/// Closest point on the line `line_origin + t * line_dir` to the ray
/// `ray_origin + s * ray_dir`. Returns the point on the line (not the ray).
///
/// If lines are parallel, returns the point on the line closest to
/// `ray_origin`.
fn closest_point_on_line_to_ray(
    line_origin: Point3,
    line_dir: Vec3,
    ray_origin: Point3,
    ray_dir: Vec3,
) -> Point3 {
    // Standard closest-point-between-two-lines derivation.
    let w = line_origin - ray_origin;
    let b = line_dir.dot(ray_dir);
    let denom = 1.0 - b * b;
    if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
        // Lines are parallel: project ray_origin onto the line.
        return project_onto_line(line_origin, line_dir, ray_origin);
    }
    let d = line_dir.dot(w);
    let e = ray_dir.dot(w);
    let t = (b * e - d) / denom;
    line_origin + line_dir * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_kind_priority_is_declaration_order() {
        // Strongest first; tools sort by this.
        assert!(SnapKind::Endpoint < SnapKind::Midpoint);
        assert!(SnapKind::Midpoint < SnapKind::Intersection);
        assert!(SnapKind::Intersection < SnapKind::OnEdge);
        assert!(SnapKind::OnEdge < SnapKind::OnFace);
        assert!(SnapKind::OnFace < SnapKind::OnGuide);
        assert!(SnapKind::OnGuide < SnapKind::OnAxis);
        assert!(SnapKind::OnAxis < SnapKind::Parallel);
        assert!(SnapKind::Parallel < SnapKind::Perpendicular);
    }

    #[test]
    fn axes_are_unit_and_orthogonal() {
        let (x, y, z) = (Axis::X.unit(), Axis::Y.unit(), Axis::Z.unit());
        assert_eq!(x.dot(y), 0.0);
        assert_eq!(y.dot(z), 0.0);
        assert_eq!(x.cross(y), z);
    }

    #[test]
    fn empty_scene_has_no_candidates() {
        assert_eq!(InferenceScene::new().candidate_counts(), (0, 0, 0));
    }

    #[test]
    fn degenerate_ray_direction_resolves_to_none() {
        // A zero-length ray direction has no axis; resolve must return None,
        // never panic in normalize.
        let scene = InferenceScene::new();
        let query = SnapQuery {
            ray: PickRay {
                origin: Point3::ORIGIN,
                direction: Vec3::ZERO,
            },
            anchor: None,
            lock: None,
            aperture: 0.3,
            constraint_plane: None,
        };
        assert!(scene.resolve(&query).is_none());
    }

    #[test]
    fn cone_test_excludes_points_behind_the_ray() {
        let origin = Point3::new(0.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        // Directly ahead: included, angular distance 0.
        let ahead = cone_test(origin, dir, Point3::new(0.0, 0.0, 5.0), 0.3);
        assert!(ahead.is_some());
        assert!(ahead.unwrap().0.abs() < tol::NORMAL_DIRECTION);
        // Directly behind: excluded regardless of how wide the cone is.
        assert!(cone_test(origin, dir, Point3::new(0.0, 0.0, -5.0), 3.0).is_none());
    }

    #[test]
    fn segment_closest_point_clamps_to_endpoints() {
        // Ray parallel to +z offset in x; the segment lies along x at z=5.
        // The closest point to the ray is beyond the segment's b end, so it
        // must clamp to b, not run off the line.
        let origin = Point3::new(10.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let hit = segment_cone_hit(
            origin,
            dir,
            Point3::new(0.0, 0.0, 5.0),
            Point3::new(2.0, 0.0, 5.0),
            3.0,
        );
        let (pos, _ang, depth) = hit.expect("segment is in front and within the wide cone");
        assert!(pos.approx_eq(Point3::new(2.0, 0.0, 5.0), tol::POINT_MERGE));
        assert!((depth - 5.0).abs() < tol::POINT_MERGE);
    }

    #[test]
    fn projection_lands_exactly_on_the_locked_line() {
        // Project an arbitrary point onto the X axis through the origin:
        // the result must be exactly (x, 0, 0).
        let p = project_onto_line(
            Point3::ORIGIN,
            Vec3::new(1.0, 0.0, 0.0),
            Point3::new(3.7, 9.0, -4.0),
        );
        assert_eq!(p.y, 0.0);
        assert_eq!(p.z, 0.0);
        assert!((p.x - 3.7).abs() < tol::POINT_MERGE);
    }

    #[test]
    fn angular_tiebreak_prefers_the_candidate_nearer_the_axis() {
        // Two vertices of a unit cube sit in a wide cone; the one the ray
        // points more directly at must win the equal-kind tiebreak.
        let cube = kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
        // Aim almost straight at the (1,1,1) corner from far along the
        // diagonal; (0,0,0) is also on the cone but much farther off-axis.
        let eye = Point3::new(4.0, 4.0, 4.0);
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: eye,
                    direction: Point3::new(1.0, 1.0, 1.0) - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.6,
                constraint_plane: None,
            })
            .expect("a corner is within the cone");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 1.0, 1.0), tol::POINT_MERGE)
        );
    }

    /// A unit cube as an inference scene, for the constraint-plane tests.
    fn cube_scene() -> InferenceScene {
        let cube = kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
        scene
    }

    /// Drawing on the top face must not "see through" the solid. A ray aimed
    /// into the top face interior whose wide cone *also* catches the hidden
    /// bottom corner (an Endpoint, the strongest kind, nearer the axis than its
    /// top twin) used to dive to that off-plane corner when unconstrained —
    /// the rectangle-on-face abort bug. Two independent guards now prevent it:
    /// occlusion culling (the front face hides the bottom corner from the eye,
    /// so it can't win even unconstrained), AND the constraint plane (which
    /// additionally restricts candidates to the active drawing plane). Assert
    /// BOTH keep the snap on z = 1.
    #[test]
    fn constraint_plane_excludes_occluded_off_plane_snaps() {
        let scene = cube_scene();
        // Straight-down ray entering the top face interior at (0.3, 0.3); the
        // wide cone also catches the hidden bottom corner (0,0,0).
        let ray = PickRay {
            origin: Point3::new(0.3, 0.3, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };

        // Unconstrained: occlusion culls the hidden bottom corner (it sits
        // behind the top face along the ray to it), so the snap stays at z = 1.
        let free = scene
            .resolve(&SnapQuery {
                ray,
                anchor: None,
                lock: None,
                aperture: 0.6,
                constraint_plane: None,
            })
            .expect("something visible in the wide cone");
        assert!(
            free.position.z > 0.5,
            "occlusion must keep the unconstrained snap on the visible top, \
             not dive to the hidden bottom: {:?}",
            free.position
        );

        // Constrained to the top plane: independently keeps the snap on z = 1.
        let top =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let constrained = scene
            .resolve(&SnapQuery {
                ray,
                anchor: None,
                lock: None,
                aperture: 0.6,
                constraint_plane: Some(top),
            })
            .expect("an on-plane candidate (top face / its edges) remains");
        assert!(
            top.signed_distance(constrained.position).abs() <= tol::PLANE_DIST,
            "constrained snap lies on the active plane: {:?}",
            constrained.position
        );
    }

    /// Core occlusion regression (bug report,  era): hovering the
    /// centre of a solid's top face must snap to that visible face, NOT pass
    /// through to a hidden back-side edge/vertex. Mirrors push/pulling a circle
    /// into a faceted cylinder, then drawing/measuring on its top: the dense
    /// far-rim facet edges (`OnEdge`, which outranks `OnFace`) must be culled.
    #[test]
    fn unconstrained_snap_does_not_see_through_to_hidden_back_geometry() {
        let scene = cube_scene();
        // Straight down through the centre of the top face, with a tight cone so
        // only the face interior is in range (the corners are ~13° off-axis,
        // well outside). The bottom face's OnFace candidate (z=0) and the hidden
        // bottom geometry are all occluded by the top, so the visible top face
        // must win — proving the hover lands ON the face, not through it.
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(0.5, 0.5, 4.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                constraint_plane: None,
            })
            .expect("the visible top face is under the cursor");
        assert_eq!(
            snap.kind,
            SnapKind::OnFace,
            "centre-of-face hover must land on the visible face, got {:?} at {:?}",
            snap.kind,
            snap.position
        );
        assert!(
            (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
            "snap must sit on the visible top (z=1), not the hidden bottom: {:?}",
            snap.position
        );
    }

    /// Occlusion must not over-cull: a snap target that is genuinely *visible*
    /// (in front of, or beside, any face) still snaps. Here the top-front edge
    /// of the cube is unobstructed from a front-corner eye, so it wins as
    /// `OnEdge` even though deeper cube faces also fall in the cone.
    #[test]
    fn occlusion_keeps_visible_front_geometry() {
        let scene = cube_scene();
        // Eye straight out in front of the +X face, aimed at its centre. The
        // four corners of that face are visible Endpoints (~13° off-axis); the
        // back face's corners (x=0) are hidden behind the cube. Occlusion must
        // cull the back corners but keep a front corner — so the snap lands on
        // the visible front face (x≈1), never diving to the hidden back.
        let eye = Point3::new(4.0, 0.5, 0.5);
        let target = Point3::new(1.0, 0.5, 0.5);
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: eye,
                    direction: target - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("a visible +X-face corner is in the cone");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "a visible front corner must still snap, got {:?} at {:?}",
            snap.kind,
            snap.position
        );
        assert!(
            (snap.position.x - 1.0).abs() <= tol::POINT_MERGE,
            "snap must stay on the visible front face (x=1), not cull through to the back: {:?}",
            snap.position
        );
    }

    /// A ray through an imprinted hole must still reach the geometry visible
    /// *through* the opening — occlusion uses the same hole-aware ray-face test
    /// as `pick_face`, so a face does not occlude what shows through its hole.
    #[test]
    fn occlusion_ignores_geometry_seen_through_a_hole() {
        let mut cube = unit_cube();
        let top = cube
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
            })
            .map(|(id, _)| id)
            .unwrap();
        // Imprint an inner square, then PULL it down would be ideal, but for a
        // pure-occlusion check we just rely on the annular parent having a hole:
        // a ray down the hole centre is NOT occluded by the parent top face, so
        // the sub-face (coplanar, at z=1) is the visible snap rather than being
        // hidden. (Regression guard that holes punch through occlusion.)
        cube.split_face_inner(
            top,
            &[
                Point3::new(0.25, 0.25, 1.0),
                Point3::new(0.75, 0.25, 1.0),
                Point3::new(0.75, 0.75, 1.0),
                Point3::new(0.25, 0.75, 1.0),
            ],
        )
        .unwrap();
        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);

        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(0.5, 0.5, 4.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                constraint_plane: None,
            })
            .expect("the sub-face seen through the parent's hole is visible");
        assert!(
            (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
            "snap stays on the visible coplanar top (z=1): {:?}",
            snap.position
        );
    }

    /// The constraint plane excludes only *off-plane* geometry — on-plane
    /// vertices/edges/midpoints still snap with their proper kind.
    #[test]
    fn constraint_plane_keeps_on_plane_geometry() {
        let scene = cube_scene();
        // Aim near the top corner (1,1,1) from above-diagonal; with the top plane
        // constraint the corner (an Endpoint, on z=1) must still win.
        let eye = Point3::new(2.0, 2.0, 4.0);
        let top =
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: eye,
                    direction: Point3::new(1.0, 1.0, 1.0) - eye,
                },
                anchor: None,
                lock: None,
                aperture: 0.6,
                constraint_plane: Some(top),
            })
            .expect("the on-plane top corner is still snappable");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 1.0, 1.0), tol::POINT_MERGE)
        );
    }

    /// Builds a unit cube via `from_polygons` (top face has normal +Z).
    fn unit_cube() -> Object {
        kernel::Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(0.0, 0.0, 1.0),
                Point3::new(1.0, 0.0, 1.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 1.0, 1.0),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    #[test]
    fn pick_face_through_a_hole_returns_the_sub_face_not_the_annular_parent() {
        // Imprint a sub-face on a cube's top, then a ray straight down through
        // the inner rectangle must pick the SUB-FACE — not the annular parent,
        // whose outer boundary still contains the point but whose hole does
        // not. (Picking the parent here was the NonManifoldResult push/pull
        // bug: translating a holed face whose hole edges are twinned with the
        // sub-face is non-manifold.)
        let mut cube = unit_cube();
        let top = cube
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane
                    .normal()
                    .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
            })
            .map(|(id, _)| id)
            .unwrap();
        let sub_face = cube
            .split_face_inner(
                top,
                &[
                    Point3::new(0.25, 0.25, 1.0),
                    Point3::new(0.75, 0.25, 1.0),
                    Point3::new(0.75, 0.75, 1.0),
                    Point3::new(0.25, 0.75, 1.0),
                ],
            )
            .unwrap()
            .sub_face;

        let mut scene = InferenceScene::new();
        scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);

        // Straight down through the centre of the inner rectangle.
        let through_hole = scene
            .pick_face(&PickRay {
                origin: Point3::new(0.5, 0.5, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            })
            .expect("the ray crosses the top of the cube");
        assert_eq!(
            through_hole.element,
            ElementRef::Face(sub_face),
            "a ray through the hole picks the sub-face, not the parent"
        );

        // Through the annular ring (clear of the hole): still the parent.
        let through_ring = scene
            .pick_face(&PickRay {
                origin: Point3::new(0.1, 0.1, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            })
            .expect("the ray crosses the annular top");
        assert_eq!(
            through_ring.element,
            ElementRef::Face(top),
            "a ray through the ring picks the annular parent"
        );
    }

    // -----------------------------------------------------------------------
    // FIX A: closest_point_on_line_to_ray sign regression
    // -----------------------------------------------------------------------

    /// The closest point on the Z-axis line to a ray pointing in -X from
    /// (10, 0, 5) must be (0, 0, 5), NOT (0, 0, -5) (which the wrong-sign
    /// formula would return).
    #[test]
    fn closest_point_on_line_to_ray_correct_sign() {
        // Line: Z axis through origin.
        let line_origin = Point3::ORIGIN;
        let line_dir = Vec3::new(0.0, 0.0, 1.0); // unit +Z

        // Ray: starts at (10, 0, 5), points in -X.
        let ray_origin = Point3::new(10.0, 0.0, 5.0);
        let ray_dir = Vec3::new(-1.0, 0.0, 0.0); // unit -X

        let pt = closest_point_on_line_to_ray(line_origin, line_dir, ray_origin, ray_dir);

        // The Z-axis point closest to this ray is at z=5 (same height as the
        // ray origin), i.e. (0, 0, 5). The wrong-sign formula gives (0, 0, -5).
        assert!(
            pt.approx_eq(Point3::new(0.0, 0.0, 5.0), tol::POINT_MERGE),
            "expected (0,0,5) but got {:?}",
            pt
        );
    }

    // -----------------------------------------------------------------------
    // FIX B: world-axis and origin snapping
    // -----------------------------------------------------------------------

    /// A ray that passes near the +X axis (but well away from the origin)
    /// should snap to OnAxis with direction ≈ (1, 0, 0).
    #[test]
    fn resolve_snaps_to_x_axis() {
        let scene = InferenceScene::new(); // empty — no objects

        // Ray origin at (5, 3, 0.05), looking in the -Y direction. The
        // closest approach to the X axis is (5, 0, 0) — well away from the
        // world origin (0,0,0), so the origin Endpoint is not on this ray at
        // all, and the result should be OnAxis along +X.
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(5.0, 3.0, 0.05),
                    direction: Vec3::new(0.0, -1.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("X-axis point (5,0,0) is within the cone");
        assert_eq!(snap.kind, SnapKind::OnAxis, "kind should be OnAxis");
        let dir = snap.direction.expect("OnAxis snap must carry a direction");
        assert!(
            dir.approx_eq(Vec3::new(1.0, 0.0, 0.0), tol::NORMAL_DIRECTION),
            "direction should be +X, got {:?}",
            dir
        );
    }

    /// A ray aimed straight at the world origin should snap to
    /// `SnapKind::Endpoint` at `Point3::ORIGIN`.
    #[test]
    fn resolve_snaps_to_world_origin() {
        let scene = InferenceScene::new();

        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(10.0, 0.0, 0.0),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("origin is directly on the ray");
        assert_eq!(snap.kind, SnapKind::Endpoint, "origin snaps as Endpoint");
        assert!(
            snap.position.approx_eq(Point3::ORIGIN, tol::POINT_MERGE),
            "position should be origin, got {:?}",
            snap.position
        );
    }

    /// Object geometry (Endpoint at a cube vertex) outranks an axis snap even
    /// when the vertex also lies near a world axis.
    #[test]
    fn object_vertex_outranks_axis_snap() {
        // The cube has a vertex at (1, 0, 0) which lies ON the X axis.
        // A ray aimed directly at that vertex must resolve to Endpoint, not OnAxis.
        let scene = cube_scene();

        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(5.0, 0.0, 0.0),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("cube vertex (1,0,0) is on this ray");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "object vertex must beat axis snap, got {:?}",
            snap
        );
        assert!(
            snap.position
                .approx_eq(Point3::new(1.0, 0.0, 0.0), tol::POINT_MERGE),
            "position should be the cube vertex, got {:?}",
            snap.position
        );
    }

    // -----------------------------------------------------------------------
    // : construction guide snapping
    // -----------------------------------------------------------------------

    /// A registered guide line resolves to `SnapKind::OnGuide`, carrying the
    /// guide's direction, at a position on the line near the ray.
    #[test]
    fn resolve_snaps_to_guide_line() {
        let mut scene = InferenceScene::new();
        let guide_dir = Vec3::new(0.0, 1.0, 0.0); // +Y
        let guide = Guide::Line {
            origin: Point3::new(2.0, 0.0, 0.0),
            direction: guide_dir,
        };
        scene.add_guide(GuideId::default(), &guide);
        assert_eq!(scene.guide_count(), 1);

        // Ray crosses the guide line (the Y axis through x=2, z=0) from the side.
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(2.0, 5.0, 3.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("ray passes through the guide line");
        assert_eq!(snap.kind, SnapKind::OnGuide);
        assert_eq!(snap.direction, Some(guide_dir));
        // The snapped position must lie on the guide line (x=2, z=0).
        assert!((snap.position.x - 2.0).abs() < tol::POINT_MERGE);
        assert!(snap.position.z.abs() < tol::POINT_MERGE);
    }

    /// A registered guide point resolves to `SnapKind::Endpoint` at that
    /// point — a guide point is a precise snap, same tier as a real vertex.
    #[test]
    fn resolve_snaps_to_guide_point() {
        let mut scene = InferenceScene::new();
        let guide_pos = Point3::new(3.0, 4.0, 0.0);
        scene.add_guide(
            GuideId::default(),
            &Guide::Point {
                position: guide_pos,
            },
        );

        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(3.0, 4.0, 10.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("ray points straight at the guide point");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.position.approx_eq(guide_pos, tol::POINT_MERGE));
    }

    /// Real object geometry coincident with a guide still wins: Endpoint and
    /// OnEdge both outrank OnGuide.
    #[test]
    fn object_geometry_outranks_coincident_guide() {
        let mut scene = cube_scene();
        // A guide line running along the cube's vertical edge through (1,1,*),
        // which coincides with the cube's edge from (1,1,0) to (1,1,1).
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(1.0, 1.0, 0.0),
                direction: Vec3::new(0.0, 0.0, 1.0),
            },
        );

        // Aim straight at the cube vertex (1,1,1), which also lies exactly on
        // the guide line: the vertex Endpoint must win over OnGuide.
        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(1.0, 1.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("cube vertex and guide line are both on this ray");
        assert_eq!(
            snap.kind,
            SnapKind::Endpoint,
            "a coincident object vertex must outrank the guide, got {:?}",
            snap
        );

        // Aim at the midpoint of that same edge (0.5 up) from a ray that does
        // NOT pass through either vertex (offset to the side and angled), so
        // only the midpoint and the guide line (both within the cone, at
        // very different angles from the vertices) compete.
        let mid_snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(1.5, 1.0, 0.5),
                    direction: Vec3::new(-1.0, 0.0, 0.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                constraint_plane: None,
            })
            .expect("cube edge midpoint and guide line are both on this ray");
        assert_eq!(
            mid_snap.kind,
            SnapKind::Midpoint,
            "a coincident edge midpoint must outrank the guide, got {:?}",
            mid_snap
        );
    }

    /// `constraint_plane` drops an off-plane guide candidate exactly like any
    /// other off-plane candidate.
    #[test]
    fn constraint_plane_drops_off_plane_guide() {
        let mut scene = InferenceScene::new();
        // A horizontal guide line at z=5, well off the z=0 constraint plane.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(0.0, 0.0, 5.0),
                direction: Vec3::new(1.0, 0.0, 0.0),
            },
        );

        let ray = PickRay {
            origin: Point3::new(0.0, 5.0, 5.0),
            direction: Vec3::new(0.0, -1.0, 0.0),
        };

        // Unconstrained: the guide line snaps.
        let free = scene
            .resolve(&SnapQuery {
                ray,
                anchor: None,
                lock: None,
                aperture: 0.3,
                constraint_plane: None,
            })
            .expect("the guide line is on this ray");
        assert_eq!(free.kind, SnapKind::OnGuide);

        // Constrained to the ground plane (z=0): the off-plane guide is
        // filtered out, leaving nothing to snap to.
        let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let constrained = scene.resolve(&SnapQuery {
            ray,
            anchor: None,
            lock: None,
            aperture: 0.3,
            constraint_plane: Some(ground),
        });
        assert!(
            constrained.is_none(),
            "off-plane guide must be filtered out, got {:?}",
            constrained
        );
    }

    /// `remove_guide` is idempotent and unregisters: a removed guide no
    /// longer snaps, and removing an unknown id is a no-op (no panic).
    #[test]
    fn remove_guide_is_idempotent_and_unregisters() {
        let mut scene = InferenceScene::new();
        let id = GuideId::default();
        // Placed well away from the world origin/axes (x=20,y=20) so the
        // tight aperture below can't pick up the ambient origin/axis
        // candidates once the guide itself is removed.
        scene.add_guide(
            id,
            &Guide::Point {
                position: Point3::new(20.0, 20.0, 1.0),
            },
        );
        assert_eq!(scene.guide_count(), 1);

        let query = SnapQuery {
            ray: PickRay {
                origin: Point3::new(20.0, 20.0, 10.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
        };
        assert!(scene.resolve(&query).is_some());

        scene.remove_guide(id);
        assert_eq!(scene.guide_count(), 0);
        let after = scene.resolve(&query);
        assert!(
            after.is_none(),
            "removed guide must no longer snap, got {:?}",
            after
        );

        // Idempotent: removing again (and removing an id that was never
        // registered) must not panic.
        scene.remove_guide(id);
        scene.remove_guide(GuideId::default());
    }

    // -----------------------------------------------------------------------
    // Phase B: sketch + transient candidates
    // -----------------------------------------------------------------------

    /// A registered sketch segment's endpoint and its midpoint each resolve
    /// as a snap along a ray through them, with `source: None`.
    #[test]
    fn sketch_segment_endpoint_and_midpoint_snap() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(0.0, 0.0, 0.0);
        let b = Point3::new(2.0, 0.0, 0.0);
        scene.add_sketch(id, &[(a, b)]);

        // Ray straight at endpoint `b`.
        let endpoint_snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(2.0, 0.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                constraint_plane: None,
            })
            .expect("sketch endpoint is on this ray");
        assert_eq!(endpoint_snap.kind, SnapKind::Endpoint);
        assert!(endpoint_snap.position.approx_eq(b, tol::POINT_MERGE));
        assert!(endpoint_snap.source.is_none());

        // Ray straight at the midpoint (1, 0, 0).
        let mid_snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(1.0, 0.0, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.05,
                constraint_plane: None,
            })
            .expect("sketch midpoint is on this ray");
        assert_eq!(mid_snap.kind, SnapKind::Midpoint);
        assert!(
            mid_snap
                .position
                .approx_eq(Point3::new(1.0, 0.0, 0.0), tol::POINT_MERGE)
        );
        assert!(mid_snap.source.is_none());
    }

    /// `remove_sketch` unregisters a sketch's candidates; removing an unknown
    /// id is a no-op.
    #[test]
    fn remove_sketch_unregisters_its_candidates() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(20.0, 20.0, 0.0);
        let b = Point3::new(22.0, 20.0, 0.0);
        scene.add_sketch(id, &[(a, b)]);

        let query = SnapQuery {
            ray: PickRay {
                origin: Point3::new(20.0, 20.0, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
        };
        assert!(scene.resolve(&query).is_some());

        scene.remove_sketch(id);
        assert!(
            scene.resolve(&query).is_none(),
            "removed sketch must no longer snap"
        );

        // Idempotent / unknown id is a no-op.
        scene.remove_sketch(id);
        scene.remove_sketch(SketchId::default());
    }

    /// `pick_sketch` returns the id of the sketch whose edge the ray passes
    /// nearest to, within the aperture; a ray that hits nothing returns `None`.
    #[test]
    fn pick_sketch_returns_the_nearest_sketch_within_aperture() {
        let mut scene = InferenceScene::new();
        let near = SketchId::default();
        scene.add_sketch(
            near,
            &[(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))],
        );

        let ray = PickRay {
            origin: Point3::new(1.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch(&ray, 0.05), Some(near));

        // A ray well off to the side hits nothing within a tight aperture.
        let miss_ray = PickRay {
            origin: Point3::new(50.0, 50.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch(&miss_ray, 0.05), None);

        scene.remove_sketch(near);
        assert_eq!(
            scene.pick_sketch(&ray, 0.05),
            None,
            "removed sketch is no longer pickable"
        );
    }

    /// `pick_sketch_vertex` returns the exact `SketchVertexId` of the nearest
    /// registered sketch vertex within the aperture, and `None` after removal.
    #[test]
    fn pick_sketch_vertex_returns_the_nearest_vertex() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let mut sk = kernel::Sketch::on_plane(
            Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ])
            .unwrap(),
        );
        sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
            .unwrap();
        let verts: Vec<_> = sk
            .vertices()
            .iter()
            .map(|(vid, v)| (vid, v.position))
            .collect();
        scene.add_sketch_vertices(id, &verts);
        let target = verts
            .iter()
            .find(|(_, p)| p.approx_eq(Point3::new(2.0, 0.0, 0.0), tol::POINT_MERGE))
            .map(|(vid, _)| *vid)
            .unwrap();

        // A ray straight down onto the (2,0,0) corner picks that exact vertex.
        let ray = PickRay {
            origin: Point3::new(2.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        let hit = scene.pick_sketch_vertex(&ray, 0.05).expect("vertex on ray");
        assert_eq!((hit.0, hit.1), (id, target));
        assert!(
            hit.2
                .approx_eq(Point3::new(2.0, 0.0, 0.0), tol::POINT_MERGE)
        );

        // A ray down the middle of the edge (1,0,0) is too far from any vertex.
        let mid_ray = PickRay {
            origin: Point3::new(1.0, 0.0, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        assert_eq!(scene.pick_sketch_vertex(&mid_ray, 0.05), None);

        scene.remove_sketch(id);
        assert_eq!(scene.pick_sketch_vertex(&ray, 0.05), None);
    }

    /// A transient segment's endpoint snaps like a sketch segment's;
    /// `clear_transient` removes it.
    #[test]
    fn transient_segment_endpoint_snaps_and_clears() {
        let mut scene = InferenceScene::new();
        let a = Point3::new(5.0, 5.0, 0.0);
        let b = Point3::new(7.0, 5.0, 0.0);
        scene.add_transient_segment(a, b);

        let query = SnapQuery {
            ray: PickRay {
                origin: Point3::new(7.0, 5.0, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
        };
        let snap = scene
            .resolve(&query)
            .expect("transient endpoint is on this ray");
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(snap.position.approx_eq(b, tol::POINT_MERGE));
        assert!(snap.source.is_none());

        scene.clear_transient();
        assert!(
            scene.resolve(&query).is_none(),
            "cleared transient segment must no longer snap"
        );
    }
}
