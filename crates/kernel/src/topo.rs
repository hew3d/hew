//! Half-edge mesh elements and the `Object` that owns them.
//!
//! An `Object` is an island of geometry (ARCHITECTURE.md): sticky-geometry rules
//! apply inside it, never across Objects. Its watertightness is tracked
//! explicitly and kept honest by the validator.

use slotmap::SlotMap;

use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, ShellId, VertexId};
use crate::material::FaceMaterial;
use crate::math::{Plane, Point3};

/// A mesh vertex.
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    /// Position in f64 meters.
    pub position: Point3,
    /// One half-edge originating at this vertex.
    pub outgoing: HalfEdgeId,
}

/// One direction of an edge, bounding one loop.
#[derive(Debug, Clone, Copy)]
pub struct HalfEdge {
    /// Vertex this half-edge starts at.
    pub origin: VertexId,
    /// Opposite-direction partner on the adjacent face; `None` on a mesh
    /// boundary (the Object is then not watertight).
    pub twin: Option<HalfEdgeId>,
    /// Next half-edge around the loop.
    pub next: HalfEdgeId,
    /// Previous half-edge around the loop.
    pub prev: HalfEdgeId,
    /// The undirected edge this half-edge belongs to.
    pub edge: EdgeId,
    /// The loop this half-edge bounds.
    pub loop_id: LoopId,
}

/// An undirected edge: one or two half-edges.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    /// Always present.
    pub half_edge: HalfEdgeId,
    /// `None` on a boundary edge.
    pub twin_half_edge: Option<HalfEdgeId>,
}

/// Whether a loop is a face's outer boundary or a hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    /// Outer boundary, wound counter-clockwise seen from the face normal.
    Outer,
    /// Hole boundary (opposite winding).
    Inner,
}

/// A closed cycle of half-edges bounding a face.
#[derive(Debug, Clone, Copy)]
pub struct Loop {
    /// The face this loop bounds.
    pub face: FaceId,
    /// An arbitrary half-edge on the cycle.
    pub first_half_edge: HalfEdgeId,
    /// Outer boundary or hole.
    pub kind: LoopKind,
}

/// A planar polygonal face; holes are represented as inner loops.
#[derive(Debug, Clone)]
pub struct Face {
    /// The outer boundary.
    pub outer_loop: LoopId,
    /// Hole boundaries (empty in M0; the builder does not create holes yet).
    pub inner_loops: Vec<LoopId>,
    /// The supporting plane, oriented so the outer loop winds CCW seen from
    /// the normal side.
    pub plane: Plane,
    /// The face's material in the [`crate::document::Document`] palette, or
    /// `None` for the default (unpainted) material (ARCHITECTURE.md). Carried by
    /// face-creating ops: a split's children inherit the parent's material; a
    /// boolean's result faces inherit their source face's; freshly extruded
    /// side walls default to `None`.
    pub material: FaceMaterial,
}

/// A connected set of faces. M0 builds a single shell per Object.
#[derive(Debug, Clone)]
pub struct Shell {
    /// Faces belonging to this shell.
    pub faces: Vec<FaceId>,
}

/// Whether an Object's mesh encloses a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatertightState {
    /// Every edge has two half-edges; the mesh encloses a volume.
    Watertight,
    /// The mesh has boundary edges; flagged, never silently tolerated.
    Open,
}

/// An island of half-edge geometry with tracked watertightness.
///
/// All mutation goes through kernel operations that re-establish invariants;
/// in debug builds every public mutation re-runs the topology validator
/// (DEVELOPMENT.md rule 2).
#[derive(Debug, Clone)]
pub struct Object {
    pub(crate) vertices: SlotMap<VertexId, Vertex>,
    pub(crate) half_edges: SlotMap<HalfEdgeId, HalfEdge>,
    pub(crate) edges: SlotMap<EdgeId, Edge>,
    pub(crate) loops: SlotMap<LoopId, Loop>,
    pub(crate) faces: SlotMap<FaceId, Face>,
    pub(crate) shells: SlotMap<ShellId, Shell>,
    pub(crate) watertight: WatertightState,
    /// The object's base material ( follow-up): a face whose own `material`
    /// is `None` resolves to this. New faces (extrude walls, boolean walls) are
    /// created `None`, so they inherit the base — giving a solid a consistent
    /// color/texture "throughout". `None` here means the renderer's neutral
    /// default.
    pub(crate) default_material: FaceMaterial,
}

impl Object {
    /// An object with no elements. Crate-internal: public construction goes
    /// through [`Object::from_polygons`], which rejects empty input.
    pub(crate) fn empty() -> Object {
        Object {
            vertices: SlotMap::with_key(),
            half_edges: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            loops: SlotMap::with_key(),
            faces: SlotMap::with_key(),
            shells: SlotMap::with_key(),
            watertight: WatertightState::Open,
            default_material: None,
        }
    }

    /// Vertex storage (read-only).
    pub fn vertices(&self) -> &SlotMap<VertexId, Vertex> {
        &self.vertices
    }

    /// Half-edge storage (read-only).
    pub fn half_edges(&self) -> &SlotMap<HalfEdgeId, HalfEdge> {
        &self.half_edges
    }

    /// Edge storage (read-only).
    pub fn edges(&self) -> &SlotMap<EdgeId, Edge> {
        &self.edges
    }

    /// Loop storage (read-only).
    pub fn loops(&self) -> &SlotMap<LoopId, Loop> {
        &self.loops
    }

    /// Face storage (read-only).
    pub fn faces(&self) -> &SlotMap<FaceId, Face> {
        &self.faces
    }

    /// Shell storage (read-only).
    pub fn shells(&self) -> &SlotMap<ShellId, Shell> {
        &self.shells
    }

    /// Whether this Object currently encloses a volume.
    pub fn watertight(&self) -> WatertightState {
        self.watertight
    }

    /// The object's base material ( follow-up): the material a face with no
    /// own material resolves to. `None` = the renderer's neutral default.
    pub fn default_material(&self) -> FaceMaterial {
        self.default_material
    }

    /// The half-edges of `loop_id` in cycle order, starting at its
    /// `first_half_edge`.
    ///
    /// Assumes a structurally valid object (loops close); guaranteed by the
    /// validator for anything the kernel hands out.
    pub fn loop_half_edges(&self, loop_id: LoopId) -> impl Iterator<Item = HalfEdgeId> + '_ {
        let first = self.loops[loop_id].first_half_edge;
        let mut current = Some(first);
        std::iter::from_fn(move || {
            let h = current?;
            let next = self.half_edges[h].next;
            current = if next == first { None } else { Some(next) };
            Some(h)
        })
    }

    /// The positions of `loop_id`'s vertices in cycle order.
    pub fn loop_positions(&self, loop_id: LoopId) -> impl Iterator<Item = Point3> + '_ {
        self.loop_half_edges(loop_id)
            .map(|h| self.vertices[self.half_edges[h].origin].position)
    }
}
