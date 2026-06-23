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
    point_inside_polygon, polygon_is_simple, segments_intersect, signed_area_on_plane,
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

/// What `merge_faces` changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceMergeReport {
    /// The single face replacing the two inputs (both input handles die).
    pub merged_face: FaceId,
    /// Now-dead handles of the dissolved shared-boundary edges.
    pub removed_edges: Vec<EdgeId>,
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
    /// The swept side walls would intersect other geometry of this Object in
    /// a way that has no manifold result (e.g., sweeping past a concave
    /// neighbor). Fail loudly; never produce near-correct geometry.
    ///
    /// Also returned in M1 for sweeps the current implementation cannot
    /// regenerate (side faces not perpendicular to the moved face); wall
    /// generation for the general case lands with the boolean machinery.
    NonManifoldResult,
    /// `extrude_sub_face`/`collapse_sub_face` on a face that is not the expected
    /// kind (a flat imprinted sub-face, or a raised one with generated walls).
    NotASubFace,
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
    /// `split_face_inner`: a loop vertex is not strictly inside the face (it lies
    /// on/outside the outer boundary or inside a hole). v1 imprints only loops
    /// strictly interior to the face.
    LoopNotStrictlyInside {
        /// Index of the offending loop vertex.
        index: usize,
    },
    /// `split_face_inner`: the loop crosses itself.
    LoopSelfIntersects,
    /// `merge_inner_face`: the face is not an imprinted sub-face (its boundary is
    /// not a single closed loop twinned entirely with one parent's hole loop).
    NotAnInnerFace,
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
            StickyError::LoopNotStrictlyInside { index } => {
                write!(f, "loop vertex {index} is not strictly inside the face")
            }
            StickyError::LoopSelfIntersects => write!(f, "loop crosses itself"),
            StickyError::NotAnInnerFace => {
                write!(f, "face is not an imprinted sub-face")
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
        for f in self.faces.values_mut() {
            // Non-singular by the check above, so apply_plane cannot fail.
            f.plane = transform
                .apply_plane(&f.plane)
                .expect("apply_plane on a validated non-singular transform");
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

        // We'll collect (outer_loop_indices, inner_loop_index_lists, plane)
        // tuples for from_faces_with_holes.
        let mut face_specs: Vec<(Vec<usize>, Vec<Vec<usize>>, Plane)> = Vec::new();

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

        face_specs.push((near_outer_reversed, near_inner_loops, near_cap_plane));

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

        face_specs.push((far_outer, far_inner_loops, far_cap_plane));

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
            face_specs.push((vec![a_near, b_near, b_far, a_far], vec![], wall_plane));
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
                face_specs.push((vec![a_near, b_near, b_far, a_far], vec![], wall_plane));
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
                .map(|(outer_idx, inner_lists, _plane)| {
                    let rev_outer: Vec<usize> = outer_idx.into_iter().rev().collect();
                    let rev_inners: Vec<Vec<usize>> = inner_lists
                        .into_iter()
                        .map(|l| l.into_iter().rev().collect())
                        .collect();
                    let pts: Vec<Point3> = rev_outer.iter().map(|&i| positions[i]).collect();
                    let plane =
                        Plane::from_polygon(&pts).map_err(|_| ExtrudeError::DegenerateGeometry)?;
                    Ok((rev_outer, rev_inners, plane))
                })
                .collect::<Result<Vec<_>, ExtrudeError>>()?;
        }

        // Freshly extruded faces take the default material and no UV frame.
        let face_specs: Vec<_> = face_specs
            .into_iter()
            .map(|(outer, inners, plane)| (outer, inners, plane, None, None))
            .collect();
        let obj = Object::from_faces_with_holes(&positions, &face_specs);
        // A valid Profile should always yield a valid solid; if the sweep
        // nonetheless produced invalid topology, return a typed error rather
        // than tripping the debug-only validator panic at the WASM boundary.
        obj.validate()
            .map_err(|_| ExtrudeError::DegenerateGeometry)?;
        Ok(obj)
    }

    /// SketchUp's signature move: translates `face` by `distance` along its
    /// outward normal (positive = outward, adding material; negative =
    /// inward, removing it), regenerating the side walls.
    ///
    /// Behavioral contract:
    /// - Side walls appear along boundary edges whose neighbors don't move;
    ///   where a new wall lands coplanar with an existing neighbor face
    ///   (within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)/[`tol::NORMAL_DIRECTION`](crate::tol::NORMAL_DIRECTION)), they merge —
    ///   pulling a box's top up yields a taller box with 6 faces, not 10.
    /// - Pushing inward until the moved face lands on the opposite face's
    ///   plane dissolves both ("push through"): material is subtracted, and
    ///   the object may split into multiple shells (still one Object).
    /// - The inverse property: `push_pull(f, d)` then `push_pull(f', -d)` on
    ///   the returned face restores the original topology and geometry.
    ///
    /// # Errors
    /// See [`PushPullError`]; all leave the object untouched.
    pub fn push_pull(
        &mut self,
        face: FaceId,
        distance: f64,
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

        let face_normal = self.faces[face].plane.normal();
        let sweep = face_normal * distance;

        // Collect all vertex IDs on the moved face (outer + inner loops).
        let moved_vertices: std::collections::HashSet<VertexId> = {
            let outer_loop = self.faces[face].outer_loop;
            let inner_loops: Vec<LoopId> = self.faces[face].inner_loops.clone();
            let mut verts = std::collections::HashSet::new();
            for h in self.loop_half_edges(outer_loop) {
                verts.insert(self.half_edges[h].origin);
            }
            for il in &inner_loops {
                for h in self.loop_half_edges(*il) {
                    verts.insert(self.half_edges[h].origin);
                }
            }
            verts
        };

        // Collect neighbor faces: faces that share a boundary edge with the moved face.
        // A boundary edge of the moved face is one where one half-edge is on the moved face
        // and the twin is on a different face.
        let neighbor_faces: Vec<FaceId> = {
            let outer_loop = self.faces[face].outer_loop;
            let inner_loops: Vec<LoopId> = self.faces[face].inner_loops.clone();
            let mut neighbors = Vec::new();
            let mut seen = std::collections::HashSet::new();

            let all_loops: Vec<LoopId> = std::iter::once(outer_loop)
                .chain(inner_loops.iter().copied())
                .collect();

            for loop_id in all_loops {
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
        // For each neighbor face, the sweep direction (face normal) must lie in
        // the neighbor's plane: |normal_moved · normal_neighbor| < tol::NORMAL_DIRECTION.
        for &nf in &neighbor_faces {
            let normal_neighbor = self.faces[nf].plane.normal();
            let dot = face_normal.dot(normal_neighbor).abs();
            if dot >= tol::NORMAL_DIRECTION {
                return Err(PushPullError::NonManifoldResult);
            }
        }

        // --- Step 3: WouldVanish check (inward push only) ---
        if distance < 0.0 {
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

            // Interior-obstruction guard: translate mode moves the shared
            // ring, so a neighbor face whose FIXED vertices sit closer along
            // the sweep than the push depth would fold past them into a
            // self-intersecting shell — every face stays planar and manifold,
            // so the validator cannot see it. Refuse at the nearest fixed
            // neighbor vertex strictly in front of the sweep.
            let mut neighbor_limit = f64::INFINITY;
            for &nf in &neighbor_faces {
                let loops: Vec<LoopId> = std::iter::once(self.faces[nf].outer_loop)
                    .chain(self.faces[nf].inner_loops.iter().copied())
                    .collect();
                for loop_id in loops {
                    for h in self.loop_half_edges(loop_id) {
                        let vid = self.half_edges[h].origin;
                        if moved_vertices.contains(&vid) {
                            continue;
                        }
                        let inward = -moved_face_plane.signed_distance(self.vertices[vid].position);
                        if inward > tol::POINT_MERGE && inward < neighbor_limit {
                            neighbor_limit = inward;
                        }
                    }
                }
            }
            if (-distance) >= neighbor_limit - tol::POINT_MERGE {
                return Err(PushPullError::NonManifoldResult);
            }
        }

        // --- Steps 4-6: Clone, mutate, validate, swap ---
        let mut obj = self.clone();

        // Step 4: Translate vertices of the moved face's boundary.
        for &vid in &moved_vertices {
            obj.vertices[vid].position = obj.vertices[vid].position + sweep;
        }

        // Step 5: Refit planes for the moved face and every neighbor face.
        // Moved face:
        {
            let outer_loop = obj.faces[face].outer_loop;
            let pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();
            // A refit failure after the guards above is a degenerate sweep
            // we failed to predict — refuse loudly, never skip the refit.
            obj.faces[face].plane =
                Plane::from_polygon(&pts).map_err(|_| PushPullError::NonManifoldResult)?;
        }
        // Neighbor faces (their boundary vertices were moved too, since they share the ring):
        for &nf in &neighbor_faces {
            let outer_loop = obj.faces[nf].outer_loop;
            let pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();
            obj.faces[nf].plane =
                Plane::from_polygon(&pts).map_err(|_| PushPullError::NonManifoldResult)?;
        }

        // Step 6: Validate, then swap.
        obj.check_invariants();
        *self = obj;

        // Step 7: Report.
        Ok(PushPullReport {
            face,
            created_faces: vec![],
            removed_faces: vec![],
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

        // Normalise winding to CCW seen from the face normal, so the sub-face
        // faces the same way as the parent.
        let mut pts = loop_path.to_vec();
        if signed_area_on_plane(&pts, normal) < 0.0 {
            pts.reverse();
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
        let sub_face = obj.faces.insert(Face {
            outer_loop: sub_loop,
            inner_loops: Vec::new(),
            plane: face_plane,
            // Imprinted sub-face inherits the parent face's material and UV
            // frame ( +  extension).
            material: obj.faces[face].material,
            uv_frame: obj.faces[face].uv_frame,
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
            let edge = obj.edges.insert(Edge {
                half_edge: h_sub[k],
                twin_half_edge: Some(h_hole[k]),
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
        *self = obj;
        Ok(FaceMergeInnerReport { parent, loop_path })
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
                // material and no UV frame.
                material: None,
                uv_frame: None,
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
            });
            obj.half_edges[wb[k]].edge = e_vert;
            obj.half_edges[wd[prev]].edge = e_vert;
        }

        // Refit the moved sub-face plane (translated; same normal).
        let pts: Vec<Point3> = obj.loop_positions(obj.faces[sub_face].outer_loop).collect();
        obj.faces[sub_face].plane =
            Plane::from_polygon(&pts).map_err(|_| PushPullError::NonManifoldResult)?;

        let shell = obj
            .shells
            .iter()
            .find(|(_, s)| s.faces.contains(&parent))
            .map(|(id, _)| id)
            .expect("parent face belongs to a shell");
        for &w in &walls {
            obj.shells[shell].faces.push(w);
        }

        obj.check_invariants();
        *self = obj;
        Ok(PushPullReport {
            face: sub_face,
            created_faces: walls,
            removed_faces: vec![],
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
            walls.push(self.loops[self.half_edges[a].loop_id].face);
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

        obj.check_invariants();
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

        // --- all checks passed — clone and mutate ---
        let mut obj = self.clone();
        let report = do_split_face(&mut obj, face, path, &ep0, &ep1)?;
        obj.check_invariants();
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
        *self = obj;
        Ok(report)
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
        let index_of: std::collections::HashMap<FaceId, usize> =
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
        let mut component_of_root: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
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
            local_index: &mut std::collections::HashMap<VertexId, usize>,
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
        let mut local_index: std::collections::HashMap<VertexId, usize> =
            std::collections::HashMap::new();
        let mut specs: Vec<(
            Vec<usize>,
            Vec<Vec<usize>>,
            Plane,
            crate::material::FaceMaterial,
            Option<crate::material::UvFrame>,
        )> = Vec::with_capacity(faces.len());
        for &fid in faces {
            let face = &self.faces[fid];
            let outer = loop_indices(self, face.outer_loop, &mut positions, &mut local_index);
            let inner: Vec<Vec<usize>> = face
                .inner_loops
                .iter()
                .map(|&il| loop_indices(self, il, &mut positions, &mut local_index))
                .collect();
            specs.push((outer, inner, face.plane, face.material, face.uv_frame));
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
        let profile =
            Profile::new(f.plane, outer, holes).map_err(|_| PushPullError::NonManifoldResult)?;
        let tool = Object::from_extrusion(&profile, distance)
            .map_err(|_| PushPullError::NonManifoldResult)?;
        match Object::boolean(BooleanOp::Subtract, self, &tool, &Transform::IDENTITY) {
            Ok(result) => {
                result.check_invariants();
                Ok(result)
            }
            Err(BooleanError::EmptyResult) => Err(PushPullError::WouldVanish),
            Err(_) => Err(PushPullError::NonManifoldResult),
        }
    }
}

// ============================================================== private helpers

/// How a path endpoint lands on the outer loop.
#[derive(Debug, Clone)]
enum EndpointHit {
    /// Snapped exactly to an existing vertex.
    Vertex(VertexId),
    /// Lies in the interior of a boundary edge.
    /// `he` is the half-edge (origin → dest) that gets split; `t` is the
    /// parameter along that half-edge (0 = origin, 1 = dest).
    Edge { he: HalfEdgeId, t: f64 },
}

/// The resolved 3-D position of an endpoint hit.
fn endpoint_position(obj: &Object, hit: &EndpointHit) -> Point3 {
    match hit {
        EndpointHit::Vertex(v) => obj.vertices[*v].position,
        EndpointHit::Edge { he, t } => {
            let h = obj.half_edges[*he];
            let p = obj.vertices[h.origin].position;
            let q = obj.vertices[obj.half_edges[h.next].origin].position;
            p + (q - p) * *t
        }
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
                return Ok(Some(EndpointHit::Edge { he: h, t }));
            }
        }
    }
    Ok(None)
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
    //    We must split ep1 before ep0 in terms of storage, but we need to
    //    re-resolve ep0's half-edge pointer after ep1 is split if they share
    //    an edge (they can't share the same half-edge — the endpoints would be
    //    the same point, which would be caught by simplicity — but let's be safe).
    let mut split_boundary_edges: Vec<(EdgeId, [EdgeId; 2])> = Vec::new();

    // Resolve both endpoints into vertices, splitting boundary edges as needed.
    // We process ep1 first so that ep0's half-edge index (if edge hit) is not
    // disturbed (ep0 comes first in the loop, ep1 comes later).
    // Actually, order matters: if both are edge hits on the *same* edge, that's a
    // degenerate case (path.len() == 2, both endpoints on same edge) — but since
    // they must be distinct boundary points and we already checked path simplicity,
    // this can't happen.  We process ep0 first (lower index in the outer loop)
    // then ep1. But since edge-splitting changes half-edge IDs, we need to be
    // careful: split ep1's edge first (it's "after" ep0 in the loop traversal),
    // then split ep0's edge.  Actually the easiest is: split ep1 first,
    // then re-derive ep0 if it was on the same edge (impossible by simplicity
    // argument), then split ep0.
    //
    // Since both endpoints are on the OUTER loop, and the outer loop's half-edges
    // are stored in the object, splitting one half-edge doesn't change the
    // position of other half-edges (slotmap doesn't move things).  So we can
    // split in any order.

    let v0 = match ep0 {
        EndpointHit::Vertex(v) => *v,
        EndpointHit::Edge { he, t } => {
            let pos = {
                let h = obj.half_edges[*he];
                let p = obj.vertices[h.origin].position;
                let q = obj.vertices[obj.half_edges[h.next].origin].position;
                p + (q - p) * *t
            };
            let (v, dead_edge, new_edges) = split_boundary_edge(obj, *he, pos);
            split_boundary_edges.push((dead_edge, new_edges));
            v
        }
    };

    // Re-read the outer loop half-edges now (ep0 split may have changed the loop).
    let outer_loop = obj.faces[face].outer_loop;

    let v1 = match ep1 {
        EndpointHit::Vertex(v) => *v,
        EndpointHit::Edge { he, t } => {
            // The original half-edge id is still valid (slotmap doesn't move it),
            // but if ep0 was an edge split that inserted a new half-edge *after* ep1's
            // half-edge in the loop, we need the original `he` pointer to still be valid.
            // Slotmap guarantees that; the only half-edge removed is the one that was split,
            // and ep1's `he` is a different half-edge.
            // HOWEVER: if ep0 split the edge immediately before ep1's edge, the `t` value
            // might now refer to the wrong portion. But `t` in EndpointHit is the original
            // t before any splits, and we store `he` as a specific half-edge id. Since
            // slotmap key stability: after split_boundary_edge(ep0), the half-edge `ep1.he`
            // still refers to the same segment (it's a different half-edge). Valid.
            let pos = {
                let h = obj.half_edges[*he];
                let p = obj.vertices[h.origin].position;
                let q = obj.vertices[obj.half_edges[h.next].origin].position;
                p + (q - p) * *t
            };
            let (v, dead_edge, new_edges) = split_boundary_edge(obj, *he, pos);
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
        // Both halves of a split inherit the original face's material and UV
        // frame ( +  extension); face A keeps them by reusing its FaceId.
        material: obj.faces[face].material,
        uv_frame: obj.faces[face].uv_frame,
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
        // Use the centroid of the hole to determine which face contains it.
        let centroid = hole_centroid(&hole_pts);

        let outer_a_pts: Vec<Point3> = obj.loop_positions(outer_loop).collect();
        if point_inside_polygon(centroid, &outer_a_pts, normal) {
            // Assign to face A.
            obj.faces[face].inner_loops.push(*il);
            obj.loops[*il].face = face;
        } else {
            // Assign to face B.
            obj.faces[face_b_id].inner_loops.push(*il);
            obj.loops[*il].face = face_b_id;
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

/// Compute the centroid of a polygon's vertices.
fn hole_centroid(pts: &[Point3]) -> Point3 {
    let n = pts.len() as f64;
    let sum = pts
        .iter()
        .fold(crate::math::Vec3::ZERO, |acc, p| acc + p.to_vec());
    Point3::new(sum.x / n, sum.y / n, sum.z / n)
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
        });
        edge_b_id = obj.edges.insert(Edge {
            half_edge: h_b,
            twin_half_edge: Some(t_a),
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

        // Remove old twin half-edge.
        obj.half_edges.remove(twin_he_id);
    } else {
        // Boundary edge (no twin).
        edge_a_id = obj.edges.insert(Edge {
            half_edge: h_a,
            twin_half_edge: None,
        });
        edge_b_id = obj.edges.insert(Edge {
            half_edge: h_b,
            twin_half_edge: None,
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

    // 1. Find all half-edges on outer_a whose twin is on outer_b.
    //    These form the shared chain(s).  There must be at least one.
    let hes_a: Vec<HalfEdgeId> = obj.loop_half_edges(outer_a).collect();
    let hes_b_set: std::collections::HashSet<HalfEdgeId> = obj.loop_half_edges(outer_b).collect();

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
    let shared_set_a: std::collections::HashSet<HalfEdgeId> = shared_on_a.iter().copied().collect();

    let chain_start_a = shared_on_a
        .iter()
        .copied()
        .find(|&h| {
            let prev = obj.half_edges[h].prev;
            !shared_set_a.contains(&prev)
        })
        .expect("at least one half-edge has a non-shared predecessor");

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
    let before_chain_a = obj.half_edges[chain_a_first].prev; // last non-shared on A (arrives at chain_a_start_v)
    let after_chain_b = obj.half_edges[chain_b_last].next; // first non-shared on B (departs chain_b_start_v)
    let before_chain_b = obj.half_edges[chain_b_first].prev; // last non-shared on B (arrives at chain_b_start_v)

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
    //    Joining: non_a.last() (before_chain_a, arriving at chain_a_start_v) now
    //             points to non_b[0] (after_chain_b, departing from chain_b_start_v).
    //             non_b.last() (before_chain_b) now points to non_a[0] (after_chain_a).

    let merged_seq: Vec<HalfEdgeId> = non_a.iter().copied().chain(non_b.iter().copied()).collect();

    // 5. Wire the merged loop on outer_a.
    wire_loop_sequence(obj, &merged_seq, outer_a, face_a);

    // 6. Find interior vertices of the shared chain (those that are not the
    //    chain endpoints and will be fully orphaned after the chain dissolves).
    let mut candidate_orphans: std::collections::HashSet<VertexId> =
        std::collections::HashSet::new();
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
    //    In the merged loop the two vertices appear as:
    //      chain_a_start_v: h_in = before_chain_a (incoming), h_out = after_chain_b (outgoing)
    //      chain_b_start_v: h_in = before_chain_b (incoming), h_out = after_chain_a (outgoing)
    //
    //    Note: after_chain_a and after_chain_b are still valid half-edge ids but
    //    their loop pointers have been updated to outer_a by wire_loop_sequence.
    //    before_chain_a and before_chain_b were already in non_a / non_b and are
    //    now in the merged loop too.
    let mut healing_removed_edges: Vec<EdgeId> = Vec::new();
    for (h_in, h_out) in [
        (before_chain_a, after_chain_b),
        (before_chain_b, after_chain_a),
    ] {
        // h_in arrives at the candidate vertex V; h_out departs from V.
        let v_pos = obj.vertices[obj.half_edges[h_out].origin].position;
        let prev_v_pos = obj.vertices[obj.half_edges[h_in].origin].position;
        let next_v_pos = obj.vertices[obj.half_edges[obj.half_edges[h_out].next].origin].position;

        let dir_in = (v_pos - prev_v_pos).normalized();
        let dir_out = (next_v_pos - v_pos).normalized();

        let collinear = match (dir_in, dir_out) {
            (Ok(a), Ok(b)) => a.cross(b).length() < tol::NORMAL_DIRECTION,
            _ => false, // degenerate edge — skip healing
        };
        if !collinear {
            continue;
        }

        let vertex_to_heal = obj.half_edges[h_out].origin;

        // Heal only true scars: V must have exactly two departing
        // half-edges (h_out here and h_in's twin in the neighbor loop).
        // Any extra incidence means V is a real model vertex shared with
        // other geometry — removing it would strand those half-edges.
        let departing = obj
            .half_edges
            .values()
            .filter(|he| he.origin == vertex_to_heal)
            .count();
        if departing != 2 {
            continue;
        }

        // Twin of h_in: this half-edge departs V in the neighbor face (t_out_of_v).
        // Twin of h_out: this half-edge arrives at V in the neighbor face (t_into_v).
        let t_out_of_v = obj.half_edges[h_in]
            .twin
            .expect("manifold: twin must exist");
        let t_into_v = obj.half_edges[h_out]
            .twin
            .expect("manifold: twin must exist");
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

        // Repair the Edge: keep the Edge for h_in, set its twin to t_into_v.
        // Retire the Edge for h_out.
        let edge_to_keep = obj.half_edges[h_in].edge;
        let edge_to_remove = obj.half_edges[h_out].edge;
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
    let outgoing_map: std::collections::HashMap<VertexId, HalfEdgeId> = obj
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

    // 13. Refit plane for the merged face over the merged boundary.
    let boundary_pts: Vec<Point3> = obj.loop_positions(outer_a).collect();
    if let Ok(new_plane) = Plane::from_polygon(&boundary_pts) {
        obj.faces[face_a].plane = new_plane;
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
    /// it. `from_faces_with_holes` (the extrude builder) assigns it via HashMap
    /// iteration, so on an extruded box a top-cap corner's `outgoing` may point
    /// at the very top-cap boundary half-edge that a split removes — which used
    /// to flake the validator (~3/5) depending on the per-process HashMap seed.
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
