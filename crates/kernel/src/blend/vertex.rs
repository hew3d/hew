//! Vertex constructions (WP05 §7, WP06 §5–6): where a chain ends, one
//! construction closes the strips against the faces around the vertex.
//! Dispatch, first match wins: the valence-3 run-out (§7.1), the rolled
//! corner (a sphere fan where three fillets meet), the prismatic miter
//! (§7.2), then the general convex clip (§7.3; with one extra plane for a
//! flat chamfer corner).

use crate::trig;
use std::collections::{BTreeMap, BTreeSet};

use super::classify::EdgeKind;
use super::clip::{CellTag, ConvexCell};
use super::plan::{CornerFace, Ctx, Out, PlanFail, Pts, Strip, StripEnd};
use super::section::{Section, Trace, solve_section};
use super::{BlendError, BlendProfile, Convexity, CornerStyle};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::patch::EdgeMeta;
use crate::sketch::CurveGeom;
use crate::tol;

/// Builds the construction at `v` for the strip ends `ends` (strip index,
/// end 0 = start / 1 = end).
pub(crate) fn construct(
    ctx: &Ctx,
    v: VertexId,
    ends: &[(usize, usize)],
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
) -> Result<(), PlanFail> {
    let at = ctx.obj.vertices()[v].position;
    let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at });
    if let [(si, which)] = *ends
        && run_out(ctx, v, si, which, strips, pts, out)?
    {
        return Ok(());
    }
    // Coves and router profiles have non-convex sections: they run out and
    // miter, and nothing else (WP07 §3.3).
    let convex_section = matches!(
        ctx.profile,
        BlendProfile::Round { .. } | BlendProfile::Chamfer { .. }
    );
    if !convex_section {
        if let [e1, e2] = *ends
            && miter(ctx, v, e1, e2, strips, pts, out)?
        {
            return Ok(());
        }
        return Err(unsupported);
    }
    if ends.len() == 3 && rolled_corner(ctx, v, ends, strips, pts, out)? {
        return Ok(());
    }
    if let [e1, e2] = *ends
        && miter(ctx, v, e1, e2, strips, pts, out)?
    {
        return Ok(());
    }
    let flat = ends.len() == 3
        && ctx.corners == CornerStyle::Flat
        && matches!(ctx.profile, BlendProfile::Chamfer { .. });
    if clip_vertex(ctx, v, ends, strips, pts, out, flat)? {
        return Ok(());
    }
    Err(unsupported)
}

/// The geometric fold of edge `e`, sharp or not: `Some(None)` when its two
/// faces are coplanar, `None` when it has no two faces.
fn fold(obj: &crate::topo::Object, e: EdgeId) -> Option<Option<Convexity>> {
    let (a, b) = obj.edge_faces(e)?;
    let n_b = obj.faces()[b].plane.normal();
    if obj.faces()[a].plane.normal().dot(n_b) >= 1.0 - tol::NORMAL_DIRECTION {
        return Some(None);
    }
    let f = obj.edge_frame(e, a)?;
    Some(Some(if f.t_a.dot(n_b) < 0.0 {
        Convexity::Convex
    } else {
        Convexity::Concave
    }))
}

/// The section points of strip `s` at its end vertex `which`.
fn station(ctx: &Ctx, s: &Strip, which: usize) -> Vec<Point3> {
    let _ = ctx;
    let q = s.qs[s.end_index(which)];
    s.end_sec(which).offsets.iter().map(|&o| q + o).collect()
}

/// The parameter where the line `p + s·t` meets `plane`, or `None` when the
/// line runs along it.
fn line_plane(p: Point3, t: Vec3, plane: &Plane) -> Option<Point3> {
    let dn = t.dot(plane.normal());
    if dn.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    Some(p + t * (-plane.signed_distance(p) / dn))
}

/// The other endpoint of edge `e` from `v`.
fn other_end(ctx: &Ctx, e: EdgeId, v: VertexId) -> Option<VertexId> {
    let (a, b) = ctx.obj.edge_vertex_ids(e)?;
    Some(if a == v { b } else { a })
}

/// §7.1: one picked edge between A and B, two unpicked edges, and a third
/// face C across the vertex. Each ruling runs out into C's plane.
fn run_out(
    ctx: &Ctx,
    v: VertexId,
    si: usize,
    which: usize,
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
) -> Result<bool, PlanFail> {
    let obj = ctx.obj;
    let edges = ctx.edges_at(v);
    let faces = ctx.faces_at(v);
    if edges.len() != 3 || faces.len() != 3 {
        return Ok(false);
    }
    let s = &strips[si];
    let info = s.end_info(which).clone();
    if edges.iter().filter(|e| ctx.picked.contains(e)).count() != 1 {
        return Ok(false);
    }
    let others: Vec<FaceId> = faces
        .iter()
        .copied()
        .filter(|&f| f != info.a && f != info.b)
        .collect();
    let [c] = others.as_slice() else {
        return Ok(false);
    };
    let c = *c;
    let (Some(e1), Some(e2)) = (
        ctx.edge_between_faces(v, info.a, c),
        ctx.edge_between_faces(v, info.b, c),
    ) else {
        return Ok(false);
    };
    let at = obj.vertices()[v].position;
    let plane_c = obj.faces()[c].plane;
    let t = s.toward_end(obj, which);
    let p = station(ctx, s, which);
    let mut e_pts: Vec<usize> = Vec::with_capacity(p.len());
    for &pk in &p {
        // A taper's rulings are its cone's generators.
        let dir = match s.apex {
            Some(apex) => (pk - apex)
                .normalized()
                .map(|d| if d.dot(t) < 0.0 { -d } else { d })
                .unwrap_or(t),
            None => t,
        };
        let x = line_plane(pk, dir, &plane_c)
            .ok_or(PlanFail::Hard(BlendError::UnsupportedVertex { at }))?;
        e_pts.push(pts.add(x));
    }
    let n = s.n;
    let b1 = other_end(ctx, e2, v);
    let arrives_on_e2 = ctx.prev_in_face(c, v) == b1;
    let mut run_c = e_pts.clone();
    if arrives_on_e2 {
        run_c.reverse();
    }
    if s.convexity == Convexity::Concave {
        out.fill_points.extend(e_pts[1..n].iter().copied());
    }
    out.face_runs.insert((info.a, v), vec![e_pts[0]]);
    out.face_runs.insert((c, v), run_c);
    out.shortened.entry(e1).or_default().push((v, e_pts[0]));
    match s.trimmed_b {
        None => {
            out.face_runs.insert((info.b, v), vec![e_pts[n]]);
            out.shortened.entry(e2).or_default().push((v, e_pts[n]));
        }
        Some(fm) => {
            // Seam crossing (WP06 §3.1): the consumed seam vertices b_1..b_m
            // follow v in C's loop on the B side; they vanish, and the
            // trimmed facet's corner at b_m becomes E_N.
            let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at });
            let on_fm = |u: VertexId| {
                obj.face_loops(fm).into_iter().any(|l| {
                    obj.loop_half_edges(l)
                        .any(|h| obj.half_edges()[h].origin == u)
                })
            };
            let (mut prev, mut cur) = (v, b1.ok_or(unsupported)?);
            let mut consumed: Vec<VertexId> = Vec::new();
            for _ in 0..obj.vertices().len() {
                consumed.push(cur);
                if on_fm(cur) {
                    break;
                }
                let next = if arrives_on_e2 {
                    ctx.prev_in_face(c, cur)
                } else {
                    ctx.next_in_face(c, cur)
                }
                .ok_or(unsupported)?;
                if next == prev {
                    return Err(unsupported);
                }
                prev = cur;
                cur = next;
            }
            let bm = *consumed.last().ok_or(unsupported)?;
            if !on_fm(bm) {
                return Err(unsupported);
            }
            for &b in &consumed {
                out.face_runs.insert((c, b), Vec::new());
            }
            out.face_runs.insert((fm, bm), vec![e_pts[n]]);
            let rim = ctx.edge_between_faces(bm, fm, c).ok_or(unsupported)?;
            out.shortened.entry(rim).or_default().push((bm, e_pts[n]));
        }
    }
    // A run-out into a face square to the chain is an arc of the section.
    if t.dot(plane_c.normal()).abs() >= 1.0 - tol::NORMAL_DIRECTION
        && let (BlendProfile::Round { .. }, Some(center), Some(radius)) = (
            ctx.profile,
            s.end_sec(which).arc_center,
            s.end_sec(which).radius,
        )
    {
        let center = s.qs[s.end_index(which)] + center;
        for k in 0..n {
            out.meta(
                e_pts[k],
                e_pts[k + 1],
                EdgeMeta {
                    curve: Some(CurveGeom { center, radius }),
                    ..EdgeMeta::default()
                },
            );
        }
    }
    strips[si].ends[which] = StripEnd {
        ruling_cut: e_pts.into_iter().map(Some).collect(),
        facet_extra: vec![Vec::new(); n],
    };
    Ok(true)
}

/// §7.2: two picked edges sharing a face T meet a third, unpicked edge
/// square to T; the strips meet in the bisector plane of their other faces.
fn miter(
    ctx: &Ctx,
    v: VertexId,
    (s1, w1): (usize, usize),
    (s2, w2): (usize, usize),
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
) -> Result<bool, PlanFail> {
    let obj = ctx.obj;
    let edges = ctx.edges_at(v);
    let faces = ctx.faces_at(v);
    if s1 == s2 {
        return Ok(false);
    }
    let (i1, i2) = (
        strips[s1].end_info(w1).clone(),
        strips[s2].end_info(w2).clone(),
    );
    // The face T the two strips share at the corner, or two coplanar faces
    // (`t1` beside the first strip, `t2` beside the second) that one
    // unpicked line from the vertex splits.
    let shared = if i1.a == i2.a || i1.a == i2.b {
        Some(i1.a)
    } else if i1.b == i2.a || i1.b == i2.b {
        Some(i1.b)
    } else {
        None
    };
    let (t1, t2, split) = match shared {
        Some(t) => {
            if edges.len() != 3 || faces.len() != 3 {
                return Ok(false);
            }
            (t, t, None)
        }
        None => {
            if edges.len() != 4 || faces.len() != 4 {
                return Ok(false);
            }
            let pair = [(i1.a, i2.a), (i1.a, i2.b), (i1.b, i2.a), (i1.b, i2.b)]
                .into_iter()
                .find_map(|(x, y)| {
                    let g = ctx.edge_between_faces(v, x, y)?;
                    (!ctx.picked.contains(&g) && super::chain::coplanar(obj, x, y))
                        .then_some((x, y, g))
                });
            let Some((x, y, g)) = pair else {
                return Ok(false);
            };
            (x, y, Some(g))
        }
    };
    let other = |i: &super::classify::EdgeInfo, t: FaceId| if i.a == t { i.b } else { i.a };
    let (f1, f2) = (other(&i1, t1), other(&i2, t2));
    if f1 == f2 {
        return Ok(false);
    }
    let Some(e3) = ctx.edge_between_faces(v, f1, f2) else {
        return Ok(false);
    };
    if ctx.picked.contains(&e3) {
        return Ok(false);
    }
    let Some((pa, pb)) = obj.edge_endpoints(e3) else {
        return Ok(false);
    };
    let Ok(d3) = (pb - pa).normalized() else {
        return Ok(false);
    };
    let n_t = obj.faces()[t1].plane.normal();
    if d3.dot(n_t).abs() < 1.0 - tol::NORMAL_DIRECTION {
        return Ok(false);
    }
    let n = strips[s1].n;
    if strips[s2].n != n {
        return Ok(false);
    }
    // Ruling index from the T side: j = 0 lies on T.
    let map = |i: &super::classify::EdgeInfo, t: FaceId, j: usize| if i.a == t { j } else { n - j };
    if let BlendProfile::Chamfer {
        setback_a,
        setback_b,
    } = *ctx.profile
    {
        let on_t = |i: &super::classify::EdgeInfo, t: FaceId| {
            if i.a == t { setback_a } else { setback_b }
        };
        if (on_t(&i1, t1) - on_t(&i2, t2)).abs() > tol::POINT_MERGE {
            return Ok(false);
        }
    }
    let q = obj.vertices()[v].position;
    let Ok(pi_n) = (obj.faces()[f1].plane.normal() - obj.faces()[f2].plane.normal()).normalized()
    else {
        return Ok(false);
    };
    let Ok(pi) = Plane::from_point_normal(q, pi_n) else {
        return Ok(false);
    };
    let (p1, p2) = (station(ctx, &strips[s1], w1), station(ctx, &strips[s2], w2));
    let (d1, d2) = (
        strips[s1].toward_end(obj, w1),
        strips[s2].toward_end(obj, w2),
    );
    let mut e_pts: Vec<Point3> = Vec::with_capacity(n + 1);
    for j in 0..=n {
        let (Some(a), Some(b)) = (
            line_plane(p1[map(&i1, t1, j)], d1, &pi),
            line_plane(p2[map(&i2, t2, j)], d2, &pi),
        ) else {
            return Ok(false);
        };
        if !a.approx_eq(b, tol::POINT_MERGE) {
            return Ok(false);
        }
        e_pts.push(a);
    }
    let idx: Vec<usize> = e_pts.into_iter().map(|p| pts.add(p)).collect();
    match split {
        None => {
            out.face_runs.insert((t1, v), vec![idx[0]]);
        }
        Some(g) => split_miter(
            ctx,
            v,
            g,
            [(s1, w1, &i1, t1), (s2, w2, &i2, t2)],
            idx[0],
            strips,
            pts,
            out,
        )?,
    }
    out.face_runs.insert((f1, v), vec![idx[n]]);
    out.face_runs.insert((f2, v), vec![idx[n]]);
    out.shortened.entry(e3).or_default().push((v, idx[n]));
    for (si, w, info, t) in [(s1, w1, &i1, t1), (s2, w2, &i2, t2)] {
        let mut cut = vec![None; n + 1];
        for (j, &p) in idx.iter().enumerate() {
            cut[map(info, t, j)] = Some(p);
        }
        strips[si].ends[w] = StripEnd {
            ruling_cut: cut,
            facet_extra: vec![Vec::new(); n],
        };
    }
    Ok(true)
}

/// A miter whose T side is two coplanar faces split by the line `g` from
/// the vertex: `g` is shortened to where it leaves the cut, on the T rail
/// of the strip whose side it crosses (it runs on into the face beyond),
/// and that point joins the rail. The face on that strip's side ends at
/// the crossing; the other runs crossing → miter point `m` (or the other
/// way, by its loop). A line along the miter itself ends at `m`.
#[allow(clippy::too_many_arguments)]
fn split_miter(
    ctx: &Ctx,
    v: VertexId,
    g: EdgeId,
    sides: [(usize, usize, &super::classify::EdgeInfo, FaceId); 2],
    m: usize,
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
) -> Result<(), PlanFail> {
    let obj = ctx.obj;
    let q = obj.vertices()[v].position;
    let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at: q });
    let g_far = other_end(ctx, g, v).ok_or(unsupported)?;
    let dir_g = (obj.vertices()[g_far].position - q)
        .normalized()
        .map_err(|_| unsupported)?;
    let g_len = (obj.vertices()[g_far].position - q).length();
    let base = pts.pos(m);
    let w = base - q;
    // Where g crosses each strip's T rail: the line from the miter point to
    // the far station's T point (askew to the edge where the radius ramps;
    // at a re-entrant corner it starts behind the vertex). Of the two, the
    // first g meets.
    let mut crossing: Option<(f64, usize, f64, Vec3, f64)> = None;
    for (k, &(si, wi, info, ti)) in sides.iter().enumerate() {
        let strip = &strips[si];
        let last = strip.chain.verts.len() - 1;
        let far_i = if wi == 0 { 1 } else { last - 1 };
        let r = if info.a == ti { 0 } else { strip.n };
        let far = strip.qs[far_i] + strip.secs[far_i].offsets[r];
        let rail = (far - base).length();
        let Ok(t) = (far - base).normalized() else {
            continue;
        };
        let tg = t.dot(dir_g);
        let denom = 1.0 - tg * tg;
        if denom <= tol::NORMAL_DIRECTION {
            continue;
        }
        let s = (w.dot(dir_g) * tg - w.dot(t)) / denom;
        let mu = (base + t * s - q).dot(dir_g);
        if s >= -tol::POINT_MERGE && mu > -tol::POINT_MERGE && crossing.is_none_or(|c| mu < c.0) {
            crossing = Some((mu, k, s.max(0.0), t, rail));
        }
    }
    let Some((mu, k, s, t, rail)) = crossing else {
        return Err(unsupported);
    };
    let (_, _, _, t_other) = sides[1 - k];
    if s <= tol::POINT_MERGE {
        // Along the miter: both faces end at its point.
        out.face_runs.insert((sides[0].3, v), vec![m]);
        out.face_runs.insert((sides[1].3, v), vec![m]);
        out.shortened.entry(g).or_default().push((v, m));
        return Ok(());
    }
    // The line ends inside the cut, or leaves it past the far station: a
    // smaller size keeps it.
    if mu >= g_len - tol::POINT_MERGE || s >= rail - tol::POINT_MERGE {
        return Err(PlanFail::Size(super::LimitFeature::Imprint));
    }
    let (si, wi, info, ti) = sides[k];
    let x = base + t * s;
    // The crossing joins the strip's ruling on its last segment at this
    // end, by its distance from the segment's start (as every insert).
    let strip = &mut strips[si];
    let seg = if wi == 0 {
        0
    } else {
        strip.chain.verts.len() - 2
    };
    let pa = obj.vertices()[strip.chain.verts[seg]].position;
    let param = (x - pa).dot(strip.chain.seg_dir(obj, seg));
    if !strip.mids[seg].is_empty() {
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    }
    let xi = pts.add(x);
    let side = if info.a == ti { 0 } else { 1 };
    strip.inserts[seg][side].push((param, xi));
    out.crossings.push(super::plan::Crossing {
        point: xi,
        strip: si,
        seg,
        side,
        from: q,
        dir: dir_g,
        reach: g_len,
        at_start: wi == 0,
    });
    out.face_runs.insert((ti, v), vec![xi]);
    let run = if ctx.prev_in_face(t_other, v) == Some(g_far) {
        vec![xi, m]
    } else {
        vec![m, xi]
    };
    out.face_runs.insert((t_other, v), run);
    out.shortened.entry(g).or_default().push((v, xi));
    Ok(())
}

/// One clipped cell and the planes of its initial cube.
fn clipped(q: Point3, half: f64, planes: &[(Plane, CellTag)]) -> Option<(ConvexCell, Vec<Plane>)> {
    let mut cell = ConvexCell::cube(q, half);
    let boxes: Vec<Plane> = cell.faces.iter().map(|f| f.plane).collect();
    for (p, tag) in planes {
        cell.clip(p, *tag).ok()?;
    }
    Some((cell, boxes))
}

/// The positions of a cell's real vertices (on no cube plane), sorted.
fn real_positions(cell: &ConvexCell, boxes: &[Plane]) -> Vec<Point3> {
    let used: BTreeSet<usize> = cell.faces.iter().flat_map(|f| f.verts.clone()).collect();
    let mut v: Vec<Point3> = used
        .into_iter()
        .filter(|&i| !cell.on_box(i, boxes))
        .map(|i| cell.verts[i])
        .collect();
    v.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    v
}

fn same_points(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|p| b.iter().any(|q| p.approx_eq(*q, tol::POINT_MERGE)))
}

/// §7.3: the general construction at a vertex whose incident edges share a
/// convexity. The material (or, at a concave vertex, the air) near the
/// vertex is a convex cone; clipping it by the strips' facet planes gives
/// the blended neighborhood, and its faces give every run.
fn clip_vertex(
    ctx: &Ctx,
    v: VertexId,
    ends: &[(usize, usize)],
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
    flat: bool,
) -> Result<bool, PlanFail> {
    let obj = ctx.obj;
    let q = obj.vertices()[v].position;
    let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at: q });
    let all_faces = ctx.faces_at(v);
    let edges = ctx.edges_at(v);
    let mut convexity: Option<Convexity> = None;
    let mut folds: Vec<Convexity> = Vec::new();
    let mut split: Option<EdgeId> = None;
    for &e in &edges {
        match obj.edge_convexity(e) {
            Some(c) => {
                if convexity.is_some_and(|x| x != c) {
                    return Ok(false);
                }
                convexity = Some(c);
            }
            // A soft seam (a lofted wall's diagonal) bounds the cell like any
            // face pair, so long as it folds the same way as the corner's
            // sharp edges. A flat edge (a drawn line between coplanar faces)
            // would clip the cell twice by one plane and leave a face out of
            // it: one, unpicked, clips as the one plane its two faces lie in
            // and splits that face's run where it leaves the cut (below);
            // any other declines.
            None => match fold(obj, e) {
                Some(Some(c)) => folds.push(c),
                Some(None) if split.is_none() && !ctx.picked.contains(&e) => split = Some(e),
                Some(None) | None => return Ok(false),
            },
        }
    }
    if folds.iter().any(|&c| Some(c) != convexity) {
        return Ok(false);
    }
    // The split line's two faces: `ta` stands for both in the cell.
    let (ta, tb) = match split {
        Some(g) => {
            let (a, b) = obj.edge_faces(g).ok_or(unsupported)?;
            (Some(a), Some(b))
        }
        None => (None, None),
    };
    let merged = |f: FaceId| if Some(f) == tb { ta.expect("split") } else { f };
    let faces: Vec<FaceId> = all_faces
        .iter()
        .copied()
        .filter(|&f| Some(f) != tb)
        .collect();
    if all_faces.iter().any(|&f| {
        let face = &obj.faces()[f];
        face.surface.is_some() || face.blend.is_some()
    }) {
        return Ok(false);
    }
    let concave = convexity == Some(Convexity::Concave);
    // A concave corner grows the face outside its outline, where the line
    // does not reach.
    if split.is_some() && concave {
        return Ok(false);
    }

    // The planes, faces by id then facets by (edge, k).
    let mut planes: Vec<(Plane, CellTag)> = faces
        .iter()
        .map(|&f| {
            let p = obj.faces()[f].plane;
            (if concave { p.flipped() } else { p }, CellTag::Face(f))
        })
        .collect();
    // Strip end by its edge at this vertex.
    let mut by_edge: BTreeMap<EdgeId, (usize, usize)> = BTreeMap::new();
    let mut size = 0.0_f64;
    let mut theta_min = std::f64::consts::PI;
    for &(si, w) in ends {
        let s = &strips[si];
        let info = s.end_info(w);
        by_edge.insert(info.edge, (si, w));
        theta_min = theta_min.min(info.theta);
        for o in &s.end_sec(w).offsets {
            size = size.max(o.length());
        }
    }
    let mut facets: Vec<(Plane, CellTag)> = Vec::new();
    for (&edge, &(si, w)) in &by_edge {
        let s = &strips[si];
        let p = station(ctx, s, w);
        let t = s.toward_end(obj, w);
        for k in 0..s.n {
            let n = (p[k + 1] - p[k])
                .cross(t)
                .normalized()
                .map_err(|_| unsupported)?;
            let mut plane = Plane::from_point_normal(p[k], n).map_err(|_| unsupported)?;
            if plane.signed_distance(q) < 0.0 {
                plane = plane.flipped();
            }
            facets.push((plane, CellTag::Facet { edge, k }));
        }
    }
    planes.extend(facets);
    let half0 = (4.0 * size / trig::sin(theta_min / 2.0)).max(4.0 * size);
    let stable = |planes: &[(Plane, CellTag)]| -> Result<(ConvexCell, Vec<Plane>), PlanFail> {
        let mut half = half0;
        for _ in 0..5 {
            let (Some(a), Some(b)) = (clipped(q, half, planes), clipped(q, 2.0 * half, planes))
            else {
                return Err(unsupported);
            };
            if same_points(&real_positions(&a.0, &a.1), &real_positions(&b.0, &b.1)) {
                return Ok(a);
            }
            half *= 2.0;
        }
        Err(unsupported)
    };
    let (mut cell, mut boxes) = stable(&planes)?;
    let split_at = match (split, ta) {
        (Some(g), Some(t)) if !flat => Some((g, split_cell(ctx, v, g, t, &mut cell, &boxes)?)),
        (Some(_), _) => return Ok(false),
        _ => None,
    };
    // A crossing on a strip's side ruling beyond the cut: (strip, end,
    // side, point), inserted once the cell is read.
    let mut ruling_crossing: Option<(usize, usize, usize, usize)> = None;
    if flat {
        // The flat corner: one plane through the three points where each
        // face's two rails cross, read from the mitered clip.
        if faces.len() != 3 {
            return Err(unsupported);
        }
        let mut xs: Vec<Point3> = Vec::with_capacity(3);
        for &f in &faces {
            let on_plane =
                |i: usize, pl: &Plane| pl.signed_distance(cell.verts[i]).abs() <= tol::PLANE_DIST;
            let face_plane = planes
                .iter()
                .find(|(_, t)| *t == CellTag::Face(f))
                .map(|(p, _)| *p)
                .ok_or(unsupported)?;
            let used: BTreeSet<usize> = cell.faces.iter().flat_map(|c| c.verts.clone()).collect();
            let x = used.into_iter().find(|&i| {
                !cell.on_box(i, &boxes)
                    && on_plane(i, &face_plane)
                    && planes
                        .iter()
                        .filter(|(p, t)| matches!(t, CellTag::Facet { .. }) && on_plane(i, p))
                        .count()
                        >= 2
            });
            xs.push(cell.verts[x.ok_or(unsupported)?]);
        }
        let mut extra = Plane::from_polygon(&xs).map_err(|_| unsupported)?;
        if extra.signed_distance(q) < 0.0 {
            extra = extra.flipped();
        }
        planes.push((extra, CellTag::Extra(0)));
        (cell, boxes) = stable(&planes)?;
    }

    // Cell edges → the faces holding them.
    let mut edge_faces: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (fi, f) in cell.faces.iter().enumerate() {
        let m = f.verts.len();
        for i in 0..m {
            let (a, b) = (f.verts[i], f.verts[(i + 1) % m]);
            edge_faces.entry((a.min(b), a.max(b))).or_default().push(fi);
        }
    }
    let neighbor = |fi: usize, a: usize, b: usize| -> Option<CellTag> {
        edge_faces
            .get(&(a.min(b), a.max(b)))?
            .iter()
            .find(|&&g| g != fi)
            .map(|&g| cell.faces[g].tag)
    };
    // Self-overlap: facets of one strip meet only along shared rulings.
    for (fi, f) in cell.faces.iter().enumerate() {
        let CellTag::Facet { edge, k } = f.tag else {
            continue;
        };
        let m = f.verts.len();
        for i in 0..m {
            if let Some(CellTag::Facet { edge: e2, k: j }) =
                neighbor(fi, f.verts[i], f.verts[(i + 1) % m])
                && e2 == edge
                && j + 1 != k
                && k + 1 != j
            {
                return Err(unsupported);
            }
        }
    }

    let mut point_of: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pt = |i: usize, pts: &mut Pts| -> usize {
        *point_of.entry(i).or_insert_with(|| pts.add(cell.verts[i]))
    };
    let mut cuts: BTreeMap<EdgeId, Vec<Option<usize>>> = by_edge
        .iter()
        .map(|(&e, &(si, _))| (e, vec![None; strips[si].n + 1]))
        .collect();
    let mut extras: BTreeMap<EdgeId, Vec<Vec<usize>>> = by_edge
        .iter()
        .map(|(&e, &(si, _))| (e, vec![Vec::new(); strips[si].n]))
        .collect();
    let set_cut = |cuts: &mut BTreeMap<EdgeId, Vec<Option<usize>>>,
                   e: EdgeId,
                   k: usize,
                   p: usize|
     -> Result<(), PlanFail> {
        let slot = &mut cuts.get_mut(&e).ok_or(unsupported)?[k];
        match *slot {
            Some(x) if x != p => Err(unsupported),
            _ => {
                *slot = Some(p);
                Ok(())
            }
        }
    };
    // Which ruling of strip `edge` a ray between `tag` and `other` is.
    let ruling = |tag: CellTag, other: CellTag| -> Option<(EdgeId, usize)> {
        let side = |edge: EdgeId, f: FaceId| -> Option<usize> {
            let &(si, w) = by_edge.get(&edge)?;
            let info = strips[si].end_info(w);
            if f == merged(info.a) {
                Some(0)
            } else if f == merged(info.b) {
                Some(strips[si].n)
            } else {
                None
            }
        };
        match (tag, other) {
            (CellTag::Facet { edge, k }, CellTag::Facet { edge: e2, k: j }) if e2 == edge => {
                if j + 1 == k {
                    Some((edge, k))
                } else if k + 1 == j {
                    Some((edge, j))
                } else {
                    None
                }
            }
            (CellTag::Facet { edge, k }, CellTag::Face(f))
            | (CellTag::Face(f), CellTag::Facet { edge, k }) => {
                let r = side(edge, f)?;
                (r == k || r == k + 1).then_some((edge, r))
            }
            _ => None,
        }
    };

    let mut seen_faces: BTreeSet<FaceId> = BTreeSet::new();
    for (fi, f) in cell.faces.iter().enumerate() {
        if matches!(f.tag, CellTag::Box(_)) {
            continue;
        }
        if let CellTag::Extra(_) = f.tag {
            let ring: Vec<usize> = f.verts.iter().map(|&i| pt(i, pts)).collect();
            out.corner_faces.push(CornerFace {
                ring,
                claim: None,
                kind: Some(crate::topo::BlendKind::Chamfer),
                role: crate::topo::BlendRole::Corner,
                at: q,
                convex: !concave,
                soft: Vec::new(),
            });
            continue;
        }
        let m = f.verts.len();
        let real: Vec<bool> = f.verts.iter().map(|&i| !cell.on_box(i, &boxes)).collect();
        let starts: Vec<usize> = (0..m)
            .filter(|&i| real[i] && !real[(i + m - 1) % m])
            .collect();
        let [start] = starts.as_slice() else {
            return Err(unsupported);
        };
        let mut run: Vec<usize> = Vec::new();
        let mut i = *start;
        while real[i] {
            run.push(f.verts[i]);
            i = (i + 1) % m;
        }
        let before = f.verts[(*start + m - 1) % m];
        let after = f.verts[i];
        let (first, last) = (run[0], *run.last().expect("non-empty run"));
        let n_first = neighbor(fi, before, first).ok_or(unsupported)?;
        let n_last = neighbor(fi, last, after).ok_or(unsupported)?;
        match f.tag {
            CellTag::Face(face) => {
                seen_faces.insert(face);
                for (end_v, other) in [(first, n_first), (last, n_last)] {
                    let p = pt(end_v, pts);
                    match other {
                        CellTag::Face(g) => {
                            let e = ctx
                                .edge_between_faces(v, face, g)
                                .or_else(|| {
                                    tb.filter(|_| Some(face) == ta)
                                        .and_then(|b| ctx.edge_between_faces(v, b, g))
                                })
                                .ok_or(unsupported)?;
                            if ctx.picked.contains(&e) {
                                return Err(unsupported);
                            }
                            let list = out.shortened.entry(e).or_default();
                            if !list.contains(&(v, p)) {
                                list.push((v, p));
                            }
                        }
                        o => {
                            let (e, r) = ruling(f.tag, o).ok_or(unsupported)?;
                            set_cut(&mut cuts, e, r, p)?;
                        }
                    }
                }
                let mut ids: Vec<usize> = run.iter().map(|&i| pt(i, pts)).collect();
                if concave {
                    if ids.len() > 2 {
                        out.fill_points
                            .extend(ids[1..ids.len() - 1].iter().copied());
                    }
                    ids.reverse();
                }
                match (split_at, tb) {
                    (Some((g, at)), Some(b)) if Some(face) == ta => {
                        // The piece beside what the run starts (or ends)
                        // against.
                        let owner = |n: CellTag| -> Result<FaceId, PlanFail> {
                            Ok(match n {
                                CellTag::Face(h) => {
                                    if ctx.edge_between_faces(v, face, h).is_some() {
                                        face
                                    } else {
                                        b
                                    }
                                }
                                CellTag::Facet { edge, .. } => {
                                    let &(si, w) = by_edge.get(&edge).ok_or(unsupported)?;
                                    let info = strips[si].end_info(w);
                                    if info.a == b || info.b == b { b } else { face }
                                }
                                _ => return Err(unsupported),
                            })
                        };
                        let twin = |f: FaceId| if f == face { b } else { face };
                        seen_faces.insert(b);
                        // Where the line crosses a strip's side ruling past
                        // the run's end: that piece ends at the crossing, the
                        // other runs on through the run to it.
                        let on_ruling = |n: CellTag| -> Result<(usize, usize, usize), PlanFail> {
                            let CellTag::Facet { edge, .. } = n else {
                                return Err(unsupported);
                            };
                            let &(si, w) = by_edge.get(&edge).ok_or(unsupported)?;
                            let info = strips[si].end_info(w);
                            let side = if merged(info.a) == face { 0 } else { 1 };
                            Ok((si, w, side))
                        };
                        let x = match at {
                            SplitAt::Run(x) => {
                                // The run splits there: the part from its
                                // first point goes to the piece beside what
                                // it starts against, the rest to the other.
                                let k = run.iter().position(|&i| i == x).ok_or(unsupported)?;
                                let first_owner = owner(n_first)?;
                                out.face_runs.insert((first_owner, v), ids[..=k].to_vec());
                                out.face_runs
                                    .insert((twin(first_owner), v), ids[k..].to_vec());
                                ids[k]
                            }
                            SplitAt::Before(p) => {
                                let xi = pts.add(p);
                                let (si, w, side) = on_ruling(n_first)?;
                                ruling_crossing = Some((si, w, side, xi));
                                let o = owner(n_first)?;
                                let mut rest = vec![xi];
                                rest.extend(ids.iter().copied());
                                out.face_runs.insert((o, v), vec![xi]);
                                out.face_runs.insert((twin(o), v), rest);
                                xi
                            }
                            SplitAt::After(p) => {
                                let xi = pts.add(p);
                                let (si, w, side) = on_ruling(n_last)?;
                                ruling_crossing = Some((si, w, side, xi));
                                let o = owner(n_last)?;
                                let mut rest = ids.clone();
                                rest.push(xi);
                                out.face_runs.insert((o, v), vec![xi]);
                                out.face_runs.insert((twin(o), v), rest);
                                xi
                            }
                        };
                        out.shortened.entry(g).or_default().push((v, x));
                    }
                    _ => {
                        out.face_runs.insert((face, v), ids);
                    }
                }
            }
            CellTag::Facet { edge, k } => {
                let (ea, ra) = ruling(f.tag, n_first).ok_or(unsupported)?;
                let (eb, rb) = ruling(f.tag, n_last).ok_or(unsupported)?;
                if ea != edge || eb != edge || ra == rb {
                    return Err(unsupported);
                }
                let mut ids: Vec<usize> = run.iter().map(|&i| pt(i, pts)).collect();
                if ra == k + 1 {
                    ids.reverse();
                }
                set_cut(&mut cuts, edge, k, ids[0])?;
                set_cut(&mut cuts, edge, k + 1, *ids.last().expect("non-empty"))?;
                if ids.len() >= 2 {
                    extras.get_mut(&edge).ok_or(unsupported)?[k] = ids[1..ids.len() - 1].to_vec();
                }
            }
            _ => return Err(unsupported),
        }
    }
    if all_faces.iter().any(|f| !seen_faces.contains(f)) {
        return Err(PlanFail::Size(super::LimitFeature::AdjacentFace));
    }
    for (e, (si, w)) in by_edge {
        let cut = cuts.remove(&e).unwrap_or_default();
        if cut.iter().any(Option::is_none) {
            return Err(unsupported);
        }
        strips[si].ends[w] = StripEnd {
            ruling_cut: cut,
            facet_extra: extras.remove(&e).unwrap_or_default(),
        };
    }
    if let (Some((si, w, side, xi)), Some((g, _))) = (ruling_crossing, split_at) {
        // On the strip's last segment at this end, placed on the ruling as
        // cut once every construction has run.
        let strip = &mut strips[si];
        let seg = if w == 0 {
            0
        } else {
            strip.chain.verts.len() - 2
        };
        if !strip.mids[seg].is_empty() {
            return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
        }
        let x = pts.pos(xi);
        let pa = obj.vertices()[strip.chain.verts[seg]].position;
        let param = (x - pa).dot(strip.chain.seg_dir(obj, seg));
        strip.inserts[seg][side].push((param, xi));
        let far = obj.vertices()[other_end(ctx, g, v).ok_or(unsupported)?].position;
        out.crossings.push(super::plan::Crossing {
            point: xi,
            strip: si,
            seg,
            side,
            from: q,
            dir: (far - q).normalized().map_err(|_| unsupported)?,
            reach: (far - q).length(),
            at_start: w == 0,
        });
    }
    Ok(true)
}

/// Where the split line `g` of [`clip_vertex`] leaves the cut.
#[derive(Clone, Copy)]
enum SplitAt {
    /// At this cell vertex of the face's run (made one where needed).
    Run(usize),
    /// On the side ruling of the strip the run starts against (before
    /// its first point) or ends against (after its last), beyond the cut.
    Before(Point3),
    After(Point3),
}

/// Where the split line `g` (from `v`, between `t` and its coplanar twin)
/// leaves the clipped cell across `t`'s face: its first crossing of the
/// face's boundary. Inside the run it becomes a cell vertex (added to the
/// run's face and the face across that run edge) unless it is one; on the
/// cell edge from the cube to the run's first point, or from its last, it
/// lies on that strip's side ruling. A line that ends inside the cut is a
/// size limit.
fn split_cell(
    ctx: &Ctx,
    v: VertexId,
    g: EdgeId,
    t: FaceId,
    cell: &mut ConvexCell,
    boxes: &[Plane],
) -> Result<SplitAt, PlanFail> {
    let obj = ctx.obj;
    let q = obj.vertices()[v].position;
    let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at: q });
    let far = obj.vertices()[other_end(ctx, g, v).ok_or(unsupported)?].position;
    let reach = (far - q).length();
    let d = (far - q).normalized().map_err(|_| unsupported)?;
    let n = obj.faces()[t].plane.normal();
    let fi = cell
        .faces
        .iter()
        .position(|f| f.tag == CellTag::Face(t))
        .ok_or(unsupported)?;
    let verts = cell.faces[fi].verts.clone();
    let m = verts.len();
    let real: Vec<bool> = verts.iter().map(|&i| !cell.on_box(i, boxes)).collect();
    let Some(start) = (0..m).find(|&i| real[i] && !real[(i + m - 1) % m]) else {
        return Err(unsupported);
    };
    // The run, then the cell edges on either side of it: (local index of
    // the segment's first vertex, where it lies relative to the run).
    let mut run: Vec<usize> = Vec::new();
    let mut i = start;
    while real[i] && run.len() < m {
        run.push(i);
        i = (i + 1) % m;
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Where {
        Before,
        Inside,
        After,
    }
    let mut segs: Vec<(usize, Where)> = vec![((start + m - 1) % m, Where::Before)];
    segs.extend(run.windows(2).map(|w| (w[0], Where::Inside)));
    segs.push((*run.last().expect("non-empty"), Where::After));
    // In the face's plane: the line q + μ d against each segment.
    let side = n.cross(d);
    let mut best: Option<(f64, usize, Where, Point3)> = None;
    for &(j, at) in &segs {
        let (a, b) = (cell.verts[verts[j]], cell.verts[verts[(j + 1) % m]]);
        let (da, db) = ((a - q).dot(side), (b - q).dot(side));
        if (da > tol::POINT_MERGE && db > tol::POINT_MERGE)
            || (da < -tol::POINT_MERGE && db < -tol::POINT_MERGE)
            || (da - db).abs() <= f64::EPSILON
        {
            continue;
        }
        let x = a + (b - a) * (da / (da - db)).clamp(0.0, 1.0);
        let mu = (x - q).dot(d);
        if mu > tol::POINT_MERGE && best.is_none_or(|(bm, ..)| mu < bm) {
            best = Some((mu, j, at, x));
        }
    }
    let (mu, j, at, x) = best.ok_or(unsupported)?;
    if mu >= reach - tol::POINT_MERGE {
        return Err(PlanFail::Size(super::LimitFeature::Imprint));
    }
    let first = verts[run[0]];
    let last = verts[*run.last().expect("non-empty")];
    match at {
        Where::Before if !cell.verts[first].approx_eq(x, tol::POINT_MERGE) => {
            return Ok(SplitAt::Before(x));
        }
        Where::Before => return Ok(SplitAt::Run(first)),
        Where::After if !cell.verts[last].approx_eq(x, tol::POINT_MERGE) => {
            return Ok(SplitAt::After(x));
        }
        Where::After => return Ok(SplitAt::Run(last)),
        Where::Inside => {}
    }
    let (ia, ib) = (verts[j], verts[(j + 1) % m]);
    for &c in &[ia, ib] {
        if cell.verts[c].approx_eq(x, tol::POINT_MERGE) {
            return Ok(SplitAt::Run(c));
        }
    }
    let nx = cell.verts.len();
    cell.verts.push(x);
    for f in cell.faces.iter_mut() {
        let k = f.verts.len();
        for p in 0..k {
            let (u, w) = (f.verts[p], f.verts[(p + 1) % k]);
            if (u, w) == (ia, ib) || (u, w) == (ib, ia) {
                f.verts.insert(p + 1, nx);
                break;
            }
        }
    }
    Ok(SplitAt::Run(nx))
}

/// WP06 §5: three equal round strips meeting at a valence-3 vertex of
/// three flat faces roll into a sphere fan about the ball center.
fn rolled_corner(
    ctx: &Ctx,
    v: VertexId,
    ends: &[(usize, usize)],
    strips: &mut [Strip],
    pts: &mut Pts,
    out: &mut Out,
) -> Result<bool, PlanFail> {
    let obj = ctx.obj;
    if !matches!(ctx.profile, BlendProfile::Round { .. }) {
        return Ok(false);
    }
    // The radius each strip has at this end; all three must agree.
    let radii: Vec<f64> = ends
        .iter()
        .filter_map(|&(si, w)| strips[si].end_sec(w).radius)
        .collect();
    let Some(&r) = radii.first() else {
        return Ok(false);
    };
    if radii.len() != 3 || radii.iter().any(|x| (x - r).abs() > tol::POINT_MERGE) {
        return Ok(false);
    }
    if ctx.corners != CornerStyle::Default {
        return Ok(false);
    }
    let edges = ctx.edges_at(v);
    let faces = ctx.faces_at(v);
    if edges.len() != 3 || faces.len() != 3 || !edges.iter().all(|e| ctx.picked.contains(e)) {
        return Ok(false);
    }
    let convexity = strips[ends[0].0].convexity;
    if ends
        .iter()
        .any(|&(si, _)| strips[si].convexity != convexity)
    {
        return Ok(false);
    }
    // A corner on a faceted curved wall (a drawn arc meeting a line at a
    // kink) is a corner of three planar facets: a rim or along-axis strip
    // ending here re-sections against the facet planes, so one ball
    // touches all three faces and every rail stays on its face (D-45).
    let mut resection: Vec<Option<Section>> = Vec::with_capacity(3);
    for &(si, w) in ends {
        let info = strips[si].end_info(w);
        resection.push(match info.kind {
            EdgeKind::PlanePlane => None,
            EdgeKind::Rim { .. } | EdgeKind::AlongAxis => {
                match solve_section(ctx.profile, &Trace::ray(info.t_a), &Trace::ray(info.t_b)) {
                    Ok(sec) => Some(sec),
                    Err(_) => return Ok(false),
                }
            }
            EdgeKind::Swept => return Ok(false),
        });
    }
    let refaceted = resection.iter().any(Option::is_some);
    if !refaceted && faces.iter().any(|&f| obj.faces()[f].surface.is_some()) {
        return Ok(false);
    }
    let at = obj.vertices()[v].position;
    let unsupported = PlanFail::Hard(BlendError::UnsupportedVertex { at });
    let sigma = if convexity == Convexity::Convex {
        1.0
    } else {
        -1.0
    };
    // 1. Ball center: n_i·c = n_i·p_i − σ r.
    let rows: Vec<(Vec3, f64)> = faces
        .iter()
        .map(|&f| {
            let pl = obj.faces()[f].plane;
            let n = pl.normal();
            (n, n.dot(pl.point().to_vec()) - sigma * r)
        })
        .collect();
    let c = solve3(&rows).ok_or(unsupported)?;
    // 3. Touch points, one per face.
    let touch: BTreeMap<FaceId, usize> = faces
        .iter()
        .map(|&f| {
            let n = obj.faces()[f].plane.normal();
            (f, pts.add(c + n * (sigma * r)))
        })
        .collect();
    // 2. Each strip ends at the foot of c on its edge, sharing the touch
    // points as rails; `oriented[e]` gives its end cuts from side x to y.
    let mut end_cuts: BTreeMap<(FaceId, FaceId), Vec<usize>> = BTreeMap::new();
    for (i, &(si, w)) in ends.iter().enumerate() {
        let s = &strips[si];
        let info = s.end_info(w).clone();
        let (p0, p1) = obj.edge_endpoints(info.edge).ok_or(unsupported)?;
        let d = (p1 - p0).normalized().map_err(|_| unsupported)?;
        let foot = p0 + d * (c - p0).dot(d);
        let sec = resection[i].clone().unwrap_or_else(|| s.end_sec(w).clone());
        if let Some(ac) = sec.arc_center
            && !(foot + ac).approx_eq(c, tol::POINT_MERGE)
        {
            if refaceted {
                return Err(unsupported);
            }
            return Err(PlanFail::Hard(BlendError::BlendDegenerate));
        }
        let n = s.n;
        let mut cut: Vec<usize> = Vec::with_capacity(n + 1);
        for (k, &o) in sec.offsets.iter().enumerate() {
            cut.push(if k == 0 {
                touch[&info.a]
            } else if k == n {
                touch[&info.b]
            } else {
                pts.add(foot + o)
            });
        }
        let idx = s.end_index(w);
        if resection[i].is_some() {
            strips[si].secs[idx] = sec;
        }
        strips[si].qs[idx] = foot;
        strips[si].ends[w] = StripEnd {
            ruling_cut: cut.iter().copied().map(Some).collect(),
            facet_extra: vec![Vec::new(); n],
        };
        let mut rev = cut.clone();
        rev.reverse();
        end_cuts.insert((info.a, info.b), cut);
        end_cuts.insert((info.b, info.a), rev);
    }
    for &f in &faces {
        out.face_runs.insert((f, v), vec![touch[&f]]);
    }
    // 4. The pole: largest |n.z|, then |n.y|, |n.x|, then lowest id.
    let mut order = faces.clone();
    order.sort_by(|&a, &b| {
        let (na, nb) = (obj.faces()[a].plane.normal(), obj.faces()[b].plane.normal());
        let key = |x: f64, y: f64| {
            if (x - y).abs() <= tol::NORMAL_DIRECTION {
                std::cmp::Ordering::Equal
            } else {
                y.total_cmp(&x)
            }
        };
        key(na.z.abs(), nb.z.abs())
            .then(key(na.y.abs(), nb.y.abs()))
            .then(key(na.x.abs(), nb.x.abs()))
            .then(a.cmp(&b))
    });
    let (fp, fj, fk) = (order[0], order[1], order[2]);
    let opp = end_cuts.get(&(fj, fk)).ok_or(unsupported)?.clone();
    let m0 = end_cuts.get(&(fp, fj)).ok_or(unsupported)?.clone();
    let mn = end_cuts.get(&(fp, fk)).ok_or(unsupported)?.clone();
    let n = opp.len() - 1;
    let pole = touch[&fp];
    // 6. Meridians from the pole to each point of the opposite arc.
    let unit_from_c = |p: Point3| (p - c) / r;
    let mut grid: Vec<Vec<usize>> = Vec::with_capacity(n + 1);
    for (m, &end) in opp.iter().enumerate() {
        if m == 0 {
            grid.push(m0.clone());
            continue;
        }
        if m == n {
            grid.push(mn.clone());
            continue;
        }
        let (u0, u1) = (unit_from_c(pts.pos(pole)), unit_from_c(pts.pos(end)));
        let cosa = u0.dot(u1).clamp(-1.0, 1.0);
        let alpha = trig::acos(cosa);
        let w = (u1 - u0 * cosa).normalized().map_err(|_| unsupported)?;
        let mut col = Vec::with_capacity(n + 1);
        col.push(pole);
        for s_ in 1..n {
            let a = alpha * s_ as f64 / n as f64;
            col.push(pts.add(c + (u0 * trig::cos(a) + w * trig::sin(a)) * r));
        }
        col.push(end);
        grid.push(col);
    }
    // 7. Faces: triangles at the pole, quads (split when not planar) below.
    let claim = crate::topo::SurfaceRef::Sphere {
        center: c,
        radius: r,
    };
    let push = |ring: Vec<usize>, out: &mut Out| {
        let soft = (0..ring.len())
            .map(|i| (ring[i], ring[(i + 1) % ring.len()]))
            .collect();
        out.corner_faces.push(CornerFace {
            ring,
            claim: Some(claim),
            kind: None,
            role: crate::topo::BlendRole::Corner,
            at,
            convex: convexity == Convexity::Convex,
            soft,
        });
    };
    for m in 0..n {
        push(vec![pole, grid[m][1], grid[m + 1][1]], out);
        for s_ in 1..n {
            let quad = [
                grid[m][s_],
                grid[m][s_ + 1],
                grid[m + 1][s_ + 1],
                grid[m + 1][s_],
            ];
            let ps: Vec<Point3> = quad.iter().map(|&i| pts.pos(i)).collect();
            let planar = Plane::from_polygon(&ps).is_ok_and(|pl| {
                ps.iter()
                    .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
            });
            if planar {
                push(quad.to_vec(), out);
            } else {
                push(vec![quad[0], quad[1], quad[2]], out);
                push(vec![quad[0], quad[2], quad[3]], out);
            }
        }
    }
    Ok(true)
}

/// Solves the 3×3 system `n_i · x = d_i`.
fn solve3(rows: &[(Vec3, f64)]) -> Option<Point3> {
    let [(a, da), (b, db), (c, dc)] = rows else {
        return None;
    };
    let det = a.dot(b.cross(*c));
    if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let x = (b.cross(*c) * *da + c.cross(*a) * *db + a.cross(*b) * *dc) / det;
    Some(Point3::new(x.x, x.y, x.z))
}
