//! Read-only queries the blend engine and the Fillet tool share: edge
//! convexity and dihedral, the sharp edges of an object, logical faces and
//! logical edges (with tangent-chain propagation), and the sharp edges of a
//! face's loop.

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use super::Convexity;
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Point3, Vec3};
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::{Object, SurfaceRef};

/// The geometry of one interior edge as the blend engine reads it, with
/// side A the face of `edge.half_edge`.
pub(crate) struct EdgeFrame {
    pub a: FaceId,
    pub b: FaceId,
    /// Unit direction of the half-edge on A.
    pub d_a: Vec3,
    /// Into face A, perpendicular to the edge.
    pub t_a: Vec3,
    /// Into face B, perpendicular to the edge.
    pub t_b: Vec3,
}

/// The most a scan's seam may turn at a vertex and still be one seam
/// ([`Object::logical_edge`] on an imported mesh).
const SCAN_SEAM_TURN: f64 = 30.0 * std::f64::consts::PI / 180.0;
/// A turn at one vertex this sharp, between two edges of comparable
/// length, ends a scan's seam whatever the longer measure says: a step
/// drawn into an imported model.
const SCAN_SEAM_STEP: f64 = 60.0 * std::f64::consts::PI / 180.0;
/// How much shorter than its neighbor an edge may be and still count as
/// comparable for [`SCAN_SEAM_STEP`]: shorter is a cut's sliver.
const SCAN_SEAM_SLIVER: f64 = 0.05;
/// The least part of the whole boundary an edge must be for a sharp turn
/// next to it to end a scan's seam: a scan's facets are far smaller than
/// its seams.
const SCAN_SEAM_FEATURE: f64 = 0.01;

thread_local! {
    /// How many vertex-to-edge adjacency maps ([`Object::vertex_edges`])
    /// this thread has built (test support).
    static ADJACENCY_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The work the logical-edge queries have done on this thread: the
    /// edges each chain step examined and the faces and half-edges each
    /// logical face took to find (test support).
    static CHAIN_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts chain work in a build with debug assertions, which is where the
/// specs that read the counter run; an optimized build counts nothing, so
/// the queries do not pay for it.
#[inline]
fn add_chain_work(n: usize) {
    if cfg!(debug_assertions) {
        CHAIN_WORK.with(|c| c.set(c.get() + n));
    }
}

/// A key whose every comparison counts as chain work: the membership sets
/// and maps of the logical-edge queries hold these, so the work counter
/// sees each comparison a lookup makes, whatever kind of set makes it (a
/// sorted set's few, or every element of a linear search). Orders and
/// compares exactly as `K`.
#[derive(Clone, Copy, Debug)]
struct Tally<K>(K);

impl<K: PartialEq> PartialEq for Tally<K> {
    fn eq(&self, other: &Self) -> bool {
        add_chain_work(1);
        self.0 == other.0
    }
}

impl<K: Eq> Eq for Tally<K> {}

impl<K: Ord> PartialOrd for Tally<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord> Ord for Tally<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        add_chain_work(1);
        self.0.cmp(&other.0)
    }
}

/// How many vertex-to-edge adjacency maps the calling thread has built.
/// Test support only.
#[doc(hidden)]
pub fn adjacency_builds_for_test() -> usize {
    ADJACENCY_BUILDS.with(std::cell::Cell::get)
}

/// The work the logical-edge queries ([`Object::logical_edge`],
/// [`Object::picked_edge`], [`Object::face_loop_edges`]) and
/// [`Object::logical_face`] have done on the calling thread, counted in
/// elements examined; always zero in a build without debug assertions.
/// Test support only.
#[doc(hidden)]
pub fn chain_query_work_for_test() -> usize {
    CHAIN_WORK.with(std::cell::Cell::get)
}

impl Object {
    /// The two faces of an edge: the face of its `half_edge`, then its twin's.
    pub(crate) fn edge_faces(&self, edge: EdgeId) -> Option<(FaceId, FaceId)> {
        let e = self.edges().get(edge)?;
        let twin = e.twin_half_edge?;
        let face = |h| self.loops()[self.half_edges()[h].loop_id].face;
        Some((face(e.half_edge), face(twin)))
    }

    /// Test hook for [`Object::edge_faces`].
    #[doc(hidden)]
    pub fn edge_faces_for_test(&self, edge: EdgeId) -> Option<(FaceId, FaceId)> {
        self.edge_faces(edge)
    }

    /// The frame of an interior edge with side A = `a` (which must be one of
    /// its two faces): `None` for a boundary edge, a degenerate edge, or a
    /// face that is not one of the edge's.
    pub(crate) fn edge_frame(&self, edge: EdgeId, a: FaceId) -> Option<EdgeFrame> {
        let e = self.edges().get(edge)?;
        let twin = e.twin_half_edge?;
        let (fa, fb) = self.edge_faces(edge)?;
        let (ha, b) = if a == fa {
            (e.half_edge, fb)
        } else if a == fb {
            (twin, fa)
        } else {
            return None;
        };
        let he = self.half_edges()[ha];
        let p0 = self.vertices()[he.origin].position;
        let p1 = self.vertices()[self.half_edges()[he.next].origin].position;
        let d_a = (p1 - p0).normalized().ok()?;
        let n_a = self.faces()[a].plane.normal();
        let n_b = self.faces()[b].plane.normal();
        let t_a = n_a.cross(d_a).normalized().ok()?;
        let t_b = n_b.cross(-d_a).normalized().ok()?;
        Some(EdgeFrame {
            a,
            b,
            d_a,
            t_a,
            t_b,
        })
    }

    /// Convexity of an interior edge between two non-coplanar faces, or
    /// `None` for a boundary edge, a smooth edge (`edge_smoothness` not
    /// `Hard`), or two coplanar faces (a drawn imprint edge).
    pub fn edge_convexity(&self, edge: EdgeId) -> Option<Convexity> {
        if self.edge_smoothness(edge) != EdgeSmoothness::Hard {
            return None;
        }
        let (a, b) = self.edge_faces(edge)?;
        let (n_a, n_b) = (
            self.faces()[a].plane.normal(),
            self.faces()[b].plane.normal(),
        );
        if n_a.dot(n_b) >= 1.0 - tol::NORMAL_DIRECTION {
            return None;
        }
        let f = self.edge_frame(edge, a)?;
        Some(if f.t_a.dot(n_b) < 0.0 {
            Convexity::Convex
        } else {
            Convexity::Concave
        })
    }

    /// The dihedral of a sharp edge, `θ ∈ (0, π)`: the angle between the two
    /// in-face directions away from the edge (through the material for a
    /// convex edge, through the air for a concave one). `None` where
    /// [`Object::edge_convexity`] is `None`.
    pub fn edge_dihedral(&self, edge: EdgeId) -> Option<f64> {
        self.edge_convexity(edge)?;
        let (a, _) = self.edge_faces(edge)?;
        let f = self.edge_frame(edge, a)?;
        Some(trig::acos(f.t_a.dot(f.t_b).clamp(-1.0, 1.0)))
    }

    /// Every sharp interior edge (one with a convexity), in slot order.
    pub fn sharp_edges(&self) -> Vec<EdgeId> {
        self.edges()
            .keys()
            .filter(|&e| self.edge_convexity(e).is_some())
            .collect()
    }

    /// The logical face of `face` (true-curves.md §1.1): every face with the
    /// same blend id if it is tagged; else every face whose claim is
    /// `same_surface` if it is claimed; else the unclaimed faces it reaches
    /// across soft edges (a revolved base's wedges, a lofted wall, a
    /// smoothed import), never across a hard edge such as a drawn line.
    /// Slot order.
    pub fn logical_face(&self, face: FaceId) -> Vec<FaceId> {
        let Some(f) = self.faces().get(face) else {
            return Vec::new();
        };
        if let Some(t) = f.blend {
            add_chain_work(self.faces().len());
            return self
                .faces()
                .iter()
                .filter(|(_, g)| g.blend.is_some_and(|u| u.id == t.id))
                .map(|(id, _)| id)
                .collect();
        }
        if let Some(s) = f.surface {
            add_chain_work(self.faces().len());
            return self
                .faces()
                .iter()
                .filter(|(_, g)| g.surface.is_some_and(|u| u.same_surface(&s)))
                .map(|(id, _)| id)
                .collect();
        }
        // Unclaimed neighbors across non-hard edges: one smooth surface.
        let plain = |g: FaceId| {
            self.faces()
                .get(g)
                .is_some_and(|x| x.surface.is_none() && x.blend.is_none())
        };
        let mut seen: BTreeSet<Tally<FaceId>> = BTreeSet::from([Tally(face)]);
        let mut stack = vec![face];
        while let Some(g) = stack.pop() {
            for l in self.face_loops(g) {
                for h in self.loop_half_edges(l) {
                    add_chain_work(1);
                    let e = self.half_edges()[h].edge;
                    if self.edge_smoothness(e) == EdgeSmoothness::Hard {
                        continue;
                    }
                    let Some((x, y)) = self.edge_faces(e) else {
                        continue;
                    };
                    let other = if x == g { y } else { x };
                    if !seen.contains(&Tally(other)) && plain(other) {
                        seen.insert(Tally(other));
                        stack.push(other);
                    }
                }
            }
        }
        seen.into_iter().map(|t| t.0).collect()
    }

    /// The edges incident to `v` (either endpoint), slot order: the entry
    /// [`Object::vertex_edges`] has for `v`, without building the map for
    /// every vertex (empty where the map has none).
    pub(crate) fn edges_at_vertex(&self, v: VertexId) -> Vec<EdgeId> {
        self.edges()
            .iter()
            .filter(|(_, e)| {
                let he = self.half_edges()[e.half_edge];
                he.origin == v || self.half_edges()[he.next].origin == v
            })
            .map(|(eid, _)| eid)
            .collect()
    }

    /// Every edge incident to each vertex (both endpoints), slot order.
    pub(crate) fn vertex_edges(&self) -> BTreeMap<VertexId, Vec<EdgeId>> {
        ADJACENCY_BUILDS.with(|c| c.set(c.get() + 1));
        self.vertex_edges_map()
    }

    fn vertex_edges_map(&self) -> BTreeMap<VertexId, Vec<EdgeId>> {
        let mut m: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
        for (eid, e) in self.edges().iter() {
            let he = self.half_edges()[e.half_edge];
            m.entry(he.origin).or_default().push(eid);
            m.entry(self.half_edges()[he.next].origin)
                .or_default()
                .push(eid);
        }
        m
    }

    /// The logical edge containing `edge` (true-curves.md §1.1): the maximal
    /// chain of sharp edges separating the same two logical faces, walked
    /// from `edge` through each end vertex while exactly one other sharp
    /// edge there separates the same pair. With `propagate`, the walk also
    /// continues along a tangent chain (§4.2): at a stop vertex where exactly
    /// one other sharp edge shares a logical face with the current one,
    /// every other edge there is smooth, and the two side faces meet
    /// smoothly (a tangent or seam edge between them at the vertex, or one
    /// logical face). No angle tolerance is involved.
    ///
    /// Ordered from one end to the other; a closed chain starts at its lowest
    /// `EdgeId`. Empty if `edge` is not sharp. The walk is bounded by the
    /// object's edge count (a bug guard).
    pub fn logical_edge(&self, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
        if self.edge_convexity(edge).is_none() {
            return Vec::new();
        }
        ChainQuery::new(self).logical_edge(edge, propagate)
    }

    /// The edges a click on `edge` picks: its [`Object::logical_edge`],
    /// and with `propagate` on an imported mesh (no curves; soft only where
    /// its facets bend gently) the longer seam a user sees there, when it
    /// is longer (see `scan_seam`). For the pick only: the kernel's own
    /// re-expansions of edge sets it already holds keep `logical_edge`.
    pub fn picked_edge(&self, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
        if self.edge_convexity(edge).is_none() {
            return Vec::new();
        }
        let mut query = ChainQuery::new(self);
        let out = query.logical_edge(edge, propagate);
        if !propagate || self.planarity_tol <= crate::tol::PLANE_DIST || out.is_empty() {
            return out;
        }
        let ve = &query.ve;
        let mut cache: BTreeMap<FaceId, std::rc::Rc<BTreeSet<FaceId>>> = BTreeMap::new();
        let mut lf = |f: FaceId| -> std::rc::Rc<BTreeSet<FaceId>> {
            if let Some(set) = cache.get(&f) {
                return set.clone();
            }
            let set = std::rc::Rc::new(self.logical_face(f).into_iter().collect::<BTreeSet<_>>());
            if self
                .faces()
                .get(f)
                .is_some_and(|x| x.blend.is_some() || x.surface.is_none())
            {
                for &g in set.iter() {
                    cache.insert(g, set.clone());
                }
            }
            cache.insert(f, set.clone());
            set
        };
        match self.scan_seam(edge, ve, &mut lf) {
            Some(arc) if arc.len() > out.len() => arc,
            _ => out,
        }
    }

    /// On an imported mesh (no curves, and soft edges only where its
    /// facets bend gently), the seam through `edge` as a user sees it: the
    /// boundary of the logical face on one side of it, followed while it
    /// stays sharp with `edge`'s convexity and turns no more than
    /// [`SCAN_SEAM_TURN`] (measured over two edges' length either side, or
    /// a fiftieth of the boundary where that is longer, so neither a sliver
    /// edge a cut leaves nor the zigzag of a noisy scan is a corner),
    /// whatever faces the other
    /// side passes through. The longer of the two sides' arcs; a closed
    /// loop starts at its lowest `EdgeId`.
    fn scan_seam(
        &self,
        edge: EdgeId,
        ve: &VertexEdges,
        lf: &mut dyn FnMut(FaceId) -> std::rc::Rc<BTreeSet<FaceId>>,
    ) -> Option<Vec<EdgeId>> {
        let (fa, fb) = self.edge_faces(edge)?;
        let convexity = self.edge_convexity(edge);
        let ends = |e: EdgeId| -> (VertexId, VertexId) {
            let he = self.half_edges()[self.edges()[e].half_edge];
            (he.origin, self.half_edges()[he.next].origin)
        };
        let pos = |v: VertexId| self.vertices()[v].position;
        let mut best: Option<Vec<EdgeId>> = None;
        for side in [lf(fa), lf(fb)] {
            // The side's boundary through `edge`, as a chain of vertices.
            let on_boundary = |o: EdgeId| {
                self.edge_faces(o)
                    .is_some_and(|(x, y)| side.contains(&x) != side.contains(&y))
            };
            let follow = |from: EdgeId, through: VertexId| -> (Vec<(EdgeId, VertexId)>, bool) {
                let mut out = Vec::new();
                let (mut cur, mut v) = (from, through);
                for _ in 0..self.edges().len() {
                    let next: Vec<EdgeId> = ve
                        .get(v)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|&o| o != cur && on_boundary(o))
                        .collect();
                    // Where the side's boundary meets itself at a vertex (a
                    // scan's sliver), the straightest way on.
                    let away = |e: EdgeId| {
                        let (a, b) = ends(e);
                        let w = if a == v { b } else { a };
                        (pos(w) - pos(v)).normalized().ok()
                    };
                    let Some(back) = away(cur) else {
                        return (out, false);
                    };
                    let Some(n) = next
                        .iter()
                        .copied()
                        .filter_map(|o| away(o).map(|d| (o, d.dot(back))))
                        .min_by(|x, y| x.1.total_cmp(&y.1))
                        .map(|(o, _)| o)
                    else {
                        return (out, false);
                    };
                    if n == from {
                        return (out, true);
                    }
                    let (a, b) = ends(n);
                    v = if a == v { b } else { a };
                    out.push((n, v));
                    cur = n;
                }
                (out, false)
            };
            let (p, q) = ends(edge);
            let (fwd, closed) = follow(edge, q);
            // Edges in order, and the vertex before each.
            let mut chain: Vec<(EdgeId, VertexId)> = Vec::new();
            if !closed {
                // Backward edges, reversed: each with the vertex it starts
                // at (the one the backward walk reached through it).
                let (back, _) = follow(edge, p);
                for &(e, v) in back.iter().rev() {
                    chain.push((e, v));
                }
            }
            chain.push((edge, p));
            let mut v = q;
            for &(e, w) in &fwd {
                chain.push((e, v));
                v = w;
            }
            let n = chain.len();
            let start = |i: usize| chain[i].1;
            let finish = |i: usize| {
                let (a, b) = ends(chain[i].0);
                if a == chain[i].1 { b } else { a }
            };
            let len = |i: usize| (pos(finish(i)) - pos(start(i))).length();
            let total: f64 = (0..n).map(len).sum();
            // The point arc length `d` back from (forward of) the start of
            // edge `i`, walking the chain.
            let along = |i: usize, d: f64, forward: bool| -> Point3 {
                let mut left = d;
                let mut k = i;
                loop {
                    if !forward && !closed && k == 0 {
                        // Back past an open boundary's first edge: its end.
                        return pos(start(0));
                    }
                    let (a, b, l) = if forward {
                        (start(k), finish(k), len(k))
                    } else {
                        let j = (k + n - 1) % n;
                        (finish(j), start(j), len(j))
                    };
                    if left <= l || l <= 0.0 && left <= 0.0 {
                        let t = if l > 0.0 { left / l } else { 0.0 };
                        return pos(a) + (pos(b) - pos(a)) * t;
                    }
                    left -= l;
                    let at_end = if forward { k + 1 >= n } else { k == 0 };
                    if at_end && !closed {
                        return pos(b);
                    }
                    k = if forward {
                        (k + 1) % n
                    } else {
                        (k + n - 1) % n
                    };
                }
            };
            // Whether the chain may pass from edge `i - 1` into edge `i`.
            // Whether edge `i` is a seam edge like `edge`.
            let like = |i: usize| self.edge_convexity(chain[i].0) == convexity;
            // Whether the seam goes on straight enough through the vertex
            // edge `i` starts at: measured over two edges' length (or a
            // fiftieth of the boundary) either side, and no sharp turn
            // between two edges of comparable length there, each a real part
            // of the boundary (a step drawn into an imported model, which the
            // long measure would smooth away; neither a sliver a cut leaves
            // nor a scan's facet-sized zigzag is such an edge).
            let through = |i: usize| -> bool {
                let j = (i + n - 1) % n;
                if !closed && i == 0 {
                    return false;
                }
                let l = (2.0 * len(i).max(len(j))).max(0.02 * total);
                let at = pos(start(i));
                let (Ok(d_in), Ok(d_out)) = (
                    (at - along(i, l, false)).normalized(),
                    (along(i, l, true) - at).normalized(),
                ) else {
                    return false;
                };
                if d_in.dot(d_out) < trig::cos(SCAN_SEAM_TURN) {
                    return false;
                }
                let (Ok(e_in), Ok(e_out)) = (
                    (at - pos(start(j))).normalized(),
                    (pos(finish(i)) - at).normalized(),
                ) else {
                    return true;
                };
                let short = len(i).min(len(j));
                let comparable = short >= SCAN_SEAM_SLIVER * len(i).max(len(j))
                    && short >= SCAN_SEAM_FEATURE * total;
                !(comparable && e_in.dot(e_out) < trig::cos(SCAN_SEAM_STEP))
            };
            let me = chain.iter().position(|&(e, _)| e == edge)?;
            if closed && (0..n).all(|i| like(i) && through(i)) {
                let mut ring: Vec<EdgeId> = chain.iter().map(|&(e, _)| e).collect();
                let k = (0..ring.len()).min_by_key(|&i| ring[i]).unwrap_or(0);
                ring.rotate_left(k);
                return Some(ring);
            }
            // Out from `edge` both ways to the corners (round the loop's
            // start when it is closed): an edge joins when it is like
            // `edge` and the seam goes through the vertex between.
            let step = |i: usize, by: isize| -> Option<usize> {
                let j = i as isize + by;
                if closed {
                    Some(j.rem_euclid(n as isize) as usize)
                } else {
                    (0..n as isize).contains(&j).then_some(j as usize)
                }
            };
            let mut arc: std::collections::VecDeque<EdgeId> =
                std::collections::VecDeque::from([edge]);
            let (mut lo, mut hi) = (me, me);
            while arc.len() < n {
                match step(hi, 1) {
                    Some(j) if like(j) && through(j) => {
                        hi = j;
                        arc.push_back(chain[j].0);
                    }
                    _ => break,
                }
            }
            while arc.len() < n {
                match step(lo, -1) {
                    Some(j) if like(j) && through(lo) => {
                        lo = j;
                        arc.push_front(chain[j].0);
                    }
                    _ => break,
                }
            }
            let arc: Vec<EdgeId> = arc.into_iter().collect();
            if best.as_ref().is_none_or(|b| arc.len() > b.len()) {
                best = Some(arc);
            }
        }
        best
    }

    /// The sharp edges of the loop of `face` nearest to `near` (its outer
    /// loop or a hole), each expanded with [`Object::logical_edge`]
    /// (propagating), deduplicated, in loop order. A facet of a larger
    /// logical face (a cylinder wall, a blend) answers for that face: the
    /// boundary loop of the whole logical face nearest `near`, so a click on
    /// one wall facet picks the rim it is near, not both rims its facet
    /// touches.
    pub fn face_loop_edges(&self, face: FaceId, near: Point3) -> Vec<EdgeId> {
        let Some(f) = self.faces().get(face) else {
            return Vec::new();
        };
        let logical = self.logical_face(face);
        if logical.len() > 1 {
            return self.logical_face_loop_edges(&logical, near);
        }
        let loops: Vec<_> = std::iter::once(f.outer_loop)
            .chain(f.inner_loops.iter().copied())
            .collect();
        let dist = |l| -> f64 {
            let ring: Vec<Point3> = self.loop_positions(l).collect();
            (0..ring.len())
                .map(|i| point_segment_distance(near, ring[i], ring[(i + 1) % ring.len()]))
                .fold(f64::INFINITY, f64::min)
        };
        let Some(&best) = loops.iter().min_by(|&&x, &&y| dist(x).total_cmp(&dist(y))) else {
            return Vec::new();
        };
        let mut query = None;
        let mut out = Ordered::default();
        for h in self.loop_half_edges(best) {
            let e = self.half_edges()[h].edge;
            if out.contains(e) || self.edge_convexity(e).is_none() {
                continue;
            }
            let q = query.get_or_insert_with(|| ChainQuery::new(self));
            for x in q.logical_edge(e, true) {
                out.push(x);
            }
        }
        out.list
    }
}

impl Object {
    /// [`Object::face_loop_edges`] for a logical face of several faces: its
    /// boundary edges (those whose other face lies outside it), grouped into
    /// loops by shared vertices; the loop nearest `near`.
    fn logical_face_loop_edges(&self, logical: &[FaceId], near: Point3) -> Vec<EdgeId> {
        let inside: BTreeSet<Tally<FaceId>> = logical.iter().copied().map(Tally).collect();
        let mut boundary = Ordered::default();
        for &g in logical {
            for l in self.face_loops(g) {
                for h in self.loop_half_edges(l) {
                    let e = self.half_edges()[h].edge;
                    let outside = self.edge_faces(e).is_none_or(|(a, b)| {
                        !(inside.contains(&Tally(a)) && inside.contains(&Tally(b)))
                    });
                    if outside {
                        boundary.push(e);
                    }
                }
            }
        }
        let boundary = boundary.list;
        // Loops: boundary edges joined through shared vertices.
        let ends = |e: EdgeId| self.edge_vertex_ids(e);
        let mut group: Vec<usize> = (0..boundary.len()).collect();
        fn root(group: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while group[r] != r {
                r = group[r];
            }
            group[i] = r;
            r
        }
        let mut at: BTreeMap<Tally<VertexId>, usize> = BTreeMap::new();
        for (i, &e) in boundary.iter().enumerate() {
            let Some((a, b)) = ends(e) else { continue };
            for v in [a, b] {
                match at.get(&Tally(v)) {
                    Some(&j) => {
                        let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                        group[ri.max(rj)] = ri.min(rj);
                    }
                    None => {
                        at.insert(Tally(v), i);
                    }
                }
            }
        }
        let dist = |e: EdgeId| {
            self.edge_endpoints(e)
                .map_or(f64::INFINITY, |(p, q)| point_segment_distance(near, p, q))
        };
        let Some(nearest) =
            (0..boundary.len()).min_by(|&i, &j| dist(boundary[i]).total_cmp(&dist(boundary[j])))
        else {
            return Vec::new();
        };
        let want = root(&mut group, nearest);
        let mut query = None;
        let mut out = Ordered::default();
        for (i, &e) in boundary.iter().enumerate() {
            if root(&mut group, i) != want || out.contains(e) || self.edge_convexity(e).is_none() {
                continue;
            }
            let q = query.get_or_insert_with(|| ChainQuery::new(self));
            for x in q.logical_edge(e, true) {
                out.push(x);
            }
        }
        out.list
    }
}

/// A face's logical face as a chain walk asks it: whether a face is a
/// member. A blend face's and a plain face's are the set itself; a claimed
/// face's is every face whose claim is `same_surface` with its own, asked
/// of the face in question rather than gathered from every face up front
/// (the same answer as [`Object::logical_face`]'s list).
#[derive(Clone)]
enum LogicalSide<'a> {
    Set(Rc<BTreeSet<Tally<FaceId>>>),
    Claimed(&'a Object, SurfaceRef),
}

impl LogicalSide<'_> {
    fn contains(&self, g: &FaceId) -> bool {
        match self {
            LogicalSide::Set(set) => set.contains(&Tally(*g)),
            LogicalSide::Claimed(obj, s) => {
                add_chain_work(1);
                obj.faces()
                    .get(*g)
                    .is_some_and(|x| x.surface.is_some_and(|u| u.same_surface(s)))
            }
        }
    }
}

/// The state one hover query shares across every chain it walks: the
/// vertex-to-edge adjacency, built once, and each face's logical face,
/// found once. Every answer is the object's own, so sharing it changes
/// nothing but the cost.
pub(crate) struct ChainQuery<'a> {
    obj: &'a Object,
    /// [`Object::vertex_edges`].
    pub(crate) ve: VertexEdges,
    /// Each face's logical face. A blend face's (one blend id) and a plain
    /// face's (one smooth surface reached across soft edges) is the same
    /// set for every member, so it is filed under all of them at once: a
    /// softened import is one logical face of hundreds of thousands of
    /// facets, and flooding it again for every facet along a seam took
    /// minutes. A claimed face's is filed under itself only, since claims
    /// match within a tolerance.
    sides: BTreeMap<Tally<FaceId>, LogicalSide<'a>>,
}

impl<'a> ChainQuery<'a> {
    pub(crate) fn new(obj: &'a Object) -> ChainQuery<'a> {
        ChainQuery {
            obj,
            ve: VertexEdges::of(obj),
            sides: BTreeMap::new(),
        }
    }

    fn side(&mut self, f: FaceId) -> LogicalSide<'a> {
        if let Some(side) = self.sides.get(&Tally(f)) {
            return side.clone();
        }
        let obj = self.obj;
        let face = obj.faces().get(f);
        if let Some(s) = face.filter(|x| x.blend.is_none()).and_then(|x| x.surface) {
            let side = LogicalSide::Claimed(obj, s);
            self.sides.insert(Tally(f), side.clone());
            return side;
        }
        let set = Rc::new(
            obj.logical_face(f)
                .into_iter()
                .map(Tally)
                .collect::<BTreeSet<_>>(),
        );
        if face.is_some_and(|x| x.blend.is_some() || x.surface.is_none()) {
            for &g in set.iter() {
                self.sides.insert(g, LogicalSide::Set(set.clone()));
            }
        }
        self.sides.insert(Tally(f), LogicalSide::Set(set.clone()));
        LogicalSide::Set(set)
    }

    fn ends(&self, e: EdgeId) -> (VertexId, VertexId) {
        let obj = self.obj;
        let he = obj.half_edges()[obj.edges()[e].half_edge];
        (he.origin, obj.half_edges()[he.next].origin)
    }

    /// One step of [`Object::logical_edge`]'s walk from `cur` through
    /// vertex `v`, or None at a stop.
    fn step(&mut self, cur: EdgeId, v: VertexId, propagate: bool) -> Option<EdgeId> {
        let obj = self.obj;
        let (fa, fb) = obj.edge_faces(cur)?;
        let (l1, l2) = (self.side(fa), self.side(fb));
        let others: Vec<EdgeId> = self
            .ve
            .get(v)
            .map(|v| v.iter().copied().filter(|&o| o != cur).collect())
            .unwrap_or_default();
        add_chain_work(others.len() + 1);
        let sharp: Vec<EdgeId> = others
            .iter()
            .copied()
            .filter(|&o| obj.edge_convexity(o).is_some())
            .collect();
        let same: Vec<EdgeId> = sharp
            .iter()
            .copied()
            .filter(|&o| {
                obj.edge_faces(o).is_some_and(|(ga, gb)| {
                    (l1.contains(&ga) && l2.contains(&gb)) || (l1.contains(&gb) && l2.contains(&ga))
                })
            })
            .collect();
        if same.len() == 1 {
            return Some(same[0]);
        }
        if !propagate || !same.is_empty() {
            return None;
        }
        // Tangent propagation.
        let sharing: Vec<EdgeId> = sharp
            .iter()
            .copied()
            .filter(|&o| {
                obj.edge_faces(o).is_some_and(|(ga, gb)| {
                    l1.contains(&ga) || l1.contains(&gb) || l2.contains(&ga) || l2.contains(&gb)
                })
            })
            .collect();
        if sharing.len() != 1 || sharp.len() != 1 {
            return None;
        }
        let next = sharing[0];
        let (ga, gb) = obj.edge_faces(next)?;
        // The logical face the two edges share, and the side face of
        // each that is not it.
        let (side_cur, side_next) = if l1.contains(&ga) || l1.contains(&gb) {
            (fb, if l1.contains(&ga) { gb } else { ga })
        } else {
            (fa, if l2.contains(&ga) { gb } else { ga })
        };
        if self.side(side_cur).contains(&side_next) {
            return Some(next);
        }
        let smooth_between = others.iter().any(|&o| {
            matches!(
                obj.edge_smoothness(o),
                EdgeSmoothness::Tangent | EdgeSmoothness::Seam
            ) && obj.edge_faces(o).is_some_and(|(x, y)| {
                (x == side_cur && y == side_next) || (x == side_next && y == side_cur)
            })
        });
        smooth_between.then_some(next)
    }

    /// The walk from `from` through vertex `through`: the edges it took,
    /// and whether it came back round to `edge`.
    fn walk(
        &mut self,
        edge: EdgeId,
        from: EdgeId,
        through: VertexId,
        propagate: bool,
    ) -> (Vec<EdgeId>, bool) {
        let bound = self.obj.edges().len();
        let mut out = Vec::new();
        let mut taken: BTreeSet<Tally<EdgeId>> = BTreeSet::new();
        let (mut cur, mut v) = (from, through);
        for _ in 0..bound {
            let Some(next) = self.step(cur, v, propagate) else {
                return (out, false);
            };
            if next == edge {
                return (out, true);
            }
            if !taken.insert(Tally(next)) {
                return (out, false);
            }
            out.push(next);
            let (a, b) = self.ends(next);
            v = if a == v { b } else { a };
            cur = next;
        }
        (out, false)
    }

    /// [`Object::logical_edge`] on this query's shared state.
    pub(crate) fn logical_edge(&mut self, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
        if self.obj.edge_convexity(edge).is_none() {
            return Vec::new();
        }
        let (p, q) = self.ends(edge);
        let (fwd, closed) = self.walk(edge, edge, q, propagate);
        if closed {
            let mut ring = vec![edge];
            ring.extend(fwd);
            let k = (0..ring.len()).min_by_key(|&i| ring[i]).unwrap_or(0);
            ring.rotate_left(k);
            return ring;
        }
        let (back, _) = self.walk(edge, edge, p, propagate);
        let mut out: Vec<EdgeId> = back.into_iter().rev().collect();
        out.push(edge);
        out.extend(fwd);
        out
    }
}

/// [`Object::vertex_edges`], read by vertex: for each vertex the same
/// edges in the same (slot) order, and none for a vertex no edge has. Kept
/// by the vertex's slot rather than in a sorted map, which is most of a
/// small query's cost on a large object. An object with an edge whose end
/// is not a live vertex (which validation never passes) keeps the map.
pub(crate) enum VertexEdges {
    Slots(slotmap::SecondaryMap<VertexId, Vec<EdgeId>>),
    Map(BTreeMap<VertexId, Vec<EdgeId>>),
}

impl VertexEdges {
    pub(crate) fn of(obj: &Object) -> VertexEdges {
        ADJACENCY_BUILDS.with(|c| c.set(c.get() + 1));
        let mut slots: slotmap::SecondaryMap<VertexId, Vec<EdgeId>> =
            slotmap::SecondaryMap::with_capacity(obj.vertices().capacity());
        for (eid, e) in obj.edges().iter() {
            let he = obj.half_edges()[e.half_edge];
            for v in [he.origin, obj.half_edges()[he.next].origin] {
                let entry = obj
                    .vertices()
                    .contains_key(v)
                    .then(|| slots.entry(v))
                    .flatten();
                let Some(entry) = entry else {
                    return VertexEdges::Map(obj.vertex_edges_map());
                };
                entry.or_default().push(eid);
            }
        }
        VertexEdges::Slots(slots)
    }

    /// The edges at `v`, as [`Object::vertex_edges`] has them.
    pub(crate) fn get(&self, v: VertexId) -> Option<&Vec<EdgeId>> {
        match self {
            VertexEdges::Slots(slots) => slots.get(v),
            VertexEdges::Map(map) => map.get(&v),
        }
    }
}

/// Edges in the order first pushed, each once.
#[derive(Default)]
struct Ordered {
    list: Vec<EdgeId>,
    seen: BTreeSet<Tally<EdgeId>>,
}

impl Ordered {
    fn contains(&self, e: EdgeId) -> bool {
        self.seen.contains(&Tally(e))
    }

    fn push(&mut self, e: EdgeId) {
        if self.seen.insert(Tally(e)) {
            self.list.push(e);
        }
    }
}

pub(crate) fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}
