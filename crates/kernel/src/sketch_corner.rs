//! 2D corner blends: rounding or chamfering the vertex where exactly two
//! straight sketch segments meet (`fillets-chamfers.md` §9).
//!
//! This is a sketch-level edit and shares no code with the 3D blend engine.
//! The result is a tangent arc committed as a curve chain with its
//! [`CurveGeom`] (so extruding it stamps cylinder walls), or one chamfer
//! segment, with both lines trimmed back to the tangent points.

use serde::{Deserialize, Serialize};

use crate::math::{Point3, Vec3};
use crate::sketch::{
    CurveGeom, MAX_CIRCLE_SEGMENTS, Sketch, SketchCurveId, SketchEdgeId, SketchError,
    SketchVertexId,
};
use crate::tol;

/// How [`Sketch::blend_corner`] shapes a corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CornerProfile {
    /// A tangent arc of `radius` meters, drawn as `segments` facets across
    /// the arc (`1..=1024`).
    Round {
        /// Arc radius in meters (finite, larger than
        /// [`tol::POINT_MERGE`]).
        radius: f64,
        /// Facets across the arc; the arc has `segments + 1` points.
        segments: u32,
    },
    /// One straight segment between a point `setback_a` along the first
    /// edge and `setback_b` along the second, in the order the two edges
    /// turn counter-clockwise seen from the sketch plane's normal side
    /// (`first edge → corner → second edge` is a left turn).
    Chamfer {
        /// Distance from the corner along the edge that starts the
        /// counter-clockwise order, in meters.
        setback_a: f64,
        /// Distance from the corner along the other edge, in meters.
        setback_b: f64,
    },
}

/// What [`Sketch::blend_corner`] built.
#[derive(Debug, Clone, PartialEq)]
pub struct CornerBlendReport {
    /// The curve chain of the arc (a round), or `None` for a chamfer.
    pub curve: Option<SketchCurveId>,
    /// Every edge the blend added: the two trimmed lines and the arc facets
    /// or chamfer segment, in sketch edge-slot order.
    pub new_edges: Vec<SketchEdgeId>,
}

/// The geometry of a corner blend, computed once: every new point is
/// derived here and referred to by index afterwards (shared by
/// construction).
#[derive(Debug, Clone)]
pub(crate) struct CornerPlan {
    /// The two incident edges, in counter-clockwise order.
    e1: SketchEdgeId,
    e2: SketchEdgeId,
    /// The far ends of `e1` and `e2`.
    f1: Point3,
    f2: Point3,
    /// The tangent points, on `e1` and `e2`.
    t1: Point3,
    t2: Point3,
    /// The circle of a round (`None` for a chamfer).
    circle: Option<CurveGeom>,
    /// The arc points `A_0 = t1 … A_N = t2` (empty for a chamfer).
    arc: Vec<Point3>,
}

impl CornerPlan {
    /// The far end of the first (`0`) or second (any other) edge.
    pub(crate) fn far_end(&self, which: usize) -> Point3 {
        if which == 0 { self.f1 } else { self.f2 }
    }

    /// Every point the blend adds to the drawing: the tangent points and the
    /// arc between them.
    pub(crate) fn new_points(&self) -> Vec<Point3> {
        if self.arc.is_empty() {
            vec![self.t1, self.t2]
        } else {
            self.arc.clone()
        }
    }
}

impl Sketch {
    /// Blends the corner at `vertex`. The two incident edges are trimmed back
    /// to the tangent points, and the arc (a new curve chain with its
    /// [`CurveGeom`], kind circle) or the chamfer segment joins them.
    /// Regions re-derive as for any sketch edit.
    ///
    /// # Errors
    /// - [`SketchError::UnknownVertex`] — stale handle.
    /// - [`SketchError::SegmentsBelowFloor`] / [`SketchError::SegmentsAboveCap`]
    ///   — a round's `segments` outside `1..=1024`.
    /// - [`SketchError::DegenerateCurve`] — a round's radius is not finite or
    ///   not larger than [`tol::POINT_MERGE`].
    /// - [`SketchError::DegenerateSegment`] — a chamfer setback is not finite
    ///   or not larger than [`tol::POINT_MERGE`].
    /// - [`SketchError::CornerNotBlendable`] — `vertex` does not have exactly
    ///   two incident edges, or either edge belongs to a curve chain (an
    ///   arc or circle facet, or a polygon side).
    /// - [`SketchError::CornerIsStraight`] — the edges are collinear (the
    ///   interior angle is within [`tol::NORMAL_DIRECTION`] of a half turn).
    /// - [`SketchError::CornerRadiusTooLarge`] — a tangent point would reach
    ///   or pass the far end of an edge (within [`tol::POINT_MERGE`] of it);
    ///   `max` is the largest radius (the largest uniform scale of both
    ///   setbacks for a chamfer) that fits.
    /// - Anything the sticky insertion of the new edges refuses, unchanged.
    ///
    /// Strong guarantee: the edit runs on a clone that is swapped in only
    /// when it succeeds. Curve handles other than the new chain are
    /// unchanged; the two removed edges' handles go stale.
    pub fn blend_corner(
        &mut self,
        vertex: SketchVertexId,
        profile: CornerProfile,
    ) -> Result<CornerBlendReport, SketchError> {
        let plan = self.corner_plan(vertex, profile)?;
        let mut work = self.clone();
        work.remove_edge(plan.e1)?;
        work.remove_edge(plan.e2)?;

        let mut segs: Vec<(Point3, Point3, Option<SketchCurveId>)> = Vec::new();
        segs.push((plan.f1, plan.t1, None));
        let curve = match plan.circle {
            Some(geom) => {
                let curve = work.begin_curve_with(geom)?;
                for w in plan.arc.windows(2) {
                    segs.push((w[0], w[1], Some(curve)));
                }
                Some(curve)
            }
            None => {
                segs.push((plan.t1, plan.t2, None));
                None
            }
        };
        segs.push((plan.t2, plan.f2, None));
        let added = work.add_segments_on_curves(&segs);
        work.end_curve();
        added?;

        let new_edges: Vec<SketchEdgeId> = work
            .edges()
            .keys()
            .filter(|&e| !self.edges().contains_key(e))
            .collect();
        *self = work;
        Ok(CornerBlendReport { curve, new_edges })
    }

    /// The corners [`Sketch::blend_corner`] accepts: vertices with exactly
    /// two incident edges, neither on a curve chain, that are not collinear.
    /// In vertex slot order.
    pub fn blendable_corners(&self) -> Vec<SketchVertexId> {
        let mut incident: std::collections::BTreeMap<SketchVertexId, Vec<SketchEdgeId>> =
            std::collections::BTreeMap::new();
        for (id, e) in self.edges() {
            incident.entry(e.from).or_default().push(id);
            incident.entry(e.to).or_default().push(id);
        }
        self.vertices()
            .keys()
            .filter(|v| {
                incident
                    .get(v)
                    .is_some_and(|edges| self.corner_geometry(*v, edges).is_ok())
            })
            .collect()
    }

    /// Plans the blend at `vertex` without changing anything: validates
    /// `profile` and the corner, and computes every new point once.
    pub(crate) fn corner_plan(
        &self,
        vertex: SketchVertexId,
        profile: CornerProfile,
    ) -> Result<CornerPlan, SketchError> {
        if !self.vertices().contains_key(vertex) {
            return Err(SketchError::UnknownVertex);
        }
        match profile {
            CornerProfile::Round { radius, segments } => {
                if segments < 1 {
                    return Err(SketchError::SegmentsBelowFloor);
                }
                if segments as usize > MAX_CIRCLE_SEGMENTS {
                    return Err(SketchError::SegmentsAboveCap);
                }
                if !radius.is_finite() || radius <= tol::POINT_MERGE {
                    return Err(SketchError::DegenerateCurve);
                }
            }
            CornerProfile::Chamfer {
                setback_a,
                setback_b,
            } => {
                if [setback_a, setback_b]
                    .iter()
                    .any(|s| !s.is_finite() || *s <= tol::POINT_MERGE)
                {
                    return Err(SketchError::DegenerateSegment);
                }
            }
        }
        let edges: Vec<SketchEdgeId> = self
            .edges()
            .iter()
            .filter(|(_, e)| e.from == vertex || e.to == vertex)
            .map(|(id, _)| id)
            .collect();
        let g = self.corner_geometry(vertex, &edges)?;
        let v = self.vertices()[vertex].position;
        let n = self.plane().normal();
        // Seen from the normal side, `e1 → V → e2` must turn counter-clockwise,
        // that is cross(d1, d2) · n < 0 (d1, d2 point away from V).
        let (e1, e2, f1, f2, d1, d2, l1, l2) = if g.d1.cross(g.d2).dot(n) <= 0.0 {
            (g.e1, g.e2, g.f1, g.f2, g.d1, g.d2, g.l1, g.l2)
        } else {
            (g.e2, g.e1, g.f2, g.f1, g.d2, g.d1, g.l2, g.l1)
        };
        let half = g.theta / 2.0;
        let room1 = l1 - tol::POINT_MERGE;
        let room2 = l2 - tol::POINT_MERGE;
        match profile {
            CornerProfile::Round { radius, segments } => {
                let s = radius / half.tan();
                if !(s < room1 && s < room2) {
                    let max = (room1.min(room2) * half.tan()).max(0.0);
                    return Err(SketchError::CornerRadiusTooLarge { max });
                }
                let t1 = v + d1 * s;
                let t2 = v + d2 * s;
                let bisector = (d1 + d2)
                    .normalized()
                    .map_err(|_| SketchError::CornerIsStraight)?;
                let center = v + bisector * (radius / half.sin());
                let (u1, u2) = (t1 - center, t2 - center);
                let sweep = std::f64::consts::PI - g.theta;
                let sign = if u1.cross(u2).dot(n) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                let tangent: Vec3 = n.cross(u1) * sign;
                let count = segments as usize;
                let mut arc = Vec::with_capacity(count + 1);
                arc.push(t1);
                for k in 1..count {
                    let a = sweep * k as f64 / count as f64;
                    arc.push(center + u1 * a.cos() + tangent * a.sin());
                }
                arc.push(t2);
                Ok(CornerPlan {
                    e1,
                    e2,
                    f1,
                    f2,
                    t1,
                    t2,
                    circle: Some(CurveGeom { center, radius }),
                    arc,
                })
            }
            CornerProfile::Chamfer {
                setback_a,
                setback_b,
            } => {
                if !(setback_a < room1 && setback_b < room2) {
                    let max = (room1 / setback_a).min(room2 / setback_b).max(0.0);
                    return Err(SketchError::CornerRadiusTooLarge { max });
                }
                Ok(CornerPlan {
                    e1,
                    e2,
                    f1,
                    f2,
                    t1: v + d1 * setback_a,
                    t2: v + d2 * setback_b,
                    circle: None,
                    arc: Vec::new(),
                })
            }
        }
    }

    /// The two straight edges at `vertex` and their directions, when it is a
    /// blendable corner.
    fn corner_geometry(
        &self,
        vertex: SketchVertexId,
        edges: &[SketchEdgeId],
    ) -> Result<CornerGeometry, SketchError> {
        let [e1, e2] = edges else {
            return Err(SketchError::CornerNotBlendable);
        };
        if self.edge_curve(*e1).is_some() || self.edge_curve(*e2).is_some() {
            return Err(SketchError::CornerNotBlendable);
        }
        let v = self.vertices()[vertex].position;
        let far = |e: SketchEdgeId| {
            let ed = self.edges()[e];
            let other = if ed.from == vertex { ed.to } else { ed.from };
            self.vertices()[other].position
        };
        let (f1, f2) = (far(*e1), far(*e2));
        let (w1, w2) = (f1 - v, f2 - v);
        let (l1, l2) = (w1.length(), w2.length());
        let d1 = w1
            .normalized()
            .map_err(|_| SketchError::CornerNotBlendable)?;
        let d2 = w2
            .normalized()
            .map_err(|_| SketchError::CornerNotBlendable)?;
        let theta = d1.cross(d2).length().atan2(d1.dot(d2));
        if std::f64::consts::PI - theta <= tol::NORMAL_DIRECTION {
            return Err(SketchError::CornerIsStraight);
        }
        if theta <= tol::NORMAL_DIRECTION {
            // Two edges leaving in the same direction cannot coexist under
            // the sticky rules; refuse rather than divide by a zero angle.
            return Err(SketchError::CornerNotBlendable);
        }
        Ok(CornerGeometry {
            e1: *e1,
            e2: *e2,
            f1,
            f2,
            d1,
            d2,
            l1,
            l2,
            theta,
        })
    }
}

/// A corner's two edges, far ends, unit directions away from the corner,
/// lengths, and interior angle `θ ∈ (0, π)`.
struct CornerGeometry {
    e1: SketchEdgeId,
    e2: SketchEdgeId,
    f1: Point3,
    f2: Point3,
    d1: Vec3,
    d2: Vec3,
    l1: f64,
    l2: f64,
    theta: f64,
}
