//! Small pure-geometry helpers the kernel does not export: planar
//! polygon area/centroid, point-in-polygon, ray–plane intersection,
//! point–segment distance, and bounding boxes. All f64 meters, world
//! frame. Tolerances: the API accepts client-authored coordinates, which
//! carry JSON round-trip noise the kernel's native 1e-9 gates would
//! reject unfairly, so locator matching uses [`API_SURFACE_TOL`] —
//! deliberately looser than `kernel::tol::PLANE_DIST`, deliberately far
//! tighter than any real feature size.

use kernel::{Plane, Point3, Vec3};

/// How close a client-supplied point must be to a surface (or edge) to
/// count as "on" it for locator resolution (docs/HEW_API.md §5.2).
pub const API_SURFACE_TOL: f64 = 1e-6;

/// Two locator candidates closer than this are indistinguishable — an
/// ambiguous locator, refused rather than guessed (§5.2).
pub const API_AMBIGUITY_TOL: f64 = 1e-9;

/// Newell-style area vector of a closed 3D polygon: half the sum of
/// cross products. Its length is the area; its direction the normal.
fn area_vector(ring: &[Point3]) -> Vec3 {
    let mut sum = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..ring.len() {
        let a = ring[i].to_vec();
        let b = ring[(i + 1) % ring.len()].to_vec();
        sum = sum + a.cross(b);
    }
    sum * 0.5
}

/// Area of a planar polygon ring (positive regardless of winding).
pub fn ring_area(ring: &[Point3]) -> f64 {
    area_vector(ring).length()
}

/// Area of a face with holes: outer minus holes.
pub fn face_area(outer: &[Point3], holes: &[Vec<Point3>]) -> f64 {
    (ring_area(outer) - holes.iter().map(|h| ring_area(h)).sum::<f64>()).max(0.0)
}

/// Area centroid of a planar polygon with holes (holes subtract).
pub fn face_centroid(outer: &[Point3], holes: &[Vec<Point3>]) -> Point3 {
    // Fan-triangulate from the ring's first vertex; signed triangle areas
    // (projected on the face normal) weight the triangle centroids. Holes
    // contribute negative weight. Valid for the kernel's simple rings.
    let normal = area_vector(outer);
    let n = if normal.length() > 0.0 {
        normal * (1.0 / normal.length())
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let mut weight = 0.0_f64;
    let mut acc = Vec3::new(0.0, 0.0, 0.0);
    let mut add_ring = |ring: &[Point3], sign: f64| {
        if ring.len() < 3 {
            return;
        }
        let p0 = ring[0].to_vec();
        for i in 1..ring.len() - 1 {
            let p1 = ring[i].to_vec();
            let p2 = ring[i + 1].to_vec();
            let a = (p1 - p0).cross(p2 - p0).dot(n) * 0.5 * sign;
            let c = (p0 + p1 + p2) * (1.0 / 3.0);
            weight += a;
            acc = acc + c * a;
        }
    };
    add_ring(outer, 1.0);
    for h in holes {
        add_ring(h, -1.0);
    }
    if weight.abs() < f64::EPSILON {
        // Degenerate: fall back to the ring's vertex mean.
        let mean = outer
            .iter()
            .fold(Vec3::new(0.0, 0.0, 0.0), |s, p| s + p.to_vec())
            * (1.0 / outer.len().max(1) as f64);
        return Point3::new(mean.x, mean.y, mean.z);
    }
    let c = acc * (1.0 / weight);
    Point3::new(c.x, c.y, c.z)
}

/// Even-odd point-in-polygon for a planar ring, testing the point's
/// projection into the ring's plane. The caller has already gated the
/// point's DISTANCE to the plane; this answers containment only.
pub fn ring_contains(ring: &[Point3], normal: Vec3, p: Point3) -> bool {
    if ring.len() < 3 {
        return false;
    }
    // Project onto the dominant-axis plane of the normal (drop the
    // largest component) — the standard robust 2D reduction.
    let (u, v) = drop_axis(normal);
    let px = pick(p, u);
    let py = pick(p, v);
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (xi, yi) = (pick(ring[i], u), pick(ring[i], v));
        let (xj, yj) = (pick(ring[j], u), pick(ring[j], v));
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A face (outer ring minus holes) contains the point's projection.
pub fn face_contains(outer: &[Point3], holes: &[Vec<Point3>], normal: Vec3, p: Point3) -> bool {
    ring_contains(outer, normal, p) && !holes.iter().any(|h| ring_contains(h, normal, p))
}

fn drop_axis(n: Vec3) -> (usize, usize) {
    let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
    if az >= ax && az >= ay {
        (0, 1)
    } else if ay >= ax {
        (0, 2)
    } else {
        (1, 2)
    }
}

fn pick(p: Point3, axis: usize) -> f64 {
    match axis {
        0 => p.x,
        1 => p.y,
        _ => p.z,
    }
}

/// Ray–plane intersection parameter `t >= 0`, or `None` when parallel or
/// behind the origin.
pub fn ray_plane_t(origin: Point3, dir: Vec3, plane: &Plane) -> Option<f64> {
    let denom = plane.normal().dot(dir);
    if denom.abs() < 1e-15 {
        return None;
    }
    let t = -plane.signed_distance(origin) / denom;
    (t >= 0.0).then_some(t)
}

/// Distance from a point to a segment, and the parameter of the nearest
/// point on it (0 at `a`, 1 at `b`).
pub fn point_segment_distance(p: Point3, a: Point3, b: Point3) -> (f64, f64) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= f64::EPSILON {
        return ((p - a).length(), 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    let nearest = a + ab * t;
    ((p - nearest).length(), t)
}

/// Axis-aligned bounding box over points.
pub fn bbox(points: impl Iterator<Item = Point3>) -> Option<(Point3, Point3)> {
    let mut min: Option<Point3> = None;
    let mut max: Option<Point3> = None;
    for p in points {
        min = Some(match min {
            None => p,
            Some(m) => Point3::new(m.x.min(p.x), m.y.min(p.y), m.z.min(p.z)),
        });
        max = Some(match max {
            None => p,
            Some(m) => Point3::new(m.x.max(p.x), m.y.max(p.y), m.z.max(p.z)),
        });
    }
    Some((min?, max?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Point3> {
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ]
    }

    #[test]
    fn area_and_centroid_of_a_square_with_a_hole() {
        let outer = square();
        let hole = vec![
            Point3::new(0.5, 0.5, 0.0),
            Point3::new(1.0, 0.5, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.5, 1.0, 0.0),
        ];
        assert!((ring_area(&outer) - 4.0).abs() < 1e-12);
        assert!((face_area(&outer, std::slice::from_ref(&hole)) - 3.75).abs() < 1e-12);
        let c = face_centroid(&outer, &[]);
        assert!(c.approx_eq(Point3::new(1.0, 1.0, 0.0), 1e-12));
        // Hole below-left of center pushes the centroid up-right.
        let c = face_centroid(&outer, &[hole]);
        assert!(c.x > 1.0 && c.y > 1.0);
    }

    #[test]
    fn containment_respects_holes() {
        let outer = square();
        let hole = vec![
            Point3::new(0.5, 0.5, 0.0),
            Point3::new(1.5, 0.5, 0.0),
            Point3::new(1.5, 1.5, 0.0),
            Point3::new(0.5, 1.5, 0.0),
        ];
        let n = Vec3::new(0.0, 0.0, 1.0);
        assert!(face_contains(&outer, &[], n, Point3::new(1.0, 1.0, 0.0)));
        assert!(!face_contains(
            &outer,
            &[hole],
            n,
            Point3::new(1.0, 1.0, 0.0)
        ));
        assert!(!face_contains(&outer, &[], n, Point3::new(3.0, 1.0, 0.0)));
    }

    #[test]
    fn ray_plane_and_segment_distance() {
        let plane = Plane::from_polygon(&square()).unwrap();
        let t = ray_plane_t(
            Point3::new(1.0, 1.0, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
            &plane,
        )
        .unwrap();
        assert!((t - 5.0).abs() < 1e-12);
        assert!(
            ray_plane_t(Point3::new(1.0, 1.0, 5.0), Vec3::new(0.0, 0.0, 1.0), &plane).is_none()
        );
        let (d, t) = point_segment_distance(
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        );
        assert!((d - 1.0).abs() < 1e-12 && (t - 0.5).abs() < 1e-12);
    }
}
