//! The unstamped swept blend (WP07 §4, decision I-24): a rolling ball,
//! or any other profile but a bullnose, along any chain of sharp edges
//! outside the analytic catalogue (a pipe tee, an oblique cut through a
//! cylinder, imported facets), closed or ending on a face (D-115).
//!
//! Each station rolls the ball between the planes of its two foot facets,
//! walking a foot across a seam when the ball would land outside it;
//! stations are inserted where a foot crosses a seam so every rail segment
//! lies in one facet. Across a ridge (a seam convex seen from the ball) the
//! ball rides the seam line; in a valley it rests against both facets at
//! the corner, and the rail steps through the seam point nearest its
//! center. A chamfer, cove or profile station traces each side across its
//! facets in the section plane instead and takes the catalogue's section
//! solver on the traces. Section planes turn continuously along the chain
//! (bisector planes at its vertices) so they sample the ball's path in
//! order; an open chain's end sections lie in its end faces, and the
//! planes near an end turn into them. The side faces are cut along the
//! rails with the sketch arrangement and everything between the rails and
//! the chain is removed by a flood fill that never crosses a rail; an end
//! face is cut along the end section (a convex chain) or grows by it (a
//! concave one); the strip's winding is the one opposite the kept pieces
//! along the rails.

use std::collections::{BTreeMap, BTreeSet};

mod geo;

use super::section::{Trace, solve_section};

thread_local! {
    /// Whether a chain the facet walk refuses takes the solve against the
    /// side surfaces (always, except where a spec turns it off to show the
    /// walk alone refuses).
    static SURFACE_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

thread_local! {
    /// Points where a swept chain's strip could not be built against the
    /// side surfaces in this attempt: the stations near them go.
    static AVOID: std::cell::RefCell<Vec<Point3>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// How many rounds [`plan_swept`] (and the blend's checks after it) take
/// out stations near where a strip could not be built.
pub(crate) const AVOID_ROUNDS: usize = 8;

thread_local! {
    /// How many rounds of taking out stations an attempt may take (fewer
    /// while a refusal searches for the size that rolls).
    static ROUNDS: std::cell::Cell<usize> = const { std::cell::Cell::new(AVOID_ROUNDS) };
}

/// The rounds an attempt may take now.
pub(crate) fn avoid_rounds() -> usize {
    ROUNDS.with(std::cell::Cell::get)
}

/// Runs `f` allowing `n` rounds per attempt.
pub(crate) fn with_avoid_rounds<T>(n: usize, f: impl FnOnce() -> T) -> T {
    let before = ROUNDS.with(|r| r.replace(n));
    let out = f();
    ROUNDS.with(|r| r.set(before));
    out
}

thread_local! {
    /// The surface-solve stations of this attempt's chains, by chain edges,
    /// radius and profile.
    #[allow(clippy::type_complexity)]
    static ROLLED: std::cell::RefCell<Vec<((Vec<EdgeId>, u64, BlendProfile), Vec<Station>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Forgets the points to avoid and the stations solved (a new attempt).
pub(crate) fn avoid_clear() {
    AVOID.with(|a| a.borrow_mut().clear());
    ROLLED.with(|c| c.borrow_mut().clear());
}

/// Adds a point to avoid.
pub(crate) fn avoid_near(p: Point3) {
    AVOID.with(|a| a.borrow_mut().push(p));
}

/// Runs `f` with the solve against the side surfaces turned off.
pub(crate) fn without_surface_fallback<T>(f: impl FnOnce() -> T) -> T {
    let before = SURFACE_FALLBACK.with(|s| s.replace(false));
    let out = f();
    SURFACE_FALLBACK.with(|s| s.set(before));
    out
}

use super::chain::Chain;
use super::plan::{Out, PlanFail, Pts};
use super::section::Section;
use super::{BlendError, BlendProfile, Convexity};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, PatchFace};
use crate::sketch::Sketch;
use crate::tol;
use crate::topo::{BlendId, BlendKind, BlendRole, BlendTag, Object};

/// Where a rail touches its side: inside a facet (the ball tangent to its
/// plane), on a seam (the ball touching the seam line where the side is
/// a convex ridge seen from the ball), or at a corner (the ball touching one
/// vertex, where a ridge bends under it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Foot {
    Face(FaceId),
    Seam(EdgeId),
    Corner(VertexId),
}

impl Foot {
    fn faces(self, obj: &Object) -> Vec<FaceId> {
        match self {
            Foot::Face(f) => vec![f],
            Foot::Seam(e) => obj
                .edge_faces(e)
                .map(|(a, b)| vec![a, b])
                .unwrap_or_default(),
            Foot::Corner(v) => around(obj, v).map(|(_, f)| f).unwrap_or_default(),
        }
    }
}

/// The edges and faces around vertex `v`, walked through its outgoing
/// half-edges; `None` at a mesh boundary.
fn around(obj: &Object, v: VertexId) -> Option<(Vec<EdgeId>, Vec<FaceId>)> {
    let hes = obj.half_edges();
    let start = obj.vertices().get(v)?.outgoing;
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    let mut h = start;
    for _ in 0..obj.edges().len() {
        edges.push(hes[h].edge);
        faces.push(obj.loops()[hes[h].loop_id].face);
        h = hes[hes[h].twin?].next;
        if h == start {
            return Some((edges, faces));
        }
    }
    None
}

/// Where an open chain ends (D-115): at each end, the face the chain runs
/// into and the normal of that end's section plane, along the chain.
struct Ends {
    faces: [FaceId; 2],
    normals: [Vec3; 2],
}

/// The ends of an open chain. At each end vertex the chain's two sides,
/// each a fan of facets joined by seams, meet one face across hard edges:
/// the end section lies in that face, which is cut along it. Anything else
/// there (another picked edge, a hard edge the chain could have gone on
/// along, a seam straight from one side to the other, several faces) is a
/// vertex the blend has no construction for.
fn open_ends(obj: &Object, chain: &Chain, picked: &BTreeSet<EdgeId>) -> Result<Ends, PlanFail> {
    let m = chain.edges.len();
    let mut found: Vec<(FaceId, Vec3)> = Vec::with_capacity(2);
    for end in 0..2 {
        let (v, info, d) = if end == 0 {
            (chain.verts[0], &chain.edges[0], chain.seg_dir(obj, 0))
        } else {
            (
                chain.verts[m],
                &chain.edges[m - 1],
                chain.seg_dir(obj, m - 1),
            )
        };
        let at = obj.vertices()[v].position;
        let unsupported = || PlanFail::Hard(BlendError::UnsupportedVertex { at });
        let (edges, faces) = around(obj, v).ok_or_else(unsupported)?;
        let n = edges.len();
        if edges.iter().any(|&e| e != info.edge && picked.contains(&e)) {
            return Err(unsupported());
        }
        // `edges[i]` lies between `faces[i]` and `faces[i + 1]`.
        let c = edges
            .iter()
            .position(|&e| e == info.edge)
            .ok_or_else(unsupported)?;
        let hard = |e: EdgeId| obj.edge_smoothness(e) == crate::surface::EdgeSmoothness::Hard;
        // The fan from the face at `start`, stepping around the vertex
        // across seams up to the first hard edge: the fan and that edge.
        let fan = |start: usize, step: isize| -> Option<(Vec<FaceId>, usize)> {
            let mut i = start % n;
            let mut fan = vec![faces[i]];
            for _ in 0..n {
                let e = if step > 0 { i } else { (i + n - 1) % n };
                if hard(edges[e]) {
                    return Some((fan, e));
                }
                i = (i as isize + step).rem_euclid(n as isize) as usize;
                fan.push(faces[i]);
            }
            None
        };
        let (fan_b, hb) = fan(c + 1, 1).ok_or_else(unsupported)?;
        let (fan_a, ha) = fan(c, -1).ok_or_else(unsupported)?;
        if ha == c || hb == c || ha == hb {
            return Err(unsupported());
        }
        let rest: Vec<FaceId> = faces
            .iter()
            .copied()
            .filter(|f| !fan_a.contains(f) && !fan_b.contains(f))
            .collect();
        let [end_face] = rest[..] else {
            return Err(unsupported());
        };
        let in_fans = |f: FaceId| fan_a.contains(&f) || fan_b.contains(&f);
        if !in_fans(info.a)
            || !in_fans(info.b)
            || fan_a.contains(&info.a) == fan_a.contains(&info.b)
            || chain
                .edges
                .iter()
                .any(|e| e.a == end_face || e.b == end_face)
        {
            return Err(unsupported());
        }
        let normal = obj.faces()[end_face].plane.normal();
        let along = normal.dot(d);
        if along.abs() <= tol::NORMAL_DIRECTION {
            return Err(unsupported());
        }
        found.push((end_face, if along > 0.0 { normal } else { -normal }));
    }
    Ok(Ends {
        faces: [found[0].0, found[1].0],
        normals: [found[0].1, found[1].1],
    })
}

/// One side traced across its facets in a section plane: the trace, the
/// facet of each segment, and whether the side ended before the profile's
/// reach.
struct Traced {
    trace: Trace,
    faces: Vec<FaceId>,
    short: bool,
}

/// One swept station: its chain point, its section, and its two feet.
#[derive(Debug, Clone)]
struct Station {
    q: Point3,
    sec: Section,
    feet: [Foot; 2],
}

/// The inputs every station solve shares.
struct Solver<'a> {
    obj: &'a Object,
    profile: &'a BlendProfile,
    /// +1 over a convex chain (the ball in the material), −1 concave.
    sigma: f64,
    /// Faces that are the other side of some chain edge, per side.
    other_side: [BTreeSet<FaceId>; 2],
    /// Solve against the side surfaces themselves (the fallback when the
    /// facet walk refuses a chain), not by walking from facet to facet.
    geo: Option<geo::Surface>,
    /// A face the ball's rails may land past while this is set: an open
    /// chain's end face, for its end station (the ball in the end plane
    /// has its tangent points a hair past the face; they are carried back
    /// into the plane along the chain afterwards).
    extend_past: std::cell::Cell<[Option<FaceId>; 2]>,
}

fn unsupported(at: Point3) -> PlanFail {
    PlanFail::Hard(BlendError::UnsupportedVertex { at })
}

/// A face's loops projected to 2D in its plane.
fn face_rings(obj: &Object, f: FaceId) -> (Plane, Vec<Vec<[f64; 2]>>) {
    let plane = obj.faces()[f].plane;
    let (u, v) = crate::triangulate::plane_basis(plane.normal());
    let to2 = |p: Point3| [p.to_vec().dot(u), p.to_vec().dot(v)];
    let rings = obj
        .face_loops(f)
        .into_iter()
        .map(|l| obj.loop_positions(l).map(to2).collect())
        .collect();
    (plane, rings)
}

fn to2(plane: &Plane, p: Point3) -> [f64; 2] {
    let (u, v) = crate::triangulate::plane_basis(plane.normal());
    [p.to_vec().dot(u), p.to_vec().dot(v)]
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 <= 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - ab[0] * t).hypot(p[1] - a[1] - ab[1] * t)
}

/// Whether `p` (in `f`'s plane) lies in face `f`, its boundary counting.
fn in_face(obj: &Object, f: FaceId, p: Point3) -> bool {
    let (plane, rings) = face_rings(obj, f);
    in_rings(&plane, &rings, p)
}

/// Whether `p` lies in the polygon `rings` (outer first, then holes) drawn
/// in `plane`, its boundary counting.
fn in_rings(plane: &Plane, rings: &[Vec<[f64; 2]>], p: Point3) -> bool {
    let q = to2(plane, p);
    let on_boundary = rings
        .iter()
        .any(|r| (0..r.len()).any(|i| seg_dist(q, r[i], r[(i + 1) % r.len()]) <= tol::PLANE_DIST));
    if on_boundary {
        return true;
    }
    let mut inside = false;
    for r in rings {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if (a[1] > q[1]) != (b[1] > q[1]) {
                let x = a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if q[0] < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// The boundary edge of `f` nearest `p` (in `f`'s plane, outside `f`) among
/// those it lies outside of: the seam a foot crosses.
fn beyond_edge(obj: &Object, f: FaceId, p: Point3) -> Option<EdgeId> {
    let face = &obj.faces()[f];
    let (u, v) = crate::triangulate::plane_basis(face.plane.normal());
    let to2 = |x: Point3| [x.to_vec().dot(u), x.to_vec().dot(v)];
    let q = to2(p);
    let mut best: Option<(f64, EdgeId)> = None;
    for l in obj.face_loops(f) {
        for h in obj.loop_half_edges(l) {
            let he = obj.half_edges()[h];
            let a = to2(obj.vertices()[he.origin].position);
            let b = to2(obj.vertices()[obj.half_edges()[he.next].origin].position);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            if len <= tol::POINT_MERGE {
                continue;
            }
            // Loops run CCW about the normal (holes CW): outward is right.
            // Of the edges `p` lies outside of, the nearest (non-convex
            // facets: a farther edge's line can pass nearer).
            let side = (q[0] - a[0]) * dy - (q[1] - a[1]) * dx;
            let dist = seg_dist(q, a, b);
            if side > 0.0 && best.is_none_or(|(bd, _)| dist < bd) {
                best = Some((dist, he.edge));
            }
        }
    }
    best.map(|(_, e)| e)
}

impl Solver<'_> {
    /// Whether seam `e` (between `f` and the face across it) is a valley
    /// seen from the ball: the face across bends toward the ball's side of
    /// `f`'s plane, so the ball can rest against both facets at once.
    fn is_valley(&self, e: EdgeId, f: FaceId) -> bool {
        let Some(fr) = self.obj.edge_frame(e, f) else {
            return false;
        };
        let nf = self.obj.faces()[f].plane.normal();
        fr.t_b.dot(nf * (-self.sigma)) > 0.0
    }

    /// A facet across one of `f`'s valley seams (on this side, not the
    /// other side's) that a ball of the profile's radius about `c` dips
    /// into.
    fn penetrated_valley(
        &self,
        f: FaceId,
        c: Point3,
        other: &[FaceId],
        x: usize,
    ) -> Option<FaceId> {
        let BlendProfile::Round { radius: r, .. } = *self.profile else {
            return None;
        };
        let obj = self.obj;
        obj.face_loops(f).into_iter().find_map(|l| {
            obj.loop_half_edges(l).find_map(|h| {
                let e = obj.half_edges()[h].edge;
                let (a, b) = obj.edge_faces(e)?;
                let g = if a == f { b } else { a };
                if other.contains(&g) || self.other_side[x].contains(&g) || !self.is_valley(e, f) {
                    return None;
                }
                let clear = -self.sigma * obj.faces()[g].plane.signed_distance(c) - r;
                // Only a ball near this seam: its foot on g's plane must fall
                // within reach of the seam segment.
                let (p0, p1) = obj.edge_endpoints(e)?;
                let near = super::query::point_segment_distance(c, p0, p1) <= 2.0 * r;
                (clear < -tol::PLANE_DIST && near).then_some(g)
            })
        })
    }

    /// Of the two facets of a valley, the one the ball rests on at this
    /// station: the one whose section keeps the center at least `r` from
    /// the other facet's plane (both at the corner itself).
    fn valley_pick(
        &self,
        q: Point3,
        t: Vec3,
        feet: [Foot; 2],
        x: usize,
        pair: (FaceId, FaceId),
    ) -> Result<FaceId, PlanFail> {
        let BlendProfile::Round { radius: r, .. } = *self.profile else {
            return Err(unsupported(q));
        };
        let (f, g) = pair;
        let mut best: Option<(f64, FaceId)> = None;
        for (cand, other) in [(f, g), (g, f)] {
            let mut ft = feet;
            ft[x] = Foot::Face(cand);
            let Ok(sec) = self.section(q, t, ft) else {
                continue;
            };
            let Some(c) = sec.arc_center.map(|c| q + c) else {
                continue;
            };
            let clear = -self.sigma * self.obj.faces()[other].plane.signed_distance(c) - r;
            if best.is_none_or(|(b, _)| clear > b) {
                best = Some((clear, cand));
            }
        }
        best.filter(|&(clear, _)| clear >= -tol::PLANE_DIST)
            .map(|(_, f)| f)
            .ok_or_else(|| unsupported(q))
    }

    /// The section at chain point `q` (direction `t`) for `feet`: the ball
    /// center in the section plane, 3D distance `r` from each foot's plane
    /// (or from a seam foot's line), the rails its feet.
    fn section(&self, q: Point3, t: Vec3, feet: [Foot; 2]) -> Result<Section, PlanFail> {
        let obj = self.obj;
        let plane_of = |f: Foot| match f {
            Foot::Face(g) => Some(obj.faces()[g].plane),
            Foot::Seam(_) | Foot::Corner(_) => None,
        };
        let (u, v) = crate::triangulate::plane_basis(t);
        if let BlendProfile::Round {
            radius: r,
            segments,
        } = *self.profile
            && feet.iter().any(|f| !matches!(f, Foot::Face(_)))
        {
            return self.ridge_section(q, t, feet, r, segments as usize);
        }
        let (Some(pa), Some(pb)) = (plane_of(feet[0]), plane_of(feet[1])) else {
            return Err(unsupported(q));
        };
        let (na, nb) = (pa.normal(), pb.normal());
        // Between two planes the ball centers lie on one line (parallel to
        // the planes' line of intersection); the station's center is where
        // that line meets the section plane, so stations sample the center
        // path continuously as the section plane turns at a chain vertex.
        // The arc itself lies square to that line, through the center.
        match *self.profile {
            BlendProfile::Round {
                radius: r,
                segments,
            } => {
                let n = segments as usize;
                // n·(q + x u + y v) = n·p − σ r for both planes.
                let rhs = |nn: Vec3, pl: &Plane| nn.dot(pl.point() - q) - self.sigma * r;
                let (a11, a12, a21, a22) = (na.dot(u), na.dot(v), nb.dot(u), nb.dot(v));
                let det = a11 * a22 - a12 * a21;
                if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
                    return Err(unsupported(q));
                }
                let (b1, b2) = (rhs(na, &pa), rhs(nb, &pb));
                let x = (b1 * a22 - b2 * a12) / det;
                let y = (a11 * b2 - a21 * b1) / det;
                let c = q + u * x + v * y;
                let fa = c + na * (self.sigma * r);
                let fb = c + nb * (self.sigma * r);
                let (u0, u1) = ((fa - c) / r, (fb - c) / r);
                let cosphi = u0.dot(u1).clamp(-1.0, 1.0);
                let phi = cosphi.acos();
                let w = (u1 - u0 * cosphi)
                    .normalized()
                    .map_err(|_| unsupported(q))?;
                let mut offsets = Vec::with_capacity(n + 1);
                offsets.push(fa - q);
                for k in 1..n {
                    let a = phi * k as f64 / n as f64;
                    offsets.push(c + (u0 * a.cos() + w * a.sin()) * r - q);
                }
                offsets.push(fb - q);
                Ok(Section {
                    offsets,
                    arc_center: Some(c - q),
                    radius: Some(r),
                    seg_a: 0,
                    seg_b: 0,
                    facet_arcs: vec![None; n],
                    soft: (0..=n).map(|k| k > 0 && k < n).collect(),
                    round: true,
                })
            }
            _ => Err(PlanFail::Hard(BlendError::UnsupportedEdge)),
        }
    }

    /// The section of a round with a foot on a seam: the center in the
    /// section plane at distance `r` from each foot's plane or seam line, on
    /// the outside of both facets at a seam, by Newton's method from the
    /// solution against the seams' support planes.
    fn ridge_section(
        &self,
        q: Point3,
        t: Vec3,
        feet: [Foot; 2],
        r: f64,
        n: usize,
    ) -> Result<Section, PlanFail> {
        let obj = self.obj;
        let sigma = self.sigma;
        enum Contact {
            Plane(Plane),
            Line(Point3, Vec3, [FaceId; 2]),
            Point(Point3, Vec<FaceId>),
        }
        let contact = |f: Foot| -> Result<Contact, PlanFail> {
            Ok(match f {
                Foot::Face(g) => Contact::Plane(obj.faces()[g].plane),
                Foot::Seam(e) => {
                    let (l0, l1) = obj.edge_endpoints(e).ok_or_else(|| unsupported(q))?;
                    let ld = (l1 - l0).normalized().map_err(|_| unsupported(q))?;
                    let (a, b) = obj.edge_faces(e).ok_or_else(|| unsupported(q))?;
                    Contact::Line(l0, ld, [a, b])
                }
                Foot::Corner(v) => {
                    let (_, faces) = around(obj, v).ok_or_else(|| unsupported(q))?;
                    Contact::Point(obj.vertices()[v].position, faces)
                }
            })
        };
        let cs = [contact(feet[0])?, contact(feet[1])?];
        // Residual and gradient of "distance to the contact = r".
        let eval = |k: &Contact, c: Point3| -> (f64, Vec3) {
            match k {
                Contact::Plane(p) => (-sigma * p.signed_distance(c) - r, p.normal() * (-sigma)),
                Contact::Line(l0, ld, _) => {
                    let w = c - *l0;
                    let d = w - *ld * w.dot(*ld);
                    let l = d.length().max(tol::NORMALIZE_MIN_LENGTH);
                    (l - r, d / l)
                }
                Contact::Point(p, _) => {
                    let d = c - *p;
                    let l = d.length().max(tol::NORMALIZE_MIN_LENGTH);
                    (l - r, d / l)
                }
            }
        };
        let (u, v) = crate::triangulate::plane_basis(t);
        // Start: support planes (a seam's facets' mean normal through it).
        let support = |k: &Contact| -> Result<Plane, PlanFail> {
            match k {
                Contact::Plane(p) => Ok(*p),
                Contact::Line(l0, _, [a, b]) => {
                    let nn = obj.faces()[*a].plane.normal() + obj.faces()[*b].plane.normal();
                    Plane::from_point_normal(*l0, nn).map_err(|_| unsupported(q))
                }
                Contact::Point(p, faces) => {
                    let nn = faces.iter().fold(Vec3::new(0.0, 0.0, 0.0), |a, &f| {
                        a + obj.faces()[f].plane.normal()
                    });
                    Plane::from_point_normal(*p, nn).map_err(|_| unsupported(q))
                }
            }
        };
        let (pa, pb) = (support(&cs[0])?, support(&cs[1])?);
        let (na, nb) = (pa.normal(), pb.normal());
        let rhs = |nn: Vec3, pl: &Plane| nn.dot(pl.point() - q) - sigma * r;
        let (a11, a12, a21, a22) = (na.dot(u), na.dot(v), nb.dot(u), nb.dot(v));
        let det = a11 * a22 - a12 * a21;
        if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
            return Err(unsupported(q));
        }
        let (b1, b2) = (rhs(na, &pa), rhs(nb, &pb));
        let (mut x, mut y) = ((b1 * a22 - b2 * a12) / det, (a11 * b2 - a21 * b1) / det);
        let mut c = q + u * x + v * y;
        for _ in 0..50 {
            let (r0, g0) = eval(&cs[0], c);
            let (r1, g1) = eval(&cs[1], c);
            if r0.abs().max(r1.abs()) <= tol::BLEND_CORNER_RESIDUAL {
                break;
            }
            let (j11, j12, j21, j22) = (g0.dot(u), g0.dot(v), g1.dot(u), g1.dot(v));
            let dj = j11 * j22 - j12 * j21;
            if dj.abs() <= tol::NORMALIZE_MIN_LENGTH {
                return Err(unsupported(q));
            }
            x -= (r0 * j22 - r1 * j12) / dj;
            y -= (j11 * r1 - j21 * r0) / dj;
            c = q + u * x + v * y;
        }
        let mut rails = [c, c];
        for (k, kc) in cs.iter().enumerate() {
            let (res, _) = eval(kc, c);
            if res.abs() > tol::PLANE_DIST {
                return Err(unsupported(q));
            }
            rails[k] = match kc {
                Contact::Plane(p) => c + p.normal() * (sigma * r),
                Contact::Line(l0, ld, faces) => {
                    // A ridge contact: the center lies outside both facets.
                    if faces.iter().any(|&f| {
                        -sigma * obj.faces()[f].plane.signed_distance(c) < -tol::PLANE_DIST
                    }) {
                        return Err(unsupported(q));
                    }
                    *l0 + *ld * (c - *l0).dot(*ld)
                }
                Contact::Point(p, faces) => {
                    // A corner contact: the center lies outside every facet
                    // around it.
                    if faces.iter().any(|&f| {
                        -sigma * obj.faces()[f].plane.signed_distance(c) < -tol::PLANE_DIST
                    }) {
                        return Err(unsupported(q));
                    }
                    *p
                }
            };
        }
        arc_section(q, c, r, rails[0], rails[1], n)
    }

    /// WP07 §4.1 with ridges: rolls the ball at `q`, walking each foot
    /// across seams until both rails land on their feet; a foot that would
    /// bounce between two facets rides their seam instead. `pinned` fixes
    /// one side's foot.
    fn solve(
        &self,
        q: Point3,
        t: Vec3,
        start: [Foot; 2],
        pinned: [Option<Foot>; 2],
    ) -> Result<(Section, [Foot; 2]), PlanFail> {
        if let Some(surf) = &self.geo {
            return geo::solve(self, surf, q, t, start, pinned);
        }
        if !matches!(self.profile, BlendProfile::Round { .. }) {
            return self.solve_traced(q, t, start, pinned);
        }
        let obj = self.obj;
        let mut feet = start;
        for x in 0..2 {
            if let Some(f) = pinned[x] {
                feet[x] = f;
            }
        }
        let mut came_from: [Option<FaceId>; 2] = [None, None];
        let mut seen: [BTreeSet<Foot>; 2] = [BTreeSet::from([feet[0]]), BTreeSet::from([feet[1]])];
        // A side resting in a valley: its two facets, re-picked each round.
        let mut valley: [Option<(FaceId, FaceId)>; 2] = [None, None];
        for _ in 0..32 {
            for x in 0..2 {
                if let Some(pair) = valley[x] {
                    feet[x] = Foot::Face(self.valley_pick(q, t, feet, x, pair)?);
                }
            }
            let sec = self.section(q, t, feet)?;
            let rails = [q + sec.offsets[0], q + *sec.offsets.last().expect("rails")];
            let mut settled = true;
            for x in 0..2 {
                if pinned[x].is_some() {
                    continue;
                }
                let f = match feet[x] {
                    Foot::Seam(e) => {
                        // The rail must stay on the seam segment.
                        let (a, b) = obj.edge_endpoints(e).ok_or_else(|| unsupported(q))?;
                        if super::query::point_segment_distance(rails[x], a, b)
                            > tol::SWEPT_PLANE_SLACK
                        {
                            return Err(unsupported(q));
                        }
                        continue;
                    }
                    // Only the solve against the surfaces rests on corners.
                    Foot::Corner(_) => return Err(unsupported(q)),
                    Foot::Face(f) => f,
                };
                if in_face(obj, f, rails[x]) {
                    // Resting on `f`, the ball must still clear the facets
                    // across `f`'s valley seams; one it dips into is a
                    // corner to resolve between the two (D-39).
                    if valley[x].is_none()
                        && let Some(c) = sec.arc_center.map(|c| q + c)
                        && let Some(g) = self.penetrated_valley(f, c, &feet[1 - x].faces(obj), x)
                    {
                        valley[x] = Some((f, g));
                        settled = false;
                    }
                    continue;
                }
                let e = beyond_edge(obj, f, rails[x]).ok_or_else(|| unsupported(q))?;
                let (g1, g2) = obj.edge_faces(e).ok_or_else(|| unsupported(q))?;
                let next = if g1 == f { g2 } else { g1 };
                if self.extend_past.get().contains(&Some(next)) {
                    // Past the end face: the facet counts as going on.
                    continue;
                }
                if let Some((p0, p1)) = valley[x] {
                    if (f == p0 && next == p1) || (f == p1 && next == p0) {
                        // Just past the valley's own seam: the corner.
                        continue;
                    }
                    valley[x] = None;
                }
                settled = false;
                let other = feet[1 - x].faces(obj);
                if self.is_valley(e, f) {
                    // Across a valley the ball rests on whichever facet
                    // keeps it clear of the other (D-39).
                    if other.contains(&next) || self.other_side[x].contains(&next) {
                        return Err(unsupported(q));
                    }
                    valley[x] = Some((f, next));
                    continue;
                }
                let next_foot = if came_from[x] == Some(next) {
                    // Bouncing between two facets: a convex ridge.
                    Foot::Seam(e)
                } else {
                    if other.contains(&next) || self.other_side[x].contains(&next) {
                        return Err(unsupported(q));
                    }
                    came_from[x] = Some(f);
                    Foot::Face(next)
                };
                if !seen[x].insert(next_foot) {
                    return Err(unsupported(q));
                }
                feet[x] = next_foot;
            }
            if settled {
                return Ok((sec, feet));
            }
        }
        Err(unsupported(q))
    }
}

impl Solver<'_> {
    /// Whether the stations are a rolling ball's (the facet walk's round
    /// solve); a chamfer, cove or profile station is traced instead.
    fn rolls(&self) -> bool {
        matches!(self.profile, BlendProfile::Round { .. })
    }

    /// How far the profile reaches along side `x`: the length of side the
    /// trace has to lay before the section can be solved.
    fn reach(&self, x: usize) -> f64 {
        match *self.profile {
            BlendProfile::Round { radius, .. } | BlendProfile::Cove { radius, .. } => radius,
            BlendProfile::Chamfer {
                setback_a,
                setback_b,
            } => {
                if x == 0 {
                    setback_a
                } else {
                    setback_b
                }
            }
            BlendProfile::Custom {
                ref shape, width, ..
            } => {
                if x == 0 {
                    width
                } else {
                    width * shape.b
                }
            }
            BlendProfile::Bullnose { .. } => f64::INFINITY,
        }
    }

    /// Where a ray from `p` along `d` in facet `f`'s plane leaves the
    /// facet: the distance and the boundary edge it crosses, or `None`
    /// when it never enters it (`p` is on the boundary and `d` points out).
    fn exit_along(&self, f: FaceId, p: Point3, d: Vec3) -> Option<(f64, EdgeId)> {
        let obj = self.obj;
        let face = &obj.faces()[f];
        // Only a facet the point lies on: projected onto another facet's
        // plane the point can land inside its polygon all the same.
        if face.plane.signed_distance(p).abs() > tol::SWEPT_PLANE_SLACK {
            return None;
        }
        let (u, v) = crate::triangulate::plane_basis(face.plane.normal());
        let to2 = |x: Point3| [x.to_vec().dot(u), x.to_vec().dot(v)];
        let (q, dd) = (to2(p), [d.dot(u), d.dot(v)]);
        let mut best: Option<(f64, EdgeId)> = None;
        for l in obj.face_loops(f) {
            for h in obj.loop_half_edges(l) {
                let he = obj.half_edges()[h];
                let a = to2(obj.vertices()[he.origin].position);
                let b = to2(obj.vertices()[obj.half_edges()[he.next].origin].position);
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let den = dd[0] * ey - dd[1] * ex;
                if den.abs() <= tol::NORMALIZE_MIN_LENGTH {
                    continue;
                }
                let (wx, wy) = (a[0] - q[0], a[1] - q[1]);
                let s = (wx * ey - wy * ex) / den;
                let r = (wx * dd[1] - wy * dd[0]) / den;
                if s > tol::POINT_MERGE
                    && (-tol::POINT_MERGE..=1.0 + tol::POINT_MERGE).contains(&r)
                    && best.is_none_or(|(bs, _)| s < bs)
                {
                    best = Some((s, he.edge));
                }
            }
        }
        // A crossing counts only when the ray runs through the facet to it.
        best.filter(|&(s, _)| in_face(obj, f, p + d * (s * 0.5)))
    }

    /// Side `x` at station `q` as a trace in the section plane (normal
    /// `t`): from the chain point along facet `start`, crossing each seam
    /// into the next facet (the setback is measured over the side), until
    /// the profile's reach is laid or the side ends at the other side's
    /// faces. Returns the trace, the facet of each of its segments, and
    /// whether the side ended before the reach (the section solver then
    /// refuses the size: every segment ends at its facet's edge).
    fn side_trace(&self, q: Point3, t: Vec3, x: usize, start: FaceId) -> Result<Traced, PlanFail> {
        let obj = self.obj;
        let need = self.reach(x);
        // A round's contact sits r / tan(θ/2) along a side, beyond r in a
        // corner sharper than a right angle: its trace is laid farther,
        // and the ball settles where it settles.
        let want = match *self.profile {
            BlendProfile::Round { radius, .. } => 4.0 * radius,
            _ => need,
        };
        let (mut dirs, mut lens, mut faces) = (Vec::new(), Vec::new(), Vec::new());
        let (mut face, mut p, mut prev): (FaceId, Point3, Option<Vec3>) = (start, q, None);
        let mut laid = 0.0;
        // The way on through facet `f` from `p`: of the two ways along the
        // plane's line in the section plane, the one that runs through the
        // facet; when both do (a corner of a bent facet), the one that
        // carries on.
        let way_in = |f: FaceId, p: Point3, prev: Option<Vec3>| -> Option<(Vec3, f64, EdgeId)> {
            let n = obj.faces()[f].plane.normal();
            let d0 = n.cross(t).normalized().ok()?;
            [d0, -d0]
                .iter()
                .filter_map(|&d| self.exit_along(f, p, d).map(|(s, e)| (d, s, e)))
                .max_by(|a, b| {
                    let score = |d: Vec3| prev.map_or(0.0, |pd| d.dot(pd));
                    score(a.0).total_cmp(&score(b.0))
                })
        };
        for _ in 0..obj.faces().len().min(512) {
            let mut pick = way_in(face, p, prev);
            if pick.is_none() && !faces.is_empty() {
                // The trace came in through a corner of this facet and the
                // way on lies in another facet of the fan around it.
                let corner = obj.face_loops(face).into_iter().find_map(|l| {
                    obj.loop_half_edges(l)
                        .map(|h| obj.half_edges()[h].origin)
                        .find(|&v| obj.vertices()[v].position.approx_eq(p, tol::SWEPT_SLACK))
                });
                if let Some((_, fan)) = corner.and_then(|v| around(obj, v)) {
                    for g in fan {
                        if g == face || faces.contains(&g) || self.other_side[x].contains(&g) {
                            continue;
                        }
                        if let Some(pk) = way_in(g, p, prev) {
                            face = g;
                            pick = Some(pk);
                            break;
                        }
                    }
                }
            }
            let Some((d, len, exit)) = pick else {
                if laid >= need - tol::POINT_MERGE {
                    break;
                }
                return Err(unsupported(q));
            };
            dirs.push(d);
            lens.push(len);
            faces.push(face);
            laid += len;
            if laid >= want - tol::POINT_MERGE {
                break;
            }
            let Some((g1, g2)) = obj.edge_faces(exit) else {
                break;
            };
            let next = if g1 == face { g2 } else { g1 };
            if self.other_side[x].contains(&next) || faces.contains(&next) {
                break;
            }
            p = p + d * len;
            prev = Some(d);
            face = next;
        }
        if dirs.is_empty() {
            return Err(unsupported(q));
        }
        let short = laid < need - tol::POINT_MERGE;
        Ok(Traced {
            trace: Trace { dirs, lens },
            faces,
            short,
        })
    }

    /// A station of a chamfer, cove or profile: each side traced across its
    /// facets, the section from the catalogue's solver, the feet where its
    /// rails land. A pinned seam holds when the rail lands on it.
    fn solve_traced(
        &self,
        q: Point3,
        t: Vec3,
        start: [Foot; 2],
        pinned: [Option<Foot>; 2],
    ) -> Result<(Section, [Foot; 2]), PlanFail> {
        let obj = self.obj;
        let mut traces: Vec<Traced> = Vec::with_capacity(2);
        for x in 0..2 {
            let other: Vec<FaceId> = pinned[1 - x].unwrap_or(start[1 - x]).faces(obj);
            let usable = |f: FaceId, seen: &[FaceId]| {
                !seen.contains(&f) && !self.other_side[x].contains(&f) && !other.contains(&f)
            };
            // The trace starts in a facet that holds the chain point. The
            // pinned foot's facets and the starting foot's first (a pin is
            // where the rail lands, which may be facets away from the
            // point); failing those, the facets across their edges (the
            // point may sit on a neighbor's boundary); then every facet
            // around a vertex at the point (a corner the section plane
            // leaves at once); last, the facets within a few seams of them
            // that hold the point.
            let mut own: Vec<FaceId> = Vec::new();
            for foot in pinned[x].into_iter().chain(std::iter::once(start[x])) {
                for f in foot.faces(obj) {
                    if !own.contains(&f) {
                        own.push(f);
                    }
                }
            }
            let mut across: Vec<FaceId> = Vec::new();
            for &f in &own {
                for l in obj.face_loops(f) {
                    for h in obj.loop_half_edges(l) {
                        if let Some(t2) = obj.half_edges()[h].twin {
                            let g = obj.loops()[obj.half_edges()[t2].loop_id].face;
                            if usable(g, &own) && !across.contains(&g) {
                                across.push(g);
                            }
                        }
                    }
                }
            }
            let mut fan: Vec<FaceId> = Vec::new();
            let at_vertex = own.iter().chain(&across).find_map(|&f| {
                obj.face_loops(f).into_iter().find_map(|l| {
                    obj.loop_half_edges(l)
                        .map(|h| obj.half_edges()[h].origin)
                        .find(|&v| obj.vertices()[v].position.approx_eq(q, tol::POINT_MERGE))
                })
            });
            if let Some((_, around_faces)) = at_vertex.and_then(|v| around(obj, v)) {
                for f in around_faces {
                    if usable(f, &own) && !across.contains(&f) && !fan.contains(&f) {
                        fan.push(f);
                    }
                }
            }
            let mut near: Vec<FaceId> = Vec::new();
            let mut ring: Vec<FaceId> = own.iter().chain(&across).chain(&fan).copied().collect();
            for _ in 0..4 {
                let mut next_ring: Vec<FaceId> = Vec::new();
                for &f in &ring {
                    for l in obj.face_loops(f) {
                        for h in obj.loop_half_edges(l) {
                            let Some(t2) = obj.half_edges()[h].twin else {
                                continue;
                            };
                            let g = obj.loops()[obj.half_edges()[t2].loop_id].face;
                            if usable(g, &own)
                                && !across.contains(&g)
                                && !fan.contains(&g)
                                && !near.contains(&g)
                                && !next_ring.contains(&g)
                            {
                                next_ring.push(g);
                            }
                        }
                    }
                }
                near.extend(next_ring.iter().copied());
                ring = next_ring;
            }
            near.retain(|&g| {
                obj.faces()[g].plane.signed_distance(q).abs() <= tol::SWEPT_PLANE_SLACK
                    && in_face(obj, g, q)
            });
            let traced = [own, across, fan, near]
                .iter()
                .find_map(|tier| tier.iter().find_map(|&f| self.side_trace(q, t, x, f).ok()))
                .ok_or_else(|| unsupported(q))?;
            traces.push(traced);
        }
        // A side that ended before the reach limits the size by itself (it
        // ran into the other side's faces or a facet with no way on); a
        // section the solver cannot fit on sides long enough is the facets'.
        let short = traces[0].short || traces[1].short;
        let size = |_| {
            PlanFail::Size(if short {
                super::LimitFeature::AdjacentFace
            } else {
                super::LimitFeature::FacetWidth
            })
        };
        let (ta, tb) = (&traces[0].trace, &traces[1].trace);
        let sec = solve_section(self.profile, ta, tb).map_err(size)?;
        let n = sec.offsets.len() - 1;
        let mut feet = [
            Foot::Face(traces[0].faces[sec.seg_a.min(traces[0].faces.len() - 1)]),
            Foot::Face(traces[1].faces[sec.seg_b.min(traces[1].faces.len() - 1)]),
        ];
        // A rail on a seam of its facet rides that seam (as the ball does
        // on a ridge): the foot is the seam, so the stations either side of
        // it share it and no change is read into the pass.
        for (x, foot) in feet.iter_mut().enumerate() {
            let rail = q + sec.offsets[if x == 0 { 0 } else { n }];
            let Foot::Face(f) = *foot else {
                continue;
            };
            let on_seam = obj.face_loops(f).into_iter().find_map(|l| {
                obj.loop_half_edges(l).find_map(|h| {
                    let e = obj.half_edges()[h].edge;
                    let (a, b) = obj.edge_endpoints(e)?;
                    (obj.edge_faces(e).is_some()
                        && super::query::point_segment_distance(rail, a, b) <= tol::SWEPT_SLACK
                        && !self.other_side[x]
                            .iter()
                            .any(|&g| obj.edge_faces(e).is_some_and(|(p0, p1)| p0 == g || p1 == g)))
                    .then_some(e)
                })
            });
            if let Some(e) = on_seam {
                *foot = Foot::Seam(e);
            }
        }
        for x in 0..2 {
            let Some(Foot::Seam(e)) = pinned[x] else {
                continue;
            };
            let rail = q + sec.offsets[if x == 0 { 0 } else { n }];
            let (a, b) = obj.edge_endpoints(e).ok_or_else(|| unsupported(q))?;
            // The bisection that found this seam stops where the rail is
            // just past the on-seam slack, so the pin takes twice it.
            if super::query::point_segment_distance(rail, a, b) > 2.0 * tol::SWEPT_SLACK {
                return Err(unsupported(q));
            }
            feet[x] = Foot::Seam(e);
        }
        Ok((sec, feet))
    }
}

/// A round section between rail points `fa` and `fb` on the ball about `c`.
fn arc_section(
    q: Point3,
    c: Point3,
    r: f64,
    fa: Point3,
    fb: Point3,
    n: usize,
) -> Result<Section, PlanFail> {
    let (u0, u1) = ((fa - c) / r, (fb - c) / r);
    let cosphi = u0.dot(u1).clamp(-1.0, 1.0);
    let phi = cosphi.acos();
    let w = (u1 - u0 * cosphi)
        .normalized()
        .map_err(|_| unsupported(q))?;
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(fa - q);
    for k in 1..n {
        let a = phi * k as f64 / n as f64;
        offsets.push(c + (u0 * a.cos() + w * a.sin()) * r - q);
    }
    offsets.push(fb - q);
    Ok(Section {
        offsets,
        arc_center: Some(c - q),
        radius: Some(r),
        seg_a: 0,
        seg_b: 0,
        facet_arcs: vec![None; n],
        soft: (0..=n).map(|k| k > 0 && k < n).collect(),
        round: true,
    })
}

/// A station solver along one chain edge: parameter, starting feet, pins.
type StationAt<'a> =
    dyn Fn(f64, Option<[Foot; 2]>, [Option<Foot>; 2]) -> Result<Station, PlanFail> + 'a;

/// Side `x`'s rail point of a station.
fn rail_point(st: &Station, x: usize) -> Point3 {
    let k = if x == 0 { 0 } else { st.sec.offsets.len() - 1 };
    st.q + st.sec.offsets[k]
}

/// Whether two feet share a facet.
fn share(obj: &Object, a: Foot, b: Foot) -> bool {
    let fb = b.faces(obj);
    a.faces(obj).iter().any(|f| fb.contains(f))
}

/// Inserts, in order, the seam stations strictly between `sa` (at `la`) and
/// `sb` (at `lb`) that make consecutive feet share a facet on both sides.
#[allow(clippy::too_many_arguments)]
fn refine(
    sv: &Solver,
    at: &StationAt,
    la: f64,
    sa: &Station,
    lb: f64,
    sb: &Station,
    out: &mut Vec<Station>,
    depth: usize,
) -> Result<(), PlanFail> {
    let obj = sv.obj;
    let Some(x) = (0..2).find(|&x| !share(obj, sa.feet[x], sb.feet[x])) else {
        return Ok(());
    };
    if depth > 64 {
        return Err(unsupported(sa.q));
    }
    if let Some(surf) = &sv.geo {
        return refine_surface(sv, surf, at, la, sa, lb, sb, out, depth);
    }
    if lb - la <= SAME_POINT {
        // Two stations at one point: side x walks its fan seam by seam to
        // `sb`'s foot, the other side solving freely from where it was;
        // then the rest is refined again.
        let (fa, fb) = (sa.feet[x].faces(obj), sb.feet[x].faces(obj));
        let path = fa
            .iter()
            .flat_map(|&f| fb.iter().filter_map(move |&g| fan_path(obj, sa.q, f, g)))
            .min_by_key(|p| p.len())
            .ok_or_else(|| unsupported(sa.q))?;
        let mut prev = sa.clone();
        for w in path.windows(2) {
            let e = common_edge(obj, w[0], w[1]).ok_or_else(|| unsupported(sa.q))?;
            // A ball crossing a valley pivots on the corner; a traced
            // profile bends with its side and takes the seam as any other.
            if sv.rolls() && sv.is_valley(e, w[0]) {
                let (before, corner, after) =
                    valley_cross(sv, at, la, Some(prev.feet), x, w[0], w[1], e)?;
                if before.sec.offsets != prev.sec.offsets {
                    out.push(before);
                }
                out.push(corner);
                out.push(after.clone());
                prev = after;
                continue;
            }
            let mut pin = [None, None];
            pin[x] = Some(Foot::Seam(e));
            let mut st = at(la, Some(prev.feet), pin)?;
            // The other side may have moved too. When it crossed one seam
            // (both sides change at this point), the ball rides both seams
            // at once; otherwise reconcile it first.
            if !share(obj, prev.feet[1 - x], st.feet[1 - x]) {
                let both = match (prev.feet[1 - x], st.feet[1 - x]) {
                    (Foot::Face(f0), Foot::Face(f1)) => common_edge(obj, f0, f1),
                    _ => None,
                };
                let mut joined = false;
                if let Some(e2) = both {
                    let mut pin2 = pin;
                    pin2[1 - x] = Some(Foot::Seam(e2));
                    if let Ok(s2) = at(la, Some(prev.feet), pin2) {
                        st = s2;
                        joined = true;
                    }
                }
                if !joined {
                    refine(sv, at, la, &prev, la, &st, out, depth + 1)?;
                }
            }
            out.push(st.clone());
            prev = st;
        }
        let mut start = prev.feet;
        start[x] = Foot::Face(*path.last().expect("a path"));
        let st = at(la, Some(start), [None, None])?;
        if !share(obj, st.feet[x], sb.feet[x]) {
            return Err(unsupported(sa.q));
        }
        out.push(st.clone());
        return refine(sv, at, la, &st, lb, sb, out, depth + 1);
    }
    let cur = sa.feet[x];
    let (mut lo, mut hi) = (la, lb);
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        if share(obj, at(mid, None, [None, None])?.feet[x], cur) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let next = at(hi, None, [None, None])?.feet[x];
    let lo_foot = at(lo, None, [None, None])?.feet[x];
    let lam = 0.5 * (lo + hi);
    let seam = match (lo_foot, next) {
        (Foot::Face(f), Foot::Face(g)) => common_edge(obj, f, g),
        (Foot::Face(_), Foot::Seam(e)) | (Foot::Seam(e), Foot::Face(_)) => Some(e),
        (Foot::Seam(e), Foot::Seam(_)) => Some(e),
        (Foot::Corner(_), _) | (_, Foot::Corner(_)) => None,
    };
    let seam = seam.filter(|_| lo_foot != next);
    if lo_foot == next {
        // The bisection closed on one foot: the change is at a point.
        let mid = at(lam, None, [None, None])?;
        refine(sv, at, lam, sa, lam, &mid, out, depth + 1)?;
        out.push(mid.clone());
        return refine(sv, at, lam, &mid, lb, sb, out, depth + 1);
    }
    let Some(seam) = seam else {
        // Facets that meet only at a vertex here: the rail rolls around it
        // across the fan of facets between them, one seam at a time.
        let (Foot::Face(f), Foot::Face(g)) = (lo_foot, next) else {
            return Err(unsupported(sa.q));
        };
        let q = at(lam, None, [None, None])?.q;
        let path = fan_path(obj, q, f, g).ok_or_else(|| unsupported(q))?;
        let mut prev = sa.clone();
        let mut lprev = la;
        for w in path.windows(2) {
            let e = common_edge(obj, w[0], w[1]).ok_or_else(|| unsupported(q))?;
            let mut pin = [None, None];
            pin[x] = Some(Foot::Seam(e));
            let st = at(lam, None, pin)?;
            refine(sv, at, lprev, &prev, lam, &st, out, depth + 1)?;
            out.push(st.clone());
            prev = st;
            lprev = lam;
        }
        return refine(sv, at, lprev, &prev, lb, sb, out, depth + 1);
    };
    if let (Foot::Face(f), Foot::Face(g)) = (lo_foot, next)
        && sv.rolls()
        && sv.is_valley(seam, f)
    {
        // A valley seen from the ball: it sits in the corner touching both
        // facets; the rail steps from one contact to the other through the
        // seam point nearest the center.
        let (before, corner, after) = valley_cross(sv, at, lam, None, x, f, g, seam)?;
        refine(sv, at, la, sa, lam, &before, out, depth + 1)?;
        out.push(before);
        out.push(corner);
        out.push(after.clone());
        return refine(sv, at, lam, &after, lb, sb, out, depth + 1);
    }
    let mut pin = [None, None];
    pin[x] = Some(Foot::Seam(seam));
    let st = at(lam, None, pin)?;
    refine(sv, at, la, sa, lam, &st, out, depth + 1)?;
    out.push(st.clone());
    refine(sv, at, lam, &st, lb, sb, out, depth + 1)
}

/// [`refine`] against the side surfaces: the feet are the geometry's own,
/// so a change is where bisection puts it; at one point (the ball touching
/// two places at once across a hollow) the rail steps between them. The
/// changes along one span are taken in turn, not nested, so a span may hold
/// as many as its facets give (a large ball over a fine scan).
#[allow(clippy::too_many_arguments)]
fn refine_surface(
    sv: &Solver,
    surf: &geo::Surface,
    at: &StationAt,
    mut la: f64,
    sa: &Station,
    lb: f64,
    sb: &Station,
    out: &mut Vec<Station>,
    depth: usize,
) -> Result<(), PlanFail> {
    let obj = sv.obj;
    let mut sa = sa.clone();
    for _ in 0..MAX_SPAN_CHANGES {
        let Some(x) = (0..2).find(|&x| !share(obj, sa.feet[x], sb.feet[x])) else {
            return Ok(());
        };
        if lb - la <= SAME_POINT {
            let steps = geo::jump(sv, surf, &sa, sb, x)?;
            let Some(last) = steps.last().cloned() else {
                return Err(unsupported(sa.q));
            };
            out.extend(steps);
            sa = last;
            continue;
        }
        let cur = sa.feet[x];
        let (mut lo, mut hi) = (la, lb);
        for _ in 0..SURFACE_BISECTIONS {
            let mid = 0.5 * (lo + hi);
            if share(obj, at(mid, None, [None, None])?.feet[x], cur) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (lo_st, hi_st) = (at(lo, None, [None, None])?, at(hi, None, [None, None])?);
        // The rail moving on across one seam (not jumping): one station on
        // the seam itself, its rail point on it, which both facets share.
        if let (Foot::Face(f), Foot::Face(g)) = (lo_st.feet[x], hi_st.feet[x])
            && let Some(e) = common_edge(obj, f, g)
            && (rail_point(&lo_st, x) - rail_point(&hi_st, x)).length() <= tol::SWEPT_SLACK
            && let Some((e0, e1)) = obj.edge_endpoints(e)
        {
            let mut st = hi_st.clone();
            st.feet[x] = Foot::Seam(e);
            let k = if x == 0 { 0 } else { st.sec.offsets.len() - 1 };
            let p = rail_point(&hi_st, x);
            let d = e1 - e0;
            let t = ((p - e0).dot(d) / d.dot(d).max(tol::NORMALIZE_MIN_LENGTH)).clamp(0.0, 1.0);
            st.sec.offsets[k] = e0 + d * t - st.q;
            refine(sv, at, la, &sa, hi, &st, out, depth + 1)?;
            out.push(st.clone());
            la = hi;
            sa = st;
            continue;
        }
        refine(sv, at, la, &sa, lo, &lo_st, out, depth + 1)?;
        out.push(lo_st.clone());
        refine(sv, at, lo, &lo_st, hi, &hi_st, out, depth + 1)?;
        out.push(hi_st.clone());
        la = hi;
        sa = hi_st;
    }
    Err(unsupported(sa.q))
}

/// Bisection steps [`refine_surface`] takes for a foot change: a span's
/// parameter to about 1e-15, under [`SAME_POINT`], where the two stations
/// either side are one chain point (fewer steps leave the stations either
/// side of a change far enough apart to fold on a scan).
const SURFACE_BISECTIONS: usize = 50;
/// Within this of the span's parameter, two stations are at one point.
const SAME_POINT: f64 = 1e-12;
/// A guard on the foot changes [`refine_surface`] takes along one span.
const MAX_SPAN_CHANGES: usize = 100_000;

/// The three stations where side `x` crosses valley seam `seam` from `f`
/// to `g` at parameter `lam`: on `f`, between, on `g`, about the exact
/// corner ball.
#[allow(clippy::too_many_arguments)]
fn valley_cross(
    sv: &Solver,
    at: &StationAt,
    lam: f64,
    start: Option<[Foot; 2]>,
    x: usize,
    f: FaceId,
    g: FaceId,
    seam: EdgeId,
) -> Result<(Station, Station, Station), PlanFail> {
    let mut pin = [None, None];
    pin[x] = Some(Foot::Face(f));
    let before = at(lam, start, pin)?;
    pin[x] = Some(Foot::Face(g));
    let after = at(lam, Some(before.feet), pin)?;
    if !share(sv.obj, before.feet[1 - x], after.feet[1 - x]) {
        return Err(unsupported(before.q));
    }
    let (before, after) = exact_corner(sv, before, after, x, f, g)?;
    let corner = corner_station(sv.obj, &before, &after, x, seam)?;
    Ok((before, corner, after))
}

/// Drops stations that run backward along the ball's path. Over a run of
/// stations on the same two facets the centers lie on one line; where the
/// chain turns (at a vertex, or toward a valley corner) the section planes
/// can sample that line out of order, which would fold the strip. A run
/// keeps its first and last stations and those strictly between them in
/// order along the line (D-40).
fn monotone_runs(stations: Vec<Station>, closed: bool) -> Vec<Station> {
    let ns = stations.len();
    let both_faces = |s: &Station| s.feet.iter().all(|f| matches!(f, Foot::Face(_)));
    // Start at a run boundary so no run wraps; an open chain starts at its
    // first station.
    let start = if closed {
        match (0..ns).find(|&j| stations[j].feet != stations[(j + ns - 1) % ns].feet) {
            Some(start) => start,
            None => return stations,
        }
    } else {
        0
    };
    let ring: Vec<Station> = (0..ns)
        .map(|j| stations[(start + j) % ns].clone())
        .collect();
    let center = |s: &Station| s.sec.arc_center.map(|c| s.q + c);
    // Kept stations by their index in `stations`, so the original order
    // (and its start) is restored.
    let mut out: Vec<(usize, Station)> = Vec::with_capacity(ns);
    let mut j = 0;
    while j < ns {
        let mut k = j;
        while k + 1 < ns && ring[k + 1].feet == ring[j].feet {
            k += 1;
        }
        let run = &ring[j..=k];
        let line = (both_faces(&run[0]) && run.len() > 2)
            .then(|| center(&run[0]).zip(center(&run[run.len() - 1])))
            .flatten()
            .and_then(|(a, b)| (b - a).normalized().ok().map(|u| (a, u)));
        match line {
            None => out.extend(
                run.iter()
                    .enumerate()
                    .map(|(i, st)| ((start + j + i) % ns, st.clone())),
            ),
            Some((a, u)) => {
                let s: Vec<f64> = run
                    .iter()
                    .map(|st| center(st).map_or(0.0, |c| (c - a).dot(u)))
                    .collect();
                let last = s.len() - 1;
                // Suffix minima: a station must lie before every later one.
                let mut later_min = vec![f64::INFINITY; s.len()];
                for i in (0..last).rev() {
                    later_min[i] = later_min[i + 1].min(s[i + 1]);
                }
                let mut hi = f64::NEG_INFINITY;
                for (i, st) in run.iter().enumerate() {
                    let keep = i == 0 || i == last || (s[i] > hi && s[i] < later_min[i]);
                    if keep {
                        hi = hi.max(s[i]);
                        out.push(((start + j + i) % ns, st.clone()));
                    }
                }
            }
        }
        j = k + 1;
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, st)| st).collect()
}

/// Removes folds: consecutive stations some ruling of which steps back
/// against the ball's motion. Where the ball rides a ridge its section
/// turns, and a free station (both feet on facets) just after it can fold
/// the strip through itself: it goes, when its neighbors still share a
/// facet on each side, so every rail segment stays in one facet. Two seam
/// stations a hair apart, one per side, that cross each other are one
/// crossing: the ball rides both seams at once, at their midpoint (D-31).
fn unfold(sv: &Solver, mut stations: Vec<Station>, closed: bool) -> Vec<Station> {
    let obj = sv.obj;
    let center = |s: &Station| s.sec.arc_center.map(|c| s.q + c);
    let free = |s: &Station| s.feet.iter().all(|f| matches!(f, Foot::Face(_)));
    let seam_side = |s: &Station| match s.feet {
        [Foot::Seam(e), Foot::Face(_)] => Some((0, e)),
        [Foot::Face(_), Foot::Seam(e)] => Some((1, e)),
        _ => None,
    };
    let folds =
        |a: &Station, b: &Station| {
            let (Some(ca), Some(cb)) = (center(a), center(b)) else {
                return false;
            };
            let u = cb - ca;
            u.length() > tol::POINT_MERGE
                && a.sec.offsets.iter().zip(&b.sec.offsets).any(|(&oa, &ob)| {
                    ((b.q + ob) - (a.q + oa)).dot(u) < -tol::POINT_MERGE * u.length()
                })
        };
    // Every fix removes a station, so this ends.
    loop {
        let ns = stations.len();
        if ns < 4 {
            return stations;
        }
        // An open chain's end stations stay: they are its ends.
        let interior = |i: usize| closed || (i > 0 && i + 1 < ns);
        let removable = |i: usize| {
            let (p, n) = (&stations[(i + ns - 1) % ns], &stations[(i + 1) % ns]);
            interior(i) && free(&stations[i]) && (0..2).all(|x| share(obj, p.feet[x], n.feet[x]))
        };
        let mut fixed = false;
        for j in 0..(if closed { ns } else { ns - 1 }) {
            let k = (j + 1) % ns;
            if !folds(&stations[j], &stations[k]) {
                continue;
            }
            if let Some(i) = [k, j].into_iter().find(|&i| removable(i)) {
                stations.remove(i);
                fixed = true;
                break;
            }
            let (a, b) = (&stations[j], &stations[k]);
            if interior(j)
                && interior(k)
                && let (Some((xa, ea)), Some((xb, eb))) = (seam_side(a), seam_side(b))
                && xa != xb
            {
                let q = a.q + (b.q - a.q) * 0.5;
                let (p, n) = (&stations[(j + ns - 1) % ns], &stations[(k + 1) % ns]);
                let Ok(t) = (n.q - p.q).normalized() else {
                    continue;
                };
                let mut pin = [None, None];
                pin[xa] = Some(Foot::Seam(ea));
                pin[xb] = Some(Foot::Seam(eb));
                if let Ok((sec, feet)) = sv.solve(q, t, a.feet, pin) {
                    stations[j] = Station { q, sec, feet };
                    stations.remove(k);
                    fixed = true;
                    break;
                }
            }
        }
        if !fixed {
            return stations;
        }
    }
}

/// The two stations at a valley corner, re-solved on the exact corner ball:
/// tangent to both facets `f`, `g` of side `x` and to the other side's
/// facet, so both share the other side's rail point bitwise.
fn exact_corner(
    sv: &Solver,
    before: Station,
    after: Station,
    x: usize,
    f: FaceId,
    g: FaceId,
) -> Result<(Station, Station), PlanFail> {
    let q = before.q;
    let BlendProfile::Round {
        radius: r,
        segments,
    } = *sv.profile
    else {
        return Err(unsupported(q));
    };
    let Foot::Face(h) = before.feet[1 - x] else {
        // The other side on a seam too: keep the bisected pair.
        return Ok((before, after));
    };
    let faces = sv.obj.faces();
    let planes = [faces[h].plane, faces[f].plane, faces[g].plane];
    // n·c = n·p − σ r for the three planes.
    let rows: Vec<(Vec3, f64)> = planes
        .iter()
        .map(|pl| {
            (
                pl.normal(),
                pl.normal().dot(pl.point().to_vec()) - sv.sigma * r,
            )
        })
        .collect();
    let (n0, n1, n2) = (rows[0].0, rows[1].0, rows[2].0);
    let det = n0.dot(n1.cross(n2));
    if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return Err(unsupported(q));
    }
    let cv = (n1.cross(n2) * rows[0].1 + n2.cross(n0) * rows[1].1 + n0.cross(n1) * rows[2].1) / det;
    let c = Point3::new(cv.x, cv.y, cv.z);
    // The bisection must have found this corner.
    if (q + before.sec.arc_center.ok_or_else(|| unsupported(q))? - c).length()
        > tol::BLEND_CORNER_AGREE
    {
        return Err(unsupported(q));
    }
    let foot = |n: Vec3| c + n * (sv.sigma * r);
    let rail_h = foot(n0);
    let mk = |st: Station, side: Vec3| -> Result<Station, PlanFail> {
        let (fa, fb) = if x == 0 {
            (foot(side), rail_h)
        } else {
            (rail_h, foot(side))
        };
        Ok(Station {
            sec: arc_section(st.q, c, r, fa, fb, segments as usize)?,
            ..st
        })
    };
    Ok((mk(before, n1)?, mk(after, n2)?))
}

/// The station between the two contacts of a ball in a valley corner:
/// `before` and `after` share the center; side `x`'s rail point is the seam
/// point nearest it, the other points lie on the ball halfway between the
/// two sections.
fn corner_station(
    obj: &Object,
    before: &Station,
    after: &Station,
    x: usize,
    seam: EdgeId,
) -> Result<Station, PlanFail> {
    let q = before.q;
    let (Some(ca), Some(cb), Some(r)) = (
        before.sec.arc_center,
        after.sec.arc_center,
        before.sec.radius,
    ) else {
        return Err(unsupported(q));
    };
    let c = before.q + (ca + (after.q + cb - before.q)) * 0.5;
    let (l0, l1) = obj.edge_endpoints(seam).ok_or_else(|| unsupported(q))?;
    let ld = (l1 - l0).normalized().map_err(|_| unsupported(q))?;
    let s = l0 + ld * (c - l0).dot(ld);
    if super::query::point_segment_distance(s, l0, l1) > tol::POINT_MERGE {
        return Err(unsupported(q));
    }
    let n = before.sec.offsets.len() - 1;
    let rail = if x == 0 { 0 } else { n };
    let offsets = (0..=n)
        .map(|k| {
            let (pa, pb) = (
                before.q + before.sec.offsets[k],
                after.q + after.sec.offsets[k],
            );
            if k == rail {
                Ok(s - q)
            } else if k == n - rail {
                Ok(pa - q)
            } else {
                let d = ((pa - c) + (pb - c))
                    .normalized()
                    .map_err(|_| unsupported(q))?;
                Ok(c + d * r - q)
            }
        })
        .collect::<Result<Vec<_>, PlanFail>>()?;
    let mut feet = before.feet;
    feet[x] = Foot::Seam(seam);
    Ok(Station {
        q,
        sec: Section {
            offsets,
            arc_center: Some(c - q),
            ..before.sec.clone()
        },
        feet,
    })
}

/// The faces around the vertex at `q`, from `f` to `g`, each sharing an
/// edge through that vertex with the next (breadth-first, shortest).
fn fan_path(obj: &Object, q: Point3, f: FaceId, g: FaceId) -> Option<Vec<FaceId>> {
    let v = obj
        .vertices()
        .iter()
        .find(|(_, x)| x.position.approx_eq(q, tol::SWEPT_SLACK))
        .map(|(id, _)| id)?;
    let ve = obj.vertex_edges();
    let edges = ve.get(&v)?;
    let mut prev: BTreeMap<FaceId, FaceId> = BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([f]);
    let mut seen = BTreeSet::from([f]);
    while let Some(h) = queue.pop_front() {
        if h == g {
            let mut path = vec![g];
            let mut cur = g;
            while let Some(&p) = prev.get(&cur) {
                path.push(p);
                cur = p;
            }
            path.reverse();
            return Some(path);
        }
        for &e in edges {
            if let Some((a, b)) = obj.edge_faces(e) {
                let other = if a == h {
                    b
                } else if b == h {
                    a
                } else {
                    continue;
                };
                if seen.insert(other) {
                    prev.insert(other, h);
                    queue.push_back(other);
                }
            }
        }
    }
    None
}

/// The shared edge of two faces.
fn common_edge(obj: &Object, f: FaceId, g: FaceId) -> Option<EdgeId> {
    obj.face_loops(f).into_iter().find_map(|l| {
        obj.loop_half_edges(l).find_map(|h| {
            let e = obj.half_edges()[h].edge;
            obj.edge_faces(e)
                .is_some_and(|(x, y)| (x == f && y == g) || (x == g && y == f))
                .then_some(e)
        })
    })
}

/// Points evenly along a closed chain, `h` apart or a little less, each
/// with the faces either side of the chain edge it lies on.
fn even_nodes(obj: &Object, chain: &Chain, h: f64) -> Option<Vec<(Point3, [FaceId; 2])>> {
    let m = chain.edges.len();
    let pos = |i: usize| obj.vertices()[chain.verts[i % m]].position;
    let mut s = Vec::with_capacity(m + 1);
    s.push(0.0);
    for i in 0..m {
        s.push(s[i] + (pos(i + 1) - pos(i)).length());
    }
    let total = s[m];
    if total <= 0.0 || h <= 0.0 {
        return None;
    }
    // A tiny ball on a long seam would take millions of stations: refuse
    // (the caller reports the chain unsupported) rather than allocate them.
    let steps = (total / h).ceil();
    if steps > crate::deform::MAX_STATIONS as f64 {
        return None;
    }
    let count = (steps as usize).max(8);
    Some(
        (0..count)
            .map(|k| {
                let at = total * k as f64 / count as f64;
                let i = s.partition_point(|&x| x <= at).saturating_sub(1).min(m - 1);
                let len = s[i + 1] - s[i];
                let t = if len > 0.0 { (at - s[i]) / len } else { 0.0 };
                let e = &chain.edges[i];
                (pos(i) + (pos(i + 1) - pos(i)) * t, [e.a, e.b])
            })
            .collect(),
    )
}

/// Per vertex of a closed polyline, the direction of the chord between
/// the points arc length `w` before and after it.
fn smoothed_directions(p: &[Point3], w: f64) -> Option<Vec<Vec3>> {
    let m = p.len();
    let mut s = Vec::with_capacity(m + 1);
    s.push(0.0);
    for i in 0..m {
        s.push(s[i] + (p[(i + 1) % m] - p[i]).length());
    }
    let total = s[m];
    if total <= 0.0 {
        return None;
    }
    let w = w.min(total / 4.0);
    // The point at arc length `at` (wrapped).
    let at = |at: f64| -> Point3 {
        let at = at.rem_euclid(total);
        let i = s.partition_point(|&x| x <= at).saturating_sub(1).min(m - 1);
        let len = s[i + 1] - s[i];
        let t = if len > 0.0 { (at - s[i]) / len } else { 0.0 };
        p[i] + (p[(i + 1) % m] - p[i]) * t
    };
    (0..m)
        .map(|i| (at(s[i] + w) - at(s[i] - w)).normalized().ok())
        .collect()
}

/// The stations of one chain: one at each chain vertex, then those where a
/// foot changes between them, in order, folds removed. An open chain
/// (`ends`) has its end stations in its end faces' planes.
fn chain_stations(
    solver: &Solver,
    chain: &Chain,
    ends: Option<&Ends>,
) -> Result<Vec<Station>, PlanFail> {
    let obj = solver.obj;
    let m = chain.edges.len();
    let closed = ends.is_none();
    let pos = |i: usize| obj.vertices()[chain.verts[if closed { i % m } else { i }]].position;
    // Section planes turn continuously along the chain: at each vertex the
    // plane is square to the bisector of its two edges (WP07 §4.1), and
    // along an edge its normal blends linearly from one end's to the
    // other's. So the planes sample the ball's path in order wherever the
    // chain turns slower than the ball is wide (D-40). An open chain's end
    // planes are its end faces' (D-115).
    let dir = |i: usize| -> Result<Vec3, PlanFail> {
        (pos(i + 1) - pos(i))
            .normalized()
            .map_err(|_| unsupported(pos(i)))
    };
    let nv = if closed { m } else { m + 1 };
    let mut bis: Vec<Vec3> = Vec::with_capacity(nv);
    for i in 0..nv {
        let t = match ends {
            Some(e) if i == 0 => e.normals[0],
            Some(e) if i == m => e.normals[1],
            _ => {
                let (d_in, d_out) = (dir((i + m - 1) % m)?, dir(i)?);
                (d_in + d_out)
                    .normalized()
                    .map_err(|_| unsupported(pos(i)))?
            }
        };
        bis.push(t);
    }
    // The chain points stations start from, the faces either side, and the
    // section planes' normals there: the chain's vertices for the walk.
    let mut nodes: Vec<(Point3, [FaceId; 2])> = (0..nv)
        .map(|i| {
            let e = &chain.edges[i.min(m - 1)];
            (pos(i), [e.a, e.b])
        })
        .collect();
    if let (Some(_), BlendProfile::Round { radius: r, .. }) = (&solver.geo, solver.profile) {
        // Against the surfaces (a scan's seam, jagged at the scale of its
        // facets, its vertices anywhere from a micron to a facet apart) the
        // stations start evenly along the chain, a quarter of the ball
        // apart, and the planes turn with the chord across a ball's width of
        // chain either side, not with each short edge, so the centers they
        // sample still run forward.
        nodes = even_nodes(obj, chain, 0.25 * *r).ok_or_else(|| unsupported(pos(0)))?;
        bis = smoothed_directions(&nodes.iter().map(|n| n.0).collect::<Vec<_>>(), *r)
            .ok_or_else(|| unsupported(pos(0)))?;
    }
    // A round's chain vertices within a radius of an open chain's end go:
    // a ball there would have its tangent points past the end face. The
    // stations between the end and the next vertex come from the walk.
    if let (Some(_), BlendProfile::Round { radius: r, .. }) = (ends, solver.profile) {
        let mut arc = vec![0.0; nodes.len()];
        for i in 1..nodes.len() {
            arc[i] = arc[i - 1] + (nodes[i].0 - nodes[i - 1].0).length();
        }
        let total = arc[nodes.len() - 1];
        let keep: Vec<bool> = (0..nodes.len())
            .map(|i| i == 0 || i + 1 == nodes.len() || (arc[i] >= *r && total - arc[i] >= *r))
            .collect();
        nodes = nodes
            .iter()
            .zip(&keep)
            .filter(|(_, k)| **k)
            .map(|(n, _)| *n)
            .collect();
        bis = bis
            .iter()
            .zip(&keep)
            .filter(|(_, k)| **k)
            .map(|(b, _)| *b)
            .collect();
    }
    let nn = nodes.len();
    let station = |i: usize, t: Vec3| -> Result<Station, PlanFail> {
        let (q, [fa, fb]) = nodes[i];
        let start = [Foot::Face(fa), Foot::Face(fb)];
        let (sec, feet) = solver.solve(q, t, start, [None, None])?;
        Ok(Station { q, sec, feet })
    };
    // A round's end section is its ball's, carried along the chain into
    // the end plane: the ball's tangent points sit off its section plane by
    // the facets' tilt to it, and the strip's end must lie in the end face.
    // The ball is solved in the end plane with its rails allowed a hair
    // past the end face; then each point of the section moves along the
    // chain's direction into the plane, a rail over its side's facets
    // (along the chain's direction projected onto each, crossing a seam
    // where the facet ends first). Where a rail crosses a seam on the way,
    // a station at that crossing goes in between (the section carried that
    // far, the rail riding the seam), so every rail segment lies in one
    // facet. The stations come back in chain order: the ball's own, the
    // crossings, the end section (the other way round at the start).
    let end_station = |i: usize, t: Vec3, d: Vec3| -> Result<Vec<Station>, PlanFail> {
        let (q, [fa, fb]) = nodes[i];
        let start = [Foot::Face(fa), Foot::Face(fb)];
        if !solver.rolls() {
            // A traced section lies in the plane already.
            let (sec, feet) = solver.solve(q, t, start, [None, None])?;
            return Ok(vec![Station { q, sec, feet }]);
        }
        let end_face = ends.map(|e| e.faces[if i == 0 { 0 } else { 1 }]);
        solver.extend_past.set([end_face, None]);
        let solved = solver.solve(q, t, start, [None, None]);
        solver.extend_past.set([None, None]);
        let (sec, feet) = solved?;
        let n = sec.offsets.len() - 1;
        let along_d = d.dot(t);
        if along_d.abs() <= tol::NORMAL_DIRECTION {
            return Err(unsupported(q));
        }
        // Out of the chain at this end; a point past the plane that way is
        // past the end face and comes straight back to the plane.
        let out_dir = if i == 0 { -t } else { t };
        let past = |p: Point3| (p - q).dot(out_dir) > tol::SWEPT_SLACK;
        let project = |d: Vec3, f: FaceId| {
            let nf = obj.faces()[f].plane.normal();
            (d - nf * d.dot(nf)).normalized().ok()
        };
        // A rail's way into the end plane: its waypoints (length so far,
        // point, the facet from there) and the seams it crosses (length,
        // seam).
        type Way = (Vec<(f64, Point3, FaceId)>, Vec<(f64, EdgeId)>);
        let carry = |x: usize, f0: FaceId, p0: Point3| -> Option<Way> {
            let (mut f, mut p, mut len) = (f0, p0, 0.0);
            let mut pts = vec![(0.0, p0, f0)];
            let mut cross: Vec<(f64, EdgeId)> = Vec::new();
            let mut visited: Vec<FaceId> = Vec::new();
            for _ in 0..64 {
                let mut dir = project(d, f)?;
                let along = dir.dot(t);
                if along.abs() <= tol::NORMAL_DIRECTION {
                    return None;
                }
                let mut to_plane = (q - p).dot(t) / along;
                if to_plane < 0.0 {
                    dir = -dir;
                    to_plane = -to_plane;
                }
                if to_plane <= tol::POINT_MERGE {
                    return Some((pts, cross));
                }
                match solver.exit_along(f, p, dir) {
                    Some((s, e)) if s < to_plane - tol::POINT_MERGE => {
                        p = p + dir * s;
                        len += s;
                        let (g1, g2) = obj.edge_faces(e)?;
                        let g = if g1 == f { g2 } else { g1 };
                        if visited.contains(&g) || solver.other_side[x].contains(&g) {
                            return None;
                        }
                        visited.push(f);
                        cross.push((len, e));
                        f = g;
                        pts.push((len, p, f));
                    }
                    _ => {
                        let at = p + dir * to_plane;
                        if !in_face(obj, f, at) {
                            return None;
                        }
                        len += to_plane;
                        pts.push((len, at, f));
                        return Some((pts, cross));
                    }
                }
            }
            None
        };
        let mut ways: Vec<Way> = Vec::with_capacity(2);
        for (x, &foot) in feet.iter().enumerate() {
            let p0 = q + sec.offsets[if x == 0 { 0 } else { n }];
            let way = match foot {
                Foot::Corner(_) => return Err(unsupported(q)),
                foot => foot
                    .faces(obj)
                    .into_iter()
                    .find_map(|f| carry(x, f, p0))
                    .ok_or_else(|| unsupported(q))?,
            };
            ways.push(way);
        }
        let total = |x: usize| ways[x].0.last().map_or(0.0, |w| w.0);
        // The point and facet of side `x`'s way at `s` along it.
        let way_at = |x: usize, s: f64| -> (Point3, FaceId) {
            let pts = &ways[x].0;
            for w in pts.windows(2) {
                let ((s0, p0, f0), (s1, p1, _)) = (w[0], w[1]);
                if s <= s1 + tol::POINT_MERGE {
                    let u = if s1 > s0 {
                        ((s - s0) / (s1 - s0)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    return (p0 + (p1 - p0) * u, f0);
                }
            }
            let last = pts[pts.len() - 1];
            (last.1, last.2)
        };
        // Where the rails cross seams, as fractions of their ways (a rail
        // past the end face crosses nothing inside the solid on its way
        // back).
        let leading = [past(ways[0].0[0].1), past(ways[1].0[0].1)];
        let mut fractions: Vec<f64> = Vec::new();
        for (x, way) in ways.iter().enumerate() {
            if total(x) > 0.0 && !leading[x] {
                for &(len, _) in &way.1 {
                    let u = len / total(x);
                    if !fractions.iter().any(|&v: &f64| (v - u).abs() <= 1e-9) {
                        fractions.push(u);
                    }
                }
            }
        }
        fractions.sort_by(|a, b| a.total_cmp(b));
        fractions.push(1.0);
        let mut out: Vec<Station> = vec![Station {
            q,
            sec: sec.clone(),
            feet,
        }];
        for &u in &fractions {
            let mut offsets = Vec::with_capacity(n + 1);
            let mut st_feet = feet;
            for k in 0..=n {
                let p = q + sec.offsets[k];
                let side = if k == 0 {
                    Some(0)
                } else {
                    (k == n).then_some(1)
                };
                let Some(x) = side else {
                    let uu = if past(p) { 1.0 } else { u };
                    offsets.push((p + d * (((q - p).dot(t) / along_d) * uu)) - q);
                    continue;
                };
                let uu = if leading[x] { 1.0 } else { u };
                let s = uu * total(x);
                let (p2, f) = way_at(x, s);
                offsets.push(p2 - q);
                let at_seam = ways[x]
                    .1
                    .iter()
                    .find(|&&(len, _)| (len - s).abs() <= 1e-9 * total(x).max(1.0))
                    .map(|&(_, e)| e);
                st_feet[x] = match at_seam {
                    Some(e) => Foot::Seam(e),
                    None if uu >= 1.0 => {
                        // On an edge of the end face the rail rides it.
                        let seam = obj.face_loops(f).into_iter().find_map(|l| {
                            obj.loop_half_edges(l).find_map(|h| {
                                let e = obj.half_edges()[h].edge;
                                let (a, b) = obj.edge_endpoints(e)?;
                                (obj.edge_faces(e).is_some_and(|(g1, g2)| {
                                    !solver.other_side[x].contains(&g1)
                                        && !solver.other_side[x].contains(&g2)
                                }) && super::query::point_segment_distance(p2, a, b)
                                    <= tol::SWEPT_SLACK)
                                    .then_some(e)
                            })
                        });
                        seam.map_or(Foot::Face(f), Foot::Seam)
                    }
                    None => Foot::Face(f),
                };
            }
            let arc_center = sec.arc_center.map(|c| {
                let cp = q + c;
                (cp + d * (((q - cp).dot(t) / along_d) * u)) - q
            });
            out.push(Station {
                q,
                sec: Section {
                    offsets,
                    arc_center,
                    ..sec.clone()
                },
                feet: st_feet,
            });
        }
        // A point of the ball's own station or a crossing's past the plane
        // is the end section's, its rail's foot the end section's.
        let end_sec = out[out.len() - 1].clone();
        let last = out.len() - 1;
        for st in out[..last].iter_mut() {
            for k in 0..=n {
                if past(st.q + st.sec.offsets[k]) {
                    st.sec.offsets[k] = end_sec.sec.offsets[k];
                    if k == 0 {
                        st.feet[0] = end_sec.feet[0];
                    } else if k == n {
                        st.feet[1] = end_sec.feet[1];
                    }
                }
            }
        }
        if i == 0 {
            out.reverse();
        }
        Ok(out)
    };
    let mut end_stations: Vec<Vec<Station>> = Vec::new();
    if ends.is_some() {
        let (d0, d1) = (dir(0)?, dir(m - 1)?);
        // Near an end the section planes turn from the end face's plane
        // into the chain's own over twice the end section's reach, so no
        // rail runs out through the end face: a plane square to the chain
        // a hair from an oblique end face meets that face within the rail.
        // An end section lies in its end face.
        end_stations.push(end_station(0, bis[0], d0)?);
        end_stations.push(end_station(nn - 1, bis[nn - 1], d1)?);
        let reach = |sts: &[Station]| {
            sts.iter()
                .map(|st| {
                    let n = st.sec.offsets.len() - 1;
                    2.0 * st.sec.offsets[0].length().max(st.sec.offsets[n].length())
                })
                .fold(0.0, f64::max)
        };
        let (r0, r1) = (reach(&end_stations[0]), reach(&end_stations[1]));
        let mut arc = vec![0.0; nn];
        for i in 1..nn {
            arc[i] = arc[i - 1] + (nodes[i].0 - nodes[i - 1].0).length();
        }
        let total = arc[nn - 1];
        let (n0, n1) = (bis[0], bis[nn - 1]);
        for i in 1..nn - 1 {
            let w0 = if r0 > 0.0 {
                (1.0 - arc[i] / r0).max(0.0)
            } else {
                0.0
            };
            let w1 = if r1 > 0.0 {
                (1.0 - (total - arc[i]) / r1).max(0.0)
            } else {
                0.0
            };
            if w0 <= 0.0 && w1 <= 0.0 {
                continue;
            }
            bis[i] = (bis[i] * ((1.0 - w0) * (1.0 - w1)) + n0 * w0 + n1 * w1)
                .normalized()
                .map_err(|_| unsupported(nodes[i].0))?;
        }
    }
    // Per chain vertex, its stations (several at an open chain's end).
    let mut vertex_stations: Vec<Vec<Station>> = Vec::with_capacity(nn);
    for (i, &t) in bis.iter().enumerate() {
        let sts = match (i, end_stations.len()) {
            (0, 2) => end_stations[0].clone(),
            (i, 2) if i == nn - 1 => end_stations[1].clone(),
            _ => vec![station(i, t)?],
        };
        vertex_stations.push(sts);
    }
    // Stations where a foot changes (WP07 §4.2): between any two stations
    // whose feet on a side share no facet, bisect for the change and pin
    // that side to the seam there; recurse until every rail segment lies in
    // one facet.
    let mut stations: Vec<Station> = Vec::new();
    for i in 0..(if closed { nn } else { nn - 1 }) {
        let (a, [fa, fb]) = nodes[i];
        let b = nodes[(i + 1) % nn].0;
        let (t0, t1) = (bis[i], bis[(i + 1) % nn]);
        let at = |lam: f64,
                  start: Option<[Foot; 2]>,
                  pin: [Option<Foot>; 2]|
         -> Result<Station, PlanFail> {
            let q = a + (b - a) * lam;
            let t = (t0 * (1.0 - lam) + t1 * lam)
                .normalized()
                .map_err(|_| unsupported(q))?;
            let st = start.unwrap_or([Foot::Face(fa), Foot::Face(fb)]);
            let (sec, feet) = solver.solve(q, t, st, pin)?;
            Ok(Station { q, sec, feet })
        };
        let v0 = vertex_stations[i][vertex_stations[i].len() - 1].clone();
        stations.extend(vertex_stations[i].iter().cloned());
        // Along an end segment the ball's rails may run past the end face
        // (its tangent points lead the station by the facets' tilt): those
        // stations go once the end sections are in.
        if let Some(e) = ends {
            let mut past = [None, None];
            if i == 0 {
                past[0] = Some(e.faces[0]);
            }
            if i + 2 == nn {
                past[1] = Some(e.faces[1]);
            }
            solver.extend_past.set(past);
        }
        let refined = refine(
            solver,
            &at,
            0.0,
            &v0,
            1.0,
            &vertex_stations[(i + 1) % nn][0],
            &mut stations,
            0,
        );
        solver.extend_past.set([None, None]);
        refined?;
    }
    if let Some(e) = ends {
        stations.extend(vertex_stations[nn - 1].iter().cloned());
        // A station's point past an end face (the ball's tangent points
        // lead the station by the facets' tilt near that end) is the end
        // section's point there, the rail's foot the end section's: the
        // strip's end lies in the end face, and a station wholly past it
        // repeats the end section and goes.
        let last = stations.len() - 1;
        let (s0, s1) = (stations[0].clone(), stations[last].clone());
        let (q0, q1) = (s0.q, s1.q);
        let (t0, t1) = (e.normals[0], e.normals[1]);
        for st in stations[1..last].iter_mut() {
            let n = st.sec.offsets.len() - 1;
            // The nearer end's plane (both ends may lie on one face).
            let near_start = (st.q - q0).length() <= (st.q - q1).length();
            for k in 0..=n {
                let p = st.q + st.sec.offsets[k];
                let end = if near_start && (p - q0).dot(t0) < -tol::SWEPT_SLACK {
                    Some(&s0)
                } else if !near_start && (p - q1).dot(t1) > tol::SWEPT_SLACK {
                    Some(&s1)
                } else {
                    None
                };
                let Some(end) = end else {
                    continue;
                };
                st.sec.offsets[k] = (end.q + end.sec.offsets[k]) - st.q;
                if k == 0 {
                    st.feet[0] = end.feet[0];
                } else if k == n {
                    st.feet[1] = end.feet[1];
                }
            }
        }
    }
    if let Some(surf) = &solver.geo {
        let _ = surf;
        stations = without_returns(stations);
    }
    Ok(unfold(solver, monotone_runs(stations, closed), closed))
}

/// Cuts out every run of stations at one chain point that comes back to
/// where it started (the same feet, the same rail points): where a ball
/// rests in a crease against two facets at once, the nearest of the two
/// can flip back and forth at a single point, which would leave a spike in
/// the rail.
fn without_returns(stations: Vec<Station>) -> Vec<Station> {
    let mut out: Vec<Station> = Vec::with_capacity(stations.len());
    for st in stations {
        let back = out
            .iter()
            .rposition(|s| (s.q - st.q).length() > tol::POINT_MERGE);
        let from = back.map_or(0, |b| b + 1);
        let n = st.sec.offsets.len() - 1;
        let same = |s: &Station| {
            s.feet == st.feet
                && [0, n].iter().all(|&k| {
                    (s.q + s.sec.offsets[k]).approx_eq(st.q + st.sec.offsets[k], tol::SWEPT_SLACK)
                })
        };
        if let Some(j) = (from..out.len()).find(|&j| same(&out[j])) {
            out.truncate(j + 1);
            continue;
        }
        out.push(st);
    }
    out
}

/// Plans the swept blend of one chain: the kept pieces of the cut side
/// faces (and of an open chain's end faces) go to `kept`, the strip to
/// `add`, and the faces it replaces to `out.removed`. Returns the facet
/// walk's refusal where the solve against the side surfaces stood in for
/// it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_swept(
    obj: &Object,
    chain: &Chain,
    profile: &BlendProfile,
    blend: BlendId,
    kind: BlendKind,
    picked: &BTreeSet<EdgeId>,
    pts: &mut Pts,
    out: &mut Out,
    kept: &mut Vec<PatchFace>,
    add: &mut Vec<PatchFace>,
) -> Result<Option<PlanFail>, PlanFail> {
    let at0 = obj.vertices()[chain.verts[0]].position;
    let ends = if chain.closed {
        None
    } else {
        Some(open_ends(obj, chain, picked)?)
    };
    let convexity = chain.edges[0].convexity;
    if chain.edges.iter().any(|e| e.convexity != convexity) {
        return Err(unsupported(at0));
    }
    let mut other_side: [BTreeSet<FaceId>; 2] = [BTreeSet::new(), BTreeSet::new()];
    for e in &chain.edges {
        other_side[0].insert(e.b);
        other_side[1].insert(e.a);
    }
    let sigma = if convexity == Convexity::Convex {
        1.0
    } else {
        -1.0
    };
    let walk = Solver {
        obj,
        profile,
        sigma,
        other_side: other_side.clone(),
        geo: None,
        extend_past: std::cell::Cell::new([None, None]),
    };
    // The facet walk first; where it refuses, the ball against the side
    // surfaces themselves (D-72). A chain the walk takes keeps its result.
    let surface_solver = Solver {
        obj,
        profile,
        sigma,
        other_side,
        geo: Some(geo::Surface::new(chain)),
        extend_past: std::cell::Cell::new([None, None]),
    };
    let refused = match chain_stations(&walk, chain, ends.as_ref()) {
        Ok(stations) => {
            let built = build_swept(
                obj,
                chain,
                convexity,
                blend,
                kind,
                stations,
                false,
                ends.as_ref(),
                pts,
                out,
                kept,
                add,
            );
            built?;
            return Ok(None);
        }
        Err(e) if !SURFACE_FALLBACK.with(std::cell::Cell::get) => return Err(e),
        Err(e) => e,
    };
    // The walk refused: the ball against the side surfaces (D-72). Where
    // the strip cannot be built at a point, the stations near it go and
    // the rails are carried across the facets there instead, a few rounds.
    // The surfaces roll closed loops only (D-115).
    let BlendProfile::Round { radius: r, .. } = *profile else {
        return Err(refused);
    };
    if ends.is_some() {
        return Err(refused);
    }
    let surf = surface_solver.geo.as_ref().expect("a surface");
    let merge = (obj.planarity_tol * tol::IMPORT_MERGE_REL).max(tol::POINT_MERGE);
    // The stations depend only on the chain and the profile: the rounds
    // of one attempt (here and after the blend's checks) share them.
    let key = (
        chain.edges.iter().map(|e| e.edge).collect::<Vec<_>>(),
        r.to_bits(),
        profile.clone(),
    );
    let cached = ROLLED.with(|c| {
        c.borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, st)| st.clone())
    });
    let rolled = match cached {
        Some(st) => st,
        None => {
            let st = chain_stations(&surface_solver, chain, None)
                .and_then(|st| geo::fold_free(&surface_solver, surf, st, merge))
                .map_err(|_| refused)?;
            ROLLED.with(|c| c.borrow_mut().push((key, st.clone())));
            st
        }
    };
    for _ in 0..avoid_rounds().max(1) {
        let avoid = AVOID.with(|a| a.borrow().clone());
        let stations =
            geo::avoiding(&surface_solver, surf, rolled.clone(), &avoid, r).map_err(|_| refused)?;
        let (out0, kept0, add0) = (out.clone(), kept.len(), add.len());
        match build_swept(
            obj, chain, convexity, blend, kind, stations, true, None, pts, out, kept, add,
        ) {
            Ok(()) => return Ok(Some(refused)),
            Err(f) => {
                *out = out0;
                kept.truncate(kept0);
                add.truncate(add0);
                let PlanFail::Hard(BlendError::UnsupportedVertex { at }) = f else {
                    return Err(refused);
                };
                if avoid.iter().any(|p| p.approx_eq(at, tol::POINT_MERGE)) {
                    return Err(refused);
                }
                avoid_near(at);
            }
        }
    }
    Err(refused)
}

/// Builds a swept chain's strip from its stations and cuts its side faces
/// (see [`plan_swept`]); `surface` when the stations come from the solve
/// against the side surfaces; `ends` for an open chain, whose end faces
/// are cut along its end sections.
#[allow(clippy::too_many_arguments)]
fn build_swept(
    obj: &Object,
    chain: &Chain,
    convexity: Convexity,
    blend: BlendId,
    kind: BlendKind,
    mut stations: Vec<Station>,
    surface: bool,
    ends: Option<&Ends>,
    pts: &mut Pts,
    out: &mut Out,
    kept: &mut Vec<PatchFace>,
    add: &mut Vec<PatchFace>,
) -> Result<(), PlanFail> {
    let m = chain.edges.len();
    let closed = ends.is_none();
    let pos = |i: usize| obj.vertices()[chain.verts[if closed { i % m } else { i }]].position;
    // A rail point on its foot facet's edge (a seam foot, or a contact that
    // lands on the facet's boundary) lies on that edge exactly: the solve
    // leaves it nanometers off, which would lift it off both facets and
    // keep it apart from the same point of a neighboring station.
    let slack = tol::SWEPT_SLACK;
    let onto = |r: Point3, e: EdgeId| -> Option<Point3> {
        let (p0, p1) = obj.edge_endpoints(e)?;
        let d = p1 - p0;
        let l2 = d.dot(d);
        (l2 > 0.0).then(|| p0 + d * ((r - p0).dot(d) / l2))
    };
    for st in &mut stations {
        let n = st.sec.offsets.len() - 1;
        for x in 0..2 {
            let k = if x == 0 { 0 } else { n };
            let r = st.q + st.sec.offsets[k];
            let on = match st.feet[x] {
                Foot::Seam(e) => onto(r, e),
                Foot::Corner(v) => Some(obj.vertices()[v].position),
                Foot::Face(f) => obj.face_loops(f).into_iter().find_map(|l| {
                    obj.loop_half_edges(l).find_map(|h| {
                        let e = obj.half_edges()[h].edge;
                        let (p0, p1) = obj.edge_endpoints(e)?;
                        (super::query::point_segment_distance(r, p0, p1) <= slack
                            && (r - p0).length() > slack
                            && (r - p1).length() > slack)
                            .then(|| onto(r, e))
                            .flatten()
                    })
                }),
            };
            if let Some(on) = on {
                st.sec.offsets[k] = on - st.q;
            }
        }
    }
    let n = stations[0].sec.offsets.len() - 1;
    // Points: every station's section points once; a point that coincides
    // with the previous station's on the same ruling is that point. On
    // imported (f32-noisy) facets coincidence is looser, well inside the
    // object's planarity tolerance (D-42).
    let merge = (obj.planarity_tol * tol::IMPORT_MERGE_REL).max(tol::POINT_MERGE);
    let mut cuts: Vec<Vec<usize>> = Vec::with_capacity(stations.len());
    // Against the surfaces a point recurs out of order (a station either
    // side of a step across a hollow): every point within `merge` of one
    // made already is that one, found through a grid of `merge` cells.
    let mut grid: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    let cell = |p: Point3| {
        [
            (p.x / merge).floor() as i64,
            (p.y / merge).floor() as i64,
            (p.z / merge).floor() as i64,
        ]
    };
    // The way the stations run at each, across the chain points either side
    // that differ from its own (several stations share a point at a step).
    let ahead: Vec<Option<Vec3>> = (0..stations.len())
        .map(|j| {
            let q = stations[j].q;
            let differs = |i: &usize| (stations[*i].q - q).length() > tol::POINT_MERGE;
            let next = (j + 1..stations.len()).find(differs)?;
            let prev = (0..j).rev().find(differs).unwrap_or(j);
            (stations[next].q - stations[prev].q).normalized().ok()
        })
        .collect();
    for (j, s_) in stations.iter().enumerate() {
        let row: Vec<usize> = s_
            .sec
            .offsets
            .iter()
            .enumerate()
            .map(|(k, &o)| {
                let p = s_.q + o;
                // Against the surfaces a point of the arc that would
                // step back (where the ball pivots on a corner of the scan,
                // its arc turns about the contact) stays where it was until
                // the ball's progress carries it forward again: the strip
                // must not fold over itself.
                // So does a rail point on the same foot as the station
                // before (riding a ridge, the contact can slide a hair back
                // along it): it is still on that foot.
                let rail = if k == 0 {
                    Some(0)
                } else {
                    (k == n).then_some(1)
                };
                if surface
                    && rail.is_none_or(|x| j > 0 && stations[j - 1].feet[x] == s_.feet[x])
                    && let (Some(prev), Some(fwd)) = (cuts.last(), ahead[j])
                    && (p - pts.pos(prev[k])).dot(fwd) <= merge
                {
                    return prev[k];
                }
                // Against the surfaces a rail point within a hair of a
                // facet's corner is that corner, so the faces either side of
                // it agree on where it is. Twice the edge-split slack: one
                // farther off still lies farther than the slack from the
                // corner once a split snaps it onto an edge there.
                if surface && (k == 0 || k == n) {
                    let x = if k == 0 { 0 } else { 1 };
                    let corner = s_.feet[x].faces(obj).into_iter().find_map(|f| {
                        obj.face_loops(f).into_iter().find_map(|l| {
                            obj.loop_half_edges(l)
                                .map(|h| obj.half_edges()[h].origin)
                                .find(|&v| {
                                    (obj.vertices()[v].position - p).length()
                                        <= 2.0 * tol::SWEPT_SLACK
                                })
                        })
                    });
                    if let Some(v) = corner {
                        return pts.existing(obj, v);
                    }
                }
                match (cuts.last(), cuts.first()) {
                    (Some(prev), _) if pts.pos(prev[k]).approx_eq(p, merge) => prev[k],
                    // The loop closes onto its first station.
                    (_, Some(first)) if closed && pts.pos(first[k]).approx_eq(p, merge) => first[k],
                    _ if surface => {
                        let [cx, cy, cz] = cell(p);
                        let mut found = None;
                        for dx in -1..=1 {
                            for dy in -1..=1 {
                                for dz in -1..=1 {
                                    for &i in
                                        grid.get(&[cx + dx, cy + dy, cz + dz]).into_iter().flatten()
                                    {
                                        if found.is_none() && pts.pos(i).approx_eq(p, merge) {
                                            found = Some(i);
                                        }
                                    }
                                }
                            }
                        }
                        found.unwrap_or_else(|| {
                            let i = pts.add(p);
                            grid.entry(cell(p)).or_default().push(i);
                            i
                        })
                    }
                    _ => pts.add(p),
                }
            })
            .collect();
        cuts.push(row);
    }
    // Drop stations identical to their predecessor.
    let mut keep: Vec<usize> = Vec::new();
    for j in 0..cuts.len() {
        // Only an exact repeat goes: same points and the same feet (a seam
        // station is the only link between the facets either side of it).
        if keep
            .last()
            .is_some_and(|&p| cuts[p] == cuts[j] && stations[p].feet == stations[j].feet)
        {
            continue;
        }
        keep.push(j);
    }
    let mut stations: Vec<Station> = keep.iter().map(|&j| stations[j].clone()).collect();
    let mut cuts: Vec<Vec<usize>> = keep.iter().map(|&j| cuts[j].clone()).collect();
    // A last station that is the first again goes (the loop closes on it).
    while closed && cuts.len() > 1 && cuts[cuts.len() - 1] == cuts[0] {
        cuts.pop();
        stations.pop();
    }
    let ns = stations.len();
    // The rail segments: between consecutive stations, round for a loop.
    let nseg = if closed { ns } else { ns - 1 };
    // The facets each rail segment j → j+1 lies in, per side: those of
    // both stations' feet (two along a seam).
    let seg_faces = |x: usize, j: usize| -> Vec<FaceId> {
        let next = stations[(j + 1) % stations.len()].feet[x].faces(obj);
        stations[j].feet[x]
            .faces(obj)
            .into_iter()
            .filter(|f| next.contains(f))
            .collect()
    };
    // The strip: quads between consecutive stations, split where not
    // planar, no claims, soft inside.
    let (fa, fb) = (
        &obj.faces()[chain.edges[0].a],
        &obj.faces()[chain.edges[0].b],
    );
    let material = (fa.material == fb.material)
        .then_some(fa.material)
        .flatten();
    let back_material = (fa.back_material == fb.back_material)
        .then_some(fa.back_material)
        .flatten();
    let soft = EdgeMeta {
        soft: true,
        ..EdgeMeta::default()
    };
    let mut strip: Vec<(Vec<usize>, Plane)> = Vec::new();
    let mut vote = 0.0;
    for j in 0..nseg {
        let jn = (j + 1) % ns;
        let (c0, c1) = (&cuts[j], &cuts[jn]);
        // A round's facets face away from its center; any other profile's
        // face the corner they replace.
        let center = |s: &Station| {
            s.sec
                .round
                .then_some(s.sec.arc_center)
                .flatten()
                .map(|c| s.q + c)
        };
        let ref_pt = center(&stations[j])
            .zip(center(&stations[jn]))
            .map(|(a, b)| a + (b - a) * 0.5);
        let q_mid = stations[j].q + (stations[jn].q - stations[j].q) * 0.5;
        for k in 0..n {
            let mut quad = vec![c0[k], c1[k], c1[k + 1], c0[k + 1]];
            quad.dedup();
            if quad.first() == quad.last() && quad.len() > 1 {
                quad.pop();
            }
            if quad.len() < 3 {
                continue;
            }
            let ps: Vec<Point3> = quad.iter().map(|&x| pts.pos(x)).collect();
            let planar = Plane::from_polygon(&ps).is_ok_and(|pl| {
                ps.iter()
                    .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
            });
            let pieces = if planar || quad.len() == 3 {
                vec![quad]
            } else {
                out.meta(c0[k], c1[k + 1], soft);
                vec![
                    vec![c0[k], c1[k], c1[k + 1]],
                    vec![c0[k], c1[k + 1], c0[k + 1]],
                ]
            };
            for ring in pieces {
                let ps: Vec<Point3> = ring.iter().map(|&x| pts.pos(x)).collect();
                let plane = Plane::from_polygon(&ps)
                    .map_err(|_| PlanFail::Hard(BlendError::BlendDegenerate))?;
                let g = ps
                    .iter()
                    .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
                    / ps.len() as f64;
                let convex = convexity == Convexity::Convex;
                let (side, want) = match ref_pt {
                    Some(c) => (plane.normal().dot(c.to_vec() - g), !convex),
                    None => (plane.normal().dot(q_mid.to_vec() - g), convex),
                };
                // Area-weighted vote for the strip's one winding: a sliver
                // quad at a seam station can fold, and its own test would
                // flip it against its neighbors.
                let area = (1..ps.len() - 1)
                    .map(|i| (ps[i] - ps[0]).cross(ps[i + 1] - ps[0]).length())
                    .sum::<f64>();
                vote += if (side > 0.0) == want { area } else { -area };
                strip.push((ring, plane));
            }
            // A ruling inside the section is soft where the profile meets
            // itself smoothly (an arc's facets); a profile's own corner
            // stays sharp.
            if k > 0 && stations[j].sec.soft.get(k).copied().unwrap_or(true) {
                out.meta(c0[k], c1[k], soft);
            }
        }
        // An open chain's first section is its end crease on the start
        // face, not a ruling inside the strip.
        if closed || j > 0 {
            for k in 0..n {
                out.meta(c0[k], c0[k + 1], soft);
            }
        }
    }
    // The side faces, cut along each rail (WP07 §4.4).
    let d_max = stations
        .iter()
        .map(|s| s.sec.offsets[0].length().max(s.sec.offsets[n].length()))
        .fold(0.0, f64::max);
    let chain_pts: Vec<Point3> = (0..=m).map(pos).collect();
    let near_chain = |p: Point3| {
        (0..m).any(|i| {
            super::query::point_segment_distance(p, chain_pts[i], chain_pts[i + 1])
                <= 2.0 * d_max + tol::POINT_MERGE
        })
    };
    // The kept pieces this chain cut (new ones and an earlier chain's
    // pieces cut again), for the strip's winding.
    let mut kept_touched: Vec<usize> = Vec::new();
    // The rail segments and the faces each lies in, per side.
    let mut side_cuts: Vec<SideCut> = Vec::with_capacity(2);
    for x in 0..2 {
        let rail: Vec<usize> = cuts
            .iter()
            .map(|c| if x == 0 { c[0] } else { c[n] })
            .collect();
        let mut rail_segs: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut by_face: BTreeMap<FaceId, Vec<(usize, usize)>> = BTreeMap::new();
        for j in 0..nseg {
            let (a, b) = (rail[j], rail[(j + 1) % ns]);
            if a == b {
                // Two stations resting on one corner.
                continue;
            }
            rail_segs.insert((a.min(b), a.max(b)));
            let faces = seg_faces(x, j);
            if faces.is_empty() {
                return Err(unsupported(pts.pos(a)));
            }
            for f in faces {
                by_face.entry(f).or_default().push((a, b));
            }
        }
        side_cuts.push((by_face, rail_segs));
    }
    match ends {
        // A loop: each side's faces are cut and flooded on their own.
        None => {
            for (x, (by_face, rail_segs)) in side_cuts.iter().enumerate() {
                kept_touched.extend(cut_and_remove(
                    obj,
                    chain,
                    &[x],
                    by_face,
                    rail_segs,
                    &[],
                    &near_chain,
                    surface,
                    pts,
                    out,
                    kept,
                )?);
            }
        }
        // An open chain: its end faces are cut along the end sections, and
        // the flood runs from both sides through the end faces' corners at
        // once (an end face is cut once, whichever side reaches it).
        Some(e) => {
            let (mut by_face, mut rail_segs) = side_cuts.remove(0);
            let (b1, r1) = side_cuts.remove(0);
            for (f, segs) in b1 {
                by_face.entry(f).or_default().extend(segs);
            }
            rail_segs.extend(r1);
            // A convex chain's end section cuts the corner off its end
            // face; a concave chain's fills it out (the fillet adds
            // material), so the face grows by the section and the sides'
            // bands end at the face's own edges.
            let concave = convexity == Convexity::Concave;
            let mut grow: Vec<(FaceId, usize, Vec<usize>)> = Vec::new();
            for (end, row) in [(0, &cuts[0]), (1, &cuts[ns - 1])] {
                let mut row = row.clone();
                row.dedup();
                if row.len() < 2 {
                    return Err(unsupported(stations[if end == 0 { 0 } else { ns - 1 }].q));
                }
                for w in row.windows(2) {
                    rail_segs.insert((w[0].min(w[1]), w[0].max(w[1])));
                }
                if concave {
                    let v = pts.existing(obj, chain.verts[if end == 0 { 0 } else { m }]);
                    for r in [row[0], row[row.len() - 1]] {
                        rail_segs.insert((v.min(r), v.max(r)));
                    }
                    grow.push((e.faces[end], v, row));
                } else {
                    for w in row.windows(2) {
                        by_face.entry(e.faces[end]).or_default().push((w[0], w[1]));
                    }
                }
            }
            kept_touched.extend(cut_and_remove(
                obj,
                chain,
                &[0, 1],
                &by_face,
                &rail_segs,
                &grow,
                &near_chain,
                surface,
                pts,
                out,
                kept,
            )?);
        }
    }
    // Rail points snapped onto the edges they split moved a hair: refit the
    // strip pieces they bound, splitting a quad that no longer lies flat.
    let fits = |ring: &[usize], pl: &Plane| {
        ring.iter()
            .all(|&i| pl.signed_distance(pts.pos(i)).abs() <= tol::PLANE_DIST)
    };
    let mut refit: Vec<(Vec<usize>, Plane)> = Vec::with_capacity(strip.len());
    for (ring, plane) in strip {
        if fits(&ring, &plane) {
            refit.push((ring, plane));
            continue;
        }
        let fitted = |r: &[usize]| -> Result<Plane, PlanFail> {
            let ps: Vec<Point3> = r.iter().map(|&i| pts.pos(i)).collect();
            let pl = Plane::from_polygon(&ps)
                .map_err(|_| PlanFail::Hard(BlendError::BlendDegenerate))?;
            Ok(if pl.normal().dot(plane.normal()) < 0.0 {
                pl.flipped()
            } else {
                pl
            })
        };
        let whole = fitted(&ring)?;
        if fits(&ring, &whole) || ring.len() == 3 {
            refit.push((ring, whole));
        } else {
            out.meta(ring[0], ring[2], soft);
            for tri in [
                vec![ring[0], ring[1], ring[2]],
                vec![ring[0], ring[2], ring[3]],
            ] {
                let pl = fitted(&tri)?;
                refit.push((tri, pl));
            }
        }
    }
    let strip = refit;
    // The strip's winding: opposite the kept side pieces along the rails
    // (every rail edge is shared by one strip face and one kept piece). The
    // ball-center vote decides only when no rail edge is shared.
    let kept_dir: BTreeSet<(usize, usize)> = kept_touched
        .iter()
        .map(|&i| &kept[i])
        .flat_map(|f| std::iter::once(&f.outer).chain(&f.holes))
        .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
        .collect();
    let (mut same, mut opposite) = (0usize, 0usize);
    for (ring, _) in &strip {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if kept_dir.contains(&(a, b)) {
                same += 1;
            } else if kept_dir.contains(&(b, a)) {
                opposite += 1;
            }
        }
    }
    let keep_winding = match (same, opposite) {
        (0, 0) => vote > 0.0,
        (0, _) => true,
        (_, 0) => false,
        _ => return Err(PlanFail::Hard(BlendError::BlendDegenerate)),
    };
    for (ring, plane) in strip {
        let (ring, plane) = if keep_winding {
            (ring, plane)
        } else {
            (ring.into_iter().rev().collect(), plane.flipped())
        };
        add.push(PatchFace {
            outer: ring,
            holes: Vec::new(),
            attrs: crate::topo::FaceAttrs {
                attrs: crate::topo::AttrSlot::NONE,
                material,
                back_material,
                uv_frame: None,
                surface: None,
                blend: Some(BlendTag {
                    id: blend,
                    kind,
                    role: BlendRole::Strip,
                }),
            },
            plane: Some(plane),
        });
    }
    Ok(())
}

/// One side's cut: the rail segments by the face each lies in, and the set
/// of them.
type SideCut = (
    BTreeMap<FaceId, Vec<(usize, usize)>>,
    BTreeSet<(usize, usize)>,
);

/// A piece of a side face: its loops as patch points.
#[derive(Debug, Clone)]
struct Piece {
    outer: Vec<usize>,
    holes: Vec<Vec<usize>>,
}

impl Piece {
    fn segments(&self) -> Vec<(usize, usize)> {
        std::iter::once(&self.outer)
            .chain(&self.holes)
            .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
            .collect()
    }
}

/// Cuts every face of the given sides the rail crosses (and, for an open
/// chain, the end faces in `by_face`) and removes what lies between the
/// rail and the chain; the end faces in `grow` take their end section into
/// their ring instead (a concave chain's blend fills the corner there).
#[allow(clippy::too_many_arguments)]
fn cut_and_remove(
    obj: &Object,
    chain: &Chain,
    sides: &[usize],
    by_face: &BTreeMap<FaceId, Vec<(usize, usize)>>,
    rail_segs: &BTreeSet<(usize, usize)>,
    grow: &[(FaceId, usize, Vec<usize>)],
    near_chain: &dyn Fn(Point3) -> bool,
    surface: bool,
    pts: &mut Pts,
    out: &mut Out,
    add: &mut Vec<PatchFace>,
) -> Result<Vec<usize>, PlanFail> {
    let at0 = obj.vertices()[chain.verts[0]].position;
    // Point lookup by position: the faces' own vertices and the rail points.
    let lookup = |pts: &mut Pts, cands: &[usize], p: Point3| -> Option<usize> {
        cands
            .iter()
            .copied()
            .map(|i| (i, (pts.pos(i) - p).length()))
            .filter(|&(_, d)| d <= tol::SWEPT_SLACK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let rail_pts: Vec<usize> = rail_segs
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    // Pieces of each face: cut faces by arrangement; others whole.
    let mut pieces_of: BTreeMap<FaceId, Vec<Piece>> = BTreeMap::new();
    // A face's rings as patch points, with the rail points on its edges
    // split in, and every point of them: one group of rings (outer first)
    // per piece. A face an earlier chain replaced is its kept pieces.
    let split_rings = |f: FaceId,
                       pts: &mut Pts,
                       kept: &[PatchFace],
                       out: &Out|
     -> (Vec<Vec<Vec<usize>>>, Vec<usize>) {
        let mut verts: Vec<usize> = Vec::new();
        let groups: Vec<Vec<Vec<usize>>> = match out.swept_faces.get(&f) {
            Some(slots) => slots
                .iter()
                .map(|&i| {
                    let p = &kept[i];
                    let rings: Vec<Vec<usize>> =
                        std::iter::once(&p.outer).chain(&p.holes).cloned().collect();
                    verts.extend(rings.iter().flatten().copied());
                    rings
                })
                .collect(),
            None => vec![
                obj.face_loops(f)
                    .into_iter()
                    .map(|l| {
                        obj.loop_half_edges(l)
                            .map(|h| {
                                let i = pts.existing(obj, obj.half_edges()[h].origin);
                                verts.push(i);
                                i
                            })
                            .collect()
                    })
                    .collect(),
            ],
        };
        // Rail points on this face's edges split them here too, so a face
        // and its neighbor agree on every shared segment (WP07 §4.4 step 1).
        let slack = tol::SWEPT_SLACK;
        let mut split = |r: Vec<usize>| -> Vec<usize> {
            {
                let mut out = Vec::with_capacity(r.len());
                for i in 0..r.len() {
                    let (a, b) = (r[i], r[(i + 1) % r.len()]);
                    out.push(a);
                    let (pa, pb) = (pts.pos(a), pts.pos(b));
                    let d = pb - pa;
                    let l2 = d.dot(d);
                    if l2 <= 0.0 {
                        continue;
                    }
                    let mut on: Vec<(f64, usize)> = rail_pts
                        .iter()
                        .copied()
                        .filter(|&p| p != a && p != b)
                        .filter_map(|p| {
                            let pp = pts.pos(p);
                            let t = (pp - pa).dot(d) / l2;
                            (t > 0.0
                                && t < 1.0
                                && super::query::point_segment_distance(pp, pa, pb) <= slack
                                && (pp - pa).length() > slack
                                && (pp - pb).length() > slack)
                                .then_some((t, p))
                        })
                        .collect();
                    on.sort_by(|x, y| x.0.total_cmp(&y.0));
                    for (t, p) in on {
                        // A rail point on this edge lies on it exactly: the
                        // station solve leaves it a few nanometers off,
                        // which would lift it off both faces' planes.
                        pts.snap(p, pa + d * t);
                        out.push(p);
                        verts.push(p);
                    }
                }
                out
            }
        };
        let groups: Vec<Vec<Vec<usize>>> = groups
            .into_iter()
            .map(|g| g.into_iter().map(&mut split).collect())
            .collect();
        (groups, verts)
    };
    // Whether `p` lies in one of the pieces' polygons.
    let in_groups = |pts: &Pts, plane: &Plane, groups: &[Vec<Vec<usize>>], p: Point3| {
        groups.iter().any(|g| {
            let rings: Vec<Vec<[f64; 2]>> = g
                .iter()
                .map(|r| r.iter().map(|&i| to2(plane, pts.pos(i))).collect())
                .collect();
            in_rings(plane, &rings, p)
        })
    };
    let piece_for = |pieces_of: &mut BTreeMap<FaceId, Vec<Piece>>,
                     f: FaceId,
                     pts: &mut Pts,
                     add: &[PatchFace],
                     out: &Out|
     -> Result<Vec<Piece>, PlanFail> {
        if let Some(p) = pieces_of.get(&f) {
            return Ok(p.clone());
        }
        let face = &obj.faces()[f];
        let (groups, verts) = split_rings(f, pts, add, out);
        let replaced = out.swept_faces.contains_key(&f);
        let rings: Vec<Vec<usize>> = groups.iter().flatten().cloned().collect();
        let pieces = match by_face.get(&f) {
            None => groups
                .iter()
                .map(|g| Piece {
                    outer: g[0].clone(),
                    holes: g[1..].to_vec(),
                })
                .collect(),
            Some(segs) => {
                let mut sk = Sketch::on_plane(face.plane);
                // A scan's cut facet holds its points to the import
                // tolerance, not to the plane: the arrangement takes them
                // onto it (the pieces map back to the points themselves).
                let flat = |p: Point3| {
                    let d = face.plane.signed_distance(p);
                    if d.abs() <= tol::PLANE_DIST {
                        p
                    } else {
                        p - face.plane.normal() * d
                    }
                };
                // Against the surfaces (a scan: thousands of rail segments
                // across one big face) the segments go in as one batch.
                let mut batch: Vec<(Point3, Point3)> = Vec::new();
                for r in &rings {
                    for i in 0..r.len() {
                        let (a, b) = (pts.pos(r[i]), pts.pos(r[(i + 1) % r.len()]));
                        if a.approx_eq(b, tol::POINT_MERGE) {
                            // A rail point on a corner of this face.
                            continue;
                        }
                        if surface {
                            batch.push((flat(a), flat(b)));
                        } else {
                            sk.add_segment(flat(a), flat(b))
                                .map_err(|_| unsupported(at0))?;
                        }
                    }
                }
                let mut cands = verts.clone();
                for &(a, b) in segs {
                    if pts.pos(a).approx_eq(pts.pos(b), tol::POINT_MERGE) {
                        continue;
                    }
                    if surface {
                        batch.push((flat(pts.pos(a)), flat(pts.pos(b))));
                    } else {
                        sk.add_segment(flat(pts.pos(a)), flat(pts.pos(b)))
                            .map_err(|_| unsupported(pts.pos(a)))?;
                    }
                    cands.push(a);
                    cands.push(b);
                }
                if surface {
                    sk.add_segments(&batch).map_err(|_| unsupported(at0))?;
                }
                let mut out_pieces = Vec::new();
                for rid in sk.regions().keys() {
                    let prof = sk.profile(rid).map_err(|_| unsupported(at0))?;
                    // Regions inside the face's holes are not the face (nor
                    // those outside the pieces an earlier chain left).
                    let inside = |p: Point3| {
                        if replaced {
                            in_groups(pts, &face.plane, &groups, p)
                        } else {
                            in_face(obj, f, p)
                        }
                    };
                    if !interior_point(prof.outer()).is_some_and(inside) {
                        continue;
                    }
                    let map = |pts: &mut Pts, ring: &[Point3]| -> Result<Vec<usize>, PlanFail> {
                        ring.iter()
                            .map(|&p| lookup(pts, &cands, p).ok_or_else(|| unsupported(p)))
                            .collect()
                    };
                    // Arrangement vertices within the lookup slack of one
                    // point are that point (f32-rounded imports split edges
                    // a hair from a rail point); a sliver that collapses
                    // to under three points has no area and goes.
                    let tidy = |mut r: Vec<usize>| {
                        r.dedup();
                        while r.len() > 1 && r.first() == r.last() {
                            r.pop();
                        }
                        r
                    };
                    let outer = tidy(map(pts, prof.outer())?);
                    if outer.len() < 3 {
                        continue;
                    }
                    let holes = prof
                        .holes()
                        .iter()
                        .map(|h| map(pts, h).map(tidy))
                        .filter(|h| h.as_ref().map_or(true, |h| h.len() >= 3))
                        .collect::<Result<Vec<_>, _>>()?;
                    out_pieces.push(Piece { outer, holes });
                }
                out_pieces
            }
        };
        pieces_of.insert(f, pieces.clone());
        Ok(pieces)
    };
    // An end face a concave chain's blend fills out to: its ring takes the
    // end section in place of its corner at the chain's end vertex (the
    // two edges from there to the rails become the section), and is not
    // flooded.
    for (f, v, row) in grow {
        let at = pts.pos(*v);
        let mut pieces: Vec<Piece> = match pieces_of.remove(f) {
            Some(pieces) => pieces,
            None => split_rings(*f, pts, add, out)
                .0
                .into_iter()
                .map(|g| Piece {
                    outer: g[0].clone(),
                    holes: g[1..].to_vec(),
                })
                .collect(),
        };
        let (a0, an) = (row[0], row[row.len() - 1]);
        let mut spliced = false;
        'pieces: for piece in pieces.iter_mut() {
            for ring in std::iter::once(&mut piece.outer).chain(piece.holes.iter_mut()) {
                let Some(i) = ring.iter().position(|&p| p == *v) else {
                    continue;
                };
                let k = ring.len();
                let (prev, next) = (ring[(i + k - 1) % k], ring[(i + 1) % k]);
                let inner: Vec<usize> = if prev == a0 && next == an {
                    row[1..row.len() - 1].to_vec()
                } else if prev == an && next == a0 {
                    row[1..row.len() - 1].iter().rev().copied().collect()
                } else {
                    return Err(unsupported(at));
                };
                ring.splice(i..i + 1, inner);
                spliced = true;
                break 'pieces;
            }
        }
        if !spliced {
            return Err(unsupported(at));
        }
        pieces_of.insert(*f, pieces);
    }
    // The chain's own segments on this side never get crossed.
    let chain_segs: BTreeSet<(usize, usize)> = chain
        .edges
        .iter()
        .filter_map(|e| obj.edge_vertex_ids(e.edge))
        .map(|(a, b)| {
            let (i, j) = (pts.existing(obj, a), pts.existing(obj, b));
            (i.min(j), i.max(j))
        })
        .collect();
    let side_face = |e: &super::classify::EdgeInfo, x: usize| if x == 0 { e.a } else { e.b };
    // The faces of the sides not flooded here: a flood that reaches one
    // has leaked past the chain.
    let other_faces: BTreeSet<FaceId> = chain
        .edges
        .iter()
        .flat_map(|e| [(0, e.a), (1, e.b)])
        .filter(|(x, _)| !sides.contains(x))
        .map(|(_, f)| f)
        .collect();
    // Flood fill from the pieces holding a chain edge.
    let mut removed: Vec<Piece> = Vec::new();
    let mut seen: BTreeSet<(FaceId, usize)> = BTreeSet::new();
    let mut queue: Vec<(FaceId, (usize, usize))> = Vec::new();
    for e in &chain.edges {
        let (a, b) = obj
            .edge_vertex_ids(e.edge)
            .ok_or_else(|| unsupported(at0))?;
        let (i, j) = (pts.existing(obj, a), pts.existing(obj, b));
        for &x in sides {
            queue.push((side_face(e, x), (i, j)));
        }
    }
    let undirected = |(a, b): (usize, usize)| (a.min(b), a.max(b));
    while let Some((f, seg)) = queue.pop() {
        if other_faces.contains(&f) {
            return Err(unsupported(pts.pos(seg.0)));
        }
        let pieces = piece_for(&mut pieces_of, f, pts, add, out)?;
        let Some((pi, piece)) = pieces.iter().enumerate().find(|(_, p)| {
            p.segments()
                .iter()
                .any(|&s| undirected(s) == undirected(seg))
        }) else {
            if surface && by_face.contains_key(&f) {
                // The rail cut off a sliver of this face too small to keep
                // (its points within the lookup slack of one another).
                continue;
            }
            return Err(unsupported(pts.pos(seg.0)));
        };
        if !seen.insert((f, pi)) {
            continue;
        }
        for s in piece.segments() {
            if !near_chain(pts.pos(s.0)) {
                // The rail did not close the region off: a leak.
                return Err(unsupported(pts.pos(s.0)));
            }
            let u = undirected(s);
            let along_rail = || {
                rail_segs.iter().any(|&(ra, rb)| {
                    let (pa, pb) = (pts.pos(ra), pts.pos(rb));
                    [s.0, s.1].iter().all(|&k| {
                        super::query::point_segment_distance(pts.pos(k), pa, pb) <= tol::SWEPT_SLACK
                    })
                })
            };
            if rail_segs.contains(&u) || chain_segs.contains(&u) || along_rail() {
                continue;
            }
            // The face across the original edge this segment lies on.
            let Some(g) = across(obj, f, pts.pos(s.0), pts.pos(s.1)) else {
                return Err(unsupported(pts.pos(s.0)));
            };
            queue.push((g, s));
        }
        removed.push(piece.clone());
    }
    // A face the flood did not reach but whose edge a rail point splits (the
    // rail touches it only at that edge, as where a junction's side meets
    // a seam) takes the split too, so both faces of the edge agree.
    let slack = tol::SWEPT_SLACK;
    loop {
        let touched: Vec<FaceId> = pieces_of.keys().copied().collect();
        let mut more: Vec<FaceId> = Vec::new();
        for f in touched {
            for l in obj.face_loops(f) {
                for h in obj.loop_half_edges(l) {
                    let e = obj.half_edges()[h].edge;
                    let Some((p, q)) = obj.edge_endpoints(e) else {
                        continue;
                    };
                    let splits = rail_pts.iter().any(|&r| {
                        let rp = pts.pos(r);
                        super::query::point_segment_distance(rp, p, q) <= slack
                            && (rp - p).length() > slack
                            && (rp - q).length() > slack
                    });
                    if !splits {
                        continue;
                    }
                    if let Some((a, b)) = obj.edge_faces(e) {
                        let g = if a == f { b } else { a };
                        if !pieces_of.contains_key(&g) && !more.contains(&g) {
                            more.push(g);
                        }
                    }
                }
            }
        }
        if more.is_empty() {
            break;
        }
        for g in more {
            piece_for(&mut pieces_of, g, pts, add, out)?;
        }
    }
    // Replace every cut face by its kept pieces; drop every removed face. A
    // face an earlier chain replaced has its pieces' places in the kept
    // list already: the new pieces take them (a place left over is emptied
    // and dropped by the planner), and more go on the end.
    let removed_ids: BTreeSet<(FaceId, usize)> = seen;
    let mut touched: Vec<usize> = Vec::new();
    for (f, pieces) in &pieces_of {
        out.removed.insert(*f);
        let face = &obj.faces()[*f];
        let mut slots: Vec<usize> = out.swept_faces.remove(f).unwrap_or_default();
        let mut filled = 0;
        for (pi, p) in pieces.iter().enumerate() {
            if removed_ids.contains(&(*f, pi)) {
                continue;
            }
            // Orient as the parent face.
            let fix = |r: &Vec<usize>, hole: bool| {
                let rp: Vec<Point3> = r.iter().map(|&i| pts.pos(i)).collect();
                let n = Plane::from_polygon(&rp)
                    .map(|q| q.normal())
                    .unwrap_or(face.plane.normal());
                let ccw = n.dot(face.plane.normal()) > 0.0;
                if ccw != hole {
                    r.clone()
                } else {
                    r.iter().rev().copied().collect()
                }
            };
            let pf = PatchFace {
                outer: fix(&p.outer, false),
                holes: p.holes.iter().map(|h| fix(h, true)).collect(),
                attrs: face.attrs(),
                plane: Some(face.plane),
            };
            let at = if filled < slots.len() {
                add[slots[filled]] = pf;
                slots[filled]
            } else {
                add.push(pf);
                slots.push(add.len() - 1);
                add.len() - 1
            };
            filled += 1;
            touched.push(at);
            // Edges split at rail points keep the original edge's metadata.
            for (a, b) in p.segments() {
                if let Some(e) = original_edge(obj, *f, pts.pos(a), pts.pos(b)) {
                    let ed = &obj.edges()[e];
                    if ed.soft || ed.crease || ed.curve.is_some() {
                        out.meta(
                            a,
                            b,
                            EdgeMeta {
                                soft: ed.soft,
                                crease: ed.crease,
                                curve: ed.curve,
                                polygon: ed.polygon,
                            },
                        );
                    }
                }
            }
        }
        for &slot in &slots[filled..] {
            add[slot].outer.clear();
            add[slot].holes.clear();
        }
        slots.truncate(filled);
        out.swept_faces.insert(*f, slots);
    }
    let _ = removed.len();
    Ok(touched)
}

/// The boundary edge of `f` whose segment contains both `a` and `b`.
fn original_edge(obj: &Object, f: FaceId, a: Point3, b: Point3) -> Option<EdgeId> {
    // The nearest such edge: around a sliver several are within the slack.
    obj.face_loops(f)
        .into_iter()
        .flat_map(|l| obj.loop_half_edges(l).collect::<Vec<_>>())
        .filter_map(|h| {
            let e = obj.half_edges()[h].edge;
            let (p, q) = obj.edge_endpoints(e)?;
            let off = super::query::point_segment_distance(a, p, q)
                .max(super::query::point_segment_distance(b, p, q));
            (off <= tol::SWEPT_SLACK).then_some((off, e))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, e)| e)
}

/// The face across the boundary edge of `f` that contains segment `a–b`.
fn across(obj: &Object, f: FaceId, a: Point3, b: Point3) -> Option<FaceId> {
    let e = original_edge(obj, f, a, b)?;
    let (x, y) = obj.edge_faces(e)?;
    Some(if x == f { y } else { x })
}

/// A point strictly inside a simple polygon: the centroid of an ear.
fn interior_point(ring: &[Point3]) -> Option<Point3> {
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let normal = Plane::from_polygon(ring).ok()?.normal();
    for i in 0..n {
        let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        if (b - a).cross(c - b).dot(normal) <= 0.0 {
            continue;
        }
        let g = Point3::new(
            (a.x + b.x + c.x) / 3.0,
            (a.y + b.y + c.y) / 3.0,
            (a.z + b.z + c.z) / 3.0,
        );
        let inside_tri = |p: Point3| {
            let s1 = (b - a).cross(p - a).dot(normal);
            let s2 = (c - b).cross(p - b).dot(normal);
            let s3 = (a - c).cross(p - c).dot(normal);
            s1 > 0.0 && s2 > 0.0 && s3 > 0.0
        };
        if ring
            .iter()
            .enumerate()
            .filter(|(j, _)| ![(i + n - 1) % n, i, (i + 1) % n].contains(j))
            .all(|(_, &p)| !inside_tri(p))
        {
            return Some(g);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::{BlendProfile, BlendSpec, classify};
    use crate::CurveGeom;
    use crate::loft::{LoftProfile, LoftSpec};
    use crate::math::Point3;
    use crate::topo::Object;
    use std::f64::consts::TAU;

    #[test]
    fn a_closed_loop_closes_on_its_first_station() {
        // Rolled as one swept loop, a lofted deck rim's last stations land
        // on the first: their points are the first's, and a repeat of the
        // first station goes, so the closing quads keep their area.
        let ring = |r: f64, z: f64| LoftProfile {
            points: (0..24)
                .map(|k| {
                    let a = TAU * k as f64 / 24.0;
                    Point3::new(r * a.cos(), r * a.sin(), z)
                })
                .collect(),
            curves: vec![
                Some(CurveGeom {
                    center: Point3::new(0.0, 0.0, z),
                    radius: r,
                });
                24
            ],
            smooth: Vec::new(),
            through: Vec::new(),
            start_offset: 0,
        };
        let hull = Object::from_loft(&LoftSpec {
            profiles: vec![ring(0.35, 0.0), ring(0.58, 0.5), ring(0.6, 1.0)],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap();
        let deck: Vec<_> = hull
            .sharp_edges()
            .into_iter()
            .filter(|&e| {
                let (p, q) = hull.edge_endpoints(e).unwrap();
                (p.z - 1.0).abs() < 1e-9 && (q.z - 1.0).abs() < 1e-9
            })
            .collect();
        let spec = BlendSpec {
            edges: deck,
            profile: BlendProfile::Round {
                radius: 0.03,
                segments: 4,
            },
            primary: None,
            corners: Default::default(),
            stops: None,
            handles: Vec::new(),
        };
        let (after, _, _) = classify::with_forced_swept(|| hull.blend_once(&spec)).unwrap();
        after.validate().unwrap();
        // The closing quads keep real sides: no edge of the result is
        // shorter than the merge tolerance.
        for e in after.edges().keys() {
            let (p, q) = after.edge_endpoints(e).unwrap();
            assert!(
                (q - p).length() > crate::tol::POINT_MERGE,
                "zero-length edge at {p:?}"
            );
        }
    }
}
