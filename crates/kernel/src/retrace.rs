//! The local re-trace primitive, [`Object::retrace_face`]: insert and/or
//! remove several paths in one face (or the faces the removed edges
//! separate) at once, then re-trace every loop of the affected region by
//! rotational order at its vertices — the half-edge form of the ground's
//! region tracer (`Sketch::compute_all_regions`).
//!
//! What one-path-at-a-time `split_face` / `merge_faces` cannot do, this
//! can: a path may join two DIFFERENT boundary loops of a face (a bridge)
//! as long as the paths inserted with it close a region, so that no bridge
//! is left; and removing the edges two faces share along several separate
//! runs merges them into one face with new holes, or splits a hole into
//! several. A traced walk that visits a vertex twice (a loop touching the
//! boundary or another loop at one point) is split there into simple loops
//! that share the vertex: every committed loop is simple, and loops of one
//! face share at most isolated vertices, never an edge (ARCHITECTURE §2.4).
//! Bridges are intermediate only: a re-trace whose result would hold a walk
//! running out along an edge and back along its twin (a bridge, or a
//! dangling end) is refused.
//!
//! Its inverse is itself with the arguments swapped: the report carries the
//! inserted paths as resolved and the removed chains with their circle
//! claims, and the history records exactly that.

use std::collections::{BTreeMap, BTreeSet};

use crate::geom2d::{
    plane_axes, point_inside_polygon, point_near_segment, segments_intersect, signed_area_on_plane,
};
use crate::ids::{EdgeId, FaceId, HalfEdgeId, LoopId, VertexId};
use crate::math::Point3;
use crate::ops::{
    FaceAttrsAt, StickyError, attrs_snapshot, build_cut_chains, curve_geom_is_finite,
    heal_run_endpoints, repair_dangling_outgoing, split_boundary_edge, wire_loop_sequence,
};
use crate::sketch::CurveGeom;
use crate::tol;
use crate::topo::{Face, Loop, LoopKind, Object, WatertightState};

/// One path a re-trace inserts: boundary point to boundary point, interior
/// points strictly inside the face, with the circle each edge is a chord
/// facet of (`curves[k]` for `path[k]` → `path[k+1]`; empty claims nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct RetracePath {
    /// The path's points, ends on the face's boundary loops.
    pub path: Vec<Point3>,
    /// One claim per path edge, or empty.
    pub curves: Vec<Option<CurveGeom>>,
    /// One regular-polygon claim per path edge ([`crate::topo::Edge::polygon`]),
    /// or empty.
    pub polygons: Vec<Option<CurveGeom>>,
}

/// What [`Object::retrace_face`] changed. (Carries `f64` positions, so
/// `PartialEq` but not `Eq`.)
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRetraceReport {
    /// The faces covering the re-traced region afterwards, in trace order.
    /// A face of the region keeps its handle on the first result face it
    /// survives into (the one a caller asked to keep, else its lowest key).
    pub faces: Vec<FaceId>,
    /// The edges of each inserted path, in path order.
    pub new_edges: Vec<Vec<EdgeId>>,
    /// Now-dead handles: the removed edges, plus edges healed away where a
    /// removed chain's end was left a straight-through scar.
    pub removed_edges: Vec<EdgeId>,
    /// Each region face's attribute state before the re-trace, pinned to a
    /// point strictly inside it (best effort: a face too thin to pin is
    /// skipped). The inverse restores them.
    pub prior_attrs: Vec<FaceAttrsAt>,
    /// The inserted paths as committed: ends resolved onto the boundary
    /// (an existing vertex's exact position, or the point an edge was split
    /// at), with their claims. Removing these chains is the inverse.
    pub inserted: Vec<RetracePath>,
    /// The removed chains as they were, with each edge's circle claim, in a
    /// canonical order (each oriented from its lexicographically smaller
    /// end, sorted by that end). Inserting these is the inverse.
    pub removed: Vec<RetracePath>,
}

/// Where an inserted path's end meets the region's boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    /// At an existing vertex.
    Vertex(VertexId),
    /// In the open interior of an existing edge, at this point.
    Edge(EdgeId, Point3),
    /// A JUNCTION strictly inside the region, shared by the ends of two or
    /// more inserted paths (a fresh vertex): lines drawn to meet each other
    /// inside a face, not only its boundary.
    Junction(Point3),
}

impl Object {
    /// Re-traces a face: inserts the paths `insert` and removes the edge
    /// chains `remove` (each given by its vertex positions, matched within
    /// [`tol::POINT_MERGE`]) in one step, then rebuilds every loop of the
    /// region — `face` plus every face on either side of a removed edge —
    /// by rotational order at each vertex, the way the ground traces a
    /// sketch's regions. Faces split and merge as the lines say: several
    /// paths cut a face into as many faces as they close, removing every
    /// edge two faces share merges them (with a hole where they enclosed
    /// another face), and a hole split by a removal becomes several holes.
    ///
    /// An inserted path runs from a boundary point of the region (an
    /// existing vertex, or a point within [`tol::POINT_MERGE`] of an edge's
    /// interior, which splits that edge and its twin) to another, its
    /// interior points strictly inside the region. An end may instead be a
    /// JUNCTION: a point strictly inside the region where the ends of two
    /// or more inserted paths meet (within [`tol::POINT_MERGE`]; they share
    /// one fresh vertex). Junctions let one re-trace insert any planar
    /// drawing — a T, a theta, a closed loop clear of the boundary given as
    /// two or more paths — not only boundary-to-boundary cuts. Unlike
    /// [`Object::split_face`], its two ends may lie on DIFFERENT loops of
    /// the face (a bridge between the outer loop and a hole, or between two
    /// holes) provided the result holds none: no result face has an edge on
    /// both sides. A traced walk visiting a vertex twice is split there, so
    /// every result loop visits each vertex once; loops of one face may
    /// share vertices (a hole touching the outer loop at one point). New faces
    /// take the attributes of the face they came from; a face merged from
    /// several keeps the attributes of `face` if it is one of them (else
    /// the lowest key's), and its analytic surface only if all agreed
    /// (map-or-drop). Removed chains' ends left a straight two-edge scar
    /// are healed away, as `merge_faces` does.
    ///
    /// The inverse is the same call with the arguments swapped:
    /// [`FaceRetraceReport::inserted`] removed and
    /// [`FaceRetraceReport::removed`] inserted.
    ///
    /// # Errors
    /// All leave the object untouched.
    /// - [`StickyError::UnknownFace`] — stale `face`.
    /// - [`StickyError::PathTooShort`] — nothing to insert or remove, or a
    ///   path or chain of fewer than two points.
    /// - [`StickyError::CurveClaimOffLoop`] — `curves` neither empty nor
    ///   one per path edge, or a claimed edge's ends off its circle.
    /// - [`StickyError::UnknownEdge`] — a chain segment that is no edge.
    /// - [`StickyError::FacesNotCoplanar`] — a removed edge between faces
    ///   not coplanar and same-facing with `face`.
    /// - [`StickyError::EndpointNotOnBoundary`] — a path end on no
    ///   boundary of the region (`which` 0 = start, 1 = end).
    /// - [`StickyError::PointNotOnFace`] — an interior point off the plane
    ///   or outside the region.
    /// - [`StickyError::PathNotSimple`] — paths crossing or touching each
    ///   other or the boundary away from their ends, a zero-length segment,
    ///   a path closing on one point, an end on a removed edge; or, when
    ///   inserting, a result holding a bridge (an edge with one face on
    ///   both sides).
    /// - [`StickyError::SharedChainDisconnected`] — a removal-only re-trace
    ///   whose result would hold a bridge.
    /// - [`StickyError::WouldCorrupt`] — anything the validator refuses.
    pub fn retrace_face(
        &mut self,
        face: FaceId,
        insert: &[RetracePath],
        remove: &[Vec<Point3>],
    ) -> Result<FaceRetraceReport, StickyError> {
        self.retrace_face_impl(face, insert, remove, &[], None)
    }

    /// [`Object::retrace_face`] with undo's attribute restoration (each
    /// snapshot lands on the result face holding its point) and the face
    /// whose handle and attributes a merge keeps (`keep`; `face` when
    /// `None`).
    pub(crate) fn retrace_face_impl(
        &mut self,
        face: FaceId,
        insert: &[RetracePath],
        remove: &[Vec<Point3>],
        restore: &[FaceAttrsAt],
        keep: Option<FaceId>,
    ) -> Result<FaceRetraceReport, StickyError> {
        // ---- input shape ----
        let seed = self.faces.get(face).ok_or(StickyError::UnknownFace)?;
        if insert.is_empty() && remove.is_empty() {
            return Err(StickyError::PathTooShort);
        }
        for p in insert {
            if p.path.len() < 2 {
                return Err(StickyError::PathTooShort);
            }
            if !p.curves.is_empty() && p.curves.len() != p.path.len() - 1 {
                return Err(StickyError::CurveClaimOffLoop);
            }
            if !p.polygons.is_empty() && p.polygons.len() != p.path.len() - 1 {
                return Err(StickyError::CurveClaimOffLoop);
            }
        }
        if remove.iter().any(|c| c.len() < 2) {
            return Err(StickyError::PathTooShort);
        }
        let plane = seed.plane;
        let normal = plane.normal();
        let keep = keep.unwrap_or(face);
        let group: BTreeSet<FaceId> = self.coplanar_face_group(face).into_iter().collect();

        // ---- the removed edges, and the region ----
        let mut removed_edges: Vec<EdgeId> = Vec::new();
        let mut removed_paths: Vec<RetracePath> = Vec::new();
        let mut region: Vec<FaceId> = vec![face];
        for chain in remove {
            let mut path = Vec::with_capacity(chain.len());
            let mut curves = Vec::with_capacity(chain.len() - 1);
            let mut polygons = Vec::with_capacity(chain.len() - 1);
            for (k, w) in chain.windows(2).enumerate() {
                let (e, a, b) = self
                    .edge_matching(w[0], w[1])
                    .ok_or(StickyError::UnknownEdge)?;
                if removed_edges.contains(&e) {
                    return Err(StickyError::PathNotSimple);
                }
                let edge = &self.edges[e];
                let twin = edge.twin_half_edge.ok_or(StickyError::BoundaryEdge)?;
                for h in [edge.half_edge, twin] {
                    let f = self.loops[self.half_edges[h].loop_id].face;
                    if !group.contains(&f) {
                        return Err(StickyError::FacesNotCoplanar);
                    }
                    if !region.contains(&f) {
                        region.push(f);
                    }
                }
                if k == 0 {
                    path.push(a);
                } else if !path.last().is_some_and(|&p: &Point3| bits(p) == bits(a)) {
                    return Err(StickyError::PathNotSimple);
                }
                path.push(b);
                curves.push(edge.curve);
                polygons.push(edge.polygon);
                removed_edges.push(e);
            }
            removed_paths.push(RetracePath {
                path,
                curves,
                polygons,
            });
        }
        // Canonical: each chain from its lexicographically smaller end,
        // chains sorted by that end — so a replay inserts them identically.
        for p in &mut removed_paths {
            if lex_less(*p.path.last().expect("len >= 2"), p.path[0]) {
                p.path.reverse();
                p.curves.reverse();
                p.polygons.reverse();
            }
        }
        removed_paths.sort_by(|a, b| lex_cmp(a.path[0], b.path[0]));
        let removed_set: BTreeSet<EdgeId> = removed_edges.iter().copied().collect();
        let region_loops: Vec<LoopId> = region
            .iter()
            .flat_map(|&f| {
                let fd = &self.faces[f];
                std::iter::once(fd.outer_loop).chain(fd.inner_loops.iter().copied())
            })
            .collect();
        let region_loop_set: BTreeSet<LoopId> = region_loops.iter().copied().collect();

        // The boundary that remains: every region edge not removed.
        let mut kept_edges: Vec<(EdgeId, Point3, Point3)> = Vec::new();
        let mut kept_seen: BTreeSet<EdgeId> = BTreeSet::new();
        let mut kept_verts: BTreeMap<VertexId, Point3> = BTreeMap::new();
        for &l in &region_loops {
            for h in self.loop_half_edges(l) {
                let he = &self.half_edges[h];
                if removed_set.contains(&he.edge) {
                    continue;
                }
                // Both ends: a kept edge's far end may lie only on removed
                // half-edges within the region's loops.
                let to = self.half_edges[he.next].origin;
                let (a, b) = (
                    self.vertices[he.origin].position,
                    self.vertices[to].position,
                );
                kept_verts.insert(he.origin, a);
                kept_verts.insert(to, b);
                if kept_seen.insert(he.edge) {
                    kept_edges.push((he.edge, a, b));
                }
            }
        }
        let removed_segs: Vec<(Point3, Point3)> = removed_edges
            .iter()
            .map(|&e| self.edge_endpoints(e).expect("live removed edge"))
            .collect();
        let in_region = |p: Point3| {
            region.iter().any(|&f| self.face_contains_point(f, p))
                || removed_segs
                    .iter()
                    .any(|&(a, b)| point_near_segment(p, a, b, tol::POINT_MERGE))
        };
        let near_kept = |p: Point3| {
            kept_edges
                .iter()
                .any(|&(_, a, b)| point_near_segment(p, a, b, tol::POINT_MERGE))
        };

        // ---- resolve the inserted paths' ends ----
        let mut ends: Vec<[End; 2]> = Vec::with_capacity(insert.len());
        let mut resolved: Vec<Vec<Point3>> = Vec::with_capacity(insert.len());
        // Edge-interior anchors already placed, so two paths meeting at one
        // point of an edge share its exact bits.
        let mut placed: Vec<(EdgeId, Point3)> = Vec::new();
        // Junction points already placed, so every path meeting at one shares
        // its exact bits.
        let mut junctions: Vec<Point3> = Vec::new();
        for p in insert {
            let mut pair = [End::Vertex(VertexId::default()); 2];
            for (which, &pt) in [p.path[0], *p.path.last().expect("len >= 2")]
                .iter()
                .enumerate()
            {
                if plane.signed_distance(pt).abs() > tol::PLANE_DIST {
                    return Err(StickyError::EndpointNotOnBoundary { which });
                }
                let end = if let Some((&v, _)) = kept_verts
                    .iter()
                    .find(|&(_, &q)| q.approx_eq(pt, tol::POINT_MERGE))
                {
                    End::Vertex(v)
                } else if let Some(&(e, _, _)) =
                    kept_edges.iter().find(|&&(_, a, b)| on_open_edge(pt, a, b))
                {
                    let at = placed
                        .iter()
                        .find(|&&(pe, q)| pe == e && q.approx_eq(pt, tol::POINT_MERGE))
                        .map_or(pt, |&(_, q)| q);
                    if placed
                        .iter()
                        .any(|&(pe, q)| pe != e && q.approx_eq(at, tol::POINT_MERGE))
                    {
                        return Err(StickyError::PathNotSimple);
                    }
                    placed.push((e, at));
                    End::Edge(e, at)
                } else if let Some(q) = junctions
                    .iter()
                    .copied()
                    .find(|q| q.approx_eq(pt, tol::POINT_MERGE))
                {
                    End::Junction(q)
                } else if insert.iter().any(|other| {
                    // Another inserted path's end (not this very end) here.
                    [other.path[0], *other.path.last().expect("len >= 2")]
                        .iter()
                        .enumerate()
                        .any(|(w2, &q)| {
                            !(std::ptr::eq(other, p) && w2 == which)
                                && q.approx_eq(pt, tol::POINT_MERGE)
                        })
                }) && in_region(pt)
                    && !near_kept(pt)
                {
                    junctions.push(pt);
                    End::Junction(pt)
                } else if removed_segs
                    .iter()
                    .any(|&(a, b)| point_near_segment(pt, a, b, tol::POINT_MERGE))
                {
                    return Err(StickyError::PathNotSimple);
                } else {
                    return Err(StickyError::EndpointNotOnBoundary { which });
                };
                pair[which] = end;
            }
            let pos = |e: End| match e {
                End::Vertex(v) => self.vertices[v].position,
                End::Edge(_, q) | End::Junction(q) => q,
            };
            let mut path = p.path.clone();
            path[0] = pos(pair[0]);
            *path.last_mut().expect("len >= 2") = pos(pair[1]);
            ends.push(pair);
            resolved.push(path);
        }

        // ---- gates on the resolved paths ----
        for (pi, path) in resolved.iter().enumerate() {
            let n = path.len();
            if path[0].approx_eq(path[n - 1], tol::POINT_MERGE)
                || path
                    .windows(2)
                    .any(|w| w[0].approx_eq(w[1], tol::POINT_MERGE))
            {
                return Err(StickyError::PathNotSimple);
            }
            for (index, &pt) in path.iter().enumerate().take(n - 1).skip(1) {
                if plane.signed_distance(pt).abs() > tol::PLANE_DIST {
                    return Err(StickyError::PointNotOnFace { index });
                }
                if near_kept(pt) {
                    return Err(StickyError::PathNotSimple);
                }
                if !in_region(pt) {
                    return Err(StickyError::PointNotOnFace { index });
                }
            }
            for (k, g) in insert[pi].curves.iter().enumerate() {
                let Some(g) = g else { continue };
                if !curve_geom_is_finite(g) || g.radius <= tol::POINT_MERGE {
                    return Err(StickyError::CurveClaimOffLoop);
                }
                for q in [path[k], path[k + 1]] {
                    if ((q - g.center).length() - g.radius).abs() > self.planarity_tol {
                        return Err(StickyError::CurveClaimOffLoop);
                    }
                }
            }
            let last = n - 2;
            for (si, w) in path.windows(2).enumerate() {
                let (a, b) = (w[0], w[1]);
                let mid = a + (b - a) * 0.5;
                if near_kept(mid) || !in_region(mid) {
                    return Err(StickyError::PathNotSimple);
                }
                for &v in kept_verts.values() {
                    if point_near_segment(v, a, b, tol::POINT_MERGE)
                        && !v.approx_eq(a, tol::POINT_MERGE)
                        && !v.approx_eq(b, tol::POINT_MERGE)
                    {
                        return Err(StickyError::PathNotSimple);
                    }
                }
                for &(_, c, d) in &kept_edges {
                    let exempt = (si == 0 && point_near_segment(a, c, d, tol::POINT_MERGE))
                        || (si == last && point_near_segment(b, c, d, tol::POINT_MERGE));
                    if !exempt && segments_intersect(a, b, c, d) {
                        return Err(StickyError::PathNotSimple);
                    }
                }
                // Against every other segment inserted: consecutive segments
                // meet at their joint, two paths may meet at an end of both
                // (a shared anchor), and nothing else touches.
                let ends_of = |si: usize, last: usize, a: Point3, b: Point3| {
                    let mut e = Vec::new();
                    if si == 0 {
                        e.push(a);
                    }
                    if si == last {
                        e.push(b);
                    }
                    e
                };
                let mine = ends_of(si, last, a, b);
                for (pj, other) in resolved.iter().enumerate().skip(pi) {
                    let other_last = other.len() - 2;
                    for (sj, w2) in other.windows(2).enumerate() {
                        if pj == pi && sj <= si {
                            continue;
                        }
                        let (c, d) = (w2[0], w2[1]);
                        let allowed: Option<Point3> = if pj == pi {
                            (sj == si + 1).then_some(b)
                        } else {
                            let theirs = ends_of(sj, other_last, c, d);
                            mine.iter()
                                .copied()
                                .find(|&x| theirs.iter().any(|&y| bits(x) == bits(y)))
                        };
                        if !meet_only_at(a, b, c, d, allowed) {
                            return Err(StickyError::PathNotSimple);
                        }
                    }
                }
            }
        }

        // ---- surgery, on a copy ----
        let mut obj = self.clone();
        let prior_attrs: Vec<FaceAttrsAt> = region
            .iter()
            .filter_map(|&f| attrs_snapshot(&obj, f))
            .collect();
        let old_face_of_loop: BTreeMap<LoopId, FaceId> = region_loops
            .iter()
            .map(|&l| (l, obj.loops[l].face))
            .collect();

        // 1. Split the edges the ends land inside (and their twins), while
        //    every region loop is still intact.
        let mut end_vertex: BTreeMap<[u64; 3], VertexId> = BTreeMap::new();
        for pair in &ends {
            for &end in pair {
                if let End::Edge(_, at) = end {
                    if end_vertex.contains_key(&bits(at)) {
                        continue;
                    }
                    let h = obj
                        .half_edges
                        .iter()
                        .filter(|(_, he)| region_loop_set.contains(&he.loop_id))
                        .find(|(_, he)| {
                            let a = obj.vertices[he.origin].position;
                            let b = obj.vertices[obj.half_edges[he.next].origin].position;
                            on_open_edge(at, a, b)
                        })
                        .map(|(h, _)| h)
                        .ok_or(StickyError::WouldCorrupt)?;
                    let (v, _, _) = split_boundary_edge(&mut obj, h, at);
                    end_vertex.insert(bits(at), v);
                }
            }
        }
        // Junctions: one fresh vertex each, shared by every path ending
        // there (its outgoing half-edge is set when the loops are wired).
        for pair in &ends {
            for &end in pair {
                if let End::Junction(at) = end
                    && !end_vertex.contains_key(&bits(at))
                {
                    let v = obj.vertices.insert(crate::topo::Vertex {
                        position: at,
                        outgoing: HalfEdgeId::default(),
                    });
                    end_vertex.insert(bits(at), v);
                }
            }
        }
        let end_v = |e: End| match e {
            End::Vertex(v) => v,
            End::Edge(_, at) | End::Junction(at) => end_vertex[&bits(at)],
        };

        // 2. Each region half-edge's destination and old face, before the
        //    removal leaves `next` pointers dangling.
        let mut dest: BTreeMap<HalfEdgeId, VertexId> = BTreeMap::new();
        let mut old_face: BTreeMap<HalfEdgeId, FaceId> = BTreeMap::new();
        for (h, he) in obj.half_edges.iter() {
            if let Some(&f) = old_face_of_loop.get(&he.loop_id) {
                dest.insert(h, obj.half_edges[he.next].origin);
                old_face.insert(h, f);
            }
        }

        // 3. Remove the chains.
        let mut touched: BTreeSet<VertexId> = BTreeSet::new();
        for &e in &removed_edges {
            let edge = obj.edges[e];
            let twin = edge.twin_half_edge.expect("checked above");
            for h in [edge.half_edge, twin] {
                touched.insert(obj.half_edges[h].origin);
                obj.half_edges.remove(h);
                dest.remove(&h);
                old_face.remove(&h);
            }
            obj.edges.remove(e);
        }

        // 4. Insert the paths.
        let placeholder = region_loops[0];
        let mut new_edges: Vec<Vec<EdgeId>> = Vec::with_capacity(insert.len());
        let mut inserted: Vec<RetracePath> = Vec::with_capacity(insert.len());
        for (pi, path) in resolved.iter().enumerate() {
            let (v0, v1) = (end_v(ends[pi][0]), end_v(ends[pi][1]));
            let (_, fwd, bwd, edges) = build_cut_chains(&mut obj, path, v0, v1, placeholder);
            for h in fwd.iter().chain(bwd.iter()) {
                let t = obj.half_edges[*h].twin.expect("a cut half-edge has a twin");
                dest.insert(*h, obj.half_edges[t].origin);
            }
            for (k, &e) in edges.iter().enumerate() {
                if let Some(&g) = insert[pi].curves.get(k) {
                    obj.edges[e].curve = g;
                }
                if let Some(&g) = insert[pi].polygons.get(k) {
                    obj.edges[e].polygon = g;
                }
            }
            new_edges.push(edges);
            inserted.push(RetracePath {
                path: path.clone(),
                curves: insert[pi].curves.clone(),
                polygons: insert[pi].polygons.clone(),
            });
        }

        // 5. Rotational order: at each vertex, the region's edges sorted
        //    counter-clockwise; the half-edge after one arriving along edge
        //    `e` leaves along the edge next CLOCKWISE from `e` (the region
        //    stays on the left).
        let (u, v) = plane_axes(normal);
        let s_set: Vec<HalfEdgeId> = dest.keys().copied().collect();
        let in_s: BTreeSet<HalfEdgeId> = s_set.iter().copied().collect();
        // vertex → [(angle, edge, half-edge in S leaving the vertex along it)]
        let mut spokes: BTreeMap<VertexId, Vec<(f64, EdgeId, Option<HalfEdgeId>)>> =
            BTreeMap::new();
        for &h in &s_set {
            let he = obj.half_edges[h];
            let (a, b) = (he.origin, dest[&h]);
            for (at, to, out) in [(a, b, Some(h)), (b, a, None)] {
                let d = obj.vertices[to].position - obj.vertices[at].position;
                let angle = d.dot(v).atan2(d.dot(u));
                let list = spokes.entry(at).or_default();
                match list.iter_mut().find(|s| s.1 == he.edge) {
                    Some(s) => {
                        if out.is_some() {
                            s.2 = out;
                        }
                    }
                    None => list.push((angle, he.edge, out)),
                }
            }
        }
        for list in spokes.values_mut() {
            list.sort_by(|x, y| x.0.total_cmp(&y.0));
            if list.windows(2).any(|w| w[0].0 == w[1].0) {
                return Err(StickyError::WouldCorrupt);
            }
        }
        let mut next: BTreeMap<HalfEdgeId, HalfEdgeId> = BTreeMap::new();
        for &h in &s_set {
            let he = obj.half_edges[h];
            let at = dest[&h];
            let list = &spokes[&at];
            let i = list
                .iter()
                .position(|s| s.1 == he.edge)
                .ok_or(StickyError::WouldCorrupt)?;
            let cw = list[(i + list.len() - 1) % list.len()];
            let n = cw.2.ok_or(StickyError::WouldCorrupt)?;
            if !in_s.contains(&n) {
                return Err(StickyError::WouldCorrupt);
            }
            next.insert(h, n);
        }
        let targets: BTreeSet<HalfEdgeId> = next.values().copied().collect();
        if targets.len() != next.len() {
            return Err(StickyError::WouldCorrupt);
        }

        // 6. Trace the cycles; classify outer (counter-clockwise) and hole.
        let bridge_err = if insert.is_empty() {
            StickyError::SharedChainDisconnected
        } else {
            StickyError::PathNotSimple
        };
        let mut seen: BTreeSet<HalfEdgeId> = BTreeSet::new();
        let mut outers: Vec<(Vec<HalfEdgeId>, f64)> = Vec::new();
        let mut holes: Vec<Vec<HalfEdgeId>> = Vec::new();
        for &start in &s_set {
            if seen.contains(&start) {
                continue;
            }
            let mut cycle = vec![start];
            seen.insert(start);
            let mut cur = next[&start];
            while cur != start {
                if !seen.insert(cur) {
                    return Err(StickyError::WouldCorrupt);
                }
                cycle.push(cur);
                cur = next[&cur];
            }
            // A walk running out along an edge and back along its twin is a
            // bridge (between two boundaries) or an antenna (a dangling
            // end): no face holds it.
            let members: BTreeSet<HalfEdgeId> = cycle.iter().copied().collect();
            if cycle
                .iter()
                .any(|&h| obj.half_edges[h].twin.is_some_and(|t| members.contains(&t)))
            {
                return Err(bridge_err);
            }
            // A walk visiting a vertex twice is split there into simple
            // loops: a hole touching the outer loop at one point, or two
            // holes touching, share that vertex as separate loops.
            for piece in split_at_repeated_vertices(&obj, cycle) {
                let pts: Vec<Point3> = piece
                    .iter()
                    .map(|&h| obj.vertices[obj.half_edges[h].origin].position)
                    .collect();
                let area = signed_area_on_plane(&pts, normal);
                if area.abs() <= tol::POINT_MERGE * tol::POINT_MERGE {
                    return Err(StickyError::WouldCorrupt);
                }
                if area > 0.0 {
                    outers.push((piece, area));
                } else {
                    holes.push(piece);
                }
            }
        }
        if outers.is_empty() {
            return Err(StickyError::WouldCorrupt);
        }
        // Each hole goes to the smallest outer that holds it. A hole lies
        // wholly inside or wholly outside each outer (no loop crosses
        // another), so one of its vertices that is not on the outer decides
        // — a vertex, not an edge midpoint: a hole may run along another
        // face's boundary (the strip between two imprints shares its sides
        // with the hole around them), where a midpoint is on both.
        let ring = |c: &[HalfEdgeId], o: &Object| -> Vec<Point3> {
            c.iter()
                .map(|&h| o.vertices[o.half_edges[h].origin].position)
                .collect()
        };
        let outer_rings: Vec<Vec<Point3>> = outers.iter().map(|(c, _)| ring(c, &obj)).collect();
        let outer_verts: Vec<BTreeSet<VertexId>> = outers
            .iter()
            .map(|(c, _)| c.iter().map(|&h| obj.half_edges[h].origin).collect())
            .collect();
        let mut holes_of: Vec<Vec<usize>> = vec![Vec::new(); outers.len()];
        for (hi, hole) in holes.iter().enumerate() {
            let holds = |k: usize| {
                hole.iter()
                    .map(|&h| obj.half_edges[h].origin)
                    .find(|x| !outer_verts[k].contains(x))
                    .is_some_and(|x| {
                        point_inside_polygon(obj.vertices[x].position, &outer_rings[k], normal)
                    })
            };
            let owner = (0..outers.len())
                .filter(|&k| holds(k))
                .min_by(|&x, &y| outers[x].1.total_cmp(&outers[y].1))
                .ok_or(StickyError::WouldCorrupt)?;
            holes_of[owner].push(hi);
        }
        // Loops of one face may share isolated vertices, never an edge: no
        // edge may have both sides on one result face.
        for (k, (cycle, _)) in outers.iter().enumerate() {
            let hs: BTreeSet<HalfEdgeId> = cycle
                .iter()
                .chain(holes_of[k].iter().flat_map(|&hi| holes[hi].iter()))
                .copied()
                .collect();
            if hs
                .iter()
                .any(|&h| obj.half_edges[h].twin.is_some_and(|t| hs.contains(&t)))
            {
                return Err(bridge_err);
            }
        }

        // 7. Faces: each result face takes over a region face it came from.
        let mut reused: BTreeSet<FaceId> = BTreeSet::new();
        let mut result_faces: Vec<FaceId> = Vec::with_capacity(outers.len());
        let mut assignments: Vec<(FaceId, Option<crate::topo::SurfaceRef>, bool)> = Vec::new();
        for (k, (cycle, _)) in outers.iter().enumerate() {
            let origins: BTreeSet<FaceId> = cycle
                .iter()
                .chain(holes_of[k].iter().flat_map(|&hi| holes[hi].iter()))
                .filter_map(|h| old_face.get(h).copied())
                .collect();
            let survivor = if origins.contains(&keep) {
                keep
            } else {
                origins.iter().next().copied().unwrap_or(face)
            };
            let first_surface = obj.faces[survivor].surface;
            let agree = origins
                .iter()
                .all(|&f| obj.faces[f].surface == first_surface);
            let surface = if agree { first_surface } else { None };
            let reuse = reused.insert(survivor);
            assignments.push((survivor, surface, reuse));
        }
        // Old loops go; every cycle gets a fresh loop.
        let old_faces: Vec<FaceId> = region.clone();
        let templates: BTreeMap<FaceId, Face> = old_faces
            .iter()
            .map(|&f| (f, obj.faces[f].clone()))
            .collect();
        for &l in &region_loops {
            obj.loops.remove(l);
        }
        for (k, (cycle, _)) in outers.iter().enumerate() {
            let (survivor, surface, reuse) = assignments[k];
            let template = &templates[&survivor];
            let fid = if reuse {
                survivor
            } else {
                let fid = obj.faces.insert(Face {
                    outer_loop: LoopId::default(),
                    inner_loops: Vec::new(),
                    plane: template.plane,
                    material: template.material,
                    back_material: template.back_material,
                    uv_frame: template.uv_frame,
                    surface: template.surface,
                    attrs: template.attrs,
                });
                for shell in obj.shells.values_mut() {
                    if shell.faces.contains(&survivor) {
                        shell.faces.push(fid);
                    }
                }
                fid
            };
            let outer = obj.loops.insert(Loop {
                face: fid,
                first_half_edge: cycle[0],
                kind: LoopKind::Outer,
            });
            let seq: Vec<HalfEdgeId> = cycle.clone();
            wire_loop_sequence(&mut obj, &seq, outer, fid);
            let mut inner = Vec::with_capacity(holes_of[k].len());
            for &hi in &holes_of[k] {
                let l = obj.loops.insert(Loop {
                    face: fid,
                    first_half_edge: holes[hi][0],
                    kind: LoopKind::Inner,
                });
                wire_loop_sequence(&mut obj, &holes[hi], l, fid);
                inner.push(l);
            }
            let f = &mut obj.faces[fid];
            f.outer_loop = outer;
            f.inner_loops = inner;
            f.plane = template.plane;
            f.material = template.material;
            f.back_material = template.back_material;
            f.uv_frame = template.uv_frame;
            f.attrs = template.attrs;
            f.surface = surface;
            result_faces.push(fid);
        }
        for &f in &old_faces {
            if !result_faces.contains(&f) {
                obj.faces.remove(f);
                for shell in obj.shells.values_mut() {
                    shell.faces.retain(|&x| x != f);
                }
            }
        }

        // 8. Orphaned vertices of the removed chains, then heal each
        //    surviving chain end left a straight two-edge scar.
        let in_use: BTreeSet<VertexId> = obj.half_edges.values().map(|he| he.origin).collect();
        for &x in &touched {
            if !in_use.contains(&x) {
                obj.vertices.remove(x);
            }
        }
        repair_dangling_outgoing(&mut obj);
        let mut healed: Vec<EdgeId> = Vec::new();
        for &x in &touched {
            if !obj.vertices.contains_key(x) {
                continue;
            }
            let leaving: Vec<HalfEdgeId> = obj
                .half_edges
                .iter()
                .filter(|(_, he)| he.origin == x)
                .map(|(h, _)| h)
                .collect();
            if leaving.len() != 2 {
                continue;
            }
            let Some(l) = leaving
                .iter()
                .map(|&h| obj.half_edges[h].loop_id)
                .find(|l| result_faces.contains(&obj.loops[*l].face))
            else {
                continue;
            };
            healed.extend(heal_run_endpoints(&mut obj, l, [x, x]));
        }
        repair_dangling_outgoing(&mut obj);

        // 9. Undo's attribute restoration, then the always-on validator.
        for r in restore {
            if let Some(&f) = result_faces
                .iter()
                .find(|&&f| obj.face_contains_point(f, r.point))
            {
                obj.faces[f].set_attrs(r.attrs);
            }
        }
        obj.watertight = if obj.half_edges.values().all(|he| he.twin.is_some()) {
            WatertightState::Watertight
        } else {
            WatertightState::Open
        };
        obj.check_invariants();
        obj.validate().map_err(|_| StickyError::WouldCorrupt)?;
        *self = obj;
        let mut removed_out = removed_edges;
        removed_out.extend(healed);
        Ok(FaceRetraceReport {
            faces: result_faces,
            new_edges,
            removed_edges: removed_out,
            prior_attrs,
            inserted,
            removed: removed_paths,
        })
    }

    /// The live edge whose endpoints match `a` and `b` (either way round)
    /// within [`tol::POINT_MERGE`], with its endpoints' exact positions in
    /// the order given.
    pub(crate) fn edge_matching(&self, a: Point3, b: Point3) -> Option<(EdgeId, Point3, Point3)> {
        self.edges.keys().find_map(|e| {
            let (p, q) = self.edge_endpoints(e)?;
            if p.approx_eq(a, tol::POINT_MERGE) && q.approx_eq(b, tol::POINT_MERGE) {
                Some((e, p, q))
            } else if p.approx_eq(b, tol::POINT_MERGE) && q.approx_eq(a, tol::POINT_MERGE) {
                Some((e, q, p))
            } else {
                None
            }
        })
    }
}

/// Splits a traced closed walk (half-edges in `next` order) into simple
/// loops at every vertex it visits more than once: each stretch between two
/// visits of one vertex closes on its own. Each piece keeps the walk's
/// order, so a piece is a closed `next` sequence; the pieces partition the
/// walk. Deterministic (a function of the walk's order alone).
pub(crate) fn split_at_repeated_vertices(
    obj: &Object,
    walk: Vec<HalfEdgeId>,
) -> Vec<Vec<HalfEdgeId>> {
    let mut pieces: Vec<Vec<HalfEdgeId>> = Vec::new();
    let mut stack: Vec<HalfEdgeId> = Vec::with_capacity(walk.len());
    // Where on the stack each origin vertex sits.
    let mut at: BTreeMap<VertexId, usize> = BTreeMap::new();
    for h in walk {
        let v = obj.half_edges[h].origin;
        if let Some(&i) = at.get(&v) {
            // The stack from `i` runs v → … → v: one closed piece.
            let piece: Vec<HalfEdgeId> = stack.split_off(i);
            for &p in &piece {
                at.remove(&obj.half_edges[p].origin);
            }
            pieces.push(piece);
        }
        at.insert(v, stack.len());
        stack.push(h);
    }
    pieces.push(stack);
    pieces
}

/// Whether `p` lies within [`tol::POINT_MERGE`] of the open interior of
/// segment `a`–`b` — `split_face`'s edge-hit classification, parameter and
/// all, so an end lands where the cut primitives would put it.
fn on_open_edge(p: Point3, a: Point3, b: Point3) -> bool {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < tol::POINT_MERGE * tol::POINT_MERGE {
        return false;
    }
    let t = (p - a).dot(ab) / len_sq;
    t > tol::POINT_MERGE
        && t < 1.0 - tol::POINT_MERGE
        && (p - (a + ab * t)).length_squared() <= tol::POINT_MERGE * tol::POINT_MERGE
}

/// Whether inserted segments `a`–`b` and `c`–`d` stay clear of each other,
/// or meet only at `allowed` — an end of both, met end to end without
/// running along each other there.
fn meet_only_at(a: Point3, b: Point3, c: Point3, d: Point3, allowed: Option<Point3>) -> bool {
    if let Some(x) = allowed {
        let far1 = if bits(a) == bits(x) { b } else { a };
        let far2 = if bits(c) == bits(x) { d } else { c };
        let shares = [a, b].iter().any(|&p| bits(p) == bits(x))
            && [c, d].iter().any(|&p| bits(p) == bits(x));
        if shares {
            return !point_near_segment(far1, c, d, tol::POINT_MERGE)
                && !point_near_segment(far2, a, b, tol::POINT_MERGE);
        }
    }
    !segments_intersect(a, b, c, d)
}

/// A point's exact bits, for "the very same point" comparisons.
fn bits(p: Point3) -> [u64; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

/// Lexicographic `a < b` on `(x, y, z)`.
fn lex_less(a: Point3, b: Point3) -> bool {
    lex_cmp(a, b) == std::cmp::Ordering::Less
}

fn lex_cmp(a: Point3, b: Point3) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x)
        .then(a.y.total_cmp(&b.y))
        .then(a.z.total_cmp(&b.z))
}

impl Object {
    /// The faces coplanar with `seed` that it is connected to: `seed` plus
    /// every face reached from it across a shared edge whose other side is
    /// coplanar with `seed` (every outer vertex within [`tol::PLANE_DIST`]
    /// of its plane) and faces the same way (normals within
    /// [`tol::NORMAL_DIRECTION`]). Breadth-first in loop and half-edge
    /// order, so deterministic. Reaches drawn sub-faces through their
    /// parent's hole loops and split siblings through outer loops.
    pub(crate) fn coplanar_face_group(&self, seed: FaceId) -> Vec<FaceId> {
        let plane = self.faces[seed].plane;
        let normal = plane.normal();
        let mut group = vec![seed];
        let mut member: BTreeSet<FaceId> = BTreeSet::from([seed]);
        let mut next = 0;
        while next < group.len() {
            let f = group[next];
            next += 1;
            let face = &self.faces[f];
            for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
                for h in self.loop_half_edges(l) {
                    let Some(t) = self.half_edges[h].twin else {
                        continue;
                    };
                    let g = self.loops[self.half_edges[t].loop_id].face;
                    if member.contains(&g) {
                        continue;
                    }
                    let gf = &self.faces[g];
                    let same_facing = (gf.plane.normal() - normal).length() < tol::NORMAL_DIRECTION;
                    if same_facing
                        && self
                            .loop_positions(gf.outer_loop)
                            .all(|p| plane.signed_distance(p).abs() <= tol::PLANE_DIST)
                    {
                        group.push(g);
                        member.insert(g);
                    }
                }
            }
        }
        group
    }
}
