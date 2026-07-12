//! Typed topology errors (DEVELOPMENT.md rule 4: fail loudly, never repair).

use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, VertexId};
use crate::topo::WatertightState;

/// An operation would have produced (or an `Object` contains) invalid
/// topology.
///
/// Construction variants carry indices into the caller's input; validation
/// variants carry kernel handles. A validation variant surfacing outside the
/// validator is a kernel bug, not a user error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    // --- construction (indices refer to the caller's input arrays) ---
    /// The input describes no geometry at all.
    EmptyObject,
    /// Face `face` refers to vertex `index`, which is out of range.
    InvalidVertexIndex {
        /// Index of the offending face in the input.
        face: usize,
        /// The out-of-range vertex index.
        index: usize,
    },
    /// Face `face` has fewer than 3 vertices, repeats a vertex, or has no
    /// well-defined normal.
    DegenerateFace {
        /// Index of the offending face in the input.
        face: usize,
    },
    /// Face `face` has a vertex farther than `tol::PLANE_DIST` from its plane.
    NonPlanarFace {
        /// Index of the offending face in the input.
        face: usize,
    },
    /// The directed edge `from -> to` is traversed by more than one face:
    /// either more than two faces share the edge, or two faces are wound
    /// inconsistently.
    NonManifoldEdge {
        /// Input index of the edge's start vertex.
        from: usize,
        /// Input index of the edge's end vertex.
        to: usize,
    },
    /// Vertex `index` is not used by any face.
    UnreferencedVertex {
        /// The unused vertex's index in the input.
        index: usize,
    },

    // --- validation (handles into kernel storage) ---
    /// A stored handle points at a missing element.
    DanglingHandle {
        /// Which link was dangling.
        context: &'static str,
    },
    /// `next`/`prev` of this half-edge are not mutually inverse.
    BrokenLink {
        /// The offending half-edge.
        half_edge: HalfEdgeId,
    },
    /// Twin pointers are not a proper involution or disagree with the edge.
    BrokenTwin {
        /// The offending half-edge.
        half_edge: HalfEdgeId,
    },
    /// A half-edge appears in no loop, or in more than one.
    LoopMembership {
        /// The offending half-edge.
        half_edge: HalfEdgeId,
    },
    /// An edge and its half-edges disagree about each other.
    EdgeHalfEdgeMismatch {
        /// The offending edge.
        edge: EdgeId,
    },
    /// A loop and its face disagree about each other.
    LoopFaceMismatch {
        /// The offending loop.
        loop_id: LoopId,
    },
    /// A face has a boundary vertex farther than `tol::PLANE_DIST` from its
    /// stored plane.
    FaceGeometryNotPlanar {
        /// The offending face.
        face: FaceId,
    },
    /// A face's inner (hole) loop lies entirely outside the face's outer
    /// boundary: the hole belongs to some other region of the plane, so the
    /// face's ring structure does not describe its geometry.
    HoleOutsideFace {
        /// The offending face.
        face: FaceId,
        /// The displaced hole loop.
        loop_id: LoopId,
    },
    /// A vertex's `outgoing` half-edge does not originate at it.
    VertexOutgoingMismatch {
        /// The offending vertex.
        vertex: VertexId,
    },
    /// A vertex is not the origin of any half-edge.
    OrphanVertex {
        /// The offending vertex.
        vertex: VertexId,
    },
    /// A face belongs to zero shells or more than one shell.
    FaceShellMembership {
        /// The offending face.
        face: FaceId,
    },
    /// The stored watertightness flag disagrees with the actual topology.
    WatertightFlagMismatch {
        /// What the flag should have been.
        expected: WatertightState,
    },
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use TopologyError::*;
        match self {
            EmptyObject => write!(f, "input describes no geometry"),
            InvalidVertexIndex { face, index } => {
                write!(f, "face {face} refers to out-of-range vertex index {index}")
            }
            DegenerateFace { face } => write!(f, "face {face} is degenerate"),
            NonPlanarFace { face } => write!(f, "face {face} is not planar"),
            NonManifoldEdge { from, to } => write!(
                f,
                "directed edge {from} -> {to} is traversed by more than one face \
                 (non-manifold or inconsistent winding)"
            ),
            UnreferencedVertex { index } => {
                write!(f, "vertex {index} is not used by any face")
            }
            DanglingHandle { context } => write!(f, "dangling handle: {context}"),
            BrokenLink { half_edge } => {
                write!(
                    f,
                    "half-edge {half_edge:?} has inconsistent next/prev links"
                )
            }
            BrokenTwin { half_edge } => {
                write!(f, "half-edge {half_edge:?} has inconsistent twin links")
            }
            LoopMembership { half_edge } => {
                write!(f, "half-edge {half_edge:?} is not in exactly one loop")
            }
            EdgeHalfEdgeMismatch { edge } => {
                write!(f, "edge {edge:?} disagrees with its half-edges")
            }
            LoopFaceMismatch { loop_id } => {
                write!(f, "loop {loop_id:?} disagrees with its face")
            }
            FaceGeometryNotPlanar { face } => {
                write!(f, "face {face:?} has vertices off its stored plane")
            }
            HoleOutsideFace { face, loop_id } => {
                write!(
                    f,
                    "face {face:?} owns hole loop {loop_id:?} that lies outside its outer boundary"
                )
            }
            VertexOutgoingMismatch { vertex } => {
                write!(
                    f,
                    "vertex {vertex:?} outgoing half-edge does not start there"
                )
            }
            OrphanVertex { vertex } => {
                write!(f, "vertex {vertex:?} is not the origin of any half-edge")
            }
            FaceShellMembership { face } => {
                write!(f, "face {face:?} is not in exactly one shell")
            }
            WatertightFlagMismatch { expected } => {
                write!(f, "watertight flag is wrong, expected {expected:?}")
            }
        }
    }
}

impl std::error::Error for TopologyError {}
