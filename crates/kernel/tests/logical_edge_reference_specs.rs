//! Differential specs for the Fillet tool's hover queries
//! ([`Object::logical_edge`], [`Object::picked_edge`],
//! [`Object::face_loop_edges`]): the kernel's queries share one
//! vertex-to-edge adjacency and one logical-face table per query, and
//! these specs hold them to the straightforward form below, kept here as
//! the reference. That form rebuilds the adjacency on every chain walk and
//! finds membership by linear search, so its cost grows with the square of
//! a chain's length; it must answer the same edges, in the same order, for
//! every edge and every face of every fixture.
//!
//! The work counters bound the queries without a clock: one adjacency
//! build per query, and work linear in the chain and the object.

use crate::common;

use common::curves::*;
use kernel::{
    BlendProfile, BlendSpec, CornerStyle, EdgeId, EdgeSmoothness, FaceId, LoopId, Object, Point3,
    VertexId,
};
use std::collections::{BTreeMap, BTreeSet};

// ------------------------------------------------------------- reference

/// The two faces of an edge: the face of its `half_edge`, then its twin's.
fn ref_edge_faces(obj: &Object, edge: EdgeId) -> Option<(FaceId, FaceId)> {
    let e = obj.edges().get(edge)?;
    let twin = e.twin_half_edge?;
    let face = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
    Some((face(e.half_edge), face(twin)))
}

fn ref_vertex_edges(obj: &Object) -> BTreeMap<VertexId, Vec<EdgeId>> {
    let mut m: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
    for (eid, e) in obj.edges().iter() {
        let he = obj.half_edges()[e.half_edge];
        m.entry(he.origin).or_default().push(eid);
        m.entry(obj.half_edges()[he.next].origin)
            .or_default()
            .push(eid);
    }
    m
}

fn ref_logical_edge(obj: &Object, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
    if obj.edge_convexity(edge).is_none() {
        return Vec::new();
    }
    let ve = ref_vertex_edges(obj);
    let mut cache: BTreeMap<FaceId, std::rc::Rc<BTreeSet<FaceId>>> = BTreeMap::new();
    let mut lf = |f: FaceId| -> std::rc::Rc<BTreeSet<FaceId>> {
        if let Some(set) = cache.get(&f) {
            return set.clone();
        }
        let set = std::rc::Rc::new(obj.logical_face(f).into_iter().collect::<BTreeSet<_>>());
        if obj
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
        let he = obj.half_edges()[obj.edges()[e].half_edge];
        (he.origin, obj.half_edges()[he.next].origin)
    };
    let mut step = |cur: EdgeId, v: VertexId| -> Option<EdgeId> {
        let (fa, fb) = ref_edge_faces(obj, cur)?;
        let (l1, l2) = (lf(fa), lf(fb));
        let others: Vec<EdgeId> = ve
            .get(&v)
            .map(|v| v.iter().copied().filter(|&o| o != cur).collect())
            .unwrap_or_default();
        let sharp: Vec<EdgeId> = others
            .iter()
            .copied()
            .filter(|&o| obj.edge_convexity(o).is_some())
            .collect();
        let same: Vec<EdgeId> = sharp
            .iter()
            .copied()
            .filter(|&o| {
                ref_edge_faces(obj, o).is_some_and(|(ga, gb)| {
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
        let sharing: Vec<EdgeId> = sharp
            .iter()
            .copied()
            .filter(|&o| {
                ref_edge_faces(obj, o).is_some_and(|(ga, gb)| {
                    l1.contains(&ga) || l1.contains(&gb) || l2.contains(&ga) || l2.contains(&gb)
                })
            })
            .collect();
        if sharing.len() != 1 || sharp.len() != 1 {
            return None;
        }
        let next = sharing[0];
        let (ga, gb) = ref_edge_faces(obj, next)?;
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
                obj.edge_smoothness(o),
                EdgeSmoothness::Tangent | EdgeSmoothness::Seam
            ) && ref_edge_faces(obj, o).is_some_and(|(x, y)| {
                (x == side_cur && y == side_next) || (x == side_next && y == side_cur)
            })
        });
        smooth_between.then_some(next)
    };
    let bound = obj.edges().len();
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

// The pick, as it was: the logical edge, or on an imported mesh the
// longer seam a user sees there (the scan-seam walk, with its constants).
const REF_SCAN_SEAM_TURN: f64 = 30.0 * std::f64::consts::PI / 180.0;
const REF_SCAN_SEAM_STEP: f64 = 60.0 * std::f64::consts::PI / 180.0;
const REF_SCAN_SEAM_SLIVER: f64 = 0.05;
const REF_SCAN_SEAM_FEATURE: f64 = 0.01;

fn ref_picked_edge(obj: &Object, edge: EdgeId, propagate: bool) -> Vec<EdgeId> {
    let out = ref_logical_edge(obj, edge, propagate);
    if !propagate || obj.planarity_tol() <= kernel::tol::PLANE_DIST || out.is_empty() {
        return out;
    }
    let ve = ref_vertex_edges(obj);
    let mut cache: BTreeMap<FaceId, std::rc::Rc<BTreeSet<FaceId>>> = BTreeMap::new();
    let mut lf = |f: FaceId| -> std::rc::Rc<BTreeSet<FaceId>> {
        if let Some(set) = cache.get(&f) {
            return set.clone();
        }
        let set = std::rc::Rc::new(obj.logical_face(f).into_iter().collect::<BTreeSet<_>>());
        if obj
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
    match ref_scan_seam(obj, edge, &ve, &mut lf) {
        Some(arc) if arc.len() > out.len() => arc,
        _ => out,
    }
}

fn ref_scan_seam(
    obj: &Object,
    edge: EdgeId,
    ve: &BTreeMap<VertexId, Vec<EdgeId>>,
    lf: &mut dyn FnMut(FaceId) -> std::rc::Rc<BTreeSet<FaceId>>,
) -> Option<Vec<EdgeId>> {
    let (fa, fb) = ref_edge_faces(obj, edge)?;
    let convexity = obj.edge_convexity(edge);
    let ends = |e: EdgeId| -> (VertexId, VertexId) {
        let he = obj.half_edges()[obj.edges()[e].half_edge];
        (he.origin, obj.half_edges()[he.next].origin)
    };
    let pos = |v: VertexId| obj.vertices()[v].position;
    let mut best: Option<Vec<EdgeId>> = None;
    for side in [lf(fa), lf(fb)] {
        // The side's boundary through `edge`, as a chain of vertices.
        let on_boundary = |o: EdgeId| {
            ref_edge_faces(obj, o).is_some_and(|(x, y)| side.contains(&x) != side.contains(&y))
        };
        let follow = |from: EdgeId, through: VertexId| -> (Vec<(EdgeId, VertexId)>, bool) {
            let mut out = Vec::new();
            let (mut cur, mut v) = (from, through);
            for _ in 0..obj.edges().len() {
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
        let like = |i: usize| obj.edge_convexity(chain[i].0) == convexity;
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
            if d_in.dot(d_out) < kernel::trig::cos(REF_SCAN_SEAM_TURN) {
                return false;
            }
            let (Ok(e_in), Ok(e_out)) = (
                (at - pos(start(j))).normalized(),
                (pos(finish(i)) - at).normalized(),
            ) else {
                return true;
            };
            let short = len(i).min(len(j));
            let comparable = short >= REF_SCAN_SEAM_SLIVER * len(i).max(len(j))
                && short >= REF_SCAN_SEAM_FEATURE * total;
            !(comparable && e_in.dot(e_out) < kernel::trig::cos(REF_SCAN_SEAM_STEP))
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
        let mut arc: std::collections::VecDeque<EdgeId> = std::collections::VecDeque::from([edge]);
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

fn ref_point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= kernel::tol::NORMALIZE_MIN_LENGTH * kernel::tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn ref_face_loops(obj: &Object, face: FaceId) -> Vec<LoopId> {
    let f = &obj.faces()[face];
    std::iter::once(f.outer_loop)
        .chain(f.inner_loops.iter().copied())
        .collect()
}

fn ref_face_loop_edges(obj: &Object, face: FaceId, near: Point3) -> Vec<EdgeId> {
    let Some(f) = obj.faces().get(face) else {
        return Vec::new();
    };
    let logical = obj.logical_face(face);
    if logical.len() > 1 {
        return ref_logical_face_loop_edges(obj, &logical, near);
    }
    let loops: Vec<_> = std::iter::once(f.outer_loop)
        .chain(f.inner_loops.iter().copied())
        .collect();
    let dist = |l| -> f64 {
        let ring: Vec<Point3> = obj.loop_positions(l).collect();
        (0..ring.len())
            .map(|i| ref_point_segment_distance(near, ring[i], ring[(i + 1) % ring.len()]))
            .fold(f64::INFINITY, f64::min)
    };
    let Some(&best) = loops.iter().min_by(|&&x, &&y| dist(x).total_cmp(&dist(y))) else {
        return Vec::new();
    };
    let mut out: Vec<EdgeId> = Vec::new();
    for h in obj.loop_half_edges(best) {
        let e = obj.half_edges()[h].edge;
        if out.contains(&e) || obj.edge_convexity(e).is_none() {
            continue;
        }
        for x in ref_logical_edge(obj, e, true) {
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

fn ref_logical_face_loop_edges(obj: &Object, logical: &[FaceId], near: Point3) -> Vec<EdgeId> {
    let inside: BTreeSet<FaceId> = logical.iter().copied().collect();
    let mut boundary: Vec<EdgeId> = Vec::new();
    for &g in logical {
        for l in ref_face_loops(obj, g) {
            for h in obj.loop_half_edges(l) {
                let e = obj.half_edges()[h].edge;
                let outside = ref_edge_faces(obj, e)
                    .is_none_or(|(a, b)| !(inside.contains(&a) && inside.contains(&b)));
                if outside && !boundary.contains(&e) {
                    boundary.push(e);
                }
            }
        }
    }
    let ends = |e: EdgeId| -> Option<(VertexId, VertexId)> {
        let ed = obj.edges().get(e)?;
        let he = obj.half_edges().get(ed.half_edge)?;
        Some((he.origin, obj.half_edges()[he.next].origin))
    };
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
        obj.edge_endpoints(e).map_or(f64::INFINITY, |(p, q)| {
            ref_point_segment_distance(near, p, q)
        })
    };
    let Some(nearest) =
        (0..boundary.len()).min_by(|&i, &j| dist(boundary[i]).total_cmp(&dist(boundary[j])))
    else {
        return Vec::new();
    };
    let want = root(&mut group, nearest);
    let mut out: Vec<EdgeId> = Vec::new();
    for (i, &e) in boundary.iter().enumerate() {
        if root(&mut group, i) != want || out.contains(&e) || obj.edge_convexity(e).is_none() {
            continue;
        }
        for x in ref_logical_edge(obj, e, true) {
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

// -------------------------------------------------------------- fixtures

fn round(edges: Vec<EdgeId>, r: f64, n: u32) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Round {
            radius: r,
            segments: n,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn chamfer(edges: Vec<EdgeId>, d: f64) -> BlendSpec {
    BlendSpec {
        profile: BlendProfile::Chamfer {
            setback_a: d,
            setback_b: d,
        },
        ..round(edges, 0.0, 1)
    }
}

/// A prism over a 60-tooth star (120 hard walls, no curves): a top face
/// with 120 sharp loop edges, each its own logical edge.
fn star_prism(teeth: usize) -> Object {
    let mut pts = Vec::new();
    for k in 0..teeth {
        let a = std::f64::consts::TAU * k as f64 / teeth as f64;
        let b = std::f64::consts::TAU * (k as f64 + 0.5) / teeth as f64;
        pts.push((0.1 * kernel::trig::cos(a), 0.1 * kernel::trig::sin(a)));
        pts.push((0.085 * kernel::trig::cos(b), 0.085 * kernel::trig::sin(b)));
    }
    prism(&pts, 0.05)
}

/// The spread the differential specs run over, by name.
fn corpus() -> Vec<(&'static str, Object)> {
    let mut out: Vec<(&'static str, Object)> = vec![
        ("unit box", unit_box()),
        ("box", box_xyz(2.0, 1.0, 0.5)),
        ("l block", l_block()),
        ("pyramid (valence-4 apex)", pyramid4()),
        ("wedge 60", wedge(60f64.to_radians())),
        ("wedge 120", wedge(120f64.to_radians())),
        ("cylinder", cylinder(0.1, 0.2, 24)),
        (
            "rounded rect prism",
            rounded_rect_prism(1.0, 0.6, 0.1, 0.2, 6),
        ),
        ("d post", d_post(0.1, 0.2, 24)),
        ("plate with hole", plate_with_hole(1.0, 0.1, 0.2, 24)),
        ("plate with boss", plate_with_boss(1.0, 0.1, 0.2, 0.1, 24)),
        ("revolved dome", revolved_dome(6, 12)),
        ("lofted hull", lofted_hull()),
        ("fan blade tangent", fan_blade(1.2)),
        ("fan blade kinked", fan_blade(1.3)),
        ("grooved knob", grooved_knob(0.0).0),
        ("star prism", star_prism(60)),
        ("imported sphere", imported_sphere(16, 24, 0.3, 0)),
        ("noisy sphere", noisy_sphere(16, 24, 0.3, 0.002)),
    ];
    let (doc, o, _) = app_pipe_tee(0.0, 0.0);
    out.push(("pipe tee", doc.object(o).unwrap().clone()));

    // Blends: tangent rails, blend logical faces, seams and hard mixes.
    let b = unit_box();
    let e = edge_between(&b, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    let (one, _) = b.blend_edges(&round(vec![e], 0.1, 6)).unwrap();
    out.push(("box, one fillet (open rails)", one.clone()));
    let corner = vec![
        edge_between(&b, p3(1.0, 1.0, 1.0), p3(0.0, 1.0, 1.0)),
        edge_between(&b, p3(1.0, 1.0, 1.0), p3(1.0, 0.0, 1.0)),
        edge_between(&b, p3(1.0, 1.0, 1.0), p3(1.0, 1.0, 0.0)),
    ];
    let (rolled, _) = b.blend_edges(&round(corner, 0.1, 4)).unwrap();
    out.push(("box, rolled corner", rolled));
    let e2 = edge_between(&one, p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0));
    let (mixed, _) = one.blend_edges(&chamfer(vec![e2], 0.05)).unwrap();
    out.push(("box, fillet and chamfer", mixed));
    let top: Vec<EdgeId> = {
        let b = unit_box();
        b.edges()
            .keys()
            .filter(|&e| {
                let (p, q) = b.edge_endpoints(e).unwrap();
                p.z > 0.5 && q.z > 0.5
            })
            .collect()
    };
    let (rim, _) = unit_box().blend_edges(&round(top, 0.1, 4)).unwrap();
    out.push(("box, filleted top rim (closed tangent loop)", rim));
    let c = cylinder(0.1, 0.2, 24);
    let rim_edges = sharp_edges_at_height(&c, 0.2);
    let (crim, _) = c.blend_edges(&round(rim_edges, 0.02, 4)).unwrap();
    out.push(("cylinder, filleted rim", crim));
    let (open, _) = facet([
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
    ]);
    out.push(("open facet", open));
    out
}

/// Points to ask a face's loop query from: a vertex of each loop, and the
/// midpoint of each loop's first edge.
fn probes(obj: &Object, face: FaceId) -> Vec<Point3> {
    let f = &obj.faces()[face];
    let mut out = Vec::new();
    for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
        let ring: Vec<Point3> = obj.loop_positions(l).collect();
        if let Some(&a) = ring.first() {
            out.push(a);
            if let Some(&b) = ring.get(1) {
                out.push(a + (b - a) * 0.5);
            }
        }
    }
    out
}

// ----------------------------------------------------------------- specs

#[test]
fn logical_edge_answers_as_the_reference_for_every_edge() {
    let mut checked = 0usize;
    for (name, obj) in corpus() {
        for e in obj.edges().keys() {
            for propagate in [false, true] {
                assert_eq!(
                    obj.logical_edge(e, propagate),
                    ref_logical_edge(&obj, e, propagate),
                    "{name}: logical_edge({e:?}, {propagate})"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1000, "the corpus covers many edges ({checked})");
}

/// `obj` as an imported mesh: its polygons, with no curves and the
/// import planarity, so a pick follows the seam a user sees.
fn imported(obj: &Object) -> Object {
    let (pts, faces) = obj.to_polygons();
    Object::from_polygons_import_for_test(&pts, &faces).unwrap()
}

/// Imported meshes, where the pick walks the seam a user sees: boxes and
/// blocks with corners and steps, a faceted cylinder and star, a softened
/// sphere joined to a disc (a scan's seam) and the same sliced.
fn imported_corpus() -> Vec<(&'static str, Object)> {
    let block = prism(
        &[
            (0.0, 0.0),
            (0.1, 0.0),
            (0.1, 0.01),
            (0.2, 0.01),
            (0.2, 0.06),
            (0.0, 0.06),
        ],
        0.05,
    );
    let mut out = vec![
        ("imported box", imported(&unit_box())),
        ("imported stepped block", imported(&block)),
        ("imported l block", imported(&l_block())),
        ("imported cylinder", imported(&cylinder(0.1, 0.2, 24))),
        ("imported star prism", imported(&star_prism(30))),
        (
            "imported plate with hole",
            imported(&plate_with_hole(1.0, 0.1, 0.2, 24)),
        ),
    ];
    for (name, op) in [
        ("softened scan joined to a disc", kernel::BooleanOp::Union),
        ("softened scan sliced", kernel::BooleanOp::Subtract),
    ] {
        let (sphere, _) = noisy_sphere(16, 24, 0.02, 0.002)
            .soften_by_angle(25f64.to_radians())
            .unwrap();
        let h = 0.03;
        let mut disc = cylinder(0.03, h, 24);
        disc.apply_transform(&kernel::Transform::translation(kernel::Vec3::new(
            0.0, 0.0, -h,
        )))
        .unwrap();
        let o = Object::boolean(op, &sphere, &disc, &kernel::Transform::IDENTITY).unwrap();
        out.push((name, o));
    }
    out
}

#[test]
fn picked_edge_answers_as_the_reference_for_every_edge() {
    let mut scanned = 0usize;
    for (name, obj) in corpus().into_iter().chain(imported_corpus()) {
        for e in obj.edges().keys() {
            for propagate in [false, true] {
                let reference = ref_picked_edge(&obj, e, propagate);
                if reference.len() > obj.logical_edge(e, propagate).len() {
                    scanned += 1;
                }
                assert_eq!(
                    obj.picked_edge(e, propagate),
                    reference,
                    "{name}: picked_edge({e:?}, {propagate})"
                );
            }
        }
    }
    assert!(
        scanned >= 10,
        "the imported corpus reaches the scan-seam walk ({scanned} picks)"
    );
}

/// A scan-sized softened sphere (radius 20 mm, about 1 mm facets, noisy)
/// joined to a disc whose top is at `z`, or with everything below `z` cut
/// away: the seam at `z`, where a pick follows the whole loop a user sees
/// through the faces the scan's noise splits it into.
fn scan(amp: f64, z: f64, union: bool) -> (Object, Vec<EdgeId>) {
    let (sphere, _) = noisy_sphere(60, 120, 0.02, amp)
        .soften_by_angle(20f64.to_radians())
        .unwrap();
    let h = if union { 0.01 } else { 0.03 };
    let mut disc = cylinder(0.03, h, 48);
    disc.apply_transform(&kernel::Transform::translation(kernel::Vec3::new(
        0.0,
        0.0,
        z - h,
    )))
    .unwrap();
    let op = if union {
        kernel::BooleanOp::Union
    } else {
        kernel::BooleanOp::Subtract
    };
    let o = Object::boolean(op, &sphere, &disc, &kernel::Transform::IDENTITY).unwrap();
    let seam = o
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            (a.z - z).abs() < 1e-7 && (b.z - z).abs() < 1e-7 && o.edge_convexity(e).is_some()
        })
        .collect();
    (o, seam)
}

#[test]
fn picked_edge_answers_as_the_reference_on_a_scans_seam() {
    let mut walked = 0usize;
    for (amp, z, union) in [(0.006, -0.012, true), (0.003, 0.0, false)] {
        let (o, seam) = scan(amp, z, union);
        assert!(!seam.is_empty());
        // Every seam edge, and every edge that shares a vertex with one.
        let ends: std::collections::BTreeSet<_> = seam
            .iter()
            .flat_map(|&e| {
                let he = o.half_edges()[o.edges()[e].half_edge];
                [he.origin, o.half_edges()[he.next].origin]
            })
            .collect();
        let near: Vec<EdgeId> = o
            .edges()
            .iter()
            .filter(|(_, ed)| {
                let he = o.half_edges()[ed.half_edge];
                ends.contains(&he.origin) || ends.contains(&o.half_edges()[he.next].origin)
            })
            .map(|(id, _)| id)
            .collect();
        for &e in &near {
            let reference = ref_picked_edge(&o, e, true);
            if reference.len() > o.logical_edge(e, true).len() {
                walked += 1;
            }
            assert_eq!(
                o.picked_edge(e, true),
                reference,
                "noise {amp}, seam at {z}: picked_edge({e:?})"
            );
        }
    }
    assert!(
        walked > 100,
        "the seam picks take the scan-seam walk ({walked})"
    );
}

#[test]
fn face_loop_edges_answers_as_the_reference_for_every_face() {
    let mut checked = 0usize;
    for (name, obj) in corpus() {
        for f in obj.faces().keys() {
            for near in probes(&obj, f) {
                assert_eq!(
                    obj.face_loop_edges(f, near),
                    ref_face_loop_edges(&obj, f, near),
                    "{name}: face_loop_edges({f:?}, {near:?})"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1000, "the corpus covers many faces ({checked})");
}

#[test]
fn unknown_ids_answer_as_the_reference() {
    let obj = unit_box();
    let other = cylinder(0.1, 0.2, 24);
    let stale_edges: Vec<EdgeId> = other.edges().keys().chain([EdgeId::default()]).collect();
    for e in stale_edges {
        for propagate in [false, true] {
            let new = std::panic::catch_unwind(|| obj.logical_edge(e, propagate));
            let old = std::panic::catch_unwind(|| ref_logical_edge(&obj, e, propagate));
            assert_eq!(new.ok(), old.ok(), "logical_edge({e:?}, {propagate})");
        }
    }
    let stale_faces: Vec<FaceId> = other.faces().keys().chain([FaceId::default()]).collect();
    for f in stale_faces {
        let near = p3(0.5, 0.5, 1.0);
        let new = std::panic::catch_unwind(|| obj.face_loop_edges(f, near));
        let old = std::panic::catch_unwind(|| ref_face_loop_edges(&obj, f, near));
        assert_eq!(new.ok(), old.ok(), "face_loop_edges({f:?})");
    }
}

// ------------------------------------------------------------ work bound

/// The adjacency builds and the work one call of `f` costs.
fn cost<T>(f: impl FnOnce() -> T) -> (usize, usize, T) {
    let (b0, w0) = (
        kernel::adjacency_builds_for_test(),
        kernel::chain_query_work_for_test(),
    );
    let out = f();
    (
        kernel::adjacency_builds_for_test() - b0,
        kernel::chain_query_work_for_test() - w0,
        out,
    )
}

/// The work of each hover query on a claimed cylinder of `n` facets:
/// its rim's logical edge (both ways), the rim's pick, the cap's loop and
/// a wall facet's loop. Each query builds one adjacency and answers the
/// whole rim.
fn rim_work(n: usize) -> (usize, [usize; 5]) {
    let c = claimed_cylinder(n);
    let size = c.edges().len() + c.faces().len();
    let e = sharp_edges_at_height(&c, 0.1)[0];
    let cap = face_with_normal(&c, kernel::Vec3::new(0.0, 0.0, 1.0));
    let wall = c
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(id, _)| id)
        .unwrap();
    let near = c.loop_positions(c.faces()[wall].outer_loop).next().unwrap();
    let mut work = [0usize; 5];
    let queries: [&dyn Fn() -> Vec<EdgeId>; 5] = [
        &|| c.logical_edge(e, false),
        &|| c.logical_edge(e, true),
        &|| c.picked_edge(e, true),
        &|| c.face_loop_edges(cap, p3(0.1, 0.0, 0.1)),
        &|| c.face_loop_edges(wall, near),
    ];
    for (k, q) in queries.iter().enumerate() {
        let (builds, w, edges) = cost(q);
        assert_eq!(edges.len(), n, "n = {n}, query {k}: the whole rim");
        assert_eq!(builds, 1, "n = {n}, query {k}: one adjacency per query");
        work[k] = w;
    }
    (size, work)
}

/// `size · (⌊log2(size)⌋ + 1)`: what a query that keeps sorted sets of the
/// object's elements may cost.
fn n_log_n(size: usize) -> f64 {
    size as f64 * f64::from(size.ilog2() + 1)
}

/// Every hover query on a long rim builds one adjacency, and its work (the
/// chain steps, the logical faces, and every comparison its sets and maps
/// make) stays within a small multiple of `size · log2(size)` and grows no
/// faster than that from 1,024 to 4,096 facets. A membership test that
/// went back to a linear search costs the square of the rim and fails
/// both. The constants are about 1.2 times the measured ones.
// The work counter counts only with debug assertions on.
#[cfg(debug_assertions)]
#[test]
fn rim_queries_build_one_adjacency_and_work_in_n_log_n() {
    const NAMES: [&str; 5] = [
        "logical edge",
        "propagating logical edge",
        "pick",
        "cap loop",
        "wall facet loop",
    ];
    const BOUND: [f64; 5] = [2.0, 2.0, 2.0, 3.0, 10.0];
    let mut by_size = Vec::new();
    for n in [256usize, 1024, 4096] {
        let (size, work) = rim_work(n);
        for k in 0..5 {
            assert!(
                work[k] as f64 <= BOUND[k] * n_log_n(size),
                "n = {n}, {}: work {} over {} x {size} log {size}",
                NAMES[k],
                work[k],
                BOUND[k]
            );
        }
        by_size.push(work);
    }
    for k in 0..5 {
        let growth = by_size[2][k] as f64 / by_size[1][k] as f64;
        assert!(
            growth <= 5.5,
            "{}: work grew {growth:.2}x from 1,024 to 4,096 facets (4.8x is n log n)",
            NAMES[k]
        );
    }
}

// The work counter counts only with debug assertions on.
#[cfg(debug_assertions)]
#[test]
fn a_face_query_builds_one_adjacency_for_all_its_loop_edges() {
    // 120 sharp loop edges, each its own logical edge.
    let star = star_prism(60);
    let top = face_with_normal(&star, kernel::Vec3::new(0.0, 0.0, 1.0));
    let size = star.edges().len() + star.faces().len();
    let (builds, work, edges) = cost(|| star.face_loop_edges(top, p3(0.1, 0.0, 0.05)));
    assert_eq!(edges.len(), 120);
    assert_eq!(builds, 1);
    assert!(
        work as f64 <= 4.0 * n_log_n(size),
        "work {work} ({size} elements)"
    );
}
