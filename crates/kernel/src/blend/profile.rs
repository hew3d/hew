//! Custom router profiles (WP07 §3, decisions I-15 and I-16): validation,
//! sampling into facets, building a shape from a drawn open chain, and the
//! bundled starter set.

use super::{BlendError, ProfileSeg, ProfileShape};
use crate::math::{Point3, Vec3};
use crate::sketch::CurveGeom;
use crate::tol;
use crate::trig;

/// One sampled point of a profile and the arc (center, radius) of the facet
/// ending at it, `None` for a line facet (and for the start point).
pub type ProfileSample = ([f64; 2], Option<([f64; 2], f64)>);

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    trig::hypot(a[0] - b[0], a[1] - b[1])
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Whether segments `ab` and `cd` intersect (touching counts).
fn segments_meet(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let eps = tol::PROFILE_SEGMENT_EPS;
    let (o1, o2, o3, o4) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    if ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
    {
        return true;
    }
    let on = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        cross(p, q, r).abs() <= eps
            && r[0] >= p[0].min(q[0]) - eps
            && r[0] <= p[0].max(q[0]) + eps
            && r[1] >= p[1].min(q[1]) - eps
            && r[1] <= p[1].max(q[1]) + eps
    };
    on(a, b, c) || on(a, b, d) || on(c, d, a) || on(c, d, b)
}

impl ProfileShape {
    /// Validates the shape (WP07 §3.2) and samples it: the polyline from
    /// `(1, 0)` to `(0, b)` with `arc_segments` facets per quarter turn of
    /// arc, each point carrying the arc of the facet that ends at it.
    ///
    /// # Errors
    /// [`BlendError::InvalidProfile`].
    pub fn sample(&self, arc_segments: u32) -> Result<Vec<ProfileSample>, BlendError> {
        let bad = BlendError::InvalidProfile;
        if self.segs.is_empty() || !(self.b.is_finite() && self.b > 0.0) || arc_segments == 0 {
            return Err(bad);
        }
        let mut out: Vec<ProfileSample> = vec![([1.0, 0.0], None)];
        let mut prev = [1.0, 0.0];
        for seg in &self.segs {
            match *seg {
                ProfileSeg::Line { to } => {
                    out.push((to, None));
                    prev = to;
                }
                ProfileSeg::Arc { to, center, ccw } => {
                    let (r0, r1) = (d2(prev, center), d2(to, center));
                    if r0 <= tol::PROFILE_ARC_MIN_RADIUS
                        || (r0 - r1).abs() > tol::PROFILE_ARC_RADIUS_REL * r0.max(r1)
                    {
                        return Err(bad);
                    }
                    let a0 = trig::atan2(prev[1] - center[1], prev[0] - center[0]);
                    let a1 = trig::atan2(to[1] - center[1], to[0] - center[0]);
                    let mut sweep = a1 - a0;
                    if ccw {
                        while sweep <= 0.0 {
                            sweep += std::f64::consts::TAU;
                        }
                    } else {
                        while sweep >= 0.0 {
                            sweep -= std::f64::consts::TAU;
                        }
                    }
                    let n = ((sweep.abs() / std::f64::consts::FRAC_PI_2) * arc_segments as f64)
                        .ceil()
                        .max(1.0) as usize;
                    for k in 1..=n {
                        let p = if k == n {
                            to
                        } else {
                            let a = a0 + sweep * k as f64 / n as f64;
                            [center[0] + r0 * trig::cos(a), center[1] + r0 * trig::sin(a)]
                        };
                        out.push((p, Some((center, r0))));
                    }
                    prev = to;
                }
            }
        }
        if d2(prev, [0.0, self.b]) > tol::PROFILE_CLOSURE {
            return Err(bad);
        }
        // Inside the unit box of the shape.
        let slack = tol::PROFILE_BOX_SLACK;
        if out.iter().any(|(p, _)| {
            p[0] < -slack || p[0] > 1.0 + slack || p[1] < -slack || p[1] > self.b + slack
        }) {
            return Err(bad);
        }
        // No zero-thickness cut along either face.
        for w in out.windows(2) {
            let (a, b) = (w[0].0, w[1].0);
            if (a[1].abs() <= slack && b[1].abs() <= slack)
                || (a[0].abs() <= slack && b[0].abs() <= slack)
            {
                return Err(bad);
            }
        }
        // Simple: no two non-adjacent segments meet.
        let n = out.len() - 1;
        for i in 0..n {
            for j in i + 2..n {
                if segments_meet(out[i].0, out[i + 1].0, out[j].0, out[j + 1].0) {
                    return Err(bad);
                }
            }
        }
        Ok(out)
    }

    /// Builds a normalized shape from a drawn open chain (I-16): `points`
    /// and per-edge `curves` in a plane with unit `normal`. The corner is
    /// `C = (end.u, start.v)` in that plane's basis; coordinates are |Δ| from
    /// `C` divided by the start's |Δu|. Returns the shape and the drawn
    /// width in meters.
    ///
    /// # Errors
    /// [`BlendError::InvalidProfile`] for a chain whose ends make that
    /// degenerate, or whose shape does not validate.
    pub fn from_chain(
        points: &[Point3],
        curves: &[Option<CurveGeom>],
        normal: Vec3,
    ) -> Result<(ProfileShape, f64), BlendError> {
        let bad = BlendError::InvalidProfile;
        if points.len() < 2 || curves.len() + 1 != points.len() {
            return Err(bad);
        }
        let (u, v) = crate::geom2d::plane_axes(normal);
        let uv = |p: Point3| [p.to_vec().dot(u), p.to_vec().dot(v)];
        let (s, e) = (uv(points[0]), uv(*points.last().expect("two points")));
        let c = [e[0], s[1]];
        let w = (s[0] - c[0]).abs();
        if w <= tol::POINT_MERGE || (e[1] - c[1]).abs() <= tol::POINT_MERGE {
            return Err(bad);
        }
        let (su, sv) = ((s[0] - c[0]).signum(), (e[1] - c[1]).signum());
        let norm = |p: [f64; 2]| [(p[0] - c[0]) * su / w, (p[1] - c[1]) * sv / w];
        // Consecutive facets of one drawn arc (equal curves) become one arc
        // segment; its sense is the first facet's turn about the center.
        let mut segs = Vec::with_capacity(curves.len());
        let mut i = 0;
        while i < curves.len() {
            match curves[i] {
                None => {
                    segs.push(ProfileSeg::Line {
                        to: norm(uv(points[i + 1])),
                    });
                    i += 1;
                }
                Some(g) => {
                    let mut j = i + 1;
                    while j < curves.len() && curves[j] == Some(g) {
                        j += 1;
                    }
                    let center = norm(uv(g.center));
                    let (p0, p1) = (norm(uv(points[i])), norm(uv(points[i + 1])));
                    segs.push(ProfileSeg::Arc {
                        to: norm(uv(points[j])),
                        center,
                        ccw: cross(center, p0, p1) > 0.0,
                    });
                    i = j;
                }
            }
        }
        let b = (e[1] - c[1]).abs() / w;
        let shape = ProfileShape { b, segs };
        shape.sample(4)?;
        Ok((shape, w))
    }
}

/// One bundled profile: its Library id, its name, and its shape.
pub type BundledProfile = (&'static str, &'static str, fn() -> ProfileShape);

/// The bundled starter set (I-15), in this order.
pub fn bundled() -> &'static [BundledProfile] {
    use ProfileSeg::{Arc, Line};
    fn ogee() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Arc {
                    to: [0.5, 0.5],
                    center: [1.0, 0.5],
                    ccw: false,
                },
                Arc {
                    to: [0.0, 1.0],
                    center: [0.0, 0.5],
                    ccw: true,
                },
            ],
        }
    }
    fn roman_ogee() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Arc {
                    to: [0.5, 0.5],
                    center: [0.5, 0.0],
                    ccw: true,
                },
                Arc {
                    to: [0.0, 1.0],
                    center: [0.5, 1.0],
                    ccw: false,
                },
            ],
        }
    }
    fn bead() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Line { to: [1.0, 0.2] },
                Arc {
                    to: [0.2, 1.0],
                    center: [1.0, 1.0],
                    ccw: false,
                },
                Line { to: [0.0, 1.0] },
            ],
        }
    }
    fn thumbnail() -> ProfileShape {
        ProfileShape {
            b: 0.5,
            segs: vec![
                Arc {
                    to: [0.2, 0.5],
                    center: [1.0, 0.89],
                    ccw: false,
                },
                Line { to: [0.0, 0.5] },
            ],
        }
    }
    fn cove_and_bead() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Arc {
                    to: [0.5, 0.5],
                    center: [0.5, 0.0],
                    ccw: true,
                },
                Line { to: [0.5, 0.6] },
                Arc {
                    to: [0.1, 1.0],
                    center: [0.5, 1.0],
                    ccw: false,
                },
                Line { to: [0.0, 1.0] },
            ],
        }
    }
    fn chamfer_and_bead() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Line { to: [0.6, 0.4] },
                Line { to: [0.6, 0.5] },
                Arc {
                    to: [0.1, 1.0],
                    center: [0.6, 1.0],
                    ccw: false,
                },
                Line { to: [0.0, 1.0] },
            ],
        }
    }
    fn cavetto() -> ProfileShape {
        ProfileShape {
            b: 1.0,
            segs: vec![
                Line { to: [1.0, 0.1] },
                Arc {
                    to: [0.1, 1.0],
                    center: [0.1, 0.1],
                    ccw: true,
                },
                Line { to: [0.0, 1.0] },
            ],
        }
    }
    &[
        ("builtin:ogee", "Ogee", ogee),
        ("builtin:roman-ogee", "Roman ogee", roman_ogee),
        ("builtin:bead", "Bead", bead),
        ("builtin:thumbnail", "Thumbnail", thumbnail),
        ("builtin:cove-and-bead", "Cove and bead", cove_and_bead),
        (
            "builtin:chamfer-and-bead",
            "Chamfer and bead",
            chamfer_and_bead,
        ),
        ("builtin:cavetto", "Cavetto", cavetto),
    ]
}
