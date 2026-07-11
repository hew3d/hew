//! Crate-internal 2-D geometry helpers (point-in-polygon, plane axes).
//!
//! These are used by both `sketch` (profile validation) and `ops` (hole
//! reassignment during `split_face`).  Nothing here is public API.

use crate::math::{Point3, Vec3};
use crate::tol;

/// Returns two orthonormal vectors (u, v) that span the plane with the given
/// normal.  The result is stable for any non-zero normal.
pub(crate) fn plane_axes(normal: Vec3) -> (Vec3, Vec3) {
    // Choose a reference vector not parallel to normal.
    let reference = if normal.x.abs() <= normal.y.abs() && normal.x.abs() <= normal.z.abs() {
        Vec3::new(1.0, 0.0, 0.0)
    } else if normal.y.abs() <= normal.z.abs() {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let u = normal
        .cross(reference)
        .normalized()
        .expect("plane_axes: normal is non-zero");
    let v = normal
        .cross(u)
        .normalized()
        .expect("plane_axes: u is non-zero");
    (u, v)
}

/// Signed area of a polygon against the given normal, using the 2-D cross
/// product projected onto the plane.  Positive = CCW seen from the normal
/// side.
pub(crate) fn signed_area_on_plane(pts: &[Point3], normal: Vec3) -> f64 {
    let (u, v) = plane_axes(normal);
    let n = pts.len();
    let mut area = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let ax = a.to_vec().dot(u);
        let ay = a.to_vec().dot(v);
        let bx = b.to_vec().dot(u);
        let by = b.to_vec().dot(v);
        area += ax * by - bx * ay;
    }
    area * 0.5
}

/// Point-in-polygon test using the ray-casting method projected onto the
/// plane spanned by `normal`. Returns `true` only for strictly interior
/// points; boundary points return `false` (strict interior required for
/// `HoleOutsideOuter`).
pub(crate) fn point_inside_polygon(pt: Point3, poly: &[Point3], normal: Vec3) -> bool {
    let (u, v) = plane_axes(normal);
    let px = pt.to_vec().dot(u);
    let py = pt.to_vec().dot(v);

    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let ai_x = poly[i].to_vec().dot(u);
        let ai_y = poly[i].to_vec().dot(v);
        let aj_x = poly[j].to_vec().dot(u);
        let aj_y = poly[j].to_vec().dot(v);

        if ((ai_y > py) != (aj_y > py)) && (px < (aj_x - ai_x) * (py - ai_y) / (aj_y - ai_y) + ai_x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 2-D segment intersection test (using the plane projection implicitly via
/// the signed-area / cross-product test in 3-D).  Returns true if the open
/// segments (p,q) and (r,s) properly cross, or if any endpoint touches the
/// other segment (closed-interval test).
pub(crate) fn segments_intersect(p: Point3, q: Point3, r: Point3, s: Point3) -> bool {
    let d1 = cross_z(r, s, p);
    let d2 = cross_z(r, s, q);
    let d3 = cross_z(p, q, r);
    let d4 = cross_z(p, q, s);

    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    // Collinear / endpoint cases.
    let eps = tol::POINT_MERGE;
    if d1.abs() <= eps && on_segment(r, s, p) {
        return true;
    }
    if d2.abs() <= eps && on_segment(r, s, q) {
        return true;
    }
    if d3.abs() <= eps && on_segment(p, q, r) {
        return true;
    }
    if d4.abs() <= eps && on_segment(p, q, s) {
        return true;
    }
    false
}

/// Signed 2-D cross product of (q-p) × (r-p) using the 3-D cross product
/// and extracting the dominant axis (which is the normal axis for coplanar
/// points).
pub(crate) fn cross_z(p: Point3, q: Point3, r: Point3) -> f64 {
    let pq = q - p;
    let pr = r - p;
    let c = pq.cross(pr);
    // Return the component along the dominant axis of the cross product.
    if c.z.abs() >= c.x.abs() && c.z.abs() >= c.y.abs() {
        c.z
    } else if c.y.abs() >= c.x.abs() {
        c.y
    } else {
        c.x
    }
}

/// True if point `t` lies on the line segment `[p, q]` (collinearity assumed).
pub(crate) fn on_segment(p: Point3, q: Point3, t: Point3) -> bool {
    let min_x = p.x.min(q.x);
    let max_x = p.x.max(q.x);
    let min_y = p.y.min(q.y);
    let max_y = p.y.max(q.y);
    let min_z = p.z.min(q.z);
    let max_z = p.z.max(q.z);
    t.x >= min_x - tol::POINT_MERGE
        && t.x <= max_x + tol::POINT_MERGE
        && t.y >= min_y - tol::POINT_MERGE
        && t.y <= max_y + tol::POINT_MERGE
        && t.z >= min_z - tol::POINT_MERGE
        && t.z <= max_z + tol::POINT_MERGE
}

/// True if `p` lies within `tol` of the closed segment `[a, b]`.
pub(crate) fn point_near_segment(p: Point3, a: Point3, b: Point3, tol: f64) -> bool {
    let ab = b - a;
    let len_sq = ab.length_squared();
    let t = if len_sq <= tol * tol {
        0.0
    } else {
        ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length_squared() <= tol * tol
}

/// True if the open segments (p,q) and (r,s), coplanar on the plane with
/// unit `normal`, cross transversally: each segment's endpoints lie strictly
/// on opposite sides of the other's line, "strictly" meaning farther than
/// [`tol::POINT_MERGE`] from it. Touching endpoints, T-contacts, and
/// collinear overlaps are NOT crossings — polygons that merely share
/// boundary do not cross.
///
/// Distances are measured in the plane's own orthonormal (u, v) basis, NOT
/// via [`cross_z`] — cross_z's dominant-axis extraction scales the result by
/// the normal's largest component (exact only for axis-aligned planes), and
/// here the values are compared against a real-world tolerance, so they must
/// be true in-plane distances on any plane.
pub(crate) fn segments_cross_properly(
    p: Point3,
    q: Point3,
    r: Point3,
    s: Point3,
    normal: Vec3,
) -> bool {
    let (u, v) = plane_axes(normal);
    let proj = |t: Point3| (t.to_vec().dot(u), t.to_vec().dot(v));
    // Signed distance of `t` from the line through (a, b): the 2-D cross
    // product of (b - a) with (t - a), divided by |b - a|.
    let dist = |a: Point3, b: Point3, t: Point3| -> Option<f64> {
        let (ax, ay) = proj(a);
        let (bx, by) = proj(b);
        let (tx, ty) = proj(t);
        let (ex, ey) = (bx - ax, by - ay);
        let len = ex.hypot(ey);
        if len < tol::NORMALIZE_MIN_LENGTH {
            return None;
        }
        Some((ex * (ty - ay) - ey * (tx - ax)) / len)
    };
    let (Some(d1), Some(d2), Some(d3), Some(d4)) =
        (dist(r, s, p), dist(r, s, q), dist(p, q, r), dist(p, q, s))
    else {
        return false;
    };
    let eps = tol::POINT_MERGE;
    ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
}

/// A point strictly inside the material of a polygon-with-holes (inside
/// `outer`, outside every hole), or `None` for degenerate loops.
///
/// Scanline method: pick the plane-axis-horizontal line whose ordinate is the
/// midpoint of the widest gap between distinct vertex ordinates (maximally
/// far from every vertex, so no edge is grazed), intersect it with every loop
/// edge, and take the midpoint of the widest inside interval under the
/// even-odd rule. Deterministic, and works for concave outers and any hole
/// arrangement — unlike a centroid, which can land outside or in a hole.
pub(crate) fn interior_point_of_loops(
    outer: &[Point3],
    holes: &[Vec<Point3>],
    normal: Vec3,
) -> Option<Point3> {
    if outer.len() < 3 {
        return None;
    }
    let (u, v) = plane_axes(normal);
    let px = |p: Point3| p.to_vec().dot(u);
    let py = |p: Point3| p.to_vec().dot(v);
    // {u, v, normal} is orthonormal, so a point on the plane reconstructs
    // exactly from its three projections; the normal component is shared by
    // every loop point.
    let w = outer[0].to_vec().dot(normal);

    let mut ys: Vec<f64> = outer.iter().map(|&p| py(p)).collect();
    let outer_min = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let outer_max = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for h in holes {
        ys.extend(h.iter().map(|&p| py(p)));
    }
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() <= tol::POINT_MERGE);

    // Widest vertex-free ordinate band, clamped to the outer loop's range so
    // the scanline is guaranteed to cross it.
    let mut band: Option<(f64, f64)> = None; // (gap, midpoint)
    for pair in ys.windows(2) {
        let (lo, hi) = (pair[0].max(outer_min), pair[1].min(outer_max));
        let gap = hi - lo;
        if gap > tol::POINT_MERGE && band.is_none_or(|(g, _)| gap > g) {
            band = Some((gap, (lo + hi) * 0.5));
        }
    }
    let (_, y_star) = band?;

    // Every crossing of the scanline with every loop edge, in plane-x.
    let mut xs: Vec<f64> = Vec::new();
    let mut collect = |lp: &[Point3]| {
        let n = lp.len();
        for i in 0..n {
            let (a, b) = (lp[i], lp[(i + 1) % n]);
            let (ax, ay) = (px(a), py(a));
            let (bx, by) = (px(b), py(b));
            if (ay > y_star) != (by > y_star) {
                xs.push(ax + (bx - ax) * (y_star - ay) / (by - ay));
            }
        }
    };
    collect(outer);
    for h in holes {
        collect(h);
    }
    if xs.len() < 2 {
        return None;
    }
    xs.sort_by(f64::total_cmp);

    // Even-odd: intervals (xs[0], xs[1]), (xs[2], xs[3]), … are material.
    let mut best: Option<(f64, f64)> = None; // (width, midpoint)
    for pair in xs.chunks_exact(2) {
        let width = pair[1] - pair[0];
        if width > tol::POINT_MERGE && best.is_none_or(|(g, _)| width > g) {
            best = Some((width, (pair[0] + pair[1]) * 0.5));
        }
    }
    let (_, x_star) = best?;
    Some(Point3::ORIGIN + u * x_star + v * y_star + normal * w)
}

/// Whether the material of polygon-with-holes A overlaps the material of
/// polygon-with-holes B. Shared boundary alone is NOT overlap: interiors
/// must intersect.
///
/// Three-way test: an interior sample of either lies in the other's material,
/// or their boundaries properly cross ([`segments_cross_properly`]). The two
/// samples alone decide every containment case (A ⊆ B, B ⊆ A, disjoint,
/// adjacent); a partial overlap whose samples both land in the non-shared
/// parts is caught by the crossing sweep — a transversal crossing of any two
/// loops puts material of both on a common quadrant of the crossing.
pub(crate) fn loops_overlap(
    a_outer: &[Point3],
    a_holes: &[Vec<Point3>],
    b_outer: &[Point3],
    b_holes: &[Vec<Point3>],
    normal: Vec3,
) -> bool {
    if !bboxes_touch(a_outer, b_outer) {
        return false;
    }
    let inside_material = |p: Point3, outer: &[Point3], holes: &[Vec<Point3>]| {
        point_inside_polygon(p, outer, normal)
            && !holes.iter().any(|h| point_inside_polygon(p, h, normal))
    };
    if let Some(p) = interior_point_of_loops(a_outer, a_holes, normal)
        && inside_material(p, b_outer, b_holes)
    {
        return true;
    }
    if let Some(p) = interior_point_of_loops(b_outer, b_holes, normal)
        && inside_material(p, a_outer, a_holes)
    {
        return true;
    }
    fn all_loops<'a>(
        outer: &'a [Point3],
        holes: &'a [Vec<Point3>],
    ) -> impl Iterator<Item = &'a [Point3]> {
        std::iter::once(outer).chain(holes.iter().map(Vec::as_slice))
    }
    for la in all_loops(a_outer, a_holes) {
        for lb in all_loops(b_outer, b_holes) {
            for i in 0..la.len() {
                let (p, q) = (la[i], la[(i + 1) % la.len()]);
                for j in 0..lb.len() {
                    let (r, s) = (lb[j], lb[(j + 1) % lb.len()]);
                    if segments_cross_properly(p, q, r, s, normal) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Axis-aligned bounding boxes of two point sets touch or overlap (with
/// [`tol::POINT_MERGE`] slack) — the cheap rejection in front of
/// [`loops_overlap`]'s exact tests.
fn bboxes_touch(a: &[Point3], b: &[Point3]) -> bool {
    let bbox = |pts: &[Point3]| {
        let mut lo = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut hi = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in pts {
            lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        (lo, hi)
    };
    let (alo, ahi) = bbox(a);
    let (blo, bhi) = bbox(b);
    let e = tol::POINT_MERGE;
    alo.x <= bhi.x + e
        && blo.x <= ahi.x + e
        && alo.y <= bhi.y + e
        && blo.y <= ahi.y + e
        && alo.z <= bhi.z + e
        && blo.z <= ahi.z + e
}

/// Check whether two separate polygons share any point, cross, or touch.
pub(crate) fn boundaries_contact(a: &[Point3], b: &[Point3]) -> bool {
    let na = a.len();
    let nb = b.len();
    for i in 0..na {
        let p = a[i];
        let q = a[(i + 1) % na];
        for j in 0..nb {
            let r = b[j];
            let s = b[(j + 1) % nb];
            if segments_intersect(p, q, r, s) {
                return true;
            }
        }
    }
    false
}

/// Check that a polygon does not cross or touch itself.  Tests every pair of
/// non-adjacent edges for intersection.
pub(crate) fn polygon_is_simple(pts: &[Point3]) -> bool {
    let n = pts.len();
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        // Skip edges that share a vertex with edge i.
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue; // edges 0 and n-1 share vertex 0
            }
            let c = pts[j];
            let d = pts[(j + 1) % n];
            if segments_intersect(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `segments_cross_properly`'s tolerance gate measures TRUE in-plane
    /// distance on any plane — a configuration with the same in-plane
    /// clearance must classify identically on the ground plane and on a
    /// tilted one. (The dominant-axis shortcut in [`cross_z`] would scale
    /// the tilted case by the normal's largest component and misclassify
    /// near-tolerance crossings.)
    #[test]
    fn crossing_tolerance_is_plane_tilt_invariant() {
        let build = |normal: Vec3, clearance: f64| {
            let (u, v) = plane_axes(normal);
            let at = |x: f64, y: f64| Point3::ORIGIN + u * x + v * y;
            // Segment r-s along the u axis; p and q straddle it at ±clearance.
            let (r, s) = (at(-1.0, 0.0), at(1.0, 0.0));
            let (p, q) = (at(0.0, clearance), at(0.0, -clearance));
            segments_cross_properly(p, q, r, s, normal)
        };
        let ground = Vec3::new(0.0, 0.0, 1.0);
        let tilted = Vec3::new(1.0, 1.0, 1.0).normalized().unwrap();
        // Just past the tolerance: crosses on every plane.
        let past = tol::POINT_MERGE * 1.2;
        assert!(build(ground, past));
        assert!(build(tilted, past), "tilted plane must not widen the gate");
        // Just inside the tolerance: a touch, not a crossing, on every plane.
        let inside = tol::POINT_MERGE * 0.8;
        assert!(!build(ground, inside));
        assert!(!build(tilted, inside));
    }

    /// Adjacent polygons sharing only a boundary edge do not overlap; a
    /// genuine partial overlap (crossing boundaries) does.
    #[test]
    fn loops_overlap_distinguishes_adjacency_from_overlap() {
        let n = Vec3::new(0.0, 0.0, 1.0);
        let sq = |x0: f64, y0: f64, x1: f64, y1: f64| {
            vec![
                Point3::new(x0, y0, 0.0),
                Point3::new(x1, y0, 0.0),
                Point3::new(x1, y1, 0.0),
                Point3::new(x0, y1, 0.0),
            ]
        };
        let a = sq(0.0, 0.0, 2.0, 2.0);
        let adjacent = sq(2.0, 0.0, 4.0, 2.0);
        let overlapping = sq(1.0, 1.0, 3.0, 3.0);
        let inside = sq(0.5, 0.5, 1.5, 1.5);
        assert!(!loops_overlap(&a, &[], &adjacent, &[], n));
        assert!(loops_overlap(&a, &[], &overlapping, &[], n));
        assert!(loops_overlap(&a, &[], &inside, &[], n));
        assert!(loops_overlap(&inside, &[], &a, &[], n));
        // A disk filling A's hole touches only the hole boundary: no overlap.
        let holed_holes = vec![inside.clone()];
        assert!(!loops_overlap(&a, &holed_holes, &inside, &[], n));
    }
}
