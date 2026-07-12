//! Solid operations: birth (extrusion), modification (push/pull, sticky face
//! split/merge), and explicit combination (booleans).
//!
//! M0 status: complete public contracts, `todo!()` bodies. The executable
//! contract (property tests, `#[ignore]`d until implemented) lives in
//! `crates/kernel/tests/op_specs.rs`. Implementations must follow the
//! mutation pattern in `validate.rs`: do the work, then `check_invariants()`
//! as the last step before returning `Ok`.
//!
//! # Watertightness transitions (the state machine)
//!
//! | Operation        | Requires            | On success                    |
//! |------------------|---------------------|-------------------------------|
//! | `from_extrusion` | valid `Profile`     | always `Watertight`           |
//! | `push_pull`      | `Watertight`        | stays `Watertight`            |
//! | `split_face`     | any state           | state unchanged               |
//! | `merge_faces`    | any state           | state unchanged               |
//! | `boolean`        | both `Watertight`   | always `Watertight`           |
//!
//! No operation in this module can *produce* an `Open` object from a
//! `Watertight` one: anything that would open the shell is a typed error
//! instead (docs/DEVELOPMENT.md: "operations that would open a shell are prevented").
//! `Open` objects exist only via construction from open soup
//! (`Object::from_polygons`) or import, and the only ops valid on them are
//! the surface-local sticky ones (`split_face`/`merge_faces`).
//!
//! # Transactionality
//!
//! Every `&mut self` operation gives the **strong guarantee**: on `Err`, the
//! object is exactly as it was before the call. First implementations may
//! get this the blunt way (clone, mutate the clone, swap on success);
//! optimize later, never weaken.
//!
//! # No dangling edges (design decision)
//!
//! SketchUp lets a half-drawn edge dangle across a face until a later stroke
//! closes a cut. A half-edge *solid* cannot represent a dangling edge without
//! breaking the manifold invariants, so the kernel only accepts **complete
//! cuts**: `split_face` takes the whole path, boundary to boundary. Partial
//! strokes are tool-layer state (the UI buffers the polyline and commits when
//! it closes). This keeps every committed state a valid solid.

use crate::geom2d::{
    boundaries_contact, interior_point_of_loops, point_inside_polygon, point_near_segment,
    polygon_is_simple, segments_intersect, signed_area_on_plane,
};
use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::sketch::Profile;
use crate::tol;
use crate::topo::{Edge, Face, HalfEdge, Loop, LoopKind, Object, Vertex, WatertightState};
use crate::transform::{Transform, TransformError};

/// The explicit combination modes (ARCHITECTURE.md: combining Objects is always a
/// deliberate user action; nothing ever welds implicitly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// Material in `a`, `b`, or both.
    Union,
    /// Material in `a` but not `b`.
    Subtract,
    /// Material in both `a` and `b`.
    Intersect,
}

/// Which operand of a two-Object operation an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// The first operand (`a`).
    A,
    /// The second operand (`b`).
    B,
}

/// What `push_pull` changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPullReport {
    /// The moved face in its new position. May differ from the input handle
    /// if the operation had to rebuild it.
    pub face: FaceId,
    /// Faces created (typically side walls).
    pub created_faces: Vec<FaceId>,
    /// Now-dead handles of faces consumed by merging or subtraction. Safe to
    /// hold (generational), useless to dereference.
    pub removed_faces: Vec<FaceId>,
    /// True iff the push erected a wall along a SLANTED (non-coplanar)
    /// neighbor, whose removal a plain `push_pull(-d)` cannot re-detect
    /// (`find_collapse_plans` matches only coplanar-far-face walls). The
    /// [`History`](crate::History) layer reads this to record the exact
    /// [`Object::unbuild_push_pull`] inverse instead. A pure translate (box)
    /// or a pure coplanar step (whose `-d` inverse still works) leaves it
    /// false. Not exposed across the WASM boundary.
    pub requires_unbuild_inverse: bool,
}

/// What `split_face` changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceSplitReport {
    /// The two faces replacing the input face, in no guaranteed order. The
    /// input handle is dead afterwards.
    pub new_faces: [FaceId; 2],
    /// The edges of the cut path, in path order. Passing any of these to
    /// `merge_faces` undoes the split.
    pub new_edges: Vec<EdgeId>,
    /// Pre-existing boundary edges split by the path endpoints: the now-dead
    /// handle and its two fragments.
    pub split_boundary_edges: Vec<(EdgeId, [EdgeId; 2])>,
}

/// What `split_face_inner` changed: a closed loop imprinted strictly inside a
/// face, splitting it into a new coplanar sub-face plus the parent (now holed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceSplitInnerReport {
    /// The new coplanar sub-face (the loop interior).
    pub sub_face: FaceId,
    /// The parent face, now carrying the loop as a hole. Handle unchanged.
    pub parent: FaceId,
    /// The loop's new edges (each shared by `sub_face` and the parent's hole).
    pub new_edges: Vec<EdgeId>,
}

/// A face's restorable attribute state pinned to a point strictly inside
/// that face's pre-merge material. The merge reports snapshot one per
/// dissolved face; the split ops' restore path re-applies each to whichever
/// result face contains the point — geometric matching, so the restoration
/// is correct regardless of which fragment ends up on which handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceAttrsAt {
    /// A point strictly inside the face the attributes belonged to.
    pub point: Point3,
    /// The face's attribute state at snapshot time.
    pub attrs: crate::topo::FaceAttrs,
}

/// What `merge_faces` changed. (`prior_attrs` carries f64 positions, so this
/// is `PartialEq` but not `Eq`.)
#[derive(Debug, Clone, PartialEq)]
pub struct FaceMergeReport {
    /// The single face replacing the two inputs (both input handles die).
    pub merged_face: FaceId,
    /// Now-dead handles of the dissolved shared-boundary edges.
    pub removed_edges: Vec<EdgeId>,
    /// Pre-merge attribute snapshots of the two input faces, each pinned to
    /// an interior point, so undoing the merge restores exactly the
    /// attributes each side had — never a copy of the survivor's. BEST
    /// EFFORT per face: a face too degenerate to pin an interior point in
    /// (a sub-[`tol::POINT_MERGE`]-width sliver, reachable only through
    /// imported polygon soup) snapshots as `None`, the merge proceeds
    /// unchanged, and undo falls back to inheriting the survivor's
    /// attributes for that face only — attribute fidelity degrades before
    /// forward behavior ever does.
    pub prior_attrs: [Option<FaceAttrsAt>; 2],
}

/// What `merge_inner_face` changed: a sub-face dissolved back into its parent.
/// (`loop_path` carries f64 positions, so this is `PartialEq` but not `Eq`.)
#[derive(Debug, Clone, PartialEq)]
pub struct FaceMergeInnerReport {
    /// The parent face, now hole-free again. Handle unchanged.
    pub parent: FaceId,
    /// The dissolved loop's positions (parent-relative), in order — re-imprinting
    /// them on `parent` restores the sub-face.
    pub loop_path: Vec<Point3>,
    /// The dissolved sub-face's attribute state at merge time, so undoing
    /// the merge restores exactly what the sub-face carried — never a fresh
    /// copy inherited from the current parent, which would resurrect an
    /// analytic claim the sub-face had legitimately lost (or lose its own
    /// paint).
    pub sub_face_attrs: crate::topo::FaceAttrs,
}

/// What `collapse_sub_face` changed: a raised sub-face flattened back, its walls
/// removed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollapseSubFaceReport {
    /// The now-flat sub-face (handle unchanged).
    pub sub_face: FaceId,
    /// The height the sub-face was raised by (signed) — re-extruding by this
    /// restores the boss/recess.
    pub distance: f64,
}

/// Typed failures of extrusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtrudeError {
    /// |distance| below [`tol::POINT_MERGE`](crate::tol::POINT_MERGE): the result would be a
    /// zero-thickness shell, which is not a solid.
    DistanceTooSmall,
    /// The sweep produced geometry that is degenerate or fails the topology
    /// validator (e.g. a near-zero-area wall, or a profile shape this version
    /// cannot extrude into a valid solid). Reported instead of panicking so
    /// the caller surfaces a clean error rather than aborting.
    DegenerateGeometry,
}

/// Typed failures of push/pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPullError {
    /// The face handle is stale or from another Object.
    UnknownFace,
    /// Push/pull is only defined on watertight solids; thickening open
    /// shells is a different (future) operation.
    ObjectNotSolid,
    /// |distance| below [`tol::POINT_MERGE`](crate::tol::POINT_MERGE); commit nothing instead of a
    /// no-op the user can't see.
    DistanceTooSmall,
    /// Pushing inward by at least the solid's extent would remove all
    /// material. The kernel refuses; deleting the Object is the document's
    /// decision, not a geometric side effect.
    WouldVanish,
    /// The built side walls would intersect other geometry of this Object in
    /// a way that has no manifold result. Fail loudly; never produce
    /// near-correct geometry.
    ///
    /// This is what bounds an inward flat-face push: the moment the rigidly
    /// translated moved face (or a wall built along one of its edges) would
    /// cross the fixed structure it is pushed into, [`validate_sweep_result`]
    /// refuses here, byte-identical — a wedge's slant face cannot be pushed in
    /// at all. An outward pull erects a prism of material and, on a
    /// convex-enough solid, does not reach this — but a pull whose new walls
    /// would ram a distant part of the SAME solid (a non-convex reach) still
    /// refuses here too.
    NonManifoldResult,
    /// `extrude_sub_face`/`collapse_sub_face` on a face that is not the expected
    /// kind (a flat imprinted sub-face, or a raised one with generated walls).
    NotASubFace,
    /// Whole-wall push/pull on an attributed cylinder wall
    /// (docs/design/true-curves.md §4.6): the offset would shrink the wall's
    /// radius to or below [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) — the
    /// cylinder would vanish into its own axis. Refused; the object is
    /// untouched.
    RadiusVanishes,
    /// Whole-wall push/pull on an attributed cylinder wall: a neighboring
    /// face would be bent off any single plane by the radial offset (e.g. a
    /// stepped or bossed wall touching the cylinder's seam, where some of
    /// its vertices follow the wall and others are pinned elsewhere).
    /// Refused rather than folded — neighbors may translate, stretch, or
    /// pivot with the wall, but never cease to be planar.
    WallNeighborNonPlanar,
}

/// How an operation treats its best-effort obstruction heuristics
/// (DEVELOPMENT.md rule 9 / ARCHITECTURE.md §5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardMode {
    /// A forward user op: every heuristic guard is enforced.
    Enforced,
    /// A history replay (recorded inverse or redo): the obstruction
    /// heuristics are skipped, because the replay re-enters a previously
    /// accepted state and the [`History`](crate::history::History) verifies
    /// the result against that state's recorded fingerprint instead.
    /// Structural checks and validation are NOT affected by this mode.
    Replay,
}

/// How a moved face's boundary half-edge relates to its neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundaryEdgeKind {
    /// Neighbor's normal is ~perpendicular to the sweep — an ordinary
    /// side wall that extends seamlessly as the moved face translates
    /// (no wall is built along this edge).
    Transverse,
    /// Neighbor's normal is neither ~perpendicular nor ~parallel to the
    /// sweep (a wedge/chamfer face, an adjacent facet of an N-gon prism, any
    /// face produced by Slice). Like a coplanar sibling, the shared edge
    /// unwelds and a fresh quad wall is built between the old edge and the
    /// raised one; the neighbor keeps its shape.
    Slanted,
    /// Neighbor's normal is ~parallel to the sweep — a coplanar sibling
    /// (a `split_face` cut edge); a wall is built along the shared edge
    /// instead of translating it.
    Coplanar,
}

/// Typed failures of the sticky surface operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickyError {
    /// The face handle is stale or from another Object.
    UnknownFace,
    /// The edge handle is stale or from another Object.
    UnknownEdge,
    /// A cut path needs at least two points.
    PathTooShort,
    /// Path endpoint not on the face boundary within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE)
    /// (`which` = 0 for first, 1 for last).
    EndpointNotOnBoundary {
        /// 0 = first path point, 1 = last.
        which: usize,
    },
    /// An interior path point is off the face plane (beyond
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)) or outside the face.
    PointNotOnFace {
        /// Index of the offending point in the path.
        index: usize,
    },
    /// The path crosses itself or touches the boundary between its
    /// endpoints, which would not produce exactly two faces.
    PathNotSimple,
    /// `merge_faces` on an edge whose two faces are not coplanar within
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) / [`tol::NORMAL_DIRECTION`](crate::tol::NORMAL_DIRECTION).
    FacesNotCoplanar,
    /// `merge_faces` on a boundary edge (only one face).
    BoundaryEdge,
    /// `merge_faces` where both sides are the same face: dissolving the edge
    /// would leave a non-manifold or punctured result.
    SameFaceOnBothSides,
    /// `merge_faces` on faces that share TWO OR MORE disconnected edge
    /// chains (a bridge/dogbone adjacency): dissolving everything shared
    /// would need the merged boundary rebuilt as outer + hole loops, which
    /// the merge does not build yet. Refused, not guessed.
    SharedChainDisconnected,
    /// `merge_faces` where the shared chain covers one face's ENTIRE outer
    /// boundary (a disk filling part of its neighbor's ring): dissolving it
    /// is `merge_inner_face`'s sub-face surgery, not an edge-chain merge.
    /// Refused, not guessed.
    SharedChainCoversBoundary,
    /// `split_face_inner`: a loop vertex is not strictly inside the face (it lies
    /// on/outside the outer boundary or inside a hole). v1 imprints only loops
    /// strictly interior to the face.
    LoopNotStrictlyInside {
        /// Index of the offending loop vertex.
        index: usize,
    },
    /// `split_face_inner`: the loop crosses itself.
    LoopSelfIntersects,
    /// `split_face_inner_with_curve`: the supplied analytic circle claim does
    /// not describe the loop — a degenerate radius, or a loop vertex not
    /// within tolerance of `radius` from `center`. The kernel never fits a
    /// circle to the points (no silent repair, DEVELOPMENT.md rule 4); the
    /// caller must supply the circle the vertices actually lie on.
    CurveClaimOffLoop,
    /// `merge_inner_face`: the face is not an imprinted sub-face (its boundary is
    /// not a single closed loop twinned entirely with one parent's hole loop).
    NotAnInnerFace,
    /// The operation's result failed topology validation. The object is left
    /// exactly as it was (strong guarantee). This is the release-safe backstop:
    /// the debug validator (`check_invariants`) is compiled out of release
    /// builds, so a cut that would corrupt topology (e.g. a near-degenerate
    /// path from a noisy UI snap) is refused here with a typed error rather
    /// than committing invalid geometry that panics on a later access.
    WouldCorrupt,
}

/// Typed failures of boolean combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanError {
    /// An operand is not watertight; booleans are volume algebra and need
    /// volumes.
    OperandNotSolid {
        /// Which operand.
        which: Operand,
    },
    /// The exact result is empty (disjoint intersect, total subtract). An
    /// "empty Object" does not exist; the caller decides what emptiness
    /// means.
    EmptyResult,
    /// The frame-mapping transform is singular.
    SingularTransform,
    /// Operands touch in a way with no manifold result (coincident faces,
    /// edge-on-face tangency). Refused rather than repaired; nudge or use a
    /// merge-group container instead.
    DegenerateContact,
}

/// Typed failures of [`Object::slice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceError {
    /// The source object is not a watertight solid; there is no volume to cut.
    NotSolid,
    /// The cutting plane does not pass through the solid's interior (the whole
    /// object lies on one side, or only grazes the plane), so a cut would
    /// produce nothing on one side. Nothing is changed.
    PlaneMissesSolid,
    /// The cut is degenerate or tangent — coincident with an existing face, or
    /// grazing an edge/vertex with no transverse area. Refused, not repaired,
    /// exactly as booleans refuse [`BooleanError::DegenerateContact`].
    Degenerate,
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            SliceError::NotSolid => "slice requires a watertight solid",
            SliceError::PlaneMissesSolid => "cutting plane does not pass through the solid",
            SliceError::Degenerate => "cut is degenerate or tangent to the solid",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for SliceError {}

impl std::fmt::Display for ExtrudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtrudeError::DistanceTooSmall => {
                write!(f, "extrusion distance below tol::POINT_MERGE")
            }
            ExtrudeError::DegenerateGeometry => {
                write!(f, "extrusion produced degenerate or invalid geometry")
            }
        }
    }
}

impl std::error::Error for ExtrudeError {}

impl std::fmt::Display for PushPullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PushPullError::UnknownFace => "no such face in this object",
            PushPullError::ObjectNotSolid => "push/pull requires a watertight object",
            PushPullError::DistanceTooSmall => "push/pull distance below tol::POINT_MERGE",
            PushPullError::WouldVanish => "push/pull would remove all material",
            PushPullError::NonManifoldResult => "push/pull sweep has no manifold result",
            PushPullError::NotASubFace => "face is not the expected imprinted sub-face",
            PushPullError::RadiusVanishes => {
                "wall offset would shrink the cylinder radius to nothing"
            }
            PushPullError::WallNeighborNonPlanar => {
                "wall offset would bend a neighboring face off its plane"
            }
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for PushPullError {}

impl std::fmt::Display for StickyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StickyError::UnknownFace => write!(f, "no such face in this object"),
            StickyError::UnknownEdge => write!(f, "no such edge in this object"),
            StickyError::PathTooShort => write!(f, "cut path needs at least two points"),
            StickyError::EndpointNotOnBoundary { which } => {
                write!(f, "path endpoint {which} is not on the face boundary")
            }
            StickyError::PointNotOnFace { index } => {
                write!(f, "path point {index} is not on the face")
            }
            StickyError::PathNotSimple => {
                write!(f, "cut path crosses itself or grazes the boundary")
            }
            StickyError::FacesNotCoplanar => {
                write!(
                    f,
                    "faces are not coplanar; dissolving the edge would bend them"
                )
            }
            StickyError::BoundaryEdge => write!(f, "cannot merge across a boundary edge"),
            StickyError::SameFaceOnBothSides => {
                write!(f, "edge has the same face on both sides")
            }
            StickyError::SharedChainCoversBoundary => {
                write!(
                    f,
                    "faces share an entire outer boundary; dissolve the sub-face instead"
                )
            }
            StickyError::SharedChainDisconnected => {
                write!(
                    f,
                    "faces share more than one separate run of edges; merging them is not supported"
                )
            }
            StickyError::LoopNotStrictlyInside { index } => {
                write!(f, "loop vertex {index} is not strictly inside the face")
            }
            StickyError::LoopSelfIntersects => write!(f, "loop crosses itself"),
            StickyError::CurveClaimOffLoop => {
                write!(f, "analytic circle claim does not match the loop vertices")
            }
            StickyError::NotAnInnerFace => {
                write!(f, "face is not an imprinted sub-face")
            }
            StickyError::WouldCorrupt => {
                write!(f, "operation would produce invalid topology")
            }
        }
    }
}

impl std::error::Error for StickyError {}

impl std::fmt::Display for BooleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BooleanError::OperandNotSolid { which } => {
                write!(f, "boolean operand {which:?} is not watertight")
            }
            BooleanError::EmptyResult => write!(f, "boolean result is empty"),
            BooleanError::SingularTransform => {
                write!(f, "operand transform is singular")
            }
            BooleanError::DegenerateContact => {
                write!(f, "operands touch degenerately; no manifold result exists")
            }
        }
    }
}

impl std::error::Error for BooleanError {}

impl Object {
    /// Bakes an affine `transform` into this object's geometry: every vertex is
    /// moved by `transform` and every face plane is remapped (the
    /// inverse-transpose rule via [`Transform::apply_plane`]), in place.
    ///
    /// Topology is untouched — no element is added or removed — so all handles
    /// stay valid and the watertightness state is preserved. This is what
    /// move/rotate/scale commit, and how a boolean brings an operand into a
    /// shared frame.
    ///
    /// Validated up front so the mutation is transactional (the strong
    /// guarantee): a singular linear part is [`TransformError::Singular`] and an
    /// orientation-flipping one (determinant < 0, e.g. a negative scale) is
    /// [`TransformError::Reflection`] — both refused before any vertex moves, so
    /// the object is never left half-transformed or inside-out.
    pub fn apply_transform(&mut self, transform: &Transform) -> Result<(), TransformError> {
        // Reject before mutating. `inverse()` fails iff the linear part is
        // singular; a negative determinant would invert every face's winding.
        transform.inverse()?;
        if transform.determinant() < 0.0 {
            return Err(TransformError::Reflection);
        }
        for v in self.vertices.values_mut() {
            v.position = transform.apply_point(v.position);
        }
        // Analytic surface references map under similarities (rotation ×
        // uniform scale — the maps that keep a cylinder a cylinder) and DROP
        // under anything else (map-or-drop, docs/design/true-curves.md).
        let scale = transform.similarity_scale();
        for f in self.faces.values_mut() {
            // Non-singular by the check above, so apply_plane cannot fail.
            f.plane = transform
                .apply_plane(&f.plane)
                .expect("apply_plane on a validated non-singular transform");
            f.surface = match (f.surface, scale) {
                (
                    Some(crate::topo::SurfaceRef::Cylinder {
                        axis_point,
                        axis,
                        radius,
                    }),
                    Some(s),
                ) => transform.apply_vector(axis).normalized().ok().map(|axis| {
                    crate::topo::SurfaceRef::Cylinder {
                        axis_point: transform.apply_point(axis_point),
                        axis,
                        radius: radius * s,
                    }
                }),
                _ => None,
            };
        }
        // Per-edge analytic circle claims follow the same map-or-drop rule:
        // a similarity maps the center as a point and scales the radius; any
        // non-similarity drops the claim (an ellipse is not a circle).
        for e in self.edges.values_mut() {
            e.curve = match (e.curve, scale) {
                (Some(crate::sketch::CurveGeom { center, radius }), Some(s)) => {
                    Some(crate::sketch::CurveGeom {
                        center: transform.apply_point(center),
                        radius: radius * s,
                    })
                }
                _ => None,
            };
        }
        self.check_invariants();
        Ok(())
    }

    /// The birth of every solid: sweeps a closed [`Profile`] by `distance`
    /// along its plane normal (negative = against the normal) into a discrete
    /// watertight Object.
    ///
    /// This is what makes the modeler solids-first: the user draws a closed
    /// region and pulls; the kernel answers with a solid, never a face soup.
    ///
    /// Geometry produced: a cap on the profile plane wound to face *against*
    /// the sweep direction, a translated cap facing *along* it, one quad wall
    /// per boundary edge (outer walls face outward, hole walls inward —
    /// holes become tunnels; caps over holed profiles are faces with inner
    /// loops). Result is `Watertight` by construction and validator-checked.
    /// A profile with `h` holes yields a genus-`h` solid; note the caps are
    /// then annuli, not disks, so the plain `V - E + F = 2 - 2h` does NOT
    /// hold — the invariant is Euler–Poincaré with hole loops counted:
    /// `V - E + F - H = 2(S - G)` where `H` = total inner loops across
    /// faces, `S` = shells, `G` = genus sum.
    ///
    /// # Errors
    /// [`ExtrudeError::DistanceTooSmall`] if |`distance`| <
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE). (Profile validity was settled at
    /// [`Profile::new`]; invalid profiles cannot reach this function.)
    pub fn from_extrusion(profile: &Profile, distance: f64) -> Result<Object, ExtrudeError> {
        if distance.abs() < crate::tol::POINT_MERGE {
            return Err(ExtrudeError::DistanceTooSmall);
        }

        let normal = profile.plane().normal();
        let sweep = normal * distance;

        let outer = profile.outer();
        let holes = profile.holes();
        let n_outer = outer.len();

        // Collect hole sizes for index arithmetic.
        let hole_sizes: Vec<usize> = holes.iter().map(|h| h.len()).collect();
        let total_hole_verts: usize = hole_sizes.iter().sum();
        let total_near = n_outer + total_hole_verts;

        // Build the flat positions array:
        //   [0..n_outer)                   near outer vertices
        //   [n_outer..total_near)          near hole vertices (holes concatenated)
        //   [total_near..total_near+n_outer)  far outer vertices
        //   [total_near+n_outer..)         far hole vertices
        let mut positions: Vec<Point3> = Vec::with_capacity(total_near * 2);
        for &p in outer {
            positions.push(p);
        }
        for hole in holes {
            for &p in hole {
                positions.push(p);
            }
        }
        for &p in outer {
            positions.push(p + sweep);
        }
        for hole in holes {
            for &p in hole {
                positions.push(p + sweep);
            }
        }

        // Index helper: start index of hole i in the near layer.
        let hole_near_start: Vec<usize> = {
            let mut starts = Vec::with_capacity(holes.len());
            let mut acc = n_outer;
            for &sz in &hole_sizes {
                starts.push(acc);
                acc += sz;
            }
            starts
        };
        // Far layer starts total_near positions later.

        // We'll collect (outer_loop_indices, inner_loop_index_lists, plane,
        // surface) tuples for from_faces_with_holes. `surface` is the wall's
        // analytic cylinder when its profile edge came from a curve chain
        // with geometry (docs/design/true-curves.md); caps carry None.
        #[allow(clippy::type_complexity)]
        let mut face_specs: Vec<(
            Vec<usize>,
            Vec<Vec<usize>>,
            Plane,
            Option<crate::topo::SurfaceRef>,
        )> = Vec::new();
        // A wall facet's cylinder: axis = the sweep direction through the
        // profile curve's center, radius = the curve's radius.
        let wall_surface = |g: crate::sketch::CurveGeom| crate::topo::SurfaceRef::Cylinder {
            axis_point: g.center,
            axis: normal,
            radius: g.radius,
        };

        // ---- near cap -------------------------------------------------------
        // Faces -normal (away from sweep for positive distance).
        // For positive distance, near cap faces -normal so outer loop is the
        // outer profile vertices reversed (making it CCW seen from -normal).
        // Inner loops: hole indices reversed too.
        let near_cap_plane = {
            // Near cap plane: same as profile plane but with inverted normal.
            // Build from the reversed outer boundary.
            let rev_outer: Vec<Point3> = outer.iter().rev().copied().collect();
            Plane::from_polygon(&rev_outer).map_err(|_| ExtrudeError::DegenerateGeometry)?
        };

        let near_outer_reversed: Vec<usize> = (0..n_outer).rev().collect();

        let near_inner_loops: Vec<Vec<usize>> = holes
            .iter()
            .enumerate()
            .map(|(i, hole)| {
                let start = hole_near_start[i];
                // Reverse hole boundary for inner loop on near cap.
                (0..hole.len()).rev().map(|k| start + k).collect()
            })
            .collect();

        face_specs.push((near_outer_reversed, near_inner_loops, near_cap_plane, None));

        // ---- far cap --------------------------------------------------------
        // Faces +normal (along sweep direction).
        // Outer loop same winding as profile outer, translated by sweep.
        let far_outer: Vec<usize> = (0..n_outer).map(|i| total_near + i).collect();

        let far_inner_loops: Vec<Vec<usize>> = holes
            .iter()
            .enumerate()
            .map(|(i, hole)| {
                let start = total_near + hole_near_start[i];
                (0..hole.len()).map(|k| start + k).collect()
            })
            .collect();

        // Far cap plane: same normal as profile, but offset by sweep.
        let far_cap_plane = {
            let far_outer_pts: Vec<Point3> =
                (0..n_outer).map(|i| positions[total_near + i]).collect();
            Plane::from_polygon(&far_outer_pts).map_err(|_| ExtrudeError::DegenerateGeometry)?
        };

        face_specs.push((far_outer, far_inner_loops, far_cap_plane, None));

        // ---- outer walls ----------------------------------------------------
        // For edge (a, b) in the outer loop (CCW from +normal), the wall quad
        // is [a_near, b_near, b_far, a_far] which faces outward.
        for k in 0..n_outer {
            let a_near = k;
            let b_near = (k + 1) % n_outer;
            let a_far = total_near + a_near;
            let b_far = total_near + b_near;
            let wall_pts = [
                positions[a_near],
                positions[b_near],
                positions[b_far],
                positions[a_far],
            ];
            let wall_plane =
                Plane::from_polygon(&wall_pts).map_err(|_| ExtrudeError::DegenerateGeometry)?;
            face_specs.push((
                vec![a_near, b_near, b_far, a_far],
                vec![],
                wall_plane,
                profile.outer_curve(k).map(wall_surface),
            ));
        }

        // ---- hole walls -----------------------------------------------------
        // For edge (a, b) in a hole loop (CW from +normal), the wall quad
        // [a_near, b_near, b_far, a_far] faces inward (correct for a tunnel).
        for (i, hole) in holes.iter().enumerate() {
            let n_hole = hole.len();
            let near_start = hole_near_start[i];
            let far_start = total_near + near_start;
            for k in 0..n_hole {
                let a_near = near_start + k;
                let b_near = near_start + (k + 1) % n_hole;
                let a_far = far_start + k;
                let b_far = far_start + (k + 1) % n_hole;
                let wall_pts = [
                    positions[a_near],
                    positions[b_near],
                    positions[b_far],
                    positions[a_far],
                ];
                let wall_plane =
                    Plane::from_polygon(&wall_pts).map_err(|_| ExtrudeError::DegenerateGeometry)?;
                face_specs.push((
                    vec![a_near, b_near, b_far, a_far],
                    vec![],
                    wall_plane,
                    profile.hole_curve(i, k).map(wall_surface),
                ));
            }
        }

        // The construction above winds outward for a sweep ALONG the normal.
        // For a negative distance the solid sits on the other side of the
        // profile plane, so every loop must flip or the result is a
        // consistently inside-out shell — twin pairing and the validator
        // cannot see that; the signed-volume specs can.
        if distance < 0.0 {
            face_specs = face_specs
                .into_iter()
                .map(|(outer_idx, inner_lists, _plane, surface)| {
                    let rev_outer: Vec<usize> = outer_idx.into_iter().rev().collect();
                    let rev_inners: Vec<Vec<usize>> = inner_lists
                        .into_iter()
                        .map(|l| l.into_iter().rev().collect())
                        .collect();
                    let pts: Vec<Point3> = rev_outer.iter().map(|&i| positions[i]).collect();
                    let plane =
                        Plane::from_polygon(&pts).map_err(|_| ExtrudeError::DegenerateGeometry)?;
                    Ok((rev_outer, rev_inners, plane, surface))
                })
                .collect::<Result<Vec<_>, ExtrudeError>>()?;
        }

        // Freshly extruded faces take the default material and no UV frame.
        let face_specs: Vec<_> = face_specs
            .into_iter()
            .map(|(outer, inners, plane, surface)| (outer, inners, plane, None, None, surface))
            .collect();
        let obj = Object::from_faces_with_holes(&positions, &face_specs);
        // Note: extrusion stamps side-wall FACES with `SurfaceRef` (above) but
        // does NOT stamp cap EDGES with the profile's circle. A cap whose
        // outer loop is a circle can only be pushed *through* into the
        // opposing cap of the same prism, which vanishes it; any reachable
        // "push a circular face through" is the imprinted disk sub-face, whose
        // edges the imprint stamps directly. Cap-edge stamping is therefore
        // omitted (it would serve no reachable case and carries a far-cap
        // center trap) — docs/design/true-curves.md, playtest fix C3.

        // A valid Profile should always yield a valid solid; if the sweep
        // nonetheless produced invalid topology, return a typed error rather
        // than tripping the debug-only validator panic at the WASM boundary.
        obj.validate()
            .map_err(|_| ExtrudeError::DegenerateGeometry)?;
        Ok(obj)
    }

    /// SketchUp's signature move on any FLAT face: translates `face` rigidly
    /// by `distance` along its outward normal (positive = outward, adding
    /// material; negative = inward, removing it) and rebuilds the side walls.
    /// This is classic translate-and-build — the moved face is a rigid loop, a
    /// wall is erected along each boundary edge whose neighbor does not extend
    /// to follow it, whatever the neighbor's angle.
    ///
    /// (A face carrying an analytic surface — a cylinder/arc wall — is a
    /// different operation: the whole curved wall expands. That lives on the
    /// curves branch. This branch has no surface references, so every facet is
    /// flat and takes the translate-and-build path below. At merge the two are
    /// disjoint by `face.surface` presence — see docs/design/flat-face-pushpull.md.)
    ///
    /// Behavioral contract:
    /// The picked face takes one of two regimes, dispatched on whether it
    /// carries an analytic [`SurfaceRef`](crate::topo::SurfaceRef): a flat
    /// face translates and rebuilds walls; an attributed cylinder wall offsets
    /// its whole logical wall radially.
    ///
    /// **Flat face — translate-and-build:**
    /// - **Transverse neighbor** (perpendicular to the sweep, e.g. a box's
    ///   side wall): the moved edge stays in the neighbor's plane, so the
    ///   neighbor simply extends — the shared ring translates in place, no wall
    ///   is minted. A purely transverse boundary (a box face) keeps the
    ///   bit-identical fast path: pulling a box's top up yields a taller box
    ///   with 6 faces.
    /// - **Coplanar neighbor** (a `split_face` sibling sub-face) and **slanted
    ///   neighbor** (a wedge/chamfer face, adjacent facets of an N-gon prism,
    ///   any face produced by Slice) are handled identically: the shared edge
    ///   **unwelds** and a fresh quad wall is erected between the old edge and
    ///   the raised one, so the neighbor keeps its shape and the solid gains a
    ///   facet. Junctions where such an edge meets a transverse one reshape the
    ///   transverse neighbor into a (still planar) stepped polygon.
    /// - **PULL (outward) is unbounded by neighbor angle:** erecting a prism of
    ///   material on a flat face is always valid however oblique the neighbors,
    ///   so pulls do not refuse on account of the neighbor angles this rework
    ///   is about. (A pull whose new walls would interpenetrate a DISTANT part
    ///   of the same non-convex solid is still refused by the result check
    ///   below — that is a genuine self-intersection, not a neighbor angle.)
    /// - **PUSH (inward) is bounded by validity:** the built result is checked
    ///   for self-intersection ([`validate_sweep_result`]) and refuses typed
    ///   ([`PushPullError::NonManifoldResult`], object byte-identical) the
    ///   moment the moved face would cross the fixed structure it is pushed
    ///   into — for a wedge's slant face that limit is zero (it cannot be
    ///   pushed in at all); pushing far enough to consume the solid refuses as
    ///   [`PushPullError::WouldVanish`].
    /// - Pushing inward through the opposite face is a separate "push through"
    ///   (a boolean subtraction, handled above the kernel op).
    /// - **Inverse:** a pure translate (box) inverts by `push_pull(f', -d)`; a
    ///   wall-building push does NOT — the walls it erects are perpendicular to
    ///   the moved face, so a plain `-d` push cannot re-collapse a slanted
    ///   neighbor's non-coplanar wall. Its exact inverse is the recorded
    ///   [`Object::unbuild_push_pull`], derived and dispatched by
    ///   [`History`](crate::History). (The direct-`push_pull` step-close of a
    ///   *coplanar* sibling still works via `find_collapse_plans`.)
    ///
    /// **Attributed cylinder wall — whole-wall radial**
    /// (docs/design/true-curves.md §4.6): a face carrying a
    /// [`SurfaceRef::Cylinder`](crate::topo::SurfaceRef) does not translate.
    /// Pushing any facet of a stamped wall acts on the **logical wall**: every
    /// face of this object claiming the same cylinder offsets radially so the
    /// wall's radius changes by exactly `distance` along the picked facet's
    /// outward normal (pulling an outer wall outward grows the radius; pulling
    /// a hole wall toward its axis shrinks the hole). The radial map is affine
    /// in the cross-section plane, so every fully-moved face (wall facets,
    /// full-seam chord walls, caps whose rim is the wall) stays planar by
    /// construction; the surface references map to the new radius (never
    /// dropped — the wall is still exactly that cylinder). Neighbors sharing
    /// vertices with the wall **follow**: they translate (a D-profile's chord
    /// wall), stretch in-plane (caps perpendicular to the axis, radial
    /// pie-slice walls), or pivot (a prism wall tangent to a rounded corner
    /// — two parallel vertical edges always span a plane). A neighbor that
    /// would be bent off any single plane (stepped/bossed walls pinned
    /// elsewhere) refuses with [`PushPullError::WallNeighborNonPlanar`];
    /// every reshaped face must stay simple with holes strictly inside and
    /// mutually disjoint ([`PushPullError::NonManifoldResult`]); an offset
    /// to radius ≤ [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) is
    /// [`PushPullError::RadiusVanishes`]. Inverse property: offsetting by
    /// `distance` then `-distance` on the same facet restores topology and
    /// geometry (within floating-point round-trip tolerance).
    ///
    /// # Errors
    /// See [`PushPullError`]; all leave the object untouched.
    pub fn push_pull(
        &mut self,
        face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        self.push_pull_impl(face, distance, GuardMode::Enforced)
    }

    /// [`push_pull`](Object::push_pull) in history-replay mode
    /// (DEVELOPMENT.md rule 9): the `WouldVanish` extent check and the
    /// interior-obstruction guard are skipped — the caller is the
    /// [`History`](crate::history::History), which dispatches only recorded
    /// inverses/redos and verifies the result against the recorded state's
    /// fingerprint before committing. Every structural refusal and the
    /// validator backstop still run.
    pub(crate) fn push_pull_replay(
        &mut self,
        face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        self.push_pull_impl(face, distance, GuardMode::Replay)
    }

    fn push_pull_impl(
        &mut self,
        face: FaceId,
        distance: f64,
        guards: GuardMode,
    ) -> Result<PushPullReport, PushPullError> {
        // --- Step 1: Validate inputs ---
        if self.watertight != crate::topo::WatertightState::Watertight {
            return Err(PushPullError::ObjectNotSolid);
        }
        if !self.faces.contains_key(face) {
            return Err(PushPullError::UnknownFace);
        }
        if distance.abs() < tol::POINT_MERGE {
            return Err(PushPullError::DistanceTooSmall);
        }

        // Attributed cylinder walls take the whole-wall path: the facet is a
        // chord of a logical wall, and push/pull on it means "offset the
        // wall's radius", never "translate this one facet"
        // (docs/design/true-curves.md §4.6).
        if self.faces[face].surface.is_some() {
            return self.offset_cylinder_wall(face, distance);
        }

        let face_normal = self.faces[face].plane.normal();
        let sweep = face_normal * distance;

        let boundary_loops: Vec<LoopId> = std::iter::once(self.faces[face].outer_loop)
            .chain(self.faces[face].inner_loops.iter().copied())
            .collect();

        // Collect all vertex IDs on the moved face (outer + inner loops).
        let moved_vertices: std::collections::BTreeSet<VertexId> = {
            let mut verts = std::collections::BTreeSet::new();
            for &loop_id in &boundary_loops {
                for h in self.loop_half_edges(loop_id) {
                    verts.insert(self.half_edges[h].origin);
                }
            }
            verts
        };

        // Collect neighbor faces: faces that share a boundary edge with the moved face.
        // A boundary edge of the moved face is one where one half-edge is on the moved face
        // and the twin is on a different face.
        let neighbor_faces: Vec<FaceId> = {
            let mut neighbors = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for &loop_id in &boundary_loops {
                for h in self.loop_half_edges(loop_id) {
                    if let Some(twin_h) = self.half_edges[h].twin {
                        let neighbor_loop = self.half_edges[twin_h].loop_id;
                        let neighbor_face = self.loops[neighbor_loop].face;
                        if neighbor_face != face && seen.insert(neighbor_face) {
                            neighbors.push(neighbor_face);
                        }
                    }
                }
            }
            neighbors
        };

        // --- Step 2: Eligibility check ---
        // Classify each boundary half-edge of the moved face by how its
        // neighbor's plane relates to the sweep direction:
        // - dot ~ 0: transverse neighbor (a perpendicular side wall) — translate
        //   the shared vertices in place; the neighbor extends seamlessly.
        // - dot ~ 1: coplanar sibling (a `split_face` cut edge) — unweld and
        //   build a wall along the shared edge.
        // - otherwise: a slanted neighbor — treated identically to a coplanar
        //   one (unweld and build a wall; see the contract above).
        let mut edge_kinds: std::collections::BTreeMap<HalfEdgeId, BoundaryEdgeKind> =
            std::collections::BTreeMap::new();
        for &loop_id in &boundary_loops {
            for h in self.loop_half_edges(loop_id) {
                let Some(twin_h) = self.half_edges[h].twin else {
                    // No twin means an open boundary edge; ObjectNotSolid above
                    // should have already refused this, but guard anyway.
                    return Err(PushPullError::NonManifoldResult);
                };
                let neighbor_loop = self.half_edges[twin_h].loop_id;
                let neighbor_face = self.loops[neighbor_loop].face;
                let normal_neighbor = self.faces[neighbor_face].plane.normal();
                let dot = face_normal.dot(normal_neighbor).abs();
                let kind = if dot <= tol::NORMAL_DIRECTION {
                    BoundaryEdgeKind::Transverse
                } else if dot >= 1.0 - tol::NORMAL_DIRECTION {
                    BoundaryEdgeKind::Coplanar
                } else {
                    BoundaryEdgeKind::Slanted
                };
                edge_kinds.insert(h, kind);
            }
        }
        let has_coplanar = edge_kinds
            .values()
            .any(|k| matches!(k, BoundaryEdgeKind::Coplanar));
        let has_slanted = edge_kinds
            .values()
            .any(|k| matches!(k, BoundaryEdgeKind::Slanted));
        // Coplanar siblings (a `split_face` cut edge) and slanted neighbors (a
        // wedge/chamfer/facet wall) are handled by the SAME translate-and-build
        // surgery: unweld the moved face's boundary and erect a fresh quad wall
        // along each such edge (`push_pull_build_walls`). A boundary mixing both
        // is therefore no longer a special case — every non-transverse edge
        // builds a wall, every transverse one extends its neighbor seamlessly.
        let has_wall = has_coplanar || has_slanted;

        // Endpoints of a Coplanar boundary edge (a `split_face` cut shared
        // with a sibling sub-face): the coplanar-aware mutation unwelds along
        // these on purpose, so a "fixed" occurrence of either endpoint on a
        // neighbor wall is never an obstruction.
        let coplanar_adjacent_vertices: std::collections::BTreeSet<VertexId> = boundary_loops
            .iter()
            .flat_map(|&loop_id| self.loop_half_edges(loop_id))
            .filter(|h| matches!(edge_kinds[h], BoundaryEdgeKind::Coplanar))
            .flat_map(|h| {
                let origin = self.half_edges[h].origin;
                let dest = self.half_edges[self.half_edges[h].next].origin;
                [origin, dest]
            })
            .collect();

        // This sweep may exactly close a step built by an earlier
        // coplanar-aware push ( inverse) — detected structurally, the same
        // way `try_collapse_coplanar_step` will apply it below. When it does,
        // the candidate walls' far corners (e.g. an L-shaped wall's original
        // far end) are part of the step machinery being collapsed away, not a
        // genuine fixed obstruction, so the interior-obstruction guard below
        // must not see them as blocking — and indeed must not run for THIS
        // sweep at all: collapsing is the structural inverse of a previously
        // accepted push, so it cannot newly self-intersect.
        // The direct-`push_pull` step-close is reserved for a boundary with no
        // slanted edge: a step weld translates the remaining boundary rigidly,
        // which is wrong for a face that must ALSO build a wall along a slanted
        // edge. Such a push instead routes through translate-and-build and is
        // bounded by result-validation (History still undoes a wall-building
        // push exactly, via the recorded `unbuild_push_pull`).
        let is_collapse = !has_slanted
            && !find_collapse_plans(self, face, &boundary_loops, &edge_kinds, sweep).is_empty();

        // --- Step 3: WouldVanish check (inward push only) ---
        // Heuristic guard: skipped on history replay (rule 9) — a replayed
        // inverse/redo re-enters an accepted state and is verified against
        // its recorded fingerprint by the History instead.
        if distance < 0.0 && !is_collapse && guards == GuardMode::Enforced {
            // Inward push: compute extent = maximum inward signed distance from
            // the moved face's plane over all vertices NOT on the moved face.
            // "Inward" means in the -face_normal direction, so the signed distance
            // from the moved face's plane to each such vertex is positive
            // (they are on the inside of the solid relative to this face).
            // extent is the maximum of those signed distances.
            let moved_face_plane = self.faces[face].plane;
            let mut extent = f64::NEG_INFINITY;
            for (vid, vertex) in &self.vertices {
                if !moved_vertices.contains(&vid) {
                    let sd = moved_face_plane.signed_distance(vertex.position);
                    // For an inward push on a watertight solid, other vertices
                    // should be on the inside (negative sd relative to the outward normal).
                    // The inward signed distance is -sd.
                    let inward_dist = -sd;
                    if inward_dist > extent {
                        extent = inward_dist;
                    }
                }
            }
            // |distance| is the inward push magnitude (distance < 0 so |distance| = -distance).
            if (-distance) >= extent - tol::POINT_MERGE {
                return Err(PushPullError::WouldVanish);
            }
        }

        // Interior-obstruction guard, PURE-TRANSLATE (fast path) only: moving
        // the shared ring in place would fold a neighbor face whose FIXED
        // vertices sit closer along the sweep than the push depth past them
        // into a self-intersecting shell — every face stays planar and
        // manifold, so the validator cannot see it. Refuse at the nearest
        // fixed neighbor vertex strictly in front of the sweep. Skipped in
        // three cases:
        //   (1) a collapse;
        //   (2) history replay (GuardMode not Enforced, rule 9) — the
        //       recorded-state proof supersedes this best-effort heuristic,
        //       which reads geometry AROUND the sweep and could otherwise
        //       refuse an exact-closure inverse whose pocket walls were
        //       subdivided by adjacent geometry even though the state it
        //       restores was already accepted;
        //   (3) the wall-building path (`has_wall`), which unwelds the
        //       boundary and erects fresh walls rather than translating the
        //       ring in place, so a fixed neighbor vertex ahead is extended
        //       past, never folded — an outward pull is unbounded by neighbor
        //       angle, and both directions are instead bounded by validating
        //       the built result for self-intersection below (which still
        //       catches a distant-geometry collision on a non-convex solid).
        if !is_collapse && guards == GuardMode::Enforced && !has_wall {
            let moved_face_plane = self.faces[face].plane;
            // +1 along the outward normal for an outward push, -1 inward.
            let along = distance.signum();
            let mut neighbor_limit = f64::INFINITY;
            for &nf in &neighbor_faces {
                let loops: Vec<LoopId> = std::iter::once(self.faces[nf].outer_loop)
                    .chain(self.faces[nf].inner_loops.iter().copied())
                    .collect();
                for loop_id in loops {
                    for h in self.loop_half_edges(loop_id) {
                        let vid = self.half_edges[h].origin;
                        if moved_vertices.contains(&vid)
                            || coplanar_adjacent_vertices.contains(&vid)
                        {
                            continue;
                        }
                        let ahead =
                            along * moved_face_plane.signed_distance(self.vertices[vid].position);
                        if ahead > tol::POINT_MERGE && ahead < neighbor_limit {
                            neighbor_limit = ahead;
                        }
                    }
                }
            }
            if distance.abs() >= neighbor_limit - tol::POINT_MERGE {
                return Err(PushPullError::NonManifoldResult);
            }
        }

        // The moved face's pre-move plane and footprint, for the engulfment
        // guard the wall-build result-validation reuses (captured before the
        // clone is mutated).
        let orig_plane = self.faces[face].plane;
        let old_outer: Vec<Point3> = self.loop_positions(self.faces[face].outer_loop).collect();
        let old_holes: Vec<Vec<Point3>> = self.faces[face]
            .inner_loops
            .iter()
            .map(|&il| self.loop_positions(il).collect())
            .collect();

        // --- Steps 4-6: Clone, mutate, validate, swap ---
        let mut obj = self.clone();
        let mut created_faces: Vec<FaceId> = Vec::new();
        let mut removed_faces: Vec<FaceId> = Vec::new();

        if is_collapse {
            // The sweep exactly closes a coplanar step built by an earlier
            // coplanar-aware push ( inverse): weld the moved face straight
            // back onto the sibling it was raised from, removing the wall and
            // un-stepping the straddling transverse walls, instead of raising
            // a new copy that would immediately coincide with one already
            // there (which `push_pull_build_walls` cannot represent: it
            // always mints a fresh vertex). Checked BEFORE `has_coplanar`:
            // once a step exists, its cut edge classifies as Transverse (the
            // wall built to bridge it is perpendicular to the moved face), so
            // `has_coplanar` alone can no longer detect this case.
            removed_faces =
                try_collapse_coplanar_step(&mut obj, face, &boundary_loops, &edge_kinds, sweep)?
                    .expect("is_collapse implies find_collapse_plans is non-empty");
        } else if has_wall {
            // Translate-and-build (classic SketchUp push/pull): the moved face
            // translates rigidly by `sweep`, every non-transverse boundary edge
            // (a coplanar `split_face` sibling OR a slanted wedge/facet
            // neighbor) unwelds and grows a fresh quad wall, and every
            // transverse neighbor extends seamlessly via spliced junction
            // steps. Topology changes; the walls are recorded for an exact
            // inverse (see `History` / `unbuild_push_pull`).
            created_faces = push_pull_build_walls(
                &mut obj,
                face,
                &boundary_loops,
                &edge_kinds,
                &neighbor_faces,
                sweep,
            )?;
            // Bound the sweep by the built RESULT's validity, reusing the
            // interpenetration / boundary / engulfment checks: an outward pull
            // erects a prism of material and validates for any neighbor angle
            // (bounded only if its walls would ram a distant part of a
            // non-convex solid); an inward push refuses typed the moment the
            // moved face conflicts with the fixed structure it is pushed into.
            validate_sweep_result(
                &obj,
                face,
                orig_plane,
                &old_outer,
                &old_holes,
                &created_faces,
                distance,
            )?;
        } else {
            // Fast path: pure translate mode (all-transverse boundary) —
            // bit-identical to the original box behavior.
            for &vid in &moved_vertices {
                obj.vertices[vid].position = obj.vertices[vid].position + sweep;
            }
            refit_face_plane(&mut obj, face)?;
            for &nf in &neighbor_faces {
                refit_face_plane(&mut obj, nf)?;
            }
        }

        if std::env::var("HEW_DEBUG_DUMP").is_ok() {
            for (vid, v) in &obj.vertices {
                eprintln!("V {:?} pos={:?}", vid, v.position);
            }
            for (fid, f) in &obj.faces {
                eprintln!(
                    "F {:?} outer_loop={:?} normal={:?}",
                    fid,
                    f.outer_loop,
                    f.plane.normal()
                );
            }
            for (l, lp) in &obj.loops {
                eprintln!(
                    "L {:?} face={:?} first={:?}",
                    l, lp.face, lp.first_half_edge
                );
            }
            for (h, he) in &obj.half_edges {
                eprintln!(
                    "HE {:?} origin={:?} twin={:?} next={:?} prev={:?} edge={:?} loop={:?}",
                    h, he.origin, he.twin, he.next, he.prev, he.edge, he.loop_id
                );
            }
        }
        // The moved face left its plane: an analytic surface claim on it no
        // longer holds (map-or-drop, docs/design/true-curves.md). Stretched
        // transverse walls keep theirs — their planes are unchanged and they
        // still lie on the same (infinite) cylinder.
        if let Some(f) = obj.faces.get_mut(face) {
            f.surface = None;
        }
        // A moved subset of vertices may carry an imprinted circle's endpoints
        // off its stored center (a hole ring pushed to thicken the solid); drop
        // any per-edge claim that no longer holds so a stale one never trips the
        // validator (map-or-drop for Edge::curve).
        obj.drop_stale_edge_curves();

        // Step 6: Validate (debug) and the always-on release-safe backstop
        //: a near-degenerate sweep that slips past the guards above yet
        // produces invalid topology must be refused, not committed.
        obj.check_invariants();
        obj.validate()
            .map_err(|_| PushPullError::NonManifoldResult)?;
        *self = obj;

        // Step 7: Report. A slanted neighbor's wall is the only case whose
        // removal a plain `push_pull(-d)` cannot re-detect, so only then does
        // the inverse need the recorded un-build. A pure coplanar step still
        // reverses through `find_collapse_plans` and keeps its `-d` inverse.
        let requires_unbuild_inverse = has_slanted && !created_faces.is_empty();
        Ok(PushPullReport {
            face,
            created_faces,
            removed_faces,
            requires_unbuild_inverse,
        })
    }

    /// The exact, recorded inverse of a translate-and-build [`Object::push_pull`]
    /// (the case that erects side walls — a coplanar `split_face` sibling or a
    /// slanted wedge/facet neighbor). Removes the recorded `walls`, welds each
    /// moved-face wall edge back onto the neighbor edge the wall bridged,
    /// un-steps the reshaped transverse neighbors, and returns `face` to where
    /// it began — restoring the pre-push topology exactly.
    ///
    /// This is dispatched only by the [`History`](crate::History) layer as the
    /// derived inverse of a wall-building push; it is not a user gesture. In
    /// the common LIFO case the `walls` a push recorded are still the pristine
    /// quads it built when that push is undone, so their removal is
    /// unambiguous — no re-detection of collapsibility, and no interaction with
    /// [`find_collapse_plans`]' step-close path (which the direct
    /// `push_pull(-d)` inverse of a *coplanar* push still uses). If an
    /// intervening op altered a recorded wall (subdivided it, or appended a
    /// hole via `split_face_inner`), it is no longer a removable pristine quad;
    /// [`find_unbuild_plans`] rejects it and this refuses typed with the object
    /// untouched, in both debug and release — never a validator panic.
    ///
    /// `forward_distance` is the signed distance of the push being inverted;
    /// the inverse sweep is `-forward_distance` along the (unchanged) face
    /// normal. Restores `face`'s original vertex positions to within
    /// floating-point noise.
    ///
    /// # Errors
    /// [`PushPullError::UnknownFace`] for a stale `face`;
    /// [`PushPullError::NonManifoldResult`] if the recorded walls are not all
    /// present as pristine quads this sweep closes, or the result fails
    /// validation (never mutates on `Err` — strong guarantee).
    pub fn unbuild_push_pull(
        &mut self,
        face: FaceId,
        walls: &[FaceId],
        forward_distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        if !self.faces.contains_key(face) {
            return Err(PushPullError::UnknownFace);
        }
        let normal = self.faces[face].plane.normal();
        // The inverse push travels back along the normal by the forward amount.
        let sweep = normal * (-forward_distance);
        let boundary_loops: Vec<LoopId> = std::iter::once(self.faces[face].outer_loop)
            .chain(self.faces[face].inner_loops.iter().copied())
            .collect();
        let wall_set: std::collections::BTreeSet<FaceId> = walls
            .iter()
            .copied()
            .filter(|&w| self.faces.contains_key(w))
            .collect();

        let mut obj = self.clone();
        let plans = find_unbuild_plans(&obj, &boundary_loops, &wall_set, sweep);
        // Every recorded wall must resolve to exactly one plan; otherwise the
        // recorded inverse cannot faithfully reverse the push, so refuse rather
        // than commit a partial un-build (the object is untouched on Err).
        if plans.len() != wall_set.len() || plans.len() != walls.len() {
            // A recorded wall is no longer a pristine quad (subdivided or
            // consumed by an intervening op that undo did not restore
            // exactly). The exact un-build is impossible: refuse typed,
            // object untouched — the documented "undo fails typed, never
            // corrupts" posture (docs/ROADMAP.md), reached here instead of
            // through `find_collapse_plans`.
            return Err(PushPullError::NonManifoldResult);
        }
        let removed_faces = collapse_plans_surgery(&mut obj, face, &boundary_loops, &plans, sweep)?;

        // Defensive map-or-drop for `Edge::curve` on the recorded-inverse
        // path, mirroring every other push/pull-family mutator
        // (docs/design/true-curves.md): the inverse translate moves the wall's
        // shared vertices, and any incident edge whose analytic circle claim
        // that move takes off its center must drop, never stay stale (a stale
        // claim panics `check_invariants` in debug / false-refuses in release).
        // `unbuild_push_pull` was authored on the pushpull branch before
        // `Edge::curve` existed and was the one push/pull path missing this.
        // (Reinstating a claim the forward push dropped is the same deferred
        // "rigid-translate mapping" polish curves leaves for every push/pull
        // move, universal to the family, not specific to this inverse.)
        obj.drop_stale_edge_curves();

        obj.check_invariants();
        obj.validate()
            .map_err(|_| PushPullError::NonManifoldResult)?;
        *self = obj;
        Ok(PushPullReport {
            face,
            created_faces: Vec::new(),
            removed_faces,
            requires_unbuild_inverse: false,
        })
    }

    /// The whole-wall half of [`Object::push_pull`]
    /// (docs/design/true-curves.md §4.6): radially offsets the logical
    /// cylinder wall `face` belongs to, changing its radius by `distance`
    /// along `face`'s outward normal.
    ///
    /// Mechanism: every face of this object claiming the same cylinder
    /// ([`SurfaceRef::same_surface`](crate::topo::SurfaceRef::same_surface))
    /// is part of the wall; all their vertices move under the radial map
    /// `p ↦ axis_proj(p) + (p − axis_proj(p)) · r'/r`, which is affine in
    /// the cross-section plane (a uniform 2-D scale about the axis), so any
    /// face **all** of whose vertices move stays planar by construction.
    /// Faces with a mix of moved and fixed vertices stay planar when their
    /// displacement lies in their own plane (caps perpendicular to the
    /// axis, radial pie-slice walls) or when the face pivots as a whole (a
    /// prism wall whose two vertical edges stay parallel); a face that
    /// would genuinely bend refuses with
    /// [`PushPullError::WallNeighborNonPlanar`]. Every reshaped face is
    /// re-fit and re-checked (simple boundary, holes strictly inside,
    /// orientation preserved) and the wall's surface references map to the
    /// new radius — never dropped, the wall is still exactly that cylinder.
    ///
    /// Strong guarantee: on `Err` the object is untouched (clone, mutate,
    /// validate, swap).
    fn offset_cylinder_wall(
        &mut self,
        face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        let surface = self.faces[face]
            .surface
            .expect("offset_cylinder_wall is only routed to for attributed faces");
        let crate::topo::SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        } = surface;

        // Radial orientation of the picked facet: a chord plane sits at
        // apothem distance from the axis, on the interior side of an outer
        // wall (outward normal points away from the axis, signed distance of
        // the axis point is negative) and on the exterior side of a hole
        // wall. A plane *through* the axis is not a chord facet of anything;
        // refuse rather than guess a direction.
        let axis_side = self.faces[face].plane.signed_distance(axis_point);
        if axis_side.abs() <= tol::POINT_MERGE {
            return Err(PushPullError::NonManifoldResult);
        }
        let sign = if axis_side < 0.0 { 1.0 } else { -1.0 };
        let new_radius = radius + sign * distance;
        if new_radius <= tol::POINT_MERGE {
            return Err(PushPullError::RadiusVanishes);
        }

        // The logical wall: every face claiming the same cylinder (slot
        // order — deterministic). Disconnected claimants (stacked bands of
        // one drilled cylinder, boolean fragments) move together: they all
        // assert the same surface, and moving only some would leave the rest
        // claiming a radius their neighbors no longer share.
        let wall_faces: Vec<FaceId> = self
            .faces
            .iter()
            .filter(|(_, f)| f.surface.as_ref().is_some_and(|s| s.same_surface(&surface)))
            .map(|(fid, _)| fid)
            .collect();

        // Every vertex on any wall face moves.
        let moved_vertices: std::collections::BTreeSet<VertexId> = wall_faces
            .iter()
            .flat_map(|&fid| {
                let f = &self.faces[fid];
                std::iter::once(f.outer_loop)
                    .chain(f.inner_loops.iter().copied())
                    .collect::<Vec<_>>()
            })
            .flat_map(|loop_id| self.loop_half_edges(loop_id))
            .map(|h| self.half_edges[h].origin)
            .collect();

        // Clone-mutate-validate-swap (strong guarantee).
        let mut obj = self.clone();
        let scale = new_radius / radius;
        for &vid in &moved_vertices {
            let p = obj.vertices[vid].position;
            let axial = axis * (p - axis_point).dot(axis);
            let foot = axis_point + axial;
            obj.vertices[vid].position = foot + (p - foot) * scale;
        }

        // Every face touching a moved vertex is reshaped: re-fit its plane
        // and re-check its boundary. Fully-moved faces are affine images of
        // planar polygons (planar by construction); partially-moved faces
        // must additionally prove they stayed planar or the offset would
        // fold them (WallNeighborNonPlanar, refused — never repaired).
        let affected: Vec<FaceId> = obj
            .faces
            .iter()
            .filter(|(_, f)| {
                std::iter::once(f.outer_loop)
                    .chain(f.inner_loops.iter().copied())
                    .flat_map(|l| obj.loop_half_edges(l))
                    .any(|h| moved_vertices.contains(&obj.half_edges[h].origin))
            })
            .map(|(fid, _)| fid)
            .collect();
        for &fid in &affected {
            // Planarity first, so a genuinely bent neighbor reports as
            // WallNeighborNonPlanar rather than whatever downstream check a
            // bent loop happens to trip. The Newell fit over the outer loop
            // is the reference plane; every loop vertex must still lie on
            // one plane within the object's planarity tolerance.
            let outer_pts: Vec<Point3> = obj.loop_positions(obj.faces[fid].outer_loop).collect();
            let fit = Plane::from_polygon(&outer_pts)
                .map_err(|_| PushPullError::WallNeighborNonPlanar)?;
            let loops: Vec<LoopId> = std::iter::once(obj.faces[fid].outer_loop)
                .chain(obj.faces[fid].inner_loops.iter().copied())
                .collect();
            for l in loops {
                for p in obj.loop_positions(l) {
                    if fit.signed_distance(p).abs() > obj.planarity_tol {
                        return Err(PushPullError::WallNeighborNonPlanar);
                    }
                }
            }
            refit_face_plane(&mut obj, fid)?;
        }

        // Hole loops of a reshaped face must also stay clear of EACH OTHER —
        // growing one tunnel until it swallows a neighboring tunnel keeps
        // every loop simple and inside the outer boundary, which is all
        // `refit_face_plane` checks.
        for &fid in &affected {
            let holes: Vec<Vec<Point3>> = obj.faces[fid]
                .inner_loops
                .iter()
                .map(|&il| obj.loop_positions(il).collect())
                .collect();
            for i in 0..holes.len() {
                for j in (i + 1)..holes.len() {
                    if boundaries_contact(&holes[i], &holes[j])
                        || holes[j].iter().any(|&p| {
                            point_inside_polygon(p, &holes[i], obj.faces[fid].plane.normal())
                        })
                        || holes[i].iter().any(|&p| {
                            point_inside_polygon(p, &holes[j], obj.faces[fid].plane.normal())
                        })
                    {
                        return Err(PushPullError::NonManifoldResult);
                    }
                }
            }
        }

        // Interpenetration guard (DEVELOPMENT.md rule 4;
        // docs/design/true-curves.md §4.6, review follow-up F2). The radial
        // map can move the wall a long way, and everything above only
        // re-validates faces sharing a moved vertex — a grown wall passing
        // straight through geometry it shares nothing with (another shell,
        // or a distant feature of this one) would come out planar,
        // twin-consistent, and invisible to the structural validator. Every
        // reshaped face is therefore tested against every other face of the
        // object — same shell or not — and any contact that is not along
        // elements the two faces legitimately share (common vertices, twin
        // edges) refuses the offset. Mirrors the stretch-mode guard of the
        // generalized push/pull so the two unify at integration.
        let affected_set: std::collections::BTreeSet<FaceId> = affected.iter().copied().collect();
        let all_faces: Vec<FaceId> = obj.faces.keys().collect();
        for &t in &affected {
            for &g in &all_faces {
                if g == t || (affected_set.contains(&g) && g <= t) {
                    continue;
                }
                if faces_improperly_contact(&obj, t, g) {
                    return Err(PushPullError::NonManifoldResult);
                }
            }
        }

        // Engulfment guard: contact detection cannot see a shell the wall
        // sweeps cleanly PAST — afterwards nothing touches anything, and
        // the entombed (or newly exposed) shell still validates. Refuse if
        // any vertex of a non-moving shell changes its point-in-solid
        // classification against the moving shells' faces between the pre-
        // and post-offset positions (parity ray-cast, the boolean
        // classifier's mechanism — exact for every neighbor motion class:
        // radial, translating, and pivoting alike).
        //
        // Why testing VERTICES suffices (completeness): the volume the
        // offset claims (grow) or vacates (shrink) is bounded by the moving
        // shells' OLD boundary and their NEW boundary. A non-moving shell
        // that intersects that volume either
        //   (a) crosses the OLD boundary — impossible for valid input:
        //       those were real faces of this object, and the improper
        //       face contact that crossing implies cannot have been
        //       present before the call;
        //   (b) crosses the NEW boundary — a transversal intersection of
        //       two planar faces is a line segment whose endpoints cross a
        //       boundary edge of one face inside the other, exactly what
        //       the all-pairs sweep above refuses (coplanar overlap is
        //       probed per boundary-cut interval there too); or
        //   (c) touches neither boundary — then it lies ENTIRELY inside
        //       the swept volume, every one of its vertices changed
        //       classification, and this test sees the change.
        // A shell therefore cannot straddle the swept volume "between
        // vertices": straddling means crossing its boundary, which is case
        // (a) or (b). Same-shell geometry needs no vertex test: anything
        // connected to the wall either moved with it or is held by the
        // boundary/hole checks above (e.g. a cap imprint orphaned by a
        // shrink refuses in `refit_face_plane`).
        let moving_shells: Vec<crate::ids::ShellId> = obj
            .shells
            .iter()
            .filter(|(_, sh)| sh.faces.iter().any(|f| affected_set.contains(f)))
            .map(|(id, _)| id)
            .collect();
        let shell_faces = |o: &Object| -> Vec<ShellFace> {
            moving_shells
                .iter()
                .flat_map(|&sid| o.shells[sid].faces.iter().copied())
                .map(|fid| {
                    let f = &o.faces[fid];
                    (
                        f.plane,
                        o.loop_positions(f.outer_loop).collect(),
                        f.inner_loops
                            .iter()
                            .map(|&il| o.loop_positions(il).collect())
                            .collect(),
                    )
                })
                .collect()
        };
        // `obj` is a clone of `self`, so ids coincide; `self` still holds
        // the pre-offset geometry (strong guarantee: nothing swapped yet).
        let old_faces = shell_faces(self);
        let new_faces = shell_faces(&obj);
        for (sid, sh) in &obj.shells {
            if moving_shells.contains(&sid) {
                continue;
            }
            let verts: std::collections::BTreeSet<VertexId> = sh
                .faces
                .iter()
                .flat_map(|&fid| {
                    let f = &obj.faces[fid];
                    std::iter::once(f.outer_loop)
                        .chain(f.inner_loops.iter().copied())
                        .collect::<Vec<_>>()
                })
                .flat_map(|lid| obj.loop_half_edges(lid).collect::<Vec<_>>())
                .map(|h| obj.half_edges[h].origin)
                .collect();
            for &vid in &verts {
                // Non-moving shells never move: the position is identical
                // in `self` and `obj`.
                let p = obj.vertices[vid].position;
                if point_in_shell_faces(&old_faces, p) != point_in_shell_faces(&new_faces, p) {
                    return Err(PushPullError::NonManifoldResult);
                }
            }
        }

        // The wall still is exactly that cylinder, one radius over: map the
        // references (map-or-drop — this is the map half). Bitwise-shared
        // value across the group, like extrusion stamps it.
        let new_surface = crate::topo::SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius: new_radius,
        };
        for &fid in &wall_faces {
            obj.faces[fid].surface = Some(new_surface);
        }
        // Neighbors that translate/stretch/pivot under the radial map can carry
        // an imprinted circle's endpoints off its center; drop any per-edge
        // claim that no longer holds (the wall FACES' surface refs mapped above
        // are a separate, still-valid claim).
        obj.drop_stale_edge_curves();

        obj.check_invariants();
        obj.validate()
            .map_err(|_| PushPullError::NonManifoldResult)?;
        *self = obj;

        Ok(PushPullReport {
            face,
            created_faces: Vec::new(),
            removed_faces: Vec::new(),
            // A radial offset inverts exactly by push_pull(-distance) on the
            // same facet; it builds no walls, so no recorded UnbuildPushPull.
            requires_unbuild_inverse: false,
        })
    }

    /// Sticky rule inside an Object: imprints a closed `loop_path` strictly
    /// inside `face`, splitting it into a new coplanar **sub-face** (the loop
    /// interior) plus the parent face, which now carries the loop as a hole.
    /// This is the within-Object "draw a rectangle on a face" gesture; the
    /// sub-face can then be push/pulled (emboss/recess).
    ///
    /// Purely additive surgery — no existing boundary is rewired. The loop edges
    /// become twin half-edge pairs (sub-face on one side, the parent's hole on
    /// the other), so the mesh stays manifold and watertight. The sub-face shares
    /// the parent's plane and outward normal.
    ///
    /// v1 requires a simple loop **strictly interior** to the face (on its plane,
    /// inside the outer boundary, clear of holes); loops touching the boundary or
    /// self-intersecting refuse cleanly.
    ///
    /// # Errors
    /// See [`StickyError`]; all leave the object untouched.
    pub fn split_face_inner(
        &mut self,
        face: FaceId,
        loop_path: &[Point3],
    ) -> Result<FaceSplitInnerReport, StickyError> {
        self.split_face_inner_impl(face, loop_path, None, None)
    }

    /// [`Object::split_face_inner`] carrying the analytic circle the imprinted
    /// loop's edges are chord facets of, so a later push-through re-attributes
    /// the tunnel walls as [`SurfaceRef::Cylinder`](crate::topo::SurfaceRef)
    /// instead of losing the circle at the imprint (docs/design/true-curves.md,
    /// playtest fix C3). `curve` is stamped onto every edge of the new loop —
    /// correct because a drawn circle/arc imprint is a chain of chord facets
    /// of ONE circle. `None` behaves exactly like [`Object::split_face_inner`].
    ///
    /// The caller owns the truth: `curve.center`/`radius` must be the circle
    /// the `loop_path` vertices actually lie on (the drawing tool computed it).
    /// The kernel never fits a circle to the points — a wrong claim is caught
    /// by the validator, never silently repaired.
    pub fn split_face_inner_with_curve(
        &mut self,
        face: FaceId,
        loop_path: &[Point3],
        curve: Option<crate::sketch::CurveGeom>,
    ) -> Result<FaceSplitInnerReport, StickyError> {
        self.split_face_inner_impl(face, loop_path, None, curve)
    }

    /// [`Object::split_face_inner`] with explicit attributes for the created
    /// sub-face. This is undo's path for reversing a `merge_inner_face` —
    /// the merge report snapshotted the dissolved sub-face's attributes
    /// ([`FaceMergeInnerReport::sub_face_attrs`]) so the sub-face comes back
    /// with exactly what it carried, never a fresh copy inherited from the
    /// current parent (which would resurrect an analytic surface claim the
    /// sub-face had legitimately lost, or lose its own paint). `None` = the
    /// sub-face inherits the parent's attributes (the forward/tool
    /// semantics).
    pub fn split_face_inner_with_attrs(
        &mut self,
        face: FaceId,
        loop_path: &[Point3],
        restore: Option<crate::topo::FaceAttrs>,
    ) -> Result<FaceSplitInnerReport, StickyError> {
        self.split_face_inner_impl(face, loop_path, restore, None)
    }

    /// Shared body of the [`Object::split_face_inner`] family. `restore` is
    /// the undo attribute snapshot (see [`Object::split_face_inner_with_attrs`]);
    /// `curve` stamps the imprinted loop's edges (see
    /// [`Object::split_face_inner_with_curve`]). Crate-internal so history's
    /// dispatch can supply both at once.
    pub(crate) fn split_face_inner_impl(
        &mut self,
        face: FaceId,
        loop_path: &[Point3],
        restore: Option<crate::topo::FaceAttrs>,
        curve: Option<crate::sketch::CurveGeom>,
    ) -> Result<FaceSplitInnerReport, StickyError> {
        // ---- validation (no mutation) ----
        if !self.faces.contains_key(face) {
            return Err(StickyError::UnknownFace);
        }
        if loop_path.len() < 3 {
            return Err(StickyError::PathTooShort);
        }
        let face_plane = self.faces[face].plane;
        let normal = face_plane.normal();
        let n = loop_path.len();

        // No zero-length edges.
        for k in 0..n {
            if loop_path[k].approx_eq(loop_path[(k + 1) % n], tol::POINT_MERGE) {
                return Err(StickyError::LoopSelfIntersects);
            }
        }

        // Every vertex on the plane and strictly inside the face region.
        let outer_pts: Vec<Point3> = self.loop_positions(self.faces[face].outer_loop).collect();
        let hole_pts: Vec<Vec<Point3>> = self.faces[face]
            .inner_loops
            .iter()
            .map(|&il| self.loop_positions(il).collect())
            .collect();
        for (index, &p) in loop_path.iter().enumerate() {
            if face_plane.signed_distance(p).abs() > tol::PLANE_DIST {
                return Err(StickyError::PointNotOnFace { index });
            }
            if !point_inside_polygon(p, &outer_pts, normal)
                || hole_pts.iter().any(|h| point_inside_polygon(p, h, normal))
            {
                return Err(StickyError::LoopNotStrictlyInside { index });
            }
        }

        // Simple closed polygon.
        if !polygon_is_simple(loop_path) {
            return Err(StickyError::LoopSelfIntersects);
        }

        // The loop's enclosed REGION must avoid existing holes entirely, not
        // just its vertices: a loop that encircles an existing hole (or
        // crosses/touches its ring) would claim area already belonging to
        // another sub-face — an unrepresentable nesting whose merge could
        // never be undone (the enclosed hole's re-imprint would no longer be
        // strictly inside the parent).
        for hole in &hole_pts {
            if hole
                .iter()
                .any(|&hp| point_inside_polygon(hp, loop_path, normal))
                || boundaries_contact(loop_path, hole)
            {
                return Err(StickyError::LoopNotStrictlyInside { index: 0 });
            }
        }

        // Normalise winding to CCW seen from the face normal, so the sub-face
        // faces the same way as the parent.
        let mut pts = loop_path.to_vec();
        if signed_area_on_plane(&pts, normal) < 0.0 {
            pts.reverse();
        }

        // The caller owns the analytic truth: a supplied circle claim must
        // describe the loop it is stamped onto (every vertex a chord facet
        // endpoint, i.e. on the circle). Reject a mismatch up front with a
        // typed error — the kernel never fits a circle to the points and
        // never commits stale metadata (map-or-drop; the same invariant the
        // validator enforces, checked here before any surgery so a wrong
        // claim is a clean refusal, not a corruption backstop).
        if let Some(g) = curve {
            if !g.radius.is_finite() || g.radius <= tol::POINT_MERGE {
                return Err(StickyError::CurveClaimOffLoop);
            }
            for &p in &pts {
                if ((p - g.center).length() - g.radius).abs() > self.planarity_tol {
                    return Err(StickyError::CurveClaimOffLoop);
                }
            }
        }

        // ---- additive surgery on a clone (strong guarantee) ----
        let mut obj = self.clone();

        let verts: Vec<VertexId> = pts
            .iter()
            .map(|&p| {
                obj.vertices.insert(Vertex {
                    position: p,
                    outgoing: HalfEdgeId::default(),
                })
            })
            .collect();

        let sub_loop = obj.loops.insert(Loop {
            face: FaceId::default(),
            first_half_edge: HalfEdgeId::default(),
            kind: LoopKind::Outer,
        });
        let hole_loop = obj.loops.insert(Loop {
            face,
            first_half_edge: HalfEdgeId::default(),
            kind: LoopKind::Inner,
        });
        // Imprinted sub-face inherits the parent face's material, UV frame
        // ( +  extension), and analytic surface (a sub-face of a chord facet
        // lies on the same chord plane of the same cylinder) — unless undo
        // supplied the dissolved sub-face's own snapshot to restore.
        let sub_attrs = restore.unwrap_or_else(|| obj.faces[face].attrs());
        let sub_face = obj.faces.insert(Face {
            outer_loop: sub_loop,
            inner_loops: Vec::new(),
            plane: face_plane,
            material: sub_attrs.material,
            uv_frame: sub_attrs.uv_frame,
            surface: sub_attrs.surface,
        });
        obj.loops[sub_loop].face = sub_face;

        // sub-face half-edges: h_sub[k] = verts[k] -> verts[k+1] (CCW).
        let h_sub: Vec<HalfEdgeId> = (0..n)
            .map(|k| {
                obj.half_edges.insert(HalfEdge {
                    origin: verts[k],
                    twin: None,
                    next: HalfEdgeId::default(),
                    prev: HalfEdgeId::default(),
                    edge: EdgeId::default(),
                    loop_id: sub_loop,
                })
            })
            .collect();
        // hole half-edges: h_hole[k] = twin of h_sub[k] = verts[k+1] -> verts[k] (CW).
        let h_hole: Vec<HalfEdgeId> = (0..n)
            .map(|k| {
                obj.half_edges.insert(HalfEdge {
                    origin: verts[(k + 1) % n],
                    twin: None,
                    next: HalfEdgeId::default(),
                    prev: HalfEdgeId::default(),
                    edge: EdgeId::default(),
                    loop_id: hole_loop,
                })
            })
            .collect();

        let mut new_edges = Vec::with_capacity(n);
        for k in 0..n {
            obj.half_edges[h_sub[k]].next = h_sub[(k + 1) % n];
            obj.half_edges[h_sub[k]].prev = h_sub[(k + n - 1) % n];
            // The hole winds opposite, so its `next` walks the array backwards.
            obj.half_edges[h_hole[k]].next = h_hole[(k + n - 1) % n];
            obj.half_edges[h_hole[k]].prev = h_hole[(k + 1) % n];

            obj.half_edges[h_sub[k]].twin = Some(h_hole[k]);
            obj.half_edges[h_hole[k]].twin = Some(h_sub[k]);
            // Every edge of a drawn circle/arc imprint is a chord facet of the
            // same circle: stamp the caller's analytic claim so a later
            // push-through re-attributes the tunnel walls (true-curves C3).
            let edge = obj.edges.insert(Edge {
                half_edge: h_sub[k],
                twin_half_edge: Some(h_hole[k]),
                curve,
            });
            obj.half_edges[h_sub[k]].edge = edge;
            obj.half_edges[h_hole[k]].edge = edge;
            new_edges.push(edge);

            obj.vertices[verts[k]].outgoing = h_sub[k];
        }
        obj.loops[sub_loop].first_half_edge = h_sub[0];
        obj.loops[hole_loop].first_half_edge = h_hole[0];
        obj.faces[face].inner_loops.push(hole_loop);

        // The sub-face joins the parent's shell.
        let shell = obj
            .shells
            .iter()
            .find(|(_, s)| s.faces.contains(&face))
            .map(|(id, _)| id)
            .expect("parent face belongs to a shell");
        obj.shells[shell].faces.push(sub_face);

        obj.check_invariants();
        // Always-on backstop (generalized in): release WASM compiles
        // out check_invariants(), so re-validate and refuse — never commit — an
        // op that slipped past the up-front guards yet corrupted topology.
        obj.validate().map_err(|_| StickyError::WouldCorrupt)?;
        *self = obj;
        Ok(FaceSplitInnerReport {
            sub_face,
            parent: face,
            new_edges,
        })
    }

    /// Inverse of [`split_face_inner`]: dissolves an imprinted `sub_face` back
    /// into its parent, removing the loop and the parent's hole. Deletes the
    /// sub-face, its loop, the parent's matching hole loop, and the shared edges,
    /// half-edges, and vertices.
    ///
    /// # Errors
    /// [`StickyError::UnknownFace`] for a stale handle, [`StickyError::NotAnInnerFace`]
    /// if `sub_face` is not a clean imprinted island (its boundary not entirely
    /// twinned with one parent's hole loop). All leave the object untouched.
    pub fn merge_inner_face(
        &mut self,
        sub_face: FaceId,
    ) -> Result<FaceMergeInnerReport, StickyError> {
        if !self.faces.contains_key(sub_face) {
            return Err(StickyError::UnknownFace);
        }
        if !self.faces[sub_face].inner_loops.is_empty() {
            return Err(StickyError::NotAnInnerFace);
        }
        let sub_loop = self.faces[sub_face].outer_loop;
        let h_sub: Vec<HalfEdgeId> = self.loop_half_edges(sub_loop).collect();

        // Every sub-loop half-edge must be twinned onto a single Inner loop of a
        // single other parent face.
        let mut hole_loop: Option<LoopId> = None;
        let mut h_hole: Vec<HalfEdgeId> = Vec::with_capacity(h_sub.len());
        for &h in &h_sub {
            let t = self.half_edges[h].twin.ok_or(StickyError::NotAnInnerFace)?;
            let l = self.half_edges[t].loop_id;
            match hole_loop {
                None => hole_loop = Some(l),
                Some(hl) if hl == l => {}
                Some(_) => return Err(StickyError::NotAnInnerFace),
            }
            h_hole.push(t);
        }
        let hole_loop = hole_loop.ok_or(StickyError::NotAnInnerFace)?;
        if self.loops[hole_loop].kind != LoopKind::Inner {
            return Err(StickyError::NotAnInnerFace);
        }
        let parent = self.loops[hole_loop].face;
        if parent == sub_face {
            return Err(StickyError::NotAnInnerFace);
        }

        // Capture loop positions (sub-loop order) so the op is invertible (redo).
        let loop_path: Vec<Point3> = h_sub
            .iter()
            .map(|&h| self.vertices[self.half_edges[h].origin].position)
            .collect();
        // Snapshot the sub-face's attribute state so undo restores exactly
        // what it carried — see FaceMergeInnerReport::sub_face_attrs.
        let sub_face_attrs = self.faces[sub_face].attrs();

        // ---- removal surgery on a clone ----
        let mut obj = self.clone();
        let verts: Vec<VertexId> = h_sub.iter().map(|&h| obj.half_edges[h].origin).collect();
        obj.faces[parent].inner_loops.retain(|&l| l != hole_loop);
        for &h in &h_sub {
            obj.edges.remove(obj.half_edges[h].edge);
        }
        for &h in h_sub.iter().chain(h_hole.iter()) {
            obj.half_edges.remove(h);
        }
        obj.loops.remove(sub_loop);
        obj.loops.remove(hole_loop);
        obj.faces.remove(sub_face);
        for (_, s) in obj.shells.iter_mut() {
            s.faces.retain(|&f| f != sub_face);
        }
        for &v in &verts {
            obj.vertices.remove(v);
        }

        obj.check_invariants();
        // Always-on backstop.
        obj.validate().map_err(|_| StickyError::WouldCorrupt)?;
        *self = obj;
        Ok(FaceMergeInnerReport {
            parent,
            loop_path,
            sub_face_attrs,
        })
    }

    /// Whether `face` is a flat imprinted sub-face (its boundary entirely twinned
    /// with one coplanar parent's hole loop) — the kind [`extrude_sub_face`]
    /// raises into a boss or recess.
    pub fn is_flat_sub_face(&self, face: FaceId) -> bool {
        self.flat_sub_face(face).is_some()
    }

    /// `(parent, hole_loop, sub half-edges, hole half-edges)` if `face` is a flat
    /// imprinted sub-face, else `None`. `h_hole[k]` is the twin of `h_sub[k]`.
    fn flat_sub_face(
        &self,
        face: FaceId,
    ) -> Option<(FaceId, LoopId, Vec<HalfEdgeId>, Vec<HalfEdgeId>)> {
        if !self.faces.contains_key(face) || !self.faces[face].inner_loops.is_empty() {
            return None;
        }
        let sub_loop = self.faces[face].outer_loop;
        let h_sub: Vec<HalfEdgeId> = self.loop_half_edges(sub_loop).collect();
        if h_sub.len() < 3 {
            return None;
        }
        let mut hole_loop: Option<LoopId> = None;
        let mut h_hole = Vec::with_capacity(h_sub.len());
        for &h in &h_sub {
            let t = self.half_edges[h].twin?;
            let l = self.half_edges[t].loop_id;
            match hole_loop {
                None => hole_loop = Some(l),
                Some(hl) if hl == l => {}
                Some(_) => return None,
            }
            h_hole.push(t);
        }
        let hole_loop = hole_loop?;
        if self.loops[hole_loop].kind != LoopKind::Inner {
            return None;
        }
        let parent = self.loops[hole_loop].face;
        if parent == face {
            return None;
        }
        Some((parent, hole_loop, h_sub, h_hole))
    }

    /// Push/pull a flat imprinted sub-face by `distance` along its outward normal,
    /// generating fresh perpendicular walls between the moved sub-face and its
    /// parent's hole. Positive embosses a boss; negative recesses. Reversed by
    /// [`collapse_sub_face`]; handle-stable (the sub-face keeps its id).
    ///
    /// # Errors
    /// See [`PushPullError`]; [`PushPullError::NotASubFace`] if `sub_face` is not
    /// a flat imprinted sub-face. All leave the object untouched.
    pub fn extrude_sub_face(
        &mut self,
        sub_face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        self.extrude_sub_face_impl(sub_face, distance, GuardMode::Enforced)
    }

    /// [`extrude_sub_face`](Object::extrude_sub_face) in history-replay mode
    /// (DEVELOPMENT.md rule 9): the centroid-ray obstruction guard is
    /// skipped — the caller is the [`History`](crate::history::History),
    /// which dispatches only recorded inverses/redos and verifies the result
    /// against the recorded state's fingerprint before committing. Every
    /// structural refusal and the validator backstop still run.
    pub(crate) fn extrude_sub_face_replay(
        &mut self,
        sub_face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        self.extrude_sub_face_impl(sub_face, distance, GuardMode::Replay)
    }

    fn extrude_sub_face_impl(
        &mut self,
        sub_face: FaceId,
        distance: f64,
        guards: GuardMode,
    ) -> Result<PushPullReport, PushPullError> {
        if self.watertight != WatertightState::Watertight {
            return Err(PushPullError::ObjectNotSolid);
        }
        if distance.abs() < tol::POINT_MERGE {
            return Err(PushPullError::DistanceTooSmall);
        }
        let (parent, _hole_loop, h_sub, h_hole) = self
            .flat_sub_face(sub_face)
            .ok_or(PushPullError::NotASubFace)?;
        let n = h_sub.len();
        let normal = self.faces[sub_face].plane.normal();
        let sweep = normal * distance;

        // Obstruction guard: unlike `push_pull`, an extrusion has no
        // push-through semantics — a recess deeper than the material under
        // the sub-face (or a boss driven through geometry in front of it)
        // would self-intersect while staying manifold, invisibly to the
        // validator. Probe with a ray from the ring's vertex average along
        // the sweep and refuse at the nearest face it crosses. Best-effort
        // like `push_pull`'s neighbor-vertex guard: for a non-convex ring the
        // vertex average can fall outside the sub-face, where the probe would
        // test a line the sweep never occupies and refuse legal extrusions
        // (including recorded inverses, which must never fail) — the guard is
        // skipped there rather than guessed. Skipped entirely on history
        // replay (rule 9): the collapse this extrusion reverses recorded the
        // raised state's fingerprint, and a centroid ray probed at replay
        // time can see the opposite face closer than the recess depth and
        // refuse a state that was already accepted.
        if guards == GuardMode::Enforced {
            let ring: Vec<Point3> = self
                .loop_positions(self.faces[sub_face].outer_loop)
                .collect();
            let inv = 1.0 / ring.len() as f64;
            let centroid = ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
            });
            let mut limit = f64::INFINITY;
            if point_inside_polygon(centroid, &ring, normal) {
                let dir = normal * distance.signum();
                for (f, face_data) in &self.faces {
                    if f == sub_face || f == parent {
                        continue;
                    }
                    let denom = face_data.plane.normal().dot(dir);
                    if denom.abs() < tol::NORMAL_DIRECTION {
                        continue; // ray parallel to the face's plane
                    }
                    let t = -face_data.plane.signed_distance(centroid) / denom;
                    if t <= tol::POINT_MERGE || t >= limit {
                        continue;
                    }
                    let hit = centroid + dir * t;
                    let outer: Vec<Point3> = self.loop_positions(face_data.outer_loop).collect();
                    // A hit anywhere inside the outer ring counts — including
                    // inside a hole: the swept region is wider than one ray,
                    // so crossing an annular face's opening still intersects
                    // its material for any realistically sized sub-face, and
                    // the validator cannot catch what slips through here.
                    if point_inside_polygon(hit, &outer, face_data.plane.normal()) {
                        limit = t;
                    }
                }
            }
            if distance.abs() >= limit - tol::POINT_MERGE {
                return Err(PushPullError::NonManifoldResult);
            }
        }

        let mut obj = self.clone();

        // The shared loop vertices stay with the parent hole; the sub-face gets a
        // fresh raised copy `vt`.
        let a: Vec<VertexId> = h_sub.iter().map(|&h| obj.half_edges[h].origin).collect();
        let vt: Vec<VertexId> = a
            .iter()
            .map(|&v| {
                let p = obj.vertices[v].position + sweep;
                obj.vertices.insert(Vertex {
                    position: p,
                    outgoing: HalfEdgeId::default(),
                })
            })
            .collect();
        // The old shared edge of each sub/hole pair: the hole keeps it, the
        // sub-face's half-edge gets a fresh one.
        let old_edge: Vec<EdgeId> = h_sub.iter().map(|&h| obj.half_edges[h].edge).collect();

        // Re-point the sub-face loop onto the raised vertices; hand the old shared
        // vertices to the hole loop.
        for k in 0..n {
            obj.half_edges[h_sub[k]].origin = vt[k];
            obj.vertices[vt[k]].outgoing = h_sub[k];
            obj.vertices[a[k]].outgoing = h_hole[(k + n - 1) % n];
        }

        // One quad wall per edge. wa twins the sub edge, wc the hole edge,
        // wb/wd the verticals (shared with neighbouring walls).
        let mut wa = vec![HalfEdgeId::default(); n];
        let mut wb = vec![HalfEdgeId::default(); n];
        let mut wc = vec![HalfEdgeId::default(); n];
        let mut wd = vec![HalfEdgeId::default(); n];
        let mut walls = Vec::with_capacity(n);
        for k in 0..n {
            let wloop = obj.loops.insert(Loop {
                face: FaceId::default(),
                first_half_edge: HalfEdgeId::default(),
                kind: LoopKind::Outer,
            });
            let mk = |origin: VertexId, obj: &mut Object| {
                obj.half_edges.insert(HalfEdge {
                    origin,
                    twin: None,
                    next: HalfEdgeId::default(),
                    prev: HalfEdgeId::default(),
                    edge: EdgeId::default(),
                    loop_id: wloop,
                })
            };
            wa[k] = mk(vt[(k + 1) % n], &mut obj); // vt[k+1] -> vt[k]
            wb[k] = mk(vt[k], &mut obj); // vt[k] -> a[k]
            wc[k] = mk(a[k], &mut obj); // a[k] -> a[k+1]
            wd[k] = mk(a[(k + 1) % n], &mut obj); // a[k+1] -> vt[k+1]
            obj.half_edges[wa[k]].next = wb[k];
            obj.half_edges[wb[k]].next = wc[k];
            obj.half_edges[wc[k]].next = wd[k];
            obj.half_edges[wd[k]].next = wa[k];
            obj.half_edges[wb[k]].prev = wa[k];
            obj.half_edges[wc[k]].prev = wb[k];
            obj.half_edges[wd[k]].prev = wc[k];
            obj.half_edges[wa[k]].prev = wd[k];
            obj.loops[wloop].first_half_edge = wa[k];
            let wall_pts = [
                obj.vertices[vt[(k + 1) % n]].position,
                obj.vertices[vt[k]].position,
                obj.vertices[a[k]].position,
                obj.vertices[a[(k + 1) % n]].position,
            ];
            let plane =
                Plane::from_polygon(&wall_pts).map_err(|_| PushPullError::NonManifoldResult)?;
            let wface = obj.faces.insert(Face {
                outer_loop: wloop,
                inner_loops: Vec::new(),
                plane,
                // Freshly generated push/pull side walls take the default
                // material, no UV frame, and no analytic surface.
                material: None,
                uv_frame: None,
                surface: None,
            });
            obj.loops[wloop].face = wface;
            walls.push(wface);
        }

        // Twins + edges.
        for k in 0..n {
            // wa[k] ↔ h_sub[k] (new edge).
            obj.half_edges[wa[k]].twin = Some(h_sub[k]);
            obj.half_edges[h_sub[k]].twin = Some(wa[k]);
            let e_top = obj.edges.insert(Edge {
                half_edge: h_sub[k],
                twin_half_edge: Some(wa[k]),
                curve: None,
            });
            obj.half_edges[h_sub[k]].edge = e_top;
            obj.half_edges[wa[k]].edge = e_top;
            // wc[k] ↔ h_hole[k] (reuse the old shared edge).
            obj.half_edges[wc[k]].twin = Some(h_hole[k]);
            obj.half_edges[h_hole[k]].twin = Some(wc[k]);
            obj.edges[old_edge[k]].half_edge = h_hole[k];
            obj.edges[old_edge[k]].twin_half_edge = Some(wc[k]);
            obj.half_edges[h_hole[k]].edge = old_edge[k];
            obj.half_edges[wc[k]].edge = old_edge[k];
            // wb[k] ↔ wd[k-1] (vertical, new edge).
            let prev = (k + n - 1) % n;
            obj.half_edges[wb[k]].twin = Some(wd[prev]);
            obj.half_edges[wd[prev]].twin = Some(wb[k]);
            let e_vert = obj.edges.insert(Edge {
                half_edge: wb[k],
                twin_half_edge: Some(wd[prev]),
                curve: None,
            });
            obj.half_edges[wb[k]].edge = e_vert;
            obj.half_edges[wd[prev]].edge = e_vert;
        }

        // Refit the moved sub-face plane (translated; same normal).
        refit_face_plane(&mut obj, sub_face)?;
        // The raised sub-face left its chord plane: its inherited analytic
        // surface claim drops (map-or-drop, docs/design/true-curves.md).
        obj.faces[sub_face].surface = None;

        let shell = obj
            .shells
            .iter()
            .find(|(_, s)| s.faces.contains(&parent))
            .map(|(id, _)| id)
            .expect("parent face belongs to a shell");
        for &w in &walls {
            obj.shells[shell].faces.push(w);
        }
        // Defensive map-or-drop: the raised sub-face is fresh geometry (its new
        // edges carry no claim and the base ring is unmoved), so this is a
        // no-op today, but every subset-moving op examines Edge::curve rather
        // than leaving a claim unchecked (docs/design/true-curves.md §4.2).
        obj.drop_stale_edge_curves();

        obj.check_invariants();
        // Always-on backstop.
        obj.validate()
            .map_err(|_| PushPullError::NonManifoldResult)?;
        *self = obj;
        Ok(PushPullReport {
            face: sub_face,
            created_faces: walls,
            removed_faces: vec![],
            // extrude_sub_face has its own inverse (collapse_sub_face).
            requires_unbuild_inverse: false,
        })
    }

    /// Inverse of [`extrude_sub_face`]: flattens a raised sub-face back into its
    /// parent, removing the generated walls. The sub-face keeps its handle. The
    /// reported `distance` is how far it was raised (re-extruding restores it).
    ///
    /// # Errors
    /// [`PushPullError::NotASubFace`] if `sub_face` is not a raised sub-face (its
    /// boundary twinned with quad walls that bridge to one parent's hole). Leaves
    /// the object untouched on error.
    pub fn collapse_sub_face(
        &mut self,
        sub_face: FaceId,
    ) -> Result<CollapseSubFaceReport, PushPullError> {
        if !self.faces.contains_key(sub_face) || !self.faces[sub_face].inner_loops.is_empty() {
            return Err(PushPullError::NotASubFace);
        }
        let sub_loop = self.faces[sub_face].outer_loop;
        let h_sub: Vec<HalfEdgeId> = self.loop_half_edges(sub_loop).collect();
        let n = h_sub.len();
        if n < 3 {
            return Err(PushPullError::NotASubFace);
        }

        // Walk each wall: wa twins the sub edge; the quad is wa→wb→wc→wd; wc
        // twins the parent's hole edge.
        let mut wa = vec![HalfEdgeId::default(); n];
        let mut wb = vec![HalfEdgeId::default(); n];
        let mut wc = vec![HalfEdgeId::default(); n];
        let mut wd = vec![HalfEdgeId::default(); n];
        let mut walls = Vec::with_capacity(n);
        let mut h_hole = vec![HalfEdgeId::default(); n];
        let mut hole_loop: Option<LoopId> = None;
        for k in 0..n {
            let a = self.half_edges[h_sub[k]]
                .twin
                .ok_or(PushPullError::NotASubFace)?;
            wa[k] = a;
            wb[k] = self.half_edges[a].next;
            wc[k] = self.half_edges[wb[k]].next;
            wd[k] = self.half_edges[wc[k]].next;
            if self.half_edges[wd[k]].next != a {
                return Err(PushPullError::NotASubFace); // not a quad wall
            }
            let wall_face = self.loops[self.half_edges[a].loop_id].face;
            // A wall that has been built upon (it carries imprinted rings) is
            // no longer a sacrificial quad: removing it would orphan its
            // inner loops and their sub-faces. (History unwinds are immune —
            // LIFO order undoes the imprint before the extrude — so this
            // refusal only ever hits forward collapses.)
            if !self.faces[wall_face].inner_loops.is_empty() {
                return Err(PushPullError::NotASubFace);
            }
            walls.push(wall_face);
            let hh = self.half_edges[wc[k]]
                .twin
                .ok_or(PushPullError::NotASubFace)?;
            h_hole[k] = hh;
            let l = self.half_edges[hh].loop_id;
            match hole_loop {
                None => hole_loop = Some(l),
                Some(hl) if hl == l => {}
                Some(_) => return Err(PushPullError::NotASubFace),
            }
        }
        let hole_loop = hole_loop.ok_or(PushPullError::NotASubFace)?;
        if self.loops[hole_loop].kind != LoopKind::Inner {
            return Err(PushPullError::NotASubFace);
        }
        let parent = self.loops[hole_loop].face;
        if parent == sub_face {
            return Err(PushPullError::NotASubFace);
        }

        // a[k] (parent hole vertex) = origin of wc[k]; vt[k] = current raised
        // sub-face vertex = origin of h_sub[k]. The raise distance is the offset
        // along the normal.
        let a: Vec<VertexId> = (0..n).map(|k| self.half_edges[wc[k]].origin).collect();
        let vt: Vec<VertexId> = (0..n).map(|k| self.half_edges[h_sub[k]].origin).collect();
        let normal = self.faces[parent].plane.normal();
        let distance = normal.dot(self.vertices[vt[0]].position - self.vertices[a[0]].position);

        // Every vertical must be exactly the sweep: vt[k] = a[k] + normal·d.
        // A wall subdivided by a later corner-to-corner cut still presents
        // quad pieces whose shape and hole-loop wiring masquerade as
        // sacrificial walls, but the cut edge poses as a NON-sweep-aligned
        // "vertical" — collapsing would silently destroy that user geometry
        // and orphan the cut's recorded inverse. Refuse instead.
        // Gate at the object's planarity regime: native geometry constructs
        // verticals exactly (POINT_MERGE-scale noise at most), but imported
        // objects carry f32 quantization and Newell-fitted normals, so their
        // legitimate verticals deviate up to the same tolerance the validator
        // itself honors for their faces.
        let sweep = normal * distance;
        let gate = self.planarity_tol.max(tol::POINT_MERGE);
        for k in 0..n {
            let delta = self.vertices[vt[k]].position - self.vertices[a[k]].position;
            if (delta - sweep).length() > gate {
                return Err(PushPullError::NotASubFace);
            }
        }

        // ---- removal surgery on a clone ----
        let mut obj = self.clone();
        for k in 0..n {
            // Restore the shared sub/hole edge, reusing the hole's current edge.
            let shared = obj.half_edges[h_hole[k]].edge;
            obj.edges[shared].half_edge = h_sub[k];
            obj.edges[shared].twin_half_edge = Some(h_hole[k]);
            obj.half_edges[h_sub[k]].edge = shared;
            obj.half_edges[h_sub[k]].twin = Some(h_hole[k]);
            obj.half_edges[h_hole[k]].twin = Some(h_sub[k]);
            obj.half_edges[h_sub[k]].origin = a[k];
            obj.vertices[a[k]].outgoing = h_sub[k];

            // Delete the sub-face's top edge and the vertical edge.
            obj.edges.remove(obj.half_edges[wa[k]].edge);
            obj.edges.remove(obj.half_edges[wb[k]].edge);
        }
        // Delete wall half-edges, loops, faces, and the raised vertices.
        for k in 0..n {
            for &h in &[wa[k], wb[k], wc[k], wd[k]] {
                let lp = obj.half_edges[h].loop_id;
                obj.half_edges.remove(h);
                obj.loops.remove(lp);
            }
        }
        for &w in &walls {
            obj.faces.remove(w);
        }
        for (_, s) in obj.shells.iter_mut() {
            s.faces.retain(|f| !walls.contains(f));
        }
        for &v in &vt {
            obj.vertices.remove(v);
        }
        // The sub-face is flat again: same plane as the parent.
        obj.faces[sub_face].plane = obj.faces[parent].plane;
        // Welding the raised sub-face back down moves its vertices; drop any
        // per-edge claim that no longer holds (map-or-drop for Edge::curve).
        obj.drop_stale_edge_curves();

        obj.check_invariants();
        // Always-on backstop.
        obj.validate()
            .map_err(|_| PushPullError::NonManifoldResult)?;
        *self = obj;
        Ok(CollapseSubFaceReport { sub_face, distance })
    }

    /// Sticky rule inside an Object: cuts `face` in two along `path`, whose
    /// endpoints lie on the face boundary (snapping to vertices, or splitting
    /// edges, within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE)) and whose interior points lie
    /// strictly inside the face on its plane.
    ///
    /// The cut inserts twin half-edge pairs, so the mesh stays manifold and
    /// the watertightness state cannot change. Partial strokes are not
    /// accepted — see the module-level "no dangling edges" decision.
    ///
    /// M1 scope: cut endpoints anchor on the face's **outer** loop only.
    /// If an endpoint matches an inner (hole) loop vertex or edge instead of
    /// the outer boundary, `EndpointNotOnBoundary` is returned — M1 cuts
    /// anchor on the outer boundary only; inner-loop endpoints are reserved
    /// for a future milestone.
    ///
    /// # Errors
    /// See [`StickyError`]; all leave the object untouched.
    pub fn split_face(
        &mut self,
        face: FaceId,
        path: &[Point3],
    ) -> Result<FaceSplitReport, StickyError> {
        self.split_face_with_attrs(face, path, None)
    }

    /// [`Object::split_face`] with an explicit attribute restoration: after
    /// the split, each present entry of `restore` is applied to whichever
    /// result face contains its interior point. This is undo's path for
    /// reversing a `merge_faces` — the merge report snapshotted each
    /// pre-merge face's attributes ([`FaceMergeReport::prior_attrs`]) so the
    /// two sides come back with exactly what they carried, never re-derived
    /// copies of the merged survivor's. An absent per-face entry is the
    /// snapshot's best-effort fallback (a face too thin to pin an interior
    /// point in): that face inherits instead. `restore: None` = both result
    /// faces inherit the split face's attributes (the forward/tool
    /// semantics).
    pub fn split_face_with_attrs(
        &mut self,
        face: FaceId,
        path: &[Point3],
        restore: Option<[Option<FaceAttrsAt>; 2]>,
    ) -> Result<FaceSplitReport, StickyError> {
        // --- validation (before any mutation) ---
        if !self.faces.contains_key(face) {
            return Err(StickyError::UnknownFace);
        }
        if path.len() < 2 {
            return Err(StickyError::PathTooShort);
        }

        let face_plane = self.faces[face].plane;
        let outer_loop = self.faces[face].outer_loop;
        let inner_loops: Vec<LoopId> = self.faces[face].inner_loops.clone();

        // Collect outer loop half-edges and vertex positions.
        let outer_hes: Vec<HalfEdgeId> = self.loop_half_edges(outer_loop).collect();

        // --- classify endpoints against the outer loop ---
        let ep0 = classify_endpoint(self, &outer_hes, path[0])?;
        let ep1 = classify_endpoint(self, &outer_hes, *path.last().unwrap())?;

        // Check: if classify returned None (not on outer loop), check inner loops;
        // if on inner loop or truly interior, return EndpointNotOnBoundary.
        let ep0 = match ep0 {
            Some(ep) => ep,
            None => {
                // Check if it's on any inner loop — if so, EndpointNotOnBoundary (M1 scope)
                // If it's truly interior, also EndpointNotOnBoundary.
                return Err(StickyError::EndpointNotOnBoundary { which: 0 });
            }
        };
        let ep1 = match ep1 {
            Some(ep) => ep,
            None => {
                return Err(StickyError::EndpointNotOnBoundary { which: 1 });
            }
        };

        // Two endpoints can't be the same boundary location (would produce a zero-length cut).
        // This is caught by pathsimplicity / PathNotSimple.  But also, if they both land
        // exactly on the same vertex, the path is degenerate.
        // We check simplicity of interior segments below.

        // --- validate interior path points ---
        // Build outer loop vertex positions for point-in-polygon test.
        let outer_pts: Vec<Point3> = outer_hes
            .iter()
            .map(|&h| self.vertices[self.half_edges[h].origin].position)
            .collect();

        // Build inner loop vertex positions for hole containment check.
        let hole_pts: Vec<Vec<Point3>> = inner_loops
            .iter()
            .map(|&il| self.loop_positions(il).collect())
            .collect();

        // Validate interior points (indices 1..path.len()-1).
        for (idx, &pt) in path.iter().enumerate().skip(1).take(path.len() - 2) {
            // Must be on the face plane.
            if face_plane.signed_distance(pt).abs() > tol::PLANE_DIST {
                return Err(StickyError::PointNotOnFace { index: idx });
            }
            // Must be strictly inside the face (inside outer boundary, outside all holes).
            if !point_inside_polygon(pt, &outer_pts, face_plane.normal()) {
                return Err(StickyError::PointNotOnFace { index: idx });
            }
            for hole in &hole_pts {
                if point_inside_polygon(pt, hole, face_plane.normal()) {
                    return Err(StickyError::PointNotOnFace { index: idx });
                }
            }
        }

        // --- check path simplicity ---
        // The interior polyline must not self-intersect or graze a hole boundary.
        // Build the full path point list (using actual resolved positions for endpoints).
        let ep0_pos = endpoint_position(self, &ep0);
        let ep1_pos = endpoint_position(self, &ep1);

        let mut resolved_path: Vec<Point3> = Vec::with_capacity(path.len());
        resolved_path.push(ep0_pos);
        resolved_path.extend_from_slice(&path[1..path.len() - 1]);
        resolved_path.push(ep1_pos);

        // Check path self-intersections (interior segments only — not adjacent).
        let n_seg = resolved_path.len() - 1;
        for i in 0..n_seg {
            let a = resolved_path[i];
            let b = resolved_path[i + 1];
            for j in (i + 2)..n_seg {
                // Skip adjacent segments.
                if i == 0 && j == n_seg - 1 {
                    continue;
                }
                let c = resolved_path[j];
                let d = resolved_path[j + 1];
                if segments_intersect(a, b, c, d) {
                    return Err(StickyError::PathNotSimple);
                }
            }
        }
        // Check path segments against hole boundaries.
        for hole in &hole_pts {
            let nh = hole.len();
            for i in 0..n_seg {
                let a = resolved_path[i];
                let b = resolved_path[i + 1];
                for j in 0..nh {
                    let c = hole[j];
                    let d = hole[(j + 1) % nh];
                    if segments_intersect(a, b, c, d) {
                        return Err(StickyError::PathNotSimple);
                    }
                }
            }
        }

        // The open interior of every cut segment must stay strictly inside
        // the face. Endpoints anchor ON the boundary by construction, but a
        // segment interior that touches it — running along a boundary edge,
        // passing through a boundary vertex, or spanning a concave notch
        // outside the face — would not produce exactly two faces (the
        // PathNotSimple contract). The interior-POINT checks above cannot see
        // these cases: a two-point chord has no interior points at all.
        let boundary_polys: Vec<&[Point3]> = std::iter::once(outer_pts.as_slice())
            .chain(hole_pts.iter().map(|h| h.as_slice()))
            .collect();
        for w in resolved_path.windows(2) {
            let (a, b) = (w[0], w[1]);
            // No boundary vertex may lie strictly inside a cut segment.
            for poly in &boundary_polys {
                for &v in *poly {
                    if point_near_segment(v, a, b, tol::POINT_MERGE)
                        && !v.approx_eq(a, tol::POINT_MERGE)
                        && !v.approx_eq(b, tol::POINT_MERGE)
                    {
                        return Err(StickyError::PathNotSimple);
                    }
                }
            }
            // The midpoint must be strictly inside the face and clear of the
            // boundary (catches chords collinear with a boundary edge and
            // chords spanning a notch outside the face).
            let mid = a + (b - a) * 0.5;
            if !point_inside_polygon(mid, &outer_pts, face_plane.normal()) {
                return Err(StickyError::PathNotSimple);
            }
            for hole in &hole_pts {
                if point_inside_polygon(mid, hole, face_plane.normal()) {
                    return Err(StickyError::PathNotSimple);
                }
            }
            for poly in &boundary_polys {
                let n = poly.len();
                for i in 0..n {
                    if point_near_segment(mid, poly[i], poly[(i + 1) % n], tol::POINT_MERGE) {
                        return Err(StickyError::PathNotSimple);
                    }
                }
            }
        }
        // A cut segment may not CROSS the outer boundary at all (a midpoint
        // sample cannot see a chord that exits through an off-center concave
        // notch and re-enters). Contact at the two anchored endpoints is the
        // only legal touch: skip boundary segments the resolved endpoints lie
        // on, and only for the path's first/last segment respectively.
        let n_outer = outer_pts.len();
        for (si, w) in resolved_path.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            for i in 0..n_outer {
                let (c, d) = (outer_pts[i], outer_pts[(i + 1) % n_outer]);
                let anchor_exempt = (si == 0 && point_near_segment(a, c, d, tol::POINT_MERGE))
                    || (si == n_seg - 1 && point_near_segment(b, c, d, tol::POINT_MERGE));
                if anchor_exempt {
                    continue;
                }
                if segments_intersect(a, b, c, d) {
                    return Err(StickyError::PathNotSimple);
                }
            }
        }

        // --- all checks passed — clone and mutate ---
        let mut obj = self.clone();
        let report = do_split_face(&mut obj, face, path, &ep0, &ep1)?;
        if let Some(restore) = restore {
            apply_split_restore(&mut obj, &report.new_faces, &restore);
        }
        obj.check_invariants();
        // Release-safe backstop: `check_invariants` is compiled out of release
        // builds (the shipped wasm), so a path that slips past the up-front
        // checks yet corrupts topology (a near-degenerate cut from a noisy UI
        // snap) would otherwise commit and panic on a later access. The
        // always-on validator refuses it here, leaving `self` untouched.
        obj.validate().map_err(|_| StickyError::WouldCorrupt)?;
        *self = obj;
        Ok(report)
    }

    /// Inverse sticky rule: dissolves the boundary between the two coplanar
    /// faces adjacent to `edge`, merging them into one face. If the two faces
    /// share a chain of edges, the entire shared chain dissolves (a single
    /// face cannot have two disconnected boundaries to the same neighbor).
    ///
    /// `split_face` followed by `merge_faces` on any returned cut edge is the
    /// identity (up to handle renaming). Chain-endpoint vertices left with
    /// exactly two collinear edges are healed away (the edges merge), so a
    /// split followed by a merge leaves no scar — this is what makes the
    /// identity property hold.
    ///
    /// # Errors
    /// See [`StickyError`]; all leave the object untouched.
    pub fn merge_faces(&mut self, edge: EdgeId) -> Result<FaceMergeReport, StickyError> {
        // --- validation ---
        let edge_data = match self.edges.get(edge) {
            Some(e) => *e,
            None => return Err(StickyError::UnknownEdge),
        };

        let twin_he_id = match edge_data.twin_half_edge {
            Some(t) => t,
            None => return Err(StickyError::BoundaryEdge),
        };

        let he_id = edge_data.half_edge;
        let he = self.half_edges[he_id];
        let twin_he = self.half_edges[twin_he_id];

        // Get the two loops (and thus two faces).
        let loop_a = he.loop_id;
        let loop_b = twin_he.loop_id;

        let face_a = self.loops[loop_a].face;
        let face_b = self.loops[loop_b].face;

        if face_a == face_b {
            return Err(StickyError::SameFaceOnBothSides);
        }

        // Both must be outer loops (inner loops cannot be merged via this op).
        if self.loops[loop_a].kind != LoopKind::Outer || self.loops[loop_b].kind != LoopKind::Outer
        {
            // Edge is between inner loops — not a supported merge path.
            return Err(StickyError::BoundaryEdge);
        }

        // Check coplanarity of the two faces.
        let plane_a = self.faces[face_a].plane;
        let plane_b = self.faces[face_b].plane;
        let normal_diff = (plane_a.normal() - plane_b.normal()).length();
        let normal_diff2 = (plane_a.normal() + plane_b.normal()).length();
        let normals_parallel =
            normal_diff < tol::NORMAL_DIRECTION || normal_diff2 < tol::NORMAL_DIRECTION;
        if !normals_parallel {
            return Err(StickyError::FacesNotCoplanar);
        }
        // Check that the planes are the same (not antiparallel opposite planes).
        // For a manifold solid, coplanar faces with same normal are on the same plane.
        // Verify via the plane distance: a vertex of face_b should be on plane_a.
        let sample_b = {
            let loop_id = self.faces[face_b].outer_loop;
            let h = self.loops[loop_id].first_half_edge;
            self.vertices[self.half_edges[h].origin].position
        };
        if plane_a.signed_distance(sample_b).abs() > tol::PLANE_DIST {
            return Err(StickyError::FacesNotCoplanar);
        }

        // --- clone and mutate ---
        let mut obj = self.clone();
        let report = do_merge_faces(&mut obj, face_a, face_b)?;
        obj.check_invariants();
        // Always-on backstop.
        obj.validate().map_err(|_| StickyError::WouldCorrupt)?;
        *self = obj;
        Ok(report)
    }

    /// The interior edges whose two adjacent faces are coplanar and
    /// identically painted — the edges [`Object::merge_coplanar_faces`]
    /// WOULD dissolve — as world-space endpoint segments.
    ///
    /// Callers about to run a boolean collect these from each operand and
    /// pass them as the result's `preserve` set: such an edge is a deliberate
    /// artifact (a face imprint drawn but not yet extruded), not a seam the
    /// boolean introduced, and must survive the cleanup. Pure query.
    pub fn coplanar_edge_segments(&self) -> Vec<(Point3, Point3)> {
        let mut out = Vec::new();
        for (_eid, edge) in &self.edges {
            let Some((a, b)) = self.mergeable_edge_endpoints(edge) else {
                continue;
            };
            out.push((a, b));
        }
        out
    }

    /// Dissolve every interior edge whose two adjacent faces are coplanar
    /// and identically painted, repeating until none remain — the cleanup
    /// run on boolean results (union / subtract / intersect and through-cut
    /// push/pull) so the seam where two solids joined does not linger as a
    /// stray edge across what is now one continuous face.
    ///
    /// `preserve` holds edge segments that must NOT be dissolved even though
    /// they qualify: the operands' pre-existing coplanar edges (face imprints
    /// awaiting push/pull — see [`Object::coplanar_edge_segments`]). A
    /// candidate is kept when its midpoint lies on any preserve segment
    /// (within [`tol::POINT_MERGE`]), which also protects imprint edges the
    /// boolean trimmed into shorter pieces.
    ///
    /// Differing face materials or UV frames are a hard stop (a painted face
    /// never bleeds into its neighbor), and edges [`Object::merge_faces`]
    /// refuses (boundary, hole loop, same face on both sides, not coplanar
    /// within tolerance) are skipped, never forced — every dissolve goes
    /// through that validated primitive, so watertightness is preserved by
    /// construction. Returns the number of edges dissolved.
    pub fn merge_coplanar_faces(&mut self, preserve: &[(Point3, Point3)]) -> usize {
        let mut dissolved = 0;
        loop {
            // Fresh scan each round: a merge deletes its edge and one face,
            // and can expose new coplanar pairs. Slotmap key order is
            // deterministic for a deterministically built object, so the
            // merge order (and thus the result) is reproducible.
            let candidates: Vec<EdgeId> = self.edges.keys().collect();
            let mut progress = false;
            for eid in candidates {
                // An earlier merge this round may have deleted the edge.
                let Some(edge) = self.edges.get(eid).copied() else {
                    continue;
                };
                if self.mergeable_edge_endpoints(&edge).is_none() {
                    continue;
                }
                // merge_faces dissolves EVERY edge the two faces share, not
                // just this candidate — so the preserve check must cover the
                // pair's whole shared set, or a preserved imprint that joined
                // the chain through an earlier merge would die as collateral.
                let chain = self.shared_face_pair_segments(&edge);
                let preserved = chain.iter().any(|&(a, b)| {
                    let mid = Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0);
                    preserve
                        .iter()
                        .any(|&(p, q)| point_on_segment(mid, p, q, tol::POINT_MERGE))
                });
                if preserved {
                    continue;
                }
                if self.merge_faces(eid).is_ok() {
                    dissolved += 1;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        dissolved
    }

    /// If `edge` separates two distinct, identically-painted, coplanar faces
    /// (both bounded by outer loops — the same preconditions
    /// [`Object::merge_faces`] enforces), its world endpoints; else `None`.
    /// The shared geometry test behind [`Object::coplanar_edge_segments`] and
    /// [`Object::merge_coplanar_faces`]'s candidate scan.
    fn mergeable_edge_endpoints(&self, edge: &Edge) -> Option<(Point3, Point3)> {
        let twin_he_id = edge.twin_half_edge?;
        let he = self.half_edges[edge.half_edge];
        let twin_he = self.half_edges[twin_he_id];

        let loop_a = he.loop_id;
        let loop_b = twin_he.loop_id;
        if self.loops[loop_a].kind != LoopKind::Outer || self.loops[loop_b].kind != LoopKind::Outer
        {
            return None;
        }

        let face_a = self.loops[loop_a].face;
        let face_b = self.loops[loop_b].face;
        if face_a == face_b {
            return None;
        }

        let fa = &self.faces[face_a];
        let fb = &self.faces[face_b];
        if fa.material != fb.material || fa.uv_frame != fb.uv_frame || fa.surface != fb.surface {
            return None;
        }

        // Same coplanarity test merge_faces itself validates with.
        let normal_diff = (fa.plane.normal() - fb.plane.normal()).length();
        let normal_diff2 = (fa.plane.normal() + fb.plane.normal()).length();
        if normal_diff >= tol::NORMAL_DIRECTION && normal_diff2 >= tol::NORMAL_DIRECTION {
            return None;
        }
        let sample_b = {
            let h = self.loops[fb.outer_loop].first_half_edge;
            self.vertices[self.half_edges[h].origin].position
        };
        if fa.plane.signed_distance(sample_b).abs() > tol::PLANE_DIST {
            return None;
        }

        let a = self.vertices[he.origin].position;
        let b = self.vertices[twin_he.origin].position;
        Some((a, b))
    }

    /// Every edge shared between the two faces adjacent to `edge` (a
    /// half-edge on one face's outer loop twinned into the other's), as
    /// world segments — the full set a `merge_faces` on that edge would
    /// dissolve, connected or not. Empty for a boundary edge.
    fn shared_face_pair_segments(&self, edge: &Edge) -> Vec<(Point3, Point3)> {
        let Some(twin_he_id) = edge.twin_half_edge else {
            return Vec::new();
        };
        let loop_a = self.half_edges[edge.half_edge].loop_id;
        let loop_b = self.half_edges[twin_he_id].loop_id;
        let mut out = Vec::new();
        for h in self.loop_half_edges(loop_a) {
            let hh = self.half_edges[h];
            if let Some(t) = hh.twin
                && self.half_edges[t].loop_id == loop_b
            {
                let a = self.vertices[hh.origin].position;
                let b = self.vertices[self.half_edges[hh.next].origin].position;
                out.push((a, b));
            }
        }
        out
    }

    /// Explicit combination of two solids — the only way Objects ever join
    /// (ARCHITECTURE.md: no implicit welding, ever).
    ///
    /// `b_to_a` maps `b`'s frame into `a`'s; the result lives in `a`'s frame.
    /// Inputs are untouched (non-destructive at the kernel level; the
    /// document layer decides what replaces what). The result is watertight
    /// and may have multiple shells (e.g., subtracting a slab that cuts `a`
    /// in two).
    ///
    /// # Errors
    /// See [`BooleanError`]. Degenerate contact (coincident faces, tangent
    /// edges) is refused, not repaired — resolution strategies (nudging,
    /// merge-group containers) belong to M2 UX.
    pub fn boolean(
        op: BooleanOp,
        a: &Object,
        b: &Object,
        b_to_a: &Transform,
    ) -> Result<Object, BooleanError> {
        crate::boolean::execute(op, a, b, b_to_a)
    }

    /// Partition this Object's faces into connected components by shared-edge
    /// (twin) adjacency, rebuilding each component as its own independent
    /// Object.
    ///
    /// Two faces are in the same component iff a chain of shared edges connects
    /// them; a vertex-only touch does not connect them (and never arises in a
    /// valid manifold). A connected Object yields a single-element `Vec`
    /// (returned as a clone, untouched). A multi-shell Object — e.g. the result
    /// of a cut that severed a solid into disjoint pieces — yields one Object
    /// per shell.
    ///
    /// Each split-out component is rebuilt via [`Object::from_faces_with_holes`]
    /// so per-face material / UV frames and the outer+inner loop structure are
    /// preserved; this object's `default_material` and `planarity_tol` carry
    /// over. Results are validated in debug builds before return.
    ///
    /// This is the shared primitive behind Slice and a push-through
    /// subtract that fully severs a solid: the boolean/cut machinery may
    /// leave one Object holding two disjoint watertight shells, which this
    /// splits back into independent solids the document can own separately.
    pub fn split_connected_components(&self) -> Vec<Object> {
        // Faces in deterministic slotmap order, with a dense index for union-find.
        let face_ids: Vec<FaceId> = self.faces.keys().collect();
        if face_ids.len() <= 1 {
            return vec![self.clone()];
        }
        let index_of: std::collections::BTreeMap<FaceId, usize> =
            face_ids.iter().enumerate().map(|(i, &f)| (f, i)).collect();

        // Union-find with path compression. `find` is iterative (no recursion).
        let mut parent: Vec<usize> = (0..face_ids.len()).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut root = x;
            while parent[root] != root {
                root = parent[root];
            }
            let mut cur = x;
            while parent[cur] != root {
                let next = parent[cur];
                parent[cur] = root;
                cur = next;
            }
            root
        }

        // Union faces that share an edge (a half-edge with its twin on the other
        // face). Iterate half-edges in slotmap order for determinism.
        for he in self.half_edges.values() {
            if let Some(twin) = he.twin {
                let fa = self.loops[he.loop_id].face;
                let fb = self.loops[self.half_edges[twin].loop_id].face;
                let ra = find(&mut parent, index_of[&fa]);
                let rb = find(&mut parent, index_of[&fb]);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }

        // Group faces by root, preserving first-appearance order of roots so the
        // output ordering is deterministic.
        let mut component_of_root: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();
        let mut components: Vec<Vec<FaceId>> = Vec::new();
        for (i, &fid) in face_ids.iter().enumerate() {
            let root = find(&mut parent, i);
            let comp = *component_of_root.entry(root).or_insert_with(|| {
                components.push(Vec::new());
                components.len() - 1
            });
            components[comp].push(fid);
        }

        if components.len() <= 1 {
            return vec![self.clone()];
        }

        components
            .into_iter()
            .map(|faces| self.rebuild_component(&faces))
            .collect()
    }

    /// Rebuild the subset of faces `faces` (all from `self`) as a standalone
    /// Object, remapping only the vertices those faces reference. Helper for
    /// [`Object::split_connected_components`]; `faces` must form a closed shell
    /// for the result to be watertight (the splitter guarantees this for a
    /// valid manifold).
    #[allow(clippy::type_complexity)] // mirrors from_faces_with_holes' spec tuple
    fn rebuild_component(&self, faces: &[FaceId]) -> Object {
        // Remap a loop's vertices to dense local indices, appending unseen
        // positions in first-encounter order (deterministic given the loop and
        // face iteration order).
        fn loop_indices(
            obj: &Object,
            lid: LoopId,
            positions: &mut Vec<Point3>,
            local_index: &mut std::collections::BTreeMap<VertexId, usize>,
        ) -> Vec<usize> {
            obj.loop_half_edges(lid)
                .map(|h| {
                    let v = obj.half_edges[h].origin;
                    *local_index.entry(v).or_insert_with(|| {
                        positions.push(obj.vertices[v].position);
                        positions.len() - 1
                    })
                })
                .collect()
        }

        let mut positions: Vec<Point3> = Vec::new();
        let mut local_index: std::collections::BTreeMap<VertexId, usize> =
            std::collections::BTreeMap::new();
        let mut specs: Vec<(
            Vec<usize>,
            Vec<Vec<usize>>,
            Plane,
            crate::material::FaceMaterial,
            Option<crate::material::UvFrame>,
            Option<crate::topo::SurfaceRef>,
        )> = Vec::with_capacity(faces.len());
        for &fid in faces {
            let face = &self.faces[fid];
            let outer = loop_indices(self, face.outer_loop, &mut positions, &mut local_index);
            let inner: Vec<Vec<usize>> = face
                .inner_loops
                .iter()
                .map(|&il| loop_indices(self, il, &mut positions, &mut local_index))
                .collect();
            specs.push((
                outer,
                inner,
                face.plane,
                face.material,
                face.uv_frame,
                face.surface,
            ));
        }

        let mut obj = Object::from_faces_with_holes(&positions, &specs);
        // Carry over object-level state the rebuild path does not know about.
        obj.default_material = self.default_material;
        obj.planarity_tol = self.planarity_tol;
        obj.check_invariants();
        obj
    }

    /// Cut this watertight solid by `plane` into the two pieces on either side
    /// (ARCHITECTURE.md  — the Fusion *Split Body* / Onshape *Split Part* model).
    ///
    /// Returns `(positive, negative)`: the piece on the plane's normal side and
    /// the piece on the opposite side, each an independent watertight Object
    /// sharing the (coincident) cut face. The cut reuses the boolean machinery
    ///: the solid is intersected with the closed half-space on each
    /// side, realized as a box whose only face passing through the interior is
    /// the cut plane (its other five faces sit a margin outside the solid's
    /// extent, so they meet nothing — general position). A plane coincident
    /// with an existing face, or merely grazing an edge/vertex, is refused as
    /// [`SliceError::Degenerate`], exactly as booleans refuse degenerate
    /// contact. A plane the solid does not straddle is
    /// [`SliceError::PlaneMissesSolid`]. Materials, per-face UV frames, and the
    /// object base material propagate through the boolean unchanged.
    ///
    /// On any `Err`, nothing is produced (the caller's source object is
    /// untouched).
    pub fn slice(&self, plane: &Plane) -> Result<(Object, Object), SliceError> {
        if self.watertight != WatertightState::Watertight {
            return Err(SliceError::NotSolid);
        }

        // In-plane orthonormal basis (u, v) with u × v = n, plus a point on the
        // plane (closest point to the origin: n * offset).
        let n = plane.normal();
        let pp = plane.point();
        let pp_v = pp.to_vec();
        // A reference vector not parallel to n, so the cross product is stable.
        let aux = if n.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let u = aux
            .cross(n)
            .normalized()
            .map_err(|_| SliceError::Degenerate)?;
        let v = n.cross(u); // unit by construction (n ⟂ u, both unit)

        // Object extent in the (u, v, n) frame, relative to pp.
        let (mut u_lo, mut u_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut v_lo, mut v_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut n_lo, mut n_hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for vert in self.vertices.values() {
            let d = vert.position.to_vec() - pp_v;
            let (du, dv, dn) = (d.dot(u), d.dot(v), d.dot(n));
            u_lo = u_lo.min(du);
            u_hi = u_hi.max(du);
            v_lo = v_lo.min(dv);
            v_hi = v_hi.max(dv);
            n_lo = n_lo.min(dn);
            n_hi = n_hi.max(dn);
        }

        // The plane must pass through the interior with real material on both
        // sides; a graze (extent within tolerance on either side) is no cut.
        let cut = tol::POINT_MERGE;
        if n_hi <= cut || n_lo >= -cut {
            return Err(SliceError::PlaneMissesSolid);
        }

        // Half-space boxes: a rectangle on the cut plane, oversized in u/v so the
        // whole cross-section fits, extruded past the solid on each side. The
        // margin pushes every non-cut box face clear of the solid (general
        // position); it scales with the extent so it is never swamped by f64.
        let span = (u_hi - u_lo).max(v_hi - v_lo).max(n_hi - n_lo);
        let m = (span * 0.5).max(1.0);
        let (ru_lo, ru_hi) = (u_lo - m, u_hi + m);
        let (rv_lo, rv_hi) = (v_lo - m, v_hi + m);
        let corner = |du: f64, dv: f64| {
            let w = pp_v + u * du + v * dv;
            Point3::new(w.x, w.y, w.z)
        };
        // CCW about +n (u × v = n): (lo,lo) → (hi,lo) → (hi,hi) → (lo,hi).
        let rect = vec![
            corner(ru_lo, rv_lo),
            corner(ru_hi, rv_lo),
            corner(ru_hi, rv_hi),
            corner(ru_lo, rv_hi),
        ];
        let face_plane = Plane::from_point_normal(pp, n).map_err(|_| SliceError::Degenerate)?;
        let profile = Profile::new(face_plane, rect, vec![]).map_err(|_| SliceError::Degenerate)?;

        // +n box extends from the plane to past the solid's far +n extent; the
        // −n box extends the other way (negative distance sweeps along −n).
        let plus_box =
            Object::from_extrusion(&profile, n_hi + m).map_err(|_| SliceError::Degenerate)?;
        let minus_box =
            Object::from_extrusion(&profile, n_lo - m).map_err(|_| SliceError::Degenerate)?;

        let map_bool = |e: BooleanError| match e {
            BooleanError::EmptyResult => SliceError::PlaneMissesSolid,
            BooleanError::OperandNotSolid { .. } => SliceError::NotSolid,
            _ => SliceError::Degenerate,
        };
        let positive = Object::boolean(BooleanOp::Intersect, self, &plus_box, &Transform::IDENTITY)
            .map_err(map_bool)?;
        let negative =
            Object::boolean(BooleanOp::Intersect, self, &minus_box, &Transform::IDENTITY)
                .map_err(map_bool)?;

        positive.check_invariants();
        negative.check_invariants();
        // Always-on backstop: refuse a cut that corrupted either
        // piece rather than hand back invalid solids.
        positive.validate().map_err(|_| SliceError::Degenerate)?;
        negative.validate().map_err(|_| SliceError::Degenerate)?;
        Ok((positive, negative))
    }

    /// Whether an inward push/pull of `face` by `distance` drives it into or
    /// past opposing material — the through case that must become a subtract
    /// rather than a translate.
    ///
    /// Returns `false` for outward pulls (which never remove material),
    /// non-solids, or stale faces. Otherwise it finds the nearest opposing
    /// wall: a face that front-faces `face` (its outward normal points back
    /// toward `face`) and whose projected footprint overlaps `face`'s, and
    /// reports the push as through once `|distance|` reaches that wall. This is
    /// the wall the swept face would punch through; it is detected by the wall
    /// *face* (not its vertices), so a small imprint over a large opposite wall
    /// — the hole-punch case — is caught even though no wall vertex lies in the
    /// imprint's column.
    pub fn push_pull_overshoots(&self, face: FaceId, distance: f64) -> bool {
        // Attributed cylinder walls never overshoot into a through-cut:
        // push/pull on them is a radial offset of the whole logical wall
        // (docs/design/true-curves.md §4.6), and deep pushes refuse there
        // with a typed error instead of punching through.
        if self.faces.get(face).is_some_and(|f| f.surface.is_some()) {
            return false;
        }
        match self.opposing_wall_depth(face) {
            Some(depth) if distance < 0.0 => (-distance) >= depth - tol::POINT_MERGE,
            _ => false,
        }
    }

    /// The inward distance from `face`'s plane to the nearest opposing wall
    /// directly across from it — the depth at which an inward push punches
    /// through. `None` if `face` is stale, the object is not solid, or nothing
    /// faces it across the swept column. See [`Object::push_pull_overshoots`].
    fn opposing_wall_depth(&self, face: FaceId) -> Option<f64> {
        if self.watertight != WatertightState::Watertight {
            return None;
        }
        let f = self.faces.get(face)?;
        let mplane = f.plane;
        let mnormal = mplane.normal();
        let mouter: Vec<Point3> = self.loop_positions(f.outer_loop).collect();

        let mut nearest = f64::INFINITY;
        for (fid, other) in &self.faces {
            if fid == face {
                continue;
            }
            // Only walls that front-face this one (normals roughly opposed) can
            // be punched through; skip co-facing and perpendicular faces.
            if mnormal.dot(other.plane.normal()) >= -tol::NORMAL_DIRECTION {
                continue;
            }
            let oouter: Vec<Point3> = self.loop_positions(other.outer_loop).collect();
            // Footprint overlap (projected along the sweep): a corner of either
            // loop inside the other. Sufficient for the axis-aligned faces and
            // imprint-over-wall cases that arise here.
            let overlaps = mouter
                .iter()
                .any(|&p| point_inside_polygon(p, &oouter, mnormal))
                || oouter
                    .iter()
                    .any(|&p| point_inside_polygon(p, &mouter, mnormal));
            if !overlaps {
                continue;
            }
            // Nearest part of this opposing wall along the inward normal.
            for &p in &oouter {
                let inward = -mplane.signed_distance(p);
                if inward > tol::POINT_MERGE {
                    nearest = nearest.min(inward);
                }
            }
        }
        nearest.is_finite().then_some(nearest)
    }

    /// Push `face` inward by `distance` (negative) past opposing material,
    /// realized as a subtract: the face's profile swept inward by
    /// `distance` is removed from the solid — a recess that breaks the far wall
    /// becomes a through-hole, and a cut that fully severs the solid leaves a
    /// multi-shell result (the caller splits it with
    /// [`Object::split_connected_components`]). Per-face materials and UV frames
    /// propagate through the boolean. The source is borrowed, not mutated.
    ///
    /// # Errors
    /// - [`PushPullError::ObjectNotSolid`] — not watertight.
    /// - [`PushPullError::UnknownFace`] — stale `face`.
    /// - [`PushPullError::DistanceTooSmall`] — `|distance|` below tolerance.
    /// - [`PushPullError::WouldVanish`] — the subtract removes all material.
    /// - [`PushPullError::NonManifoldResult`] — the swept tool is degenerate or
    ///   the cut is tangent (refused, not repaired).
    pub fn push_through(&self, face: FaceId, distance: f64) -> Result<Object, PushPullError> {
        if self.watertight != WatertightState::Watertight {
            return Err(PushPullError::ObjectNotSolid);
        }
        let f = self.faces.get(face).ok_or(PushPullError::UnknownFace)?;
        if distance.abs() < tol::POINT_MERGE {
            return Err(PushPullError::DistanceTooSmall);
        }
        // Swept prism: the face's loops extruded inward by `distance`. The face
        // outer loop is CCW and inner loops CW seen from the normal — exactly the
        // winding Profile::new expects for outer + holes.
        let outer: Vec<Point3> = self.loop_positions(f.outer_loop).collect();
        let holes: Vec<Vec<Point3>> = f
            .inner_loops
            .iter()
            .map(|&il| self.loop_positions(il).collect())
            .collect();
        // Carry the analytic circle each boundary edge is a chord facet of
        // (stamped at imprint / extrusion) into the tool profile, so
        // `from_extrusion` re-stamps the cut's tunnel walls as
        // `SurfaceRef::Cylinder` — otherwise a circular hole's walls are flat
        // facets and refuse whole-wall push/pull (true-curves C3). The loop
        // half-edges walk in the same order as `loop_positions`, so the
        // attribution stays parallel to the profile boundaries.
        let loop_curves = |lid: crate::ids::LoopId| -> Vec<Option<crate::sketch::CurveGeom>> {
            self.loop_half_edges(lid)
                .map(|h| self.edges[self.half_edges[h].edge].curve)
                .collect()
        };
        let outer_curves = loop_curves(f.outer_loop);
        let hole_curves: Vec<Vec<Option<crate::sketch::CurveGeom>>> =
            f.inner_loops.iter().map(|&il| loop_curves(il)).collect();
        let mut profile =
            Profile::new(f.plane, outer, holes).map_err(|_| PushPullError::NonManifoldResult)?;
        profile.set_curve_attribution(outer_curves, hole_curves);
        let tool = Object::from_extrusion(&profile, distance)
            .map_err(|_| PushPullError::NonManifoldResult)?;
        match Object::boolean(BooleanOp::Subtract, self, &tool, &Transform::IDENTITY) {
            Ok(mut result) => {
                // Dissolve coplanar seams the cut introduced (a cut wall
                // flush with an existing wall must read as ONE face), but
                // preserve this object's pre-existing coplanar edges — face
                // imprints awaiting their own push/pull. The tool is a fresh
                // extrusion with no imprints, so it contributes none.
                let preserve = self.coplanar_edge_segments();
                result.merge_coplanar_faces(&preserve);
                result.check_invariants();
                // Always-on backstop.
                result
                    .validate()
                    .map_err(|_| PushPullError::NonManifoldResult)?;
                Ok(result)
            }
            Err(BooleanError::EmptyResult) => Err(PushPullError::WouldVanish),
            Err(_) => Err(PushPullError::NonManifoldResult),
        }
    }
}

// ============================================================== private helpers

/// Whether `point` lies on the segment `a`→`b` within `tol` (distance to the
/// CLAMPED closest point — a point beyond an endpoint measures to that
/// endpoint, so collinear-but-outside points do not qualify).
fn point_on_segment(point: Point3, a: Point3, b: Point3, tol: f64) -> bool {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 <= f64::EPSILON {
        0.0
    } else {
        ((point - a).dot(ab) / len2).clamp(0.0, 1.0)
    };
    let closest = Point3::new(a.x + ab.x * t, a.y + ab.y * t, a.z + ab.z * t);
    (point - closest).length() <= tol
}

/// One face of a closed shell, flattened for the parity cast: its plane,
/// outer loop positions, and hole loop positions.
type ShellFace = (Plane, Vec<Point3>, Vec<Vec<Point3>>);

/// A few non-axis-aligned ray directions for [`point_in_shell_faces`]; the
/// first one whose cast grazes no face boundary decides the parity (same
/// posture as the boolean classifier's `RAY_DIRS`).
const WALL_GUARD_RAY_DIRS: [Vec3; 4] = [
    Vec3::new(0.4651, 0.7345, 0.4949),
    Vec3::new(0.8012, -0.3461, 0.4877),
    Vec3::new(-0.3299, 0.5813, 0.7438),
    Vec3::new(0.1234, -0.5678, 0.8137),
];

/// Point-in-solid by ray-cast parity against a closed face set — the boolean
/// classifier's mechanism (`crates/kernel/src/boolean.rs::point_in_solid`),
/// restated over plain loop positions for the whole-wall offset guards.
/// Tries each candidate direction until one casts cleanly (no boundary
/// graze, no in-plane run through `p`); falls back to the first direction's
/// count if all graze. Deterministic either way.
fn point_in_shell_faces(faces: &[ShellFace], p: Point3) -> bool {
    for d in WALL_GUARD_RAY_DIRS {
        let (count, ambiguous) = shell_face_ray_cast(faces, p, d);
        if !ambiguous {
            return count % 2 == 1;
        }
    }
    shell_face_ray_cast(faces, p, WALL_GUARD_RAY_DIRS[0]).0 % 2 == 1
}

/// Counts forward crossings of the ray `p + t·d` (t > 0) through the face
/// set, flagging the cast ambiguous if a hit grazes a face boundary or the
/// ray lies in a face's plane through `p`.
fn shell_face_ray_cast(faces: &[ShellFace], p: Point3, d: Vec3) -> (usize, bool) {
    let mut count = 0;
    let mut ambiguous = false;
    for (plane, outer, holes) in faces {
        let denom = d.dot(plane.normal());
        let signed = plane.signed_distance(p);
        if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
            if signed.abs() <= tol::PLANE_DIST {
                ambiguous = true;
            }
            continue;
        }
        let t = -signed / denom;
        if t <= tol::POINT_MERGE {
            continue;
        }
        let hit = p + d * t;
        let near = |poly: &[Point3]| {
            (0..poly.len()).any(|i| {
                point_on_segment(hit, poly[i], poly[(i + 1) % poly.len()], tol::POINT_MERGE)
            })
        };
        if near(outer) || holes.iter().any(|h| near(h)) {
            ambiguous = true;
            continue;
        }
        let n = plane.normal();
        if point_inside_polygon(hit, outer, n)
            && !holes.iter().any(|h| point_inside_polygon(hit, h, n))
        {
            count += 1;
        }
    }
    (count, ambiguous)
}

/// Detects and undoes the exact inverse of [`push_pull_build_walls`]'s
/// unweld: a push whose sweep exactly closes a coplanar step it (or an
/// earlier coplanar-aware push) built. A previously built cut wall is itself
/// perpendicular to the moved face, so it classifies as an ordinary
/// `Transverse` boundary edge ('s dot-product test cannot tell "an
/// original side wall" from "a wall straddling a cut" apart — both are
/// perpendicular); this scan instead recognizes the wall by shape: a quad
/// whose far edge (two hops around from the moved face's edge) is itself
/// twinned with a face that IS coplanar with the moved face — the sibling the
/// wall was built to bridge to.
///
/// Returns `Ok(Some(removed_faces))` if every such wall's far edge sits
/// exactly `sweep` away (within [`tol::POINT_MERGE`]) — i.e. the wall is
/// collapsing to zero height — after welding `face`'s boundary straight back
/// onto the sibling the wall was bridging to and removing the wall and its
/// spliced junction steps. Returns `Ok(None)` (object untouched, no Transverse
/// edge qualifies) so the caller falls back to [`push_pull_build_walls`]'s
/// raise path — including the ordinary case where every neighbor really is
/// just an unrelated side wall.
///
/// Scope: this only ever welds a step shut. It never merges `face`
/// into a flush single face with its sibling (the cut edge persists, exactly
/// reversing the unweld) and never attempts push-through.
struct CollapsePlan {
    h: HalfEdgeId,
    h_hole: HalfEdgeId,
    hb: HalfEdgeId,
    hc: HalfEdgeId,
    hd: HalfEdgeId,
    wall_face: FaceId,
    far_a: VertexId,
    far_b: VertexId,
}

/// Pure (non-mutating) detection half of `try_collapse_coplanar_step`: finds
/// every Transverse boundary half-edge `h` (moved face, origin `va`, twin
/// `h_hole` on a candidate wall `w`) where `w`'s loop is a quad
/// `h_hole(vb) -> hb(va) -> hc(far_a) -> hd(far_b) -> h_hole`, `hc`'s far-side
/// twin is coplanar with the moved face (the telltale sign `w` is a cut wall,
/// not an ordinary side wall), and both `position(va) + sweep ==
/// position(far_a)` and `position(vb) + sweep == position(far_b)` — i.e. this
/// sweep exactly closes a step built by an earlier coplanar-aware push.
///
/// Read-only: callers use this both to decide whether to apply the collapse
/// (`try_collapse_coplanar_step`) and to know — *before* Step 3's
/// interior-obstruction guard runs — that vertices on these candidate walls
/// are part of the step machinery, not a genuine fixed obstruction.
fn find_collapse_plans(
    obj: &Object,
    face: FaceId,
    boundary_loops: &[LoopId],
    edge_kinds: &std::collections::BTreeMap<HalfEdgeId, BoundaryEdgeKind>,
    sweep: Vec3,
) -> Vec<CollapsePlan> {
    let face_normal = obj.faces[face].plane.normal();
    let mut plans = Vec::new();
    for &loop_id in boundary_loops {
        for h in obj.loop_half_edges(loop_id) {
            if !matches!(edge_kinds[&h], BoundaryEdgeKind::Transverse) {
                continue;
            }
            let va = obj.half_edges[h].origin;
            let h_hole = obj.half_edges[h].twin.expect("watertight boundary edge");
            let vb = obj.half_edges[h_hole].origin;
            let hb = obj.half_edges[h_hole].next;
            let hc = obj.half_edges[hb].next;
            let hd = obj.half_edges[hc].next;
            if obj.half_edges[hd].next != h_hole {
                continue; // wall isn't a quad — not a cut wall built by us.
            }
            let Some(hc_twin) = obj.half_edges[hc].twin else {
                continue;
            };
            let far_face = obj.loops[obj.half_edges[hc_twin].loop_id].face;
            let far_normal = obj.faces[far_face].plane.normal();
            // A step sibling shares the moved face's normal DIRECTION (dot ≈ +1):
            // it came from the same `split_face`d face. The opposite side of the
            // solid is antiparallel (dot ≈ -1); collapsing onto it is push-THROUGH,
            // not a step re-merge, and must fall through to the WouldVanish path.
            if face_normal.dot(far_normal) < 1.0 - tol::NORMAL_DIRECTION {
                continue; // far side isn't a same-facing coplanar sibling.
            }
            let far_a = obj.half_edges[hc].origin;
            let far_b = obj.half_edges[hd].origin;
            let target_a = obj.vertices[va].position + sweep;
            let target_b = obj.vertices[vb].position + sweep;
            if !target_a.approx_eq(obj.vertices[far_a].position, tol::POINT_MERGE)
                || !target_b.approx_eq(obj.vertices[far_b].position, tol::POINT_MERGE)
            {
                continue; // shape matches a cut wall, but this push doesn't close it exactly.
            }
            let wall_face = obj.loops[obj.half_edges[h_hole].loop_id].face;
            plans.push(CollapsePlan {
                h,
                h_hole,
                hb,
                hc,
                hd,
                wall_face,
                far_a,
                far_b,
            });
        }
    }
    plans
}

/// The plan-finder for [`Object::unbuild_push_pull`], the exact inverse of a
/// translate-and-build sweep. Like [`find_collapse_plans`] it matches each
/// recorded quad wall on the moved face's boundary and reads off the collapse
/// plan, but it is driven by the RECORDED set of `walls` the forward push
/// created rather than by re-detecting collapsibility, and it therefore drops
/// the coplanar-far-face restriction: a wall built along a *slanted* neighbor
/// bridges a face that is NOT coplanar with the moved one, yet is removed
/// identically.
///
/// A recorded wall must still be the **pristine quad** it was built as, or its
/// plan is skipped so the caller refuses typed (leaving the object untouched)
/// rather than corrupting it. Undo is LIFO, so in the common case the wall is
/// pristine — but an intervening op can alter it (e.g. a `split_face_inner`
/// appends a hole loop and a sub-face, leaving the outer 4-cycle untouched),
/// and removing such a wall would orphan its hole (a debug validator panic, a
/// release corruption). The completeness check is therefore: only walls in
/// `walls`, whose outer loop is a clean 4-cycle, that carry **no inner loops**,
/// and that this inverse `sweep` (`−distance·n`) closes exactly. Any deviation
/// skips the wall; the caller requires every recorded wall to yield a plan, so
/// one skip is a typed refusal, not a partial un-build.
fn find_unbuild_plans(
    obj: &Object,
    boundary_loops: &[LoopId],
    walls: &std::collections::BTreeSet<FaceId>,
    sweep: Vec3,
) -> Vec<CollapsePlan> {
    let mut plans = Vec::new();
    for &loop_id in boundary_loops {
        for h in obj.loop_half_edges(loop_id) {
            let Some(h_hole) = obj.half_edges[h].twin else {
                continue;
            };
            let wall_face = obj.loops[obj.half_edges[h_hole].loop_id].face;
            if !walls.contains(&wall_face) {
                continue;
            }
            // A hole appended to the wall (by an intervening imprint) leaves the
            // outer 4-cycle intact but would be orphaned on removal: refuse.
            if !obj.faces[wall_face].inner_loops.is_empty() {
                continue;
            }
            let vb = obj.half_edges[h_hole].origin;
            let hb = obj.half_edges[h_hole].next;
            let hc = obj.half_edges[hb].next;
            let hd = obj.half_edges[hc].next;
            if obj.half_edges[hd].next != h_hole {
                continue; // recorded face's outer loop is no longer a quad.
            }
            let va = obj.half_edges[h].origin;
            let far_a = obj.half_edges[hc].origin;
            let far_b = obj.half_edges[hd].origin;
            let target_a = obj.vertices[va].position + sweep;
            let target_b = obj.vertices[vb].position + sweep;
            if !target_a.approx_eq(obj.vertices[far_a].position, tol::POINT_MERGE)
                || !target_b.approx_eq(obj.vertices[far_b].position, tol::POINT_MERGE)
            {
                continue; // this inverse sweep does not close the recorded wall.
            }
            plans.push(CollapsePlan {
                h,
                h_hole,
                hb,
                hc,
                hd,
                wall_face,
                far_a,
                far_b,
            });
        }
    }
    plans
}

fn try_collapse_coplanar_step(
    obj: &mut Object,
    face: FaceId,
    boundary_loops: &[LoopId],
    edge_kinds: &std::collections::BTreeMap<HalfEdgeId, BoundaryEdgeKind>,
    sweep: Vec3,
) -> Result<Option<Vec<FaceId>>, PushPullError> {
    let plans = find_collapse_plans(obj, face, boundary_loops, edge_kinds, sweep);
    if plans.is_empty() {
        return Ok(None);
    }
    collapse_plans_surgery(obj, face, boundary_loops, &plans, sweep).map(Some)
}

/// The topology surgery that reverses a translate-and-build sweep: given
/// `plans` (one per pristine quad wall to remove), it welds each moved-face
/// wall edge back onto the neighbor edge the wall bridged, retires the raised
/// junction copies onto their originals, un-splices the junction steps that
/// reshaped the straddling transverse walls, removes the wall faces, and
/// translates every non-welded boundary vertex back by `sweep`. Shared by the
/// coplanar step-close ([`try_collapse_coplanar_step`], plans from
/// [`find_collapse_plans`]) and the exact push/pull inverse
/// ([`Object::unbuild_push_pull`], plans from [`find_unbuild_plans`], which may
/// bridge a non-coplanar neighbor). Returns the removed wall faces.
fn collapse_plans_surgery(
    obj: &mut Object,
    face: FaceId,
    boundary_loops: &[LoopId],
    plans: &[CollapsePlan],
    sweep: Vec3,
) -> Result<Vec<FaceId>, PushPullError> {
    // Every vertex on the moved face's boundary that ISN'T a junction welded
    // back onto a sibling below (i.e. every corner Pass 1 of the original
    // push translated in place rather than raising, because both its incident
    // edges were Transverse) still needs to translate back by `sweep` now —
    // exactly mirroring the fast (pure-translate) path. Collected up front,
    // before any welding below changes the loops' membership.
    let welded: std::collections::BTreeSet<VertexId> =
        plans.iter().flat_map(|p| [p.far_a, p.far_b]).collect();
    let mut to_translate: Vec<VertexId> = Vec::new();
    for &loop_id in boundary_loops {
        for h in obj.loop_half_edges(loop_id) {
            let v = obj.half_edges[h].origin;
            if !welded.contains(&v) && !to_translate.contains(&v) {
                to_translate.push(v);
            }
        }
    }
    for v in to_translate {
        obj.vertices[v].position = obj.vertices[v].position + sweep;
    }

    let mut removed_faces = Vec::new();
    // The raised junction copies retired by the re-weld below. They are still
    // referenced by the straddling transverse side walls (their step vertices),
    // so after the per-plan surgery every remaining reference to a raised copy
    // is welded onto its surviving sibling vertex — otherwise a side-wall edge
    // keeps starting at a vertex the moved face no longer shares, breaking the
    // twin-origin involution (validate.rs `twin.origin == next.origin`).
    let mut welds: Vec<(VertexId, VertexId)> = Vec::new();
    // Loops of every wall being collapsed: a vertical whose twin lives in one
    // of these is shared with an ADJACENT collapsing wall (interior cut
    // vertex), not spliced into a transverse wall — see the unsplice below.
    let collapsing_wall_loops: std::collections::BTreeSet<LoopId> = plans
        .iter()
        .map(|p| obj.faces[p.wall_face].outer_loop)
        .collect();
    for plan in plans {
        // Weld `h` directly onto whatever the wall's far edge (`hc`) was
        // twinned with — exactly reversing `build_coplanar_wall`'s "wc
        // reuses the old shared edge" move. `h`'s origin retires the raised
        // copy in favor of `far_a` (mirrors Pass 2's repoint in reverse); its
        // DESTINATION — `h.next`'s origin, currently the other raised copy —
        // must likewise retire to `far_b`, or that raised vertex is left
        // dangling once the wall (its only other referrer) is removed below.
        let hc_twin = obj.half_edges[plan.hc].twin;
        let h_next = obj.half_edges[plan.h].next;
        let raised_a = obj.half_edges[plan.h].origin;
        let raised_b = obj.half_edges[h_next].origin;
        if raised_a != plan.far_a {
            welds.push((raised_a, plan.far_a));
        }
        if raised_b != plan.far_b {
            welds.push((raised_b, plan.far_b));
        }
        obj.half_edges[plan.h].origin = plan.far_a;
        obj.vertices[plan.far_a].outgoing = plan.h;
        obj.half_edges[h_next].origin = plan.far_b;
        match hc_twin {
            Some(t) => {
                let edge = obj.half_edges[t].edge;
                obj.half_edges[plan.h].twin = Some(t);
                obj.half_edges[plan.h].edge = edge;
                obj.half_edges[t].twin = Some(plan.h);
                obj.half_edges[t].edge = edge;
                obj.edges[edge].half_edge = t;
                obj.edges[edge].twin_half_edge = Some(plan.h);
            }
            None => return Err(PushPullError::NonManifoldResult),
        }
        obj.vertices[plan.far_b].outgoing = obj.half_edges[plan.h].twin.expect("just set");

        // Un-splice the two junction steps (`hb`/`hd`) from the straddling
        // transverse walls, undoing whichever of `splice_after`/`splice_before`
        // created each one. A vertical shared with an ADJACENT collapsing
        // wall is not a junction step: its twin is that wall's own vertical,
        // which dies with that wall's removal (and may already be gone if the
        // adjacent plan ran first) — skip it instead of unsplicing.
        for &step in &[plan.hb, plan.hd] {
            let Some(step_he) = obj.half_edges.get(step) else {
                continue; // removed with an already-collapsed adjacent wall
            };
            let step_twin = step_he.twin.expect("junction step is twinned");
            let Some(twin_he) = obj.half_edges.get(step_twin) else {
                continue; // twin removed with an already-collapsed adjacent wall
            };
            if collapsing_wall_loops.contains(&twin_he.loop_id) {
                continue; // shared vertical of an adjacent collapsing wall
            }
            unsplice_step(obj, step_twin);
        }

        // Remove the wall face, its loop, its 4 half-edges, and the 2 edges
        // that belonged solely to it (the verticals; the far edge's `Edge`
        // was just reassigned above, and `h`'s old edge is removed with it).
        let h_edge = obj.half_edges[plan.h_hole].edge;
        let hb_edge = obj.half_edges[plan.hb].edge;
        let hd_edge = obj.half_edges[plan.hd].edge;
        obj.edges.remove(h_edge);
        obj.edges.remove(hb_edge);
        obj.edges.remove(hd_edge);
        let wall_loop = obj.half_edges[plan.h_hole].loop_id;
        obj.loops.remove(wall_loop);
        for &he in &[plan.h_hole, plan.hb, plan.hc, plan.hd] {
            obj.half_edges.remove(he);
        }
        let shell = obj
            .shells
            .iter()
            .find(|(_, s)| s.faces.contains(&plan.wall_face))
            .map(|(id, _)| id)
            .expect("wall face belongs to a shell");
        obj.shells[shell].faces.retain(|&f| f != plan.wall_face);
        obj.faces.remove(plan.wall_face);
        removed_faces.push(plan.wall_face);
    }

    // Weld every retired raised junction copy onto its surviving sibling: any
    // half-edge still starting at a raised copy (the straddling side walls'
    // step vertices) is repointed to the original, then the raised vertex is
    // removed. This restores the twin-origin involution at the junctions.
    for (raised, survivor) in welds {
        if raised == survivor || !obj.vertices.contains_key(raised) {
            continue;
        }
        let hes: Vec<HalfEdgeId> = obj
            .half_edges
            .iter()
            .filter(|(_, he)| he.origin == raised)
            .map(|(h, _)| h)
            .collect();
        for h in hes {
            obj.half_edges[h].origin = survivor;
        }
        // `survivor` already has a valid outgoing from the re-weld; the raised
        // copy is now unreferenced.
        obj.vertices.remove(raised);
    }

    // Refit the moved face's plane (its Coplanar edges now sit on the
    // sibling's old boundary again).
    refit_face_plane(obj, face)?;

    // Refit the straddling transverse walls' planes too (they un-stepped
    // back to quads/pentagons and the planarity check must see the new
    // boundary, never a hand-translated one). Refit every current neighbor
    // of the moved face — cheap, and correct regardless of which walls were
    // actually touched.
    let mut refit_faces: Vec<FaceId> = Vec::new();
    for &loop_id in boundary_loops {
        for h in obj.loop_half_edges(loop_id) {
            if let Some(t) = obj.half_edges[h].twin {
                let nf = obj.loops[obj.half_edges[t].loop_id].face;
                if !refit_faces.contains(&nf) {
                    refit_faces.push(nf);
                }
            }
        }
    }
    for nf in refit_faces {
        refit_face_plane(obj, nf)?;
    }
    Ok(removed_faces)
}

/// Undoes whichever of `splice_after`/`splice_before` created the junction
/// step half-edge twinned with `step_twin` (the moved face's now-removed
/// vertical, i.e. `step_twin` is the half-edge actually embedded in the
/// straddling transverse wall's loop). Two shapes, distinguished by whether
/// `step_twin`'s origin is the raised vertex (a `splice_after` "down" step,
/// origin never repointed — just unlink and remove) or the original vertex
/// (impossible here, since `splice_after` never repoints; kept for the
/// `splice_before` "up" step, where the step's `next` was repointed to the
/// raised copy and must be repointed back).
fn unsplice_step(obj: &mut Object, step_twin: HalfEdgeId) {
    let prev = obj.half_edges[step_twin].prev;
    let next = obj.half_edges[step_twin].next;
    // `splice_after` shape: `step_twin` was inserted between `prev` and
    // `next`, origin untouched elsewhere — just unlink it.
    // `splice_before` shape: `step_twin` was inserted between `prev` and
    // `next`, and `next`'s origin was repointed to the raised copy that is
    // about to vanish — repoint it back to `step_twin`'s own origin (the
    // original vertex, which both share).
    if obj.half_edges[next].origin != obj.half_edges[step_twin].origin {
        let v_orig = obj.half_edges[step_twin].origin;
        obj.half_edges[next].origin = v_orig;
        obj.vertices[v_orig].outgoing = next;
    }
    obj.half_edges[prev].next = next;
    obj.half_edges[next].prev = prev;
    // The loop's anchor may be the half-edge being removed.
    let loop_id = obj.half_edges[step_twin].loop_id;
    if obj.loops[loop_id].first_half_edge == step_twin {
        obj.loops[loop_id].first_half_edge = next;
    }
    // `prev` (and `next`, in the splice_before shape) already have valid
    // `outgoing` half-edges that survive this removal — `step_twin` was
    // never anyone's recorded `outgoing` except possibly transiently, and
    // a quad wall's other three half-edges always include one starting at
    // each of `step_twin`'s endpoints.
    obj.half_edges.remove(step_twin);
}

/// The hybrid surgery behind `push_pull`'s coplanar-aware mode: walks
/// every boundary loop of the moved face, translating vertices in place where
/// every incident edge is transverse (the pre- behavior), and unwelding +
/// building a wall where an edge is coplanar (a `split_face` cut edge shared
/// with a sibling sub-face).
///
/// At a **junction** — a vertex where a coplanar edge meets a transverse one,
/// i.e. a cut endpoint `split_face` planted on the moved face's outer
/// boundary — the vertex is shared by the moved face, the sibling, and one
/// straddling transverse wall. It is unwelded: the moved face gets a raised
/// copy, while the sibling and a new "step" edge spliced into the straddling
/// wall's loop keep the original. The straddling wall thereby reshapes from a
/// quad into an L-shaped (still planar) polygon; its plane is refit from its
/// new boundary, never translated by hand (KERNEL_GUIDE "Traps").
///
/// Mirrors `extrude_sub_face`'s per-edge wall construction (`wa`/`wb`/`wc`/`wd`)
/// one coplanar edge at a time instead of over a whole imprinted sub-face, so
/// the wall's two "vertical" half-edges are shared either with the next
/// coplanar wall around an interior cut vertex (exactly as `extrude_sub_face`
/// shares them between consecutive walls), or with the spliced step inside a
/// straddling transverse wall at a junction.
///
/// Returns the newly created wall faces, or refuses with a typed
/// [`PushPullError`] (never partially mutates: the caller validates and swaps
/// only on success).
fn push_pull_build_walls(
    obj: &mut Object,
    face: FaceId,
    boundary_loops: &[LoopId],
    edge_kinds: &std::collections::BTreeMap<HalfEdgeId, BoundaryEdgeKind>,
    neighbor_faces: &[FaceId],
    sweep: Vec3,
) -> Result<Vec<FaceId>, PushPullError> {
    // A "wall edge" is any non-transverse boundary edge: a coplanar
    // `split_face` sibling (`dot ≈ 1`) or a slanted wedge/facet neighbor
    // (`0 < dot < 1`). Both unweld and grow a fresh quad wall — the
    // construction is identical; only the wall's resulting plane differs. A
    // transverse edge (`dot ≈ 0`) instead keeps its neighbor and extends it in
    // place. (Before the flat-face translate-and-build generalization this
    // path saw only coplanar edges; slanted ones stretched. Now they share
    // exactly one surgery.)
    let is_wall_edge = |h: &HalfEdgeId| {
        matches!(
            edge_kinds[h],
            BoundaryEdgeKind::Coplanar | BoundaryEdgeKind::Slanted
        )
    };
    // Snapshot each boundary loop's half-edges AND original vertices up front
    // — every later pass mutates origins/positions, so the un-raised vertex
    // at each position must be read once, before any of that happens.
    struct LoopWalk {
        /// Boundary half-edges of the moved face, in cycle order.
        hes: Vec<HalfEdgeId>,
        /// `verts[k]` is the ORIGINAL vertex at `hes[k]`'s origin.
        verts: Vec<VertexId>,
    }
    let walks: Vec<LoopWalk> = boundary_loops
        .iter()
        .map(|&loop_id| {
            let hes: Vec<HalfEdgeId> = obj.loop_half_edges(loop_id).collect();
            let verts: Vec<VertexId> = hes.iter().map(|&h| obj.half_edges[h].origin).collect();
            LoopWalk { hes, verts }
        })
        .collect();

    // --- Pass 1: classify each boundary vertex and create raised copies. ---
    // A vertex is "raised" (gets a fresh copy at +sweep; the original stays
    // put for whoever doesn't move) iff at least one of its two incident
    // boundary half-edges in the moved face's loop is Coplanar. A vertex with
    // both edges Transverse translates in place, exactly as pre-.
    let mut raised: std::collections::BTreeMap<VertexId, VertexId> =
        std::collections::BTreeMap::new();
    for walk in &walks {
        let n = walk.hes.len();
        for k in 0..n {
            let h = walk.hes[k];
            let h_prev = walk.hes[(k + n - 1) % n];
            let needs_raise = is_wall_edge(&h) || is_wall_edge(&h_prev);
            let v = walk.verts[k];
            if needs_raise {
                raised.entry(v).or_insert_with(|| {
                    let p = obj.vertices[v].position + sweep;
                    obj.vertices.insert(Vertex {
                        position: p,
                        outgoing: HalfEdgeId::default(),
                    })
                });
            } else {
                // Pure transverse corner: translate in place (shared with the
                // transverse neighbor wall, which extends to follow it).
                obj.vertices[v].position = obj.vertices[v].position + sweep;
            }
        }
    }

    // --- Pass 2: repoint the moved face's own boundary onto raised copies. ---
    // The original vertex keeps its identity (the sibling/wall still uses
    // it), so if its `outgoing` happened to be this now-repointed half-edge,
    // it must be repatched to something that still starts there — a sibling
    // or wall half-edge sharing the same original vertex always exists,
    // since a raised vertex is by definition shared with at least one other
    // face that did not move.
    for walk in &walks {
        let n = walk.hes.len();
        for (k, &h) in walk.hes.iter().enumerate() {
            let v_orig = walk.verts[k];
            if let Some(&v_raised) = raised.get(&v_orig) {
                if obj.vertices[v_orig].outgoing == h {
                    let h_prev = walk.hes[(k + n - 1) % n];
                    let fallback = obj.half_edges[h_prev]
                        .twin
                        .expect("watertight boundary edge");
                    obj.vertices[v_orig].outgoing = fallback;
                }
                obj.half_edges[h].origin = v_raised;
                obj.vertices[v_raised].outgoing = h;
            }
        }
    }

    // --- Pass 3: per junction (a raised vertex where a Transverse edge meets
    // a Coplanar one in the moved face's loop), splice a "step" half-edge
    // into the straddling transverse wall's loop, connecting the raised copy
    // to the original. Exactly one direction is created here — `down`
    // (raised -> original) if the junction sits at a Transverse half-edge's
    // ORIGIN, `up` (original -> raised) if it sits at its DESTINATION — the
    // opposite direction is the new coplanar wall's vertical (Pass 4), twinned
    // to this one.
    //
    // Reasoning (mirroring `extrude_sub_face`'s wb[k] <-> wd[k-1] sharing):
    // the wall's existing half-edge already agrees with the moved face on the
    // non-junction end (translated in place in Pass 1), so only the junction
    // end needs a new vertex spliced into the wall's loop.
    let mut steps: std::collections::BTreeMap<VertexId, StepEdges> =
        std::collections::BTreeMap::new();
    for walk in &walks {
        let n = walk.hes.len();
        for k in 0..n {
            let h = walk.hes[k];
            if !matches!(edge_kinds[&h], BoundaryEdgeKind::Transverse) {
                continue;
            }
            let twin_h = obj.half_edges[h].twin.expect("watertight boundary edge");
            let wall_loop = obj.half_edges[twin_h].loop_id;

            // Junction at `h`'s origin (walk.verts[k]): `twin_h`'s
            // DESTINATION is this vertex (twin_h runs dest_of_h -> origin_of_h).
            // Splice a `down` step (raised -> original) right after `twin_h`.
            if let Some(&v_raised) = raised.get(&walk.verts[k]) {
                let v = walk.verts[k];
                steps
                    .entry(v)
                    .or_default()
                    .down
                    .get_or_insert_with(|| splice_after(obj, wall_loop, twin_h, v_raised, v));
            }
            // Junction at `h`'s destination (walk.verts[(k+1)%n]): `twin_h`'s
            // ORIGIN is this vertex. Splice an `up` step (original -> raised)
            // right before `twin_h`, then repoint `twin_h`'s origin to raised.
            let v_dest = walk.verts[(k + 1) % n];
            if let Some(&v_raised) = raised.get(&v_dest) {
                steps
                    .entry(v_dest)
                    .or_default()
                    .up
                    .get_or_insert_with(|| splice_before(obj, wall_loop, twin_h, v_dest, v_raised));
            }
        }
    }

    // --- Pass 4: build a wall along each Coplanar boundary edge, mirroring
    // `extrude_sub_face`'s per-edge construction. The two verticals at each
    // end either reuse a Pass-3 step (junction) or are created fresh here and
    // shared with the next coplanar wall around an interior cut vertex
    // (exactly as `extrude_sub_face`'s `wb[k]`/`wd[k-1]` share one).
    let shell = obj
        .shells
        .iter()
        .find(|(_, s)| s.faces.contains(&face))
        .map(|(id, _)| id)
        .expect("moved face belongs to a shell");
    let mut created_faces = Vec::new();
    for walk in &walks {
        let n = walk.hes.len();
        for k in 0..n {
            let h = walk.hes[k];
            if !is_wall_edge(&h) {
                continue;
            }
            let v_a = walk.verts[k]; // edge origin, original vertex
            let v_b = walk.verts[(k + 1) % n]; // edge dest, original vertex
            let va_raised = raised[&v_a];
            let vb_raised = raised[&v_b];
            let h_sub = h; // already repointed to va_raised by Pass 2
            let h_hole = obj.half_edges[h].twin.expect("watertight boundary edge");

            // `wb = down(v_a)` (va_raised -> v_a), this wall's own half-edge.
            // Its twin is `up(v_a)`: either Pass 3's junction splice, an
            // earlier Pass-4 wall's `wd` at this same interior-cut vertex, or
            // freshly created now (registered under `.down` for a later wall
            // to find as ITS `up(v_a)` partner).
            let wb = obj.new_half_edge(va_raised);
            match steps.entry(v_a).or_default().up.take() {
                Some(partner) => twin_half_edges(obj, wb, partner),
                None => steps.entry(v_a).or_default().down = Some(wb),
            }
            // `wd = up(v_b)` (v_b -> vb_raised), this wall's own half-edge.
            // Its twin is `down(v_b)`: Pass 3's junction splice, an earlier
            // Pass-4 wall's `wb` at this vertex, or created now.
            let wd = obj.new_half_edge(v_b);
            match steps.entry(v_b).or_default().down.take() {
                Some(partner) => twin_half_edges(obj, wd, partner),
                None => steps.entry(v_b).or_default().up = Some(wd),
            }

            let wface =
                build_coplanar_wall(obj, h_sub, h_hole, wb, wd, va_raised, vb_raised, v_a, v_b)?;
            obj.shells[shell].faces.push(wface);
            created_faces.push(wface);
        }
    }

    // --- Pass 5: refit planes. ---
    // Moved face.
    {
        refit_face_plane(obj, face)?;
    }
    // Every original neighbor face — transverse walls (extended or step-
    // reshaped) and the wall-edge siblings are all unaffected in count here;
    // `neighbor_faces` already excludes the moved face itself.
    for &nf in neighbor_faces {
        refit_face_plane(obj, nf)?;
    }

    Ok(created_faces)
}

/// Per-original-vertex bookkeeping for `push_pull_build_walls`'s unweld:
/// `down` is a half-edge `raised(v) -> v`; `up` is `v -> raised(v)`. Each is
/// filled by whichever pass (junction splice, or an earlier coplanar wall at
/// an interior cut vertex) creates that direction first; the next consumer
/// twins against it instead of creating a duplicate.
#[derive(Default)]
struct StepEdges {
    down: Option<HalfEdgeId>,
    up: Option<HalfEdgeId>,
}

impl Object {
    /// A bare half-edge with `origin` set and everything else left as a
    /// placeholder, for callers that wire `next`/`prev`/`loop_id`/`edge`/`twin`
    /// themselves afterward (mirrors the `mk` closure in `extrude_sub_face`).
    fn new_half_edge(&mut self, origin: VertexId) -> HalfEdgeId {
        self.half_edges.insert(HalfEdge {
            origin,
            twin: None,
            next: HalfEdgeId::default(),
            prev: HalfEdgeId::default(),
            edge: EdgeId::default(),
            loop_id: LoopId::default(),
        })
    }
}

/// Refits `face`'s plane from its current outer loop, refusing the two
/// silent in-plane corruptions a vertex translation can produce: a boundary
/// that self-intersects (the face folded over itself laterally), and a
/// winding whose Newell normal flipped against the face's previous
/// orientation (net signed area crossed zero). Both keep every vertex on the
/// plane and every twin consistent, so the final validator backstop cannot
/// see them — this refit is the only place they are observable.
fn refit_face_plane(obj: &mut Object, face: FaceId) -> Result<(), PushPullError> {
    let outer_loop = obj.faces[face].outer_loop;
    let pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();
    if !polygon_is_simple(&pts) {
        return Err(PushPullError::NonManifoldResult);
    }
    let new_plane = Plane::from_polygon(&pts).map_err(|_| PushPullError::NonManifoldResult)?;
    if new_plane.normal().dot(obj.faces[face].plane.normal()) <= 0.0 {
        return Err(PushPullError::NonManifoldResult);
    }
    // Inner loops must stay strictly inside the (possibly reshaped) outer
    // boundary: a translation that swings a boundary edge across an imprinted
    // ring leaves the ring poking outside its face — planar, twin-consistent,
    // invisible to the validator, and fatal to the ring's own inverses.
    for &il in &obj.faces[face].inner_loops {
        let hole: Vec<Point3> = obj.loop_positions(il).collect();
        if boundaries_contact(&pts, &hole)
            || hole
                .iter()
                .any(|&hp| !point_inside_polygon(hp, &pts, new_plane.normal()))
        {
            return Err(PushPullError::NonManifoldResult);
        }
    }
    obj.faces[face].plane = new_plane;
    Ok(())
}

/// Validates the RESULT of a translate-and-build sweep
/// ([`push_pull_build_walls`]), reusing the sweep-validity checks the earlier
/// stretch path introduced: no zero-length boundary edge, every reshaped or
/// created face boundary well-formed ([`check_face_boundary`]), no face
/// interpenetration ([`faces_improperly_contact`]), and no disjoint shell
/// engulfed by the swept prism.
///
/// Together these BOUND the sweep by geometric validity, which is the whole
/// asymmetry the flat-face push/pull ruling asks for:
/// - An **outward pull** erects a prism of fresh material on the face; on a
///   convex-enough solid the moved face travels away from the fixed structure
///   and the new walls are non-degenerate, so every check passes and the pull
///   is unbounded regardless of neighbor angle. (The interpenetration and
///   engulfment checks still fire if a pull's walls would reach into a distant
///   part of a non-convex solid — a real self-intersection, refused typed.)
/// - An **inward push** refuses typed the instant the moved face conflicts
///   with the fixed structure it is pushed into — for a wedge's slant face the
///   very first infinitesimal push drives the moved face across a fixed
///   neighbor's interior, so it cannot be pushed in at all; the exact limit is
///   derived here, never hardcoded.
///
/// The object is inspected, never mutated (the caller commits only on `Ok`).
fn validate_sweep_result(
    obj: &Object,
    face: FaceId,
    orig_plane: Plane,
    old_outer: &[Point3],
    old_holes: &[Vec<Point3>],
    created_faces: &[FaceId],
    distance: f64,
) -> Result<(), PushPullError> {
    let normal = orig_plane.normal();

    // Touched = the moved face, the walls just erected, and every face still
    // adjacent to the moved face (the transverse neighbors reshaped by spliced
    // junction steps). These hold all the geometry the sweep created or moved;
    // the fixed neighbors beyond the new walls never moved.
    let mut touched: std::collections::BTreeSet<FaceId> = created_faces.iter().copied().collect();
    touched.insert(face);
    for lid in std::iter::once(obj.faces[face].outer_loop)
        .chain(obj.faces[face].inner_loops.iter().copied())
    {
        for h in obj.loop_half_edges(lid) {
            if let Some(t) = obj.half_edges[h].twin {
                touched.insert(obj.loops[obj.half_edges[t].loop_id].face);
            }
        }
    }

    // Zero-length boundary edges are the collapse signature (a wall or the
    // moved face itself pushed flat): positions merge while the half-edge
    // structure still holds distinct vertices, which the validator cannot see.
    for &tf in &touched {
        let loops: Vec<LoopId> = std::iter::once(obj.faces[tf].outer_loop)
            .chain(obj.faces[tf].inner_loops.iter().copied())
            .collect();
        for lid in loops {
            let pts: Vec<Point3> = obj.loop_positions(lid).collect();
            for (i, &pt) in pts.iter().enumerate() {
                if pt.approx_eq(pts[(i + 1) % pts.len()], tol::POINT_MERGE) {
                    return Err(PushPullError::NonManifoldResult);
                }
            }
        }
    }

    // The pushed-past-flat family (a self-intersecting boundary, a flipped
    // Newell normal, an escaped or overlapping hole ring) on every touched
    // face. A degenerate wall (from_polygon fails) is refused here too.
    for &tf in &touched {
        check_face_boundary(obj, tf)?;
    }

    // Interpenetration guard (DEVELOPMENT.md rule 4). Any touched face
    // contacting another face of the object away from the elements they
    // legitimately share (common vertices, twin edges) is a self-intersection
    // that stays planar and twin-consistent — invisible to the structural
    // validator, observable only here. This is what stops an inward push the
    // moment the moved face crosses a fixed neighbor.
    let all_faces: Vec<FaceId> = obj.faces.keys().collect();
    for &t in &touched {
        for &g in &all_faces {
            if g == t || (touched.contains(&g) && g <= t) {
                continue;
            }
            if faces_improperly_contact(obj, t, g) {
                return Err(PushPullError::NonManifoldResult);
            }
        }
    }

    // Engulfment guard: interpenetration cannot see a small disjoint shell the
    // moved face sweeps cleanly PAST — afterwards nothing intersects, but the
    // shell sits inside newly claimed material. The moved face translated
    // rigidly, so the swept region is the prism bounded by the original and
    // translated planes and, laterally, by the face's old/new footprints. Any
    // vertex of geometry uninvolved in the sweep that lands strictly inside
    // that prism was swept over: refuse. (Touched faces' own vertices are
    // exempt — they legitimately occupy the band.)
    let touched_verts: std::collections::BTreeSet<VertexId> = touched
        .iter()
        .flat_map(|&tf| {
            std::iter::once(obj.faces[tf].outer_loop)
                .chain(obj.faces[tf].inner_loops.iter().copied())
                .collect::<Vec<_>>()
        })
        .flat_map(|lid| obj.loop_half_edges(lid).collect::<Vec<_>>())
        .map(|h| obj.half_edges[h].origin)
        .collect();
    let new_outer: Vec<Point3> = obj.loop_positions(obj.faces[face].outer_loop).collect();
    let new_holes: Vec<Vec<Point3>> = obj.faces[face]
        .inner_loops
        .iter()
        .map(|&il| obj.loop_positions(il).collect())
        .collect();
    let inside_footprint = |p: Point3, outer: &[Point3], holes: &[Vec<Point3>]| {
        point_inside_polygon(p, outer, normal)
            && !holes.iter().any(|h| point_inside_polygon(p, h, normal))
    };
    for (vid, vtx) in &obj.vertices {
        if touched_verts.contains(&vid) {
            continue;
        }
        let sd = orig_plane.signed_distance(vtx.position);
        let in_band = if distance > 0.0 {
            sd > tol::POINT_MERGE && sd < distance - tol::POINT_MERGE
        } else {
            sd < -tol::POINT_MERGE && sd > distance + tol::POINT_MERGE
        };
        if in_band
            && (inside_footprint(vtx.position, old_outer, old_holes)
                || inside_footprint(vtx.position, &new_outer, &new_holes))
        {
            return Err(PushPullError::NonManifoldResult);
        }
    }

    Ok(())
}

/// Whether faces `a` and `b` of one Object make contact anywhere other than
/// along the elements they legitimately share — their common vertices and
/// their twinned (shared) edges. A boundary segment of one face crossing the
/// other's interior, poking a vertex into it, or overlapping it coplanarly
/// is improper contact: interpenetrating geometry that stays planar and
/// twin-consistent, which the structural validator cannot see.
///
/// Exactness posture: crossing points are classified against the other
/// face's *strict* interior, and contact within [`tol::POINT_MERGE`] of a
/// shared vertex or shared edge is legitimate (that is exactly how adjacent
/// faces of a manifold touch). Grazing contact between unrelated faces
/// (tangencies) refuses conservatively — the kernel cannot prove it safe.
fn faces_improperly_contact(obj: &Object, a: FaceId, b: FaceId) -> bool {
    let verts_of = |f: FaceId| -> std::collections::BTreeSet<VertexId> {
        std::iter::once(obj.faces[f].outer_loop)
            .chain(obj.faces[f].inner_loops.iter().copied())
            .flat_map(|lid| obj.loop_half_edges(lid).collect::<Vec<_>>())
            .map(|h| obj.half_edges[h].origin)
            .collect()
    };
    let va = verts_of(a);
    let vb = verts_of(b);
    let shared_pts: Vec<Point3> = va
        .intersection(&vb)
        .map(|&v| obj.vertices[v].position)
        .collect();
    // Segments of `a` twinned into `b` (the legitimately shared edges).
    let mut shared_segs: Vec<(Point3, Point3)> = Vec::new();
    for lid in
        std::iter::once(obj.faces[a].outer_loop).chain(obj.faces[a].inner_loops.iter().copied())
    {
        for h in obj.loop_half_edges(lid) {
            if let Some(t) = obj.half_edges[h].twin
                && obj.loops[obj.half_edges[t].loop_id].face == b
            {
                let p = obj.vertices[obj.half_edges[h].origin].position;
                let q = obj.vertices[obj.half_edges[obj.half_edges[h].next].origin].position;
                shared_segs.push((p, q));
            }
        }
    }
    boundary_pokes_face(obj, a, b, &shared_pts, &shared_segs)
        || boundary_pokes_face(obj, b, a, &shared_pts, &shared_segs)
}

/// One direction of [`faces_improperly_contact`]: whether any boundary
/// segment of `x` contacts the interior of face `y` away from their shared
/// vertices/edges. Segments strictly on one side of `y`'s plane cannot;
/// crossing segments are tested at the crossing point; segments lying in
/// `y`'s plane are cut at every contact with `y`'s boundary and probed one
/// interior point per piece (coplanar overlap).
fn boundary_pokes_face(
    obj: &Object,
    x: FaceId,
    y: FaceId,
    shared_pts: &[Point3],
    shared_segs: &[(Point3, Point3)],
) -> bool {
    let y_plane = obj.faces[y].plane;
    let y_normal = y_plane.normal();
    let y_outer: Vec<Point3> = obj.loop_positions(obj.faces[y].outer_loop).collect();
    let y_holes: Vec<Vec<Point3>> = obj.faces[y]
        .inner_loops
        .iter()
        .map(|&il| obj.loop_positions(il).collect())
        .collect();
    let in_y_interior = |p: Point3| -> bool {
        point_inside_polygon(p, &y_outer, y_normal)
            && !y_holes.iter().any(|h| point_inside_polygon(p, h, y_normal))
    };
    let is_shared_contact = |p: Point3| -> bool {
        shared_pts.iter().any(|&s| s.approx_eq(p, tol::POINT_MERGE))
            || shared_segs
                .iter()
                .any(|&(s0, s1)| point_on_segment(p, s0, s1, tol::POINT_MERGE))
    };
    for lid in
        std::iter::once(obj.faces[x].outer_loop).chain(obj.faces[x].inner_loops.iter().copied())
    {
        for h in obj.loop_half_edges(lid) {
            // A twinned (shared) edge is legitimate contact by definition.
            if let Some(t) = obj.half_edges[h].twin
                && obj.loops[obj.half_edges[t].loop_id].face == y
            {
                continue;
            }
            let p = obj.vertices[obj.half_edges[h].origin].position;
            let q = obj.vertices[obj.half_edges[obj.half_edges[h].next].origin].position;
            let sdp = y_plane.signed_distance(p);
            let sdq = y_plane.signed_distance(q);
            if (sdp > tol::PLANE_DIST && sdq > tol::PLANE_DIST)
                || (sdp < -tol::PLANE_DIST && sdq < -tol::PLANE_DIST)
            {
                continue; // strictly one side of y's plane: no contact.
            }
            if sdp.abs() <= tol::PLANE_DIST && sdq.abs() <= tol::PLANE_DIST {
                // Segment lies in y's plane. Its overlap with y's material
                // is a union of sub-intervals whose endpoints are contacts
                // with y's boundary (or the segment's own ends), which
                // makes the complete test two-part:
                // (1) any boundary contact away from the shared
                //     vertices/edges is improper on its own — exact 2D
                //     segment/segment intersections against every boundary
                //     segment of y (point sampling misses the off-center
                //     plus-sign crossing);
                // (2) cut the segment at ALL boundary contacts, whitelisted
                //     ones included, and probe each piece's midpoint for
                //     non-shared strict interior containment. Every
                //     material-overlap interval is bounded by boundary
                //     contacts, so it contains a probed midpoint — this
                //     catches a chord whose only boundary contacts are
                //     whitelisted (a diagonal between two shared corners
                //     slicing across y), which crossing detection alone
                //     cannot see, and it subsumes endpoint testing for a
                //     segment lying wholly inside y (no contact at all:
                //     the single piece is the whole segment).
                let len = (q - p).length();
                if len <= tol::POINT_MERGE {
                    continue; // degenerate segment: refused upstream anyway.
                }
                let dir = (q - p) / len;
                let mut cuts: Vec<f64> = vec![0.0, len];
                for ylid in std::iter::once(obj.faces[y].outer_loop)
                    .chain(obj.faces[y].inner_loops.iter().copied())
                {
                    for yh in obj.loop_half_edges(ylid) {
                        let r = obj.vertices[obj.half_edges[yh].origin].position;
                        let s =
                            obj.vertices[obj.half_edges[obj.half_edges[yh].next].origin].position;
                        for c in coplanar_segment_contacts(p, q, r, s, y_normal) {
                            if !is_shared_contact(c) {
                                return true;
                            }
                            cuts.push((c - p).dot(dir).clamp(0.0, len));
                        }
                    }
                }
                cuts.sort_by(f64::total_cmp);
                cuts.dedup_by(|a, b| (*a - *b).abs() <= tol::POINT_MERGE);
                for pair in cuts.windows(2) {
                    let mid = p + dir * ((pair[0] + pair[1]) * 0.5);
                    if in_y_interior(mid) && !is_shared_contact(mid) {
                        return true;
                    }
                }
            } else {
                // Transversal crossing: the single, exact plane-crossing
                // point is the only place the segment can touch y.
                let c = p + (q - p) * (sdp / (sdp - sdq));
                if in_y_interior(c) && !is_shared_contact(c) {
                    return true;
                }
            }
        }
    }
    false
}

/// Contact points between two segments `p→q` and `r→s` lying in a common
/// plane with unit normal `n`: a proper crossing or endpoint touch yields
/// the single intersection point; collinear overlap yields both extremes of
/// the shared interval. Empty when the segments miss each other.
///
/// Used by [`boundary_pokes_face`]'s coplanar branch, which must whitelist
/// each contact against the faces' shared vertices/edges — so this returns
/// the actual contact locations rather than a boolean.
fn coplanar_segment_contacts(p: Point3, q: Point3, r: Point3, s: Point3, n: Vec3) -> Vec<Point3> {
    let d1 = q - p;
    let d2 = s - r;
    let (Ok(u1), Ok(u2)) = (d1.normalized(), d2.normalized()) else {
        return Vec::new(); // zero-length segment: refused upstream anyway.
    };
    let w = r - p;
    // Signed sine of the angle between the segments, in the plane.
    let denom = u1.cross(u2).dot(n);
    if denom.abs() <= tol::NORMAL_DIRECTION {
        // Parallel. Collinear only if `r` sits on p→q's carrier line.
        let off = w - u1 * w.dot(u1);
        if off.length() > tol::POINT_MERGE {
            return Vec::new();
        }
        // Overlap interval of r→s on p→q, in p→q's arc length.
        let len1 = d1.length();
        let (tr, ts) = (w.dot(u1), (s - p).dot(u1));
        let lo = tr.min(ts).max(0.0);
        let hi = tr.max(ts).min(len1);
        if lo > hi {
            return Vec::new();
        }
        return vec![p + u1 * lo, p + u1 * hi];
    }
    // Proper (or endpoint-touching) intersection: solve p + t·u1 = r + v·u2
    // by crossing both sides against each direction.
    let t = w.cross(u2).dot(n) / denom;
    let v = w.cross(u1).dot(n) / denom;
    if t >= -tol::POINT_MERGE
        && t <= d1.length() + tol::POINT_MERGE
        && v >= -tol::POINT_MERGE
        && v <= d2.length() + tol::POINT_MERGE
    {
        return vec![p + u1 * t];
    }
    Vec::new()
}

/// The validation half of [`refit_face_plane`], without storing the fitted
/// plane: refuses a self-intersecting outer boundary, a winding whose Newell
/// normal flipped against the face's stored orientation, and any hole-ring
/// corruption a reshaped face can carry — a hole that self-intersects, a hole
/// whose winding inverted, a hole touching or escaping the outer boundary, and
/// two holes overlapping each other. Used by [`validate_sweep_result`] to
/// check each touched face of a translate-and-build push while keeping the
/// stored (exact) planes; the pure translation moves rings rigidly and cannot
/// distort them, so `refit_face_plane` doesn't need these.
fn check_face_boundary(obj: &Object, face: FaceId) -> Result<(), PushPullError> {
    let outer_loop = obj.faces[face].outer_loop;
    let pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();
    if !polygon_is_simple(&pts) {
        return Err(PushPullError::NonManifoldResult);
    }
    let fitted = Plane::from_polygon(&pts).map_err(|_| PushPullError::NonManifoldResult)?;
    let normal = obj.faces[face].plane.normal();
    if fitted.normal().dot(normal) <= 0.0 {
        return Err(PushPullError::NonManifoldResult);
    }
    let mut holes: Vec<Vec<Point3>> = Vec::new();
    for &il in &obj.faces[face].inner_loops {
        let hole: Vec<Point3> = obj.loop_positions(il).collect();
        // A slid ring must still be a simple polygon...
        if !polygon_is_simple(&hole) {
            return Err(PushPullError::NonManifoldResult);
        }
        // ...wound opposite the outer loop (a ring pushed through the apex
        // of its tapered walls comes back inverted: still simple, still
        // planar, but enclosing negated area — the validator cannot see it)...
        if signed_area_on_plane(&hole, normal) >= 0.0 {
            return Err(PushPullError::NonManifoldResult);
        }
        // ...strictly inside the outer boundary...
        if boundaries_contact(&pts, &hole)
            || hole
                .iter()
                .any(|&hp| !point_inside_polygon(hp, &pts, normal))
        {
            return Err(PushPullError::NonManifoldResult);
        }
        // ...and disjoint from every other hole (rings that slid into each
        // other claim the same area twice).
        for prev in &holes {
            if boundaries_contact(prev, &hole)
                || hole
                    .iter()
                    .any(|&hp| point_inside_polygon(hp, prev, normal))
                || prev
                    .iter()
                    .any(|&pp| point_inside_polygon(pp, &hole, normal))
            {
                return Err(PushPullError::NonManifoldResult);
            }
        }
        holes.push(hole);
    }
    Ok(())
}

/// Links two half-edges as twins of a shared (new) `Edge`. Does not touch
/// `next`/`prev`/`loop_id` — callers that need this freshly created (the
/// interior-cut case) wire those themselves; callers consuming an
/// already-spliced step (the junction case) leave them as spliced.
fn twin_half_edges(obj: &mut Object, a: HalfEdgeId, b: HalfEdgeId) {
    let edge = obj.edges.insert(Edge {
        half_edge: a,
        twin_half_edge: Some(b),
        curve: None,
    });
    obj.half_edges[a].twin = Some(b);
    obj.half_edges[a].edge = edge;
    obj.half_edges[b].twin = Some(a);
    obj.half_edges[b].edge = edge;
}

/// Splices a fresh half-edge `origin -> existing_next.origin` into `loop_id`
/// immediately after `after`, i.e. between `after` and its current `next`.
/// Returns the new half-edge (still twin-less; the caller links it).
fn splice_after(
    obj: &mut Object,
    loop_id: LoopId,
    after: HalfEdgeId,
    origin: VertexId,
    _dest_hint: VertexId,
) -> HalfEdgeId {
    let old_next = obj.half_edges[after].next;
    let h = obj.half_edges.insert(HalfEdge {
        origin,
        twin: None,
        next: old_next,
        prev: after,
        edge: EdgeId::default(),
        loop_id,
    });
    obj.half_edges[after].next = h;
    obj.half_edges[old_next].prev = h;
    obj.vertices[origin].outgoing = h;
    h
}

/// Splices a fresh half-edge `origin -> raised` into `loop_id` immediately
/// before `before`, then repoints `before`'s own origin to `raised` (the
/// junction vertex moves with the moved face from this point in the wall's
/// loop onward). Returns the new half-edge (still twin-less).
fn splice_before(
    obj: &mut Object,
    loop_id: LoopId,
    before: HalfEdgeId,
    origin: VertexId,
    raised: VertexId,
) -> HalfEdgeId {
    let old_prev = obj.half_edges[before].prev;
    let h = obj.half_edges.insert(HalfEdge {
        origin,
        twin: None,
        next: before,
        prev: old_prev,
        edge: EdgeId::default(),
        loop_id,
    });
    obj.half_edges[old_prev].next = h;
    obj.half_edges[before].prev = h;
    obj.half_edges[before].origin = raised;
    obj.vertices[raised].outgoing = before;
    obj.vertices[origin].outgoing = h;
    h
}

/// Builds the quad wall along one coplanar boundary edge of the moved face,
/// mirroring `extrude_sub_face`'s per-edge construction (`wa`/`wb`/`wc`/`wd`).
///
/// `h_sub` is the moved face's edge, already repointed by Pass 2 to run
/// `va_raised -> vb_raised`. `h_hole` is the sibling's existing boundary
/// half-edge, untouched, running `v_b -> v_a`. `wb` (`va_raised -> v_a`) and
/// `wd` (`v_b -> vb_raised`) are the two verticals, already minted by the
/// caller. Each is twinned either before this call (against a Pass-3
/// junction splice embedded in a transverse wall's loop, or against an
/// earlier coplanar wall's vertical) or retroactively, when the NEXT
/// coplanar wall around an interior cut vertex takes it from `steps` — so a
/// vertical may legitimately still be twin-less here. This function only
/// wires their `next`/`prev`/`loop_id` into the new wall's own quad loop,
/// never their `twin`/`edge`; the final validator backstop refuses any
/// pairing the pass fails to complete.
#[allow(clippy::too_many_arguments)]
fn build_coplanar_wall(
    obj: &mut Object,
    h_sub: HalfEdgeId,
    h_hole: HalfEdgeId,
    wb: HalfEdgeId,
    wd: HalfEdgeId,
    va_raised: VertexId,
    vb_raised: VertexId,
    v_a: VertexId,
    v_b: VertexId,
) -> Result<FaceId, PushPullError> {
    // wa twins h_sub: h_sub runs va_raised -> vb_raised, so wa runs the
    // opposite direction, vb_raised -> va_raised.
    let wa = obj.new_half_edge(vb_raised);
    // wc twins h_hole: h_hole runs v_b -> v_a, so wc runs v_a -> v_b.
    let wc = obj.new_half_edge(v_a);

    let wloop = obj.loops.insert(Loop {
        face: FaceId::default(),
        first_half_edge: HalfEdgeId::default(),
        kind: LoopKind::Outer,
    });
    // Quad order: wa (vb_raised -> va_raised) -> wb (va_raised -> v_a) ->
    // wc (v_a -> v_b) -> wd (v_b -> vb_raised) -> back to wa.
    for &h in &[wa, wb, wc, wd] {
        obj.half_edges[h].loop_id = wloop;
    }
    obj.half_edges[wa].next = wb;
    obj.half_edges[wb].next = wc;
    obj.half_edges[wc].next = wd;
    obj.half_edges[wd].next = wa;
    obj.half_edges[wb].prev = wa;
    obj.half_edges[wc].prev = wb;
    obj.half_edges[wd].prev = wc;
    obj.half_edges[wa].prev = wd;
    obj.loops[wloop].first_half_edge = wa;

    // wa <-> h_sub (new edge; h_sub keeps the moved face's side).
    obj.half_edges[wa].twin = Some(h_sub);
    obj.half_edges[h_sub].twin = Some(wa);
    let e_top = obj.edges.insert(Edge {
        half_edge: h_sub,
        twin_half_edge: Some(wa),
        curve: None,
    });
    obj.half_edges[h_sub].edge = e_top;
    obj.half_edges[wa].edge = e_top;

    // wc <-> h_hole (reuse the old shared edge — the sibling's boundary
    // half-edge is untouched, just re-twinned onto the wall instead of the
    // moved face).
    let old_edge = obj.half_edges[h_hole].edge;
    obj.half_edges[wc].twin = Some(h_hole);
    obj.half_edges[h_hole].twin = Some(wc);
    obj.edges[old_edge].half_edge = h_hole;
    obj.edges[old_edge].twin_half_edge = Some(wc);
    obj.half_edges[h_hole].edge = old_edge;
    obj.half_edges[wc].edge = old_edge;

    obj.vertices[vb_raised].outgoing = wa;
    obj.vertices[va_raised].outgoing = wb;
    obj.vertices[v_a].outgoing = wc;

    let wall_pts = [
        obj.vertices[vb_raised].position,
        obj.vertices[va_raised].position,
        obj.vertices[v_a].position,
        obj.vertices[v_b].position,
    ];
    let plane = Plane::from_polygon(&wall_pts).map_err(|_| PushPullError::NonManifoldResult)?;
    let wface = obj.faces.insert(Face {
        outer_loop: wloop,
        inner_loops: Vec::new(),
        plane,
        material: None,
        uv_frame: None,
        surface: None,
    });
    obj.loops[wloop].face = wface;
    Ok(wface)
}

/// How a path endpoint lands on the outer loop.
#[derive(Debug, Clone)]
enum EndpointHit {
    /// Snapped exactly to an existing vertex.
    Vertex(VertexId),
    /// Lies in the interior of a boundary edge.
    /// `he` is the half-edge (origin → dest) that gets split; `pos` is the
    /// path endpoint itself — already validated to lie on the edge within
    /// [`tol::POINT_MERGE`] — and is where the split vertex is placed.
    /// Placing the CALLER's point rather than its re-interpolated projection
    /// keeps replayed splits bit-exact (determinism lane): an undo captures
    /// the split vertex's live position into the reconstructed path, and the
    /// redo must land the new vertex on exactly those bits, not one ulp off.
    Edge { he: HalfEdgeId, pos: Point3 },
}

/// The resolved 3-D position of an endpoint hit.
fn endpoint_position(obj: &Object, hit: &EndpointHit) -> Point3 {
    match hit {
        EndpointHit::Vertex(v) => obj.vertices[*v].position,
        EndpointHit::Edge { pos, .. } => *pos,
    }
}

/// Try to classify `pt` against the `outer_hes` loop of an object.
///
/// Returns:
/// - `Ok(Some(hit))` if on the outer loop
/// - `Ok(None)` if not on the outer loop at all (caller handles the inner-loop case)
fn classify_endpoint(
    obj: &Object,
    outer_hes: &[HalfEdgeId],
    pt: Point3,
) -> Result<Option<EndpointHit>, StickyError> {
    // First check vertex hits.
    for &h in outer_hes {
        let v = obj.half_edges[h].origin;
        if obj.vertices[v].position.approx_eq(pt, tol::POINT_MERGE) {
            return Ok(Some(EndpointHit::Vertex(v)));
        }
    }
    // Then check edge-interior hits.
    for &h in outer_hes {
        let origin = obj.vertices[obj.half_edges[h].origin].position;
        let dest_he = obj.half_edges[h].next;
        let dest = obj.vertices[obj.half_edges[dest_he].origin].position;
        let edge_vec = dest - origin;
        let edge_len_sq = edge_vec.length_squared();
        if edge_len_sq < tol::POINT_MERGE * tol::POINT_MERGE {
            continue;
        }
        // Project pt onto the edge line.
        let t = (pt - origin).dot(edge_vec) / edge_len_sq;
        if t > tol::POINT_MERGE && t < 1.0 - tol::POINT_MERGE {
            let closest = origin + edge_vec * t;
            if (pt - closest).length_squared() <= tol::POINT_MERGE * tol::POINT_MERGE {
                return Ok(Some(EndpointHit::Edge { he: h, pos: pt }));
            }
        }
    }
    Ok(None)
}

/// Applies a recorded attribute restoration after a split (undo of a
/// merge): each present snapshot lands on whichever of the two result faces
/// contains its interior point. An absent snapshot is the merge's
/// best-effort fallback (the face was too thin to pin — see
/// [`FaceMergeReport::prior_attrs`]): that face keeps the attributes it
/// inherited from the split. A PRESENT snapshot matching neither face means
/// the split landed differently than the recorded merge — impossible for a
/// recorded inverse, so it is a debug-build assertion — and likewise leaves
/// the inherited attributes rather than guessing.
fn apply_split_restore(
    obj: &mut Object,
    new_faces: &[FaceId; 2],
    restore: &[Option<FaceAttrsAt>; 2],
) {
    for r in restore.iter().flatten() {
        let target = new_faces.iter().copied().find(|&f| {
            let face = &obj.faces[f];
            let n = face.plane.normal();
            if face.plane.signed_distance(r.point).abs() > tol::PLANE_DIST {
                return false;
            }
            let outer: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
            if !point_inside_polygon(r.point, &outer, n) {
                return false;
            }
            !face.inner_loops.iter().any(|&il| {
                let hole: Vec<Point3> = obj.loop_positions(il).collect();
                point_inside_polygon(r.point, &hole, n)
            })
        });
        debug_assert!(
            target.is_some(),
            "a recorded restore anchor must land in one of the split's fragments"
        );
        if let Some(f) = target {
            obj.faces[f].set_attrs(r.attrs);
        }
    }
}

/// Core split surgery on a cloned object.
///
/// Preconditions:
/// - `face` exists
/// - `ep0` and `ep1` are valid hits on `face`'s outer loop
/// - interior path points are valid
/// - path is simple
fn do_split_face(
    obj: &mut Object,
    face: FaceId,
    path: &[Point3],
    ep0: &EndpointHit,
    ep1: &EndpointHit,
) -> Result<FaceSplitReport, StickyError> {
    let face_plane = obj.faces[face].plane;
    let inner_loops: Vec<LoopId> = obj.faces[face].inner_loops.clone();

    // 1. If either endpoint is an edge hit, split that boundary edge.
    //
    //    Ordering subtlety: a STRAIGHT 2-point path cannot have both
    //    endpoints interior to the same boundary edge (it would be collinear
    //    with it), but a MULTI-SEGMENT path legally can — a "lens" cut that
    //    leaves the edge and returns to it (Arc tool, V-shaped Line chains).
    //    In that case ep0's `split_boundary_edge` CONSUMES the shared
    //    half-edge, so ep1's stored `he` key is dead by the time we get to
    //    it; blindly indexing it panics ("invalid SlotMap key used" — the
    //     lens-cut bug). ep1 therefore re-resolves against the
    //    CURRENT outer loop when its key is gone. For distinct edges,
    //    slotmap key stability makes order irrelevant, as before.
    let mut split_boundary_edges: Vec<(EdgeId, [EdgeId; 2])> = Vec::new();

    let v0 = match ep0 {
        EndpointHit::Vertex(v) => *v,
        EndpointHit::Edge { he, pos } => {
            let (v, dead_edge, new_edges) = split_boundary_edge(obj, *he, *pos);
            split_boundary_edges.push((dead_edge, new_edges));
            v
        }
    };

    // Re-read the outer loop half-edges now (ep0 split may have changed the loop).
    let outer_loop = obj.faces[face].outer_loop;

    let v1 = match ep1 {
        EndpointHit::Vertex(v) => *v,
        EndpointHit::Edge { he, pos } => {
            let live_he = if obj.half_edges.contains_key(*he) {
                // Distinct edge (or ep0 was a vertex hit): the stored key is
                // still the same segment — slotmap key stability.
                *he
            } else {
                // Lens cut: ep0's split consumed this half-edge. Re-resolve
                // the endpoint against the CURRENT outer loop — the
                // containing sub-edge lies on the same carrier line as the
                // original edge, so projection reproduces the classification
                // snap (the split vertex is still placed at `pos` itself).
                // Typed error if nothing contains it (rule 4: never panic on
                // a user-reachable path).
                let mut found: Option<HalfEdgeId> = None;
                for h2 in obj.loop_half_edges(obj.faces[face].outer_loop) {
                    let p = obj.vertices[obj.half_edges[h2].origin].position;
                    let next = obj.half_edges[h2].next;
                    let q = obj.vertices[obj.half_edges[next].origin].position;
                    let seg = q - p;
                    let len_sq = seg.dot(seg);
                    if len_sq <= tol::POINT_MERGE * tol::POINT_MERGE {
                        continue;
                    }
                    // The parameter must lie strictly INSIDE (0, 1), exactly
                    // as `classify_endpoint` requires: an unclamped
                    // projection extrapolates onto the carrier line, where a
                    // point on a COLLINEAR SIBLING sub-edge (ep0's split just
                    // divided the original edge in two) sits at zero distance
                    // from its extrapolation while lying outside this
                    // segment — endpoint-distance checks alone cannot tell
                    // those apart, and splitting the wrong sub-edge wires a
                    // self-overlapping (validator-invisible) ring.
                    let s = (*pos - p).dot(seg) / len_sq;
                    if s <= tol::POINT_MERGE || s >= 1.0 - tol::POINT_MERGE {
                        continue;
                    }
                    let proj = p + seg * s;
                    if (*pos - proj).length() <= tol::POINT_MERGE {
                        found = Some(h2);
                        break;
                    }
                }
                match found {
                    Some(hit) => hit,
                    None => return Err(StickyError::EndpointNotOnBoundary { which: 1 }),
                }
            };
            let (v, dead_edge, new_edges) = split_boundary_edge(obj, live_he, *pos);
            split_boundary_edges.push((dead_edge, new_edges));
            v
        }
    };

    // 2. Now we have vertices v0 and v1 on the outer loop.
    //    Find the two half-edges that START at v0 and v1 on this outer loop.
    let outer_hes: Vec<HalfEdgeId> = obj.loop_half_edges(outer_loop).collect();

    let he_at_v0 = outer_hes
        .iter()
        .copied()
        .find(|&h| obj.half_edges[h].origin == v0)
        .expect("v0 is on the outer loop");
    let he_at_v1 = outer_hes
        .iter()
        .copied()
        .find(|&h| obj.half_edges[h].origin == v1)
        .expect("v1 is on the outer loop");

    // 3. Build the interior path vertices.
    //    path[0] and path.last() are the endpoints; intermediate points are new vertices.
    let interior_pts: Vec<Point3> = path[1..path.len() - 1].to_vec();
    let interior_verts: Vec<VertexId> = interior_pts
        .iter()
        .map(|&pos| {
            // Placeholder outgoing; will be set below.
            obj.vertices.insert(Vertex {
                position: pos,
                outgoing: HalfEdgeId::default(),
            })
        })
        .collect();

    // 4. The cut produces two new directed half-edge chains along the path:
    //    - "forward": v0 → ... → v1 (used in face A's loop)
    //    - "backward": v1 → ... → v0 (used in face B's loop)
    //
    //    These come in twin pairs: one new Edge per path segment.
    //
    //    Path vertices in order: v0, interior_verts..., v1
    let path_verts: Vec<VertexId> = {
        let mut v = vec![v0];
        v.extend_from_slice(&interior_verts);
        v.push(v1);
        v
    };
    let n_path_segs = path_verts.len() - 1;

    // Insert the half-edges for the cut.
    let mut cut_fwd: Vec<HalfEdgeId> = Vec::with_capacity(n_path_segs);
    let mut cut_bwd: Vec<HalfEdgeId> = Vec::with_capacity(n_path_segs);
    let mut new_edges: Vec<EdgeId> = Vec::with_capacity(n_path_segs);

    // We'll assign loop_ids after creating the loops.
    // Use a placeholder loop id for now.
    let placeholder_loop = obj.faces[face].outer_loop; // will be fixed up

    // First pass: create all forward and backward half-edges without wiring twins yet.
    // cut_fwd[i]: goes from path_verts[i] to path_verts[i+1]    (0 = v0, last = v1)
    // cut_bwd[i]: goes from path_verts[n_path_segs-i] to path_verts[n_path_segs-i-1]
    //             i.e. cut_bwd[0] starts at v1, cut_bwd.last() ends at v0.
    // Twin pairing: cut_fwd[i] ↔ cut_bwd[n_path_segs-1-i]
    //   (cut_fwd[i] goes v→w, cut_bwd[n_path_segs-1-i] goes w→v).
    for i in 0..n_path_segs {
        let h_fwd = obj.half_edges.insert(HalfEdge {
            origin: path_verts[i],
            twin: None, // set in second pass
            next: HalfEdgeId::default(),
            prev: HalfEdgeId::default(),
            edge: EdgeId::default(),
            loop_id: placeholder_loop, // fixed later
        });
        let h_bwd = obj.half_edges.insert(HalfEdge {
            origin: path_verts[n_path_segs - i], // reverse direction
            twin: None,
            next: HalfEdgeId::default(),
            prev: HalfEdgeId::default(),
            edge: EdgeId::default(),
            loop_id: placeholder_loop, // fixed later
        });
        cut_fwd.push(h_fwd);
        cut_bwd.push(h_bwd);
    }
    // Second pass: wire twin pairs and create Edge records.
    // cut_fwd[i] (path_verts[i]→path_verts[i+1]) pairs with
    // cut_bwd[n_path_segs-1-i] (path_verts[i+1]→path_verts[i]).
    for (i, &h_fwd) in cut_fwd.iter().enumerate() {
        let j = n_path_segs - 1 - i; // twin index in cut_bwd
        let h_bwd = cut_bwd[j];
        let edge_id = obj.edges.insert(Edge {
            half_edge: h_fwd,
            twin_half_edge: Some(h_bwd),
            curve: None,
        });
        obj.half_edges[h_fwd].edge = edge_id;
        obj.half_edges[h_bwd].edge = edge_id;
        obj.half_edges[h_fwd].twin = Some(h_bwd);
        obj.half_edges[h_bwd].twin = Some(h_fwd);
        new_edges.push(edge_id);
    }
    // cut_bwd is in order v1→v0:
    // cut_bwd[0].origin = path_verts[n_path_segs] = v1
    // cut_bwd[1].origin = path_verts[n_path_segs - 1]
    // cut_bwd.last().origin = path_verts[1]  (the last interior vertex or v0 if single-seg)
    // So cut_bwd is already in the correct backward order (v1 → ... → v0).

    // 5. Wire next/prev within each cut chain.
    for i in 0..n_path_segs {
        if i + 1 < n_path_segs {
            obj.half_edges[cut_fwd[i]].next = cut_fwd[i + 1];
            obj.half_edges[cut_fwd[i + 1]].prev = cut_fwd[i];
        }
    }
    for i in 0..n_path_segs {
        if i + 1 < n_path_segs {
            obj.half_edges[cut_bwd[i]].next = cut_bwd[i + 1];
            obj.half_edges[cut_bwd[i + 1]].prev = cut_bwd[i];
        }
    }

    // 6. Now we need to split the outer loop into two loops around the cut.
    //    Current outer loop: ... → he_at_v0 → [chain from v0 to v1] → he_at_v1 → [chain from v1 to v0] → he_at_v0 → ...
    //
    //    Face A loop: he_at_v0 → [outer chain v0→v1 (CCW)] → [predecessor of he_at_v1] → cut_bwd[0..] → (last cut_bwd) → he_at_v0
    //    Wait, let me think again.
    //
    //    The outer loop goes: ... → prev(he_at_v0) → he_at_v0 → next(he_at_v0) → ... → prev(he_at_v1) → he_at_v1 → next(he_at_v1) → ... → prev(he_at_v0) → ...
    //
    //    Let chain_A = [he_at_v0, next(he_at_v0), ..., prev(he_at_v1)]  (from v0 toward v1 around the loop)
    //    Let chain_B = [he_at_v1, next(he_at_v1), ..., prev(he_at_v0)]  (from v1 back to v0)
    //
    //    New face A loop: chain_A + cut_bwd  (outer chain v0→v1, then cut backward v1→v0)
    //    New face B loop: chain_B + cut_fwd  (outer chain v1→v0→..., then cut forward v0→v1)
    //
    //    Wait: we want CCW winding to be maintained.  The original face has CCW outer loop.
    //    Splitting with a path from v0 to v1:
    //    - Face A's boundary = outer-chain from v0 to v1 + cut-back from v1 to v0
    //    - Face B's boundary = outer-chain from v1 to v0 + cut-forward from v0 to v1
    //
    //    The winding is inherited from the original loop.

    // Collect chain_A: half-edges from he_at_v0 up to (but not including) he_at_v1.
    // Collect chain_B: half-edges from he_at_v1 up to (but not including) he_at_v0.
    let chain_a: Vec<HalfEdgeId> = collect_chain(obj, he_at_v0, he_at_v1);
    let chain_b: Vec<HalfEdgeId> = collect_chain(obj, he_at_v1, he_at_v0);

    // 7. Create two new loops and a new face.
    //    face_a (keeps the original FaceId? no — both are new).
    //    Actually, to keep it simple: create loop_a and loop_b;
    //    keep the original face id for face_a; create a new face for face_b.

    // Update the original outer_loop to be loop_a (face A).
    // Create a new loop for face B.
    let loop_b_id = obj.loops.insert(Loop {
        face: FaceId::default(), // set below
        first_half_edge: HalfEdgeId::default(),
        kind: LoopKind::Outer,
    });

    // Face B is a new face (face A reuses the original FaceId).
    let face_b_id = obj.faces.insert(Face {
        outer_loop: loop_b_id,
        inner_loops: Vec::new(), // holes assigned below
        plane: face_plane,       // same plane as original
        // Both halves of a split inherit the original face's material, UV
        // frame ( +  extension), and analytic surface; face A keeps them by
        // reusing its FaceId.
        material: obj.faces[face].material,
        uv_frame: obj.faces[face].uv_frame,
        surface: obj.faces[face].surface,
    });
    obj.loops[loop_b_id].face = face_b_id;

    // 8. Wire face A's loop: chain_a + cut_bwd.
    //    Loop A uses the original loop id (outer_loop).
    //    cut_bwd goes from v1 → v0, which is exactly what we need after chain_a.
    //
    //    chain_a ends at prev(he_at_v1): its last element leads TO v1.
    //    cut_bwd[0] starts at v1 (its origin = v1).
    //    cut_bwd last: ends at v0 (the next half-edge after the last cut_bwd would be he_at_v0).

    // The full sequence for face A:
    //   chain_a[0], chain_a[1], ..., chain_a.last(), cut_bwd[0], ..., cut_bwd.last()
    // then wraps back to chain_a[0].

    let loop_a_seq: Vec<HalfEdgeId> = chain_a
        .iter()
        .copied()
        .chain(cut_bwd.iter().copied())
        .collect();

    let loop_b_seq: Vec<HalfEdgeId> = chain_b
        .iter()
        .copied()
        .chain(cut_fwd.iter().copied())
        .collect();

    wire_loop_sequence(obj, &loop_a_seq, outer_loop, face);
    wire_loop_sequence(obj, &loop_b_seq, loop_b_id, face_b_id);

    // Update interior vertex outgoing pointers to cut half-edges.
    // interior_verts[i] = path_verts[i+1]; the departing forward half-edge is cut_fwd[i+1].
    for i in 0..interior_verts.len() {
        obj.vertices[interior_verts[i]].outgoing = cut_fwd[i + 1];
    }
    // v0 and v1 outgoing: they might already have valid outgoing (if they were pre-existing
    // vertices), but we should make sure v0's outgoing is in a valid loop.
    // The safest update: set outgoing to the first half-edge in a loop that starts there.
    obj.vertices[v0].outgoing = he_at_v0; // still in loop_a (chain_a starts at he_at_v0)
    // v1: he_at_v1 is now in loop_b, cut_bwd[0] is in loop_a.
    obj.vertices[v1].outgoing = he_at_v1; // in loop_b (chain_b starts at he_at_v1)

    // 9. Assign hole loops to the two result faces.
    let normal = face_plane.normal();

    // Remove inner loops from the original face (will be re-assigned).
    obj.faces[face].inner_loops.clear();

    for il in &inner_loops {
        let hole_pts: Vec<Point3> = obj.loop_positions(*il).collect();
        let outer_a_pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();

        // Decide by testing the hole RING itself: the cut path is validated
        // to stay clear of every hole, so all of a hole's vertices lie
        // strictly on one side of the cut and inside exactly one result
        // face. A vertex-average "centroid" is NOT a usable proxy — for a
        // concave hole it can fall outside the hole entirely (and inside the
        // other result face), silently assigning the hole to a face whose
        // outer boundary does not contain it; loop-ownership checks alone
        // cannot catch that downstream (rule 4: decide correctly here, never
        // repair later).
        let inside_a = hole_pts
            .iter()
            .filter(|&&hp| point_inside_polygon(hp, &outer_a_pts, normal))
            .count();
        if inside_a == hole_pts.len() {
            // Assign to face A.
            obj.faces[face].inner_loops.push(*il);
            obj.loops[*il].face = face;
        } else if inside_a == 0 {
            // Assign to face B.
            obj.faces[face_b_id].inner_loops.push(*il);
            obj.loops[*il].face = face_b_id;
        } else {
            // A hole straddling both result faces means the cut path crosses
            // hole territory in a way the up-front point/midpoint validation
            // did not see. No valid assignment exists — refuse rather than
            // guess (rule 4).
            return Err(StickyError::PathNotSimple);
        }
    }

    // 10. Update the shell to include the new face.
    // The original face is still in the shell; add face_b_id.
    for shell in obj.shells.values_mut() {
        if shell.faces.contains(&face) {
            shell.faces.push(face_b_id);
            break;
        }
    }

    // 11. Recompute watertightness.
    obj.watertight = if obj.half_edges.values().all(|he| he.twin.is_some()) {
        WatertightState::Watertight
    } else {
        WatertightState::Open
    };

    Ok(FaceSplitReport {
        new_faces: [face, face_b_id],
        new_edges,
        split_boundary_edges,
    })
}

/// Collect the half-edge chain starting at `start` and stopping just before `stop`.
/// The chain includes `start` but not `stop`.
fn collect_chain(obj: &Object, start: HalfEdgeId, stop: HalfEdgeId) -> Vec<HalfEdgeId> {
    let mut chain = Vec::new();
    let mut current = start;
    loop {
        chain.push(current);
        current = obj.half_edges[current].next;
        if current == stop {
            break;
        }
        // Safety guard against infinite loops (shouldn't happen with valid topology).
        if chain.len() > obj.half_edges.len() + 1 {
            panic!("collect_chain: infinite loop detected — broken topology");
        }
    }
    chain
}

/// Wire a sequence of half-edges into a closed loop, setting `next`, `prev`,
/// and `loop_id` for each.  Also sets `first_half_edge` on the loop.
fn wire_loop_sequence(obj: &mut Object, seq: &[HalfEdgeId], loop_id: LoopId, face_id: FaceId) {
    let n = seq.len();
    for i in 0..n {
        let h = seq[i];
        obj.half_edges[h].next = seq[(i + 1) % n];
        obj.half_edges[h].prev = seq[(i + n - 1) % n];
        obj.half_edges[h].loop_id = loop_id;
    }
    obj.loops[loop_id].first_half_edge = seq[0];
    obj.loops[loop_id].face = face_id;
}

/// Split a boundary (or interior) half-edge at a given position, inserting a
/// new vertex.  The original edge is removed; two new edges replace it.  The
/// neighbor face's loop (twin) is also split consistently.
///
/// Returns `(new_vertex, dead_edge_id, [edge_a_id, edge_b_id])`.
///
/// - `edge_a_id` covers `origin → new_vertex`
/// - `edge_b_id` covers `new_vertex → original_dest`
fn split_boundary_edge(
    obj: &mut Object,
    he: HalfEdgeId,
    pos: Point3,
) -> (VertexId, EdgeId, [EdgeId; 2]) {
    let h = obj.half_edges[he];
    let next_he = h.next;
    let prev_he = h.prev;
    let edge_id = h.edge;
    let loop_id = h.loop_id;
    let dest_v = obj.half_edges[next_he].origin;

    // Insert the new vertex.
    let new_v = obj.vertices.insert(Vertex {
        position: pos,
        outgoing: HalfEdgeId::default(), // set below
    });

    // Create two new half-edges for the "forward" side (in `he`'s loop).
    // h_a: origin → new_v
    // h_b: new_v → dest_v
    let h_a = obj.half_edges.insert(HalfEdge {
        origin: h.origin,
        twin: None,
        next: HalfEdgeId::default(),
        prev: prev_he,
        edge: EdgeId::default(),
        loop_id,
    });
    let h_b = obj.half_edges.insert(HalfEdge {
        origin: new_v,
        twin: None,
        next: next_he,
        prev: h_a,
        edge: EdgeId::default(),
        loop_id,
    });
    obj.half_edges[h_a].next = h_b;
    obj.half_edges[prev_he].next = h_a;
    obj.half_edges[next_he].prev = h_b;
    obj.vertices[new_v].outgoing = h_b;

    // Handle the twin side.
    let edge_a_id;
    let edge_b_id;
    // A live half-edge originating at `dest_v` to re-point its `outgoing` to, if
    // the split invalidates the one it currently caches (twin case only — see
    // the outgoing repair below).
    let mut dest_outgoing_fix: Option<HalfEdgeId> = None;

    if let Some(twin_he_id) = h.twin {
        let t = obj.half_edges[twin_he_id];
        let twin_next = t.next;
        let twin_prev = t.prev;
        let twin_loop = t.loop_id;

        // Twin goes in the opposite direction: it goes from dest_v → origin.
        // After split: t_a covers dest_v → new_v, t_b covers new_v → origin.
        let t_a = obj.half_edges.insert(HalfEdge {
            origin: dest_v,
            twin: None,
            next: HalfEdgeId::default(),
            prev: twin_prev,
            edge: EdgeId::default(),
            loop_id: twin_loop,
        });
        let t_b = obj.half_edges.insert(HalfEdge {
            origin: new_v,
            twin: None,
            next: twin_next,
            prev: t_a,
            edge: EdgeId::default(),
            loop_id: twin_loop,
        });
        obj.half_edges[t_a].next = t_b;
        obj.half_edges[twin_prev].next = t_a;
        obj.half_edges[twin_next].prev = t_b;

        // Pair: h_a ↔ t_b, h_b ↔ t_a.
        // h_a goes origin → new_v; its twin should go new_v → origin = t_b.
        // h_b goes new_v → dest_v; its twin should go dest_v → new_v = t_a.
        edge_a_id = obj.edges.insert(Edge {
            half_edge: h_a,
            twin_half_edge: Some(t_b),
            curve: None,
        });
        edge_b_id = obj.edges.insert(Edge {
            half_edge: h_b,
            twin_half_edge: Some(t_a),
            curve: None,
        });
        obj.half_edges[h_a].edge = edge_a_id;
        obj.half_edges[t_b].edge = edge_a_id;
        obj.half_edges[h_a].twin = Some(t_b);
        obj.half_edges[t_b].twin = Some(h_a);

        obj.half_edges[h_b].edge = edge_b_id;
        obj.half_edges[t_a].edge = edge_b_id;
        obj.half_edges[h_b].twin = Some(t_a);
        obj.half_edges[t_a].twin = Some(h_b);

        // `t_a` is the new half-edge originating at `dest_v` (dest_v → new_v);
        // use it to heal `dest_v.outgoing` if it cached the about-to-die twin.
        dest_outgoing_fix = Some(t_a);

        // The twin's loop needs the same first_half_edge repair as `he`'s own
        // loop below: the removed twin may be the neighbor loop's anchor.
        if obj.loops[twin_loop].first_half_edge == twin_he_id {
            obj.loops[twin_loop].first_half_edge = t_a;
        }

        // Remove old twin half-edge.
        obj.half_edges.remove(twin_he_id);
    } else {
        // Boundary edge (no twin). A split at an interior point puts the new
        // vertex INSIDE any circle the edge was a chord of (a chord midpoint
        // is nearer the center than the radius), so the fragments are no
        // longer valid chords — drop the claim (map-or-drop).
        edge_a_id = obj.edges.insert(Edge {
            half_edge: h_a,
            twin_half_edge: None,
            curve: None,
        });
        edge_b_id = obj.edges.insert(Edge {
            half_edge: h_b,
            twin_half_edge: None,
            curve: None,
        });
        obj.half_edges[h_a].edge = edge_a_id;
        obj.half_edges[h_b].edge = edge_b_id;
    }

    // Update the loop's first_half_edge if it pointed at the removed he.
    if obj.loops[loop_id].first_half_edge == he {
        obj.loops[loop_id].first_half_edge = h_a;
    }

    // Remove old half-edge and edge.
    obj.half_edges.remove(he);
    obj.edges.remove(edge_id);

    // Repair endpoint vertices' cached `outgoing`. A vertex's `outgoing` is any
    // one valid half-edge originating there; `he` (and its twin) are gone, so a
    // vertex that cached one of them is now dangling. The new vertex was set
    // above; the two endpoints (`h.origin` and `dest_v`) are the only others
    // that could have referenced the removed half-edges. Re-point them order-
    // independently — `h_a` originates at `h.origin`, `t_a` (twin case) at
    // `dest_v`. This mirrors the merge path's outgoing healing.
    if !obj.half_edges.contains_key(obj.vertices[h.origin].outgoing) {
        obj.vertices[h.origin].outgoing = h_a; // h_a.origin == h.origin
    }
    if let Some(t_a) = dest_outgoing_fix
        && !obj.half_edges.contains_key(obj.vertices[dest_v].outgoing)
    {
        obj.vertices[dest_v].outgoing = t_a; // t_a.origin == dest_v
    }

    (new_v, edge_id, [edge_a_id, edge_b_id])
}

/// Core merge surgery on a cloned object.
///
/// Merges face_a and face_b (both with known coplanar outer loops) by
/// dissolving the entire shared boundary chain between them.
fn do_merge_faces(
    obj: &mut Object,
    face_a: FaceId,
    face_b: FaceId,
) -> Result<FaceMergeReport, StickyError> {
    let outer_a = obj.faces[face_a].outer_loop;
    let outer_b = obj.faces[face_b].outer_loop;
    let face_b_surface = obj.faces[face_b].surface;

    // Snapshot each input face's attribute state, pinned to a point strictly
    // inside it, BEFORE surgery: undoing this merge must give each side back
    // exactly what it carried (paint, UV frame, analytic surface), never a
    // copy of the survivor's. Geometric pinning makes the restoration
    // independent of which fragment the undo-split puts on which handle.
    //
    // BEST EFFORT, never a refusal: a face too thin to pin an interior
    // point in (every ordinate within `tol::POINT_MERGE` of another — a
    // sub-nanometer sliver, constructible only from imported polygon soup)
    // snapshots as `None` and the merge proceeds exactly as it always has;
    // undo then inherits the survivor's attributes for that face only.
    // Refusing here would reject merges the carrier itself accepts.
    let snapshot = |f: FaceId| -> Option<FaceAttrsAt> {
        let face = &obj.faces[f];
        let outer: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
        let holes: Vec<Vec<Point3>> = face
            .inner_loops
            .iter()
            .map(|&il| obj.loop_positions(il).collect())
            .collect();
        let point = interior_point_of_loops(&outer, &holes, face.plane.normal())?;
        Some(FaceAttrsAt {
            point,
            attrs: face.attrs(),
        })
    };
    let prior_attrs = [snapshot(face_a), snapshot(face_b)];

    // 1. Find all half-edges on outer_a whose twin is on outer_b.
    //    These form the shared chain(s).  There must be at least one.
    let hes_a: Vec<HalfEdgeId> = obj.loop_half_edges(outer_a).collect();
    let hes_b_set: std::collections::BTreeSet<HalfEdgeId> = obj.loop_half_edges(outer_b).collect();

    // Find the starting half-edge on outer_a that is in the shared chain,
    // preceded by a non-shared half-edge. This ensures we start at a boundary
    // of the shared chain.
    let shared_on_a: Vec<HalfEdgeId> = hes_a
        .iter()
        .copied()
        .filter(|&h| {
            obj.half_edges[h]
                .twin
                .map(|t| hes_b_set.contains(&t))
                .unwrap_or(false)
        })
        .collect();

    if shared_on_a.is_empty() {
        // No shared edges — the two faces are not adjacent.
        return Err(StickyError::FacesNotCoplanar);
    }

    let mut removed_edges: Vec<EdgeId> = shared_on_a
        .iter()
        .map(|&h| obj.half_edges[h].edge)
        .collect();

    // 2. Find the entry and exit points of the shared chain on outer_a.
    //    "entry" = first shared half-edge on outer_a (whose prev is not shared)
    //    "exit"  = last shared half-edge on outer_a (whose next is not shared)
    //
    //    We may have multiple disconnected shared chains.  The contract says:
    //    "the entire shared chain dissolves."  With two valid adjacent faces,
    //    there's exactly one connected chain of shared edges (two manifold faces
    //    can share a connected boundary, not multiple disconnected segments,
    //    without being non-manifold).  We handle a single chain.

    // Find the start of the chain: a shared half-edge whose prev is NOT shared.
    let shared_set_a: std::collections::BTreeSet<HalfEdgeId> =
        shared_on_a.iter().copied().collect();

    // No chain start means EVERY half-edge of face_a's outer loop is shared
    // with face_b: face_a is a disk filling (part of) face_b's boundary —
    // the sub-face/hole shape, whose dissolution is `merge_inner_face`'s
    // surgery, not this one. Reachable through the boolean seam-dissolution
    // path when a coverage piece shares its whole ring; refuse typed rather
    // than walk a chain that has no endpoints (rule 4).
    let Some(chain_start_a) = shared_on_a.iter().copied().find(|&h| {
        let prev = obj.half_edges[h].prev;
        !shared_set_a.contains(&prev)
    }) else {
        return Err(StickyError::SharedChainCoversBoundary);
    };

    // Walk the shared chain on outer_a (chain_a_seq starts at chain_start_a
    // and continues while the next half-edge is also shared).
    let mut chain_a_seq: Vec<HalfEdgeId> = Vec::new();
    {
        let mut cur = chain_start_a;
        loop {
            chain_a_seq.push(cur);
            let nxt = obj.half_edges[cur].next;
            if shared_set_a.contains(&nxt) {
                cur = nxt;
            } else {
                break;
            }
        }
    }

    // The walk above follows ONE connected run. If it did not cover every
    // shared half-edge, the two faces meet along two or more disconnected
    // chains (a bridge/dogbone adjacency) and the splice below would strand
    // the other chains' half-edges on deleted Edge records. Refuse instead —
    // supporting this needs the merged boundary rebuilt as outer + hole
    // loops, which this op does not do yet.
    if chain_a_seq.len() != shared_on_a.len() {
        return Err(StickyError::SharedChainDisconnected);
    }

    // The corresponding chain on outer_b (in reverse order of chain_a).
    // chain_a[0]'s twin is on outer_b; that twin goes B→A, which is the last
    // segment of the shared portion on B.  We need the chain on B in the
    // order they appear around outer_b.
    //
    // The twin of chain_a_seq[i] is on outer_b and goes in reverse direction.
    // twin of chain_a[0] originates at dest(chain_a[0]) = origin(chain_a[1]) (or v1).
    //
    // chain_b in outer_b order is: twin(chain_a.last()), ..., twin(chain_a[0]).
    let chain_b_seq: Vec<HalfEdgeId> = chain_a_seq
        .iter()
        .rev()
        .map(|&h| obj.half_edges[h].twin.expect("shared edge has twin"))
        .collect();

    // 3. Collect the non-shared portions of outer_a and outer_b.
    //    non_a = chain from next(chain_a.last()) to prev(chain_a[0]) (exclusive).
    //    non_b = chain from next(chain_b.last()) to prev(chain_b[0]) (exclusive).

    // last element of chain_a_seq: the half-edge just before re-entering non-shared territory.
    let chain_a_last = *chain_a_seq.last().unwrap();
    let chain_a_first = chain_a_seq[0];
    let chain_b_last = *chain_b_seq.last().unwrap();
    let chain_b_first = chain_b_seq[0];

    let after_chain_a = obj.half_edges[chain_a_last].next; // first non-shared on A (departs chain_a_start_v)
    let after_chain_b = obj.half_edges[chain_b_last].next; // first non-shared on B (departs chain_b_start_v)

    // The endpoints of the shared chain: two vertices that were inserted by
    // split_boundary_edge if endpoints landed on edge interiors.  They survive
    // the chain dissolve and may need boundary healing (see step 7 below).
    //   chain_a_start_v = origin of chain_a_first (= origin of the chain from face A's side)
    //   chain_b_start_v = origin of chain_b_first (= destination of the chain from face A's side)
    let chain_a_start_v = obj.half_edges[chain_a_first].origin;
    let chain_b_start_v = obj.half_edges[chain_b_first].origin;

    // non_a: from after_chain_a to before_chain_a (inclusive).
    let non_a: Vec<HalfEdgeId> = collect_chain(obj, after_chain_a, chain_a_first);
    // non_b: from after_chain_b to before_chain_b (inclusive).
    let non_b: Vec<HalfEdgeId> = collect_chain(obj, after_chain_b, chain_b_first);

    // 4. Build the merged outer loop: non_a + non_b (all on face_a's loop).
    //    The merged boundary goes:
    //      non_a[0] → ... → non_a.last() → non_b[0] → ... → non_b.last() → non_a[0]
    //
    //    Joining: non_a.last() (arriving at chain_a_start_v) now points to
    //             non_b[0] (after_chain_b, departing from chain_b_start_v).
    //             non_b.last() now points to non_a[0] (after_chain_a).

    let merged_seq: Vec<HalfEdgeId> = non_a.iter().copied().chain(non_b.iter().copied()).collect();

    // Completion check, BEFORE any mutation: the chain walks and non-shared
    // collects are disjoint next-pointer walks, so count equality means they
    // jointly account for every half-edge of both outer loops. Anything
    // unaccounted for would survive the splice still pointing at the removed
    // outer_b loop (seen through the boolean seam-dissolution path on
    // annular operands whose boundary wiring the walks above do not cover).
    // Refuse typed rather than strand a half-edge (rule 4).
    let accounted = chain_a_seq.len() + chain_b_seq.len() + merged_seq.len();
    let outer_total = obj.loop_half_edges(outer_a).count() + obj.loop_half_edges(outer_b).count();
    if accounted != outer_total {
        return Err(StickyError::WouldCorrupt);
    }

    // 5. Wire the merged loop on outer_a.
    wire_loop_sequence(obj, &merged_seq, outer_a, face_a);

    // 6. Find interior vertices of the shared chain (those that are not the
    //    chain endpoints and will be fully orphaned after the chain dissolves).
    let mut candidate_orphans: std::collections::BTreeSet<VertexId> =
        std::collections::BTreeSet::new();
    for &h in &chain_a_seq {
        let v = obj.half_edges[h].origin;
        if v != chain_a_start_v && v != chain_b_start_v {
            candidate_orphans.insert(v);
        }
    }
    for &h in &chain_b_seq {
        let v = obj.half_edges[h].origin;
        if v != chain_a_start_v && v != chain_b_start_v {
            candidate_orphans.insert(v);
        }
    }

    // 7. Remove the shared half-edges and their edges.
    for &h in chain_a_seq.iter().chain(chain_b_seq.iter()) {
        obj.half_edges.remove(h);
    }
    for &e in &removed_edges {
        obj.edges.remove(e);
    }

    // 8. Remove actually-orphaned interior chain vertices.
    for v in &candidate_orphans {
        let still_in_use = obj.half_edges.values().any(|he| he.origin == *v);
        if !still_in_use {
            obj.vertices.remove(*v);
        }
    }

    // 9. Boundary healing: each chain-endpoint vertex now has exactly two
    //    incident edges in the merged boundary and in its neighbor face(s).
    //    If those two edges are collinear (their directions have a cross-product
    //    length below tol::NORMAL_DIRECTION), the vertex is a scar from a
    //    split_boundary_edge call and must be dissolved.
    //
    // The endpoint VERTICES are captured up front and each one's live
    // incoming/outgoing half-edges are re-derived at heal time, never taken
    // from pre-heal ids: healing the first endpoint can consume the
    // half-edges the second endpoint's stale pair referenced (a triangular
    // neighbor's single non-shared half-edge, for example), and skipping the
    // second endpoint in that case made the heal outcome depend on which
    // side happened to be `face_a` — the merge edge's primary-half-edge
    // orientation, i.e. internal representation state. Identical geometric
    // input must heal identically (the determinism invariant; History replay
    // verifies redo results against the recorded state and fails typed on
    // any asymmetry).
    let endpoint_vertices = [chain_a_start_v, chain_b_start_v];
    let mut healing_removed_edges: Vec<EdgeId> = Vec::new();
    for vertex_to_heal in endpoint_vertices {
        if !obj.vertices.contains_key(vertex_to_heal) {
            continue; // healed away together with the other endpoint
        }

        // Heal only true scars: V must have exactly two departing
        // half-edges (one in the merged loop, one in the neighbor loop).
        // Any extra incidence means V is a real model vertex shared with
        // other geometry — removing it would strand those half-edges.
        let departing: Vec<HalfEdgeId> = obj
            .half_edges
            .iter()
            .filter(|(_, he)| he.origin == vertex_to_heal)
            .map(|(h, _)| h)
            .collect();
        if departing.len() != 2 {
            continue;
        }

        // h_out departs V along the merged outer loop; h_in arrives at V.
        // Every reference below is re-derived from live wiring and the heal
        // is SKIPPED if any of it is missing: an earlier heal (or, in the
        // boolean seam-dissolution path, an earlier merge over a face this
        // loop wiring still references) can leave V's neighborhood without
        // the clean two-edge scar pattern this surgery reverses. Skipping
        // leaves a legal (merely unhealed) scar vertex; the committed merge
        // still passes the full validator either way (rule 4: skip, never
        // guess).
        let Some(&h_out) = departing
            .iter()
            .find(|&&h| obj.half_edges[h].loop_id == outer_a)
        else {
            continue; // endpoint no longer on the merged boundary
        };
        let h_in = obj.half_edges[h_out].prev;
        let (Some(h_in_he), Some(h_out_next)) = (
            obj.half_edges.get(h_in),
            obj.half_edges.get(obj.half_edges[h_out].next),
        ) else {
            continue; // wiring at V no longer forms the scar pattern
        };

        let v_pos = obj.vertices[vertex_to_heal].position;
        let prev_v_pos = obj.vertices[h_in_he.origin].position;
        let next_v_pos = obj.vertices[h_out_next.origin].position;

        let dir_in = (v_pos - prev_v_pos).normalized();
        let dir_out = (next_v_pos - v_pos).normalized();

        let collinear = match (dir_in, dir_out) {
            (Ok(a), Ok(b)) => a.cross(b).length() < tol::NORMAL_DIRECTION,
            _ => false, // degenerate edge — skip healing
        };
        if !collinear {
            continue;
        }

        // Twin of h_in: this half-edge departs V in the neighbor face (t_out_of_v).
        // Twin of h_out: this half-edge arrives at V in the neighbor face (t_into_v).
        let (Some(t_out_of_v), Some(t_into_v)) = (h_in_he.twin, obj.half_edges[h_out].twin) else {
            continue; // boundary edge at V — not a closed scar pattern
        };
        if !obj.half_edges.contains_key(t_out_of_v) || !obj.half_edges.contains_key(t_into_v) {
            continue; // neighbor side already consumed
        }
        // The neighbor side must be ONE loop passing V exactly once —
        // t_into_v arrives at V and t_out_of_v departs it, consecutively.
        // Anything else (the two twins living in different loops, e.g. when
        // V is also touched by a hole ring or a third coplanar face in the
        // boolean seam-dissolution path) is not the two-edge scar pattern
        // this surgery reverses; splicing across two distinct loops would
        // corrupt both. Skip — an unhealed scar vertex is legal geometry.
        if obj.half_edges[t_into_v].next != t_out_of_v {
            continue;
        }
        let t_next = obj.half_edges[t_out_of_v].next; // half-edge after t_out_of_v in neighbor loop
        let neighbor_loop = obj.half_edges[t_out_of_v].loop_id;

        // In the merged loop: skip over V by connecting h_in → next(h_out).
        let h_next = obj.half_edges[h_out].next;
        obj.half_edges[h_in].next = h_next;
        obj.half_edges[h_next].prev = h_in;
        // Fix loop first_half_edge if it pointed at the removed h_out.
        if obj.loops[outer_a].first_half_edge == h_out {
            obj.loops[outer_a].first_half_edge = h_in;
        }

        // In the neighbor loop: skip over V by connecting t_into_v → next(t_out_of_v).
        obj.half_edges[t_into_v].next = t_next;
        obj.half_edges[t_next].prev = t_into_v;
        // Fix loop first_half_edge if it pointed at the removed t_out_of_v.
        if obj.loops[neighbor_loop].first_half_edge == t_out_of_v {
            obj.loops[neighbor_loop].first_half_edge = t_into_v;
        }

        // Repair the Edge: keep the Edge for h_in with (h_in, t_into_v) as
        // its half-edge pair. h_in may have been the TWIN side of its edge
        // (the primary being t_out_of_v, which dies below), so the primary
        // must be reset explicitly, not assumed. Retire the Edge for h_out.
        let edge_to_keep = obj.half_edges[h_in].edge;
        let edge_to_remove = obj.half_edges[h_out].edge;
        obj.edges[edge_to_keep].half_edge = h_in;
        obj.edges[edge_to_keep].twin_half_edge = Some(t_into_v);
        obj.half_edges[h_in].twin = Some(t_into_v);
        obj.half_edges[t_into_v].twin = Some(h_in);
        obj.half_edges[h_in].edge = edge_to_keep;
        obj.half_edges[t_into_v].edge = edge_to_keep;
        obj.edges.remove(edge_to_remove);
        healing_removed_edges.push(edge_to_remove);

        // Fix vertex outgoing pointer (if it pointed at the now-dead h_out or t_out_of_v).
        // Vertices prev_v and next_v retain valid half-edges; just ensure outgoing is live.
        let prev_v = obj.half_edges[h_in].origin;
        let next_v_id = obj.half_edges[h_next].origin;
        if !obj.half_edges.contains_key(obj.vertices[prev_v].outgoing) {
            obj.vertices[prev_v].outgoing = h_in;
        }
        if !obj
            .half_edges
            .contains_key(obj.vertices[next_v_id].outgoing)
        {
            obj.vertices[next_v_id].outgoing = h_next;
        }

        // Remove the healed-away elements.
        obj.half_edges.remove(h_out);
        obj.half_edges.remove(t_out_of_v);
        obj.vertices.remove(vertex_to_heal);
    }

    // 10. Fix any remaining broken outgoing pointers (shared-chain endpoints
    //     that are still present but whose outgoing pointed into the dissolved chain).
    let outgoing_map: std::collections::BTreeMap<VertexId, HalfEdgeId> = obj
        .half_edges
        .iter()
        .map(|(hid, he)| (he.origin, hid))
        .collect();
    for (_vid, vertex) in obj.vertices.iter_mut() {
        if !obj.half_edges.contains_key(vertex.outgoing)
            && let Some(&new_out) = outgoing_map.get(&_vid)
        {
            vertex.outgoing = new_out;
        }
    }

    // 11. Combine inner loops from face_b into face_a.
    let inner_b: Vec<LoopId> = obj.faces[face_b].inner_loops.clone();
    for il in inner_b {
        obj.loops[il].face = face_a;
        obj.faces[face_a].inner_loops.push(il);
    }

    // 12. Remove face_b and its outer loop.
    obj.loops.remove(outer_b);
    obj.faces.remove(face_b);
    for shell in obj.shells.values_mut() {
        shell.faces.retain(|&f| f != face_b);
    }

    // 13. Refit plane for the merged face over the merged boundary. The
    //     walk is CHECKED, not assumed: a shared-chain configuration whose
    //     surgery left the merged loop wired through removed half-edges
    //     (seen in the boolean seam-dissolution path when a chain endpoint
    //     also touches a hole ring or a third coplanar face) must surface
    //     as the typed WouldCorrupt refusal — the caller discards the
    //     clone — rather than a panic on a dead key before validation can
    //     run (rule 4: refuse loudly, never crash).
    let boundary_pts: Vec<Point3> = {
        let Some(lp) = obj.loops.get(outer_a) else {
            return Err(StickyError::WouldCorrupt);
        };
        let first = lp.first_half_edge;
        let mut pts = Vec::new();
        let mut cur = first;
        let mut closed = false;
        for _ in 0..=obj.half_edges.len() {
            let Some(he) = obj.half_edges.get(cur) else {
                return Err(StickyError::WouldCorrupt);
            };
            let Some(v) = obj.vertices.get(he.origin) else {
                return Err(StickyError::WouldCorrupt);
            };
            pts.push(v.position);
            cur = he.next;
            if cur == first {
                closed = true;
                break;
            }
        }
        if !closed {
            return Err(StickyError::WouldCorrupt);
        }
        pts
    };
    if let Ok(new_plane) = Plane::from_polygon(&boundary_pts) {
        obj.faces[face_a].plane = new_plane;
    }
    // The survivor keeps its analytic surface only when BOTH faces claimed
    // the same one (map-or-drop: absorbing area from a face on a different —
    // or no — cylinder would make the claim a lie). `face_b_surface` was
    // captured before face_b died.
    if obj.faces[face_a].surface != face_b_surface {
        obj.faces[face_a].surface = None;
    }
    // (If refit fails, keep the original plane — shouldn't happen with valid geometry.)

    // 14. Recompute watertightness.
    obj.watertight = if obj.half_edges.values().all(|he| he.twin.is_some()) {
        WatertightState::Watertight
    } else {
        WatertightState::Open
    };

    removed_edges.extend(healing_removed_edges);
    Ok(FaceMergeReport {
        merged_face: face_a,
        removed_edges,
        prior_attrs,
    })
}

// ================================================================= unit tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::topo::WatertightState;

    // ---------------------------------------------------------------- helpers

    /// Build a unit cube (1×1×1) as a watertight Object.
    fn unit_cube() -> Object {
        Object::from_polygons(
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

    /// Return the FaceId whose plane normal matches `dir`.
    fn face_with_normal(obj: &Object, dir: Vec3) -> FaceId {
        obj.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION))
            .map(|(id, _)| id)
            .expect("face with that normal must exist")
    }

    // ------------------------------- interpenetration guard primitive

    /// The plus-sign coplanar overlap: two coplanar strips crossing
    /// off-center, sharing real area but NO vertex of either inside the
    /// other and no boundary endpoint or midpoint landing inside either —
    /// point sampling misses it entirely; only real segment/segment
    /// intersection of the boundaries sees the contact. This is exactly the
    /// footprint a built wall or a rigidly translated moved face can produce
    /// when it lands coplanar with, and crossing, a stationary face.
    #[test]
    fn faces_improperly_contact_detects_plus_sign_coplanar_overlap() {
        // Two disjoint coplanar quads at z = 0 (an Open two-face object is
        // enough — the guard primitive reads geometry, not watertightness):
        // a long thin strip along x and a tall thin strip along y, crossing
        // over x ∈ [3,4] × y ∈ [-0.5, 0.5].
        let obj = Object::from_polygons(
            &[
                Point3::new(-10.0, -0.5, 0.0),
                Point3::new(10.0, -0.5, 0.0),
                Point3::new(10.0, 0.5, 0.0),
                Point3::new(-10.0, 0.5, 0.0),
                Point3::new(3.0, -5.0, 0.0),
                Point3::new(4.0, -5.0, 0.0),
                Point3::new(4.0, 8.0, 0.0),
                Point3::new(3.0, 8.0, 0.0),
            ],
            &[vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
        )
        .expect("two coplanar strips");
        let faces: Vec<FaceId> = obj.faces().keys().collect();
        assert!(
            faces_improperly_contact(&obj, faces[0], faces[1]),
            "plus-sign coplanar overlap must register as improper contact"
        );
        assert!(
            faces_improperly_contact(&obj, faces[1], faces[0]),
            "the test must be symmetric in its arguments"
        );
    }

    /// The diagonal-chord overlap: a wall standing on a floor's diagonal,
    /// sharing both diagonal endpoints as true vertices. Every boundary
    /// contact between the two faces is a whitelisted shared corner, and
    /// the wall's other edges are strictly off the floor's plane — yet the
    /// chord's interior slices straight across the floor. Crossing
    /// detection alone cannot see it (every intersection point is
    /// whitelisted); only probing between the cut points does. The dual of
    /// the plus-sign case: together they show why the coplanar test needs
    /// both parts.
    #[test]
    fn faces_improperly_contact_detects_diagonal_chord_between_shared_corners() {
        let obj = Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(1.0, 1.0, 1.0),
                Point3::new(0.0, 0.0, 1.0),
            ],
            &[vec![0, 1, 2, 3], vec![0, 2, 4, 5]],
        )
        .expect("floor plus a wall on its diagonal");
        let faces: Vec<FaceId> = obj.faces().keys().collect();
        assert!(
            faces_improperly_contact(&obj, faces[0], faces[1]),
            "a chord between two shared corners slicing across the face \
             must register as improper contact"
        );
        assert!(
            faces_improperly_contact(&obj, faces[1], faces[0]),
            "the test must be symmetric in its arguments"
        );
    }

    /// Control for the diagonal-chord case: the same two-shared-corner
    /// shape on an L-shaped floor, with the chord crossing the notch —
    /// strictly OUTSIDE the floor's material. Shared corners plus an
    /// exterior chord is legitimate, not contact.
    #[test]
    fn faces_improperly_contact_ignores_chord_outside_the_face() {
        let obj = Object::from_polygons(
            &[
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(2.0, 0.0, 0.0),
                Point3::new(2.0, 1.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(1.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
                Point3::new(2.0, 1.0, 1.0),
                Point3::new(1.0, 2.0, 1.0),
            ],
            &[vec![0, 1, 2, 3, 4, 5], vec![2, 4, 7, 6]],
        )
        .expect("L-shaped floor plus a wall bridging its notch");
        let faces: Vec<FaceId> = obj.faces().keys().collect();
        assert!(!faces_improperly_contact(&obj, faces[0], faces[1]));
        assert!(!faces_improperly_contact(&obj, faces[1], faces[0]));
    }

    /// Control for the plus-sign case: the same two strips, pulled apart so
    /// they no longer overlap, are NOT in contact.
    #[test]
    fn faces_improperly_contact_ignores_disjoint_coplanar_strips() {
        let obj = Object::from_polygons(
            &[
                Point3::new(-10.0, -0.5, 0.0),
                Point3::new(10.0, -0.5, 0.0),
                Point3::new(10.0, 0.5, 0.0),
                Point3::new(-10.0, 0.5, 0.0),
                Point3::new(3.0, 2.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(4.0, 8.0, 0.0),
                Point3::new(3.0, 8.0, 0.0),
            ],
            &[vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
        )
        .expect("two coplanar strips");
        let faces: Vec<FaceId> = obj.faces().keys().collect();
        assert!(!faces_improperly_contact(&obj, faces[0], faces[1]));
        assert!(!faces_improperly_contact(&obj, faces[1], faces[0]));
    }

    // ------------------------------------------------------ material

    /// Splitting a painted face propagates its material to *both* halves: face A
    /// keeps it by reusing the original FaceId, face B inherits it explicitly.
    #[test]
    fn split_face_propagates_material_to_both_halves() {
        use crate::ids::MaterialId;
        use slotmap::SlotMap;
        // A throwaway palette just to mint a valid MaterialId value to compare.
        let mut palette: SlotMap<MaterialId, ()> = SlotMap::with_key();
        let mat = palette.insert(());

        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        cube.faces[top].material = Some(mat);

        let path = vec![Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
        let report = cube.split_face(top, &path).expect("split the painted top");
        cube.validate().expect("topology valid after split");

        for fid in report.new_faces {
            assert_eq!(
                cube.faces()[fid].material,
                Some(mat),
                "both split halves inherit the painted material"
            );
        }
    }

    /// An imprinted interior sub-face inherits the parent face's material.
    #[test]
    fn split_face_inner_sub_face_inherits_material() {
        use crate::ids::MaterialId;
        use slotmap::SlotMap;
        let mut palette: SlotMap<MaterialId, ()> = SlotMap::with_key();
        let mat = palette.insert(());

        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        cube.faces[top].material = Some(mat);

        // A small rectangle strictly inside the top face becomes a coplanar
        // sub-face (additive imprint).
        let loop_path = vec![
            Point3::new(0.25, 0.25, 1.0),
            Point3::new(0.75, 0.25, 1.0),
            Point3::new(0.75, 0.75, 1.0),
            Point3::new(0.25, 0.75, 1.0),
        ];
        let report = cube
            .split_face_inner(top, &loop_path)
            .expect("imprint interior sub-face");
        cube.validate().expect("topology valid after imprint");

        assert_eq!(
            cube.faces()[report.sub_face].material,
            Some(mat),
            "imprinted sub-face inherits the parent face's material"
        );
    }

    // -------------------------------------------------------- classify_endpoint

    /// classify_endpoint should snap to a vertex when the point coincides with
    /// a loop vertex (within POINT_MERGE).
    #[test]
    fn classify_endpoint_hits_vertex() {
        let cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let outer_loop = cube.faces()[top].outer_loop;
        let outer_hes: Vec<HalfEdgeId> = cube.loop_half_edges(outer_loop).collect();

        // (0,0,1) is an exact corner of the top face.
        let corner = Point3::new(0.0, 0.0, 1.0);
        let hit = classify_endpoint(&cube, &outer_hes, corner)
            .unwrap()
            .expect("corner must be on the boundary");
        assert!(
            matches!(hit, EndpointHit::Vertex(_)),
            "exact corner must resolve to Vertex, got {hit:?}"
        );
    }

    /// classify_endpoint should resolve to an Edge hit when the point lies in
    /// the strict interior of a boundary edge.
    #[test]
    fn classify_endpoint_hits_edge_interior() {
        let cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let outer_loop = cube.faces()[top].outer_loop;
        let outer_hes: Vec<HalfEdgeId> = cube.loop_half_edges(outer_loop).collect();

        // (0.5, 0.0, 1.0) is the midpoint of one boundary edge of the top face.
        let midpoint = Point3::new(0.5, 0.0, 1.0);
        let hit = classify_endpoint(&cube, &outer_hes, midpoint)
            .unwrap()
            .expect("midpoint must be on the boundary");
        assert!(
            matches!(hit, EndpointHit::Edge { .. }),
            "edge midpoint must resolve to Edge hit, got {hit:?}"
        );
    }

    /// classify_endpoint returns None (not an error) for a point strictly
    /// interior to the face.
    #[test]
    fn classify_endpoint_interior_point_returns_none() {
        let cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let outer_loop = cube.faces()[top].outer_loop;
        let outer_hes: Vec<HalfEdgeId> = cube.loop_half_edges(outer_loop).collect();

        // (0.5, 0.5, 1.0) is the centroid — strictly interior.
        let interior = Point3::new(0.5, 0.5, 1.0);
        let result = classify_endpoint(&cube, &outer_hes, interior).unwrap();
        assert!(
            result.is_none(),
            "interior point must return None, got {result:?}"
        );
    }

    // ------------------------------------------------- multi-segment path split

    /// A path with an interior waypoint (3 points, 2 segments) should produce a
    /// split with 2 new edges and a valid watertight result.
    #[test]
    fn split_face_multi_segment_path_creates_two_new_edges() {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

        // Path: bottom-edge midpoint → interior waypoint → top-edge midpoint.
        let path = [
            Point3::new(0.5, 0.0, 1.0), // midpoint of bottom edge (y=0)
            Point3::new(0.5, 0.5, 1.0), // interior waypoint
            Point3::new(0.5, 1.0, 1.0), // midpoint of top edge (y=1)
        ];
        let report = cube.split_face(top, &path).unwrap();

        assert_eq!(
            report.new_edges.len(),
            2,
            "two path segments → two new cut edges"
        );
        cube.validate().unwrap();
        assert_eq!(cube.watertight(), WatertightState::Watertight);
        // Two original boundary edges were split (one per endpoint).
        assert_eq!(
            report.split_boundary_edges.len(),
            2,
            "both endpoints land on edge interiors → two boundary splits"
        );
    }

    /// Regression (DESIGN risk #1): splitting a boundary edge must not leave a
    /// dangling `vertex.outgoing`.
    ///
    /// A vertex's `outgoing` is an arbitrary-but-valid half-edge originating at
    /// it. On an extruded box a top-cap corner's `outgoing` may point at the very
    /// top-cap boundary half-edge that a split removes; the builder once assigned
    /// it via HashMap iteration, so the validator flaked (~3/5) on the per-process
    /// seed (the determinism class now banned by  — the builder iterates the
    /// half-edge slotmap and `directed` is a `BTreeMap`).
    /// Here we *force* that adversarial assignment so the case is deterministic:
    /// every top-loop origin's `outgoing` points into the top loop, then we cut
    /// the top face boundary-to-boundary. `split_boundary_edge` must re-point any
    /// endpoint vertex whose `outgoing` it invalidates.
    #[test]
    fn split_face_repoints_dangling_outgoing_on_boundary_split() {
        let mut box_obj = Object::from_extrusion(&rect_profile(1.0, 1.0), 1.0).unwrap();
        let top = face_with_normal(&box_obj, Vec3::new(0.0, 0.0, 1.0));

        // Adversarial: point every top-loop vertex's `outgoing` at its top-loop
        // half-edge (the ones whose twins live on the walls). The two endpoints
        // of the cut below land on top-loop edges, so their origin vertices'
        // `outgoing` will reference half-edges that the split removes.
        let top_loop = box_obj.faces[top].outer_loop;
        let top_hes: Vec<HalfEdgeId> = box_obj.loop_half_edges(top_loop).collect();
        for h in top_hes {
            let origin = box_obj.half_edges[h].origin;
            box_obj.vertices[origin].outgoing = h;
        }

        // Cut from one top edge midpoint to the opposite — both endpoints are
        // edge-interior hits, so both trigger split_boundary_edge.
        let path = [Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
        box_obj
            .split_face(top, &path)
            .expect("boundary-to-boundary split on extruded box");

        // The internal validator (run by split_face) already guards this, but
        // assert explicitly: no vertex points at a removed half-edge.
        box_obj
            .validate()
            .expect("no dangling outgoing after split");
        assert_eq!(box_obj.watertight(), WatertightState::Watertight);
    }

    /// rect_profile mirror of the test helper in `tests/op_specs.rs`.
    fn rect_profile(width: f64, height: f64) -> Profile {
        let plane = crate::math::Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        Profile::new(
            plane,
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(width, 0.0, 0.0),
                Point3::new(width, height, 0.0),
                Point3::new(0.0, height, 0.0),
            ],
            vec![],
        )
        .unwrap()
    }

    /// A multi-segment split followed by merge restores the original topology.
    #[test]
    fn split_face_multi_segment_then_merge_is_identity() {
        let original = unit_cube();
        let mut cube = original.clone();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

        let path = [
            Point3::new(0.5, 0.0, 1.0),
            Point3::new(0.5, 0.5, 1.0),
            Point3::new(0.5, 1.0, 1.0),
        ];
        let split = cube.split_face(top, &path).unwrap();
        cube.validate().unwrap();

        // Merge using the first cut edge.
        cube.merge_faces(split.new_edges[0]).unwrap();
        cube.validate().unwrap();

        // After heal, the cube must be topologically equivalent to original.
        // Verify vertex count and face count match — objects_equivalent is
        // defined in op_specs so we check the proxy invariants here.
        assert_eq!(
            cube.vertices().len(),
            original.vertices().len(),
            "healed merge must remove split-point scars"
        );
        assert_eq!(cube.faces().len(), original.faces().len());
        assert_eq!(cube.edges().len(), original.edges().len());
        assert_eq!(cube.watertight(), WatertightState::Watertight);
    }

    // -------------------------------------- neighbor-face integrity after split

    /// After split_face where both endpoints land on boundary-edge interiors,
    /// the two neighbor faces that share those boundary edges must remain
    /// valid: their loops must close, and their planes must still contain all
    /// their vertices.
    #[test]
    fn neighbor_faces_valid_after_boundary_edge_split() {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

        // Endpoint 0 lies on the boundary edge shared with the front face (y=0).
        // Endpoint 1 lies on the boundary edge shared with the back face (y=1).
        let path = [Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
        cube.split_face(top, &path).unwrap();

        // The full topology validator checks that all loops close,
        // every half-edge has a valid twin (watertight), and planes are
        // consistent (via the debug invariant check already called in split_face).
        cube.validate().unwrap();

        // Extra check: all 8 faces must have vertices that lie on their plane.
        for (_, face) in cube.faces().iter() {
            let plane = face.plane;
            for pt in cube.loop_positions(face.outer_loop) {
                assert!(
                    plane.signed_distance(pt).abs() <= tol::PLANE_DIST * 1000.0,
                    "vertex {pt:?} is not on face plane {plane:?}"
                );
            }
        }

        // The front face (normal -y) must now have 5 vertices (V0 inserted).
        let front = face_with_normal(&cube, Vec3::new(0.0, -1.0, 0.0));
        let front_verts: Vec<_> = cube
            .loop_positions(cube.faces()[front].outer_loop)
            .collect();
        assert_eq!(
            front_verts.len(),
            5,
            "front face gains the split vertex; expected 5-gon, got {front_verts:?}"
        );

        // The back face (normal +y) must also have 5 vertices.
        let back = face_with_normal(&cube, Vec3::new(0.0, 1.0, 0.0));
        let back_verts: Vec<_> = cube.loop_positions(cube.faces()[back].outer_loop).collect();
        assert_eq!(
            back_verts.len(),
            5,
            "back face gains the split vertex; expected 5-gon, got {back_verts:?}"
        );
    }

    /// Endpoints that land exactly on existing vertices (vertex-to-vertex cut)
    /// must NOT insert new boundary-edge splits.
    #[test]
    fn vertex_to_vertex_cut_has_no_boundary_splits() {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

        // Both endpoints are exact corners of the top face.
        let corners: Vec<Point3> = cube.loop_positions(cube.faces()[top].outer_loop).collect();
        // corners[0] and corners[2] are opposite corners of the top square.
        let path = [corners[0], corners[2]];
        let report = cube.split_face(top, &path).unwrap();

        assert_eq!(
            report.split_boundary_edges.len(),
            0,
            "vertex-to-vertex cut must not split any boundary edges"
        );
        assert_eq!(report.new_edges.len(), 1, "one segment → one new edge");
        cube.validate().unwrap();
        assert_eq!(cube.watertight(), WatertightState::Watertight);
    }

    /// split_face must reject a path where an endpoint is strictly interior
    /// to the face (not on any boundary).
    #[test]
    fn split_face_endpoint_off_boundary_is_rejected() {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

        let path = [
            Point3::new(0.5, 0.0, 1.0), // on boundary
            Point3::new(0.5, 0.5, 1.0), // interior — not on boundary
        ];
        let err = cube.split_face(top, &path).unwrap_err();
        assert_eq!(err, StickyError::EndpointNotOnBoundary { which: 1 });
        // Strong guarantee: cube is unchanged.
        cube.validate().unwrap();
    }
}
