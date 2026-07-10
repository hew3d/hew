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
