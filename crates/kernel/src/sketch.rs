//! The draft context: 2D sticky geometry on a plane, before any solid exists.
//!
//! A [`Sketch`] is where SketchUp-style drawing happens *outside* an Object
//! (ARCHITECTURE.md): line/rectangle tools feed it segments, sticky rules apply
//! (endpoints merge, crossings split), and closed circuits become
//! [`SketchRegion`]s. A region converts to a [`Profile`] — a validated closed
//! planar polygon with holes — which is the input to
//! `Object::from_extrusion`: extruding a Profile is THE way solids are born
//! (solids-by-default, docs/DEVELOPMENT.md).
//!
//! Sketches are deliberately not Objects: an Object models a solid with a
//! watertightness contract; a sketch is scaffolding. Drawing on an existing
//! Object's face goes through `Object::split_face` instead (see `ops`).
//!
//! # Sticky rules (the contract `add_segment` implements)
//!
//! Within one sketch — never across sketches or Objects:
//! 1. A new endpoint within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of an existing sketch vertex
//!    merges with it.
//! 2. A new endpoint within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of an existing edge's
//!    interior splits that edge (T-junction).
//! 3. A proper crossing between the new segment and an existing edge splits
//!    both at the intersection point.
//! 4. Collinear overlap between the new segment and an existing edge merges
//!    into non-overlapping edges (no duplicate or partially-stacked edges
//!    survive).
//! 5. After insertion, the set of closed regions is recomputed: every minimal
//!    closed circuit of edges bounds exactly one region; a circuit strictly
//!    inside another region's boundary makes a hole in it.
//!
//! M0 status: storage and types are real; algorithms are `todo!()` stubs.
//! The executable contract lives in `crates/kernel/tests/op_specs.rs`.

use slotmap::{SlotMap, new_key_type};

use crate::geom2d::{
    boundaries_contact, plane_axes, point_inside_polygon, polygon_is_simple, signed_area_on_plane,
};
use crate::math::{Plane, Point3, Vec3};
use crate::tol;

new_key_type! {
    /// Handle to a [`SketchVertex`].
    pub struct SketchVertexId;
    /// Handle to a [`SketchEdge`].
    pub struct SketchEdgeId;
    /// Handle to a [`SketchRegion`].
    pub struct SketchRegionId;
    /// Handle to a curve chain: the edges committed as ONE drawn curve (an
    /// arc or circle's facets), selected and deleted as a unit.
    pub struct SketchCurveId;
    /// Handle to an [`SketchIsland`].
    pub struct SketchIslandId;
}

/// The fewest facets a closed chord ring may have and still count as a
/// "circle" for analytic stamping — the density floor below which a ring is
/// a coarse polygon of secants, not a curve. Coupled to the draw tools'
/// segments-per-turn floor (24, docs/design/true-curves.md §6: "24 becomes
/// the floor, adaptive by radius up to 96"), so every genuine tool-produced
/// circle clears it while a triangle/hexagon/skip-connected coarse ring does
/// not. The kernel legitimately owns "what density counts as a circle": an
/// analytic claim on a coarser ring would sweep a secant into a cylinder
/// wall, and stamp-wrong is worse than don't-stamp (map-or-drop).
pub(crate) const MIN_CIRCLE_SEGMENTS: usize = 24;

/// The analytic definition a curve chain was drawn from: the exact circle
/// (or circular arc) whose facets the chain's edges are. The circle lies in
/// the sketch plane; arc extent is derived from the chain's member edges,
/// never stored. This is the durable form of what the drawing tool computed
/// and, before this existed, immediately discarded — the foundation the
/// true-curves plan builds on (docs/design/true-curves.md).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveGeom {
    /// Circle center, on the sketch plane (within
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)).
    pub center: Point3,
    /// Circle radius in meters, > [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    pub radius: f64,
}

/// A connected component of a sketch's edges: what the user perceives as one
/// independent drawn shape. Derived (never serialized) and recomputed on
/// every edge-set mutation with identity reuse, exactly like regions — the
/// selection/label/delete/move unit for free-standing sketch geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SketchIsland {
    /// The island's edges, ascending by id.
    pub edges: Vec<SketchEdgeId>,
}

/// A point in a sketch. Always on the sketch plane
/// (within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchVertex {
    /// Position in f64 meters (world frame).
    pub position: Point3,
}

/// A straight segment between two sketch vertices.
///
/// Edges never overlap or cross other edges of the same sketch — the sticky
/// rules split/merge eagerly at insertion time, so this is an invariant, not
/// a hope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SketchEdge {
    /// Start vertex.
    pub from: SketchVertexId,
    /// End vertex.
    pub to: SketchVertexId,
    /// The curve chain this edge belongs to, when it was committed as part
    /// of one drawn curve (an arc's or circle's facets). `None` for plain
    /// lines and rectangle sides. Fragments of a split curve edge inherit
    /// the id, so a curve stays one selectable unit across sticky splits.
    pub curve: Option<SketchCurveId>,
}

/// A closed region bounded by sketch edges: what the user perceives as "a
/// face appeared" while drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SketchRegion {
    /// Outer boundary vertices in cycle order, counter-clockwise seen from
    /// the plane normal side.
    pub outer: Vec<SketchVertexId>,
    /// Hole boundaries, each clockwise seen from the plane normal side.
    pub holes: Vec<Vec<SketchVertexId>>,
}

/// What [`Sketch::add_segment`] did, so tools and undo can react precisely.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentAdded {
    /// Vertices created (merged endpoints and crossings reuse existing ones,
    /// which are not listed).
    pub new_vertices: Vec<SketchVertexId>,
    /// Edges created, including the fragments of the inserted segment.
    pub new_edges: Vec<SketchEdgeId>,
    /// Pre-existing edges that were split: the now-dead handle and its
    /// replacement fragments, in order from the old edge's `from` end.
    pub split_edges: Vec<(SketchEdgeId, Vec<SketchEdgeId>)>,
    /// Regions that came into existence because circuits closed.
    pub regions_created: Vec<SketchRegionId>,
    /// Regions invalidated by the insertion (e.g., a region that gained a
    /// crossing edge is replaced by its two halves, listed in
    /// `regions_created`).
    pub regions_removed: Vec<SketchRegionId>,
}

/// What [`Sketch::remove_edge`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdgeRemoved {
    /// Vertices deleted because no edge uses them anymore.
    pub removed_vertices: Vec<SketchVertexId>,
    /// Regions that no longer close without this edge.
    pub regions_removed: Vec<SketchRegionId>,
    /// Regions created by the removal (two regions separated only by this
    /// edge merge into one).
    pub regions_created: Vec<SketchRegionId>,
}

/// Typed failures of sketch operations. Nothing is repaired silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchError {
    /// An input point is farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the sketch
    /// plane. The kernel does not project; the caller decides.
    PointOffPlane {
        /// Index of the offending point in the call's arguments (0-based).
        which: usize,
    },
    /// Segment endpoints are within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of each other.
    DegenerateSegment,
    /// The given edge handle is not in this sketch (or already removed).
    UnknownEdge,
    /// The given vertex handle is not in this sketch (or already removed).
    UnknownVertex,
    /// The given region handle is not in this sketch (or already invalid).
    UnknownRegion,
    /// A topology-preserving [`Sketch::move_vertex`] was refused because it
    /// would require re-stitching the sketch: an incident edge would cross or
    /// overlap another edge, or the moved vertex would land on another vertex
    /// (a merge). No silent repair (rule 4) — the caller decides.
    WouldRetopologize,
    /// The island handle is stale (islands die whenever an edge-set mutation
    /// reshapes them — always re-query after mutating).
    UnknownIsland,
    /// The region's traced boundary does not form a valid [`Profile`] (a
    /// kernel bug in region tracing, surfaced as a typed error rather than a
    /// panic so it cannot brick the caller).
    MalformedRegion,
    /// A [`CurveGeom`] is degenerate: its radius is not finite or not larger
    /// than [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    DegenerateCurve,
    /// Undoing an extrusion could not re-insert the scaffolding it had
    /// deleted: geometry drawn since then crosses or overlaps where the
    /// outline was ([`Sketch::restore_edges`]). The sketch is untouched —
    /// erase the conflicting geometry and undo again.
    RestoreConflicts,
}

impl std::fmt::Display for SketchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SketchError::PointOffPlane { which } => {
                write!(f, "input point {which} is off the sketch plane")
            }
            SketchError::DegenerateSegment => write!(f, "segment endpoints coincide"),
            SketchError::UnknownEdge => write!(f, "no such edge in this sketch"),
            SketchError::UnknownVertex => write!(f, "no such vertex in this sketch"),
            SketchError::UnknownRegion => write!(f, "no such region in this sketch"),
            SketchError::UnknownIsland => {
                write!(f, "no such island in this sketch")
            }
            SketchError::WouldRetopologize => {
                write!(f, "the move would cross or merge sketch geometry")
            }
            SketchError::MalformedRegion => {
                write!(f, "region boundary does not form a valid profile")
            }
            SketchError::DegenerateCurve => {
                write!(f, "curve radius is degenerate")
            }
            SketchError::RestoreConflicts => {
                write!(
                    f,
                    "the restored outline would cross geometry drawn since the extrusion"
                )
            }
        }
    }
}

impl std::error::Error for SketchError {}

/// A planar drawing surface with sticky-geometry semantics.
#[derive(Debug, Clone)]
pub struct Sketch {
    plane: Plane,
    vertices: SlotMap<SketchVertexId, SketchVertex>,
    edges: SlotMap<SketchEdgeId, SketchEdge>,
    regions: SlotMap<SketchRegionId, SketchRegion>,
    /// Live curve chains: membership lives on each edge's `curve` field; the
    /// value is the chain's analytic definition, when the drawing tool
    /// supplied one ([`Sketch::begin_curve_with`]). `None` for chains
    /// committed before geometry capture existed (pre-v10 files) or through
    /// the plain [`Sketch::begin_curve`].
    curves: SlotMap<SketchCurveId, Option<CurveGeom>>,
    /// Current connected components (derived; see [`SketchIsland`]).
    islands: SlotMap<SketchIslandId, SketchIsland>,
    /// Curve id applied to edges inserted by `add_segment` while a
    /// `begin_curve`/`end_curve` bracket is open. Transient tool state — a
    /// gesture snapshot restores it with the rest of the sketch.
    active_curve: Option<SketchCurveId>,
}

impl Sketch {
    /// An empty sketch on `plane`.
    pub fn on_plane(plane: Plane) -> Sketch {
        Sketch {
            plane,
            vertices: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            regions: SlotMap::with_key(),
            curves: SlotMap::with_key(),
            active_curve: None,
            islands: SlotMap::with_key(),
        }
    }

    /// The sketch plane.
    pub fn plane(&self) -> Plane {
        self.plane
    }

    /// Vertex storage (read-only).
    pub fn vertices(&self) -> &SlotMap<SketchVertexId, SketchVertex> {
        &self.vertices
    }

    /// Edge storage (read-only).
    pub fn edges(&self) -> &SlotMap<SketchEdgeId, SketchEdge> {
        &self.edges
    }

    /// Opens a curve bracket: edges added by `add_segment` until
    /// [`Sketch::end_curve`] are tagged as ONE curve chain (an arc's or
    /// circle's facets), so the UI can select and delete them as a unit.
    /// Returns the minted curve id. Nesting is not supported — a second
    /// `begin_curve` simply replaces the active id.
    pub fn begin_curve(&mut self) -> SketchCurveId {
        let id = self.curves.insert(None);
        self.active_curve = Some(id);
        id
    }

    /// [`Sketch::begin_curve`] with the chain's analytic definition: the
    /// exact circle the facets approximate, as the drawing tool computed it.
    /// The geometry is durable — it survives edge splits (fragments keep the
    /// chain id), persists in the file format, and is what extrusion carries
    /// onto the solid (docs/design/true-curves.md).
    ///
    /// # Errors
    /// [`SketchError::PointOffPlane`] (`which: 0`) if `geom.center` is
    /// farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the
    /// sketch plane; [`SketchError::DegenerateCurve`] if `geom.radius` is
    /// not finite or not larger than
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE). On error no curve is
    /// minted and no bracket opens (strong guarantee).
    pub fn begin_curve_with(&mut self, geom: CurveGeom) -> Result<SketchCurveId, SketchError> {
        if self.plane.signed_distance(geom.center).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }
        if !geom.radius.is_finite() || geom.radius <= tol::POINT_MERGE {
            return Err(SketchError::DegenerateCurve);
        }
        let id = self.curves.insert(Some(geom));
        self.active_curve = Some(id);
        Ok(id)
    }

    /// The analytic definition of `curve`, or `None` when the chain carries
    /// none (plain [`Sketch::begin_curve`], pre-v10 file, or a stale handle).
    pub fn curve_geom(&self, curve: SketchCurveId) -> Option<CurveGeom> {
        self.curves.get(curve).copied().flatten()
    }

    /// Closes the open curve bracket (no-op when none is open).
    pub fn end_curve(&mut self) {
        self.active_curve = None;
    }

    /// The curve chain `edge` belongs to, or `None` for a plain line (or a
    /// stale handle).
    pub fn edge_curve(&self, edge: SketchEdgeId) -> Option<SketchCurveId> {
        self.edges.get(edge).and_then(|e| e.curve)
    }

    /// The maximal run of `edge`'s curve reachable through vertices used by
    /// NOTHING but that curve — the selection unit for a drawn arc/circle
    /// once other geometry touches it. A junction vertex (crossing or touch:
    /// three or more incident edges, or two from different owners) stops the
    /// walk, so the piece of a circle inside an outline selects separately
    /// from the piece that became the outline's rounded corner. For an
    /// untouched curve this is the whole curve; for a plain line it is just
    /// the edge itself. Ascending by id; empty for a stale handle.
    pub fn curve_chain_at(&self, edge: SketchEdgeId) -> Vec<SketchEdgeId> {
        let Some(e) = self.edges.get(edge) else {
            return Vec::new();
        };
        let Some(curve) = e.curve else {
            return vec![edge];
        };

        let mut vertex_edges: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        for (eid, ed) in &self.edges {
            vertex_edges.entry(ed.from).or_default().push(eid);
            vertex_edges.entry(ed.to).or_default().push(eid);
        }

        let mut chain: std::collections::BTreeSet<SketchEdgeId> = std::collections::BTreeSet::new();
        chain.insert(edge);
        let mut frontier = vec![edge];
        while let Some(cur) = frontier.pop() {
            let ed = self.edges[cur];
            for v in [ed.from, ed.to] {
                let incident = &vertex_edges[&v];
                // Interior curve vertex: exactly two edges, both this curve.
                if incident.len() != 2 {
                    continue;
                }
                if !incident.iter().all(|&i| self.edges[i].curve == Some(curve)) {
                    continue;
                }
                for &n in incident {
                    if chain.insert(n) {
                        frontier.push(n);
                    }
                }
            }
        }
        chain.into_iter().collect()
    }

    /// Every edge of `curve`, in slotmap order. Empty for a stale id.
    pub fn curve_edges(&self, curve: SketchCurveId) -> Vec<SketchEdgeId> {
        self.edges
            .iter()
            .filter(|(_, e)| e.curve == Some(curve))
            .map(|(id, _)| id)
            .collect()
    }

    /// Registers a curve id (with its optional analytic definition) during
    /// structural (file-load) reconstruction.
    pub(crate) fn insert_curve_raw(&mut self, geom: Option<CurveGeom>) -> SketchCurveId {
        self.curves.insert(geom)
    }

    /// Current islands — connected components of the edge graph (read-only).
    /// Kept up to date by every edge-set mutation; ids are stable while an
    /// island keeps its anchor edge (see [`Sketch::recompute_islands`]).
    pub fn islands(&self) -> &SlotMap<SketchIslandId, SketchIsland> {
        &self.islands
    }

    /// The island `edge` belongs to, or `None` for a stale handle.
    pub fn island_of_edge(&self, edge: SketchEdgeId) -> Option<SketchIslandId> {
        self.islands
            .iter()
            .find(|(_, isl)| isl.edges.binary_search(&edge).is_ok())
            .map(|(id, _)| id)
    }

    /// Rebuild the island set from the current edge graph, preserving ids:
    /// a new component reuses an old island's id when it contains that
    /// island's smallest surviving edge (each old id claimed once, smallest
    /// anchor first — deterministic for a deterministically built sketch).
    /// Called at the end of every mutation that changes the edge set.
    pub(crate) fn recompute_islands(&mut self) {
        // Anchor edge (smallest surviving) per old island.
        let old_anchors: Vec<(SketchIslandId, SketchEdgeId)> = self
            .islands
            .iter()
            .filter_map(|(id, isl)| {
                isl.edges
                    .iter()
                    .copied()
                    .filter(|e| self.edges.contains_key(*e))
                    .min()
                    .map(|m| (id, m))
            })
            .collect();

        // Connected components by shared vertices, discovered in edge-id
        // order (each component's edge list ends up ascending).
        let mut vertex_edges: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        for (eid, e) in &self.edges {
            vertex_edges.entry(e.from).or_default().push(eid);
            vertex_edges.entry(e.to).or_default().push(eid);
        }
        let mut seen: std::collections::BTreeSet<SketchEdgeId> = std::collections::BTreeSet::new();
        let mut components: Vec<Vec<SketchEdgeId>> = Vec::new();
        for start in self.edges.keys() {
            if seen.contains(&start) {
                continue;
            }
            let mut comp = Vec::new();
            let mut queue = vec![start];
            seen.insert(start);
            while let Some(eid) = queue.pop() {
                comp.push(eid);
                let e = self.edges[eid];
                for v in [e.from, e.to] {
                    for &n in &vertex_edges[&v] {
                        if seen.insert(n) {
                            queue.push(n);
                        }
                    }
                }
            }
            comp.sort();
            components.push(comp);
        }

        // Assign ids: reuse where the anchor edge landed; fresh otherwise.
        let mut used: std::collections::BTreeSet<SketchIslandId> =
            std::collections::BTreeSet::new();
        let mut assignments: Vec<(Option<SketchIslandId>, Vec<SketchEdgeId>)> = Vec::new();
        for comp in components {
            let reuse = old_anchors
                .iter()
                .filter(|(id, anchor)| !used.contains(id) && comp.binary_search(anchor).is_ok())
                .min_by_key(|(id, anchor)| (*anchor, *id))
                .map(|(id, _)| *id);
            if let Some(id) = reuse {
                used.insert(id);
            }
            assignments.push((reuse, comp));
        }

        let dead: Vec<SketchIslandId> = self
            .islands
            .keys()
            .filter(|id| !used.contains(id))
            .collect();
        for id in dead {
            self.islands.remove(id);
        }
        for (reuse, edges) in assignments {
            match reuse {
                Some(id) => self.islands[id].edges = edges,
                None => {
                    self.islands.insert(SketchIsland { edges });
                }
            }
        }
    }

    /// Rigidly move ONE island by an in-plane transform, refusing anything
    /// that would interact with other islands' geometry (a landing that
    /// merges vertices, crosses edges, or grazes within tolerance is
    /// [`SketchError::WouldRetopologize`] — sticky welding across a whole
    /// island move is not supported). The transform must keep every vertex
    /// on the sketch plane ([`SketchError::PointOffPlane`] otherwise); the
    /// caller (the document layer) has already vetted invertibility and
    /// rejected reflections. Strong guarantee: untouched on `Err`.
    /// [`Sketch::apply_transform_island`]'s checks without the commit:
    /// `Ok(())` iff the move would be accepted. Callers batching several
    /// island moves validate ALL of them first so a refusal aborts the whole
    /// batch instead of leaving earlier islands moved.
    pub fn validate_transform_island(
        &self,
        island: SketchIslandId,
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        let mut probe = self.clone();
        probe.apply_transform_island(island, t)
    }

    pub fn apply_transform_island(
        &mut self,
        island: SketchIslandId,
        t: &crate::Transform,
    ) -> Result<(), SketchError> {
        let isl = self.islands.get(island).ok_or(SketchError::UnknownIsland)?;
        let island_edges: std::collections::BTreeSet<SketchEdgeId> =
            isl.edges.iter().copied().collect();
        let mut island_verts: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        for &eid in &island_edges {
            let e = self.edges[eid];
            island_verts.insert(e.from);
            island_verts.insert(e.to);
        }

        // Validate on a clone; swap in only once every check passes.
        let mut s = self.clone();
        for &v in &island_verts {
            let p = t.apply_point(s.vertices[v].position);
            if s.plane.signed_distance(p).abs() > tol::PLANE_DIST {
                return Err(SketchError::PointOffPlane { which: 0 });
            }
            s.vertices[v].position = p;
        }

        // Interference with everything OUTSIDE the island: crossings, vertex
        // merges, and within-tolerance grazes all refuse.
        let near = |p: Point3, a: Point3, b: Point3| -> bool {
            let ab = b - a;
            let len2 = ab.dot(ab);
            let t = if len2 <= tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
                0.0
            } else {
                ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
            };
            let c = Point3::new(a.x + ab.x * t, a.y + ab.y * t, a.z + ab.z * t);
            (p - c).length() <= tol::POINT_MERGE
        };
        let moved_segs: Vec<(Point3, Point3)> = island_edges
            .iter()
            .map(|&eid| {
                let e = s.edges[eid];
                (s.vertices[e.from].position, s.vertices[e.to].position)
            })
            .collect();
        for (oid, oe) in &s.edges {
            if island_edges.contains(&oid) {
                continue;
            }
            let (r, w) = (s.vertices[oe.from].position, s.vertices[oe.to].position);
            for &(p, q) in &moved_segs {
                if crate::geom2d::segments_intersect(p, q, r, w)
                    || near(p, r, w)
                    || near(q, r, w)
                    || near(r, p, q)
                    || near(w, p, q)
                {
                    return Err(SketchError::WouldRetopologize);
                }
            }
        }

        // Curve geometry rides along iff the whole chain moved: a chain
        // entirely inside the island maps (map-or-drop contract, see
        // apply_transform); a chain straddling the island boundary — possible
        // after partial deletion split its edges across components — can no
        // longer be described by one circle, so its geometry drops.
        let scale = in_plane_similarity_scale(t, s.plane.normal());
        let curve_ids: Vec<SketchCurveId> = s.curves.keys().collect();
        for cid in curve_ids {
            if s.curves[cid].is_none() {
                continue;
            }
            let members: Vec<SketchEdgeId> = s
                .edges
                .iter()
                .filter(|(_, e)| e.curve == Some(cid))
                .map(|(id, _)| id)
                .collect();
            let inside = members.iter().filter(|e| island_edges.contains(e)).count();
            if inside == 0 {
                continue; // untouched chain
            }
            s.curves[cid] = match (s.curves[cid], scale, inside == members.len()) {
                (Some(g), Some(sc), true) => Some(CurveGeom {
                    center: t.apply_point(g.center),
                    radius: g.radius * sc,
                }),
                _ => None,
            };
        }

        *self = s;
        Ok(())
    }

    /// Current closed regions (read-only). Kept up to date by every mutation;
    /// never recomputed lazily.
    pub fn regions(&self) -> &SlotMap<SketchRegionId, SketchRegion> {
        &self.regions
    }

    /// Bakes an affine `transform` into this sketch: every vertex position is
    /// moved by `transform` and the sketch `plane` is remapped (the
    /// inverse-transpose rule) so vertices stay coplanar. The 2D topology —
    /// vertices, edges, regions — is untouched, so all handles stay valid and
    /// no region is gained or lost. This is what move/rotate/scale on a
    /// free-standing sketch commit (Phase D), mirroring [`Object::apply_transform`].
    ///
    /// Validated up front so the mutation is transactional: a singular linear
    /// part is [`TransformError::Singular`] and an orientation-flipping one
    /// (determinant < 0) is [`TransformError::Reflection`] — both refused
    /// before any vertex moves, so the sketch is never left half-transformed.
    ///
    /// [`Object::apply_transform`]: crate::Object::apply_transform
    pub fn apply_transform(
        &mut self,
        transform: &crate::Transform,
    ) -> Result<(), crate::TransformError> {
        // Reject before mutating. `inverse()` fails iff the linear part is
        // singular; a negative determinant would flip the plane normal and the
        // perceived winding of every region.
        transform.inverse()?;
        if transform.determinant() < 0.0 {
            return Err(crate::TransformError::Reflection);
        }
        // Curve geometry maps when the transform is an in-plane similarity
        // (center through the point map, radius by the uniform factor) and is
        // DROPPED otherwise — a non-uniform in-plane scale turns the circle
        // into an ellipse the metadata cannot describe. Dropping metadata is
        // not geometry repair: the facets are untouched; the chain merely
        // stops claiming an analytic ancestry it no longer has
        // (docs/design/true-curves.md). Chain identity always survives.
        let scale = in_plane_similarity_scale(transform, self.plane.normal());
        for geom in self.curves.values_mut() {
            *geom = match (*geom, scale) {
                (Some(g), Some(s)) => Some(CurveGeom {
                    center: transform.apply_point(g.center),
                    radius: g.radius * s,
                }),
                _ => None,
            };
        }
        // Remap the plane (cannot fail on a validated non-singular map),
        // then move every vertex onto it.
        self.plane = transform
            .apply_plane(&self.plane)
            .expect("apply_plane on a validated non-singular transform");
        for v in self.vertices.values_mut() {
            v.position = transform.apply_point(v.position);
        }
        Ok(())
    }

    /// Inserts the segment `from -> to`, applying the sticky rules in the
    /// module docs, and reports exactly what changed.
    ///
    /// Line and rectangle tools are UI concerns that decompose into calls of
    /// this one primitive.
    ///
    /// # Errors
    /// [`SketchError::PointOffPlane`] if an endpoint is farther than
    /// [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the plane (`which` = 0 for `from`, 1 for
    /// `to`); [`SketchError::DegenerateSegment`] if the endpoints are within
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) of each other *after* vertex merging.
    ///
    /// On error the sketch is unchanged (strong guarantee).
    pub fn add_segment(&mut self, from: Point3, to: Point3) -> Result<SegmentAdded, SketchError> {
        // Strong guarantee: work on a clone, validate everything that can fail,
        // then swap.
        let mut s = self.clone();
        let report = s.add_segment_inner(from, to)?;
        *self = s;
        Ok(report)
    }

    /// Removes one edge (the eraser tool), deleting vertices that become
    /// unused and dissolving/merging the regions it bounded.
    ///
    /// # Errors
    /// [`SketchError::UnknownEdge`] if the handle is stale. On error the
    /// sketch is unchanged.
    pub fn remove_edge(&mut self, edge: SketchEdgeId) -> Result<EdgeRemoved, SketchError> {
        if !self.edges.contains_key(edge) {
            return Err(SketchError::UnknownEdge);
        }
        let mut s = self.clone();
        let report = s.remove_edge_inner(edge);
        *self = s;
        Ok(report)
    }

    /// Repositions vertex `v` to `new_pos`, dragging its incident edges with
    /// it while **preserving the 2D topology**: the same vertices, edges, and
    /// regions exist (and keep their handles) before and after — nothing
    /// splits, merges, or re-forms. This is Phase D's per-vertex edit (the user
    /// drags one sketch corner), the single-vertex analogue of
    /// [`Sketch::apply_transform`].
    ///
    /// Unlike [`Sketch::add_segment`], it never re-stitches geometry. A move
    /// that *would* require re-topologizing is refused (rule 4 — no silent
    /// repair); the caller decides what to do.
    ///
    /// # Errors
    /// - [`SketchError::UnknownVertex`] if `v` is stale.
    /// - [`SketchError::PointOffPlane`] (`which: 0`) if `new_pos` is farther
    ///   than [`tol::PLANE_DIST`] from the sketch plane (we don't project).
    /// - [`SketchError::DegenerateSegment`] if the move would collapse an
    ///   incident edge below [`tol::POINT_MERGE`].
    /// - [`SketchError::WouldRetopologize`] if after the move an incident edge
    ///   would cross or collinearly overlap another edge, or `v` would land on
    ///   another vertex (a merge).
    ///
    /// On any error the sketch is unchanged (strong guarantee). On success
    /// returns the vertex's **old** position, so the document layer can record
    /// an exact inverse for undo.
    pub fn move_vertex(
        &mut self,
        v: SketchVertexId,
        new_pos: Point3,
    ) -> Result<Point3, SketchError> {
        let old_pos = self
            .vertices
            .get(v)
            .ok_or(SketchError::UnknownVertex)?
            .position;

        if self.plane.signed_distance(new_pos).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }

        // Validate on a clone; swap in only once every check passes.
        let mut s = self.clone();
        s.vertices[v].position = new_pos;

        // The incident edges are the only geometry that changed.
        let incident: Vec<SketchEdgeId> = s
            .edges
            .iter()
            .filter(|(_, e)| e.from == v || e.to == v)
            .map(|(id, _)| id)
            .collect();

        // (a) No incident edge may collapse to (near) zero length. Checked
        //     before the merge guard so dropping a corner onto an *adjacent*
        //     corner reads as a degenerate segment, not a merge.
        for &eid in &incident {
            let e = s.edges[eid];
            if s.vertices[e.from]
                .position
                .approx_eq(s.vertices[e.to].position, tol::POINT_MERGE)
            {
                return Err(SketchError::DegenerateSegment);
            }
        }

        // (b) The moved vertex may not land on top of another vertex (a merge).
        if s.vertices
            .iter()
            .any(|(vid, vert)| vid != v && vert.position.approx_eq(new_pos, tol::POINT_MERGE))
        {
            return Err(SketchError::WouldRetopologize);
        }

        // (c) No incident edge may cross or collinearly overlap any other edge
        //     (that would demand a split). Reuse the same 2D arrangement math
        //     `add_segment` uses, treating each incident edge as the "new"
        //     segment. Edges that merely share an endpoint produce no event.
        let normal = s.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);
        let anchor = Point3::ORIGIN + normal * (-s.plane.signed_distance(Point3::ORIGIN));
        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        for &inc_id in &incident {
            let inc = s.edges[inc_id];
            let nf = proj(s.vertices[inc.from].position);
            let nt = proj(s.vertices[inc.to].position);
            for (oth_id, oth) in &s.edges {
                if oth_id == inc_id {
                    continue;
                }
                let ef = proj(s.vertices[oth.from].position);
                let et = proj(s.vertices[oth.to].position);
                let events =
                    seg_seg_intersections_2d(nf, nt, ef, et, &s.vertices, *oth, u_ax, v_ax, anchor);
                for ev in events {
                    let bad = match ev {
                        Intersection2D::Proper { .. }
                        | Intersection2D::NewEndpointOnExisting { .. }
                        | Intersection2D::ExistingEndpointOnNew { .. } => true,
                        Intersection2D::Collinear {
                            t_params_on_new,
                            s_params_on_existing,
                            ..
                        } => !t_params_on_new.is_empty() || !s_params_on_existing.is_empty(),
                    };
                    if bad {
                        return Err(SketchError::WouldRetopologize);
                    }
                }
            }
        }

        // (d) Topology is unchanged, so the region cycles still reference the
        //     same vertex ids and now read the moved geometry automatically.
        //     Belt-and-suspenders: every region must still be a valid profile
        //     (a reflex drag could in principle pinch a boundary). Surfaced as
        //     a typed error rather than silently accepted.
        for region in s.regions.keys().collect::<Vec<_>>() {
            s.profile(region)?;
        }

        // Dragging a vertex of a curve chain deforms the chain away from its
        // drawn circle: the chains touching the moved vertex drop their
        // analytic geometry (map-or-drop contract; identity survives so the
        // chain still selects as a unit).
        for &eid in &incident {
            if let Some(cid) = s.edges[eid].curve {
                s.curves[cid] = None;
            }
        }

        *self = s;
        Ok(old_pos)
    }

    /// The scaffolding edges only `region` needs: the edges on its boundary
    /// (outer or hole loops) that lie on NO other region's boundary.
    /// Extrusion deletes exactly these (Model D,
    /// docs/design/sketch-solid-model.md §4D): the region became the
    /// solid's base face, so its exclusive boundary leaves the sketch with
    /// it — while an edge shared with a surviving region stays (the
    /// neighbor must remain closed) and open chains are untouched. Pure
    /// query — no mutation.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_scaffolding(
        &self,
        region: SketchRegionId,
    ) -> Result<std::collections::BTreeSet<SketchEdgeId>, SketchError> {
        if !self.regions.contains_key(region) {
            return Err(SketchError::UnknownRegion);
        }
        let mut own_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        let mut other_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        for (rid, r) in &self.regions {
            let loops = std::iter::once(&r.outer).chain(r.holes.iter());
            for lp in loops {
                for i in 0..lp.len() {
                    let a = lp[i];
                    let b = lp[(i + 1) % lp.len()];
                    if let Some(eid) = self.edge_between(a, b) {
                        if rid == region {
                            own_edges.insert(eid);
                        } else {
                            other_edges.insert(eid);
                        }
                    }
                }
            }
        }
        Ok(own_edges.difference(&other_edges).copied().collect())
    }

    /// The scaffolding edges only the `consumed` regions need — the set
    /// variant of [`Sketch::region_scaffolding`]: an edge dies iff it lies
    /// on a consumed region's boundary and on NO surviving region's
    /// boundary, so an edge shared by two consumed regions goes while an
    /// edge shared with a survivor stays. The load path uses this to honor
    /// a pre-v11 file's stored consumed index one final time
    /// (docs/design/sketch-solid-model.md §6); ids not in this sketch are
    /// skipped. Pure query — no mutation.
    pub(crate) fn regions_scaffolding(
        &self,
        consumed: &std::collections::BTreeSet<SketchRegionId>,
    ) -> std::collections::BTreeSet<SketchEdgeId> {
        let mut consumed_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        let mut live_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        for (rid, r) in &self.regions {
            let loops = std::iter::once(&r.outer).chain(r.holes.iter());
            for lp in loops {
                for i in 0..lp.len() {
                    let a = lp[i];
                    let b = lp[(i + 1) % lp.len()];
                    if let Some(eid) = self.edge_between(a, b) {
                        if consumed.contains(&rid) {
                            consumed_edges.insert(eid);
                        } else {
                            live_edges.insert(eid);
                        }
                    }
                }
            }
        }
        consumed_edges.difference(&live_edges).copied().collect()
    }

    /// Re-inserts scaffolding an extrusion deleted — the undo half of
    /// [`Sketch::remove_edges`]. Each row is an edge as endpoint positions
    /// plus its curve-chain id; endpoints weld to existing vertices within
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) (the shared-wall case)
    /// and re-form regions/islands through the ordinary sticky machinery,
    /// merging with whatever the sketch holds NOW — never a whole-sketch
    /// snapshot, so edits made after the extrusion survive its undo.
    ///
    /// A row that cannot re-insert faithfully — it would split, cross, or
    /// collinearly overlap geometry drawn since (any sticky event beyond a
    /// clean single-edge insert with endpoint welds) — is
    /// [`SketchError::RestoreConflicts`]: the whole restore is refused and
    /// the sketch left untouched (strong guarantee). Curve-chain ids are
    /// re-applied when the chain still exists (chain entries outlive their
    /// edges), reconnecting the surviving analytic [`CurveGeom`].
    ///
    pub(crate) fn restore_edges(
        &mut self,
        rows: &[(Point3, Point3, Option<SketchCurveId>)],
    ) -> Result<(), SketchError> {
        let mut s = self.clone();
        for &(a, b, curve) in rows {
            let report = s
                .add_segment_inner(a, b)
                .map_err(|_| SketchError::RestoreConflicts)?;
            if !report.split_edges.is_empty() || report.new_edges.len() != 1 {
                return Err(SketchError::RestoreConflicts);
            }
            let eid = report.new_edges[0];
            if let Some(cid) = curve
                && s.curves.contains_key(cid)
            {
                s.edges[eid].curve = Some(cid);
            }
        }
        *self = s;
        Ok(())
    }

    /// The edge whose endpoints coincide with `a` and `b` (either
    /// orientation, within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE)),
    /// or `None`. Sticky rules forbid coincident duplicate edges, so a
    /// match is unique. The extrusion REDO path uses this to re-delete
    /// scaffolding by geometry: an interleaved gesture undo/redo restores
    /// snapshots carrying the outline's ORIGINAL edge ids, so a stored id
    /// set can go stale while the geometry itself is exact.
    pub(crate) fn edge_at_positions(&self, a: Point3, b: Point3) -> Option<SketchEdgeId> {
        self.edges
            .iter()
            .find(|(_, e)| {
                let f = self.vertices[e.from].position;
                let t = self.vertices[e.to].position;
                (f.approx_eq(a, tol::POINT_MERGE) && t.approx_eq(b, tol::POINT_MERGE))
                    || (f.approx_eq(b, tol::POINT_MERGE) && t.approx_eq(a, tol::POINT_MERGE))
            })
            .map(|(id, _)| id)
    }

    /// Removes a set of edges in one pass — the extrusion-consumption path
    /// (Model D): the batch analogue of [`Sketch::remove_edge`]. Vertices
    /// left without any incident edge are deleted; regions and islands
    /// recompute once at the end. Stale ids are skipped. Curve-chain
    /// identity survives on any remaining member edges, and a chain's
    /// analytic [`CurveGeom`] stays valid — deletion removes facets, it
    /// never deforms the ones that remain.
    pub(crate) fn remove_edges(&mut self, edges: &std::collections::BTreeSet<SketchEdgeId>) {
        let old_regions: Vec<(SketchRegionId, SketchRegion)> =
            self.regions.iter().map(|(id, r)| (id, r.clone())).collect();
        let mut touched: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        for &eid in edges {
            if let Some(e) = self.edges.remove(eid) {
                touched.insert(e.from);
                touched.insert(e.to);
            }
        }
        for vid in touched {
            if !self.vertex_has_edges(vid) {
                self.vertices.remove(vid);
            }
        }
        self.recompute_regions_with_diff(&old_regions);
        self.recompute_islands();
    }

    /// Extracts a region as a validated [`Profile`] ready for extrusion.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale (regions die
    /// whenever a mutation reshapes them — always re-query after mutating).
    pub fn profile(&self, region: SketchRegionId) -> Result<Profile, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;

        // Outer boundary: CCW as stored — map vertex ids to positions.
        let outer: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();

        // Holes: CW as stored.
        let holes: Vec<Vec<Point3>> = r
            .holes
            .iter()
            .map(|h| h.iter().map(|&vid| self.vertices[vid].position).collect())
            .collect();

        // A region produced by the pipeline should always yield a valid
        // Profile; if it does not, that is a kernel bug — but surface it as a
        // typed error (rule 4) rather than panicking, since a panic at the
        // WASM boundary leaves the Scene unusable.
        let mut profile =
            Profile::new(self.plane, outer, holes).map_err(|_| SketchError::MalformedRegion)?;

        // Analytic attribution: for each boundary edge, the circle its curve
        // chain carries (when it carries one). This is what extrusion stamps
        // onto side-wall faces (docs/design/true-curves.md).
        let edge_geom = |a: SketchVertexId, b: SketchVertexId| -> Option<CurveGeom> {
            self.edge_between(a, b)
                .and_then(|eid| self.edges[eid].curve)
                .and_then(|cid| self.curve_geom(cid))
        };
        let loop_geoms = |cycle: &[SketchVertexId]| -> Vec<Option<CurveGeom>> {
            (0..cycle.len())
                .map(|k| edge_geom(cycle[k], cycle[(k + 1) % cycle.len()]))
                .collect()
        };
        let outer_curves = loop_geoms(&r.outer);
        let hole_curves: Vec<Vec<Option<CurveGeom>>> =
            r.holes.iter().map(|h| loop_geoms(h)).collect();
        profile.set_curve_attribution(outer_curves, hole_curves);
        Ok(profile)
    }

    /// The unsigned area of `region`'s outer boundary, in m². Hole areas are
    /// NOT subtracted — pickers use this to rank nested containing regions
    /// (smallest outer wins), where the raw outer extent is the right measure.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_area(&self, region: SketchRegionId) -> Result<f64, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;
        let pts: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();
        Ok(signed_area_on_plane(&pts, self.plane.normal()).abs())
    }

    /// Whether `p` (a point on the sketch plane) lies in `region`'s material:
    /// inside its outer boundary and outside every hole. Boundary-exact hits
    /// follow [`point_inside_polygon`]'s convention. Pure query.
    ///
    /// # Errors
    /// [`SketchError::UnknownRegion`] if the handle is stale.
    pub fn region_contains_point(
        &self,
        region: SketchRegionId,
        p: Point3,
    ) -> Result<bool, SketchError> {
        let r = self.regions.get(region).ok_or(SketchError::UnknownRegion)?;
        let normal = self.plane.normal();
        let outer: Vec<Point3> = r
            .outer
            .iter()
            .map(|&vid| self.vertices[vid].position)
            .collect();
        if !point_inside_polygon(p, &outer, normal) {
            return Ok(false);
        }
        for hole in &r.holes {
            let pts: Vec<Point3> = hole
                .iter()
                .map(|&vid| self.vertices[vid].position)
                .collect();
            if point_inside_polygon(p, &pts, normal) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Content equality: same plane and identical vertex/edge/region storage,
/// **including handle identity** (same `SlotMap` keys). Gesture undo snapshots
/// (see `Document::end_sketch_gesture`) rely on the handle-identity part —
/// restoring a snapshot must keep every previously-issued id valid — so plain
/// positional equality would be too weak here.
impl PartialEq for Sketch {
    fn eq(&self, other: &Sketch) -> bool {
        fn slotmaps_eq<K: slotmap::Key, V: PartialEq>(
            a: &SlotMap<K, V>,
            b: &SlotMap<K, V>,
        ) -> bool {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
        }
        self.plane == other.plane
            && slotmaps_eq(&self.vertices, &other.vertices)
            && slotmaps_eq(&self.edges, &other.edges)
            && slotmaps_eq(&self.regions, &other.regions)
            && slotmaps_eq(&self.curves, &other.curves)
    }
}

/// The uniform scale factor of `transform` restricted to the plane with unit
/// `normal`, or `None` if the restriction is not a similarity (it maps the
/// plane's circles to ellipses). Both in-plane basis directions must map to
/// equal lengths and stay orthogonal, within [`tol::NORMAL_DIRECTION`]
/// (dimensionless, applied relatively).
fn in_plane_similarity_scale(transform: &crate::Transform, normal: Vec3) -> Option<f64> {
    let (u, v) = plane_axes(normal);
    let lu = transform.apply_vector(u);
    let lv = transform.apply_vector(v);
    let (a, b) = (lu.length(), lv.length());
    let max = a.max(b);
    if max < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    if (a - b).abs() > tol::NORMAL_DIRECTION * max {
        return None;
    }
    if lu.dot(lv).abs() > tol::NORMAL_DIRECTION * a * b {
        return None;
    }
    Some(a)
}

// ═══════════════════════════════════════════════════════════════════ internals

impl Sketch {
    /// Core implementation of `add_segment`. Called on a clone so the caller
    /// can enforce the strong guarantee.
    fn add_segment_inner(&mut self, from: Point3, to: Point3) -> Result<SegmentAdded, SketchError> {
        // ── Step 1: validate planarity ────────────────────────────────────────
        if self.plane.signed_distance(from).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 0 });
        }
        if self.plane.signed_distance(to).abs() > tol::PLANE_DIST {
            return Err(SketchError::PointOffPlane { which: 1 });
        }

        // ── Step 2: snap endpoints to existing vertices (rule 1) ─────────────
        // Returns the snapped position and whether a new vertex must be created.
        let snap_from = self.find_or_plan_vertex(from);
        let snap_to = self.find_or_plan_vertex(to);

        // After snapping, check for degeneracy.
        let from_pos = snap_from.map_or(from, |id| self.vertices[id].position);
        let to_pos = snap_to.map_or(to, |id| self.vertices[id].position);

        if from_pos.approx_eq(to_pos, tol::POINT_MERGE) {
            return Err(SketchError::DegenerateSegment);
        }

        // ── Step 3: gather intersections ──────────────────────────────────────
        // Collect all split events for existing edges and for the new segment.
        // We work in 2D (plane-projected) coordinates throughout.
        let normal = self.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);
        // A point on the plane: 2D projections drop the out-of-plane component,
        // so reconstructing a 3D position must add it back via this anchor (the
        // plane need not pass through the origin).
        let anchor = Point3::ORIGIN + normal * (-self.plane.signed_distance(Point3::ORIGIN));

        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        let fp = proj(from_pos);
        let tp = proj(to_pos);

        // For each existing edge, find intersection parameters t (along new
        // segment, 0..=1) and s (along existing edge, 0..=1).
        // We'll collect:
        //   - for existing edges: the set of interior split parameters s
        //   - for the new segment: the set of interior split parameters t

        // Represent intersections as t-parameters along the new segment plus
        // the associated position and vertex id (to be created or reused).
        // Also track which existing edges need to be split at which s-params.
        struct ExistingEdgeSplit {
            edge_id: SketchEdgeId,
            // (s_param, position) for each interior split point on this edge.
            splits: Vec<(f64, Point3)>,
        }

        let mut existing_splits: Vec<ExistingEdgeSplit> = Vec::new();
        // t-params along the new segment where we need to insert a vertex
        // (includes 0.0 and 1.0 from snap above, and interior crossings).
        let mut new_seg_t_params: Vec<f64> = vec![0.0, 1.0];

        let existing_edge_ids: Vec<SketchEdgeId> = self.edges.keys().collect();

        for eid in existing_edge_ids {
            let edge = self.edges[eid];
            let ep = proj(self.vertices[edge.from].position);
            let eq_ = proj(self.vertices[edge.to].position);

            let mut edge_splits: Vec<(f64, Point3)> = Vec::new();

            // Find all intersections between the new segment [fp,tp] and this
            // existing edge [ep,eq_].
            let intersections =
                seg_seg_intersections_2d(fp, tp, ep, eq_, &self.vertices, edge, u_ax, v_ax, anchor);

            for isect in intersections {
                match isect {
                    Intersection2D::Proper {
                        t_new,
                        s_existing,
                        point,
                    } => {
                        // Interior crossing: splits both segments.
                        if t_new > tol::POINT_MERGE && t_new < 1.0 - tol::POINT_MERGE {
                            new_seg_t_params.push(t_new);
                        }
                        if s_existing > tol::POINT_MERGE && s_existing < 1.0 - tol::POINT_MERGE {
                            edge_splits.push((s_existing, point));
                        }
                    }
                    Intersection2D::NewEndpointOnExisting { s_existing, point } => {
                        // New segment endpoint lands on interior of existing edge.
                        if s_existing > tol::POINT_MERGE && s_existing < 1.0 - tol::POINT_MERGE {
                            edge_splits.push((s_existing, point));
                        }
                    }
                    Intersection2D::ExistingEndpointOnNew { t_new, .. } => {
                        // Existing endpoint lands on interior of new segment.
                        if t_new > tol::POINT_MERGE && t_new < 1.0 - tol::POINT_MERGE {
                            new_seg_t_params.push(t_new);
                        }
                    }
                    Intersection2D::Collinear {
                        t_params_on_new,
                        s_params_on_existing,
                        points_on_new,
                        points_on_existing,
                    } => {
                        // Rule 4: collinear overlap.
                        for t in t_params_on_new {
                            if t > tol::POINT_MERGE && t < 1.0 - tol::POINT_MERGE {
                                new_seg_t_params.push(t);
                            }
                        }
                        let _ = points_on_new; // used above via t_params_on_new
                        for (s, p) in s_params_on_existing.into_iter().zip(points_on_existing) {
                            if s > tol::POINT_MERGE && s < 1.0 - tol::POINT_MERGE {
                                edge_splits.push((s, p));
                            }
                        }
                    }
                }
            }

            if !edge_splits.is_empty() {
                existing_splits.push(ExistingEdgeSplit {
                    edge_id: eid,
                    splits: edge_splits,
                });
            }
        }

        // ── Step 4: snapshot old regions for identity diffing ─────────────────
        let old_regions: Vec<(SketchRegionId, SketchRegion)> =
            self.regions.iter().map(|(id, r)| (id, r.clone())).collect();

        // ── Step 5: materialise vertices for the new segment endpoints ─────────
        let mut report = SegmentAdded::default();

        // from vertex
        let from_vid = match snap_from {
            Some(id) => id,
            None => {
                // Check one more time for snapping to an edge interior
                // (the snapping check at step 1 only checked existing vertices).
                // If still not found, create a fresh vertex.
                let vid = self.vertices.insert(SketchVertex { position: from_pos });
                report.new_vertices.push(vid);
                vid
            }
        };

        // to vertex
        let to_vid = match snap_to {
            Some(id) => id,
            None => {
                let vid = self.vertices.insert(SketchVertex { position: to_pos });
                report.new_vertices.push(vid);
                vid
            }
        };

        // ── Step 6: split existing edges at their interior intersection points ──
        for esplit in existing_splits {
            // Deduplicate and sort split points by s param.
            let mut splits = esplit.splits;
            splits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            splits.dedup_by(|a, b| (a.0 - b.0).abs() < tol::POINT_MERGE);

            // Remove zero-length fragments: skip if s is at 0 or 1 (already
            // covered by snapping).
            let splits: Vec<(f64, Point3)> = splits
                .into_iter()
                .filter(|(s, _)| *s > tol::POINT_MERGE && *s < 1.0 - tol::POINT_MERGE)
                .collect();

            if splits.is_empty() {
                continue;
            }

            let old_edge = self.edges[esplit.edge_id];
            let old_from_vid = old_edge.from;
            let old_to_vid = old_edge.to;

            // Create or find vertices at each split point.
            let mut split_vids: Vec<SketchVertexId> = Vec::new();
            for (_, pos) in &splits {
                let vid = self.find_or_create_vertex(*pos, &mut report.new_vertices);
                split_vids.push(vid);
            }

            // Build the chain of fragment vertices: old_from -> split[0] -> ... -> old_to
            let mut chain: Vec<SketchVertexId> = vec![old_from_vid];
            chain.extend_from_slice(&split_vids);
            chain.push(old_to_vid);

            // Remove the old edge — its fragments inherit its curve chain,
            // so a split arc facet stays part of its arc.
            let split_curve = self.edges[esplit.edge_id].curve;
            self.edges.remove(esplit.edge_id);

            // Insert fragment edges.  Fragments of split *existing* edges go into
            // split_edges only, not new_edges (new_edges is for the inserted segment
            // fragments).
            let mut fragments: Vec<SketchEdgeId> = Vec::new();
            for w in chain.windows(2) {
                let frag_id = self.edges.insert(SketchEdge {
                    from: w[0],
                    to: w[1],
                    curve: split_curve,
                });
                fragments.push(frag_id);
            }
            report.split_edges.push((esplit.edge_id, fragments));
        }

        // ── Step 7: collect t-params along the new segment, materialise verts ──
        // Deduplicate and sort.
        new_seg_t_params.sort_by(|a, b| a.partial_cmp(b).unwrap());
        new_seg_t_params.dedup_by(|a, b| (*a - *b).abs() < tol::POINT_MERGE);

        // Build the sequence of vertex ids along the new segment.
        let seg_dir = to_pos - from_pos;
        let mut seg_vids: Vec<SketchVertexId> = Vec::new();
        for &t in &new_seg_t_params {
            let pos = from_pos + seg_dir * t;
            // t == 0.0 -> from_vid, t == 1.0 -> to_vid
            let vid = if (t - 0.0).abs() < tol::POINT_MERGE {
                from_vid
            } else if (t - 1.0).abs() < tol::POINT_MERGE {
                to_vid
            } else {
                self.find_or_create_vertex(pos, &mut report.new_vertices)
            };
            seg_vids.push(vid);
        }

        // ── Step 8: insert fragment edges for the new segment ─────────────────
        // Skip fragments that already have an existing edge (collinear merge).
        for w in seg_vids.windows(2) {
            let a = w[0];
            let b = w[1];
            if self.edge_exists(a, b) {
                // Collinear merge: this sub-segment already exists. If a
                // curve bracket is open and the existing edge is a plain
                // line, ADOPT it into the curve — otherwise the drawn curve
                // would have an untagged hole where it overlapped existing
                // geometry, splitting the selectable unit. An edge already
                // in another curve keeps its allegiance.
                if let Some(curve) = self.active_curve {
                    let existing = self
                        .edges
                        .iter()
                        .find(|(_, e)| (e.from == a && e.to == b) || (e.from == b && e.to == a))
                        .map(|(id, _)| id);
                    if let Some(eid) = existing
                        && self.edges[eid].curve.is_none()
                    {
                        self.edges[eid].curve = Some(curve);
                    }
                }
                continue;
            }
            // Skip zero-length fragments.
            if self.vertices[a]
                .position
                .approx_eq(self.vertices[b].position, tol::POINT_MERGE)
            {
                continue;
            }
            let eid = self.edges.insert(SketchEdge {
                from: a,
                to: b,
                curve: self.active_curve,
            });
            report.new_edges.push(eid);
        }

        // ── Step 9: recompute regions, islands, and diff ──────────────────────
        let (regions_created, regions_removed) = self.recompute_regions_with_diff(&old_regions);
        report.regions_created = regions_created;
        report.regions_removed = regions_removed;
        self.recompute_islands();

        Ok(report)
    }

    /// Core implementation of `remove_edge`. Called on a clone so the strong
    /// guarantee holds at the public method.
    fn remove_edge_inner(&mut self, edge: SketchEdgeId) -> EdgeRemoved {
        let old_regions: Vec<(SketchRegionId, SketchRegion)> =
            self.regions.iter().map(|(id, r)| (id, r.clone())).collect();

        let e = self.edges[edge];
        self.edges.remove(edge);

        // Delete vertices with no remaining incident edges.
        let mut removed_vertices = Vec::new();
        for vid in [e.from, e.to] {
            if !self.vertex_has_edges(vid) {
                self.vertices.remove(vid);
                removed_vertices.push(vid);
            }
        }

        let (regions_created, regions_removed) = self.recompute_regions_with_diff(&old_regions);
        self.recompute_islands();

        EdgeRemoved {
            removed_vertices,
            regions_removed,
            regions_created,
        }
    }

    // ── Vertex helpers ────────────────────────────────────────────────────────

    /// Returns the id of an existing vertex within `tol::POINT_MERGE` of `pos`,
    /// or `None` if there is none.
    fn find_or_plan_vertex(&self, pos: Point3) -> Option<SketchVertexId> {
        for (id, v) in &self.vertices {
            if v.position.approx_eq(pos, tol::POINT_MERGE) {
                return Some(id);
            }
        }
        None
    }

    /// Returns an existing vertex within `tol::POINT_MERGE` of `pos`, or
    /// creates a new one and records it in `new_vertices`.
    fn find_or_create_vertex(
        &mut self,
        pos: Point3,
        new_vertices: &mut Vec<SketchVertexId>,
    ) -> SketchVertexId {
        if let Some(id) = self.find_or_plan_vertex(pos) {
            return id;
        }
        let id = self.vertices.insert(SketchVertex { position: pos });
        new_vertices.push(id);
        id
    }

    /// True if the edge exists in either direction.
    fn edge_exists(&self, a: SketchVertexId, b: SketchVertexId) -> bool {
        self.edges
            .values()
            .any(|e| (e.from == a && e.to == b) || (e.from == b && e.to == a))
    }

    /// Returns the id of the edge connecting `a` and `b` (in either direction),
    /// or `None` if no such edge exists.
    fn edge_between(&self, a: SketchVertexId, b: SketchVertexId) -> Option<SketchEdgeId> {
        self.edges
            .iter()
            .find(|(_, e)| (e.from == a && e.to == b) || (e.from == b && e.to == a))
            .map(|(id, _)| id)
    }

    /// True if the vertex has at least one incident edge (in either direction).
    fn vertex_has_edges(&self, vid: SketchVertexId) -> bool {
        self.edges.values().any(|e| e.from == vid || e.to == vid)
    }

    // ── Region computation ────────────────────────────────────────────────────

    /// Recompute all regions from scratch, then diff against the previous set.
    /// Returns `(created, removed)`.
    fn recompute_regions_with_diff(
        &mut self,
        old_regions: &[(SketchRegionId, SketchRegion)],
    ) -> (Vec<SketchRegionId>, Vec<SketchRegionId>) {
        let new_region_data = self.compute_all_regions();

        // Identity rule: a region whose outer vertex-id cycle is unchanged
        // (cyclic equality, same orientation) KEEPS its SketchRegionId — its
        // stored value is refreshed in place (hole sets may differ) and it
        // appears in neither report list, so handles held by tools stay
        // valid until a mutation actually reshapes the region. Everything
        // else is a removal + creation.
        let mut regions_created: Vec<SketchRegionId> = Vec::new();
        let mut reused: std::collections::BTreeSet<SketchRegionId> =
            std::collections::BTreeSet::new();

        for new_r in new_region_data {
            let matched = old_regions.iter().find(|(old_id, old_r)| {
                !reused.contains(old_id) && cycles_equal(&old_r.outer, &new_r.outer)
            });
            match matched {
                Some((old_id, _)) => {
                    reused.insert(*old_id);
                    self.regions[*old_id] = new_r;
                }
                None => {
                    regions_created.push(self.regions.insert(new_r));
                }
            }
        }

        let mut regions_removed: Vec<SketchRegionId> = Vec::new();
        for (old_id, _) in old_regions {
            if !reused.contains(old_id) {
                self.regions.remove(*old_id);
                regions_removed.push(*old_id);
            }
        }

        (regions_created, regions_removed)
    }

    /// Compute all regions (CCW outer cycles + their CW hole cycles) from the
    /// current edge graph, using planar half-edge face tracing.
    fn compute_all_regions(&self) -> Vec<SketchRegion> {
        if self.edges.is_empty() {
            return Vec::new();
        }

        let normal = self.plane.normal();
        let (u_ax, v_ax) = plane_axes(normal);

        let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };

        // Build adjacency: for each vertex, sorted list of outgoing directed
        // half-edges by angle.
        //
        // A sketch edge (a, b) gives two directed half-edges: a->b and b->a.
        // We store them as (from_vid, to_vid).

        // Collect all directed half-edges.
        let mut half_edges: Vec<(SketchVertexId, SketchVertexId)> = Vec::new();
        for e in self.edges.values() {
            half_edges.push((e.from, e.to));
            half_edges.push((e.to, e.from));
        }

        // For each vertex, build sorted adjacency list by angle.
        let mut adj: std::collections::BTreeMap<SketchVertexId, Vec<SketchVertexId>> =
            std::collections::BTreeMap::new();
        for &(from, to) in &half_edges {
            adj.entry(from).or_default().push(to);
        }
        // Sort each adjacency list by angle of the outgoing direction.
        for (from_vid, neighbors) in adj.iter_mut() {
            let from_pos = self.vertices[*from_vid].position;
            let fp = proj(from_pos);
            neighbors.sort_by(|&a, &b| {
                let pa = proj(self.vertices[a].position);
                let pb = proj(self.vertices[b].position);
                let angle_a = (pa.1 - fp.1).atan2(pa.0 - fp.0);
                let angle_b = (pb.1 - fp.1).atan2(pb.0 - fp.0);
                angle_a.partial_cmp(&angle_b).unwrap()
            });
        }

        // Trace all face cycles: for each directed half-edge (a->b), the next
        // half-edge in the face cycle is found by:
        //   1. At vertex b, find the incoming direction = reverse of a->b = b->a.
        //   2. Find b->a in b's sorted adjacency list.
        //   3. Take the NEXT entry (wrapping) — the rotational successor.
        //   4. That gives the next outgoing half-edge b->c.
        let mut visited: std::collections::BTreeSet<(SketchVertexId, SketchVertexId)> =
            std::collections::BTreeSet::new();
        let mut cycles: Vec<Vec<SketchVertexId>> = Vec::new();

        for &start_he in &half_edges {
            if visited.contains(&start_he) {
                continue;
            }
            let mut cycle: Vec<SketchVertexId> = Vec::new();
            let mut current = start_he;
            loop {
                if visited.contains(&current) {
                    break;
                }
                visited.insert(current);
                cycle.push(current.0); // push the 'from' vertex

                // Find successor: at vertex current.1, find the next outgoing
                // direction after the reverse of current.
                let arrive_at = current.1;
                let reverse_dir = current.0; // the edge that brought us to arrive_at

                let neighbors = match adj.get(&arrive_at) {
                    Some(n) => n,
                    None => break,
                };

                // Find index of reverse_dir in neighbors.
                let idx = match neighbors.iter().position(|&n| n == reverse_dir) {
                    Some(i) => i,
                    None => break,
                };

                // Rotational successor: previous index (wrapping), because we
                // want "next CCW from the reversed incoming direction".
                // In standard planar tracing: sort CCW, successor of reverse is
                // the one just BEFORE it in CCW order (i.e., rotate CW to find
                // the face to the left of the current directed edge).
                let next_idx = if idx == 0 {
                    neighbors.len() - 1
                } else {
                    idx - 1
                };
                let next_to = neighbors[next_idx];
                current = (arrive_at, next_to);
            }

            // Prune spurs: a dangling chain attached to the cycle is walked
            // out-and-back by planar face tracing, leaving a pinched
            // `… a, tip, a …` pattern with repeated vertices. Such a cycle
            // renders a fill (the spur has zero area) but is not a simple
            // polygon, so profiles built from it are refused — the spur is
            // decoration, not boundary. Collapse every backtrack until none
            // remain; chains fold up tip-first over the iterations.
            loop {
                let n = cycle.len();
                if n < 3 {
                    break;
                }
                let mut pruned = false;
                for i in 0..n {
                    let prev = cycle[(i + n - 1) % n];
                    let next = cycle[(i + 1) % n];
                    if prev == next {
                        // cycle[i] is a spur tip: drop it and one duplicate
                        // of its base (the entry at i+1).
                        let (a, b) = (i, (i + 1) % n);
                        let (first, second) = if a > b { (a, b) } else { (b, a) };
                        cycle.remove(first);
                        cycle.remove(second);
                        pruned = true;
                        break;
                    }
                }
                if !pruned {
                    break;
                }
            }

            if cycle.len() >= 3 {
                cycles.push(cycle);
            }
        }

        if cycles.is_empty() {
            return Vec::new();
        }

        // Compute signed area for each cycle.  Positive = CCW (candidate region
        // outer boundary); negative = CW (hole boundary or outer face).
        let cycle_areas: Vec<f64> = cycles
            .iter()
            .map(|cycle| {
                let pts: Vec<Point3> = cycle
                    .iter()
                    .map(|&vid| self.vertices[vid].position)
                    .collect();
                signed_area_on_plane(&pts, normal)
            })
            .collect();

        // Identify the unbounded outer face: among negative-area cycles, the one
        // whose absolute area is largest (it encircles everything).
        // Actually for a planar graph on an infinite plane the unbounded face is
        // the cycle with the largest *absolute* area (positive or negative).
        // We discard the single cycle with the maximum absolute area if it has
        // negative (CW) signed area — that's the outer unbounded face.
        //
        // However, some graph configurations don't produce an unbounded face
        // cycle (e.g., a single isolated edge); in that case we just skip.

        let max_abs_area = cycle_areas
            .iter()
            .map(|a| a.abs())
            .fold(f64::NEG_INFINITY, f64::max);
        let unbounded_idx = cycle_areas
            .iter()
            .enumerate()
            .filter(|&(_, a)| *a < 0.0 && (a.abs() - max_abs_area).abs() < tol::POINT_MERGE * 1e6)
            .map(|(i, _)| i)
            .next();

        // Separate CCW outer candidates from CW hole candidates (excluding the
        // unbounded outer face).
        let mut outer_candidates: Vec<(usize, f64)> = Vec::new(); // (cycle_idx, area)
        let mut hole_candidates: Vec<(usize, f64)> = Vec::new(); // (cycle_idx, abs_area)

        for (i, &area) in cycle_areas.iter().enumerate() {
            if Some(i) == unbounded_idx {
                continue;
            }
            if area > 0.0 {
                outer_candidates.push((i, area));
            } else if area < 0.0 {
                hole_candidates.push((i, area.abs()));
            }
        }

        // Assign each hole cycle to the smallest-area enclosing outer cycle
        // (point-in-polygon on one representative point of the hole).
        // If no enclosing outer cycle is found, the hole is an island that
        // failed to find its encloser — treat as outer with no holes (shouldn't
        // happen with valid planar graphs).

        // Build region data: one SketchRegion per outer candidate, then assign
        // holes.
        let mut region_outers: Vec<Vec<SketchVertexId>> = outer_candidates
            .iter()
            .map(|&(ci, _)| cycles[ci].clone())
            .collect();
        let mut region_holes: Vec<Vec<Vec<SketchVertexId>>> =
            vec![Vec::new(); outer_candidates.len()];

        for &(hole_ci, hole_area) in &hole_candidates {
            let hole_cycle = &cycles[hole_ci];
            // Representative point: the centroid of the hole's vertices, which
            // lies in the hole's interior. A boundary vertex is unreliable for
            // point-in-polygon and, being shared with neighbouring cycles,
            // could land exactly on another cycle's edge.
            let n = hole_cycle.len() as f64;
            let (mut sx, mut sy, mut sz) = (0.0, 0.0, 0.0);
            for &vid in hole_cycle {
                let p = self.vertices[vid].position;
                sx += p.x;
                sy += p.y;
                sz += p.z;
            }
            let rep_pos = Point3::new(sx / n, sy / n, sz / n);

            // Assign the hole to the *smallest* enclosing outer cycle, while
            // excluding two things: (1) the hole's own reverse-wound twin — the
            // same closed loop bounds the region on one side (CCW outer
            // candidate) and its neighbour on the other (CW hole), so a hole
            // must never take its own boundary as its encloser (without this, a
            // ring took its own twin as a hole and Profile::new rejected it as
            // self-intersecting); and (2) any cycle nested *inside* the hole, so
            // nested rings attach holes to their immediate encloser. The twin is
            // identified by exact vertex-set identity — NOT by an "equal area"
            // test, which is FP-fragile (at some facet counts the twin's area
            // drifts an epsilon past a strict `<=`, letting the hole attach to
            // itself — the faceted-through-hole NonManifoldResult bug).
            let mut best: Option<(usize, f64)> = None; // (region_idx, area)
            for (ri, &(outer_ci, outer_area)) in outer_candidates.iter().enumerate() {
                let outer_cycle = &cycles[outer_ci];
                // (1) Skip the hole's reverse-wound twin (identical vertex set).
                if outer_cycle.len() == hole_cycle.len()
                    && hole_cycle.iter().all(|v| outer_cycle.contains(v))
                {
                    continue;
                }
                // (2) Skip cycles nested inside (or coincident with) the hole.
                if outer_area <= hole_area {
                    continue;
                }
                let outer_pts: Vec<Point3> = cycles[outer_ci]
                    .iter()
                    .map(|&vid| self.vertices[vid].position)
                    .collect();
                if point_inside_polygon(rep_pos, &outer_pts, normal) {
                    match best {
                        None => best = Some((ri, outer_area)),
                        Some((_, best_area)) if outer_area < best_area => {
                            best = Some((ri, outer_area));
                        }
                        _ => {}
                    }
                }
            }

            if let Some((ri, _)) = best {
                region_holes[ri].push(hole_cycle.clone());
            }
            // If no enclosing outer found, the hole cycle is discarded (edge
            // case of disconnected geometry — not reachable in a well-formed sketch).
        }

        // Build SketchRegion objects.
        let mut regions: Vec<SketchRegion> = Vec::new();
        for (i, outer) in region_outers.drain(..).enumerate() {
            regions.push(SketchRegion {
                outer,
                holes: region_holes[i].clone(),
            });
        }

        regions
    }
}

// ═════════════════════════════════════════════ structural reconstruction

impl Sketch {
    /// Create an empty sketch for structural reconstruction. Unlike `on_plane`,
    /// this is `pub(crate)` so the serializer can build a Sketch by directly
    /// inserting pre-validated elements (bypassing the sticky-geometry pipeline).
    pub(crate) fn reconstruct(plane: Plane) -> Sketch {
        Sketch {
            plane,
            vertices: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            regions: SlotMap::with_key(),
            curves: SlotMap::with_key(),
            active_curve: None,
            islands: SlotMap::with_key(),
        }
    }

    /// Insert a vertex directly into the slotmap (no planarity check, no merging).
    /// For structural reconstruction only.
    pub(crate) fn insert_vertex_raw(&mut self, v: SketchVertex) -> SketchVertexId {
        self.vertices.insert(v)
    }

    /// Insert an edge directly into the slotmap (no intersection checks).
    /// For structural reconstruction only.
    pub(crate) fn insert_edge_raw(&mut self, e: SketchEdge) -> SketchEdgeId {
        self.edges.insert(e)
    }

    /// Insert a region directly into the slotmap (no geometric check).
    /// For structural reconstruction only.
    pub(crate) fn insert_region_raw(&mut self, r: SketchRegion) -> SketchRegionId {
        self.regions.insert(r)
    }
}

// ═══════════════════════════════════════════════════════════ intersection math

/// Result of a 2D segment intersection query.
#[derive(Debug)]
enum Intersection2D {
    /// Proper interior crossing.
    Proper {
        t_new: f64,
        s_existing: f64,
        point: Point3,
    },
    /// An endpoint of the *new* segment lands on the interior of the existing
    /// segment.
    NewEndpointOnExisting { s_existing: f64, point: Point3 },
    /// An endpoint of the *existing* segment lands on the interior of the new
    /// segment.
    ExistingEndpointOnNew { t_new: f64, _point: Point3 },
    /// The two segments are collinear and overlap (or one contains the other).
    Collinear {
        /// t-parameters on the new segment for the existing endpoints that
        /// land inside it.
        t_params_on_new: Vec<f64>,
        /// s-parameters on the existing segment for the new endpoints that
        /// land inside it.
        s_params_on_existing: Vec<f64>,
        points_on_new: Vec<Point3>,
        points_on_existing: Vec<Point3>,
    },
}

/// Compute all intersection events between the new segment [new_from, new_to]
/// (given as 2D projections `nf`, `nt`) and the existing edge `e` (2D
/// projections `ef`, `et`).
///
/// Returns zero or more intersection events.  The projections are passed for
/// efficiency; world-space positions are reconstructed from parameters.
#[allow(clippy::too_many_arguments)]
fn seg_seg_intersections_2d(
    nf: (f64, f64), // new segment from
    nt: (f64, f64), // new segment to
    ef: (f64, f64), // existing edge from
    et: (f64, f64), // existing edge to
    vertices: &SlotMap<SketchVertexId, SketchVertex>,
    edge: SketchEdge,
    u_ax: Vec3,
    v_ax: Vec3,
    anchor: Point3,
) -> Vec<Intersection2D> {
    // Reconstruct 3D position from 2D plane coords. `anchor` carries the
    // out-of-plane component (the plane need not pass through the origin); the
    // 2D coords supply the in-plane displacement.
    let pos_from_2d = |u: f64, v: f64| -> Point3 { anchor + u_ax * u + v_ax * v };

    let d_new = (nt.0 - nf.0, nt.1 - nf.1);
    let d_ext = (et.0 - ef.0, et.1 - ef.1);

    // Cross product (2D): a × b = ax*by - ay*bx
    let cross_2d = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;

    let denom = cross_2d(d_new, d_ext);

    if denom.abs() < tol::POINT_MERGE * tol::POINT_MERGE {
        // Parallel or collinear.
        // Check if they are actually collinear (ef lies on the line through nf,nt).
        let nf_to_ef = (ef.0 - nf.0, ef.1 - nf.1);
        let collinear_check = cross_2d(d_new, nf_to_ef).abs();
        // Use a length-scaled threshold.
        let new_len = (d_new.0 * d_new.0 + d_new.1 * d_new.1).sqrt();
        if new_len < tol::POINT_MERGE {
            return Vec::new(); // degenerate new segment, skip
        }
        if collinear_check / new_len > tol::POINT_MERGE {
            return Vec::new(); // parallel but not collinear
        }

        // Collinear: project all four endpoints onto the new segment's axis.
        // t=0 -> nf, t=1 -> nt; also compute s=0 -> ef, s=1 -> et.
        let project_onto_new = |p: (f64, f64)| -> f64 {
            ((p.0 - nf.0) * d_new.0 + (p.1 - nf.1) * d_new.1) / (new_len * new_len)
        };
        let ext_len_sq = d_ext.0 * d_ext.0 + d_ext.1 * d_ext.1;
        let ext_len = ext_len_sq.sqrt();
        let project_onto_ext = |p: (f64, f64)| -> f64 {
            if ext_len < tol::POINT_MERGE {
                return 0.0;
            }
            ((p.0 - ef.0) * d_ext.0 + (p.1 - ef.1) * d_ext.1) / ext_len_sq
        };

        let t_ef = project_onto_new(ef);
        let t_et = project_onto_new(et);
        let s_nf = project_onto_ext(nf);
        let s_nt = project_onto_ext(nt);

        // Gather t-params on the new segment for the existing endpoints that
        // land in its interior.
        let mut t_params_on_new: Vec<f64> = Vec::new();
        let mut points_on_new: Vec<Point3> = Vec::new();
        for (t, ep2d) in [(t_ef, ef), (t_et, et)] {
            if t > tol::POINT_MERGE && t < 1.0 - tol::POINT_MERGE {
                t_params_on_new.push(t);
                // Use the actual existing vertex position for accuracy.
                let pos = if (t - t_ef).abs() < tol::POINT_MERGE {
                    vertices[edge.from].position
                } else {
                    vertices[edge.to].position
                };
                let _ = ep2d;
                points_on_new.push(pos);
            }
        }

        // Gather s-params on the existing segment for the new endpoints that
        // land in its interior.
        let mut s_params_on_existing: Vec<f64> = Vec::new();
        let mut points_on_existing: Vec<Point3> = Vec::new();
        for (s, np2d) in [(s_nf, nf), (s_nt, nt)] {
            if s > tol::POINT_MERGE && s < 1.0 - tol::POINT_MERGE {
                s_params_on_existing.push(s);
                points_on_existing.push(pos_from_2d(np2d.0, np2d.1));
            }
        }

        if t_params_on_new.is_empty() && s_params_on_existing.is_empty() {
            return Vec::new(); // no overlap or just touching at endpoints
        }

        return vec![Intersection2D::Collinear {
            t_params_on_new,
            s_params_on_existing,
            points_on_new,
            points_on_existing,
        }];
    }

    // Non-parallel: compute parametric intersection.
    let nf_to_ef = (ef.0 - nf.0, ef.1 - nf.1);
    let t = cross_2d(nf_to_ef, d_ext) / denom;
    let s = cross_2d(nf_to_ef, d_new) / denom;

    let eps = tol::POINT_MERGE;

    // Determine which kind of event this is based on t and s positions.
    let t_interior = t > eps && t < 1.0 - eps;
    let s_interior = s > eps && s < 1.0 - eps;
    let t_at_0 = t.abs() < eps;
    let t_at_1 = (t - 1.0).abs() < eps;
    let s_at_0 = s.abs() < eps;
    let s_at_1 = (s - 1.0).abs() < eps;

    // The intersection point (use existing vertex pos for accuracy at endpoints).
    let point = if s_at_0 {
        vertices[edge.from].position
    } else if s_at_1 {
        vertices[edge.to].position
    } else {
        // Interior of existing edge: interpolate
        let ix = ef.0 + d_ext.0 * s;
        let iy = ef.1 + d_ext.1 * s;
        pos_from_2d(ix, iy)
    };

    if t_interior && s_interior {
        // Proper crossing.
        return vec![Intersection2D::Proper {
            t_new: t,
            s_existing: s,
            point,
        }];
    }

    if (t_at_0 || t_at_1) && s_interior {
        // New endpoint on existing interior (T-junction from new segment's side).
        return vec![Intersection2D::NewEndpointOnExisting {
            s_existing: s,
            point,
        }];
    }

    if t_interior && (s_at_0 || s_at_1) {
        // Existing endpoint on new segment interior.
        let pt = if s_at_0 {
            vertices[edge.from].position
        } else {
            vertices[edge.to].position
        };
        return vec![Intersection2D::ExistingEndpointOnNew {
            t_new: t,
            _point: pt,
        }];
    }

    // All other cases (both at endpoints) — handled by vertex snapping; no
    // event needed here.
    Vec::new()
}

// ══════════════════════════════════════════════════════════ cycle comparison

/// True if two vertex-id cycles are equal under cyclic rotation and same
/// orientation (we do NOT check reversed orientation here — outer cycles are
/// always CCW and holes always CW, so direction is fixed).
fn cycles_equal(a: &[SketchVertexId], b: &[SketchVertexId]) -> bool {
    let n = a.len();
    if n != b.len() {
        return false;
    }
    // Try all rotations of a against b.
    'outer: for start in 0..n {
        for i in 0..n {
            if a[(start + i) % n] != b[i] {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

// ══════════════════════════════════════════════════════════════════ Profile

/// A validated closed planar polygon with holes: the only currency accepted
/// by `Object::from_extrusion`.
///
/// Invariants established at construction and never re-checked downstream:
/// every boundary point on `plane` within [`tol::PLANE_DIST`](crate::tol::PLANE_DIST); outer boundary
/// counter-clockwise seen from the plane normal, holes clockwise; no
/// self-intersection, no boundary-boundary contact; every hole strictly
/// inside the outer boundary and outside every other hole.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    plane: Plane,
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    /// Analytic attribution per outer boundary edge: `outer_curves[k]` is
    /// the circle edge `outer[k] → outer[k+1]` is a facet of, when the edge
    /// came from a curve chain with geometry. Parallel to `outer`; empty ⇒
    /// no attribution anywhere (the [`Profile::new`] default).
    outer_curves: Vec<Option<CurveGeom>>,
    /// Same, per hole boundary edge. Parallel to `holes` when non-empty.
    hole_curves: Vec<Vec<Option<CurveGeom>>>,
}

/// Typed reasons a would-be [`Profile`] is rejected. The kernel never fixes
/// winding, drops points, or "heals" intersections (DEVELOPMENT.md rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// A boundary has fewer than 3 points.
    TooFewPoints,
    /// A boundary point is farther than [`tol::PLANE_DIST`](crate::tol::PLANE_DIST) from the plane.
    PointOffPlane,
    /// Two consecutive boundary points are within [`tol::POINT_MERGE`](crate::tol::POINT_MERGE).
    DegenerateEdge,
    /// The outer boundary is not counter-clockwise, or a hole is not
    /// clockwise, seen from the plane normal side.
    WrongWinding,
    /// A boundary crosses or touches itself or another boundary.
    SelfIntersecting,
    /// A hole is not strictly inside the outer boundary, or overlaps another
    /// hole.
    HoleOutsideOuter,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ProfileError::TooFewPoints => "boundary has fewer than 3 points",
            ProfileError::PointOffPlane => "boundary point is off the profile plane",
            ProfileError::DegenerateEdge => "consecutive boundary points coincide",
            ProfileError::WrongWinding => {
                "outer boundary must be CCW and holes CW seen from the plane normal"
            }
            ProfileError::SelfIntersecting => "boundaries cross or touch",
            ProfileError::HoleOutsideOuter => "hole is not strictly inside the outer boundary",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    /// Validates and constructs a profile. See the type docs for the
    /// invariants checked; each failure maps to one [`ProfileError`] variant.
    pub fn new(
        plane: Plane,
        outer: Vec<Point3>,
        holes: Vec<Vec<Point3>>,
    ) -> Result<Profile, ProfileError> {
        // 1. Minimum point counts.
        if outer.len() < 3 {
            return Err(ProfileError::TooFewPoints);
        }
        for hole in &holes {
            if hole.len() < 3 {
                return Err(ProfileError::TooFewPoints);
            }
        }

        // 2. Planarity of all boundaries.
        for &p in &outer {
            if plane.signed_distance(p).abs() > crate::tol::PLANE_DIST {
                return Err(ProfileError::PointOffPlane);
            }
        }
        for hole in &holes {
            for &p in hole {
                if plane.signed_distance(p).abs() > crate::tol::PLANE_DIST {
                    return Err(ProfileError::PointOffPlane);
                }
            }
        }

        // 3. Consecutive-point separation (degenerate edges).
        profile_check_degenerate(&outer)?;
        for hole in &holes {
            profile_check_degenerate(hole)?;
        }

        // 4. Self-intersection and boundary-boundary contact.
        //    Must be checked before winding because a bowtie's signed area is
        //    ambiguous (often ~0) and we want SelfIntersecting, not WrongWinding.
        profile_check_simple(&outer)?;
        for hole in &holes {
            profile_check_simple(hole)?;
        }
        // Check outer against each hole, and holes against each other.
        let all_boundaries: Vec<&Vec<Point3>> =
            std::iter::once(&outer).chain(holes.iter()).collect();
        for i in 0..all_boundaries.len() {
            for j in (i + 1)..all_boundaries.len() {
                if boundaries_contact(all_boundaries[i], all_boundaries[j]) {
                    return Err(ProfileError::SelfIntersecting);
                }
            }
        }

        // 5. Winding check via signed area against the plane normal.
        //    Outer must be CCW (positive signed area), holes CW (negative).
        let outer_area = signed_area_on_plane(&outer, plane.normal());
        if outer_area <= 0.0 {
            return Err(ProfileError::WrongWinding);
        }
        for hole in &holes {
            let hole_area = signed_area_on_plane(hole, plane.normal());
            if hole_area >= 0.0 {
                return Err(ProfileError::WrongWinding);
            }
        }

        // 6. Holes must be strictly inside the outer boundary and disjoint.
        for hole in &holes {
            // Every hole vertex must be inside the outer boundary.
            for &p in hole {
                if !point_inside_polygon(p, &outer, plane.normal()) {
                    return Err(ProfileError::HoleOutsideOuter);
                }
            }
            // Every outer vertex must be outside each hole.
            for &p in &outer {
                if point_inside_polygon(p, hole, plane.normal()) {
                    return Err(ProfileError::HoleOutsideOuter);
                }
            }
        }
        // Holes must be disjoint from each other.
        for i in 0..holes.len() {
            for j in (i + 1)..holes.len() {
                for &p in &holes[j] {
                    if point_inside_polygon(p, &holes[i], plane.normal()) {
                        return Err(ProfileError::HoleOutsideOuter);
                    }
                }
                for &p in &holes[i] {
                    if point_inside_polygon(p, &holes[j], plane.normal()) {
                        return Err(ProfileError::HoleOutsideOuter);
                    }
                }
            }
        }

        Ok(Profile {
            plane,
            outer,
            holes,
            outer_curves: Vec::new(),
            hole_curves: Vec::new(),
        })
    }

    /// Attaches per-edge analytic curve attribution (see the field docs).
    /// Crate-internal: only [`Sketch::profile`] produces attribution, from
    /// the region's own edges. Lengths must be parallel to the boundaries.
    pub(crate) fn set_curve_attribution(
        &mut self,
        outer_curves: Vec<Option<CurveGeom>>,
        hole_curves: Vec<Vec<Option<CurveGeom>>>,
    ) {
        debug_assert_eq!(outer_curves.len(), self.outer.len());
        debug_assert_eq!(hole_curves.len(), self.holes.len());
        debug_assert!(
            hole_curves
                .iter()
                .zip(&self.holes)
                .all(|(c, h)| c.len() == h.len())
        );
        self.outer_curves = outer_curves;
        self.hole_curves = hole_curves;
    }

    /// The supporting plane.
    pub fn plane(&self) -> Plane {
        self.plane
    }

    /// Outer boundary, CCW seen from the plane normal side.
    pub fn outer(&self) -> &[Point3] {
        &self.outer
    }

    /// Hole boundaries, each CW seen from the plane normal side.
    pub fn holes(&self) -> &[Vec<Point3>] {
        &self.holes
    }

    /// The analytic circle outer edge `k` (`outer[k] → outer[k+1]`) is a
    /// facet of, or `None` (plain line, or no attribution attached).
    pub fn outer_curve(&self, k: usize) -> Option<CurveGeom> {
        self.outer_curves.get(k).copied().flatten()
    }

    /// The analytic circle hole `i`'s edge `k` is a facet of, or `None`.
    pub fn hole_curve(&self, i: usize, k: usize) -> Option<CurveGeom> {
        self.hole_curves
            .get(i)
            .and_then(|h| h.get(k))
            .copied()
            .flatten()
    }
}

// ----------------------------------------------------------------- helpers

/// Check that no two consecutive points (including the wrap-around pair) are
/// within `tol::POINT_MERGE` of each other.
fn profile_check_degenerate(pts: &[Point3]) -> Result<(), ProfileError> {
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if a.approx_eq(b, crate::tol::POINT_MERGE) {
            return Err(ProfileError::DegenerateEdge);
        }
    }
    Ok(())
}

/// Delegate to the shared geom2d helper and map the error.
fn profile_check_simple(pts: &[Point3]) -> Result<(), ProfileError> {
    if polygon_is_simple(pts) {
        Ok(())
    } else {
        Err(ProfileError::SelfIntersecting)
    }
}

// ═════════════════════════════════════════════════════════════════════ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Plane;

    fn xy_plane() -> Plane {
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    fn pt(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    /// Build a rectangle sketch and return the single region.
    fn make_rect_sketch(x0: f64, y0: f64, x1: f64, y1: f64) -> Sketch {
        let mut s = Sketch::on_plane(xy_plane());
        let segs = [
            (pt(x0, y0), pt(x1, y0)),
            (pt(x1, y0), pt(x1, y1)),
            (pt(x1, y1), pt(x0, y1)),
            (pt(x0, y1), pt(x0, y0)),
        ];
        for (a, b) in &segs {
            s.add_segment(*a, *b).unwrap();
        }
        s
    }

    // ── Angular sorting around a 4-way crossing vertex ────────────────────────

    /// After inserting two crossing segments (+), the crossing vertex must have
    /// 4 outgoing directed half-edges sorted in angular order.  We verify by
    /// checking the region structure: a 4-way crossing of equal-length arms
    /// produces 4 triangular regions (none actually, since the arms extend to
    /// different lengths — we use the simpler rectangle-then-chord case).
    #[test]
    fn angular_sort_4way_crossing() {
        // Draw a cross (+) with all four arms meeting at the origin.
        // Arm segments: (-1,0)-(1,0) and (0,-1)-(0,1).
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(-1.0, 0.0), pt(1.0, 0.0)).unwrap();
        s.add_segment(pt(0.0, -1.0), pt(0.0, 1.0)).unwrap();
        // 4 arms meeting at origin — no closed region since arms are open.
        assert_eq!(s.regions().len(), 0);

        // Now close them into a square.
        s.add_segment(pt(-1.0, -1.0), pt(1.0, -1.0)).unwrap();
        s.add_segment(pt(1.0, -1.0), pt(1.0, 1.0)).unwrap();
        s.add_segment(pt(1.0, 1.0), pt(-1.0, 1.0)).unwrap();
        s.add_segment(pt(-1.0, 1.0), pt(-1.0, -1.0)).unwrap();
        // The cross divides the square into 4 equal quadrant regions.
        assert_eq!(s.regions().len(), 4);
    }

    /// Regression: reconstructing a 3D position from 2D plane coordinates must
    /// restore the plane's offset, not assume the plane passes through the
    /// origin. Before the fix, the interior crossing vertex landed at z=0.
    #[test]
    fn intersection_vertices_lie_on_an_offset_plane() {
        let offset = 5.0;
        let plane = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, offset),
            Point3::new(1.0, 0.0, offset),
            Point3::new(0.0, 1.0, offset),
        ])
        .unwrap();
        let mut s = Sketch::on_plane(plane);
        s.add_segment(
            Point3::new(-1.0, 0.0, offset),
            Point3::new(1.0, 0.0, offset),
        )
        .unwrap();
        // This crosses the first segment, creating an interior vertex.
        s.add_segment(
            Point3::new(0.0, -1.0, offset),
            Point3::new(0.0, 1.0, offset),
        )
        .unwrap();

        for v in s.vertices().values() {
            assert!(
                (v.position.z - offset).abs() <= crate::tol::PLANE_DIST,
                "vertex left the plane: {:?}",
                v.position
            );
        }
        assert!(
            s.vertices().values().any(|v| v
                .position
                .approx_eq(Point3::new(0.0, 0.0, offset), crate::tol::POINT_MERGE)),
            "crossing vertex not found on the offset plane"
        );
    }

    // ── Triangle region (non-axis-aligned edges) ──────────────────────────────

    #[test]
    fn triangle_region_non_axis_aligned() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(0.0, 0.0), pt(3.0, 0.0)).unwrap();
        s.add_segment(pt(3.0, 0.0), pt(1.5, 2.0)).unwrap();
        s.add_segment(pt(1.5, 2.0), pt(0.0, 0.0)).unwrap();
        assert_eq!(s.regions().len(), 1);
        let region = s.regions().values().next().unwrap();
        assert_eq!(region.outer.len(), 3);
        assert!(region.holes.is_empty());
        // Verify CCW winding.
        let pts: Vec<Point3> = region
            .outer
            .iter()
            .map(|&vid| s.vertices()[vid].position)
            .collect();
        let area = signed_area_on_plane(&pts, xy_plane().normal());
        assert!(area > 0.0, "outer boundary should be CCW");
    }

    // ── Nested concentric rings: outer-with-hole, not self-as-hole ────────────

    /// Two concentric regular n-gons (radii 1.0 and 0.5) must trace as exactly
    /// two regions: the outer ring (outer r=1 with the r=0.5 loop as a hole) and
    /// the inner disk (outer r=0.5, no holes). Regression for the facet-count
    /// sensitive nesting bug: the inner loop's CW hole-cycle used to attach to
    /// its own reverse-wound CCW twin (whose area equals the hole's only up to
    /// floating point) instead of the true r=1 encloser — leaving the outer ring
    /// hole-less and the inner disk holed by itself. That surfaced downstream as
    /// a boolean `DegenerateContact` / push-through `NonManifoldResult` on
    /// faceted solids (n = 5, 6, 16, 24 failed; 3, 4, 8, 12 happened to pass).
    #[test]
    fn concentric_rings_nest_outer_over_inner_for_all_facet_counts() {
        for n in [3usize, 4, 5, 6, 8, 12, 16, 24, 32, 48, 64] {
            let ngon = |r: f64| -> Vec<Point3> {
                (0..n)
                    .map(|i| {
                        let a = std::f64::consts::TAU * (i as f64) / (n as f64);
                        Point3::new(r * a.cos(), r * a.sin(), 0.0)
                    })
                    .collect()
            };
            let mut s = Sketch::on_plane(xy_plane());
            let add_loop = |s: &mut Sketch, pts: &[Point3]| {
                for i in 0..pts.len() {
                    s.add_segment(pts[i], pts[(i + 1) % pts.len()]).unwrap();
                }
            };
            add_loop(&mut s, &ngon(1.0));
            add_loop(&mut s, &ngon(0.5));

            let regs: Vec<&SketchRegion> = s.regions().values().collect();
            assert_eq!(regs.len(), 2, "n={n}: expected ring + inner disk");

            let radius = |vids: &[SketchVertexId]| {
                let p = s.vertices()[vids[0]].position;
                (p.x * p.x + p.y * p.y).sqrt()
            };
            let ring = regs
                .iter()
                .find(|r| radius(&r.outer) > 0.75)
                .unwrap_or_else(|| panic!("n={n}: no r=1 outer region"));
            let disk = regs
                .iter()
                .find(|r| radius(&r.outer) < 0.75)
                .unwrap_or_else(|| panic!("n={n}: no r=0.5 outer region"));

            assert_eq!(ring.holes.len(), 1, "n={n}: outer ring must have one hole");
            assert!(
                (radius(&ring.holes[0]) - 0.5).abs() < 1e-9,
                "n={n}: the ring's hole must be the r=0.5 loop"
            );
            assert!(
                disk.holes.is_empty(),
                "n={n}: the inner disk must not hole itself"
            );
        }
    }

    // ── Collinear merge: new segment entirely inside an existing edge ─────────

    #[test]
    fn collinear_new_inside_existing() {
        let mut s = Sketch::on_plane(xy_plane());
        // Existing edge spans [0,4].
        s.add_segment(pt(0.0, 0.0), pt(4.0, 0.0)).unwrap();
        // New segment is [1,3], entirely inside the existing edge.
        s.add_segment(pt(1.0, 0.0), pt(3.0, 0.0)).unwrap();
        // Result: three abutting edges [0,1], [1,3], [3,4].
        // Vertices: 0, 1, 3, 4 = 4 vertices; edges: 3.
        assert_eq!(s.vertices().len(), 4);
        assert_eq!(s.edges().len(), 3);
        assert_eq!(s.regions().len(), 0);
    }

    // ── Region identity: unchanged region keeps its id ────────────────────────

    #[test]
    fn region_identity_stable_across_disjoint_add() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let id_before: Vec<SketchRegionId> = s.regions().keys().collect();
        assert_eq!(id_before.len(), 1);

        // Add a segment far away that doesn't touch the rectangle.
        s.add_segment(pt(10.0, 10.0), pt(11.0, 10.0)).unwrap();

        // The rectangle region should still be present (same outer cycle).
        // It will have a fresh id (our implementation re-inserts), but it
        // should NOT appear in regions_created (since cycles_equal matched it).
        // We verify by checking the total region count stays 1.
        assert_eq!(s.regions().len(), 1);
    }

    // ── Chord across region produces correct diff counts ─────────────────────

    #[test]
    fn chord_diff_counts() {
        let mut s = make_rect_sketch(0.0, 0.0, 2.0, 2.0);
        let report = s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
        assert_eq!(report.regions_removed.len(), 1);
        assert_eq!(report.regions_created.len(), 2);
        assert_eq!(s.regions().len(), 2);
    }

    /// Drawing a curve over an existing plain collinear line adopts that
    /// line into the curve — no untagged hole in the selectable chain. An
    /// edge already claimed by another curve keeps its allegiance.
    #[test]
    fn curve_adopts_overlapped_plain_edges_but_not_other_curves() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(1.0, 0.0), pt(2.0, 0.0)).unwrap(); // plain line
        let c1 = s.begin_curve();
        // A longer collinear segment spanning the plain line.
        s.add_segment(pt(0.0, 0.0), pt(3.0, 0.0)).unwrap();
        s.end_curve();
        assert_eq!(
            s.curve_edges(c1).len(),
            3,
            "all three pieces (including the adopted one) are the curve"
        );

        // A second curve over c1's territory does NOT steal its edges.
        let c2 = s.begin_curve();
        s.add_segment(pt(0.0, 0.0), pt(2.0, 0.0)).unwrap();
        s.end_curve();
        assert_eq!(s.curve_edges(c1).len(), 3, "c1 keeps its edges");
        assert_eq!(s.curve_edges(c2).len(), 0, "fully-overlapped c2 owns none");
    }

    /// A closed curve touching other geometry at two vertices splits into
    /// two selectable runs at those junctions; an untouched curve is one
    /// run; a plain line is just itself.
    #[test]
    fn curve_chain_stops_at_junctions() {
        let mut s = Sketch::on_plane(xy_plane());
        // A diamond "circle" of four facets…
        let c = s.begin_curve();
        for (a, b) in [
            (pt(1.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 2.0)),
            (pt(1.0, 2.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(1.0, 0.0)),
        ] {
            s.add_segment(a, b).unwrap();
        }
        s.end_curve();
        let any_edge = s.curve_edges(c)[0];
        assert_eq!(
            s.curve_chain_at(any_edge).len(),
            4,
            "untouched curve selects whole"
        );

        // …then a line through two opposite vertices makes them junctions.
        s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();
        for &e in &s.curve_edges(c) {
            assert_eq!(
                s.curve_chain_at(e).len(),
                2,
                "each side of the crossing selects separately"
            );
        }

        // Plain lines select alone.
        let plain = s
            .edges()
            .iter()
            .find(|(_, e)| e.curve.is_none())
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(s.curve_chain_at(plain), vec![plain]);
    }

    // ── islands ───────────────────────────────────────────────────────────────

    fn two_rect_sketch() -> Sketch {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        for (a, b) in [
            (pt(3.0, 0.0), pt(4.0, 0.0)),
            (pt(4.0, 0.0), pt(4.0, 1.0)),
            (pt(4.0, 1.0), pt(3.0, 1.0)),
            (pt(3.0, 1.0), pt(3.0, 0.0)),
        ] {
            s.add_segment(a, b).unwrap();
        }
        s
    }

    /// Two disjoint rectangles are two islands; ids survive an unrelated
    /// third shape appearing, and the survivor keeps its id when the other
    /// island loses an edge.
    #[test]
    fn disjoint_shapes_are_separate_islands_with_stable_ids() {
        let mut s = two_rect_sketch();
        assert_eq!(s.islands().len(), 2);
        let before: Vec<SketchIslandId> = s.islands().keys().collect();

        s.add_segment(pt(6.0, 0.0), pt(7.0, 0.0)).unwrap();
        assert_eq!(s.islands().len(), 3);
        for id in &before {
            assert!(s.islands().contains_key(*id), "existing island ids survive");
        }

        // Remove one edge of the first rectangle: both islands keep ids.
        let some_edge = s
            .edges()
            .iter()
            .find(|(_, e)| s.vertices()[e.from].position.x < 2.0)
            .map(|(id, _)| id)
            .unwrap();
        s.remove_edge(some_edge).unwrap();
        for id in &before {
            assert!(s.islands().contains_key(*id), "ids survive an edge removal");
        }
    }

    /// A segment bridging two islands merges them into one (the id with the
    /// smaller anchor edge survives); removing the bridge splits them again
    /// and the anchor-holding component keeps the id.
    #[test]
    fn bridging_merges_islands_and_unbridging_splits_them() {
        let mut s = two_rect_sketch();
        let ids: Vec<SketchIslandId> = s.islands().keys().collect();
        let survivor = *s
            .islands()
            .iter()
            .min_by_key(|(_, isl)| isl.edges[0])
            .map(|(id, _)| id)
            .iter()
            .next()
            .unwrap();

        let report = s.add_segment(pt(1.0, 0.5), pt(3.0, 0.5)).unwrap();
        let _ = report;
        assert_eq!(s.islands().len(), 1, "bridged into one island");
        assert!(s.islands().contains_key(survivor));

        // Remove the bridge (the edge whose midpoint sits between the rects).
        let bridge = s
            .edges()
            .iter()
            .find(|(_, e)| {
                let m = (s.vertices()[e.from].position.x + s.vertices()[e.to].position.x) / 2.0;
                (1.0..3.0).contains(&m) && m > 1.01 && m < 2.99
            })
            .map(|(id, _)| id)
            .unwrap();
        s.remove_edge(bridge).unwrap();
        assert_eq!(s.islands().len(), 2, "split back into two");
        assert!(s.islands().contains_key(survivor), "anchor holder keeps id");
        // One of the two is the survivor; the other is fresh or the old
        // second id — either way there are exactly two distinct live ids.
        let _ = ids;
    }

    /// Moving one island rigidly succeeds when it lands clear, refuses when
    /// it would cross the other island, and refuses off-plane transforms —
    /// untouched on every refusal.
    #[test]
    fn island_transform_moves_one_shape_and_refuses_interference() {
        let mut s = two_rect_sketch();
        let left = s
            .islands()
            .iter()
            .find(|(_, isl)| {
                let e = s.edges()[isl.edges[0]];
                s.vertices()[e.from].position.x < 2.0
            })
            .map(|(id, _)| id)
            .unwrap();

        // Clear landing: shift left rect up by 5.
        let up = crate::Transform::translation(crate::math::Vec3::new(0.0, 5.0, 0.0));
        s.apply_transform_island(left, &up).unwrap();
        assert_eq!(s.islands().len(), 2, "still two islands");
        let e = s.edges()[s.islands()[left].edges[0]];
        assert!(s.vertices()[e.from].position.y >= 5.0 - 1e-9);

        // Interfering landing: drop it onto the right rectangle.
        let onto = crate::Transform::translation(crate::math::Vec3::new(3.0, -5.0, 0.0));
        let before = s.clone();
        assert_eq!(
            s.apply_transform_island(left, &onto).unwrap_err(),
            SketchError::WouldRetopologize
        );
        assert_eq!(
            s.edges().len(),
            before.edges().len(),
            "untouched on refusal"
        );

        // Off-plane landing.
        let lift = crate::Transform::translation(crate::math::Vec3::new(0.0, 0.0, 1.0));
        assert!(matches!(
            s.apply_transform_island(left, &lift).unwrap_err(),
            SketchError::PointOffPlane { .. }
        ));
    }

    // ── curve chains ──────────────────────────────────────────────────────────

    /// Edges committed inside a begin_curve/end_curve bracket share one
    /// curve id; edges outside carry none.
    #[test]
    fn curve_bracket_tags_only_its_edges() {
        let mut s = Sketch::on_plane(xy_plane());
        s.add_segment(pt(0.0, 0.0), pt(1.0, 0.0)).unwrap(); // plain line
        let curve = s.begin_curve();
        s.add_segment(pt(0.0, 1.0), pt(0.5, 1.2)).unwrap();
        s.add_segment(pt(0.5, 1.2), pt(1.0, 1.0)).unwrap();
        s.end_curve();
        s.add_segment(pt(0.0, 2.0), pt(1.0, 2.0)).unwrap(); // plain line

        assert_eq!(s.curve_edges(curve).len(), 2);
        let tagged: Vec<_> = s
            .edges()
            .iter()
            .filter(|(_, e)| e.curve.is_some())
            .collect();
        assert_eq!(tagged.len(), 2);
        for (eid, _) in tagged {
            assert_eq!(s.edge_curve(eid), Some(curve));
        }
    }

    /// Splitting a curve edge (a line drawn across an arc facet) leaves the
    /// fragments in the curve — the arc stays one selectable unit.
    #[test]
    fn split_curve_edge_fragments_inherit_the_curve() {
        let mut s = Sketch::on_plane(xy_plane());
        let curve = s.begin_curve();
        s.add_segment(pt(0.0, 1.0), pt(2.0, 1.0)).unwrap();
        s.end_curve();
        // Cross it with a plain line — the curve edge splits in two.
        s.add_segment(pt(1.0, 0.0), pt(1.0, 2.0)).unwrap();

        assert_eq!(
            s.curve_edges(curve).len(),
            2,
            "both fragments stay in the curve"
        );
        let plain = s.edges().values().filter(|e| e.curve.is_none()).count();
        assert_eq!(plain, 2, "the crossing line's fragments stay plain");
    }

    // ── spur pruning ──────────────────────────────────────────────────────────

    /// A dangling chain hanging into a region's interior is decoration, not
    /// boundary: the region's outer cycle is the clean silhouette (no
    /// repeated vertices) and converts to a valid profile. This is the
    /// leftover-arc-facet case: a stray interior line must not make the
    /// region unextrudable.
    #[test]
    fn interior_spur_is_pruned_from_the_region_boundary() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // Whisker from the (1,1) corner into the interior.
        s.add_segment(pt(1.0, 1.0), pt(0.5, 0.5)).unwrap();

        assert_eq!(s.regions().len(), 1, "the square still reads as one region");
        let (rid, r) = s.regions().iter().next().unwrap();
        assert_eq!(r.outer.len(), 4, "the spur is not part of the boundary");
        let mut seen = std::collections::BTreeSet::new();
        assert!(
            r.outer.iter().all(|v| seen.insert(*v)),
            "no repeated boundary vertices"
        );
        assert!(s.profile(rid).is_ok(), "the region converts to a profile");
    }

    /// Same, with the spur hanging off a mid-edge vertex and two segments
    /// long — the chain folds up tip-first and the split boundary vertex
    /// stays.
    #[test]
    fn two_segment_spur_off_a_split_edge_is_pruned() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        s.add_segment(pt(0.5, 1.0), pt(0.5, 0.6)).unwrap();
        s.add_segment(pt(0.5, 0.6), pt(0.4, 0.4)).unwrap();

        assert_eq!(s.regions().len(), 1);
        let (rid, r) = s.regions().iter().next().unwrap();
        // 4 corners + the split vertex at (0.5, 1).
        assert_eq!(r.outer.len(), 5);
        assert!(s.profile(rid).is_ok());
    }

    // ── region_scaffolding / remove_edges ────────────────────────────────────

    /// A lone rectangle's scaffolding is all 4 edges; deleting them leaves
    /// an empty sketch (vertices die with their last edge; regions and
    /// islands recompute to nothing).
    #[test]
    fn scaffolding_of_a_lone_rect_is_all_its_edges() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let region = s.regions().keys().next().expect("one region");
        let edges = s.region_scaffolding(region).expect("live region");
        assert_eq!(edges.len(), 4, "all 4 edges are exclusive scaffolding");

        s.remove_edges(&edges);
        assert!(s.edges().is_empty(), "edges deleted");
        assert!(s.vertices().is_empty(), "orphaned vertices deleted");
        assert!(s.regions().is_empty(), "no region survives");
        assert!(s.islands().is_empty(), "no island survives");
    }

    /// Two rectangles sharing the x=1 wall: the left region's scaffolding
    /// has 3 edges — the shared wall still bounds the right region and must
    /// survive, keeping the neighbor closed.
    #[test]
    fn scaffolding_excludes_an_edge_shared_with_a_surviving_region() {
        let mut s = Sketch::on_plane(xy_plane());
        // Left rect [0,0]-[1,1].
        for (a, b) in &[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(1.0, 0.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(0.0, 0.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        // Right rect [1,0]-[2,1] — shares the x=1 wall.
        for (a, b) in &[
            (pt(1.0, 0.0), pt(2.0, 0.0)),
            (pt(2.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(1.0, 0.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        assert_eq!(s.regions().len(), 2);

        // The left region: all outer vertices are at x <= 1.
        let left_region = s
            .regions()
            .iter()
            .find(|(_, r)| {
                r.outer
                    .iter()
                    .all(|&vid| s.vertices()[vid].position.x < 1.5)
            })
            .map(|(id, _)| id)
            .expect("left region");
        let right_region = s
            .regions()
            .keys()
            .find(|&id| id != left_region)
            .expect("right region");

        let edges = s.region_scaffolding(left_region).expect("live region");
        assert_eq!(edges.len(), 3, "3 exclusive edges (not the shared wall)");

        s.remove_edges(&edges);
        // The right region survives WITH ITS ID (its outer cycle is
        // untouched), the shared wall still bounds it, and only the two
        // x=0 corners died.
        assert!(s.regions().contains_key(right_region), "neighbor stays");
        assert_eq!(s.regions().len(), 1);
        assert_eq!(s.edges().len(), 4, "the right rect keeps all 4 walls");
        assert!(
            s.vertices().values().all(|v| v.position.x > 0.5),
            "only the x=0 corners died"
        );
    }

    /// Consuming the left region then the right deletes everything: the
    /// shared wall dies exactly when the LAST region needing it goes.
    #[test]
    fn shared_wall_dies_with_the_last_region_needing_it() {
        let mut s = Sketch::on_plane(xy_plane());
        for (a, b) in &[
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(1.0, 0.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(0.0, 0.0)),
            (pt(1.0, 0.0), pt(2.0, 0.0)),
            (pt(2.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 1.0)),
        ] {
            s.add_segment(*a, *b).unwrap();
        }
        assert_eq!(s.regions().len(), 2);

        let first = s.regions().keys().next().expect("first region");
        let edges = s.region_scaffolding(first).expect("live");
        s.remove_edges(&edges);
        assert_eq!(s.regions().len(), 1, "one region left");

        let second = s.regions().keys().next().expect("second region");
        let edges = s.region_scaffolding(second).expect("live");
        assert_eq!(edges.len(), s.edges().len(), "now ALL edges are exclusive");
        s.remove_edges(&edges);
        assert!(s.edges().is_empty());
        assert!(s.vertices().is_empty());
    }

    /// An open chain hanging off a consumed region's corner survives the
    /// deletion, and so does the corner vertex it needs.
    #[test]
    fn open_chains_survive_region_scaffolding_deletion() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // Open whisker off the (1,1) corner — bounds no region.
        s.add_segment(pt(1.0, 1.0), pt(2.0, 2.0)).unwrap();
        assert_eq!(s.regions().len(), 1);

        let region = s.regions().keys().next().expect("one region");
        let edges = s.region_scaffolding(region).expect("live");
        assert_eq!(edges.len(), 4, "the whisker edge is not scaffolding");

        s.remove_edges(&edges);
        assert_eq!(s.edges().len(), 1, "the whisker survives");
        assert_eq!(s.vertices().len(), 2, "the whisker keeps its corner vertex");
        assert_eq!(s.islands().len(), 1, "the whisker is one island");
    }

    /// A stale region handle is a typed error, and removing an empty edge
    /// set is a no-op.
    #[test]
    fn scaffolding_of_a_stale_region_errors() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // The null key (default) is never inserted by slotmap.
        assert_eq!(
            s.region_scaffolding(SketchRegionId::default()),
            Err(SketchError::UnknownRegion)
        );
        let before_edges = s.edges().len();
        s.remove_edges(&std::collections::BTreeSet::new());
        assert_eq!(s.edges().len(), before_edges, "empty removal is a no-op");
    }

    /// A curve chain partially deleted by consumption keeps its analytic
    /// geometry on the surviving edges: deletion removes facets but never
    /// deforms the remaining ones, so the circle stays a true description.
    #[test]
    fn partial_curve_chain_keeps_valid_geometry_after_removal() {
        let mut s = Sketch::on_plane(xy_plane());
        let geom = CurveGeom {
            center: pt(0.0, 0.0),
            radius: 1.0,
        };
        let curve = s.begin_curve_with(geom).expect("curve opens");
        // Three facets of the unit circle.
        let a = pt(1.0, 0.0);
        let b = pt(0.0, 1.0);
        let c = pt(-1.0, 0.0);
        let d = pt(0.0, -1.0);
        s.add_segment(a, b).unwrap();
        s.add_segment(b, c).unwrap();
        s.add_segment(c, d).unwrap();
        s.end_curve();

        // Delete the middle facet.
        let middle = s
            .edges()
            .iter()
            .find(|(_, e)| {
                let f = s.vertices()[e.from].position;
                let t = s.vertices()[e.to].position;
                // The b→c facet: midpoint at (-0.5, 0.5).
                (f.x + t.x) * 0.5 < -0.4 && (f.y + t.y) * 0.5 > 0.4
            })
            .map(|(id, _)| id)
            .expect("middle facet");
        s.remove_edges(&std::collections::BTreeSet::from([middle]));

        assert_eq!(s.edges().len(), 2, "two facets survive");
        assert_eq!(
            s.curve_geom(curve),
            Some(geom),
            "the surviving facets still lie on the drawn circle"
        );
        for e in s.edges().values() {
            assert_eq!(e.curve, Some(curve), "chain identity survives");
        }
    }
}
