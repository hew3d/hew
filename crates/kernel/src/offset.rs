//! Uniform boundary offset of a closed planar profile (the Offset tool).
//!
//! Given a [`Profile`] — a validated closed planar polygon with holes, with
//! optional per-edge analytic curve attribution — [`offset_profile`] computes
//! a new set of boundary loops in the same plane, each point of the original
//! boundary moved a uniform `distance` away from (positive) or toward
//! (negative) the profile's material.
//!
//! Semantics, matching the classic SketchUp Offset tool:
//!
//! - **Every loop offsets.** A positive distance grows the material: the
//!   outer boundary expands and every hole shrinks. A negative distance
//!   shrinks it: the outer contracts and holes grow. The offset band between
//!   the original and offset boundaries has uniform width `|distance|`
//!   along every edge.
//! - **Straight edges offset to parallel lines**, joined at corners by miter
//!   joins (the intersection of the two adjacent offset lines) — bounded by
//!   a miter limit: a join displaced more than
//!   [`tol::OFFSET_MITER_LIMIT`] × `|distance|` from its source vertex (a
//!   needle corner, whose miter grows as `d / sin(θ/2)` without bound) is a
//!   typed refusal, never an emitted spike.
//! - **Curve facets offset analytically.** An edge claiming a [`CurveGeom`]
//!   is treated as a chord facet of that circle: its vertices map radially
//!   onto the concentric circle of radius `r ± distance` (sign chosen so the
//!   arc moves away from the material), and the output edge carries the
//!   offset circle as its own [`CurveGeom`] — the result is a true curve,
//!   not loose facets.
//! - **Junction vertices** (where a straight edge meets an arc, or two
//!   different arcs meet) land on the exact intersection of the two offset
//!   primitives. When the two sides' exact endpoint images coincide (an
//!   arc-interior vertex, a tangent join such as a rounded rectangle's, or
//!   collinear straight edges) the shared image is used directly — no
//!   intersection is solved, so tangency survives exactly.
//! - **No silent repair, and no silently absurd success** (DEVELOPMENT.md
//!   rule 4). A distance that collapses a loop (larger than the inradius,
//!   an arc radius driven to zero or negative, a self-intersecting or
//!   winding-flipped result, holes colliding with the outer boundary) — or
//!   a corner too sharp for its miter to stay within the limit above — is a
//!   typed [`OffsetError`]; nothing is clamped, dropped, or reordered to
//!   force success, and nothing geometrically unreasonable is emitted just
//!   because it happens to validate.
//!
//! The output is validated by constructing a [`Profile`] over the offset
//! loops, so every invariant `Profile::new` establishes (simple loops,
//! correct winding, holes strictly inside the outer and disjoint) holds for
//! the result by construction.
//!
//! [`offset_face_boundary`] is the solid-face entry point: it recovers the
//! outer boundary of an `Object`'s face — with analytic attribution from the
//! face's own imprinted edge claims and from adjacent stamped cylinder walls
//! — and offsets it in the face plane, producing the loop a face imprint
//! (`Object::split_face_inner_with_curve`) then commits.

use crate::geom2d::plane_axes;
use crate::ids::FaceId;
use crate::math::{Point3, Vec3};
use crate::sketch::{CurveGeom, Profile};
use crate::tol;
use crate::topo::{Object, SurfaceRef};

/// One offset boundary loop: `points[k] → points[k+1]` (cyclic) is edge `k`,
/// carrying the offset circle it is a facet of when the source edge belonged
/// to an analytic curve.
#[derive(Debug, Clone, PartialEq)]
pub struct OffsetLoop {
    /// Loop vertices in the source loop's cyclic order (outer loops CCW,
    /// hole loops CW, seen from the plane normal side). Index-aligned with
    /// the source loop's vertices except where junction trimming removed
    /// arc vertices that fell beyond a miter (see [`offset_profile`]).
    pub points: Vec<Point3>,
    /// Per-edge analytic attribution, parallel to `points`.
    pub curves: Vec<Option<CurveGeom>>,
}

/// The full result of offsetting a profile: the outer loop's image and each
/// hole loop's image.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileOffset {
    /// The offset image of the outer boundary.
    pub outer: OffsetLoop,
    /// The offset images of the hole boundaries, in source order.
    pub holes: Vec<OffsetLoop>,
}

/// Typed failures of [`offset_profile`] / [`offset_face_boundary`]. Nothing
/// is repaired silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetError {
    /// `|distance|` is not larger than
    /// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE), or is not finite — the
    /// offset boundary would coincide with the original.
    OffsetTooSmall,
    /// The offset collapses the profile: the distance exceeds what the shape
    /// can absorb (an inward offset past the inradius, an arc radius driven
    /// to zero, a hole shrunk away, adjacent offset primitives that no
    /// longer intersect), the offset loops cross themselves, each other, or
    /// flip their winding; a corner is too sharp for its miter join to stay
    /// within [`tol::OFFSET_MITER_LIMIT`](crate::tol::OFFSET_MITER_LIMIT) ×
    /// `|distance|` of its vertex (a needle dart's spike); or a boundary
    /// facet's analytic claim is unresolvable (a chord through its own
    /// circle's center leaves the material side undecidable, never guessed
    /// from noise).
    OffsetCollapsed,
}

impl std::fmt::Display for OffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OffsetError::OffsetTooSmall => {
                write!(f, "offset distance is too small to produce a boundary")
            }
            OffsetError::OffsetCollapsed => {
                write!(f, "the offset distance collapses the boundary")
            }
        }
    }
}

impl std::error::Error for OffsetError {}

/// Typed failures of [`offset_face_boundary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceOffsetError {
    /// The face handle is stale.
    UnknownFace,
    /// The offset itself failed (see [`OffsetError`]).
    Offset(OffsetError),
}

impl std::fmt::Display for FaceOffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FaceOffsetError::UnknownFace => write!(f, "no such face on this object"),
            FaceOffsetError::Offset(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for FaceOffsetError {}

/// Offsets every boundary loop of `profile` by `distance`: positive grows
/// the material (outer expands, holes shrink), negative shrinks it. See the
/// module docs for the full semantics.
///
/// # Errors
/// [`OffsetError::OffsetTooSmall`] if `|distance|` is not larger than
/// [`tol::POINT_MERGE`](crate::tol::POINT_MERGE) or not finite;
/// [`OffsetError::OffsetCollapsed`] if the offset result is not a valid
/// profile (see the variant docs). Pure function — no state to leave
/// untouched.
pub fn offset_profile(profile: &Profile, distance: f64) -> Result<ProfileOffset, OffsetError> {
    let normal = profile.plane().normal();

    // As stored, both loop kinds have the material on the LEFT of their
    // traversal direction (outer CCW, holes CW, seen from the normal side),
    // so one signed offset rule covers every loop: move each boundary point
    // `distance` along the local outward direction `dir × normal`.
    let outer_curves: Vec<Option<CurveGeom>> = (0..profile.outer().len())
        .map(|k| profile.outer_curve(k))
        .collect();
    let outer = offset_loop(profile.outer(), &outer_curves, normal, distance)?;

    let mut holes: Vec<OffsetLoop> = Vec::with_capacity(profile.holes().len());
    for (i, hole) in profile.holes().iter().enumerate() {
        let hole_curves: Vec<Option<CurveGeom>> =
            (0..hole.len()).map(|k| profile.hole_curve(i, k)).collect();
        holes.push(offset_loop(hole, &hole_curves, normal, distance)?);
    }

    // Validate the result as a whole by constructing a Profile over it: this
    // establishes simplicity, winding, loop-loop separation, and hole
    // containment in one place, with the exact invariants extrusion relies
    // on. Any rejection means the distance collapsed the shape.
    Profile::new(
        profile.plane(),
        outer.points.clone(),
        holes.iter().map(|h| h.points.clone()).collect(),
    )
    .map_err(|_| OffsetError::OffsetCollapsed)?;

    Ok(ProfileOffset { outer, holes })
}

/// Offsets the outer boundary of `face` by `distance` in the face plane:
/// negative moves the boundary into the face's interior (the loop a
/// boss/recess imprint commits), positive moves it outside. The boundary's
/// analytic attribution is recovered from two sources, in order: an edge's
/// own [`CurveGeom`](crate::topo::Edge::curve) claim (imprinted circles),
/// else the adjacent face's stamped
/// [`SurfaceRef::Cylinder`](crate::topo::SurfaceRef) when its axis is
/// parallel to this face's normal and the edge's endpoints lie on the
/// cylinder's circle in this plane (a drawn cylinder's cap rim). Edges with
/// neither offset as straight lines.
///
/// Pure query — the object is not mutated; committing the returned loop is
/// the caller's move (`Object::split_face_inner_with_curve`), which enforces
/// containment in the face and the object's own invariants.
///
/// # Errors
/// [`FaceOffsetError::UnknownFace`] for a stale handle;
/// [`FaceOffsetError::Offset`] when the offset itself fails or the resulting
/// loop is not simple / flips its winding.
pub fn offset_face_boundary(
    object: &Object,
    face: FaceId,
    distance: f64,
) -> Result<OffsetLoop, FaceOffsetError> {
    let f = object
        .faces()
        .get(face)
        .ok_or(FaceOffsetError::UnknownFace)?;
    let normal = f.plane.normal();
    let pts: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
    let n = pts.len();

    // Per-edge analytic attribution. `loop_half_edges` walks in the same
    // order as `loop_positions`, so half-edge k spans pts[k] → pts[k+1].
    let half_edges: Vec<_> = object.loop_half_edges(f.outer_loop).collect();
    let curves: Vec<Option<CurveGeom>> = half_edges
        .iter()
        .enumerate()
        .map(|(k, &h)| {
            let he = object.half_edges()[h];
            // An imprinted circle's own edge claim is authoritative.
            if let Some(g) = object.edges()[he.edge].curve {
                return Some(g);
            }
            // A cap rim edge: the neighboring wall carries the cylinder.
            let twin = he.twin?;
            let neighbor = object.loops()[object.half_edges()[twin].loop_id].face;
            let SurfaceRef::Cylinder {
                axis_point,
                axis,
                radius,
            } = object.faces()[neighbor].surface?;
            // The cap circle only exists where the axis meets this plane
            // perpendicularly.
            let along = axis.dot(normal);
            if along.abs() < 1.0 - tol::NORMAL_DIRECTION {
                return None;
            }
            let center = axis_point + axis * (-f.plane.signed_distance(axis_point) / along);
            // The claim must describe this edge (both endpoints on the
            // circle) — a slant-cut rim is not a facet of the cap circle.
            let claim_tol = object.planarity_tol;
            let on_circle = |p: Point3| ((p - center).length() - radius).abs() <= claim_tol;
            if on_circle(pts[k]) && on_circle(pts[(k + 1) % n]) {
                Some(CurveGeom { center, radius })
            } else {
                None
            }
        })
        .collect();

    let lp = offset_loop(&pts, &curves, normal, distance).map_err(FaceOffsetError::Offset)?;

    // Loop-local validation (simplicity, winding). Containment in the face
    // is the imprint's own check at commit time.
    Profile::new(f.plane, lp.points.clone(), Vec::new())
        .map_err(|_| FaceOffsetError::Offset(OffsetError::OffsetCollapsed))?;
    Ok(lp)
}

/// The offset primitive an edge belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Prim {
    /// A straight edge: offsets to the parallel line through
    /// `point + outward * d` with direction `dir` (unit, in-plane).
    Line { point: Point3, dir: Vec3 },
    /// A facet of an analytic circle: offsets to the concentric circle of
    /// radius `offset_radius` (already signed against the material side).
    Circle { center: Point3, offset_radius: f64 },
}

/// Offsets one loop. `pts[k] → pts[k+1]` (cyclic) is edge `k`; `curves[k]`
/// is its analytic attribution. Material lies to the LEFT of the traversal
/// (both stored loop kinds satisfy this), so positive `d` moves every edge
/// along `dir × normal` — away from the material.
pub(crate) fn offset_loop(
    pts: &[Point3],
    curves: &[Option<CurveGeom>],
    normal: Vec3,
    d: f64,
) -> Result<OffsetLoop, OffsetError> {
    if !d.is_finite() || d.abs() <= tol::POINT_MERGE {
        return Err(OffsetError::OffsetTooSmall);
    }
    let n = pts.len();
    debug_assert_eq!(curves.len(), n);
    if n < 3 {
        return Err(OffsetError::OffsetCollapsed);
    }

    // ── Per-edge offset primitives ────────────────────────────────────────
    // For a straight edge the primitive is its offset line. For a curve
    // facet it is the offset circle, with the radius sign resolved per edge
    // from which side of the facet the material is on (a convex boundary
    // arc bulges away from its center, a concave one toward it).
    let mut prims: Vec<Prim> = Vec::with_capacity(n);
    for k in 0..n {
        let a = pts[k];
        let b = pts[(k + 1) % n];
        let dir = (b - a)
            .normalized()
            .map_err(|_| OffsetError::OffsetCollapsed)?;
        let outward = dir.cross(normal);
        match curves[k] {
            Some(g) => {
                let mid = a + (b - a) * 0.5;
                let radial = mid - g.center;
                // A facet whose chord midpoint lies on the circle center
                // subtends (numerically) a half turn: which side of the
                // circle the material is on cannot be read from this facet,
                // and the sign test below must not be decided by noise
                // (rule 6). Refuse typed rather than guess — no drawing
                // tool produces such a facet (the density floor keeps
                // spans at a few degrees), so this only gates hand-built
                // coarse chains whose claim is not offsetable anyway.
                if radial.length() <= tol::POINT_MERGE {
                    return Err(OffsetError::OffsetCollapsed);
                }
                // Outward agrees with radial ⇒ the material is on the
                // center's side ⇒ growing the material grows the radius.
                let offset_radius = if radial.dot(outward) >= 0.0 {
                    g.radius + d
                } else {
                    g.radius - d
                };
                if !offset_radius.is_finite() || offset_radius <= tol::POINT_MERGE {
                    return Err(OffsetError::OffsetCollapsed);
                }
                prims.push(Prim::Circle {
                    center: g.center,
                    offset_radius,
                });
            }
            None => prims.push(Prim::Line {
                point: a + outward * d,
                dir,
            }),
        }
    }

    // ── Per-vertex images ─────────────────────────────────────────────────
    // Vertex k joins edge k-1 (incoming) and edge k (outgoing). Each side
    // has an exact image of the vertex on its own offset primitive; when
    // the two images coincide (arc-interior vertices, tangent joins,
    // collinear straight edges) that shared image is the offset vertex, and
    // otherwise the vertex lands on the exact intersection of the two
    // primitives nearest the original vertex (the miter join) — bounded by
    // the miter limit below.
    let mut out_pts: Vec<Point3> = Vec::with_capacity(n);
    for k in 0..n {
        let prev = prims[(k + n - 1) % n];
        let next = prims[k];
        let v = pts[k];

        let img_prev = vertex_image(v, prev, normal, d)?;
        let img_next = vertex_image(v, next, normal, d)?;

        let p = if img_prev.approx_eq(img_next, tol::POINT_MERGE) {
            img_prev
        } else {
            let join =
                intersect_prims(prev, next, v, normal).ok_or(OffsetError::OffsetCollapsed)?;
            // Miter limit ([`tol::OFFSET_MITER_LIMIT`]): a uniform offset
            // moves every EDGE by exactly |d|, but a needle corner's miter
            // vertex runs away as d/sin(θ/2) — unbounded, while still
            // producing a loop that passes every simplicity/winding check.
            // A join displaced past the limit is refused typed, never
            // emitted as a silently absurd Ok (rule 4's posture).
            if (join - v).length() > tol::OFFSET_MITER_LIMIT * d.abs() {
                return Err(OffsetError::OffsetCollapsed);
            }
            join
        };
        out_pts.push(p);
    }

    // ── Junction trimming on arc runs ─────────────────────────────────────
    // A non-tangent junction's miter is the true corner of the offset loop,
    // and it can land INSIDE an adjacent arc's angular span (shrinking a pie
    // wedge pulls the spoke's offset line across the arc's first facets).
    // The arc stations that fell beyond the junction are cut off — classic
    // offset trimming, applied per analytic run: interior vertices survive
    // only when their station lies strictly between the run's two terminal
    // (junction) images, measured along the run's source sweep direction.
    // A full-circle loop is one uniform run with no junctions and trims
    // nothing.
    let mut keep = vec![true; n];
    if prims.iter().any(|p| *p != prims[0]) {
        let (u_ax, v_ax) = plane_axes(normal);
        let angle_of = |p: Point3, c: Point3| -> f64 {
            let r = p - c;
            r.dot(v_ax).atan2(r.dot(u_ax))
        };
        // Angular progress from `from` in sweep direction `s`, in [0, τ).
        let progress = |theta: f64, from: f64, s: f64| -> f64 {
            let x = (s * (theta - from)) % std::f64::consts::TAU;
            if x < 0.0 {
                x + std::f64::consts::TAU
            } else {
                x
            }
        };
        // Walk maximal runs, starting at a prim boundary so no run wraps
        // the seam.
        let start = (0..n)
            .find(|&k| prims[(k + n - 1) % n] != prims[k])
            .expect("a non-uniform loop has a prim boundary");
        let mut k = 0;
        while k < n {
            let e0 = (start + k) % n;
            let prim = prims[e0];
            let mut len = 1;
            while k + len < n && prims[(start + k + len) % n] == prim {
                len += 1;
            }
            if let Prim::Circle { center, .. } = prim
                && len >= 2
            {
                // Sweep direction from the first source facet (a chord
                // subtends less than a half turn by construction).
                let a0 = angle_of(pts[e0], center);
                let a1 = angle_of(pts[(e0 + 1) % n], center);
                let s = if progress(a1, a0, 1.0) <= std::f64::consts::PI {
                    1.0
                } else {
                    -1.0
                };
                let th0 = angle_of(out_pts[e0], center);
                let span = progress(angle_of(out_pts[(e0 + len) % n], center), th0, s);
                for j in 1..len {
                    let vi = (e0 + j) % n;
                    let a = progress(angle_of(out_pts[vi], center), th0, s);
                    if a <= 0.0 || a >= span {
                        keep[vi] = false;
                    }
                }
            }
            k += len;
        }
    }

    // ── Assemble the surviving loop, with per-edge attribution ────────────
    // A dropped vertex only ever sits strictly inside an analytic run, so
    // the edges it merged shared one attribution: the edge starting at each
    // surviving vertex keeps that vertex's outgoing prim.
    let mut points: Vec<Point3> = Vec::with_capacity(n);
    let mut out_prims: Vec<Prim> = Vec::with_capacity(n);
    for k in 0..n {
        if keep[k] {
            points.push(out_pts[k]);
            out_prims.push(prims[k]);
        }
    }
    let m = points.len();
    if m < 3 {
        return Err(OffsetError::OffsetCollapsed);
    }

    // ── Orientation guard ─────────────────────────────────────────────────
    // An offset pushed past what the shape can absorb can invert a loop
    // while staying simple and correctly wound (a square offset inward past
    // its inradius inverts through its own center). The tell is a straight
    // edge whose image runs OPPOSITE its source direction — refuse it.
    // Analytic edges cannot invert here: trimming keeps their stations
    // strictly monotone between the run terminals.
    for j in 0..m {
        if let Prim::Line { dir, .. } = out_prims[j]
            && (points[(j + 1) % m] - points[j]).dot(dir) <= 0.0
        {
            return Err(OffsetError::OffsetCollapsed);
        }
    }

    let curves: Vec<Option<CurveGeom>> = out_prims
        .iter()
        .map(|p| match *p {
            Prim::Circle {
                center,
                offset_radius,
            } => Some(CurveGeom {
                center,
                radius: offset_radius,
            }),
            Prim::Line { .. } => None,
        })
        .collect();

    // Debug-build invariant (rule 2's spirit for a pure computation): every
    // output edge claiming a circle has both endpoints on that circle —
    // the same claim the solid-side validator holds `Edge::curve` to.
    #[cfg(debug_assertions)]
    for j in 0..m {
        if let Some(g) = curves[j] {
            for p in [points[j], points[(j + 1) % m]] {
                debug_assert!(
                    ((p - g.center).length() - g.radius).abs() <= tol::PLANE_DIST,
                    "offset facet endpoint left its claimed circle"
                );
            }
        }
    }

    Ok(OffsetLoop { points, curves })
}

/// The exact image of vertex `v` under one adjacent edge's offset: moved
/// along the edge's outward normal for a line, radially onto the offset
/// circle for an arc facet.
fn vertex_image(v: Point3, prim: Prim, normal: Vec3, d: f64) -> Result<Point3, OffsetError> {
    match prim {
        // `v` is an endpoint of the edge, so its image lies on the offset
        // line exactly: the whole edge translates by `outward * d`.
        Prim::Line { dir, .. } => Ok(v + dir.cross(normal) * d),
        Prim::Circle {
            center,
            offset_radius,
        } => {
            let radial = (v - center)
                .normalized()
                .map_err(|_| OffsetError::OffsetCollapsed)?;
            Ok(center + radial * offset_radius)
        }
    }
}

/// The intersection of two offset primitives nearest `v`, in the plane with
/// unit `normal`, or `None` when they do not intersect (the join collapses).
fn intersect_prims(a: Prim, b: Prim, v: Point3, normal: Vec3) -> Option<Point3> {
    let (u_ax, v_ax) = plane_axes(normal);
    let proj = |p: Point3| -> (f64, f64) { (p.to_vec().dot(u_ax), p.to_vec().dot(v_ax)) };
    // {u, v, normal} is orthonormal: reconstruct with the shared normal
    // component so results land back on the profile plane exactly.
    let w = v.to_vec().dot(normal);
    let lift = |x: f64, y: f64| -> Point3 { Point3::ORIGIN + u_ax * x + v_ax * y + normal * w };
    let (vx, vy) = proj(v);
    let nearest = |cands: [(f64, f64); 2]| -> Option<Point3> {
        cands
            .into_iter()
            .min_by(|p, q| {
                let dp = (p.0 - vx).powi(2) + (p.1 - vy).powi(2);
                let dq = (q.0 - vx).powi(2) + (q.1 - vy).powi(2);
                dp.total_cmp(&dq)
            })
            .map(|(x, y)| lift(x, y))
    };

    match (a, b) {
        (Prim::Line { point: p1, dir: d1 }, Prim::Line { point: p2, dir: d2 }) => {
            let (p1x, p1y) = proj(p1);
            let (p2x, p2y) = proj(p2);
            let (d1x, d1y) = (d1.dot(u_ax), d1.dot(v_ax));
            let (d2x, d2y) = (d2.dot(u_ax), d2.dot(v_ax));
            let denom = d1x * d2y - d1y * d2x;
            // Both directions are unit vectors, so the cross magnitude is
            // the sine of the angle between them: parallel offset lines
            // whose exact images did not already coincide cannot be joined.
            if denom.abs() <= tol::NORMAL_DIRECTION {
                return None;
            }
            let t = ((p2x - p1x) * d2y - (p2y - p1y) * d2x) / denom;
            Some(lift(p1x + d1x * t, p1y + d1y * t))
        }
        (
            Prim::Line { point, dir },
            Prim::Circle {
                center,
                offset_radius,
            },
        )
        | (
            Prim::Circle {
                center,
                offset_radius,
            },
            Prim::Line { point, dir },
        ) => {
            let (px, py) = proj(point);
            let (dx, dy) = (dir.dot(u_ax), dir.dot(v_ax));
            let (cx, cy) = proj(center);
            // |p + t·d − c|² = r², with |d| = 1.
            let (ox, oy) = (px - cx, py - cy);
            let bq = ox * dx + oy * dy;
            let cq = ox * ox + oy * oy - offset_radius * offset_radius;
            let disc = bq * bq - cq;
            if disc < 0.0 {
                return None;
            }
            let s = disc.sqrt();
            nearest([
                (px + dx * (-bq - s), py + dy * (-bq - s)),
                (px + dx * (-bq + s), py + dy * (-bq + s)),
            ])
        }
        (
            Prim::Circle {
                center: c1,
                offset_radius: r1,
            },
            Prim::Circle {
                center: c2,
                offset_radius: r2,
            },
        ) => {
            let (x1, y1) = proj(c1);
            let (x2, y2) = proj(c2);
            let (ex, ey) = (x2 - x1, y2 - y1);
            let d2 = ex * ex + ey * ey;
            if d2 <= tol::POINT_MERGE * tol::POINT_MERGE {
                // Concentric circles of different radii never intersect
                // (equal radii coincide, which the caller's coincidence
                // branch already handled).
                return None;
            }
            let dist = d2.sqrt();
            // Standard two-circle intersection.
            let along = (d2 + r1 * r1 - r2 * r2) / (2.0 * dist);
            let h2 = r1 * r1 - along * along;
            if h2 < 0.0 {
                return None;
            }
            let h = h2.sqrt();
            let (ux, uy) = (ex / dist, ey / dist);
            let (mx, my) = (x1 + ux * along, y1 + uy * along);
            nearest([(mx - uy * h, my + ux * h), (mx + uy * h, my - ux * h)])
        }
    }
}

// ═════════════════════════════════════════════════════════════════════ tests

#[cfg(test)]
mod tests {
    use super::*;

    /// A facet subtending exactly half a turn has its chord midpoint ON the
    /// claimed circle's center, so the material-side test's dot product is
    /// pure floating-point noise. Before the guard this could pick a radius
    /// sign by that noise and emit a "valid" loop (observed: this exact
    /// input offset to radius `r + d` on a noise-signed decision); now it
    /// refuses typed. Direct `offset_loop` exercise — through a sketch this
    /// shape happens to refuse at insertion, which would mask the guard.
    #[test]
    fn half_turn_facet_refuses_instead_of_noise_signing_the_radius() {
        let center = Point3::new(5.0, 5.0, 0.0);
        let radius = 3.0;
        let p = |deg: f64| {
            let a = deg.to_radians();
            Point3::new(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
                0.0,
            )
        };
        let normal = Vec3::new(0.0, 0.0, 1.0);
        // Facet 1 spans 80° → 260° (exactly half a turn), facet 2 closes to
        // 280°, and a plain chord closes the loop (CCW, material left).
        let pts = vec![p(80.0), p(260.0), p(280.0)];
        let geom = CurveGeom { center, radius };
        let curves = vec![Some(geom), Some(geom), None];

        for d in [0.2, -0.2, 1.0, -1.0] {
            assert_eq!(
                offset_loop(&pts, &curves, normal, d).unwrap_err(),
                OffsetError::OffsetCollapsed,
                "d={d}: the material side of a half-turn facet is undecidable"
            );
        }
    }
}
