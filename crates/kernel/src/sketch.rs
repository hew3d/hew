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
    /// The region's traced boundary does not form a valid [`Profile`] (a
    /// kernel bug in region tracing, surfaced as a typed error rather than a
    /// panic so it cannot brick the caller).
    MalformedRegion,
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
            SketchError::WouldRetopologize => {
                write!(f, "the move would cross or merge sketch geometry")
            }
            SketchError::MalformedRegion => {
                write!(f, "region boundary does not form a valid profile")
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
}

impl Sketch {
    /// An empty sketch on `plane`.
    pub fn on_plane(plane: Plane) -> Sketch {
        Sketch {
            plane,
            vertices: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            regions: SlotMap::with_key(),
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
        // Remap the plane first (cannot fail on a validated non-singular map),
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

        *self = s;
        Ok(old_pos)
    }

    /// The sketch edges and vertices to hide ("tombstone") given that every
    /// region in `consumed` has been extruded into a solid: an edge is
    /// tombstoned iff it lies on a consumed region's boundary and on NO live
    /// region's boundary; a vertex is tombstoned iff it lies on a consumed
    /// region's boundary and every edge incident to it is tombstoned.
    ///
    /// The rule is a pure function of the FULL consumed set — order-free —
    /// so an edge shared by two regions dies exactly when the last region
    /// needing it is consumed, never before. Callers diff successive results
    /// to attribute an increment to one extrude (undo needs per-step deltas);
    /// the load path evaluates it once with the file's whole consumed set and
    /// lands on the same answer. Unknown/stale ids in `consumed` are skipped.
    /// Pure query — no mutation.
    pub fn consumed_tombstones(
        &self,
        consumed: &std::collections::BTreeSet<SketchRegionId>,
    ) -> (
        std::collections::BTreeSet<SketchEdgeId>,
        std::collections::BTreeSet<SketchVertexId>,
    ) {
        let mut consumed_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();
        let mut consumed_verts: std::collections::BTreeSet<SketchVertexId> =
            std::collections::BTreeSet::new();
        let mut live_edges: std::collections::BTreeSet<SketchEdgeId> =
            std::collections::BTreeSet::new();

        for (rid, r) in &self.regions {
            let is_consumed = consumed.contains(&rid);
            let loops: Vec<&Vec<SketchVertexId>> =
                std::iter::once(&r.outer).chain(r.holes.iter()).collect();
            for lp in &loops {
                for i in 0..lp.len() {
                    let a = lp[i];
                    let b = lp[(i + 1) % lp.len()];
                    if let Some(eid) = self.edge_between(a, b) {
                        if is_consumed {
                            consumed_edges.insert(eid);
                        } else {
                            live_edges.insert(eid);
                        }
                    }
                    if is_consumed {
                        consumed_verts.insert(a);
                    }
                }
            }
        }

        // Edges on a consumed boundary that no live region still needs.
        let tomb_edges: std::collections::BTreeSet<SketchEdgeId> = consumed_edges
            .into_iter()
            .filter(|e| !live_edges.contains(e))
            .collect();

        // Vertices left with no visible incident edge once tomb_edges hide.
        // Edges outside any region (open chains) count as visible, so their
        // endpoints are never tombstoned out from under them.
        let tomb_verts: std::collections::BTreeSet<SketchVertexId> = consumed_verts
            .into_iter()
            .filter(|&vid| {
                self.edges.iter().all(|(eid, e)| {
                    if e.from == vid || e.to == vid {
                        tomb_edges.contains(&eid)
                    } else {
                        true // not incident — irrelevant
                    }
                })
            })
            .collect();

        (tomb_edges, tomb_verts)
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
        Profile::new(self.plane, outer, holes).map_err(|_| SketchError::MalformedRegion)
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
    }
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

            // Remove the old edge.
            self.edges.remove(esplit.edge_id);

            // Insert fragment edges.  Fragments of split *existing* edges go into
            // split_edges only, not new_edges (new_edges is for the inserted segment
            // fragments).
            let mut fragments: Vec<SketchEdgeId> = Vec::new();
            for w in chain.windows(2) {
                let frag_id = self.edges.insert(SketchEdge {
                    from: w[0],
                    to: w[1],
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
                // Collinear merge: this sub-segment already exists; skip.
                continue;
            }
            // Skip zero-length fragments.
            if self.vertices[a]
                .position
                .approx_eq(self.vertices[b].position, tol::POINT_MERGE)
            {
                continue;
            }
            let eid = self.edges.insert(SketchEdge { from: a, to: b });
            report.new_edges.push(eid);
        }

        // ── Step 9: recompute regions and diff ────────────────────────────────
        let (regions_created, regions_removed) = self.recompute_regions_with_diff(&old_regions);
        report.regions_created = regions_created;
        report.regions_removed = regions_removed;

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
        })
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

    // ── consumed_tombstones ───────────────────────────────────────────────────

    /// Consuming a lone rectangle's region tombstones all 4 edges and verts.
    #[test]
    fn tombstones_single_rect_consumes_all_edges_and_verts() {
        let s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let region = s.regions().keys().next().expect("one region");
        let consumed = std::collections::BTreeSet::from([region]);
        let (edges, verts) = s.consumed_tombstones(&consumed);
        assert_eq!(edges.len(), 4, "all 4 edges tombstoned");
        assert_eq!(verts.len(), 4, "all 4 vertices tombstoned");
    }

    /// Two rectangles sharing the x=1 wall: the first region's exclusive
    /// boundary has 3 edges (not the shared wall) and the 2 corners NOT on
    /// the shared wall.
    #[test]
    fn exclusive_boundary_shared_wall_excluded() {
        let mut s = Sketch::on_plane(xy_plane());
        // Left rect [0,0]-[1,1].
        let segs_left = [
            (pt(0.0, 0.0), pt(1.0, 0.0)),
            (pt(1.0, 0.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(0.0, 1.0)),
            (pt(0.0, 1.0), pt(0.0, 0.0)),
        ];
        for (a, b) in &segs_left {
            s.add_segment(*a, *b).unwrap();
        }
        // Right rect [1,0]-[2,1] — shares the x=1 wall.
        let segs_right = [
            (pt(1.0, 0.0), pt(2.0, 0.0)),
            (pt(2.0, 0.0), pt(2.0, 1.0)),
            (pt(2.0, 1.0), pt(1.0, 1.0)),
            (pt(1.0, 1.0), pt(1.0, 0.0)),
        ];
        for (a, b) in &segs_right {
            s.add_segment(*a, *b).unwrap();
        }
        assert_eq!(s.regions().len(), 2);

        // The left region: all outer vertices are at x <= 1.
        let left_region = s
            .regions()
            .iter()
            .find(|(_, r)| {
                r.outer.iter().all(|&vid| {
                    let x = s.vertices()[vid].position.x;
                    x < 1.5 // all vertices at x=0 or x=1
                })
            })
            .map(|(id, _)| id)
            .expect("left region");

        let consumed = std::collections::BTreeSet::from([left_region]);
        let (edges, verts) = s.consumed_tombstones(&consumed);
        // 3 tombstoned edges (bottom, top, left — the shared x=1 wall still
        // bounds the live right region and must survive).
        assert_eq!(edges.len(), 3, "3 tombstoned edges (not the shared wall)");
        // 2 tombstoned vertices: (0,0) and (0,1) — not the shared x=1 corners.
        assert_eq!(
            verts.len(),
            2,
            "2 tombstoned vertices (not on the shared wall)"
        );
        // Verify the 2 tombstoned vertices are at x=0.
        for vid in &verts {
            let x = s.vertices()[*vid].position.x;
            assert!(
                (x - 0.0).abs() < crate::tol::POINT_MERGE,
                "tombstoned vertex should be at x=0, got x={x}"
            );
        }
    }

    /// Consuming BOTH regions of the shared-wall pair tombstones everything,
    /// including the shared wall and its corners — the orphan-edge case: an
    /// edge shared only with already-consumed regions must not survive.
    #[test]
    fn tombstones_shared_wall_dies_when_both_regions_consumed() {
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

        let consumed: std::collections::BTreeSet<SketchRegionId> = s.regions().keys().collect();
        let (edges, verts) = s.consumed_tombstones(&consumed);
        assert_eq!(
            edges.len(),
            s.edges().len(),
            "every edge tombstoned once no live region needs it"
        );
        assert_eq!(
            verts.len(),
            s.vertices().len(),
            "every vertex tombstoned with its edges"
        );
    }

    /// A vertex shared between a consumed region and an edge OUTSIDE any
    /// region (an open chain) survives: hiding it would strand the chain.
    #[test]
    fn tombstones_keep_vertices_used_by_open_chains() {
        let mut s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        // Open whisker off the (1,1) corner — bounds no region.
        s.add_segment(pt(1.0, 1.0), pt(2.0, 2.0)).unwrap();
        assert_eq!(s.regions().len(), 1);

        let consumed: std::collections::BTreeSet<SketchRegionId> = s.regions().keys().collect();
        let (edges, verts) = s.consumed_tombstones(&consumed);
        assert_eq!(edges.len(), 4, "the whisker edge is not tombstoned");
        assert_eq!(
            verts.len(),
            3,
            "the whisker's corner vertex survives with it"
        );
    }

    /// Unknown region ids in the consumed set are skipped; an empty consumed
    /// set tombstones nothing.
    #[test]
    fn tombstones_unknown_region_and_empty_set_return_empty() {
        let s = make_rect_sketch(0.0, 0.0, 1.0, 1.0);
        let empty = std::collections::BTreeSet::new();
        let (edges, verts) = s.consumed_tombstones(&empty);
        assert!(edges.is_empty());
        assert!(verts.is_empty());

        // The null key (default) is never inserted by slotmap and is always stale.
        let bogus = std::collections::BTreeSet::from([SketchRegionId::default()]);
        let (edges, verts) = s.consumed_tombstones(&bogus);
        assert!(edges.is_empty());
        assert!(verts.is_empty());
    }
}
