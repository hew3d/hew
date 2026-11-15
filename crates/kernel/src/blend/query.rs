//! Read-only queries the blend engine and the Fillet tool share: edge
//! convexity and dihedral, the sharp edges of an object, logical faces and
//! logical edges (with tangent-chain propagation), and the sharp edges of a
//! face's loop.

use std::collections::{BTreeMap, BTreeSet};

use super::Convexity;
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Point3, Vec3};
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::Object;

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
        Some(f.t_a.dot(f.t_b).clamp(-1.0, 1.0).acos())
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
            return self
                .faces()
                .iter()
                .filter(|(_, g)| g.blend.is_some_and(|u| u.id == t.id))
                .map(|(id, _)| id)
                .collect();
        }
        if let Some(s) = f.surface {
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
        let mut seen: BTreeSet<FaceId> = BTreeSet::from([face]);
        let mut stack = vec![face];
        while let Some(g) = stack.pop() {
            for l in self.face_loops(g) {
                for h in self.loop_half_edges(l) {
                    let e = self.half_edges()[h].edge;
                    if self.edge_smoothness(e) == EdgeSmoothness::Hard {
                        continue;
                    }
                    let Some((x, y)) = self.edge_faces(e) else {
                        continue;
                    };
                    let other = if x == g { y } else { x };
                    if !seen.contains(&other) && plain(other) {
                        seen.insert(other);
                        stack.push(other);
                    }
                }
            }
        }
        seen.into_iter().collect()
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
        let ve = self.vertex_edges();
        // Each face's logical face, shared rather than copied. A blend
        // face's (one blend id) and a plain face's (one smooth surface
        // reached across soft edges) is the same set for every member, so it
        // is filed under all of them at once: a softened import is one
        // logical face of hundreds of thousands of facets, and flooding it
        // again for every facet along a seam took minutes. A claimed face's
        // is filed under itself only, since claims match within a tolerance.
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
        let ends = |e: EdgeId| -> (VertexId, VertexId) {
            let he = self.half_edges()[self.edges()[e].half_edge];
            (he.origin, self.half_edges()[he.next].origin)
        };
        // One step from `cur` through vertex `v`, or None at a stop.
        let mut step = |cur: EdgeId, v: VertexId| -> Option<EdgeId> {
            let (fa, fb) = self.edge_faces(cur)?;
            let (l1, l2) = (lf(fa), lf(fb));
            let others: Vec<EdgeId> = ve
                .get(&v)
                .map(|v| v.iter().copied().filter(|&o| o != cur).collect())
                .unwrap_or_default();
            let sharp: Vec<EdgeId> = others
                .iter()
                .copied()
                .filter(|&o| self.edge_convexity(o).is_some())
                .collect();
            let same: Vec<EdgeId> = sharp
                .iter()
                .copied()
                .filter(|&o| {
                    self.edge_faces(o).is_some_and(|(ga, gb)| {
                        (l1.contains(&ga) && l2.contains(&gb))
                            || (l1.contains(&gb) && l2.contains(&ga))
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
                    self.edge_faces(o).is_some_and(|(ga, gb)| {
                        l1.contains(&ga) || l1.contains(&gb) || l2.contains(&ga) || l2.contains(&gb)
                    })
                })
                .collect();
            if sharing.len() != 1 || sharp.len() != 1 {
                return None;
            }
            let next = sharing[0];
            let (ga, gb) = self.edge_faces(next)?;
            // The logical face the two edges share, and the side face of
            // each that is not it.
            let (side_cur, side_next) = if l1.contains(&ga) || l1.contains(&gb) {
                (fb, if l1.contains(&ga) { gb } else { ga })
            } else {
                (fa, if l2.contains(&ga) { gb } else { ga })
            };
            if lf(side_cur).contains(&side_next) {
                return Some(next);
            }
            let smooth_between = others.iter().any(|&o| {
                matches!(
                    self.edge_smoothness(o),
                    EdgeSmoothness::Tangent | EdgeSmoothness::Seam
                ) && self.edge_faces(o).is_some_and(|(x, y)| {
                    (x == side_cur && y == side_next) || (x == side_next && y == side_cur)
                })
            });
            smooth_between.then_some(next)
        };
        let bound = self.edges().len();
        let walk = |from: EdgeId,
                    through: VertexId,
                    step: &mut dyn FnMut(EdgeId, VertexId) -> Option<EdgeId>|
         -> (Vec<EdgeId>, bool) {
            let mut out = Vec::new();
            let (mut cur, mut v) = (from, through);
            for _ in 0..bound {
                let Some(next) = step(cur, v) else {
                    return (out, false);
                };
                if next == edge {
                    return (out, true);
                }
                if out.contains(&next) {
                    return (out, false);
                }
                out.push(next);
                let (a, b) = ends(next);
                v = if a == v { b } else { a };
                cur = next;
            }
            (out, false)
        };
        let (p, q) = ends(edge);
        let (fwd, closed) = walk(edge, q, &mut step);
        if closed {
            let mut ring = vec![edge];
            ring.extend(fwd);
            let k = (0..ring.len()).min_by_key(|&i| ring[i]).unwrap_or(0);
            ring.rotate_left(k);
            return ring;
        }
        let (back, _) = walk(edge, p, &mut step);
        let mut out: Vec<EdgeId> = back.into_iter().rev().collect();
        out.push(edge);
        out.extend(fwd);
        out
    }

    /// The edges a click on `edge` picks: its [`Object::logical_edge`],
    /// and with `propagate` on an imported mesh (no curves; soft only where
    /// its facets bend gently) the longer seam a user sees there, when it
    /// is longer (see `scan_seam`). For the pick only: the kernel's own
    /// re-expansions of edge sets it already holds keep `logical_edge`.
    pub fn picked_edge(&self, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
        let out = self.logical_edge(edge, propagate);
        if !propagate || self.planarity_tol <= crate::tol::PLANE_DIST || out.is_empty() {
            return out;
        }
        let ve = self.vertex_edges();
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
        match self.scan_seam(edge, &ve, &mut lf) {
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
        ve: &BTreeMap<VertexId, Vec<EdgeId>>,
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
                        .get(&v)
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
                if d_in.dot(d_out) < SCAN_SEAM_TURN.cos() {
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
                !(comparable && e_in.dot(e_out) < SCAN_SEAM_STEP.cos())
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
        let mut out: Vec<EdgeId> = Vec::new();
        for h in self.loop_half_edges(best) {
            let e = self.half_edges()[h].edge;
            if out.contains(&e) || self.edge_convexity(e).is_none() {
                continue;
            }
            for x in self.logical_edge(e, true) {
                if !out.contains(&x) {
                    out.push(x);
                }
            }
        }
        out
    }
}

impl Object {
    /// [`Object::face_loop_edges`] for a logical face of several faces: its
    /// boundary edges (those whose other face lies outside it), grouped into
    /// loops by shared vertices; the loop nearest `near`.
    fn logical_face_loop_edges(&self, logical: &[FaceId], near: Point3) -> Vec<EdgeId> {
        let inside: BTreeSet<FaceId> = logical.iter().copied().collect();
        let mut boundary: Vec<EdgeId> = Vec::new();
        for &g in logical {
            for l in self.face_loops(g) {
                for h in self.loop_half_edges(l) {
                    let e = self.half_edges()[h].edge;
                    let outside = self
                        .edge_faces(e)
                        .is_none_or(|(a, b)| !(inside.contains(&a) && inside.contains(&b)));
                    if outside && !boundary.contains(&e) {
                        boundary.push(e);
                    }
                }
            }
        }
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
        let mut at: BTreeMap<VertexId, usize> = BTreeMap::new();
        for (i, &e) in boundary.iter().enumerate() {
            let Some((a, b)) = ends(e) else { continue };
            for v in [a, b] {
                match at.get(&v) {
                    Some(&j) => {
                        let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                        group[ri.max(rj)] = ri.min(rj);
                    }
                    None => {
                        at.insert(v, i);
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
        let mut out: Vec<EdgeId> = Vec::new();
        for (i, &e) in boundary.iter().enumerate() {
            if root(&mut group, i) != want || out.contains(&e) || self.edge_convexity(e).is_none() {
                continue;
            }
            for x in self.logical_edge(e, true) {
                if !out.contains(&x) {
                    out.push(x);
                }
            }
        }
        out
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
