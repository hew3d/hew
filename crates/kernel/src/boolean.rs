//! Boolean combination of solids — `Object::boolean` lives in `ops.rs`; this
//! module is its implementation.
//!
//! ARCHITECTURE.md : polygon arrangement reusing the sketch region tracer
//! (not triangle-soup CSG);  extends it to resolve coplanar contact.
//! The work is sliced:
//!
//! - **1a:** operand/transform validation and mapping `b` into `a`'s frame.
//! - **1b:** the unified arrangement pipeline. Every face of each operand is
//!   imprinted with the seam segments where the *other* solid's faces cross it,
//!   partitioned into sub-faces by the sketch region tracer, and each sub-face
//!   kept or dropped (and for subtract, flipped) by testing its interior against
//!   the other solid. The kept sub-faces are welded and rebuilt into a watertight
//!   Object. This single path covers crossing, nested, and disjoint solids alike.
//! - **1c (this commit):** split a disconnected result into one shell per
//!   connected component (subtract that cuts a solid in two; disjoint union), a
//!   hole-robust interior-point test for annular sub-faces, and the property
//!   suite (set-algebra volumes, multi-shell, degenerate refusal).
//!
//! **Coplanar contact is resolved, not refused (ARCHITECTURE.md #19, extends #15).**
//! Each face is also subdivided by the boundaries of any coplanar partner faces,
//! and every sub-face is classified by the coverage-based neighborhood test in
//! [`classify`]: it keeps a sub-face iff inside-result differs across it,
//! evaluating the other operand's membership exactly from a covering coplanar
//! partner (or a ray cast otherwise). Coincident shared patches produced by both
//! operands are deduplicated ([`faces_coincide`]). This handles boxes on a shared
//! ground, shared walls, stacking, coincident solids, and interpenetrating
//! coplanar overlaps.
//!
//! Known gaps (refused as `DegenerateContact`, never silently wrong):
//! - **Face-adjacency with no volume overlap** — two solids that merely *touch*
//!   on a partial coplanar face (e.g. boxes set side-by-side, different heights)
//!   union to a non-watertight weld. The contact patch's boundary lies on the
//!   perpendicular faces' *boundary edges*, which must be split consistently
//!   across both solids; the per-face imprint here does not do that global
//!   intersection-vertex insertion. Fix is a dedicated vertex-consistency pass
//!   (deferred). Subtract/Intersect of this config work. Workaround: overlap the
//!   solids slightly in volume.
//! - **Measure-zero tangency** — an edge lying within a non-coplanar face, a lone
//!   vertex touching a face.
//!
//! Both surface as a clean refusal (a non-watertight weld caught at assembly)
//! rather than a silently wrong result.
//!
//! Why the result is watertight: a seam is the real intersection of two faces,
//! computed once and imprinted identically on both. Where the seam meets a face
//! boundary, the intersection curve continues onto the neighbouring face through
//! the *same* point, so that neighbour is split there too — T-junctions are
//! consistent by construction, and welding coincident vertices pairs the seam
//! half-edges into twins.

use slotmap::SecondaryMap;

use crate::geom2d::{plane_axes, point_inside_polygon};
use crate::ids::FaceId;
use crate::math::{Plane, Point3, Vec3};
use crate::ops::{BooleanError, BooleanOp, Operand};
use crate::sketch::Sketch;
use crate::tol;
use crate::topo::{Object, Shell, WatertightState};
use crate::transform::Transform;

/// A face flattened to position lists for geometric queries: the outer boundary,
/// any hole boundaries, and the supporting (oriented) plane.
struct FacePoly {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    plane: Plane,
}

/// A sub-face produced by the arrangement: oriented loops in the face's plane.
struct Region {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

/// A kept sub-face ready for assembly, with winding already final (flipped for
/// the subtracted operand) and its plane consistent with that winding.
struct OrientedFace {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
    plane: Plane,
}

/// Which operand a batch of faces comes from (selection rules differ for
/// subtract, which is asymmetric).
#[derive(Debug, Clone, Copy)]
enum Side {
    A,
    B,
}

/// Implements [`Object::boolean`]; see that method's contract.
pub(crate) fn execute(
    op: BooleanOp,
    a: &Object,
    b: &Object,
    b_to_a: &Transform,
) -> Result<Object, BooleanError> {
    if a.watertight() != WatertightState::Watertight {
        return Err(BooleanError::OperandNotSolid { which: Operand::A });
    }
    if b.watertight() != WatertightState::Watertight {
        return Err(BooleanError::OperandNotSolid { which: Operand::B });
    }
    if b_to_a.determinant().abs() < tol::NORMALIZE_MIN_LENGTH {
        return Err(BooleanError::SingularTransform);
    }

    // Everything happens in `a`'s frame.
    let b_in_a = transformed_object(b, b_to_a)?;
    let a_polys = face_polys(a);
    let b_polys = face_polys(&b_in_a);

    let mut faces = Vec::new();
    collect_faces(op, Side::A, &a_polys, &b_polys, &mut faces)?;
    collect_faces(op, Side::B, &a_polys, &b_polys, &mut faces)?;

    if faces.is_empty() {
        return Err(BooleanError::EmptyResult);
    }
    assemble(faces)
}

/// A copy of `obj` with `t` baked into every vertex position and face plane.
///
/// Connectivity is unchanged, so the result is structurally valid whenever the
/// input is. A singular or orientation-flipping `t` is refused as
/// [`BooleanError::SingularTransform`] (operands must map by a proper frame
/// change); boolean callers pass a rigid `b_to_a`, so this never fires in
/// practice.
fn transformed_object(obj: &Object, t: &Transform) -> Result<Object, BooleanError> {
    let mut out = obj.clone();
    out.apply_transform(t)
        .map_err(|_| BooleanError::SingularTransform)?;
    Ok(out)
}

/// Flattens every face of `obj` to a [`FacePoly`].
fn face_polys(obj: &Object) -> Vec<FacePoly> {
    obj.faces()
        .values()
        .map(|f| FacePoly {
            outer: obj.loop_positions(f.outer_loop).collect(),
            holes: f
                .inner_loops
                .iter()
                .map(|&l| obj.loop_positions(l).collect())
                .collect(),
            plane: f.plane,
        })
        .collect()
}

/// Arranges each face of one operand against the other, classifies every
/// sub-face by the coverage-based neighborhood test, and pushes the kept
/// (correctly oriented) ones into `out`.
///
/// A face is subdivided by transversal seams (where non-coplanar partner faces
/// cross its plane) *and* by the boundaries of any coplanar partner faces, so
/// each resulting sub-face is uniformly inside-or-outside every partner. Then
/// [`classify`] keeps it iff it separates inside-result from outside-result.
fn collect_faces(
    op: BooleanOp,
    this_side: Side,
    a_polys: &[FacePoly],
    b_polys: &[FacePoly],
    out: &mut Vec<OrientedFace>,
) -> Result<(), BooleanError> {
    let (these, others) = match this_side {
        Side::A => (a_polys, b_polys),
        Side::B => (b_polys, a_polys),
    };
    for fp in these {
        let na = fp.plane.normal();

        // Partner faces of the other operand lying in this face's plane.
        let coplanar: Vec<&FacePoly> = others.iter().filter(|o| coplanar_planes(fp, o)).collect();

        // Imprint: transversal seams (empty for parallel planes) plus coplanar
        // partner boundaries, minus anything coincident with `fp`'s own edges or
        // an already-collected segment.
        let mut seams: Vec<(Point3, Point3)> = Vec::new();
        for ofp in others {
            for seg in seam_segments(fp, ofp) {
                push_unique_segment(&mut seams, seg, fp);
            }
        }
        for cp in &coplanar {
            for seg in loop_edges(&cp.outer).chain(cp.holes.iter().flat_map(|h| loop_edges(h))) {
                push_unique_segment(&mut seams, seg, fp);
            }
        }

        for region in arrange_face(fp, &seams)? {
            let c = interior_point(&region, na);
            // Coplanar partner boundaries can trace regions outside the original
            // face; keep only sub-faces actually inside it.
            if !region_contains(fp, c) {
                continue;
            }
            if let Some(flip) = classify(op, this_side, na, c, a_polys, b_polys, &coplanar) {
                let face = OrientedFace::new(region, fp.plane, flip)?;
                // A coincident shared patch is produced by both operands (e.g.
                // a shared ground for union, or A's top vs B's flipped bottom for
                // subtract). Both describe one boundary face — keep the first.
                if !out.iter().any(|f| faces_coincide(f, &face)) {
                    out.push(face);
                }
            }
        }
    }
    Ok(())
}

/// Whether a sub-face (interior point `c`, owning-face normal `na`, from operand
/// `this_side`) lies on the result's boundary, and with which winding: `Some(true)`
/// to keep it flipped, `Some(false)` to keep as-is, `None` to drop it.
///
/// Membership is evaluated just inside (`-na`) and just outside (`+na`) the face.
/// This operand's membership there is definitional (the point is on its
/// boundary). The other operand's is exact when a coplanar partner covers `c`
/// (it occupies the `-partner_normal` side), else a ray cast (the point is then
/// off the other's boundary, so parity is clean). The face is kept iff
/// inside-result differs across it.
fn classify(
    op: BooleanOp,
    this_side: Side,
    na: Vec3,
    c: Point3,
    a_polys: &[FacePoly],
    b_polys: &[FacePoly],
    coplanar: &[&FacePoly],
) -> Option<bool> {
    let others = match this_side {
        Side::A => b_polys,
        Side::B => a_polys,
    };
    let (other_in, other_out) = match cover_normal(coplanar, c) {
        Some(no) => (na.dot(no) > 0.0, na.dot(no) < 0.0),
        None => {
            let m = point_in_solid(others, c);
            (m, m)
        }
    };
    // `-na` is inside this operand, `+na` is outside.
    let (a_in, a_out, b_in, b_out) = match this_side {
        Side::A => (true, false, other_in, other_out),
        Side::B => (other_in, other_out, true, false),
    };
    let res_in = op_member(op, a_in, b_in);
    let res_out = op_member(op, a_out, b_out);
    if res_in == res_out {
        return None;
    }
    // `res_out` true ⇒ the result interior is on the `+na` side, so reverse.
    Some(res_out)
}

/// Membership of a point in `A op B`, from its memberships in A and B.
fn op_member(op: BooleanOp, a: bool, b: bool) -> bool {
    match op {
        BooleanOp::Union => a || b,
        BooleanOp::Intersect => a && b,
        BooleanOp::Subtract => a && !b,
    }
}

/// Outward normal of the coplanar partner whose region contains `c`, if any.
fn cover_normal(coplanar: &[&FacePoly], c: Point3) -> Option<Vec3> {
    coplanar
        .iter()
        .find(|cp| region_contains(cp, c))
        .map(|cp| cp.plane.normal())
}

/// True if the two faces lie in the same (parallel and coincident) plane.
fn coplanar_planes(fa: &FacePoly, fb: &FacePoly) -> bool {
    if fa.plane.normal().cross(fb.plane.normal()).length() > tol::NORMAL_DIRECTION {
        return false;
    }
    fa.plane.signed_distance(fb.outer[0]).abs() <= tol::PLANE_DIST
}

/// The edges of a closed loop as `(start, end)` pairs.
fn loop_edges(loop_pts: &[Point3]) -> impl Iterator<Item = (Point3, Point3)> + '_ {
    let n = loop_pts.len();
    (0..n).map(move |i| (loop_pts[i], loop_pts[(i + 1) % n]))
}

/// Appends `seg` to `seams` unless it is degenerate, coincides with an edge of
/// `fp`'s own boundary, or duplicates a segment already collected.
fn push_unique_segment(seams: &mut Vec<(Point3, Point3)>, seg: (Point3, Point3), fp: &FacePoly) {
    let (p, q) = seg;
    if p.approx_eq(q, tol::POINT_MERGE) {
        return;
    }
    if loop_edges(&fp.outer)
        .chain(fp.holes.iter().flat_map(|h| loop_edges(h)))
        .any(|(a, b)| seg_eq(p, q, a, b))
    {
        return;
    }
    if seams.iter().any(|&(a, b)| seg_eq(p, q, a, b)) {
        return;
    }
    seams.push(seg);
}

/// Unordered segment equality within [`tol::POINT_MERGE`].
fn seg_eq(a0: Point3, a1: Point3, b0: Point3, b1: Point3) -> bool {
    (a0.approx_eq(b0, tol::POINT_MERGE) && a1.approx_eq(b1, tol::POINT_MERGE))
        || (a0.approx_eq(b1, tol::POINT_MERGE) && a1.approx_eq(b0, tol::POINT_MERGE))
}

/// Whether two oriented faces are the same boundary patch: identical outer
/// vertex set (within tolerance) and the same outward direction. Such a pair
/// arises when both operands claim a coincident coplanar patch; only one belongs
/// in the result.
fn faces_coincide(f: &OrientedFace, g: &OrientedFace) -> bool {
    f.outer.len() == g.outer.len()
        && f.plane.normal().dot(g.plane.normal()) > 0.0
        && f.outer
            .iter()
            .all(|p| g.outer.iter().any(|q| p.approx_eq(*q, tol::POINT_MERGE)))
}

impl OrientedFace {
    fn new(region: Region, plane: Plane, flip: bool) -> Result<OrientedFace, BooleanError> {
        if !flip {
            return Ok(OrientedFace {
                outer: region.outer,
                holes: region.holes,
                plane,
            });
        }
        // Reverse every loop and refit the plane so its normal matches the new
        // winding (the subtracted solid's walls face into the removed volume).
        let mut outer = region.outer;
        outer.reverse();
        let holes = region
            .holes
            .into_iter()
            .map(|mut h| {
                h.reverse();
                h
            })
            .collect();
        let plane = Plane::from_polygon(&outer).map_err(|_| BooleanError::DegenerateContact)?;
        Ok(OrientedFace {
            outer,
            holes,
            plane,
        })
    }
}

// ─────────────────────────────────────────────────────────── seam computation

/// The intersection segments of two non-coplanar faces: the line of their planes
/// clipped to both face regions. Empty if the planes are parallel or the regions
/// do not overlap along the line.
fn seam_segments(fa: &FacePoly, fb: &FacePoly) -> Vec<(Point3, Point3)> {
    let Some((p0, dir)) = plane_line(&fa.plane, &fb.plane) else {
        return Vec::new();
    };
    let ia = line_region_intervals(p0, dir, fa);
    let ib = line_region_intervals(p0, dir, fb);
    let mut out = Vec::new();
    for &(a0, a1) in &ia {
        for &(b0, b1) in &ib {
            let lo = a0.max(b0);
            let hi = a1.min(b1);
            if hi - lo > tol::POINT_MERGE {
                out.push((p0 + dir * lo, p0 + dir * hi));
            }
        }
    }
    out
}

/// A point on, and the unit direction of, the line where two planes meet, or
/// `None` if they are (near) parallel.
fn plane_line(pa: &Plane, pb: &Plane) -> Option<(Point3, Vec3)> {
    let (na, nb) = (pa.normal(), pb.normal());
    let dir = na.cross(nb).normalized().ok()?;
    let c = na.dot(nb);
    let denom = 1.0 - c * c;
    if denom < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let off_a = -pa.signed_distance(Point3::ORIGIN);
    let off_b = -pb.signed_distance(Point3::ORIGIN);
    let p0 =
        Point3::ORIGIN + na * ((off_a - off_b * c) / denom) + nb * ((off_b - off_a * c) / denom);
    Some((p0, dir))
}

/// The parameter intervals (along `dir` from `p0`) where the line lies inside the
/// face region. The line is assumed to lie in the face plane.
fn line_region_intervals(p0: Point3, dir: Vec3, fp: &FacePoly) -> Vec<(f64, f64)> {
    let normal = fp.plane.normal();
    let mut params = Vec::new();
    let mut collect = |loop_pts: &[Point3]| {
        let n = loop_pts.len();
        for i in 0..n {
            if let Some(t) = line_segment_param(p0, dir, loop_pts[i], loop_pts[(i + 1) % n], normal)
            {
                params.push(t);
            }
        }
    };
    collect(&fp.outer);
    for h in &fp.holes {
        collect(h);
    }
    params.sort_by(|a, b| a.partial_cmp(b).unwrap());
    params.dedup_by(|a, b| (*a - *b).abs() <= tol::POINT_MERGE);

    let mut intervals = Vec::new();
    for w in params.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        if hi - lo <= tol::POINT_MERGE {
            continue;
        }
        let mid = p0 + dir * (0.5 * (lo + hi));
        if region_contains(fp, mid) {
            intervals.push((lo, hi));
        }
    }
    intervals
}

/// The parameter `t` along the line `p0 + t·dir` where it crosses the segment
/// `v0`–`v1` (both in the same plane, spanned by `normal`), or `None` if they are
/// parallel or the crossing is off the segment.
fn line_segment_param(p0: Point3, dir: Vec3, v0: Point3, v1: Point3, normal: Vec3) -> Option<f64> {
    let (u, w) = plane_axes(normal);
    let proj = |v: Vec3| (v.dot(u), v.dot(w));
    let (ax, ay) = proj(dir);
    let e = v1 - v0;
    let (ex, ey) = proj(e);
    let (rx, ry) = proj(v0 - p0);
    // Solve [ax, -ex; ay, -ey] · [t, s] = [rx, ry].
    let det = ax * (-ey) - (-ex) * ay;
    if det.abs() < tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let t = (rx * (-ey) - (-ex) * ry) / det;
    let s = (ax * ry - ay * rx) / det;
    if (-tol::POINT_MERGE..=1.0 + tol::POINT_MERGE).contains(&s) {
        Some(t)
    } else {
        None
    }
}

// ───────────────────────────────────────────────────────────────── arrangement

/// Partitions `fp` into sub-faces by imprinting its boundary and `seams` onto a
/// fresh sketch on its plane and reading back the traced regions.
fn arrange_face(fp: &FacePoly, seams: &[(Point3, Point3)]) -> Result<Vec<Region>, BooleanError> {
    let mut sketch = Sketch::on_plane(fp.plane);
    add_loop(&mut sketch, &fp.outer)?;
    for h in &fp.holes {
        add_loop(&mut sketch, h)?;
    }
    for &(p, q) in seams {
        // A zero-length seam carries no information; skip rather than error.
        if !p.approx_eq(q, tol::POINT_MERGE) {
            sketch
                .add_segment(p, q)
                .map_err(|_| BooleanError::DegenerateContact)?;
        }
    }
    Ok(sketch
        .regions()
        .values()
        .map(|r| Region {
            outer: r
                .outer
                .iter()
                .map(|&v| sketch.vertices()[v].position)
                .collect(),
            holes: r
                .holes
                .iter()
                .map(|h| h.iter().map(|&v| sketch.vertices()[v].position).collect())
                .collect(),
        })
        .collect())
}

/// Adds a closed loop of positions to `sketch` as edges.
fn add_loop(sketch: &mut Sketch, loop_pts: &[Point3]) -> Result<(), BooleanError> {
    let n = loop_pts.len();
    for i in 0..n {
        let (p, q) = (loop_pts[i], loop_pts[(i + 1) % n]);
        if p.approx_eq(q, tol::POINT_MERGE) {
            continue;
        }
        sketch
            .add_segment(p, q)
            .map_err(|_| BooleanError::DegenerateContact)?;
    }
    Ok(())
}

/// A point strictly inside the region, robust to holes (annular regions, where
/// every outer-boundary ear centroid would fall in the hole).
///
/// From each outer-edge midpoint we march inward (toward the region interior)
/// until the nearest boundary — outer or hole — and take the half-distance
/// point, which is guaranteed strictly interior because the ray is inside the
/// region from the midpoint until that first crossing.
fn interior_point(region: &Region, normal: Vec3) -> Point3 {
    let mut boundary: Vec<(Point3, Point3)> = Vec::new();
    let mut push_loop = |pts: &[Point3]| {
        for i in 0..pts.len() {
            boundary.push((pts[i], pts[(i + 1) % pts.len()]));
        }
    };
    push_loop(&region.outer);
    for h in &region.holes {
        push_loop(h);
    }

    let o = &region.outer;
    for i in 0..o.len() {
        let (a, b) = (o[i], o[(i + 1) % o.len()]);
        let Ok(edge) = (b - a).normalized() else {
            continue;
        };
        // Interior is to the left of a CCW outer edge: normal × edge.
        let Ok(inward) = normal.cross(edge).normalized() else {
            continue;
        };
        let mid = Point3::ORIGIN + (a.to_vec() + b.to_vec()) * 0.5;
        let mut nearest = f64::INFINITY;
        for &(c, d) in &boundary {
            if let Some(t) = line_segment_param(mid, inward, c, d, normal)
                && t > tol::POINT_MERGE
                && t < nearest
            {
                nearest = t;
            }
        }
        if nearest.is_finite() {
            return mid + inward * (0.5 * nearest);
        }
    }
    centroid(o)
}

fn centroid(pts: &[Point3]) -> Point3 {
    let sum = pts.iter().fold(Vec3::ZERO, |acc, p| acc + p.to_vec());
    Point3::ORIGIN + sum / (pts.len() as f64)
}

// ───────────────────────────────────────────────────────────────────── queries

/// True if `p` is strictly inside the face region: inside the outer boundary and
/// outside every hole.
fn region_contains(fp: &FacePoly, p: Point3) -> bool {
    let n = fp.plane.normal();
    point_inside_polygon(p, &fp.outer, n) && !fp.holes.iter().any(|h| point_inside_polygon(p, h, n))
}

/// A few non-axis-aligned ray directions; the first one whose cast grazes no
/// face boundary decides the parity.
const RAY_DIRS: [Vec3; 4] = [
    Vec3::new(0.4651, 0.7345, 0.4949),
    Vec3::new(0.8012, -0.3461, 0.4877),
    Vec3::new(-0.3299, 0.5813, 0.7438),
    Vec3::new(0.1234, -0.5678, 0.8137),
];

/// Point-in-solid by ray-cast parity against a watertight face set. Tries each
/// candidate direction until one casts cleanly (no boundary graze).
fn point_in_solid(polys: &[FacePoly], p: Point3) -> bool {
    for d in RAY_DIRS {
        let (count, ambiguous) = ray_cast(polys, p, d);
        if !ambiguous {
            return count % 2 == 1;
        }
    }
    ray_cast(polys, p, RAY_DIRS[0]).0 % 2 == 1
}

/// Counts forward crossings of the ray `p + t·d` (t > 0) through the face set,
/// flagging the cast ambiguous if a hit grazes a face boundary or the ray lies
/// in a face's plane through `p`.
fn ray_cast(polys: &[FacePoly], p: Point3, d: Vec3) -> (usize, bool) {
    let mut count = 0;
    let mut ambiguous = false;
    for fp in polys {
        let denom = d.dot(fp.plane.normal());
        let signed = fp.plane.signed_distance(p);
        if denom.abs() < tol::NORMALIZE_MIN_LENGTH {
            if signed.abs() <= tol::PLANE_DIST {
                ambiguous = true;
            }
            continue;
        }
        let t = -signed / denom;
        if t <= tol::POINT_MERGE {
            continue;
        }
        let hit = p + d * t;
        if near_polygon_boundary(hit, fp) {
            ambiguous = true;
            continue;
        }
        if region_contains(fp, hit) {
            count += 1;
        }
    }
    (count, ambiguous)
}

/// True if `p` lies within [`tol::POINT_MERGE`] of any boundary segment of `fp`.
fn near_polygon_boundary(p: Point3, fp: &FacePoly) -> bool {
    let near = |poly: &[Point3]| {
        (0..poly.len()).any(|i| {
            point_segment_distance(p, poly[i], poly[(i + 1) % poly.len()]) <= tol::POINT_MERGE
        })
    };
    near(&fp.outer) || fp.holes.iter().any(|h| near(h))
}

/// Distance from `p` to the segment `a`–`b`.
fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < tol::NORMALIZE_MIN_LENGTH * tol::NORMALIZE_MIN_LENGTH {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

// ─────────────────────────────────────────────────────────────────── assembly

/// Welds the kept faces into a watertight Object, then splits a disconnected
/// result into one shell per connected component.
fn assemble(faces: Vec<OrientedFace>) -> Result<Object, BooleanError> {
    let mut positions: Vec<Point3> = Vec::new();
    let has_holes = faces.iter().any(|f| !f.holes.is_empty());

    let mut obj = if !has_holes {
        let polys: Vec<Vec<usize>> = faces
            .iter()
            .map(|f| f.outer.iter().map(|&p| intern(p, &mut positions)).collect())
            .collect();
        Object::from_polygons(&positions, &polys).map_err(|_| BooleanError::DegenerateContact)?
    } else {
        let with_holes: Vec<(Vec<usize>, Vec<Vec<usize>>, Plane)> = faces
            .iter()
            .map(|f| {
                let outer = f.outer.iter().map(|&p| intern(p, &mut positions)).collect();
                let holes = f
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&p| intern(p, &mut positions)).collect())
                    .collect();
                (outer, holes, f.plane)
            })
            .collect();
        Object::from_faces_with_holes(&positions, &with_holes)
    };

    if obj.watertight() != WatertightState::Watertight {
        return Err(BooleanError::DegenerateContact);
    }
    resplit_shells(&mut obj);
    obj.check_invariants();
    Ok(obj)
}

/// Replaces the object's shells with one per connected component of the
/// face-adjacency graph (faces linked by a shared, twinned edge). The builders
/// emit a single shell; a boolean result can be disconnected (a subtract that
/// cuts a solid in two, or a disjoint union).
fn resplit_shells(obj: &mut Object) {
    let mut adjacency: SecondaryMap<FaceId, Vec<FaceId>> = SecondaryMap::new();
    for f in obj.faces.keys() {
        adjacency.insert(f, Vec::new());
    }
    for edge in obj.edges.values() {
        if let Some(twin) = edge.twin_half_edge {
            let fa = obj.loops[obj.half_edges[edge.half_edge].loop_id].face;
            let fb = obj.loops[obj.half_edges[twin].loop_id].face;
            adjacency[fa].push(fb);
            adjacency[fb].push(fa);
        }
    }

    let mut component: SecondaryMap<FaceId, ()> = SecondaryMap::new();
    let mut shells: Vec<Vec<FaceId>> = Vec::new();
    for start in obj.faces.keys() {
        if component.contains_key(start) {
            continue;
        }
        let mut group = Vec::new();
        let mut stack = vec![start];
        component.insert(start, ());
        while let Some(f) = stack.pop() {
            group.push(f);
            for &nb in &adjacency[f] {
                if component.insert(nb, ()).is_none() {
                    stack.push(nb);
                }
            }
        }
        shells.push(group);
    }

    obj.shells.clear();
    for faces in shells {
        obj.shells.insert(Shell { faces });
    }
}

/// Interns `p` into `positions`, merging within [`tol::POINT_MERGE`]. Welding is
/// what pairs seam half-edges into twins, so the result is watertight.
fn intern(p: Point3, positions: &mut Vec<Point3>) -> usize {
    if let Some(i) = positions
        .iter()
        .position(|&q| p.approx_eq(q, tol::POINT_MERGE))
    {
        return i;
    }
    positions.push(p);
    positions.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(min: Point3, max: Point3) -> Object {
        let (a, b) = (min, max);
        Object::from_polygons(
            &[
                Point3::new(a.x, a.y, a.z),
                Point3::new(b.x, a.y, a.z),
                Point3::new(b.x, b.y, a.z),
                Point3::new(a.x, b.y, a.z),
                Point3::new(a.x, a.y, b.z),
                Point3::new(b.x, a.y, b.z),
                Point3::new(b.x, b.y, b.z),
                Point3::new(a.x, b.y, b.z),
            ],
            &[
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ],
        )
        .unwrap()
    }

    fn unit_cube() -> Object {
        cube(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0))
    }

    /// Divergence-theorem signed volume, fan-triangulating every loop of every
    /// face. Inner (hole) loops are wound opposite to the outer, so summing
    /// their fans subtracts the hole correctly — `to_polygons` drops holes, so
    /// it cannot be used here (coplanar results can be genus > 0).
    fn volume(obj: &Object) -> f64 {
        let mut v6 = 0.0;
        for f in obj.faces().values() {
            for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
                let p: Vec<Vec3> = obj.loop_positions(lid).map(|pt| pt.to_vec()).collect();
                for i in 1..p.len().saturating_sub(1) {
                    v6 += p[0].dot(p[i].cross(p[i + 1]));
                }
            }
        }
        v6 / 6.0
    }

    const VOL_TOL: f64 = 1e-9;

    fn overlapping(shift: Vec3) -> (Object, Object, Transform) {
        (unit_cube(), unit_cube(), Transform::translation(shift))
    }

    #[test]
    fn union_of_overlapping_cubes_has_correct_volume() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        // 2·1 − overlap(0.5³).
        assert!(
            (volume(&r) - (2.0 - 0.125)).abs() < VOL_TOL,
            "vol {}",
            volume(&r)
        );
    }

    #[test]
    fn intersect_of_overlapping_cubes_is_the_overlap_box() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Intersect, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 0.125).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    #[test]
    fn subtract_of_overlapping_cubes_removes_the_overlap() {
        let (a, b, t) = overlapping(Vec3::new(0.5, 0.5, 0.5));
        let r = Object::boolean(BooleanOp::Subtract, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&r) - (1.0 - 0.125)).abs() < VOL_TOL,
            "vol {}",
            volume(&r)
        );
    }

    #[test]
    fn disjoint_union_keeps_both_volumes() {
        let (a, b, t) = overlapping(Vec3::new(5.0, 0.0, 0.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    #[test]
    fn nested_intersect_is_the_inner_solid() {
        let a = unit_cube();
        let inner = cube(Point3::new(0.25, 0.25, 0.25), Point3::new(0.75, 0.75, 0.75));
        let r = Object::boolean(BooleanOp::Intersect, &a, &inner, &Transform::IDENTITY).unwrap();
        r.validate().unwrap();
        assert!((volume(&r) - 0.125).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    // ─────────────────────────────────────────────── coplanar contact

    /// Two boxes meeting at a single shared face (opposite-facing coincident):
    /// the interface vanishes and they fuse into one taller solid.
    #[test]
    fn stacked_union_is_one_tall_box() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &stacked).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    /// Stacked boxes share no volume, so their intersection is empty.
    #[test]
    fn stacked_intersect_is_empty() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(
            Object::boolean(BooleanOp::Intersect, &a, &b, &stacked).unwrap_err(),
            BooleanError::EmptyResult
        );
    }

    /// Subtracting a box that only touches `a`'s top face removes nothing.
    #[test]
    fn stacked_subtract_leaves_a_unchanged() {
        let a = unit_cube();
        let b = unit_cube();
        let stacked = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
        let r = Object::boolean(BooleanOp::Subtract, &a, &b, &stacked).unwrap();
        r.validate().unwrap();
        assert!((volume(&r) - 1.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    /// The user's case: two boxes sitting on the ground (shared z=0 *and* z=1
    /// planes, same-facing) overlapping in x/y. Walls cross transversally; caps
    /// are coplanar. Set algebra must hold for all three ops.
    #[test]
    fn ground_boxes_obey_set_algebra() {
        // a = unit cube; b shifted (0.5, 0.5, 0): overlap = 0.5·0.5·1 = 0.25.
        let a = unit_cube();
        let b = unit_cube();
        let t = Transform::translation(Vec3::new(0.5, 0.5, 0.0));
        let vol = |op| {
            let r = Object::boolean(op, &a, &b, &t).unwrap();
            r.validate().unwrap();
            assert_eq!(r.watertight(), WatertightState::Watertight);
            volume(&r)
        };
        assert!((vol(BooleanOp::Union) - 1.75).abs() < VOL_TOL);
        assert!((vol(BooleanOp::Subtract) - 0.75).abs() < VOL_TOL);
        assert!((vol(BooleanOp::Intersect) - 0.25).abs() < VOL_TOL);
    }

    /// Two boxes sharing an exact wall (opposite-facing coincident) fuse on union.
    #[test]
    fn shared_wall_union_fuses() {
        let a = unit_cube();
        let b = unit_cube();
        let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
        let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight);
        assert!((volume(&r) - 2.0).abs() < VOL_TOL, "vol {}", volume(&r));
    }

    /// A pillar standing on the same ground as a slab and poking through it:
    /// shared z=0 plane (same-facing, *different* footprints) with transversal
    /// walls. Subtract drills a through-hole, so the result is genus-1 with an
    /// annular coplanar bottom face — exercises holed coplanar output.
    #[test]
    fn slab_with_pillar_through_it() {
        let slab = cube(Point3::ORIGIN, Point3::new(2.0, 2.0, 1.0)); // vol 4
        let pillar = cube(Point3::new(0.5, 0.5, 0.0), Point3::new(1.5, 1.5, 3.0)); // vol 3
        // Overlap (pillar ∩ slab) = 1×1×1 = 1.
        let u = Object::boolean(BooleanOp::Union, &slab, &pillar, &Transform::IDENTITY).unwrap();
        u.validate().unwrap();
        assert_eq!(u.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&u) - 6.0).abs() < VOL_TOL,
            "union vol {}",
            volume(&u)
        );

        let s = Object::boolean(BooleanOp::Subtract, &slab, &pillar, &Transform::IDENTITY).unwrap();
        s.validate().unwrap();
        assert_eq!(s.watertight(), WatertightState::Watertight);
        assert!(
            (volume(&s) - 3.0).abs() < VOL_TOL,
            "subtract vol {}",
            volume(&s)
        );
    }

    /// Identical operands (all faces coincident, same-facing): union and
    /// intersect are the operand itself, subtract is empty.
    #[test]
    fn coincident_operands() {
        let a = unit_cube();
        let b = unit_cube();
        let id = Transform::IDENTITY;
        let u = Object::boolean(BooleanOp::Union, &a, &b, &id).unwrap();
        u.validate().unwrap();
        assert!(
            (volume(&u) - 1.0).abs() < VOL_TOL,
            "union vol {}",
            volume(&u)
        );
        let i = Object::boolean(BooleanOp::Intersect, &a, &b, &id).unwrap();
        i.validate().unwrap();
        assert!(
            (volume(&i) - 1.0).abs() < VOL_TOL,
            "intersect vol {}",
            volume(&i)
        );
        assert_eq!(
            Object::boolean(BooleanOp::Subtract, &a, &b, &id).unwrap_err(),
            BooleanError::EmptyResult
        );
    }
}
