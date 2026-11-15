//! The 2D section solver (WP05 §5, WP06 §1, decision I-9): the
//! cross-section of a blend at one station, in the station frame `x = tA`,
//! `y ⟂ x` toward `tB`, with the edge point at the origin. Each side is a
//! trace: a polyline leaving the edge point along that side's faces (a
//! single ray for a flat side; the seam polyline of a band; the facet
//! perpendiculars of a faceted cylinder).

use super::BlendProfile;
use crate::math::Vec3;
use crate::tol;

/// One side's cross-section as 3D directions in the section plane, from the
/// edge point outward: segment `i` runs `dirs[i]·lens[i]`. The last length
/// may be `f64::INFINITY`.
#[derive(Debug, Clone)]
pub(crate) struct Trace {
    pub dirs: Vec<Vec3>,
    pub lens: Vec<f64>,
}

impl Trace {
    /// A single unbounded ray.
    pub(crate) fn ray(dir: Vec3) -> Trace {
        Trace {
            dirs: vec![dir],
            lens: vec![f64::INFINITY],
        }
    }
}

/// One station's section: points `P_0` (on trace A) … `P_N` (on trace B)
/// as 3D offsets from the edge point, the arc center offset and radius of a
/// round, and the trace segment each rail lands on.
#[derive(Debug, Clone)]
pub(crate) struct Section {
    pub offsets: Vec<Vec3>,
    pub arc_center: Option<Vec3>,
    pub radius: Option<f64>,
    pub seg_a: usize,
    pub seg_b: usize,
    /// Per facet `k`: the arc it chords (center offset, radius), or `None`
    /// for a flat facet.
    pub facet_arcs: Vec<Option<(Vec3, f64)>>,
    /// Per ruling `k` (`0..=N`): whether the two facets meeting there meet
    /// smoothly (same arc, or a tangent junction). The rails are `false`.
    pub soft: Vec<bool>,
    /// Whether the section is a round, whose facets face away from its arc
    /// center over a convex edge.
    pub round: bool,
}

/// The profile does not fit on the traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TraceEnd;

type P2 = [f64; 2];

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}
fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn d2(a: P2, b: P2) -> f64 {
    len(sub(a, b))
}
fn len(a: P2) -> f64 {
    dot(a, a).sqrt()
}
fn perp(a: P2) -> P2 {
    [-a[1], a[0]]
}

/// A trace in 2D: its vertices, unit directions, lengths and inward normals.
struct T2 {
    verts: Vec<P2>,
    dirs: Vec<P2>,
    lens: Vec<f64>,
    normals: Vec<P2>,
}

impl T2 {
    fn seg_distance(&self, i: usize, p: P2) -> f64 {
        let a = self.verts[i];
        let l = self.lens[i];
        let s = dot(sub(p, a), self.dirs[i]);
        let s = if l.is_finite() {
            s.clamp(0.0, l)
        } else {
            s.max(0.0)
        };
        len(sub(p, add(a, mul(self.dirs[i], s))))
    }
    /// The foot of `p` on segment `i`, if it lies on the segment.
    fn foot(&self, i: usize, p: P2) -> Option<P2> {
        let s = dot(sub(p, self.verts[i]), self.dirs[i]);
        (s >= -tol::POINT_MERGE && s <= self.lens[i] + tol::POINT_MERGE)
            .then(|| add(self.verts[i], mul(self.dirs[i], s.clamp(0.0, self.lens[i]))))
    }
    /// The point at arc length `d` along the trace, and its segment.
    fn at_length(&self, d: f64) -> Option<(P2, usize)> {
        let mut rest = d;
        for i in 0..self.dirs.len() {
            if rest <= self.lens[i] + tol::POINT_MERGE {
                return Some((add(self.verts[i], mul(self.dirs[i], rest)), i));
            }
            rest -= self.lens[i];
        }
        None
    }
}

/// Solves the section for `profile` between traces `a` and `b` (their first
/// directions fix the 2D frame).
pub(crate) fn solve_section(
    profile: &BlendProfile,
    a: &Trace,
    b: &Trace,
) -> Result<Section, TraceEnd> {
    let x = a.dirs[0];
    let y = (b.dirs[0] - x * b.dirs[0].dot(x))
        .normalized()
        .map_err(|_| TraceEnd)?;
    let to2 = |v: Vec3| -> P2 { [v.dot(x), v.dot(y)] };
    let to3 = |p: P2| x * p[0] + y * p[1];
    let mk = |t: &Trace, sign: f64| -> T2 {
        let mut verts = vec![[0.0, 0.0]];
        let mut dirs = Vec::new();
        let mut normals = Vec::new();
        for (i, d) in t.dirs.iter().enumerate() {
            let d2 = to2(*d);
            let d2 = mul(d2, 1.0 / len(d2));
            dirs.push(d2);
            normals.push(mul(perp(d2), sign));
            if t.lens[i].is_finite() {
                let last = *verts.last().expect("a trace has a start");
                verts.push(add(last, mul(d2, t.lens[i])));
            }
        }
        T2 {
            verts,
            dirs,
            lens: t.lens.clone(),
            normals,
        }
    };
    // Trace A starts along +x with B above it, so its inside is +perp;
    // trace B's inside (toward A) is −perp of its own direction.
    let ta = mk(a, 1.0);
    let tb = mk(b, -1.0);
    match *profile {
        BlendProfile::Round { radius, segments } => {
            let r = radius;
            let n = segments as usize;
            let (c, p0, pn, sa, sb) = round_contact(&ta, &tb, r).ok_or(TraceEnd)?;
            let u = mul(sub(p0, c), 1.0 / r);
            let v = mul(sub(pn, c), 1.0 / r);
            let cosphi = dot(u, v).clamp(-1.0, 1.0);
            let phi = cosphi.acos();
            let w = sub(v, mul(u, cosphi));
            let wl = len(w);
            let w = if wl > tol::NORMALIZE_MIN_LENGTH {
                mul(w, 1.0 / wl)
            } else {
                perp(u)
            };
            let mut offsets = Vec::with_capacity(n + 1);
            offsets.push(to3(p0));
            for k in 1..n {
                let ang = phi * k as f64 / n as f64;
                offsets.push(to3(add(
                    c,
                    add(mul(u, r * ang.cos()), mul(w, r * ang.sin())),
                )));
            }
            offsets.push(to3(pn));
            // Rails on the first segments lie exactly on the trace
            // directions (WP05: never computed from the arc).
            if sa == 0 {
                offsets[0] = a.dirs[0] * len(p0);
            }
            if sb == 0 {
                offsets[n] = b.dirs[0] * len(pn);
            }
            Ok(Section {
                offsets,
                arc_center: Some(to3(c)),
                radius: Some(r),
                seg_a: sa,
                seg_b: sb,
                facet_arcs: vec![Some((to3(c), r)); n],
                soft: (0..=n).map(|k| k > 0 && k < n).collect(),
                round: true,
            })
        }
        BlendProfile::Chamfer {
            setback_a,
            setback_b,
        } => {
            let (p0, sa) = ta.at_length(setback_a).ok_or(TraceEnd)?;
            let (pn, sb) = tb.at_length(setback_b).ok_or(TraceEnd)?;
            let mut offsets = vec![to3(p0), to3(pn)];
            if sa == 0 {
                offsets[0] = a.dirs[0] * setback_a;
            }
            if sb == 0 {
                offsets[1] = b.dirs[0] * setback_b;
            }
            Ok(Section {
                offsets,
                arc_center: None,
                radius: None,
                seg_a: sa,
                seg_b: sb,
                facet_arcs: vec![None],
                soft: vec![false, false],
                round: false,
            })
        }
        BlendProfile::Cove { radius, segments } => {
            // A quarter-round cut about the corner itself (convex edges only,
            // checked by classification). Within the first facet of each
            // side it is a true arc; where a side bends at a seam inside the
            // radius (a swept chain's facets) the rails sit at the radius
            // along the side and the arc runs between the two chords.
            let (r, n) = (radius, segments as usize);
            let (p0, sa) = ta.at_length(r).ok_or(TraceEnd)?;
            let (pn, sb) = tb.at_length(r).ok_or(TraceEnd)?;
            let flat = sa == 0 && sb == 0;
            let mut offsets: Vec<Vec3> = if flat {
                let theta = dot(to2(b.dirs[0]), [1.0, 0.0]).clamp(-1.0, 1.0).acos();
                (0..=n)
                    .map(|k| {
                        let ang = theta * k as f64 / n as f64;
                        to3([r * ang.cos(), r * ang.sin()])
                    })
                    .collect()
            } else {
                let (va, vb) = (to3(p0), to3(pn));
                let (ra, rb) = (va.length(), vb.length());
                let ua = va.normalized().map_err(|_| TraceEnd)?;
                let ub = vb.normalized().map_err(|_| TraceEnd)?;
                let cos_theta = ua.dot(ub).clamp(-1.0, 1.0);
                let theta = cos_theta.acos();
                let w = (ub - ua * cos_theta).normalized().map_err(|_| TraceEnd)?;
                (0..=n)
                    .map(|k| {
                        let s = k as f64 / n as f64;
                        let ang = theta * s;
                        (ua * ang.cos() + w * ang.sin()) * (ra + (rb - ra) * s)
                    })
                    .collect()
            };
            offsets[0] = if sa == 0 { a.dirs[0] * r } else { to3(p0) };
            offsets[n] = if sb == 0 { b.dirs[0] * r } else { to3(pn) };
            Ok(Section {
                offsets,
                // Over a bend the points lie on no one arc about the corner.
                arc_center: flat.then_some(Vec3::new(0.0, 0.0, 0.0)),
                radius: Some(r),
                seg_a: sa,
                seg_b: sb,
                facet_arcs: vec![flat.then_some((Vec3::new(0.0, 0.0, 0.0), r)); n],
                soft: (0..=n).map(|k| k > 0 && k < n).collect(),
                round: false,
            })
        }
        BlendProfile::Custom {
            ref shape,
            width,
            arc_segments,
        } => {
            // Registered by the affine map (u, v) → u·W·tA + v·W·tB.
            // Where a side bends at a seam inside the profile's width (a
            // swept chain's facets), the map runs along the chords to the
            // rails instead, so the strip still lands on the sides.
            let pts = shape.sample(arc_segments).map_err(|_| TraceEnd)?;
            let (p0, sa) = ta.at_length(width).ok_or(TraceEnd)?;
            let (pn, sb) = tb.at_length(shape.b * width).ok_or(TraceEnd)?;
            let flat = sa == 0 && sb == 0;
            let (va, vb) = (to3(p0), to3(pn) * (1.0 / shape.b));
            let map = |p: [f64; 2]| {
                if flat {
                    a.dirs[0] * (p[0] * width) + b.dirs[0] * (p[1] * width)
                } else {
                    va * p[0] + vb * p[1]
                }
            };
            let offsets: Vec<Vec3> = pts.iter().map(|(p, _)| map(*p)).collect();
            let n = offsets.len() - 1;
            // Arc facets claim only where the map is a similarity (θ = 90°).
            let square = flat && (dot(to2(b.dirs[0]), [1.0, 0.0])).abs() <= tol::NORMAL_DIRECTION;
            let facet_arcs: Vec<Option<(Vec3, f64)>> = pts[1..]
                .iter()
                .map(|(_, h)| h.filter(|_| square).map(|(c, r)| (map(c), r * width)))
                .collect();
            let dir = |k: usize| {
                let (p, q) = (pts[k].0, pts[k + 1].0);
                let l = d2(p, q);
                [(q[0] - p[0]) / l, (q[1] - p[1]) / l]
            };
            let soft = (0..=n)
                .map(|k| {
                    k > 0
                        && k < n
                        && (pts[k].1.is_some() && pts[k].1 == pts[k + 1].1
                            || dot(dir(k - 1), dir(k)) >= 1.0 - tol::NORMAL_DIRECTION)
                })
                .collect();
            Ok(Section {
                offsets,
                arc_center: None,
                radius: None,
                seg_a: sa,
                seg_b: sb,
                facet_arcs,
                soft,
                round: false,
            })
        }
        BlendProfile::Bullnose { .. } => Err(TraceEnd),
    }
}

/// The first rolling-ball contact in the WP06 §1 candidate order: the
/// center, the two rail points and the segments they land on.
fn round_contact(a: &T2, b: &T2, r: f64) -> Option<(P2, P2, P2, usize, usize)> {
    let (na, nb) = (a.dirs.len(), b.dirs.len());
    let clear = |c: P2, i: usize, j: usize| {
        (0..=i).all(|k| a.seg_distance(k, c) >= r - tol::PLANE_DIST)
            && (0..=j).all(|k| b.seg_distance(k, c) >= r - tol::PLANE_DIST)
    };
    for total in 0..=(na + nb - 2) {
        for i in 0..=total {
            let j = total - i;
            if i >= na || j >= nb {
                continue;
            }
            // (a) tangent to line i of A and line j of B.
            if let Some(c) = offset_intersection(a, i, b, j, r)
                && let (Some(fa), Some(fb)) = (a.foot(i, c), b.foot(j, c))
                && clear(c, i, j)
            {
                return Some((c, fa, fb, i, j));
            }
            // (b) tangent to line j of B, through vertex a_i.
            if i >= 1
                && let Some(c) = through_vertex(b, j, a.verts[i], r)
                && let Some(fb) = b.foot(j, c)
                && clear(c, i - 1, j)
            {
                return Some((c, a.verts[i], fb, i - 1, j));
            }
            // (c) tangent to line i of A, through vertex b_j.
            if j >= 1
                && let Some(c) = through_vertex(a, i, b.verts[j], r)
                && let Some(fa) = a.foot(i, c)
                && clear(c, i, j - 1)
            {
                return Some((c, fa, b.verts[j], i, j - 1));
            }
        }
    }
    None
}

/// The intersection of line `i` of A and line `j` of B, each offset by `r`
/// toward the corner's inside.
fn offset_intersection(a: &T2, i: usize, b: &T2, j: usize, r: f64) -> Option<P2> {
    let (n1, n2) = (a.normals[i], b.normals[j]);
    let (c1, c2) = (dot(n1, a.verts[i]) + r, dot(n2, b.verts[j]) + r);
    let det = n1[0] * n2[1] - n1[1] * n2[0];
    if det.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    Some([
        (c1 * n2[1] - c2 * n1[1]) / det,
        (n1[0] * c2 - n2[0] * c1) / det,
    ])
}

/// A center on line `i`'s offset of trace `t` at distance `r` from `p`: the
/// root nearer the corner (the origin).
fn through_vertex(t: &T2, i: usize, p: P2, r: f64) -> Option<P2> {
    let base = add(t.verts[i], mul(t.normals[i], r));
    let d = t.dirs[i];
    let w = sub(base, p);
    let bq = dot(w, d);
    let disc = bq * bq - (dot(w, w) - r * r);
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    [add(base, mul(d, -bq - sq)), add(base, mul(d, -bq + sq))]
        .into_iter()
        .min_by(|x, y| len(*x).total_cmp(&len(*y)))
}
