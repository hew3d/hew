//! Coaxial rims recognized from their facets (decision D-118): a Loft
//! through coaxial circles leaves its walls unclaimed (one smooth surface
//! to paint and pick, D-47), but the band of facets beside each end cap is
//! exactly a faceted cone (or cylinder) about the cap's circle. A fillet of
//! that rim claims the band's cone for the plan, so the rim rounds as an
//! exact torus as any coaxial rim does; the claim is checked on every
//! facet's own vertices, and the wall's faces keep no claim afterward.
//! Metadata only: no vertex moves.

use super::BlendSpec;
use crate::ids::FaceId;
use crate::math::{Point3, Vec3};
use crate::surface;
use crate::tol;
use crate::topo::{Object, SurfaceRef};

/// The circle the loop of `face` holding `edge` runs on, as (center, axis,
/// radius): every edge of the loop a chord of one attributed circle
/// (`Edge::curve`, as an extrusion's or a coaxial Loft's cap carries), in
/// the face's plane, the axis its normal.
fn loop_circle(
    obj: &Object,
    face: FaceId,
    edge: crate::ids::EdgeId,
) -> Option<(Point3, Vec3, f64)> {
    let g = obj.edges()[edge].curve?;
    let l = obj.face_loops(face).into_iter().find(|&l| {
        obj.loop_half_edges(l)
            .any(|h| obj.half_edges()[h].edge == edge)
    })?;
    let same = |c: Option<crate::sketch::CurveGeom>| {
        c.is_some_and(|c| {
            c.center.approx_eq(g.center, tol::POINT_MERGE)
                && (c.radius - g.radius).abs() <= tol::POINT_MERGE
        })
    };
    if !obj
        .loop_half_edges(l)
        .all(|h| same(obj.edges()[obj.half_edges()[h].edge].curve))
    {
        return None;
    }
    let plane = obj.faces()[face].plane;
    (plane.signed_distance(g.center).abs() <= tol::PLANE_DIST && g.radius > tol::POINT_MERGE)
        .then(|| (g.center, plane.normal(), g.radius))
}

/// The cone (or cylinder) through the rim circle `(center, axis, radius)`
/// that facet `f` is a chord facet of: its vertices off the rim's plane all
/// at one height and one distance from the axis, and the claim holding on
/// the facet.
fn band_claim(
    obj: &Object,
    f: FaceId,
    (center, axis, radius): (Point3, Vec3, f64),
) -> Option<SurfaceRef> {
    let face = &obj.faces()[f];
    let ring: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
    let mut other: Option<(f64, f64)> = None;
    for &p in &ring {
        let d = p - center;
        let (h, r) = (d.dot(axis), (d - axis * d.dot(axis)).length());
        if h.abs() <= tol::PLANE_DIST {
            if (r - radius).abs() > tol::POINT_MERGE {
                return None;
            }
            continue;
        }
        match other {
            None => other = Some((h, r)),
            Some((h0, r0)) => {
                if (h - h0).abs() > tol::POINT_MERGE || (r - r0).abs() > tol::POINT_MERGE {
                    return None;
                }
            }
        }
    }
    let (h, r) = other?;
    let claim = if (r - radius).abs() <= tol::POINT_MERGE {
        SurfaceRef::Cylinder {
            axis_point: center,
            axis,
            radius,
        }
    } else {
        let d = axis * h;
        let apex = center + d * (radius / (radius - r));
        SurfaceRef::Cone {
            apex,
            axis: (center - apex).normalized().ok()?,
            half_angle: ((radius - r).abs() / h.abs()).atan(),
        }
    };
    (claim.holds_on(&ring, &face.plane, tol::PLANE_DIST)
        && surface::validate_face_claim(&claim, &face.plane, &ring, tol::PLANE_DIST))
    .then_some(claim)
}

impl Object {
    /// A copy of `self` with the band beside each picked rim claimed, and
    /// the claims made: `None` when no picked edge is such a rim. A rim
    /// here is an edge between an unclaimed flat cap whose loop is one
    /// attributed circle and an unclaimed facet of a cone or cylinder about
    /// it; the whole band along that circle is claimed where it holds.
    pub(crate) fn with_rim_claims(&self, spec: &BlendSpec) -> Option<(Object, Vec<SurfaceRef>)> {
        let plain = |f: FaceId| {
            self.faces()
                .get(f)
                .is_some_and(|x| x.surface.is_none() && x.blend.is_none())
        };
        let mut work: Option<Object> = None;
        let mut made: Vec<SurfaceRef> = Vec::new();
        for &e in &spec.edges {
            let Some((f, g)) = self.edge_faces(e) else {
                continue;
            };
            if !plain(f) || !plain(g) {
                continue;
            }
            for (cap, wall) in [(f, g), (g, f)] {
                let Some(circle) = loop_circle(self, cap, e) else {
                    continue;
                };
                let Some(claim) = band_claim(self, wall, circle) else {
                    continue;
                };
                if made.iter().any(|c| c.same_surface(&claim)) {
                    break;
                }
                // The band: every unclaimed face across the cap's loop
                // that the same claim holds on.
                let obj = work.get_or_insert_with(|| self.clone());
                let l = self
                    .face_loops(cap)
                    .into_iter()
                    .find(|&l| {
                        self.loop_half_edges(l)
                            .any(|h| self.half_edges()[h].edge == e)
                    })
                    .expect("the rim's loop");
                for h in self.loop_half_edges(l) {
                    let x = self.half_edges()[h].edge;
                    let Some((p, q)) = self.edge_faces(x) else {
                        continue;
                    };
                    let band = if p == cap { q } else { p };
                    if plain(band)
                        && band_claim(self, band, circle).is_some_and(|c| c.same_surface(&claim))
                    {
                        obj.faces[band].surface = Some(claim);
                    }
                }
                made.push(claim);
                break;
            }
        }
        Some((work?, made))
    }
}

/// Drops the claims [`Object::with_rim_claims`] made from the faces of
/// `after` that are not the blend's and were unclaimed in `base`: the wall
/// stays the unclaimed surface it was.
pub(crate) fn drop_rim_claims(base: &Object, after: &mut Object, claims: &[SurfaceRef]) {
    let ids: Vec<FaceId> = after
        .faces()
        .iter()
        .filter(|(id, f)| {
            f.blend.is_none()
                && f.surface
                    .is_some_and(|s| claims.iter().any(|c| c.same_surface(&s)))
                && base.faces().get(*id).is_none_or(|b| b.surface.is_none())
        })
        .map(|(id, _)| id)
        .collect();
    for id in ids {
        after.faces[id].surface = None;
    }
}
