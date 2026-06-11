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

use crate::ids::{EdgeId, FaceId};
use crate::math::{Plane, Point3};
use crate::sketch::Profile;
use crate::topo::Object;
use crate::transform::Transform;

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

/// What `merge_faces` changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceMergeReport {
    /// The single face replacing the two inputs (both input handles die).
    pub merged_face: FaceId,
    /// Now-dead handles of the dissolved shared-boundary edges.
    pub removed_edges: Vec<EdgeId>,
}

/// Typed failures of extrusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtrudeError {
    /// |distance| below [`tol::POINT_MERGE`](crate::tol::POINT_MERGE): the result would be a
    /// zero-thickness shell, which is not a solid.
    DistanceTooSmall,
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
    NonManifoldResult,
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

impl std::fmt::Display for ExtrudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtrudeError::DistanceTooSmall => {
                write!(f, "extrusion distance below tol::POINT_MERGE")
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
            Plane::from_polygon(&rev_outer).expect("profile outer is valid")
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
            Plane::from_polygon(&far_outer_pts).expect("far outer is valid")
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
            let wall_plane = Plane::from_polygon(&wall_pts).expect("outer wall is non-degenerate");
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
                    Plane::from_polygon(&wall_pts).expect("hole wall is non-degenerate");
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
                        Plane::from_polygon(&pts).expect("reversed loop still spans its plane");
                    (rev_outer, rev_inners, plane)
                })
                .collect();
        }

        let obj = Object::from_faces_with_holes(&positions, &face_specs);
        obj.check_invariants();
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
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn push_pull(
        &mut self,
        face: FaceId,
        distance: f64,
    ) -> Result<PushPullReport, PushPullError> {
        todo!("M1: push/pull (see tests/op_specs.rs: inverse restores topology)")
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
    /// # Errors
    /// See [`StickyError`]; all leave the object untouched.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn split_face(
        &mut self,
        face: FaceId,
        path: &[Point3],
    ) -> Result<FaceSplitReport, StickyError> {
        todo!("M1: face split (see tests/op_specs.rs: split + merge is identity)")
    }

    /// Inverse sticky rule: dissolves the boundary between the two coplanar
    /// faces adjacent to `edge`, merging them into one face. If the two faces
    /// share a chain of edges, the entire shared chain dissolves (a single
    /// face cannot have two disconnected boundaries to the same neighbor).
    ///
    /// `split_face` followed by `merge_faces` on any returned cut edge is the
    /// identity (up to handle renaming).
    ///
    /// # Errors
    /// See [`StickyError`]; all leave the object untouched.
    #[allow(unused_variables)] // contract stub: implementation lands in M1
    pub fn merge_faces(&mut self, edge: EdgeId) -> Result<FaceMergeReport, StickyError> {
        todo!("M1: face merge (see tests/op_specs.rs)")
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
    #[allow(unused_variables)] // contract stub: implementation lands in M2
    pub fn boolean(
        op: BooleanOp,
        a: &Object,
        b: &Object,
        b_to_a: &Transform,
    ) -> Result<Object, BooleanError> {
        todo!("M2: boolean combination (see tests/op_specs.rs)")
    }
}
