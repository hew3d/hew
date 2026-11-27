//! Station sections (WP05 §6, WP06 §2–3): the section of one picked edge at
//! one of its vertices, by the edge's kind. A straight edge between flat
//! faces sections square to the edge with a ray on each side; a faceted
//! cylinder side lying along the edge traces across its facets (seam
//! crossing); a rim sections in the meridian plane, with the cap's radial
//! ray and the band's seam polyline.

use std::collections::BTreeMap;

use super::BlendProfile;
use super::classify::{EdgeInfo, EdgeKind};
use super::plan::PlanFail;
use super::section::{Section, Trace, solve_section};
use super::{BlendError, LimitFeature};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::math::{Point3, Vec3};
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::{Object, SurfaceRef};

/// The axis of a coaxial claim (cylinder, cone, torus).
pub(crate) fn claim_axis(s: &SurfaceRef) -> Option<(Point3, Vec3)> {
    match *s {
        SurfaceRef::Cylinder {
            axis_point, axis, ..
        } => Some((axis_point, axis)),
        SurfaceRef::Cone { apex, axis, .. } => Some((apex, axis)),
        SurfaceRef::Torus { center, axis, .. } => Some((center, axis)),
        SurfaceRef::Sphere { .. } => None,
    }
}

/// The foot of `p` on the axis line and the radial vector from it.
pub(crate) fn axial(p: Point3, axis_point: Point3, axis: Vec3) -> (f64, Vec3) {
    let t = (p - axis_point).dot(axis);
    (t, (p - axis_point) - axis * t)
}

/// Whether faces `f` and `g` belong to one side: the same face, coplanar,
/// claiming the same surface, or meeting smoothly along one of `at`, the
/// edges at the vertex in question.
pub(crate) fn same_side(obj: &Object, f: FaceId, g: FaceId, at: Option<&[EdgeId]>) -> bool {
    if f == g || super::chain::coplanar(obj, f, g) {
        return true;
    }
    let (sf, sg) = (obj.faces()[f].surface, obj.faces()[g].surface);
    if let (Some(a), Some(b)) = (sf, sg)
        && a.same_surface(&b)
    {
        return true;
    }
    let Some(at) = at else {
        return false;
    };
    at.iter().any(|&e| {
        matches!(
            obj.edge_smoothness(e),
            EdgeSmoothness::Tangent | EdgeSmoothness::Seam
        ) && obj
            .edge_faces(e)
            .is_some_and(|(x, y)| (x == f && y == g) || (x == g && y == f))
    })
}

/// The section of `info` at its vertex `v`, with the rail landing segments.
/// `vertex_edges` is the object's [`Object::vertex_edges`], built once by
/// the caller.
pub(crate) fn station_section(
    obj: &Object,
    vertex_edges: &BTreeMap<VertexId, Vec<EdgeId>>,
    info: &EdgeInfo,
    v: VertexId,
    profile: &BlendProfile,
) -> Result<Section, PlanFail> {
    let q = obj.vertices()[v].position;
    let (ta, tb) = match info.kind {
        EdgeKind::PlanePlane => (Trace::ray(info.t_a), Trace::ray(info.t_b)),
        EdgeKind::AlongAxis => (
            facet_trace(obj, info, 0, v)?.0,
            facet_trace(obj, info, 1, v)?.0,
        ),
        EdgeKind::Swept => return Err(PlanFail::Hard(BlendError::UnsupportedEdge)),
        EdgeKind::Rim { axis_point, axis } => {
            let (_, radial) = axial(q, axis_point, axis);
            let radial = radial
                .normalized()
                .map_err(|_| BlendError::BlendDegenerate)?;
            let meridian = axis.cross(radial);
            (
                rim_trace(obj, vertex_edges, info, 0, v, radial, meridian)?,
                rim_trace(obj, vertex_edges, info, 1, v, radial, meridian)?,
            )
        }
    };
    solve_section(profile, &ta, &tb).map_err(|_| PlanFail::Size(LimitFeature::FacetWidth))
}

/// The face of side `side` (0 = A, 1 = B).
fn side_face(info: &EdgeInfo, side: usize) -> FaceId {
    if side == 0 { info.a } else { info.b }
}

fn side_dir(info: &EdgeInfo, side: usize) -> Vec3 {
    if side == 0 { info.t_a } else { info.t_b }
}

/// Whether face `f` is a flat side: no claim.
fn flat(obj: &Object, f: FaceId) -> bool {
    obj.faces()[f].surface.is_none()
}

/// The far end of edge `e` from `v`.
fn far_end(obj: &Object, e: EdgeId, v: VertexId) -> Option<VertexId> {
    let (a, b) = obj.edge_vertex_ids(e)?;
    if a == v {
        Some(b)
    } else if b == v {
        Some(a)
    } else {
        None
    }
}

/// A faceted cylinder side along a straight edge (WP06 §3): the in-plane
/// perpendicular of each facet in turn, crossing seams into facets of the
/// same surface whose seams run along the edge and span it.
fn facet_trace(
    obj: &Object,
    info: &EdgeInfo,
    side: usize,
    v: VertexId,
) -> Result<(Trace, Vec<FaceId>), PlanFail> {
    let f0 = side_face(info, side);
    if flat(obj, f0) {
        return Ok((Trace::ray(side_dir(info, side)), vec![f0]));
    }
    let mut faces: Vec<FaceId> = Vec::new();
    let claim = obj.faces()[f0].surface.expect("claimed side");
    let d = info.d_a;
    let (p0, p1) = obj
        .edge_endpoints(info.edge)
        .ok_or(BlendError::BlendDegenerate)?;
    let (e_lo, e_hi) = {
        let b = (p1 - p0).dot(d);
        (b.min(0.0), b.max(0.0))
    };
    let mut dirs: Vec<Vec3> = Vec::new();
    let mut lens: Vec<f64> = Vec::new();
    // Walk facets: the current facet and the line it starts from.
    let mut face = f0;
    let mut line_pt = obj.vertices()[v].position;
    let mut prev_face: Option<FaceId> = None;
    for _ in 0..obj.faces().len() {
        let n = obj.faces()[face].plane.normal();
        // In-plane direction perpendicular to the edge, into the facet.
        let mut t = n
            .cross(d)
            .normalized()
            .map_err(|_| BlendError::BlendDegenerate)?;
        let centroid = {
            let ps: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            let s = ps
                .iter()
                .fold(Vec3::new(0.0, 0.0, 0.0), |a, p| a + p.to_vec())
                / ps.len() as f64;
            Point3::new(s.x, s.y, s.z)
        };
        if (centroid - line_pt).dot(t) < 0.0 {
            t = -t;
        }
        // The facet's far seam: the loop edge parallel to d farthest along t.
        let mut best: Option<(f64, EdgeId)> = None;
        for lid in obj.face_loops(face) {
            for h in obj.loop_half_edges(lid) {
                let e = obj.half_edges()[h].edge;
                let Some((a, b)) = obj.edge_endpoints(e) else {
                    continue;
                };
                let Ok(dir) = (b - a).normalized() else {
                    continue;
                };
                if dir.dot(d).abs() < 1.0 - tol::NORMAL_DIRECTION {
                    continue;
                }
                let w = (a - line_pt).dot(t);
                if w > tol::POINT_MERGE && best.is_none_or(|(bw, _)| w > bw) {
                    best = Some((w, e));
                }
            }
        }
        let Some((width, seam)) = best else {
            return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
        };
        dirs.push(t);
        lens.push(width);
        faces.push(face);
        // Cross into the next facet if it continues the surface and its
        // seam spans the chain's extent.
        let Some((x, y)) = obj.edge_faces(seam) else {
            break;
        };
        let next = if x == face { y } else { x };
        if Some(next) == prev_face {
            break;
        }
        let same = obj.faces()[next]
            .surface
            .is_some_and(|s| s.same_surface(&claim));
        let (sa, sb) = obj
            .edge_endpoints(seam)
            .ok_or(BlendError::BlendDegenerate)?;
        let (ta, tb) = ((sa - p0).dot(d), (sb - p0).dot(d));
        let spans = ta.min(tb) <= e_lo + tol::POINT_MERGE && ta.max(tb) >= e_hi - tol::POINT_MERGE;
        if !(same && spans) {
            break;
        }
        line_pt = line_pt + t * width;
        prev_face = Some(face);
        face = next;
    }
    Ok((Trace { dirs, lens }, faces))
}

/// The facets a faceted-cylinder side's trace crosses at `v`, in order.
pub(crate) fn facet_faces(
    obj: &Object,
    info: &EdgeInfo,
    side: usize,
    v: VertexId,
) -> Result<Vec<FaceId>, PlanFail> {
    Ok(facet_trace(obj, info, side, v)?.1)
}

/// A rim side at station `v` (WP06 §2): a cap gives the radial ray into
/// it; a band gives its seam polyline in the meridian plane.
fn rim_trace(
    obj: &Object,
    ve: &BTreeMap<VertexId, Vec<EdgeId>>,
    info: &EdgeInfo,
    side: usize,
    v: VertexId,
    radial: Vec3,
    meridian: Vec3,
) -> Result<Trace, PlanFail> {
    let f = side_face(info, side);
    let toward = side_dir(info, side);
    if flat(obj, f) {
        let dir = if radial.dot(toward) >= 0.0 {
            radial
        } else {
            -radial
        };
        return Ok(Trace::ray(dir));
    }
    let claim = obj.faces()[f].surface.expect("band side");
    let in_band = |g: FaceId| {
        obj.faces()[g]
            .surface
            .is_some_and(|s| s.same_surface(&claim))
            || g == f
    };
    let mut dirs: Vec<Vec3> = Vec::new();
    let mut lens: Vec<f64> = Vec::new();
    let (mut cur, mut from): (VertexId, Option<EdgeId>) = (v, Some(info.edge));
    for _ in 0..obj.edges().len() {
        let p = obj.vertices()[cur].position;
        let next = ve.get(&cur).into_iter().flatten().copied().find(|&e| {
            Some(e) != from
                && e != info.edge
                // Inside the band, or (at a band's end) its tangent joint
                // with the next face.
                && obj
                    .edge_faces(e)
                    .is_some_and(|(x, y)| in_band(x) || in_band(y))
                && far_end(obj, e, cur).is_some_and(|w| {
                    let dir = obj.vertices()[w].position - p;
                    dir.length() > tol::POINT_MERGE
                        && dir.normalized().is_ok_and(|u| {
                            u.dot(meridian).abs() <= tol::NORMAL_DIRECTION
                                // Leaving the chain, not running along it.
                                && (dirs.is_empty() || u.dot(*dirs.last().expect("non-empty")) > 0.0)
                        })
                })
        });
        let Some(e) = next else {
            break;
        };
        let w = far_end(obj, e, cur).expect("checked");
        let seg = obj.vertices()[w].position - p;
        dirs.push(seg.normalized().map_err(|_| BlendError::BlendDegenerate)?);
        lens.push(seg.length());
        from = Some(e);
        cur = w;
    }
    if dirs.is_empty() {
        // No meridian seam: not a rim this engine sections (WP07 sweeps it).
        return Err(PlanFail::Hard(BlendError::UnsupportedEdge));
    }
    Ok(Trace { dirs, lens })
}
