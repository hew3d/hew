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
//! # Storage and indexing
//!
//! World-object candidates are baked into world space at registration and
//! pruned through a lazily rebuilt AABB BVH (see `index`) before the exact
//! per-candidate tests run. Component-instance candidates are stored **once
//! per definition member**, in definition space ([`InferenceScene::set_def_member`]);
//! each placement is a lightweight `(instance, member, pose)` record
//! ([`InferenceScene::add_placement`]), resolved through a two-level walk —
//! a top-level BVH over placement world boxes, descending into the member's
//! persistent definition-space tree with pose-mapped node tests. Exact tests
//! always run in world space on `pose.apply_point(definition_position)`, the
//! same computation per-placement baking used to perform, so query results
//! are unchanged — only registration cost (once per member, not once per
//! placement) and memory (one copy per definition) collapse. Constant-count
//! candidates (guides, world axes/origin) and gesture-scoped ones
//! (sketch/transient segments) stay on a linear walk. The API deliberately
//! hides the storage so the index strategy can change without touching
//! callers. Intersection snaps (`SnapKind::Intersection`) are emitted where
//! a guide line crosses a segment (sketch or object edge) or another guide
//! line — the crossing is precisely why the guide was drawn.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    AnalyticRim, EdgeId, FaceId, Guide, GuideId, InstanceId, Object, ObjectId, Plane, Point3,
    SketchCurveRim, SketchEdgeId, SketchId, SketchVertexId, Transform, Vec3, VertexId, tol,
};

mod index;

use index::{DefIndex, SceneIndex};

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
    /// On the true center of a drawn circle or arc, derived from the
    /// solid's analytic surface references (`kernel::SurfaceRef`,
    /// the true-curves design) — the exact drawn center, not a facet
    /// artifact. Ranked just below Endpoint: a real vertex at the same spot
    /// still wins, but a center beats everything derived (midpoints,
    /// intersections, edges, faces).
    Center,
    /// On a quadrant point of a drawn circle or arc's rim — the four
    /// cardinal points of the exact analytic circle, offered only over the
    /// angular range the facets actually cover. Derived from the same
    /// surface references as [`SnapKind::Center`]; ranked with it (a
    /// center at the same spot still wins on order).
    Quadrant,
    /// On the midpoint of an edge.
    Midpoint,
    /// On the apparent intersection of two edges.
    Intersection,
    /// On the point of a drawn circle or arc's rim where the segment from
    /// the tool's anchor is tangent to the exact analytic circle. Needs an
    /// anchor ([`SnapQuery::anchor`]); offered only over the covered
    /// angular range. Beats bare OnEdge (it is a *specific* point of the
    /// rim) but loses to explicit points (endpoints, quadrants, midpoints,
    /// intersections).
    Tangent,
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
    /// Committed-sketch provenance: which sketch edge a Midpoint/OnEdge snap
    /// derives from, when it came from a sketch rather than an Object.
    /// Mutually exclusive with `source`. Lets tools use a sketch edge as a
    /// reference (Tape Measure parallel guides) without object plumbing.
    pub sketch_source: Option<(SketchId, SketchEdgeId)>,
    /// The inference direction for directional snaps (axis / parallel /
    /// perpendicular), for drawing the dashed guide line.
    pub direction: Option<Vec3>,
}

/// Internal candidate provenance: an Object element or a committed sketch
/// edge. Split back into [`Snap::source`] / [`Snap::sketch_source`] when the
/// winning candidate becomes a snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provenance {
    Object(SnapSource),
    SketchEdge(SketchId, SketchEdgeId),
}

impl Provenance {
    /// Split into the two public [`Snap`] provenance fields.
    fn split(this: Option<Provenance>) -> (Option<SnapSource>, Option<(SketchId, SketchEdgeId)>) {
        match this {
            Some(Provenance::Object(s)) => (Some(s), None),
            Some(Provenance::SketchEdge(sid, eid)) => (None, Some((sid, eid))),
            None => (None, None),
        }
    }
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

/// A rim circle of a claimed cylinder in world space, for tangent
/// inference (mirrors [`kernel::AnalyticRim`] through a placement).
#[derive(Debug, Clone, PartialEq)]
struct SceneRim {
    center: Point3,
    /// Unit rim-plane normal (the cylinder axis).
    axis: Vec3,
    radius: f64,
    /// Unit angular-frame basis (perpendicular to `axis`).
    u: Vec3,
    v: Vec3,
    /// Merged coverage intervals in the (u, v) frame; `None` = full circle
    /// (see [`kernel::AnalyticRim::coverage`]). Angles are similarity
    /// invariant, so the object-space intervals apply verbatim.
    coverage: Option<Vec<[f64; 2]>>,
    source: SnapSource,
}

impl SceneRim {
    /// Whether `angle` (radians in the (u, v) frame) is covered, within the
    /// same tolerance rule as [`kernel::AnalyticRim::covers`].
    fn covers(&self, angle: f64) -> bool {
        let Some(intervals) = &self.coverage else {
            return true;
        };
        let eps = tol::POINT_MERGE / self.radius;
        let tau = 2.0 * std::f64::consts::PI;
        let mut a = angle;
        while a >= std::f64::consts::PI {
            a -= tau;
        }
        while a < -std::f64::consts::PI {
            a += tau;
        }
        intervals.iter().any(|&[s, e]| {
            (a >= s - eps && a <= e + eps) || (a + tau >= s - eps && a + tau <= e + eps)
        })
    }

    /// The rim point at `angle` in the (u, v) frame.
    fn point_at(&self, angle: f64) -> Point3 {
        self.center + self.u * (self.radius * angle.cos()) + self.v * (self.radius * angle.sin())
    }
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

/// A definition-space snap point: one member vertex, stored once and shared
/// by every placement (world position = `pose.apply_point(position)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DefPoint {
    pub(crate) position: Point3,
    pub(crate) vertex: VertexId,
}

/// A definition-space snap segment (one member edge; see [`DefPoint`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DefSegment {
    pub(crate) a: Point3,
    pub(crate) b: Point3,
    pub(crate) edge: EdgeId,
}

/// A definition-space snap face (one member face; see [`DefPoint`]). The
/// plane is carried in definition space and mapped per placement via
/// [`Transform::apply_plane`] — the same inverse-transpose-safe path
/// registration-time baking used, so mirrored and non-uniformly scaled
/// placements keep exact parity with the old per-placement storage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefFace {
    pub(crate) plane: Plane,
    pub(crate) boundary: Vec<Point3>,
    pub(crate) holes: Vec<Vec<Point3>>,
    pub(crate) face: FaceId,
}

/// One definition member's shared snap geometry: definition-space
/// candidates extracted once ([`InferenceScene::set_def_member`]) plus the
/// persistent per-member spatial trees. Every placement of the member
/// resolves against this one copy.
#[derive(Debug, Clone)]
pub(crate) struct DefMember {
    pub(crate) points: Vec<DefPoint>,
    pub(crate) segments: Vec<DefSegment>,
    pub(crate) faces: Vec<DefFace>,
    /// Definition-space analytic rims (coverage-bearing only), materialized
    /// per placement at query time (see [`DefMember::rims_at`]) — rims are
    /// few per scene and linear-walked, so placements pay no baking cost and
    /// every removal/replace path stays automatic.
    pub(crate) rims: Vec<AnalyticRim>,
    pub(crate) index: DefIndex,
}

impl DefMember {
    /// Extracts `object`'s vertices/edges/faces into definition-space
    /// candidates — the same element walk [`InferenceScene::register`] does
    /// for world objects, minus the world transform (applied per placement
    /// at query time instead).
    fn extract(object: &Object) -> DefMember {
        let mut points = Vec::new();
        let mut segments = Vec::new();
        let mut faces = Vec::new();
        for (vid, vertex) in object.vertices() {
            points.push(DefPoint {
                position: vertex.position,
                vertex: vid,
            });
        }
        for (eid, edge) in object.edges() {
            let half_edges = object.half_edges();
            let he = &half_edges[edge.half_edge];
            let origin_vid = he.origin;
            let dest_vid = half_edges[he.next].origin;
            segments.push(DefSegment {
                a: object.vertices()[origin_vid].position,
                b: object.vertices()[dest_vid].position,
                edge: eid,
            });
        }
        for (fid, face) in object.faces() {
            let boundary: Vec<Point3> = object.loop_positions(face.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = face
                .inner_loops
                .iter()
                .map(|&lid| object.loop_positions(lid).collect())
                .collect();
            faces.push(DefFace {
                plane: face.plane,
                boundary,
                holes,
                face: fid,
            });
        }
        let index = DefIndex::build(&points, &segments, &faces);
        // Analytic rims, gated on surviving coverage exactly like the world
        // path (`register`): a vacant rim offers no candidates at all.
        let rims = object
            .analytic_rims()
            .into_iter()
            .filter(AnalyticRim::has_coverage)
            .collect();
        DefMember {
            points,
            segments,
            faces,
            rims,
            index,
        }
    }

    /// Materializes one placed point into the world-space candidate the old
    /// per-placement baking would have stored: the identical
    /// `apply_point` on the identical inputs, so positions are bit-equal.
    fn point_at(&self, li: usize, pl: &Placement) -> ScenePoint {
        let p = &self.points[li];
        ScenePoint {
            position: pl.pose.apply_point(p.position),
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Vertex(p.vertex),
                instance: Some(pl.instance),
            },
        }
    }

    /// Materializes one placed segment (see [`DefMember::point_at`]).
    fn segment_at(&self, li: usize, pl: &Placement) -> SceneSegment {
        let s = &self.segments[li];
        SceneSegment {
            a: pl.pose.apply_point(s.a),
            b: pl.pose.apply_point(s.b),
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Edge(s.edge),
                instance: Some(pl.instance),
            },
        }
    }

    /// Materializes this member's analytic-rim candidates under one
    /// placement, pushing them into the caller's per-query vectors: the
    /// identical center/quadrant mapping the world path
    /// ([`InferenceScene::register`]) bakes at registration time, and the
    /// same map-or-drop similarity gate for the tangent rims — so a placed
    /// cylinder snaps exactly like the world object it instances.
    fn rims_at(
        &self,
        pl: &Placement,
        centers: &mut Vec<ScenePoint>,
        quadrants: &mut Vec<ScenePoint>,
        rims: &mut Vec<SceneRim>,
    ) {
        let similarity = pl.pose.similarity_scale();
        for rim in &self.rims {
            let source = SnapSource {
                object: pl.member,
                element: ElementRef::Face(rim.rep),
                instance: Some(pl.instance),
            };
            centers.push(ScenePoint {
                position: pl.pose.apply_point(rim.center),
                source,
            });
            // Quadrant points transform as plain points: under any affine
            // pose they stay on the (possibly elliptical) rim curve.
            for q in rim.quadrant_points() {
                quadrants.push(ScenePoint {
                    position: pl.pose.apply_point(q),
                    source,
                });
            }
            // Tangency needs a genuine circle: map the rim under
            // similarities, drop it otherwise (never approximate).
            if let Some(scale) = similarity {
                let map_unit = |w: Vec3| pl.pose.apply_vector(w).normalized();
                if let (Ok(axis), Ok(u), Ok(v)) = (
                    map_unit(rim.axis),
                    map_unit(rim.basis_u),
                    map_unit(rim.basis_v),
                ) {
                    rims.push(SceneRim {
                        center: pl.pose.apply_point(rim.center),
                        axis,
                        radius: rim.radius * scale,
                        u,
                        v,
                        coverage: rim.coverage.clone(),
                        source,
                    });
                }
            }
        }
    }

    /// Materializes one placed face (see [`DefMember::point_at`]), or `None`
    /// for a singular pose — the same faces registration-time baking used to
    /// skip, so both storage schemes emit the same candidate set.
    fn face_at(&self, li: usize, pl: &Placement) -> Option<SceneFace> {
        let f = &self.faces[li];
        let plane = pl.pose.apply_plane(&f.plane).ok()?;
        Some(SceneFace {
            plane,
            boundary: f.boundary.iter().map(|&p| pl.pose.apply_point(p)).collect(),
            holes: f
                .holes
                .iter()
                .map(|hole| hole.iter().map(|&p| pl.pose.apply_point(p)).collect())
                .collect(),
            source: SnapSource {
                object: pl.member,
                element: ElementRef::Face(f.face),
                instance: Some(pl.instance),
            },
        })
    }
}

/// One placement of a definition member: everything an instance contributes
/// to the scene, now that the geometry itself is shared (see [`DefMember`]).
/// Registration order is the candidate enumeration order for placements, so
/// it participates in ranking tie-breaks; callers drive registration from
/// deterministic document iteration, keeping resolve results reproducible.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) instance: InstanceId,
    pub(crate) member: ObjectId,
    pub(crate) pose: Transform,
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
    /// True circle centers derived from objects' analytic surface
    /// references (see [`SnapKind::Center`]): few per scene, registered
    /// alongside `points` and resolved on a linear walk (never indexed).
    centers: Vec<ScenePoint>,
    /// Rim quadrant points (see [`SnapKind::Quadrant`]): the covered
    /// cardinal points of each claimed cylinder's two rim circles. Same
    /// lifecycle and linear-walk resolution as `centers`.
    quadrants: Vec<ScenePoint>,
    /// Rim circles for tangent inference (see [`SnapKind::Tangent`]):
    /// exact center/axis/radius plus angular coverage, in world space.
    /// Registered only for placements that preserve circles (world objects
    /// and similarity-posed instances) — a non-uniformly scaled instance's
    /// rims are ellipses the reference cannot represent, so they are
    /// dropped, never approximated. Linear walk, like `centers`.
    rims: Vec<SceneRim>,
    guides: Vec<SceneGuide>,
    /// Persistent sketch candidates (committed sketch edges, not yet kernel
    /// Objects): keyed by `SketchId` so a caller can replace one sketch's
    /// segments without touching another's. No `SnapSource` provenance —
    /// sketch elements aren't selectable in this phase.
    sketch_segments: Vec<(SketchId, SketchEdgeId, BareSegment)>,
    /// Committed sketch *vertices*, keyed by `SketchId`, carrying their
    /// `SketchVertexId` so the per-vertex edit tool (Phase D) can pick an exact
    /// vertex to drag. Registered/cleared alongside `sketch_segments`.
    sketch_vertices: Vec<(SketchId, SketchVertexId, Point3)>,
    /// Committed sketch *curve rims* (drawn circles/arcs carrying an
    /// analytic [`kernel::CurveGeom`]), keyed by `SketchId` like
    /// `sketch_segments`: each offers its exact center, covered quadrant
    /// points, and anchor-based tangents — the sketch-level analogue of
    /// `centers`/`quadrants`/`rims`, resolved on the same linear walk, but
    /// with no provenance (sketch curves aren't `SnapSource` elements; like
    /// guides they snap with `source: None`). Registered/cleared alongside
    /// `sketch_segments`, so a drawn circle's true center snaps BEFORE any
    /// extrusion exists.
    sketch_rims: Vec<(SketchId, SketchCurveRim)>,
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
    /// [`InferenceScene::add_placement`] — exactly the ids for which
    /// placements can exist (across every definition member the instance
    /// places). Same fast path for [`InferenceScene::remove_instance`].
    instance_owners: BTreeSet<InstanceId>,
    /// Member ids for which placements may exist (inserted by
    /// [`InferenceScene::add_placement`]) — the fast path for
    /// [`InferenceScene::remove_def_member`], which reconcile calls for
    /// EVERY touched non-world object: without it each never-registered id
    /// pays a full placement scan, the same accidental quadratic the other
    /// owner sets exist to prevent. Like `instance_owners`, entries may be
    /// stale ("may exist", not "exist") — a stale entry costs one wasted
    /// retain pass, never a wrong skip.
    member_owners: BTreeSet<ObjectId>,
    /// Cumulative count of candidates walked by the removal retain passes
    /// (see [`InferenceScene::removal_candidates_visited`]). Plain `u64`
    /// (not `Cell`) because removal takes `&mut self`. Instance removal no
    /// longer visits candidates at all — placements are records, not
    /// candidate spans — so only world-object removal contributes.
    removal_visits: u64,
    /// Definition-space snap geometry, one entry per registered definition
    /// member, shared by every placement of that member. Survives
    /// [`InferenceScene::clear_solids`] (visibility changes never touch
    /// definition geometry); a caller switching documents must start from a
    /// fresh scene, since member ids from another document could collide.
    /// `BTreeMap` (not a hash map) per the determinism rule
    /// (DEVELOPMENT.md §7).
    def_members: BTreeMap<ObjectId, DefMember>,
    /// Live placements, in registration order (the enumeration order for
    /// placed candidates — see [`Placement`]).
    placements: Vec<Placement>,
    /// Cumulative count of definition-member extraction passes (see
    /// [`InferenceScene::def_extractions`]).
    def_extractions: u64,
}

impl Default for InferenceScene {
    fn default() -> Self {
        InferenceScene {
            points: Vec::new(),
            segments: Vec::new(),
            faces: Vec::new(),
            centers: Vec::new(),
            quadrants: Vec::new(),
            rims: Vec::new(),
            guides: Vec::new(),
            sketch_segments: Vec::new(),
            sketch_vertices: Vec::new(),
            sketch_rims: Vec::new(),
            transient_segments: Vec::new(),
            guides_enabled: true,
            axes_enabled: true,
            spatial: RefCell::new(None),
            occlusion_tests: Cell::new(0),
            world_owners: BTreeSet::new(),
            instance_owners: BTreeSet::new(),
            member_owners: BTreeSet::new(),
            removal_visits: 0,
            def_members: BTreeMap::new(),
            placements: Vec::new(),
            def_extractions: 0,
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
    /// for tests and debug overlays. Counts what queries can *see*: world
    /// candidates plus each placement's share of its member's candidates
    /// (definition geometry with no live placement contributes nothing).
    pub fn candidate_counts(&self) -> (usize, usize, usize) {
        let (mut p, mut s, mut f) = (self.points.len(), self.segments.len(), self.faces.len());
        for pl in &self.placements {
            if let Some(m) = self.def_members.get(&pl.member) {
                p += m.points.len();
                s += m.segments.len();
                // Placed faces resolve through `apply_plane`, which refuses
                // singular poses (`face_at` → None) — mirror its determinant
                // gate so a degenerate placement's faces aren't counted as
                // visible.
                if pl.pose.determinant().abs() >= tol::NORMALIZE_MIN_LENGTH {
                    f += m.faces.len();
                }
            }
        }
        (p, s, f)
    }

    /// Cumulative number of definition-member extraction passes performed by
    /// [`InferenceScene::set_def_member`] across the scene's lifetime —
    /// cheap introspection for tests and debug overlays, like
    /// [`InferenceScene::occlusion_face_tests`]. Shared definition storage
    /// exists to make this scale with *definitions*, not placements: the
    /// registration perf-sanity spec asserts registering N instances of one
    /// member costs exactly one extraction.
    pub fn def_extractions(&self) -> u64 {
        self.def_extractions
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
            *self.spatial.borrow_mut() = Some(SceneIndex::build(
                &self.points,
                &self.segments,
                &self.faces,
                &self.placements,
                &self.def_members,
            ));
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
        self.register(object, placement, id);
    }

    /// Registers (or re-registers) the shared definition-space snap geometry
    /// of one component-definition member. Replace semantics: any prior
    /// geometry for `member` is dropped, and existing placements of `member`
    /// resolve against the new copy from the next query on — this is how a
    /// definition edit propagates to every placement in one extraction.
    ///
    /// Cost model: linear in the member's elements (plus the per-member tree
    /// build), paid once per member per geometry change — never per
    /// placement, which is the entire point of the shared storage (see the
    /// module docs). Callers re-registering placements cheaply gate on
    /// [`InferenceScene::has_def_member`] to skip this when the geometry is
    /// already current.
    pub fn set_def_member(&mut self, member: ObjectId, object: &Object) {
        self.def_members.insert(member, DefMember::extract(object));
        self.def_extractions += 1;
        // Placement-level world boxes derive from the member's class boxes:
        // the top-level index is stale even though no placement changed.
        *self.spatial.get_mut() = None;
    }

    /// Whether shared definition geometry is currently registered for
    /// `member` (see [`InferenceScene::set_def_member`]).
    pub fn has_def_member(&self, member: ObjectId) -> bool {
        self.def_members.contains_key(&member)
    }

    /// Drops the shared definition geometry registered for `member`, along
    /// with every placement of it (a placement without geometry can't
    /// snap). Unknown ids are a no-op — callers invalidate freely: the
    /// reconcile path calls this for every touched non-world object, using
    /// removal as the staleness signal that makes the next placement
    /// registration re-extract.
    pub fn remove_def_member(&mut self, member: ObjectId) {
        let had_geometry = self.def_members.remove(&member).is_some();
        // Owner-set fast path, mirroring `remove_object`/`remove_instance`:
        // reconcile calls this for every touched non-world id, and a
        // never-registered id must not pay a placement scan (see
        // `member_owners`).
        let may_have_placements = self.member_owners.remove(&member);
        if !had_geometry && !may_have_placements {
            return; // nothing registered: keep the index valid
        }
        self.placements.retain(|p| p.member != member);
        // `instance_owners` deliberately keeps ids whose last placement just
        // vanished: the set means "placements may exist", so a stale entry
        // only costs one wasted retain pass on a later remove_instance.
        *self.spatial.get_mut() = None;
    }

    /// Registers one placement of definition member `member` by `instance`
    /// at `pose`. **Additive** — an instance places every member of its
    /// definition, one placement each; the caller clears an instance's prior
    /// placements with [`InferenceScene::remove_instance`] before
    /// re-registering. The member's geometry should already be registered
    /// ([`InferenceScene::set_def_member`]); a placement of an unregistered
    /// member is inert (it emits no candidates) until the geometry arrives.
    pub fn add_placement(&mut self, instance: InstanceId, member: ObjectId, pose: &Transform) {
        debug_assert!(
            self.has_def_member(member),
            "register the member's geometry before placing it"
        );
        // Record the owners (idempotent): placements for `instance` and for
        // `member` now exist, which is exactly what `remove_instance`'s and
        // `remove_def_member`'s fast paths key on.
        self.instance_owners.insert(instance);
        self.member_owners.insert(member);
        self.placements.push(Placement {
            instance,
            member,
            pose: *pose,
        });
        *self.spatial.get_mut() = None;
    }

    /// Extracts `object`'s vertices/edges/faces into world-space candidates
    /// owned by `owner` — the [`InferenceScene::add_object`] path (world
    /// solids only; instanced geometry lives in shared definition storage,
    /// see [`InferenceScene::set_def_member`]).
    fn register(&mut self, object: &Object, placement: &Transform, owner: ObjectId) {
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
                    instance: None,
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
                    instance: None,
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
                    instance: None,
                },
            });
        }

        // --- Analytic rims -> Center / Quadrant / Tangent candidates ---
        // Derived from the object's surface references
        // (the true-curves design): each claimed cylinder's two rim
        // circles yield a Center at the exact axis point, Quadrant points
        // over the covered angular range, and — when the placement
        // preserves circles — the rim itself for anchor-based tangent
        // resolution at query time.
        let similarity = placement.similarity_scale();
        for rim in object.analytic_rims() {
            // A rim with zero surviving arc (a slant-cut station) offers no
            // candidates AT ALL: its center would be the center of no
            // surviving circle, its quadrant set is empty by construction,
            // and tangency has no arc to touch. Same gate as the kernel's
            // own `analytic_cap_centers`.
            if !rim.has_coverage() {
                continue;
            }
            let source = SnapSource {
                object: owner,
                element: ElementRef::Face(rim.rep),
                instance: None,
            };
            self.centers.push(ScenePoint {
                position: placement.apply_point(rim.center),
                source,
            });
            // Quadrant points transform as plain points: under any affine
            // placement they stay on the (possibly elliptical) rim curve.
            for q in rim.quadrant_points() {
                self.quadrants.push(ScenePoint {
                    position: placement.apply_point(q),
                    source,
                });
            }
            // Tangency needs a genuine circle: map the rim under
            // similarities, drop it otherwise (never approximate — the
            // kernel's map-or-drop rule, applied at the query layer).
            if let Some(scale) = similarity {
                let map_unit = |w: Vec3| placement.apply_vector(w).normalized();
                if let (Ok(axis), Ok(u), Ok(v)) = (
                    map_unit(rim.axis),
                    map_unit(rim.basis_u),
                    map_unit(rim.basis_v),
                ) {
                    self.rims.push(SceneRim {
                        center: placement.apply_point(rim.center),
                        axis,
                        radius: rim.radius * scale,
                        u,
                        v,
                        coverage: rim.coverage.clone(),
                        source,
                    });
                }
            }
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
        self.removal_visits += (self.points.len()
            + self.segments.len()
            + self.faces.len()
            + self.centers.len()
            + self.quadrants.len()
            + self.rims.len()) as u64;
        let world = |s: &SnapSource| s.object == id && s.instance.is_none();
        self.points.retain(|p| !world(&p.source));
        self.segments.retain(|s| !world(&s.source));
        self.faces.retain(|f| !world(&f.source));
        self.centers.retain(|c| !world(&c.source));
        self.quadrants.retain(|c| !world(&c.source));
        self.rims.retain(|r| !world(&r.source));
    }

    /// Drops all placements registered for `instance` (across every
    /// definition member it places); the member geometry itself stays, since
    /// other instances (or a later re-registration) share it. Idempotent, so
    /// document undo can call it freely — and removal never scans
    /// *candidates* at all (see
    /// [`InferenceScene::removal_candidates_visited`]): placements are
    /// lightweight records, so even the non-empty case is one retain pass
    /// over the placement Vec, not the candidate storage.
    pub fn remove_instance(&mut self, instance: InstanceId) {
        // Owner-set fast path, mirroring `remove_object`: placements for
        // `instance` exist only for ids in `instance_owners`
        // (`add_placement` inserts before pushing), so a never-registered id
        // has nothing to remove — return without touching the still-valid
        // spatial index.
        if !self.instance_owners.remove(&instance) {
            return;
        }
        self.placements.retain(|p| p.instance != instance);
        *self.spatial.get_mut() = None;
    }

    /// Drops every object-sourced candidate and every placement at once,
    /// leaving guides, sketches, transient segments — and shared definition
    /// geometry — registered. For bulk visibility rebuilds (e.g. applying a
    /// whole hidden set): removing N registered owners one at a time scans
    /// the world-candidate vectors once per owner, while clearing and
    /// re-registering the visible remainder is one linear pass in total —
    /// each re-registration's replace-semantics removal hits the empty-owner
    /// fast path, and each placement re-registration reuses the surviving
    /// definition geometry ([`InferenceScene::has_def_member`]) instead of
    /// re-extracting it. Definition geometry is safe to keep precisely
    /// because visibility changes never alter geometry; callers switching
    /// documents start from a fresh scene (see the `def_members` field
    /// docs).
    pub fn clear_solids(&mut self) {
        *self.spatial.get_mut() = None;
        self.points.clear();
        self.segments.clear();
        self.faces.clear();
        self.centers.clear();
        self.quadrants.clear();
        self.rims.clear();
        self.world_owners.clear();
        self.instance_owners.clear();
        self.member_owners.clear();
        self.placements.clear();
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
    pub fn add_sketch(&mut self, id: SketchId, segments: &[(SketchEdgeId, Point3, Point3)]) {
        self.remove_sketch(id);
        self.sketch_segments.extend(
            segments
                .iter()
                .map(|&(eid, a, b)| (id, eid, BareSegment { a, b })),
        );
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

    /// Registers (or re-registers) the committed *curve rims* of sketch `id`
    /// — the exact circles of its drawn curves ([`kernel::Sketch::curve_rims`])
    /// — as Center/Quadrant/Tangent candidates, so an unextruded circle or
    /// arc snaps at its true center exactly like a solid's rim. Replace
    /// semantics like [`InferenceScene::add_sketch_vertices`]: drops any
    /// prior rims for `id` first. Callers register rims, vertices, and
    /// segments together on every sketch mutation.
    pub fn add_sketch_curves(&mut self, id: SketchId, rims: &[SketchCurveRim]) {
        self.sketch_rims.retain(|(sid, _)| *sid != id);
        self.sketch_rims
            .extend(rims.iter().map(|r| (id, r.clone())));
    }

    /// Drops all candidates registered for sketch `id`. Unknown ids are a
    /// no-op — removal must be idempotent (mirroring
    /// [`InferenceScene::remove_object`]) so callers can remove-then-add
    /// freely.
    pub fn remove_sketch(&mut self, id: SketchId) {
        self.sketch_segments.retain(|(sid, _, _)| *sid != id);
        self.sketch_vertices.retain(|(sid, _, _)| *sid != id);
        self.sketch_rims.retain(|(sid, _)| *sid != id);
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
        // takes everything. World candidates are indices into the baked
        // Vecs; placed candidates are (placement, member-local) pairs
        // materialized into world space below — with the identical
        // `apply_point`/`apply_plane` per-placement baking used to run at
        // registration, so both storage schemes produce bit-equal positions.
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
        let (placed_point_ids, placed_segment_ids, placed_face_ids) = match index {
            Some(ix) => (
                ix.placed_points_in_cone(
                    &self.placements,
                    &self.def_members,
                    origin,
                    dir,
                    tan_aperture,
                ),
                ix.placed_segments_in_cone(
                    &self.placements,
                    &self.def_members,
                    origin,
                    dir,
                    tan_aperture,
                ),
                ix.placed_faces_crossing_ray(&self.placements, &self.def_members, origin, dir),
            ),
            None => (
                self.all_placed(|m| m.points.len()),
                self.all_placed(|m| m.segments.len()),
                self.all_placed(|m| m.faces.len()),
            ),
        };
        let placed_points: Vec<ScenePoint> = placed_point_ids
            .iter()
            .map(|&(pi, li)| {
                let pl = &self.placements[pi];
                self.def_members[&pl.member].point_at(li, pl)
            })
            .collect();
        let placed_segments: Vec<SceneSegment> = placed_segment_ids
            .iter()
            .map(|&(pi, li)| {
                let pl = &self.placements[pi];
                self.def_members[&pl.member].segment_at(li, pl)
            })
            .collect();
        // Singular poses drop out here (`face_at` is `None`), exactly as
        // registration-time baking skipped them.
        let placed_faces: Vec<SceneFace> = placed_face_ids
            .iter()
            .filter_map(|&(pi, li)| {
                let pl = &self.placements[pi];
                self.def_members[&pl.member].face_at(li, pl)
            })
            .collect();

        // Collect all candidates that fall inside the pick cone.
        // Tuple: (kind, angular_dist, depth, position, source, direction)
        let mut candidates: Vec<Candidate> = Vec::new();

        // --- Endpoint candidates: from ScenePoints (world, then placed —
        //     ascending emission order on both index paths, so ranking ties
        //     break identically) ---
        let world_points = point_ids.iter().map(|&pi| &self.points[pi]);
        for sp in world_points.chain(placed_points.iter()) {
            if let Some((ang, depth)) = cone_test(origin, dir, sp.position, aperture) {
                candidates.push((
                    SnapKind::Endpoint,
                    ang,
                    depth,
                    sp.position,
                    Some(Provenance::Object(sp.source)),
                    None,
                ));
            }
        }

        // --- Placed analytic candidates: each placement materializes its
        //     member's rims on the fly (few per scene, linear like the world
        //     ones below — never indexed, so the indexed and reference paths
        //     see the identical set). ---
        let mut placed_centers: Vec<ScenePoint> = Vec::new();
        let mut placed_quadrants: Vec<ScenePoint> = Vec::new();
        let mut placed_rims: Vec<SceneRim> = Vec::new();
        for pl in &self.placements {
            if let Some(dm) = self.def_members.get(&pl.member) {
                dm.rims_at(
                    pl,
                    &mut placed_centers,
                    &mut placed_quadrants,
                    &mut placed_rims,
                );
            }
        }

        // --- Center candidates: true circle centers (linear walk — few per
        //     scene and deliberately outside the spatial index, so the
        //     indexed and reference paths see the identical set). ---
        for cp in self.centers.iter().chain(placed_centers.iter()) {
            if let Some((ang, depth)) = cone_test(origin, dir, cp.position, aperture) {
                candidates.push((
                    SnapKind::Center,
                    ang,
                    depth,
                    cp.position,
                    Some(Provenance::Object(cp.source)),
                    None,
                ));
            }
        }

        // --- Quadrant candidates: covered cardinal points of the rim
        //     circles. Same linear-walk rationale as centers. ---
        for qp in self.quadrants.iter().chain(placed_quadrants.iter()) {
            if let Some((ang, depth)) = cone_test(origin, dir, qp.position, aperture) {
                candidates.push((
                    SnapKind::Quadrant,
                    ang,
                    depth,
                    qp.position,
                    Some(Provenance::Object(qp.source)),
                    None,
                ));
            }
        }

        // --- Sketch-curve candidates: a drawn (unextruded) circle or arc
        //     offers its exact center and covered quadrant points, so
        //     Center/Quadrant snapping exists BEFORE any extrusion. No
        //     provenance — like guides and axes, sketch curves aren't
        //     `SnapSource` elements. Linear walk, like `centers`. ---
        for (_, rim) in &self.sketch_rims {
            if let Some((ang, depth)) = cone_test(origin, dir, rim.center, aperture) {
                candidates.push((SnapKind::Center, ang, depth, rim.center, None, None));
            }
            for q in rim.quadrant_points() {
                if let Some((ang, depth)) = cone_test(origin, dir, q, aperture) {
                    candidates.push((SnapKind::Quadrant, ang, depth, q, None, None));
                }
            }
        }

        // --- Tangent candidates: for each rim circle, the two points where
        //     a segment from the tool's anchor touches the exact circle —
        //     computed per query (they depend on the anchor), offered only
        //     over the covered angular range, and only when the anchor lies
        //     strictly outside the circle in its own plane. Linear walk,
        //     like centers. ---
        if let Some(anchor) = query.anchor {
            for rim in self.rims.iter().chain(placed_rims.iter()) {
                let Some(angles) =
                    tangent_angles(anchor, rim.center, rim.axis, rim.radius, rim.u, rim.v)
                else {
                    continue; // anchor inside or on the circle: no tangent
                };
                for angle in angles {
                    if !rim.covers(angle) {
                        continue;
                    }
                    let pos = rim.point_at(angle);
                    if let Some((ang, depth)) = cone_test(origin, dir, pos, aperture) {
                        candidates.push((
                            SnapKind::Tangent,
                            ang,
                            depth,
                            pos,
                            Some(Provenance::Object(rim.source)),
                            None,
                        ));
                    }
                }
            }
            // Sketch-curve rims tangent-snap identically, just with no
            // provenance (see the sketch-curve Center/Quadrant walk above).
            for (_, rim) in &self.sketch_rims {
                let Some(angles) = tangent_angles(
                    anchor,
                    rim.center,
                    rim.axis,
                    rim.radius,
                    rim.basis_u,
                    rim.basis_v,
                ) else {
                    continue;
                };
                for angle in angles {
                    if !rim.covers(angle) {
                        continue;
                    }
                    let pos = rim.center
                        + rim.basis_u * (rim.radius * angle.cos())
                        + rim.basis_v * (rim.radius * angle.sin());
                    if let Some((ang, depth)) = cone_test(origin, dir, pos, aperture) {
                        candidates.push((SnapKind::Tangent, ang, depth, pos, None, None));
                    }
                }
            }
        }

        // --- Segment candidates: Midpoint and OnEdge (world, then placed) ---
        let world_segments = || segment_ids.iter().map(|&si| &self.segments[si]);
        for seg in world_segments().chain(placed_segments.iter()) {
            let mid = midpoint(seg.a, seg.b);

            // Midpoint candidate: emitted when the midpoint itself is in the cone.
            if let Some((ang, depth)) = cone_test(origin, dir, mid, aperture) {
                candidates.push((
                    SnapKind::Midpoint,
                    ang,
                    depth,
                    mid,
                    Some(Provenance::Object(seg.source)),
                    None,
                ));
            }

            // OnEdge candidate: the closest point on the segment to the ray,
            // if it lies within the cone. Emit even when the midpoint is also
            // in the cone — priority ranking handles "Midpoint beats OnEdge".
            if let Some((pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture) {
                // Skip if this is the same point as the midpoint (it would be
                // a duplicate; the Midpoint candidate already covers it with
                // the stronger kind).
                if !pos.approx_eq(mid, tol::POINT_MERGE) {
                    candidates.push((
                        SnapKind::OnEdge,
                        ang,
                        depth,
                        pos,
                        Some(Provenance::Object(seg.source)),
                        None,
                    ));
                }
            }
        }

        // --- Sketch and transient segment candidates: Endpoint, Midpoint,
        //     and OnEdge. A committed sketch segment's Midpoint/OnEdge carry
        //     its (SketchId, SketchEdgeId) so tools can use the edge as a
        //     reference (Tape Measure parallel guides); endpoints are vertex
        //     snaps and carry none. Transient segments carry none at all. ---
        let bare_segments = self
            .sketch_segments
            .iter()
            .map(|&(sid, eid, ref seg)| (Some(Provenance::SketchEdge(sid, eid)), seg))
            .chain(self.transient_segments.iter().map(|seg| (None, seg)));
        for (prov, seg) in bare_segments {
            for endpoint in [seg.a, seg.b] {
                if let Some((ang, depth)) = cone_test(origin, dir, endpoint, aperture) {
                    candidates.push((SnapKind::Endpoint, ang, depth, endpoint, None, None));
                }
            }

            let mid = midpoint(seg.a, seg.b);
            if let Some((ang, depth)) = cone_test(origin, dir, mid, aperture) {
                candidates.push((SnapKind::Midpoint, ang, depth, mid, prov, None));
            }

            if let Some((pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture)
                && !pos.approx_eq(mid, tol::POINT_MERGE)
            {
                candidates.push((SnapKind::OnEdge, ang, depth, pos, prov, None));
            }
        }

        // --- Face candidates: OnFace (world, then placed) ---
        let world_faces = face_ids.iter().map(|&fi| &self.faces[fi]);
        for face in world_faces.chain(placed_faces.iter()) {
            if let Some((pos, ang, depth)) = face_cone_hit(
                origin,
                dir,
                &face.plane,
                &face.boundary,
                &face.holes,
                aperture,
            ) {
                candidates.push((
                    SnapKind::OnFace,
                    ang,
                    depth,
                    pos,
                    Some(Provenance::Object(face.source)),
                    None,
                ));
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

        // --- Guide-intersection candidates ---
        // The point where a guide line crosses a sketch segment, an object
        // edge, or another guide line is precisely why the guide was drawn —
        // snap it as SnapKind::Intersection (between Midpoint and OnEdge in
        // strength, so a real vertex at the crossing still wins). Like plain
        // guide snaps: no provenance, suppressed when guides are hidden.
        if self.guides_enabled {
            let guide_lines: Vec<(Point3, Vec3)> = self
                .guides
                .iter()
                .filter_map(|g| match g.geom {
                    SceneGuideGeom::Line { origin, direction } => Some((origin, direction)),
                    SceneGuideGeom::Point { .. } => None,
                })
                .collect();

            // Guide × segment (object edges near the ray — world and placed
            // alike — plus every live sketch segment; sketch candidates stay
            // on the linear walk).
            let emit = |p: Point3, candidates: &mut Vec<Candidate>| {
                if let Some((ang, depth)) = cone_test(origin, dir, p, aperture) {
                    candidates.push((SnapKind::Intersection, ang, depth, p, None, None));
                }
            };
            for &(go, gd) in &guide_lines {
                for seg in world_segments().chain(placed_segments.iter()) {
                    if let Some(p) = line_segment_intersection(go, gd, seg.a, seg.b) {
                        emit(p, &mut candidates);
                    }
                }
                for (_, _, seg) in &self.sketch_segments {
                    if let Some(p) = line_segment_intersection(go, gd, seg.a, seg.b) {
                        emit(p, &mut candidates);
                    }
                }
            }
            // Guide × guide.
            for (i, &(ao, ad)) in guide_lines.iter().enumerate() {
                for &(bo, bd) in guide_lines.iter().skip(i + 1) {
                    if let Some(p) = line_line_intersection(ao, ad, bo, bd) {
                        emit(p, &mut candidates);
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

                if let Some((kind, _ang, _depth, pos, prov, _cdir)) = winner.as_ref() {
                    // A candidate snapped: project its position onto the locked line.
                    let projected = project_onto_line(anchor, lock_dir, *pos);
                    let (source, sketch_source) = Provenance::split(*prov);
                    Some(Snap {
                        position: projected,
                        kind: *kind,
                        source,
                        sketch_source,
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
                        sketch_source: None,
                        direction: Some(lock_dir),
                    })
                }
            }
            _ => {
                // No lock (or lock with no anchor): return the top-ranked
                // candidate that is actually visible (see the occlusion cull above).
                winner.map(|(kind, _ang, _depth, pos, prov, snap_dir)| {
                    let (source, sketch_source) = Provenance::split(prov);
                    Snap {
                        position: pos,
                        kind,
                        source,
                        sketch_source,
                        direction: snap_dir,
                    }
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
        let placed_face_ids: Vec<(usize, usize)> = match index {
            Some(ix) => {
                ix.placed_faces_crossing_ray(&self.placements, &self.def_members, origin, dir)
            }
            None => self.all_placed(|m| m.faces.len()),
        };
        let placed_faces: Vec<SceneFace> = placed_face_ids
            .iter()
            .filter_map(|&(pi, li)| {
                let pl = &self.placements[pi];
                self.def_members[&pl.member].face_at(li, pl)
            })
            .collect();
        let mut best: Option<(f64, SnapSource)> = None;
        let world_faces = face_ids.iter().map(|&fi| &self.faces[fi]);
        for face in world_faces.chain(placed_faces.iter()) {
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

    /// Every `(placement, member-local)` candidate pair of one class, in
    /// enumeration order — the linear reference's counterpart to the
    /// two-level index walks. Placements of an unregistered member are
    /// skipped, matching the indexed paths (their world boxes are empty).
    fn all_placed(&self, count: impl Fn(&DefMember) -> usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (pi, pl) in self.placements.iter().enumerate() {
            if let Some(m) = self.def_members.get(&pl.member) {
                out.extend((0..count(m)).map(|li| (pi, li)));
            }
        }
        out
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
        // A placed face materializes on demand and skips singular poses,
        // exactly like the candidate paths (`DefMember::face_at`).
        let occludes_placed = |pi: usize, li: usize| {
            let pl = &self.placements[pi];
            self.def_members[&pl.member]
                .face_at(li, pl)
                .is_some_and(|face| occludes(&face))
        };
        match index {
            // Early-out walks: only subtrees whose boxes the ray enters
            // nearer than the threshold can hold an occluder, and each walk
            // stops at the first face that actually occludes. World faces
            // first, placed faces second — order can't change the boolean.
            Some(ix) => {
                ix.any_face_hit_before(origin, dir, near_threshold, |fi| occludes(&self.faces[fi]))
                    || ix.any_placed_face_hit_before(
                        &self.placements,
                        &self.def_members,
                        origin,
                        dir,
                        near_threshold,
                        occludes_placed,
                    )
            }
            None => {
                self.faces.iter().any(occludes)
                    || self
                        .all_placed(|m| m.faces.len())
                        .into_iter()
                        .any(|(pi, li)| occludes_placed(pi, li))
            }
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
        self.pick_sketch_edge(ray, aperture).map(|(id, _)| id)
    }

    /// Like [`InferenceScene::pick_sketch`], but says WHICH edge was hit:
    /// the `(SketchId, SketchEdgeId)` of the live sketch segment nearest the
    /// ray within `aperture` (same ranking — smallest angular distance,
    /// depth breaks ties). The Select tool's per-edge pick. `None` if the
    /// ray hits no live sketch edge within `aperture`.
    pub fn pick_sketch_edge(
        &self,
        ray: &PickRay,
        aperture: f64,
    ) -> Option<(SketchId, SketchEdgeId)> {
        let dir = ray.direction.normalized().ok()?;
        let origin = ray.origin;
        // (angular_dist, depth, sketch, edge)
        let mut best: Option<(f64, f64, SketchId, SketchEdgeId)> = None;
        for &(id, eid, ref seg) in &self.sketch_segments {
            if let Some((_pos, ang, depth)) = segment_cone_hit(origin, dir, seg.a, seg.b, aperture)
                && best
                    .as_ref()
                    .is_none_or(|&(a, d, _, _)| (ang, depth) < (a, d))
            {
                best = Some((ang, depth, id, eid));
            }
        }
        best.map(|(_, _, id, eid)| (id, eid))
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
type Candidate = (SnapKind, f64, f64, Point3, Option<Provenance>, Option<Vec3>);

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

/// The two angles (in the rim's `u`/`v` frame) where a segment from
/// `anchor` is tangent to the circle `(center, axis, radius)`, or `None`
/// when the anchor's in-plane projection lies inside or on the circle (no
/// tangent exists). Shared by the object-rim and sketch-curve tangent
/// walks so both resolve the identical geometry.
fn tangent_angles(
    anchor: Point3,
    center: Point3,
    axis: Vec3,
    radius: f64,
    u: Vec3,
    v: Vec3,
) -> Option<[f64; 2]> {
    let d = anchor - center;
    let in_plane = d - axis * d.dot(axis);
    let dist = in_plane.length();
    if dist <= radius + tol::POINT_MERGE {
        return None;
    }
    let phi = in_plane.dot(v).atan2(in_plane.dot(u));
    let alpha = (radius / dist).acos();
    Some([phi + alpha, phi - alpha])
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
/// The point where the infinite line `(o, d)` crosses the segment `[a, b]`,
/// or `None` when they are parallel, skew beyond [`tol::POINT_MERGE`], or
/// the crossing falls outside the segment (with [`tol::POINT_MERGE`] of
/// world-distance slack at the endpoints). The returned point lies ON the
/// segment (real geometry), not on the guide.
fn line_segment_intersection(o: Point3, d: Vec3, a: Point3, b: Point3) -> Option<Point3> {
    let e = b - a;
    let seg_len2 = e.dot(e);
    if seg_len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return None; // degenerate segment
    }
    let dd = d.dot(d);
    if dd < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return None; // degenerate guide direction
    }
    // Solve min |o + s·d − (a + t·e)| for (s, t) — standard line/line closest
    // points. denom / (dd·seg_len2) = sin²θ between the directions; treat the
    // pair as parallel below the same normalize floor segment_cone_hit uses.
    let w = o - a;
    let de = d.dot(e);
    let denom = dd * seg_len2 - de * de;
    if denom < dd * seg_len2 * tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let dw = d.dot(w);
    let ew = e.dot(w);
    let t = (dd * ew - de * dw) / denom;
    // Endpoint slack in world distance, expressed in the parameter t.
    let t_slack = tol::POINT_MERGE / seg_len2.sqrt();
    if t < -t_slack || t > 1.0 + t_slack {
        return None;
    }
    let t = t.clamp(0.0, 1.0);
    let on_seg = Point3::new(a.x + e.x * t, a.y + e.y * t, a.z + e.z * t);
    let s = (de * t - dw) / dd; // closest param on the guide for that t
    let on_line = Point3::new(o.x + d.x * s, o.y + d.y * s, o.z + d.z * s);
    if (on_seg - on_line).length() > tol::POINT_MERGE {
        return None; // skew — the lines pass near, not through, each other
    }
    Some(on_seg)
}

/// The point where two infinite lines cross, or `None` when parallel or
/// skew beyond [`tol::POINT_MERGE`]. Returns the midpoint of the closest
/// pair (exact crossing → the crossing itself).
fn line_line_intersection(ao: Point3, ad: Vec3, bo: Point3, bd: Vec3) -> Option<Point3> {
    let w = ao - bo;
    let aa = ad.dot(ad);
    let bb = bd.dot(bd);
    if aa < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
        || bb < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH
    {
        return None; // degenerate direction
    }
    let ab = ad.dot(bd);
    // denom / (aa·bb) = sin²θ — same parallel floor as the segment case.
    let denom = aa * bb - ab * ab;
    if denom < aa * bb * tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let aw = ad.dot(w);
    let bw = bd.dot(w);
    let s = (ab * bw - bb * aw) / denom;
    let t = (aa * bw - ab * aw) / denom;
    let pa = Point3::new(ao.x + ad.x * s, ao.y + ad.y * s, ao.z + ad.z * s);
    let pb = Point3::new(bo.x + bd.x * t, bo.y + bd.y * t, bo.z + bd.z * t);
    if (pa - pb).length() > tol::POINT_MERGE {
        return None;
    }
    Some(Point3::new(
        (pa.x + pb.x) / 2.0,
        (pa.y + pb.y) / 2.0,
        (pa.z + pb.z) / 2.0,
    ))
}

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
        assert!(SnapKind::Endpoint < SnapKind::Center);
        assert!(SnapKind::Center < SnapKind::Quadrant);
        assert!(SnapKind::Quadrant < SnapKind::Midpoint);
        assert!(SnapKind::Midpoint < SnapKind::Intersection);
        assert!(SnapKind::Intersection < SnapKind::Tangent);
        assert!(SnapKind::Tangent < SnapKind::OnEdge);
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
    /// as a snap along a ray through them, with `source: None` (sketch edges
    /// have no Object provenance; the midpoint carries `sketch_source`
    /// instead — see `sketch_segment_snaps_carry_sketch_provenance`).
    #[test]
    fn sketch_segment_endpoint_and_midpoint_snap() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(0.0, 0.0, 0.0);
        let b = Point3::new(2.0, 0.0, 0.0);
        scene.add_sketch(id, &[(SketchEdgeId::default(), a, b)]);

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

    /// A committed sketch segment's Midpoint and OnEdge snaps carry the
    /// owning `(SketchId, SketchEdgeId)` in `sketch_source`, so tools can
    /// use the edge as a reference (Tape Measure parallel guides). Endpoint
    /// snaps are vertex snaps and carry none; transient segments carry none.
    #[test]
    fn sketch_segment_snaps_carry_sketch_provenance() {
        let mut scene = InferenceScene::new();
        let sid = SketchId::default();
        let eid = SketchEdgeId::default();
        let a = Point3::new(0.0, 0.0, 0.0);
        let b = Point3::new(2.0, 0.0, 0.0);
        scene.add_sketch(sid, &[(eid, a, b)]);

        let query_at = |target: Point3| SnapQuery {
            ray: PickRay {
                origin: Point3::new(target.x, target.y, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.05,
            constraint_plane: None,
        };

        // Midpoint: carries the sketch provenance.
        let mid = scene
            .resolve(&query_at(Point3::new(1.0, 0.0, 0.0)))
            .expect("midpoint on ray");
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert_eq!(mid.sketch_source, Some((sid, eid)));

        // On-edge (off-midpoint interior point): carries it too.
        let on_edge = scene
            .resolve(&query_at(Point3::new(0.5, 0.0, 0.0)))
            .expect("on-edge on ray");
        assert_eq!(on_edge.kind, SnapKind::OnEdge);
        assert_eq!(on_edge.sketch_source, Some((sid, eid)));

        // Endpoint: a vertex snap — no edge provenance.
        let endpoint = scene.resolve(&query_at(b)).expect("endpoint on ray");
        assert_eq!(endpoint.kind, SnapKind::Endpoint);
        assert_eq!(endpoint.sketch_source, None);

        // A transient segment's snaps carry none.
        let mut scene2 = InferenceScene::new();
        scene2.add_transient_segment(a, b);
        let t_mid = scene2
            .resolve(&query_at(Point3::new(1.0, 0.0, 0.0)))
            .expect("transient midpoint on ray");
        assert_eq!(t_mid.kind, SnapKind::Midpoint);
        assert_eq!(t_mid.sketch_source, None);
    }

    /// `remove_sketch` unregisters a sketch's candidates; removing an unknown
    /// id is a no-op.
    #[test]
    fn remove_sketch_unregisters_its_candidates() {
        let mut scene = InferenceScene::new();
        let id = SketchId::default();
        let a = Point3::new(20.0, 20.0, 0.0);
        let b = Point3::new(22.0, 20.0, 0.0);
        scene.add_sketch(id, &[(SketchEdgeId::default(), a, b)]);

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

    /// A guide line crossing a sketch segment snaps as Intersection exactly
    /// at the crossing — the reason the guide was drawn. A real vertex at
    /// the same spot still outranks it, and hidden guides emit nothing.
    #[test]
    fn guide_crossing_a_sketch_segment_snaps_as_intersection() {
        let mut scene = InferenceScene::new();
        // Horizontal sketch segment y=1, x in 0..2.
        scene.add_sketch(
            SketchId::default(),
            &[(
                SketchEdgeId::default(),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(2.0, 1.0, 0.0),
            )],
        );
        // Vertical guide line through x = 0.5.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(0.5, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
            },
        );

        let query_at = |x: f64, y: f64| SnapQuery {
            ray: PickRay {
                origin: Point3::new(x, y, 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.02,
            constraint_plane: None,
        };

        let snap = scene
            .resolve(&query_at(0.5, 1.0))
            .expect("the crossing snaps");
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert!(
            snap.position
                .approx_eq(Point3::new(0.5, 1.0, 0.0), tol::POINT_MERGE)
        );

        // The segment's endpoint still outranks the intersection when the
        // guide passes through it.
        scene.add_guide(
            GuideId::default(),
            &Guide::Line {
                origin: Point3::new(0.0, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
            },
        );
        let at_vertex = scene
            .resolve(&query_at(0.0, 1.0))
            .expect("the vertex snaps");
        assert_eq!(at_vertex.kind, SnapKind::Endpoint);

        // Hidden guides emit neither OnGuide nor Intersection.
        scene.set_guides_enabled(false);
        let hidden = scene.resolve(&query_at(0.5, 1.0));
        assert!(hidden.is_none_or(|s| s.kind != SnapKind::Intersection));
    }

    /// Two crossing guide lines snap as Intersection at their crossing.
    #[test]
    fn crossing_guides_snap_as_intersection() {
        let mut scene = InferenceScene::new();
        // add_guide has replace semantics per id — mint two DISTINCT GuideIds
        // from a real Document so both guides coexist.
        let mut doc = kernel::Document::new();
        let g1 = doc
            .add_guide_line(Point3::new(0.3, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0))
            .expect("guide 1");
        let g2 = doc
            .add_guide_line(Point3::new(0.0, 0.7, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .expect("guide 2");
        scene.add_guide(
            g1,
            &Guide::Line {
                origin: Point3::new(0.3, 0.0, 0.0),
                direction: Vec3::new(0.0, 1.0, 0.0),
            },
        );
        scene.add_guide(
            g2,
            &Guide::Line {
                origin: Point3::new(0.0, 0.7, 0.0),
                direction: Vec3::new(1.0, 0.0, 0.0),
            },
        );

        let snap = scene
            .resolve(&SnapQuery {
                ray: PickRay {
                    origin: Point3::new(0.3, 0.7, 5.0),
                    direction: Vec3::new(0.0, 0.0, -1.0),
                },
                anchor: None,
                lock: None,
                aperture: 0.02,
                constraint_plane: None,
            })
            .expect("the guide crossing snaps");
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert!(
            snap.position
                .approx_eq(Point3::new(0.3, 0.7, 0.0), tol::POINT_MERGE)
        );
    }

    /// `pick_sketch_edge` returns WHICH edge was hit, not just the sketch:
    /// two edges registered with distinct real ids resolve to the right one
    /// depending on where the ray points.
    #[test]
    fn pick_sketch_edge_distinguishes_edges_within_one_sketch() {
        // Mint two genuinely distinct SketchEdgeIds from a real kernel
        // sketch (slotmap keys — Default would alias them).
        let mut sk = kernel::Sketch::on_plane(
            kernel::Plane::from_polygon(&[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ])
            .unwrap(),
        );
        sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
            .unwrap();
        sk.add_segment(Point3::new(0.0, 5.0, 0.0), Point3::new(2.0, 5.0, 0.0))
            .unwrap();
        let ids: Vec<SketchEdgeId> = sk.edges().keys().collect();
        let [e_low, e_high] = [ids[0], ids[1]];
        let seg_of = |eid: SketchEdgeId| {
            let e = &sk.edges()[eid];
            (
                eid,
                sk.vertices()[e.from].position,
                sk.vertices()[e.to].position,
            )
        };

        let mut scene = InferenceScene::new();
        let sid = SketchId::default();
        scene.add_sketch(sid, &[seg_of(e_low), seg_of(e_high)]);

        let ray_at = |x: f64, y: f64| PickRay {
            origin: Point3::new(x, y, 5.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        // Which id is which depends on which segment sits at y=0 vs y=5.
        let e = &sk.edges()[e_low];
        let low_is_y0 = sk.vertices()[e.from].position.y.abs() < 1e-9;
        let (y0_edge, y5_edge) = if low_is_y0 {
            (e_low, e_high)
        } else {
            (e_high, e_low)
        };

        assert_eq!(
            scene.pick_sketch_edge(&ray_at(1.0, 0.0), 0.05),
            Some((sid, y0_edge)),
        );
        assert_eq!(
            scene.pick_sketch_edge(&ray_at(1.0, 5.0), 0.05),
            Some((sid, y5_edge)),
        );
        assert_eq!(scene.pick_sketch_edge(&ray_at(50.0, 50.0), 0.05), None);
    }

    /// `pick_sketch` returns the id of the sketch whose edge the ray passes
    /// nearest to, within the aperture; a ray that hits nothing returns `None`.
    #[test]
    fn pick_sketch_returns_the_nearest_sketch_within_aperture() {
        let mut scene = InferenceScene::new();
        let near = SketchId::default();
        scene.add_sketch(
            near,
            &[(
                SketchEdgeId::default(),
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(2.0, 0.0, 0.0),
            )],
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
