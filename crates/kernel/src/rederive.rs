//! Smoothness that survives what it should (true-curves.md §3.3 rules 1–2):
//! the edge metadata a boolean or a non-similarity transform would
//! otherwise lose, re-derived from the geometry and provenance it still
//! has. Pure metadata — no vertex moves, no face changes (not repair).

use std::collections::BTreeMap;

use crate::ids::{EdgeId, FaceId};
use crate::math::Point3;
use crate::sketch::CurveGeom;
use crate::surface::EdgeSmoothness;
use crate::tol;
use crate::topo::{Object, SurfaceRef};

/// Before a non-similarity transform drops claims and tags: marks `soft` on
/// every interior, non-creased edge whose smoothness is `Seam` or `Tangent`,
/// so the facets keep shading as the curve the user drew (E1) — a stretched
/// cylinder stays smooth, and a fillet's rails stay hidden under Deform.
/// Must run on the PRE-transform state, since the classification needs the
/// claims the transform is about to drop.
pub(crate) fn soften_before_claim_drop(obj: &mut Object) {
    let smooth: Vec<EdgeId> = obj
        .edges()
        .keys()
        .filter(|&e| {
            matches!(
                obj.edge_smoothness(e),
                EdgeSmoothness::Seam | EdgeSmoothness::Tangent
            )
        })
        .collect();
    for e in smooth {
        let edge = &mut obj.edges[e];
        if edge.twin_half_edge.is_some() && !edge.crease {
            edge.soft = true;
        }
    }
}

/// Source of a boolean result face: which operand and which of its faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Provenance {
    /// 0 = operand A, 1 = operand B.
    pub side: u8,
    /// The operand's own `FaceId` (B's in the frame the boolean saw it).
    pub face: FaceId,
}

/// One source edge's metadata and segment, for the provenance tests.
struct SourceEdge {
    a: Point3,
    b: Point3,
    soft: bool,
    crease: bool,
    curve: Option<CurveGeom>,
    planarity_tol: f64,
}

/// Every interior edge of `obj`, keyed by its two faces (smaller first).
fn edges_by_face_pair(
    obj: &Object,
    side: u8,
    out: &mut BTreeMap<(u8, FaceId, FaceId), Vec<SourceEdge>>,
) {
    let all: Vec<EdgeId> = obj.edges().keys().collect();
    edges_by_face_pair_of(obj, side, &all, out);
}

/// [`edges_by_face_pair`] over the listed edges only (in the given order).
fn edges_by_face_pair_of(
    obj: &Object,
    side: u8,
    edges: &[EdgeId],
    out: &mut BTreeMap<(u8, FaceId, FaceId), Vec<SourceEdge>>,
) {
    for (eid, e) in edges
        .iter()
        .filter_map(|&id| obj.edges().get(id).map(|e| (id, e)))
    {
        let Some(twin) = e.twin_half_edge else {
            continue;
        };
        if !e.soft && !e.crease && e.curve.is_none() {
            continue;
        }
        let face_of = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
        let (fa, fb) = (face_of(e.half_edge), face_of(twin));
        let Some((a, b)) = obj.edge_endpoints(eid) else {
            continue;
        };
        out.entry((side, fa.min(fb), fa.max(fb)))
            .or_default()
            .push(SourceEdge {
                a,
                b,
                soft: e.soft,
                crease: e.crease,
                curve: e.curve,
                planarity_tol: obj.planarity_tol,
            });
    }
}

fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Re-derives edge metadata on a boolean result (true-curves-gaps.md §1).
/// `a` and `b_in_a` are the operands as the boolean saw them (B already in
/// A's frame); `prov[f]` is the source of each result face. For each result
/// edge between faces `fa`, `fb` (sources `pa`, `pb`):
///
/// - **crease** when `pa` and `pb` come from the same operand and that
///   operand had a creased edge between `pa.face` and `pb.face` whose
///   segment contains both endpoints (each within `POINT_MERGE`);
/// - **soft** when not creased and either both result faces claim the same
///   surface (R1) or the same provenance test holds for a soft source edge
///   (R2);
/// - **curve** (R3a) copied from a source edge passing the provenance test
///   whose circle both endpoints lie on (the validator's own test, at the
///   source's planarity tolerance); else (R3b) set where one face claims a
///   cylinder and the other's plane is perpendicular to its axis, with both
///   endpoints at the cylinder's radius and one axial coordinate (within
///   `POINT_MERGE`) — a rim chord of the cylinder.
///
/// `polygon` claims are not re-derived. The caller validates afterwards.
pub(crate) fn rederive_after_boolean(
    result: &mut Object,
    a: &Object,
    b_in_a: &Object,
    prov: &BTreeMap<FaceId, Provenance>,
) {
    let mut source: BTreeMap<(u8, FaceId, FaceId), Vec<SourceEdge>> = BTreeMap::new();
    edges_by_face_pair(a, 0, &mut source);
    edges_by_face_pair(b_in_a, 1, &mut source);

    let edge_ids: Vec<EdgeId> = result.edges().keys().collect();
    rederive_edges(result, &edge_ids, &source, &|f| prov.get(&f).copied());
}

/// [`rederive_after_boolean`] for a result spliced into a kept operand
/// (the boolean's untouched faces left as they were): only `edges` (those
/// of the new faces) are re-derived, from the source edges of the operand
/// faces in `near` (`near[side]`, the faces the boolean rebuilt); `prov`
/// names the source of any result face, kept ones included.
pub(crate) fn rederive_edges_after_boolean(
    result: &mut Object,
    operands: [&Object; 2],
    near: [&[FaceId]; 2],
    edges: &[EdgeId],
    prov: &dyn Fn(FaceId) -> Option<Provenance>,
) {
    let mut source: BTreeMap<(u8, FaceId, FaceId), Vec<SourceEdge>> = BTreeMap::new();
    for side in 0..2 {
        let obj = operands[side];
        let mut seen: Vec<EdgeId> = near[side]
            .iter()
            .filter_map(|&f| obj.faces().get(f))
            .flat_map(|face| {
                std::iter::once(face.outer_loop)
                    .chain(face.inner_loops.iter().copied())
                    .flat_map(|l| obj.loop_half_edges(l).collect::<Vec<_>>())
            })
            .map(|h| obj.half_edges()[h].edge)
            .collect();
        seen.sort_unstable();
        seen.dedup();
        edges_by_face_pair_of(obj, side as u8, &seen, &mut source);
    }
    rederive_edges(result, edges, &source, prov);
}

fn rederive_edges(
    result: &mut Object,
    edge_ids: &[EdgeId],
    source: &BTreeMap<(u8, FaceId, FaceId), Vec<SourceEdge>>,
    prov: &dyn Fn(FaceId) -> Option<Provenance>,
) {
    for &eid in edge_ids {
        let e = result.edges()[eid];
        let Some(twin) = e.twin_half_edge else {
            continue;
        };
        let face_of = |h| result.loops()[result.half_edges()[h].loop_id].face;
        let (fa, fb) = (face_of(e.half_edge), face_of(twin));
        let (Some(pa), Some(pb)) = (prov(fa), prov(fb)) else {
            continue;
        };
        let Some((p, q)) = result.edge_endpoints(eid) else {
            continue;
        };
        let along = |s: &SourceEdge| {
            point_segment_distance(p, s.a, s.b) <= tol::POINT_MERGE
                && point_segment_distance(q, s.a, s.b) <= tol::POINT_MERGE
        };
        let sources: &[SourceEdge] = if pa.side == pb.side {
            source
                .get(&(pa.side, pa.face.min(pb.face), pa.face.max(pb.face)))
                .map_or(&[], |v| v.as_slice())
        } else {
            &[]
        };
        let crease = sources.iter().any(|s| s.crease && along(s));
        let (sa, sb) = (result.faces()[fa].surface, result.faces()[fb].surface);
        let same_claim = matches!((sa, sb), (Some(x), Some(y)) if x.same_surface(&y));
        let soft = !crease && (same_claim || sources.iter().any(|s| s.soft && along(s)));
        let on_circle = |g: &CurveGeom, t: f64| {
            ((p - g.center).length() - g.radius).abs() <= t
                && ((q - g.center).length() - g.radius).abs() <= t
        };
        let curve = sources
            .iter()
            .filter(|s| along(s))
            .find_map(|s| s.curve.filter(|g| on_circle(g, s.planarity_tol)))
            .or_else(|| {
                let rim = |claim: Option<SurfaceRef>, other: FaceId| -> Option<CurveGeom> {
                    let Some(SurfaceRef::Cylinder {
                        axis_point,
                        axis,
                        radius,
                    }) = claim
                    else {
                        return None;
                    };
                    let n = result.faces()[other].plane.normal();
                    if n.dot(axis).abs() < 1.0 - tol::NORMAL_DIRECTION {
                        return None;
                    }
                    let split = |x: Point3| {
                        let d = x - axis_point;
                        let t = d.dot(axis);
                        (t, (d - axis * t).length())
                    };
                    let ((tp, rp), (tq, rq)) = (split(p), split(q));
                    ((tp - tq).abs() <= tol::POINT_MERGE
                        && (rp - radius).abs() <= tol::POINT_MERGE
                        && (rq - radius).abs() <= tol::POINT_MERGE)
                        .then(|| CurveGeom {
                            center: axis_point + axis * tp,
                            radius,
                        })
                };
                rim(sa, fb).or_else(|| rim(sb, fa))
            });
        let edge = &mut result.edges[eid];
        edge.crease = crease;
        edge.soft = soft;
        edge.curve = curve;
    }
}
