//! The rolling ball against the side surfaces themselves (D-72): the
//! fallback station solve for a swept chain the facet walk refuses, such as
//! a seam along a noisy scan whose sharp ridges bend under the ball.
//!
//! Each side is the set of facets near the station on its side of the
//! chain. The ball's center, in the section plane, is where its distance
//! to each side is the radius; the distance is to the facets as polygons,
//! so the contact is whatever is nearest (a facet's inside, a seam, or a
//! corner) and depends on the geometry alone, never on the path a walk
//! took to it. That makes the feet a function of the station, so bisecting
//! for where they change finds the change.

use crate::trig;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use super::{
    Foot, PlanFail, Section, Solver, Station, arc_section, common_edge, rail_point, unsupported,
};
use crate::ids::{EdgeId, FaceId};
use crate::math::{Point3, Vec3};
use crate::tol;
use crate::topo::Object;

/// A facet's loops, ready for nearest-point queries.
struct Facet {
    plane: crate::math::Plane,
    u: Vec3,
    v: Vec3,
    /// Each loop's 2D points in the plane, and per point its vertex and the
    /// edge to the next point.
    rings: Vec<Vec<[f64; 2]>>,
    points: Vec<Vec<Point3>>,
    feet: Vec<Vec<(crate::ids::VertexId, EdgeId)>>,
    /// A bounding sphere.
    center: Point3,
    radius: f64,
}

impl Facet {
    fn new(obj: &Object, f: FaceId) -> Facet {
        let plane = obj.faces()[f].plane;
        let (u, v) = crate::triangulate::plane_basis(plane.normal());
        let mut rings = Vec::new();
        let mut points = Vec::new();
        let mut feet = Vec::new();
        for l in obj.face_loops(f) {
            let hes: Vec<_> = obj.loop_half_edges(l).collect();
            // Onto the plane: a cut facet's points sit a few nanometers
            // off it, and the distance must not jump at its boundary.
            let ps: Vec<Point3> = hes
                .iter()
                .map(|&h| {
                    let p = obj.vertices()[obj.half_edges()[h].origin].position;
                    p - plane.normal() * plane.signed_distance(p)
                })
                .collect();
            rings.push(
                ps.iter()
                    .map(|p| [p.to_vec().dot(u), p.to_vec().dot(v)])
                    .collect(),
            );
            feet.push(
                hes.iter()
                    .map(|&h| (obj.half_edges()[h].origin, obj.half_edges()[h].edge))
                    .collect(),
            );
            points.push(ps);
        }
        let all = points.iter().flatten();
        let n = points.iter().map(Vec::len).sum::<usize>().max(1) as f64;
        let sum = all
            .clone()
            .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec());
        let center = Point3::new(sum.x / n, sum.y / n, sum.z / n);
        let radius = all.map(|&p| (p - center).length()).fold(0.0, f64::max);
        Facet {
            plane,
            u,
            v,
            rings,
            points,
            feet,
            center,
            radius,
        }
    }

    fn inside(&self, p: [f64; 2]) -> bool {
        let mut inside = false;
        for r in &self.rings {
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                if (a[1] > p[1]) != (b[1] > p[1]) {
                    let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    if p[0] < x {
                        inside = !inside;
                    }
                }
            }
        }
        inside
    }

    /// The point of the facet nearest `c`, the foot it lies on, and the
    /// distance. Unsigned, so continuous wherever `c` goes; which side the
    /// ball is on is checked once it has settled.
    fn nearest(&self, f: FaceId, c: Point3) -> (f64, Point3, Foot) {
        let sd = self.plane.signed_distance(c);
        let p = c - self.plane.normal() * sd;
        if self.inside([p.to_vec().dot(self.u), p.to_vec().dot(self.v)]) {
            return (sd.abs(), p, Foot::Face(f));
        }
        let mut best = (f64::INFINITY, p, Foot::Face(f));
        for (ps, ft) in self.points.iter().zip(&self.feet) {
            let m = ps.len();
            for i in 0..m {
                let (a, b) = (ps[i], ps[(i + 1) % m]);
                let ab = b - a;
                let l2 = ab.dot(ab);
                let t = if l2 > 0.0 {
                    ((c - a).dot(ab) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = a + ab * t;
                let d = (c - q).length();
                if d < best.0 {
                    // Within a hair of an end the contact is that corner.
                    let slack = tol::SWEPT_SLACK / l2.sqrt().max(tol::POINT_MERGE);
                    let foot = if t <= slack {
                        Foot::Corner(ft[i].0)
                    } else if t >= 1.0 - slack {
                        Foot::Corner(ft[(i + 1) % m].0)
                    } else {
                        Foot::Seam(ft[i].1)
                    };
                    best = (d, q, foot);
                }
            }
        }
        best
    }
}

/// The side surfaces near a station, kept while the stations stay close.
pub(super) struct Surface {
    facets: RefCell<BTreeMap<FaceId, Rc<Facet>>>,
    /// Each side's facets near the segment from `.0` to `.1`.
    near: RefCell<Option<(Point3, Point3, Rc<Sides>)>>,
    chain_edges: BTreeSet<EdgeId>,
    /// Recent stations' chain points and ball centers: each solve starts
    /// from the nearest, so where a hollow gives the ball two resting places
    /// it stays on the one it rolled into.
    recent: RefCell<VecDeque<(Point3, Point3)>>,
    /// The chain's faces on each side, to seed the gathering.
    seeds: [Vec<FaceId>; 2],
}

/// One side's facets near a point, nearest first by how close each could
/// come to it (center distance less bounding radius), so a nearest-point
/// query stops once no facet left could be nearer.
pub(super) struct SideSet {
    at: Point3,
    facets: Vec<(f64, FaceId, Rc<Facet>)>,
}

/// Each side's facets near a station.
type Sides = [SideSet; 2];

/// How far, in radii, a side is gathered around the segment from a chain
/// point to the ball's center (the ball only reaches a radius past its
/// center; the corridor from the chain covers where the side runs to it).
const REACH: f64 = 1.5;
/// How far, in radii, the chain point or the center may move before the
/// sides are gathered again.
const DRIFT: f64 = 0.5;
/// How near, in radii, the ball must come to touching both sides: where
/// the nearest facet changes the distance kinks and a solve can stall a few
/// nanometers short, well inside a scan's own noise.
const SETTLE: f64 = 1e-5;
/// How many rounds [`fold_free`] takes out folding stations.
const FOLD_ROUNDS: usize = 16;
/// How many times a solve gathers the sides again as its ball settles.
const GATHER_ROUNDS: usize = 4;
/// How many recent stations a solve may start from.
const RECENT: usize = 64;
/// A guard on the facets gathered per side.
const MAX_FACETS: usize = 200_000;

impl Surface {
    pub(super) fn new(chain: &super::Chain) -> Surface {
        Surface {
            facets: RefCell::new(BTreeMap::new()),
            near: RefCell::new(None),
            recent: RefCell::new(VecDeque::new()),
            chain_edges: chain.edges.iter().map(|e| e.edge).collect(),
            seeds: [
                chain.edges.iter().map(|e| e.a).collect(),
                chain.edges.iter().map(|e| e.b).collect(),
            ],
        }
    }

    fn facet(&self, obj: &Object, f: FaceId) -> Rc<Facet> {
        self.facets
            .borrow_mut()
            .entry(f)
            .or_insert_with(|| Rc::new(Facet::new(obj, f)))
            .clone()
    }

    /// Side `x`'s facets within reach of the segment from `q` to `c`:
    /// gathered across edges from the chain's faces on that side, never
    /// across the chain.
    fn gather(
        &self,
        sv: &Solver,
        x: usize,
        q: Point3,
        c: Point3,
        reach: f64,
    ) -> Result<SideSet, PlanFail> {
        let obj = sv.obj;
        let close = |f: FaceId| {
            let fc = self.facet(obj, f);
            super::super::query::point_segment_distance(fc.center, q, c) - fc.radius <= reach
        };
        let mut seen: BTreeSet<FaceId> = BTreeSet::new();
        let mut queue: VecDeque<FaceId> = VecDeque::new();
        for &f in &self.seeds[x] {
            if close(f) && seen.insert(f) {
                queue.push_back(f);
            }
        }
        let mut out = Vec::new();
        while let Some(f) = queue.pop_front() {
            out.push((f, self.facet(obj, f)));
            if out.len() > MAX_FACETS {
                return Err(unsupported(q));
            }
            for l in obj.face_loops(f) {
                for h in obj.loop_half_edges(l) {
                    let e = obj.half_edges()[h].edge;
                    if self.chain_edges.contains(&e) {
                        continue;
                    }
                    let Some((a, b)) = obj.edge_faces(e) else {
                        continue;
                    };
                    let g = if a == f { b } else { a };
                    // A side keeps to one claim: it runs on across seams
                    // between facets of one claimed surface, and across any
                    // edge between unclaimed faces (a scan, an import, a
                    // generated wall; sharp ones included, as before), but
                    // never between a claimed face and a face of another
                    // surface or none (a disc's rim, a pipe's end cap),
                    // where the ball would ride that edge instead of
                    // rolling on the side (D-229).
                    let same_surface = match (obj.faces()[f].surface, obj.faces()[g].surface) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.same_surface(&b),
                        _ => false,
                    };
                    if same_surface
                        && !sv.other_side[x].contains(&g)
                        && !seen.contains(&g)
                        && close(g)
                    {
                        seen.insert(g);
                        queue.push_back(g);
                    }
                }
            }
        }
        let mut facets: Vec<(f64, FaceId, Rc<Facet>)> = out
            .into_iter()
            .map(|(f, fc)| ((c - fc.center).length() - fc.radius, f, fc))
            .collect();
        facets.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(SideSet { at: c, facets })
    }

    /// Each side's facets near the segment from `q` to `c` (as last
    /// gathered, while both ends are within [`DRIFT`] of where they were).
    fn sides(&self, sv: &Solver, q: Point3, c: Point3, r: f64) -> Result<Rc<Sides>, PlanFail> {
        if let Some((q0, c0, sides)) = &*self.near.borrow()
            && (q - *q0).length() <= DRIFT * r
            && (c - *c0).length() <= DRIFT * r
        {
            return Ok(sides.clone());
        }
        let reach = (REACH + DRIFT) * r;
        let sides = Rc::new([
            self.gather(sv, 0, q, c, reach)?,
            self.gather(sv, 1, q, c, reach)?,
        ]);
        *self.near.borrow_mut() = Some((q, c, sides.clone()));
        Ok(sides)
    }

    /// The nearest point of a side's facets to `c`.
    fn nearest(side: &SideSet, c: Point3) -> Option<(f64, Point3, Foot)> {
        let mut best: Option<(f64, Point3, Foot)> = None;
        let dc = (c - side.at).length();
        for (k, f, fc) in &side.facets {
            let f = *f;
            let bound = best.map_or(f64::INFINITY, |b| b.0);
            // No facet from here on comes nearer `c` than `k - dc`.
            if k - dc >= bound {
                break;
            }
            if (c - fc.center).length() - fc.radius >= bound {
                continue;
            }
            let hit = fc.nearest(f, c);
            if best.is_none_or(|b| hit.0 < b.0) {
                best = Some(hit);
            }
        }
        best
    }
}

/// One side's contact during the solve: a pinned foot, or the side's
/// nearest point.
enum Side<'a> {
    Pinned(Foot),
    Free(&'a SideSet),
}

/// Distance from `c` to a pinned foot's plane, line, or point, its
/// gradient, and the contact point.
fn pinned(sv: &Solver, foot: Foot, c: Point3, q: Point3) -> Result<(f64, Vec3, Point3), PlanFail> {
    let obj = sv.obj;
    Ok(match foot {
        Foot::Face(f) => {
            let pl = obj.faces()[f].plane;
            let d = -sv.sigma * pl.signed_distance(c);
            (
                d,
                pl.normal() * (-sv.sigma),
                c + pl.normal() * (sv.sigma * d),
            )
        }
        Foot::Seam(e) => {
            let (l0, l1) = obj.edge_endpoints(e).ok_or_else(|| unsupported(q))?;
            let ld = (l1 - l0).normalized().map_err(|_| unsupported(q))?;
            let w = c - l0;
            let p = l0 + ld * w.dot(ld);
            let d = (c - p).length().max(tol::NORMALIZE_MIN_LENGTH);
            (d, (c - p) / d, p)
        }
        Foot::Corner(v) => {
            let p = obj.vertices()[v].position;
            let d = (c - p).length().max(tol::NORMALIZE_MIN_LENGTH);
            (d, (c - p) / d, p)
        }
    })
}

/// The station at chain point `q` (section direction `t`) against the side
/// surfaces, `pinned` fixing a side's foot; `start` gives the first guess.
pub(super) fn solve(
    sv: &Solver,
    surf: &Surface,
    q: Point3,
    t: Vec3,
    start: [Foot; 2],
    pin: [Option<Foot>; 2],
) -> Result<(Section, [Foot; 2]), PlanFail> {
    let super::BlendProfile::Round {
        radius: r,
        segments,
    } = *sv.profile
    else {
        return Err(unsupported(q));
    };
    // The first guess: the ball between the start feet's planes.
    let first = start.map(|f| match f {
        Foot::Face(_) => f,
        _ => f.faces(sv.obj).first().map_or(f, |&g| Foot::Face(g)),
    });
    let warm = surf
        .recent
        .borrow()
        .iter()
        .filter(|(rq, _)| (*rq - q).length() <= r)
        .min_by(|a, b| (a.0 - q).length().total_cmp(&(b.0 - q).length()))
        .map(|&(_, rc)| rc - t * (rc - q).dot(t));
    let c0 = match warm {
        Some(c) => c,
        None => {
            q + sv
                .section(q, t, first)?
                .arc_center
                .ok_or_else(|| unsupported(q))?
        }
    };
    // Gathered about the first guess's corridor; where the ball settles
    // farther off than the gathering allows, gathered again about it.
    let mut c = c0;
    for _ in 0..GATHER_ROUNDS {
        let sides = surf.sides(sv, q, c, r)?;
        let (sec, feet, settled) =
            solve_with(sv, surf, &sides, q, t, pin, c, r, segments as usize)?;
        if Rc::ptr_eq(&surf.sides(sv, q, settled, r)?, &sides) {
            // The ball rests on its sides, not on the edge where one of them
            // ends: a seam or corner foot has every face around it on that
            // side (D-229). Past a side's last face the round is too large
            // for it (a disc's rim under a ball wider than the disc's top).
            for (x, foot) in feet.iter().enumerate() {
                if !rests_on_side(sv, surf, &sides[x], *foot) {
                    return Err(unsupported(q));
                }
            }
            let mut recent = surf.recent.borrow_mut();
            recent.push_back((q, settled));
            if recent.len() > RECENT {
                recent.pop_front();
            }
            return Ok((sec, feet));
        }
        c = settled;
    }
    Err(unsupported(q))
}

/// Whether `foot` lies within `side`: a facet of it, a seam between two of
/// its facets, or a corner all of whose faces are its facets (a chain
/// vertex, where the other side's faces meet it too, always is).
fn rests_on_side(sv: &Solver, surf: &Surface, side: &SideSet, foot: Foot) -> bool {
    let obj = sv.obj;
    let on = |f: FaceId| side.facets.iter().any(|(_, g, _)| *g == f);
    match foot {
        Foot::Face(_) => true,
        Foot::Seam(e) => surf.chain_edges.contains(&e) || foot.faces(obj).into_iter().all(on),
        Foot::Corner(v) => {
            let at_chain = surf.chain_edges.iter().any(|&e| {
                obj.edge_vertex_ids(e)
                    .is_some_and(|(a, b)| a == v || b == v)
            });
            at_chain || foot.faces(obj).into_iter().all(on)
        }
    }
}

/// [`solve`] against one gathering of the sides, from the first guess `c`:
/// the section, the feet, and where the ball's center settled.
#[allow(clippy::too_many_arguments)]
fn solve_with(
    sv: &Solver,
    _surf: &Surface,
    sides: &Sides,
    q: Point3,
    t: Vec3,
    pin: [Option<Foot>; 2],
    mut c: Point3,
    r: f64,
    segments: usize,
) -> Result<(Section, [Foot; 2], Point3), PlanFail> {
    let side = |x: usize| match pin[x] {
        Some(f) => Side::Pinned(f),
        None => Side::Free(&sides[x]),
    };
    let contact = |x: usize, c: Point3| -> Result<(f64, Vec3, Point3, Foot), PlanFail> {
        match side(x) {
            Side::Pinned(f) => {
                let (d, g, p) = pinned(sv, f, c, q)?;
                Ok((d, g, p, f))
            }
            Side::Free(fs) => {
                let (d, p, f) = Surface::nearest(fs, c).ok_or_else(|| unsupported(q))?;
                let g = match f {
                    Foot::Face(h) => {
                        let n = sv.obj.faces()[h].plane.normal();
                        if n.dot(c - p) >= 0.0 { n } else { -n }
                    }
                    _ => (c - p) / d.max(tol::NORMALIZE_MIN_LENGTH),
                };
                Ok((d, g, p, f))
            }
        }
    };
    let (u, v) = crate::triangulate::plane_basis(t);
    let res = |c: Point3| -> Result<f64, PlanFail> {
        let (d0, ..) = contact(0, c)?;
        let (d1, ..) = contact(1, c)?;
        Ok((d0 - r).abs().max((d1 - r).abs()))
    };
    let mut err = res(c)?;
    for _ in 0..100 {
        if err <= tol::BLEND_CONTACT_RESIDUAL {
            break;
        }
        let (d0, g0, ..) = contact(0, c)?;
        let (d1, g1, ..) = contact(1, c)?;
        let (r0, r1) = (d0 - r, d1 - r);
        let (j11, j12, j21, j22) = (g0.dot(u), g0.dot(v), g1.dot(u), g1.dot(v));
        let dj = j11 * j22 - j12 * j21;
        if dj.abs() <= tol::NORMALIZE_MIN_LENGTH {
            return Err(unsupported(q));
        }
        let dx = (r0 * j22 - r1 * j12) / dj;
        let dy = (j11 * r1 - j21 * r0) / dj;
        // Backtrack while the step does not help: the nearest-point
        // distance kinks where the nearest facet changes.
        let mut step = 1.0;
        let mut moved = false;
        for _ in 0..30 {
            let cn = c - u * (dx * step) - v * (dy * step);
            let en = res(cn)?;
            if en < err {
                c = cn;
                err = en;
                moved = true;
                break;
            }
            step *= 0.5;
        }
        if !moved {
            break;
        }
    }
    // Polish on the nearest feet themselves (exact planes, lines, points),
    // moving on to the next nearest until they agree; this also settles a
    // solve the kinks stalled.
    let mut tried: BTreeSet<[Foot; 2]> = BTreeSet::new();
    for _ in 0..32 {
        if err <= tol::BLEND_CONTACT_RESIDUAL {
            break;
        }
        let feet = [contact(0, c)?.3, contact(1, c)?.3];
        if !tried.insert(feet) {
            break;
        }
        let Ok(sec) = sv.section(q, t, feet) else {
            break;
        };
        let cn = q + sec.arc_center.ok_or_else(|| unsupported(q))?;
        let en = res(cn)?;
        if en < err {
            c = cn;
            err = en;
        }
    }
    if err > tol::BLEND_CONTACT_RESIDUAL {
        // Still short at a kink: along the line where one side's facet plane
        // is touched, bracket where the other side is touched too; that
        // distance is continuous across its kinks.
        let at = c;
        for y in 0..2 {
            let Foot::Face(h) = contact(y, at)?.3 else {
                continue;
            };
            let n = sv.obj.faces()[h].plane.normal();
            let (Ok(m), Ok(w)) = ((n - t * t.dot(n)).normalized(), t.cross(n).normalized()) else {
                continue;
            };
            // Onto the line: -σ·sd(c + m·k) = r.
            let dn = -sv.sigma * m.dot(n);
            if dn.abs() <= tol::NORMALIZE_MIN_LENGTH {
                continue;
            }
            let c0 = at + m * ((r - contact(y, at)?.0) / dn);
            let g = |s: f64| -> Result<f64, PlanFail> { Ok(contact(1 - y, c0 + w * s)?.0 - r) };
            let g0 = g(0.0)?;
            let mut bracket = None;
            let mut h = tol::BLEND_BRACKET_START;
            while h <= r && bracket.is_none() {
                for s1 in [h, -h] {
                    if g(s1)?.signum() != g0.signum() {
                        bracket = Some((0.0, s1));
                        break;
                    }
                }
                h *= 2.0;
            }
            let Some((mut a, mut b)) = bracket else {
                continue;
            };
            let ga = g0;
            for _ in 0..100 {
                let mid = 0.5 * (a + b);
                if g(mid)?.signum() == ga.signum() {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            let cn = c0 + w * (0.5 * (a + b));
            let en = res(cn)?;
            if en < err {
                c = cn;
                err = en;
            }
        }
    }
    if err > tol::PLANE_DIST.max(SETTLE * r) {
        return Err(unsupported(q));
    }
    let (_, _, pa, fa) = contact(0, c)?;
    let (_, _, pb, fb) = contact(1, c)?;
    let feet = [fa, fb];
    // The ball stays outside both sides at its contacts: in front of a
    // touched facet, and of every facet at a touched seam or corner.
    for &f in &feet {
        if f.faces(sv.obj)
            .iter()
            .any(|&h| -sv.sigma * sv.obj.faces()[h].plane.signed_distance(c) < -tol::PLANE_DIST)
        {
            return Err(unsupported(q));
        }
    }
    Ok((arc_section(q, c, r, pa, pb, segments)?, feet, c))
}

/// The stations between `a` and `b`, one chain point apart, where side `x`
/// touches two places at once (across a hollow under the ball): the rail
/// steps from one contact to the other across the facets between them,
/// crossing each seam at its point nearest the line between the contacts;
/// the other points lie on the ball, turning from `a`'s section to `b`'s.
pub(super) fn jump(
    sv: &Solver,
    surf: &Surface,
    a: &Station,
    b: &Station,
    x: usize,
) -> Result<Vec<Station>, PlanFail> {
    let obj = sv.obj;
    let q = a.q;
    let super::BlendProfile::Round { radius: r, .. } = *sv.profile else {
        return Err(unsupported(q));
    };
    let center = |s: &Station| {
        s.sec
            .arc_center
            .map(|c| s.q + c)
            .ok_or_else(|| unsupported(q))
    };
    let c = {
        let (ca, cb) = (center(a)?, center(b)?);
        ca + (cb - ca) * 0.5
    };
    let (p1, p2) = (rail_point(a, x), rail_point(b, x));
    let from = a.feet[x].faces(obj);
    let to: BTreeSet<FaceId> = b.feet[x].faces(obj).into_iter().collect();
    if from.iter().any(|f| to.contains(f)) {
        return Ok(Vec::new());
    }
    // Along the surface's cut by the plane through the center and the two
    // contacts where it gets there; else seam by seam across the facets
    // between, each at the cut's crossing.
    let crossed: Vec<(EdgeId, Point3)> = match slice_walk(sv, surf, x, &from, &to, p1, p2, c, r) {
        Some(walk) => walk,
        None => {
            let path =
                facet_path(sv, surf, x, &from, &to, p1, p2, r)?.ok_or_else(|| unsupported(q))?;
            path.windows(2)
                .map(|w| {
                    let e = common_edge(obj, w[0], w[1]).ok_or_else(|| unsupported(q))?;
                    let (e0, e1) = obj.edge_endpoints(e).ok_or_else(|| unsupported(q))?;
                    Ok((e, slice_point(e0, e1, c, p1, p2)))
                })
                .collect::<Result<_, PlanFail>>()?
        }
    };
    let crossed = at_corners(obj, crossed);
    let n = a.sec.offsets.len() - 1;
    let rail = if x == 0 { 0 } else { n };
    let mut out = Vec::with_capacity(crossed.len());
    for &(foot, s) in &crossed {
        let o = a.q + a.sec.offsets[n - rail];
        let offsets = ball_arc(q, c, r, if x == 0 { [s, o] } else { [o, s] }, n)?;
        let mut feet = a.feet;
        feet[x] = foot;
        out.push(Station {
            q,
            sec: Section {
                offsets,
                arc_center: Some(c - q),
                ..a.sec.clone()
            },
            feet,
        });
    }
    Ok(out)
}

/// A section's points from rail point `ends[0]` to rail point `ends[1]`
/// (as offsets from `q`): the points between on the ball about `c`, from the
/// direction of one rail point to the other's, so the strip next to a rail
/// leaves it toward the ball, never along the side.
fn ball_arc(
    q: Point3,
    c: Point3,
    r: f64,
    ends: [Point3; 2],
    n: usize,
) -> Result<Vec<Vec3>, PlanFail> {
    let u0 = (ends[0] - c).normalized().map_err(|_| unsupported(q))?;
    let u1 = (ends[1] - c).normalized().map_err(|_| unsupported(q))?;
    let cosphi = u0.dot(u1).clamp(-1.0, 1.0);
    let phi = trig::acos(cosphi);
    let w = (u1 - u0 * cosphi)
        .normalized()
        .map_err(|_| unsupported(q))?;
    let mut out = Vec::with_capacity(n + 1);
    out.push(ends[0] - q);
    for k in 1..n {
        let a = phi * k as f64 / n as f64;
        out.push(c + (u0 * trig::cos(a) + w * trig::sin(a)) * r - q);
    }
    out.push(ends[1] - q);
    Ok(out)
}

/// A rail's seam crossings, those within a hair of one another round a
/// vertex of the scan taken as one, on that vertex: a cut that passes a
/// vertex crosses each edge there a fraction of a micron apart, and the
/// strip would keep slivers between.
fn at_corners(obj: &Object, crossed: Vec<(EdgeId, Point3)>) -> Vec<(Foot, Point3)> {
    let slack = tol::SWEPT_SLACK;
    let ends = |e: EdgeId| -> Option<[crate::ids::VertexId; 2]> {
        let he = obj.half_edges()[obj.edges().get(e)?.half_edge];
        Some([he.origin, obj.half_edges()[he.next].origin])
    };
    let mut out: Vec<(Foot, Point3)> = Vec::with_capacity(crossed.len());
    let mut i = 0;
    while i < crossed.len() {
        let (e, p) = crossed[i];
        // The vertex of this edge the crossing is within a hair of.
        let corner = ends(e).and_then(|vs| {
            vs.into_iter()
                .find(|&v| (obj.vertices()[v].position - p).length() <= 2.0 * slack)
        });
        let Some(v) = corner else {
            out.push((Foot::Seam(e), p));
            i += 1;
            continue;
        };
        // Every crossing after it through the same vertex.
        let mut j = i + 1;
        while j < crossed.len()
            && ends(crossed[j].0).is_some_and(|vs| vs.contains(&v))
            && (crossed[j].1 - obj.vertices()[v].position).length() <= 2.0 * slack
        {
            j += 1;
        }
        out.push((Foot::Corner(v), obj.vertices()[v].position));
        i = j;
    }
    out
}

/// The seams a rail crosses going from `p1` (on a facet of `from`) to `p2`
/// (on a facet of `to`) along side `x`'s cut by the plane through `c`, `p1`
/// and `p2`: each seam with the point the plane crosses it, in order,
/// always moving on toward `p2`. `None` where that walk does not get there
/// (the plane leaves the side's facets near the two points).
#[allow(clippy::too_many_arguments)]
fn slice_walk(
    sv: &Solver,
    surf: &Surface,
    x: usize,
    from: &[FaceId],
    to: &BTreeSet<FaceId>,
    p1: Point3,
    p2: Point3,
    c: Point3,
    r: f64,
) -> Option<Vec<(EdgeId, Point3)>> {
    let obj = sv.obj;
    let n = (p1 - c).cross(p2 - c).normalized().ok()?;
    let dir = (p2 - p1).normalized().ok()?;
    let sides = surf.sides(sv, p1, p2, r).ok()?;
    let near: BTreeSet<FaceId> = sides[x].facets.iter().map(|(_, f, _)| *f).collect();
    let crossings = |f: FaceId| -> Vec<(EdgeId, Point3)> {
        let mut out = Vec::new();
        for l in obj.face_loops(f) {
            for h in obj.loop_half_edges(l) {
                let e = obj.half_edges()[h].edge;
                let Some((a, b)) = obj.edge_endpoints(e) else {
                    continue;
                };
                let (sa, sb) = (n.dot(a - c), n.dot(b - c));
                if (sa > 0.0 && sb > 0.0) || (sa < 0.0 && sb < 0.0) || sa == sb {
                    continue;
                }
                out.push((e, a + (b - a) * (sa / (sa - sb))));
            }
        }
        out
    };
    let mut path = Vec::new();
    let mut at = p1;
    let mut cur: Option<(FaceId, Option<EdgeId>)> = None;
    // The facet of `from` the plane leaves toward `p2`.
    for &f in from {
        if crossings(f).iter().any(|&(_, p)| (p - at).dot(dir) > 0.0) {
            cur = Some((f, None));
            break;
        }
    }
    let (mut f, mut came) = cur?;
    for _ in 0..4096 {
        if to.contains(&f) {
            return Some(path);
        }
        let (e, p) = crossings(f)
            .into_iter()
            .filter(|&(e, p)| Some(e) != came && (p - at).dot(dir) > -tol::POINT_MERGE)
            .max_by(|a, b| {
                (a.1 - at)
                    .dot(dir)
                    .total_cmp(&(b.1 - at).dot(dir))
                    .reverse()
            })?;
        if (p - p1).dot(dir) > (p2 - p1).dot(dir) + r {
            return None;
        }
        let (g1, g2) = obj.edge_faces(e)?;
        let g = if g1 == f { g2 } else { g1 };
        if sv.other_side[x].contains(&g) || !(near.contains(&g) || to.contains(&g)) {
            return None;
        }
        path.push((e, p));
        at = p;
        came = Some(e);
        f = g;
    }
    None
}

/// The facets of side `x` from one of `from` to one of `to`, each sharing
/// an edge with the next (breadth-first over the side's facets near the
/// segment `p1`–`p2`); `None` where they share one already.
#[allow(clippy::too_many_arguments)]
fn facet_path(
    sv: &Solver,
    surf: &Surface,
    x: usize,
    from: &[FaceId],
    to: &BTreeSet<FaceId>,
    p1: Point3,
    p2: Point3,
    r: f64,
) -> Result<Option<Vec<FaceId>>, PlanFail> {
    let obj = sv.obj;
    if from.iter().any(|f| to.contains(f)) {
        return Ok(None);
    }
    let m = p1 + (p2 - p1) * 0.5;
    let sides = surf.sides(sv, p1, p2, r)?;
    let span = 0.5 * (p2 - p1).length() + r;
    let side: BTreeSet<FaceId> = sides[x]
        .facets
        .iter()
        .filter(|(_, _, fc)| (m - fc.center).length() - fc.radius <= span)
        .map(|(_, f, _)| *f)
        .collect();
    let mut prev: BTreeMap<FaceId, FaceId> = BTreeMap::new();
    let mut queue: VecDeque<FaceId> = from.iter().copied().collect();
    let mut seen: BTreeSet<FaceId> = from.iter().copied().collect();
    while let Some(f) = queue.pop_front() {
        if to.contains(&f) {
            let mut path = vec![f];
            while let Some(&p) = prev.get(path.last().expect("a path")) {
                path.push(p);
            }
            path.reverse();
            return Ok(Some(path));
        }
        for l in obj.face_loops(f) {
            for h in obj.loop_half_edges(l) {
                let Some((g1, g2)) = obj.edge_faces(obj.half_edges()[h].edge) else {
                    continue;
                };
                let g = if g1 == f { g2 } else { g1 };
                if (side.contains(&g) || to.contains(&g)) && seen.insert(g) {
                    prev.insert(g, f);
                    queue.push_back(g);
                }
            }
        }
    }
    Err(unsupported(p1))
}

/// The stations that carry side `x`'s rail from station `a` to station `b`
/// across the facets between their feet: one per seam crossed, at its point
/// nearest the line between the two rail points, the chain point, the
/// ball's center and the other points carried in proportion from `a`'s to
/// `b`'s. The other side's rail is carried too where the two stations'
/// feet there share a facet, and stays at `a`'s otherwise.
fn link(
    sv: &Solver,
    surf: &Surface,
    a: &Station,
    b: &Station,
    x: usize,
) -> Result<Vec<Station>, PlanFail> {
    let obj = sv.obj;
    let super::BlendProfile::Round { radius: r, .. } = *sv.profile else {
        return Err(unsupported(a.q));
    };
    let (p1, p2) = (rail_point(a, x), rail_point(b, x));
    let to: BTreeSet<FaceId> = b.feet[x].faces(obj).into_iter().collect();
    let from = a.feet[x].faces(obj);
    if from.iter().any(|f| to.contains(f)) {
        return Ok(Vec::new());
    }
    let center = |s: &Station| {
        s.sec
            .arc_center
            .map(|c| s.q + c)
            .ok_or_else(|| unsupported(a.q))
    };
    let (ca, cb) = (center(a)?, center(b)?);
    // Along the surface's cut between the two rail points where it gets
    // there; else seam by seam across the facets between.
    let crossed: Vec<(EdgeId, Point3)> =
        match slice_walk(sv, surf, x, &from, &to, p1, p2, ca + (cb - ca) * 0.5, r) {
            Some(walk) => walk,
            None => {
                let Some(path) = facet_path(sv, surf, x, &from, &to, p1, p2, r)? else {
                    return Ok(Vec::new());
                };
                path.windows(2)
                    .map(|w| {
                        let e = common_edge(obj, w[0], w[1]).ok_or_else(|| unsupported(a.q))?;
                        let (e0, e1) = obj.edge_endpoints(e).ok_or_else(|| unsupported(a.q))?;
                        Ok((e, nearest_on_segment(e0, e1, p1, p2)))
                    })
                    .collect::<Result<_, PlanFail>>()?
            }
        };
    let n = a.sec.offsets.len() - 1;
    let rail = if x == 0 { 0 } else { n };
    let crossed = at_corners(obj, crossed);
    let shared = a.feet[1 - x]
        .faces(obj)
        .into_iter()
        .find(|f| b.feet[1 - x].faces(obj).contains(f));
    let k = crossed.len();
    let mut out = Vec::with_capacity(k);
    for (i, &(foot, s)) in crossed.iter().enumerate() {
        let t = (i + 1) as f64 / (k + 1) as f64;
        let q = a.q + (b.q - a.q) * t;
        let c = ca + (cb - ca) * t;
        let mut feet = a.feet;
        feet[x] = foot;
        let (oa, ob) = (a.q + a.sec.offsets[n - rail], b.q + b.sec.offsets[n - rail]);
        let o = match shared {
            Some(_) => oa + (ob - oa) * t,
            None => oa,
        };
        let offsets = ball_arc(q, c, r, if x == 0 { [s, o] } else { [o, s] }, n)?;
        if let Some(f) = shared {
            feet[1 - x] = Foot::Face(f);
        }
        out.push(Station {
            q,
            sec: Section {
                offsets,
                arc_center: Some(c - q),
                ..a.sec.clone()
            },
            feet,
        });
    }
    Ok(out)
}

/// The way the stations run at each, across the chain points either side
/// that differ from its own (several stations share a point at a step).
pub(super) fn headings(stations: &[Station]) -> Vec<Option<Vec3>> {
    let ns = stations.len();
    (0..ns)
        .map(|j| {
            let q = stations[j].q;
            let differs = |i: &usize| (stations[*i].q - q).length() > tol::POINT_MERGE;
            let next = (j + 1..ns).find(differs)?;
            let prev = (0..j).rev().find(differs).unwrap_or(j);
            (stations[next].q - stations[prev].q).normalized().ok()
        })
        .collect()
}

/// Each station's section points as the strip will take them: a point that
/// would step back along the way the stations run (an inner one, or a rail
/// on the same foot as the station before) stays where it was (see
/// `plan_swept`).
pub(super) fn held_rows(stations: &[Station], merge: f64) -> Vec<Vec<Point3>> {
    let ahead = headings(stations);
    let mut rows: Vec<Vec<Point3>> = Vec::with_capacity(stations.len());
    for (j, st) in stations.iter().enumerate() {
        let n = st.sec.offsets.len() - 1;
        let row = (0..=n)
            .map(|k| {
                let p = st.q + st.sec.offsets[k];
                let rail = if k == 0 {
                    Some(0)
                } else {
                    (k == n).then_some(1)
                };
                if rail.is_none_or(|x| j > 0 && stations[j - 1].feet[x] == st.feet[x])
                    && let (Some(prev), Some(fwd)) = (rows.last(), ahead[j])
                    && (p - prev[k]).dot(fwd) <= merge
                {
                    return prev[k];
                }
                p
            })
            .collect();
        rows.push(row);
    }
    rows
}

/// Takes out the stations where the strip would fold over itself (a strip
/// triangle facing the other way from the strip's own, weighed by area),
/// and carries the rails across the facets between the stations either side
/// where they no longer share one. A few rounds; what remains is left to the
/// checks after.
pub(super) fn fold_free(
    sv: &Solver,
    surf: &Surface,
    mut stations: Vec<Station>,
    merge: f64,
) -> Result<Vec<Station>, PlanFail> {
    for _ in 0..FOLD_ROUNDS {
        let ns = stations.len();
        if ns < 4 {
            return Ok(stations);
        }
        let rows = held_rows(&stations, merge);
        let n = rows[0].len() - 1;
        let center = |s: &Station| s.sec.arc_center.map(|c| s.q + c);
        // Each strip triangle's facing (+1 toward the ball, −1 away) and
        // area, with the station pair it spans.
        let mut tris: Vec<(f64, f64, usize)> = Vec::new();
        for j in 0..ns {
            let jn = (j + 1) % ns;
            let Some(mid) = center(&stations[j])
                .zip(center(&stations[jn]))
                .map(|(a, b)| a + (b - a) * 0.5)
            else {
                continue;
            };
            for k in 0..n {
                let mut ring = vec![rows[j][k], rows[jn][k], rows[jn][k + 1], rows[j][k + 1]];
                ring.dedup_by(|a, b| a.approx_eq(*b, tol::POINT_MERGE));
                if ring.len() > 1 && ring[0].approx_eq(ring[ring.len() - 1], tol::POINT_MERGE) {
                    ring.pop();
                }
                if ring.len() < 3 {
                    continue;
                }
                let pieces: Vec<[Point3; 3]> = if ring.len() == 3 {
                    vec![[ring[0], ring[1], ring[2]]]
                } else {
                    vec![[ring[0], ring[1], ring[2]], [ring[0], ring[2], ring[3]]]
                };
                for [a, b, c] in pieces {
                    let nrm = (b - a).cross(c - a);
                    let area = nrm.length();
                    if area <= 0.0 {
                        continue;
                    }
                    let g = Point3::new(
                        (a.x + b.x + c.x) / 3.0,
                        (a.y + b.y + c.y) / 3.0,
                        (a.z + b.z + c.z) / 3.0,
                    );
                    tris.push((nrm.dot(mid - g).signum(), area, j));
                }
            }
        }
        let vote: f64 = tris.iter().map(|&(s, a, _)| s * a).sum();
        let bad: BTreeSet<usize> = tris
            .iter()
            .filter(|&&(s, _, _)| s * vote < 0.0)
            .flat_map(|&(_, _, j)| [j, (j + 1) % ns])
            .collect();
        if bad.is_empty() || bad.len() * 2 > ns {
            return Ok(stations);
        }
        let kept: Vec<Station> = stations
            .into_iter()
            .enumerate()
            .filter(|(j, _)| !bad.contains(j))
            .map(|(_, s)| s)
            .collect();
        stations = relinked(sv, surf, kept)?;
    }
    Ok(stations)
}

/// `stations` with the rails carried across the facets between each two in
/// a row (round the closed chain) whose feet on a side share none.
fn relinked(sv: &Solver, surf: &Surface, kept: Vec<Station>) -> Result<Vec<Station>, PlanFail> {
    let obj = sv.obj;
    let mut out: Vec<Station> = Vec::with_capacity(kept.len());
    let nk = kept.len();
    for j in 0..nk {
        let (a, b) = (&kept[j], &kept[(j + 1) % nk]);
        out.push(a.clone());
        let mut from = a.clone();
        for x in 0..2 {
            if super::share(obj, from.feet[x], b.feet[x]) {
                continue;
            }
            let steps = link(sv, surf, &from, b, x)?;
            if let Some(l) = steps.last() {
                from = l.clone();
            }
            out.extend(steps);
        }
    }
    Ok(out)
}

/// `stations` without those with a section point within `r` of a point of
/// `avoid` (where a strip could not be built), the rails carried across.
pub(super) fn avoiding(
    sv: &Solver,
    surf: &Surface,
    stations: Vec<Station>,
    avoid: &[Point3],
    r: f64,
) -> Result<Vec<Station>, PlanFail> {
    if avoid.is_empty() {
        return Ok(stations);
    }
    let near = |st: &Station| {
        st.sec
            .offsets
            .iter()
            .any(|&o| avoid.iter().any(|&p| (st.q + o - p).length() <= r))
    };
    let kept: Vec<Station> = stations.into_iter().filter(|st| !near(st)).collect();
    if kept.len() < 4 {
        return Err(unsupported(avoid[0]));
    }
    relinked(sv, surf, kept)
}

/// Where seam `e0`–`e1` crosses the plane through the ball's center `c`
/// and its two contacts `p1`, `p2` (the section of the hollow between them),
/// or its point nearest `p1`–`p2` where the plane is degenerate or misses.
fn slice_point(e0: Point3, e1: Point3, c: Point3, p1: Point3, p2: Point3) -> Point3 {
    if let Ok(n) = (p1 - c).cross(p2 - c).normalized() {
        let (d0, d1) = (n.dot(e0 - c), n.dot(e1 - c));
        if d0 != d1 && d0 * d1 <= 0.0 {
            return e0 + (e1 - e0) * (d0 / (d0 - d1));
        }
    }
    nearest_on_segment(e0, e1, p1, p2)
}

/// The point of segment `e0`–`e1` nearest the segment `p1`–`p2`.
fn nearest_on_segment(e0: Point3, e1: Point3, p1: Point3, p2: Point3) -> Point3 {
    let (d1, d2, w) = (e1 - e0, p2 - p1, e0 - p1);
    let (a, b, c, d, e) = (d1.dot(d1), d1.dot(d2), d2.dot(d2), d1.dot(w), d2.dot(w));
    let den = a * c - b * b;
    let mut s = if den > tol::NORMALIZE_MIN_LENGTH {
        ((b * e - c * d) / den).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // With s fixed, the nearest point of the other segment, then s again.
    let t = if c > 0.0 {
        ((b * s + e) / c).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if a > 0.0 {
        s = ((t * b - d) / a).clamp(0.0, 1.0);
    }
    e0 + d1 * s
}
