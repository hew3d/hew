//! Per-class analytic surface math for [`SurfaceRef`] (the true-curves
//! design): normals, signed distances, similarity mapping, the stamp-time
//! "claim holds" predicate, and the validator's per-class invariants.
//!
//! Every per-class formula lives here, so no other module matches on the
//! surface classes to do geometry; consumers call these methods.

use crate::math::{Plane, Point3, Vec3};
use crate::tol;
use crate::topo::SurfaceRef;
use crate::transform::Transform;
use crate::trig;

/// Unit vector, or `None` when `v` is too short to normalize
/// ([`tol::NORMALIZE_MIN_LENGTH`]).
fn unit(v: Vec3) -> Option<Vec3> {
    v.normalized().ok()
}

/// `p − origin` split into its axial coordinate along unit `axis` and the
/// radial (perpendicular) remainder.
fn axial_radial(p: Point3, origin: Point3, axis: Vec3) -> (f64, Vec3) {
    let d = p - origin;
    let t = d.dot(axis);
    (t, d - axis * t)
}

/// `|v| − 1` within [`tol::NORMAL_DIRECTION`].
fn is_unit(v: Vec3) -> bool {
    (v.length() - 1.0).abs() <= tol::NORMAL_DIRECTION
}

impl SurfaceRef {
    /// Stable class name for queries and the Hew API:
    /// `"cylinder" | "sphere" | "torus" | "cone"`.
    pub fn kind_name(&self) -> &'static str {
        match self {
            SurfaceRef::Cylinder { .. } => "cylinder",
            SurfaceRef::Sphere { .. } => "sphere",
            SurfaceRef::Torus { .. } => "torus",
            SurfaceRef::Cone { .. } => "cone",
        }
    }

    /// Constructor helper for surfaces of revolution swept by a circle:
    /// a [`SurfaceRef::Torus`] unless `|major| ≤ POINT_MERGE`, in which case
    /// the tube degenerates to a [`SurfaceRef::Sphere`] of radius `minor`
    /// about `center` (a zero-major torus is never constructed).
    ///
    /// `axis` is normalized here. Returns `None` for non-finite input, an
    /// axis too short to normalize, `major < −POINT_MERGE`, or
    /// `minor ≤ POINT_MERGE`.
    pub fn torus_or_sphere(
        center: Point3,
        axis: Vec3,
        major: f64,
        minor: f64,
    ) -> Option<SurfaceRef> {
        let finite = [
            center.x, center.y, center.z, axis.x, axis.y, axis.z, major, minor,
        ]
        .iter()
        .all(|c| c.is_finite());
        if !finite || major < -tol::POINT_MERGE || minor <= tol::POINT_MERGE {
            return None;
        }
        if major.abs() <= tol::POINT_MERGE {
            return Some(SurfaceRef::Sphere {
                center,
                radius: minor,
            });
        }
        let axis = unit(axis)?;
        Some(SurfaceRef::Torus {
            center,
            axis,
            major_radius: major,
            minor_radius: minor,
        })
    }

    /// The unit normal of the analytic surface at `p`, pointing away from
    /// the surface's interior (away from a cylinder's or cone's axis, a
    /// sphere's center, a torus's center circle). Direction only: callers
    /// orient it against a face plane. `p` need not lie on the surface; the
    /// normal of the nearest surface point is returned.
    ///
    /// `None` where undefined: `p` on a cylinder's or cone's axis or a torus's
    /// axis, at a sphere's center, on a torus's center circle.
    pub fn normal_at(&self, p: Point3) -> Option<Vec3> {
        match *self {
            SurfaceRef::Cylinder {
                axis_point, axis, ..
            } => unit(axial_radial(p, axis_point, axis).1),
            SurfaceRef::Sphere { center, .. } => unit(p - center),
            SurfaceRef::Torus {
                center,
                axis,
                major_radius,
                ..
            } => {
                let radial = unit(axial_radial(p, center, axis).1)?;
                let ring = center + radial * major_radius;
                unit(p - ring)
            }
            SurfaceRef::Cone {
                apex,
                axis,
                half_angle,
            } => {
                let u = unit(axial_radial(p, apex, axis).1)?;
                unit(u * trig::cos(half_angle) - axis * trig::sin(half_angle))
            }
        }
    }

    /// Signed distance from `p` to the surface: positive outside (along
    /// [`SurfaceRef::normal_at`]), negative inside. For a cone, the
    /// perpendicular distance to the generator line in `p`'s meridian plane.
    /// Used by claim verification ([`SurfaceRef::holds_on`]).
    pub fn distance(&self, p: Point3) -> f64 {
        match *self {
            SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } => axial_radial(p, axis_point, axis).1.length() - radius,
            SurfaceRef::Sphere { center, radius } => (p - center).length() - radius,
            SurfaceRef::Torus {
                center,
                axis,
                major_radius,
                minor_radius,
            } => {
                let (t, radial) = axial_radial(p, center, axis);
                // Distance to the center circle: every circle point is
                // equidistant from an axis point, so the formula needs no
                // radial direction there.
                let rho = radial.length();
                ((rho - major_radius).powi(2) + t * t).sqrt() - minor_radius
            }
            SurfaceRef::Cone {
                apex,
                axis,
                half_angle,
            } => {
                let (t, radial) = axial_radial(p, apex, axis);
                radial.length() * trig::cos(half_angle) - t * trig::sin(half_angle)
            }
        }
    }

    /// The reference mapped by a similarity `t` of uniform scale `s`
    /// (map-or-drop's "map" half): points through `t.apply_point`,
    /// directions through `t.apply_vector` then normalized, lengths
    /// multiplied by `s`; a cone's half-angle is unchanged. `None` when a
    /// direction fails to normalize (the caller drops the claim).
    pub fn mapped(&self, t: &Transform, s: f64) -> Option<SurfaceRef> {
        let dir = |v: Vec3| unit(t.apply_vector(v));
        Some(match *self {
            SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } => SurfaceRef::Cylinder {
                axis_point: t.apply_point(axis_point),
                axis: dir(axis)?,
                radius: radius * s,
            },
            SurfaceRef::Sphere { center, radius } => SurfaceRef::Sphere {
                center: t.apply_point(center),
                radius: radius * s,
            },
            SurfaceRef::Torus {
                center,
                axis,
                major_radius,
                minor_radius,
            } => SurfaceRef::Torus {
                center: t.apply_point(center),
                axis: dir(axis)?,
                major_radius: major_radius * s,
                minor_radius: minor_radius * s,
            },
            SurfaceRef::Cone {
                apex,
                axis,
                half_angle,
            } => SurfaceRef::Cone {
                apex: t.apply_point(apex),
                axis: dir(axis)?,
                half_angle,
            },
        })
    }

    /// The stamp-time "claim holds" predicate: every point lies on the
    /// surface within `tol` (`|distance| ≤ tol`), and the class invariants
    /// [`validate_face_claim`] checks against a face's plane hold for
    /// `plane`. Stamping code calls this before writing a claim (the strict
    /// half of map-or-drop); the validator's own check is the weaker,
    /// one-sided [`validate_face_claim`].
    pub fn holds_on(&self, points: &[Point3], plane: &Plane, tol: f64) -> bool {
        self.plane_invariants_hold(plane, tol)
            && points.iter().all(|&p| self.distance(p).abs() <= tol)
    }

    /// The class invariants that need no loop vertices: well-formed
    /// parameters and the face plane's relation to the surface
    /// (true-curves.md §3.1; fillets-chamfers.md §6.1).
    fn plane_invariants_hold(&self, plane: &Plane, tol: f64) -> bool {
        match *self {
            SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } => {
                is_unit(axis)
                    && radius.is_finite()
                    && radius > tol::POINT_MERGE
                    && axis.dot(plane.normal()).abs() <= tol::NORMAL_DIRECTION
                    && plane.signed_distance(axis_point).abs() <= radius + tol
            }
            SurfaceRef::Sphere { center, radius } => {
                radius.is_finite()
                    && radius > tol::POINT_MERGE
                    && plane.signed_distance(center).abs() <= radius + tol
            }
            SurfaceRef::Torus {
                axis,
                major_radius,
                minor_radius,
                ..
            } => {
                is_unit(axis)
                    && major_radius.is_finite()
                    && minor_radius.is_finite()
                    && minor_radius > tol::POINT_MERGE
                    && major_radius > tol::POINT_MERGE
            }
            SurfaceRef::Cone {
                apex,
                axis,
                half_angle,
            } => {
                is_unit(axis)
                    && half_angle.is_finite()
                    && half_angle > 0.0
                    && half_angle < std::f64::consts::FRAC_PI_2
                    && plane.signed_distance(apex).abs() <= tol
            }
        }
    }
}

/// The validator's per-class invariants for a face claiming `claim`: the
/// weak necessary conditions every chord facet of that surface (and every
/// split sub-face of one) satisfies, true-curves.md §3.1 /
/// fillets-chamfers.md §6.1. `points` are the face's loop vertices (outer
/// and holes), `tol` the object's `planarity_tol`.
///
/// - Cylinder: unit axis; finite radius `> POINT_MERGE`; the plane parallel
///   to the axis; the plane within `radius + tol` of the axis; every vertex
///   within `radius + tol` of the axis line.
/// - Sphere: finite radius `> POINT_MERGE`; the plane within `radius + tol`
///   of the center; every vertex within `radius + tol` of the center.
/// - Torus: unit axis; finite radii, both `> POINT_MERGE`; every vertex
///   within `minor + σ(ρ) + tol` of the center circle, where `ρ` is the
///   vertex's distance from the axis and `σ(ρ) = ρ·(sec(π / MIN_CIRCLE_SEGMENTS) − 1)`
///   allows for a chord across the ring at the coarsest drawn density.
/// - Cone: unit axis; `0 < half_angle < π/2`; the plane through the apex
///   within `tol` (a facet between two generators contains both); every
///   vertex at axial coordinate `≥ −tol` and within `t·tan(half_angle) + tol`
///   of the axis.
pub(crate) fn validate_face_claim(
    claim: &SurfaceRef,
    plane: &Plane,
    points: &[Point3],
    tol: f64,
) -> bool {
    if !claim.plane_invariants_hold(plane, tol) {
        return false;
    }
    match *claim {
        SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        } => points
            .iter()
            .all(|&p| axial_radial(p, axis_point, axis).1.length() <= radius + tol),
        SurfaceRef::Sphere { center, radius } => points
            .iter()
            .all(|&p| (p - center).length() <= radius + tol),
        SurfaceRef::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => {
            // A chord across the ring at the coarsest drawn density sags
            // inward by ρ_v·(1 − cos α) from its endpoints' radius ρ_v; seen
            // from the chord point's own (smaller) radius ρ that is at most
            // ρ·(sec α − 1).
            let sag = 1.0
                / trig::cos(std::f64::consts::PI / crate::sketch::MIN_CIRCLE_SEGMENTS as f64)
                - 1.0;
            points.iter().all(|&p| {
                let (t, radial) = axial_radial(p, center, axis);
                let rho = radial.length();
                let delta = ((rho - major_radius).powi(2) + t * t).sqrt();
                delta <= minor_radius + rho * sag + tol
            })
        }
        SurfaceRef::Cone {
            apex,
            axis,
            half_angle,
        } => {
            let tan = trig::tan(half_angle);
            points.iter().all(|&p| {
                let (t, radial) = axial_radial(p, apex, axis);
                t >= -tol && radial.length() <= t * tan + tol
            })
        }
    }
}

/// How an edge reads visually and to the Fillet tool (true-curves.md §3.3
/// rule 3). The one smoothness predicate: the tessellator, line drawings,
/// `.skp` export and edge picking all classify through
/// [`Object::edge_smoothness`] and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeSmoothness {
    /// A real crease: drawn, flat-shaded across, pickable by Fillet.
    Hard,
    /// A facet seam of one smooth surface: both faces claim the same
    /// surface, or the edge is `soft` and is not a tangent joint.
    Seam,
    /// Two different surfaces tangent along the edge (a fillet rail, a
    /// corner patch's border, a rounded rectangle's wall-to-corner joint):
    /// the structure lines of a curved part.
    Tangent,
}

impl crate::topo::Object {
    /// Classifies `edge`; first match wins:
    ///
    /// 1. no twin (boundary) → `Hard`;
    /// 2. `crease` → `Hard`;
    /// 3. both faces claimed and the claims are `same_surface` → `Seam`;
    /// 4. at least one face claimed, and at BOTH endpoints the faces'
    ///    oriented normals agree (`|n_a − n_b| ≤ NORMAL_DIRECTION`), where an
    ///    unclaimed face's normal is its plane normal and a claimed face's is
    ///    its [`SurfaceRef::normal_at`] flipped to agree with its plane normal
    ///    (the plane normal where that is undefined), and the edge is not
    ///    `soft` or both its faces belong to one blend → `Tangent` (a blend
    ///    marks the joints inside it soft, a corner patch's border among
    ///    them, and those are still joints between two surfaces; elsewhere
    ///    a soft mark is the user's Soften and hides the edge);
    /// 5. `soft` → `Seam`;
    /// 6. both faces unclaimed (and rule 7 does not apply) → `Hard`;
    /// 7. one face is a strip of a rolling-ball fillet (`BlendKind::Fillet`,
    ///    `BlendRole::Strip`) and the other belongs to no blend, and at both
    ///    endpoints: for an unclaimed (swept) strip facet, the oriented
    ///    normals differ by at most [`tol::BLEND_RAIL_NORMAL`]; for a claimed
    ///    strip, its normal and the other face's plane normal, each without
    ///    its component along the edge, differ by at most
    ///    [`tol::BLEND_CLAIMED_RAIL_NORMAL`] → `Tangent` (a fillet's rail,
    ///    where facets only approximate the tangency);
    /// 8. otherwise `Hard`.
    ///
    /// Two unclaimed coplanar faces (a drawn imprint edge) stay `Hard`
    /// (decision I-20): line drawings and snapshots draw only hard edges.
    /// A stale `edge` reads `Hard`.
    pub fn edge_smoothness(&self, edge: crate::ids::EdgeId) -> EdgeSmoothness {
        let Some(e) = self.edges().get(edge) else {
            return EdgeSmoothness::Hard;
        };
        let Some(twin) = e.twin_half_edge else {
            return EdgeSmoothness::Hard;
        };
        if e.crease {
            return EdgeSmoothness::Hard;
        }
        let soft = || {
            if e.soft {
                EdgeSmoothness::Seam
            } else {
                EdgeSmoothness::Hard
            }
        };
        let face_of = |h: crate::ids::HalfEdgeId| {
            let l = self.half_edges()[h].loop_id;
            &self.faces()[self.loops()[l].face]
        };
        let (fa, fb) = (face_of(e.half_edge), face_of(twin));
        // A rolling-ball fillet's strip against a face of no blend (a face
        // of another fillet, or of a later operand's blend, is not this
        // strip's side). A swept strip's flat facets get the wide allowance,
        // a claimed strip the narrow one.
        let strip = |f: &crate::topo::Face| {
            f.blend.is_some_and(|t| {
                t.kind == crate::topo::BlendKind::Fillet && t.role == crate::topo::BlendRole::Strip
            })
        };
        let rail_strip = if strip(fa) && fb.blend.is_none() {
            Some(fa)
        } else if strip(fb) && fa.blend.is_none() {
            Some(fb)
        } else {
            None
        };
        let rail = rail_strip.is_some();
        match (fa.surface, fb.surface) {
            (Some(a), Some(b)) if a.same_surface(&b) => return EdgeSmoothness::Seam,
            (None, None) if e.soft || !rail => return soft(),
            _ => {}
        }
        let Some((p, q)) = self.edge_endpoints(edge) else {
            return soft();
        };
        let oriented = |f: &crate::topo::Face, x: Point3| -> Vec3 {
            let n = f.plane.normal();
            match f.surface.and_then(|s| s.normal_at(x)) {
                Some(a) if a.dot(n) < 0.0 => -a,
                Some(a) => a,
                None => n,
            }
        };
        let within = |x: Point3| (oriented(fa, x) - oriented(fb, x)).length();
        let agree = |bound: f64| within(p) <= bound && within(q) <= bound;
        if e.soft {
            let one_blend = fa.blend.zip(fb.blend).is_some_and(|(a, b)| a.id == b.id);
            return if one_blend && agree(tol::NORMAL_DIRECTION) {
                EdgeSmoothness::Tangent
            } else {
                EdgeSmoothness::Seam
            };
        }
        let tangent = match rail_strip {
            _ if agree(tol::NORMAL_DIRECTION) => true,
            None => false,
            Some(s) if s.surface.is_none() => agree(tol::BLEND_RAIL_NORMAL),
            Some(s) => {
                // Across the edge: the claimed strip against the neighbor's
                // facet plane, each normal with its along-edge part removed.
                let side = if std::ptr::eq(s, fa) { fb } else { fa };
                let Ok(d) = (q - p).normalized() else {
                    return EdgeSmoothness::Hard;
                };
                let across = |v: Vec3| (v - d * v.dot(d)).normalized();
                let meets = |x: Point3| match (across(oriented(s, x)), across(side.plane.normal()))
                {
                    (Ok(a), Ok(b)) => (a - b).length() <= tol::BLEND_CLAIMED_RAIL_NORMAL,
                    _ => false,
                };
                meets(p) && meets(q)
            }
        };
        if tangent {
            EdgeSmoothness::Tangent
        } else {
            EdgeSmoothness::Hard
        }
    }

    /// The shading normal of `face` at point `p` of its own boundary: its
    /// claim's normal there, turned to the side the face's plane faces, or
    /// the plane normal when it has no claim.
    pub(crate) fn oriented_normal(&self, face: crate::ids::FaceId, p: Point3) -> Option<Vec3> {
        let f = self.faces().get(face)?;
        let n = f.plane.normal();
        Some(match f.surface.and_then(|s| s.normal_at(p)) {
            Some(a) if a.dot(n) < 0.0 => -a,
            Some(a) => a,
            None => n,
        })
    }

    /// Smooth shading normals for the corners of a blend face (a strip, a
    /// corner or a runout facet): `(position, normal)` for each distinct
    /// corner of `face` whose normal is not simply the face's own. Pure;
    /// normals only, never a vertex moved. Empty for a face of no blend.
    ///
    /// At a corner, the fan of faces around the vertex is walked outward
    /// from `face` across every edge that is not a `crease` and whose two
    /// faces carry the same blend (the strip's facets along and across, a
    /// corner's sphere fan, two strips meeting in a ridge at a rim-only
    /// corner). Where the fan stops at an edge to a face of no blend that
    /// is not [`EdgeSmoothness::Hard`] (a rail, tangent to the strip), that
    /// face's true normal at the corner is an anchor. With anchors the
    /// normal is their mean, so a swept strip's rail shades exactly as the
    /// face it meets; otherwise it is the mean of the fan's normals. An edge
    /// the fan does not cross and no anchor reads (a cut through the fillet,
    /// a strip's end cap, a seam to another blend) stays a crease: the faces
    /// on either side shade apart.
    pub fn blend_shading_normals(&self, face: crate::ids::FaceId) -> Vec<(Point3, Vec3)> {
        let Some(f) = self.faces().get(face) else {
            return Vec::new();
        };
        let Some(tag) = f.blend else {
            return Vec::new();
        };
        let hes = self.half_edges();
        let face_of = |h: crate::ids::HalfEdgeId| self.loops()[hes[h].loop_id].face;
        // Crossing `edge` from this fan: `Some(true)` to walk on, `Some(false)`
        // for an anchor, `None` for a crease.
        let across = |edge: crate::ids::EdgeId, other: crate::ids::FaceId| -> Option<bool> {
            let e = self.edges().get(edge)?;
            if e.crease {
                return None;
            }
            match self.faces().get(other)?.blend {
                Some(t) if t.id == tag.id => Some(true),
                Some(_) => None,
                None => (self.edge_smoothness(edge) != EdgeSmoothness::Hard).then_some(false),
            }
        };
        let mut out: Vec<(Point3, Vec3)> = Vec::new();
        let mut loops = vec![f.outer_loop];
        loops.extend(f.inner_loops.iter().copied());
        for l in loops {
            let first = self.loops()[l].first_half_edge;
            let mut h0 = first;
            loop {
                let v = self.vertices()[hes[h0].origin].position;
                let mut sum = self
                    .oriented_normal(face, v)
                    .unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                let mut anchors = Vec3::new(0.0, 0.0, 0.0);
                let mut count = 1usize;
                let mut anchored = 0usize;
                // Forward around the vertex, across the outgoing edge; then
                // backward, across the incoming one. The fan's own faces are
                // bounded by the slot count, a guard against a bad twin.
                let limit = self.faces().len() + 2;
                let mut closed = false;
                for backward in [false, true] {
                    if closed {
                        break;
                    }
                    let mut cur = h0;
                    for _ in 0..limit {
                        let step = if backward { hes[cur].prev } else { cur };
                        let Some(twin) = hes[step].twin else { break };
                        let other = face_of(twin);
                        match across(hes[step].edge, other) {
                            Some(true) => {
                                cur = if backward { twin } else { hes[twin].next };
                                if cur == h0 {
                                    closed = true;
                                    break;
                                }
                                if let Some(n) = self.oriented_normal(face_of(cur), v) {
                                    sum = sum + n;
                                    count += 1;
                                }
                            }
                            Some(false) => {
                                if let Some(n) = self.oriented_normal(other, v) {
                                    anchors = anchors + n;
                                    anchored += 1;
                                }
                                break;
                            }
                            None => break,
                        }
                    }
                }
                let pick = if anchored > 0 {
                    Some(anchors)
                } else if count > 1 {
                    Some(sum)
                } else {
                    None
                };
                if let Some(n) = pick.and_then(unit) {
                    out.push((v, n));
                }
                h0 = hes[h0].next;
                if h0 == first {
                    break;
                }
            }
        }
        out
    }
}
