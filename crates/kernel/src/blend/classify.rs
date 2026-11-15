//! Per-edge classification (WP05 §4): side A, the section frame, dihedral,
//! convexity and the edge kind.

use super::{BlendError, BlendProfile, BlendSpec, Convexity};
use crate::ids::{EdgeId, FaceId};
use crate::math::{Point3, Vec3};
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::{Object, WatertightState};

/// The catalogue of edge neighborhoods the engine blends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EdgeKind {
    /// Both faces flat (unclaimed).
    PlanePlane,
    /// A straight edge between a flat face and a faceted cylinder whose
    /// axis runs along the edge (the D-post).
    AlongAxis,
    /// An edge of a circle about `axis` (through `axis_point`) between a
    /// cap (flat, square to the axis) or band (coaxial claim) on each side.
    Rim { axis_point: Point3, axis: Vec3 },
    /// Any other sharp edge: the unstamped swept blend (WP07 §4).
    Swept,
}

thread_local! {
    /// Whether every edge classifies as swept: the retry of a spec whose
    /// faceted plane–plane chain the catalogue planners refuse (D-41).
    static FORCE_SWEPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether every edge classifies as swept right now.
pub(crate) fn is_forced_swept() -> bool {
    FORCE_SWEPT.with(std::cell::Cell::get)
}

/// Runs `f` with every edge classified as swept.
pub(crate) fn with_forced_swept<T>(f: impl FnOnce() -> T) -> T {
    let before = FORCE_SWEPT.with(|s| s.replace(true));
    let out = f();
    FORCE_SWEPT.with(|s| s.set(before));
    out
}

/// One picked edge, classified.
#[derive(Debug, Clone)]
pub(crate) struct EdgeInfo {
    pub edge: EdgeId,
    pub a: FaceId,
    pub b: FaceId,
    pub d_a: Vec3,
    pub t_a: Vec3,
    pub t_b: Vec3,
    pub theta: f64,
    pub convexity: Convexity,
    pub kind: EdgeKind,
}

/// Classifies one edge with side A = `a`.
pub(crate) fn info_for(obj: &Object, edge: EdgeId, a: FaceId) -> Result<EdgeInfo, BlendError> {
    let f = obj.edge_frame(edge, a).ok_or(BlendError::UnknownEdge)?;
    let theta = f.t_a.dot(f.t_b).clamp(-1.0, 1.0).acos();
    if std::f64::consts::PI - theta <= tol::NORMAL_DIRECTION {
        return Err(BlendError::EdgeIsSmooth);
    }
    if theta <= tol::NORMAL_DIRECTION {
        return Err(BlendError::UnsupportedEdge);
    }
    let n_b = obj.faces()[f.b].plane.normal();
    let convexity = if f.t_a.dot(n_b) < 0.0 {
        Convexity::Convex
    } else {
        Convexity::Concave
    };
    // Outside the catalogue (and on a forced retry) the swept blend rolls
    // the ball over the facets as they are (WP07 §4, I-24).
    let kind = if FORCE_SWEPT.with(|s| s.get()) {
        EdgeKind::Swept
    } else {
        edge_kind(obj, edge, f.a, f.b, f.d_a).unwrap_or(EdgeKind::Swept)
    };
    Ok(EdgeInfo {
        edge,
        a: f.a,
        b: f.b,
        d_a: f.d_a,
        t_a: f.t_a,
        t_b: f.t_b,
        theta,
        convexity,
        kind,
    })
}

/// The kind of the edge between `a` and `b`, or `None` outside the
/// catalogue.
fn edge_kind(obj: &Object, edge: EdgeId, a: FaceId, b: FaceId, d: Vec3) -> Option<EdgeKind> {
    use crate::topo::SurfaceRef;
    let (sa, sb) = (obj.faces()[a].surface, obj.faces()[b].surface);
    if sa.is_none() && sb.is_none() {
        return Some(EdgeKind::PlanePlane);
    }
    let along = |s: Option<SurfaceRef>| {
        matches!(s, Some(SurfaceRef::Cylinder { axis, .. })
            if axis.dot(d).abs() >= 1.0 - tol::NORMAL_DIRECTION)
    };
    if (along(sa) && sb.is_none()) || (along(sb) && sa.is_none()) {
        return Some(EdgeKind::AlongAxis);
    }
    // A sphere has no axis of its own: where it meets a flat face, its rim
    // circle turns about the line through its center along that face's
    // normal (a dome's base, a hemispherical cap on a slab).
    let sphere_rim = |s: Option<SurfaceRef>, other: FaceId, os: Option<SurfaceRef>| match (s, os) {
        (Some(SurfaceRef::Sphere { center, .. }), None) => {
            Some((center, obj.faces()[other].plane.normal()))
        }
        _ => None,
    };
    let (axis_point, axis) = sa
        .and_then(|s| super::station::claim_axis(&s))
        .or_else(|| sb.and_then(|s| super::station::claim_axis(&s)))
        .or_else(|| sphere_rim(sa, b, sb))
        .or_else(|| sphere_rim(sb, a, sa))?;
    let (p0, p1) = obj.edge_endpoints(edge)?;
    let (t0, r0) = super::station::axial(p0, axis_point, axis);
    let (t1, r1) = super::station::axial(p1, axis_point, axis);
    if (t0 - t1).abs() > tol::POINT_MERGE
        || (r0.length() - r1.length()).abs() > tol::POINT_MERGE
        || r0.length() <= tol::POINT_MERGE
    {
        return None;
    }
    let side_ok = |f: FaceId, s: Option<SurfaceRef>| match s {
        None => obj.faces()[f].plane.normal().dot(axis).abs() >= 1.0 - tol::NORMAL_DIRECTION,
        // A sphere is coaxial with any axis through its center.
        Some(SurfaceRef::Sphere { center, .. }) => {
            super::station::axial(center, axis_point, axis).1.length() <= tol::POINT_MERGE
        }
        Some(c) => super::station::claim_axis(&c).is_some_and(|(p, ax)| {
            ax.dot(axis).abs() >= 1.0 - tol::NORMAL_DIRECTION
                && super::station::axial(p, axis_point, axis).1.length() <= tol::POINT_MERGE
        }),
    };
    (side_ok(a, sa) && side_ok(b, sb)).then_some(EdgeKind::Rim { axis_point, axis })
}

/// Classifies every spec edge (deduplicated, ascending id) and checks the
/// spec-level preconditions.
pub(crate) fn classify(obj: &Object, spec: &BlendSpec) -> Result<Vec<EdgeInfo>, BlendError> {
    if spec.edges.is_empty() {
        return Err(BlendError::EmptySpec);
    }
    let mut edges = spec.edges.clone();
    edges.sort();
    edges.dedup();
    for &e in &edges {
        if !obj.edges().contains_key(e) {
            return Err(BlendError::UnknownEdge);
        }
    }
    if obj.watertight() != WatertightState::Watertight {
        return Err(BlendError::BlendNeedsSolid);
    }
    // What this engine supports so far.
    match &spec.profile {
        BlendProfile::Round { radius, segments } => {
            if !(2..=96).contains(segments) {
                return Err(BlendError::SegmentsOutOfRange {
                    segments: *segments,
                });
            }
            check_size(*radius)?;
        }
        BlendProfile::Chamfer {
            setback_a,
            setback_b,
        } => {
            check_size(*setback_a)?;
            check_size(*setback_b)?;
        }
        BlendProfile::Cove { radius, segments } => {
            if !(2..=96).contains(segments) {
                return Err(BlendError::SegmentsOutOfRange {
                    segments: *segments,
                });
            }
            check_size(*radius)?;
        }
        BlendProfile::Bullnose { segments } => {
            if !(2..=96).contains(segments) {
                return Err(BlendError::SegmentsOutOfRange {
                    segments: *segments,
                });
            }
        }
        BlendProfile::Custom {
            shape,
            width,
            arc_segments,
        } => {
            if !(1..=96).contains(arc_segments) {
                return Err(BlendError::SegmentsOutOfRange {
                    segments: *arc_segments,
                });
            }
            shape.sample(*arc_segments)?;
            check_size(*width)?;
        }
    }
    if let Some(st) = spec.stops
        && !(st.start.is_finite() && st.end.is_finite() && st.start >= 0.0 && st.end >= 0.0)
    {
        return Err(BlendError::StopOutOfRange);
    }
    if spec.stops.is_some() && !spec.handles.is_empty() {
        return Err(BlendError::UnsupportedEdge);
    }
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        if obj.edge_smoothness(e) != EdgeSmoothness::Hard {
            return Err(BlendError::EdgeIsSmooth);
        }
        let (fa, fb) = obj.edge_faces(e).ok_or(BlendError::BlendNeedsSolid)?;
        let a = match spec.primary {
            Some(p) if p == fb => fb,
            _ => fa,
        };
        let mut info = info_for(obj, e, a)?;
        // A faceted cylinder along the edge is side B unless the spec says
        // which side is primary.
        if info.kind == EdgeKind::AlongAxis
            && spec.primary.is_none()
            && obj.faces()[info.a].surface.is_some()
        {
            info = info_for(obj, e, info.b)?;
        }
        if matches!(spec.profile, BlendProfile::Cove { .. }) && info.convexity != Convexity::Convex
        {
            return Err(BlendError::ProfileNeedsConvexEdge);
        }
        // The swept blend takes every profile but a bullnose (which needs
        // two parallel sides), at one size, without stops (WP07 §4, D-43,
        // D-106).
        if info.kind == EdgeKind::Swept
            && (spec.stops.is_some()
                || !spec.handles.is_empty()
                || matches!(spec.profile, BlendProfile::Bullnose { .. }))
        {
            return Err(BlendError::UnsupportedEdge);
        }
        out.push(info);
    }
    Ok(out)
}

fn check_size(s: f64) -> Result<(), BlendError> {
    if !s.is_finite() || s <= tol::POINT_MERGE {
        return Err(BlendError::RadiusTooLarge {
            max: 0.0,
            limited_by: super::LimitFeature::Unknown,
        });
    }
    Ok(())
}
