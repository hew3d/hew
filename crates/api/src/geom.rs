//! Small pure-geometry helpers the kernel does not export: planar
//! polygon area/centroid, point-in-polygon, ray–plane intersection,
//! point–segment distance, and bounding boxes. All f64 meters, world
//! frame. Tolerances: the API accepts client-authored coordinates, which
//! carry JSON round-trip noise the kernel's native 1e-9 gates would
//! reject unfairly, so locator matching uses [`API_SURFACE_TOL`] —
//! deliberately looser than `kernel::tol::PLANE_DIST`, deliberately far
//! tighter than any real feature size.

use kernel::trig;
use kernel::{Plane, Point3, Vec3};

/// How close a client-supplied point must be to a surface (or edge) to
/// count as "on" it for locator resolution (docs/agents/HEW_API.md §5.2).
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

/// Two points closer than this ordinate gap (or interval width) are
/// treated as coincident by [`interior_point_of_loops`] — points fed to it
/// come straight from kernel storage (never JSON round-tripped), so a
/// tolerance far tighter than [`API_SURFACE_TOL`] is appropriate.
const INTERIOR_POINT_TOL: f64 = 1e-9;

/// A point strictly inside a planar polygon-with-holes: inside `outer`,
/// outside every ring in `holes`. The API-side counterpart of the
/// kernel's crate-internal `geom2d::interior_point_of_loops` (`pub(crate)`,
/// not reachable from here) — duplicated rather than exposed, per this
/// module's own charter of small geometry helpers the kernel does not
/// export.
///
/// Scanline method: pick the plane-axis-horizontal line whose ordinate is
/// the midpoint of the widest gap between distinct vertex ordinates
/// (maximally far from every vertex, so no edge is grazed; a narrower gap
/// when the widest crosses no material), intersect it
/// with every ring, and take the midpoint of the widest inside interval
/// under the even-odd rule. Deterministic, and correct for a concave
/// outer ring or an off-center hole — unlike an area centroid
/// ([`face_centroid`]), which can land outside the ring or inside a hole.
/// `None` for a degenerate outer ring (fewer than 3 points) or one with no
/// interior at the chosen scanline.
pub fn interior_point_of_loops(
    outer: &[Point3],
    holes: &[Vec<Point3>],
    normal: Vec3,
) -> Option<Point3> {
    if outer.len() < 3 {
        return None;
    }
    // Project onto the two kept axes of `drop_axis`, the same 2-D
    // reduction `ring_contains` already uses; the dropped axis's
    // coordinate is reconstructed below from the plane equation, since
    // (unlike an orthonormal plane basis) it is not directly one of the
    // two kept values.
    let (ua, va) = drop_axis(normal);
    let da = (0..3).find(|a| *a != ua && *a != va)?;
    let n = [normal.x, normal.y, normal.z];
    let p0 = outer[0];
    let d = n[0] * p0.x + n[1] * p0.y + n[2] * p0.z;

    let mut ys: Vec<f64> = outer.iter().map(|&p| pick(p, va)).collect();
    let outer_min = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let outer_max = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for h in holes {
        ys.extend(h.iter().map(|&p| pick(p, va)));
    }
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() <= INTERIOR_POINT_TOL);

    // Vertex-free ordinate bands, clamped to the outer loop's range so the
    // scanline is guaranteed to cross it, tried widest first until one
    // finds material: the widest band can lie wholly inside a hole that
    // spans the outer ring's full width (the kernel's
    // `geom2d::interior_point_of_loops` does the same).
    let mut bands: Vec<(f64, f64)> = Vec::new(); // (gap, midpoint)
    for pair in ys.windows(2) {
        let (lo, hi) = (pair[0].max(outer_min), pair[1].min(outer_max));
        let gap = hi - lo;
        if gap > INTERIOR_POINT_TOL {
            bands.push((gap, (lo + hi) * 0.5));
        }
    }
    // Widest first; equal gaps keep their ordinate order (stable sort), so
    // the first band tried is the one a single-band search would pick.
    bands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (x_star, y_star) = bands.into_iter().find_map(|(_, y_star)| {
        // Every crossing of the scanline with every ring edge, in plane-x.
        let mut xs: Vec<f64> = Vec::new();
        let mut collect = |lp: &[Point3]| {
            let n = lp.len();
            for i in 0..n {
                let (a, b) = (lp[i], lp[(i + 1) % n]);
                let (ax, ay) = (pick(a, ua), pick(a, va));
                let (bx, by) = (pick(b, ua), pick(b, va));
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
        for pair in xs.as_chunks::<2>().0 {
            let width = pair[1] - pair[0];
            if width > INTERIOR_POINT_TOL && best.is_none_or(|(g, _)| width > g) {
                best = Some((width, (pair[0] + pair[1]) * 0.5));
            }
        }
        best.map(|(_, x_star)| (x_star, y_star))
    })?;

    // Reconstruct the dropped axis's coordinate from the plane equation
    // n·p = d — every ring point shares the same offset since they're
    // coplanar. `drop_axis` always drops the LARGEST-magnitude component
    // of `normal`, so the division below never sees a near-zero divisor.
    let mut coords = [0.0_f64; 3];
    coords[ua] = x_star;
    coords[va] = y_star;
    coords[da] = (d - n[ua] * x_star - n[va] * y_star) / n[da];
    Some(Point3::new(coords[0], coords[1], coords[2]))
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
    fn interior_point_avoids_a_hole_centered_on_the_outer_rings_own_centroid() {
        // A hole exactly centered on the square's own area centroid — a
        // plain centroid (`face_centroid`) would land inside it; the
        // scanline method must not.
        let outer = square();
        let hole = vec![
            Point3::new(0.9, 0.9, 0.0),
            Point3::new(1.1, 0.9, 0.0),
            Point3::new(1.1, 1.1, 0.0),
            Point3::new(0.9, 1.1, 0.0),
        ];
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let p = interior_point_of_loops(&outer, std::slice::from_ref(&hole), normal)
            .expect("a square with a small centered hole still has interior material");
        assert!(ring_contains(&outer, normal, p), "inside the outer ring");
        assert!(!ring_contains(&hole, normal, p), "clear of the hole");
    }

    #[test]
    fn interior_point_of_a_concave_l_shape_lands_inside_the_material() {
        // An L-shape (a 2x2 square with its top-right 1x1 corner notched
        // out) — concave enough that a naive centroid can miss the
        // polygon entirely.
        let l_shape = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ];
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let p = interior_point_of_loops(&l_shape, &[], normal)
            .expect("a non-degenerate L-shape has interior material");
        assert!(ring_contains(&l_shape, normal, p));
    }

    #[test]
    fn interior_point_of_a_degenerate_loop_is_none() {
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let degenerate = vec![Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)];
        assert!(interior_point_of_loops(&degenerate, &[], normal).is_none());
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

/// The drawing tools' sagitta budget (`app/src/tools/arcMath.ts`
/// `DRAW_SAGITTA_TOL_M`): a drawn curve's facets stay within this of the
/// true circle. A client-side density rule, not a kernel tolerance.
pub const DRAW_SAGITTA_TOL_M: f64 = 5e-4;
/// Floor of the drawing tools' segments-per-turn rule.
pub const DRAW_MIN_SEGMENTS_PER_TURN: u32 = 24;
/// Cap of the drawing tools' segments-per-turn rule.
pub const DRAW_MAX_SEGMENTS_PER_TURN: u32 = 96;

/// Segments per full turn for a drawn curve of `radius` meters — an exact
/// port of `arcMath.ts` `segmentsPerTurn`:
/// `clamp(24, 96, ceil4(π / acos(1 − s/r)))` with `s` the sagitta budget,
/// rounded up to a multiple of 4; non-finite or non-positive radii (and radii
/// at or under the budget) take the floor. The parity table in this crate's
/// tests and in `arcMath.test.ts` pins the two together.
pub fn draw_segments_per_turn(radius: f64) -> u32 {
    if !radius.is_finite() || radius <= 0.0 {
        return DRAW_MIN_SEGMENTS_PER_TURN;
    }
    let ratio = DRAW_SAGITTA_TOL_M / radius;
    if ratio >= 1.0 {
        return DRAW_MIN_SEGMENTS_PER_TURN;
    }
    let exact = std::f64::consts::PI / trig::acos(1.0 - ratio);
    let ceil4 = (exact / 4.0).ceil() * 4.0;
    (ceil4 as u32).clamp(DRAW_MIN_SEGMENTS_PER_TURN, DRAW_MAX_SEGMENTS_PER_TURN)
}

/// Facets across an arc of sweep `phi` radians at `radius`:
/// `max(2, ceil(draw_segments_per_turn(radius) · |phi| / 2π))` — the port of
/// `arcMath.ts` `arcSegmentCount`.
pub fn arc_segments(radius: f64, phi: f64) -> u32 {
    let turns = phi.abs() / std::f64::consts::TAU;
    ((turns * f64::from(draw_segments_per_turn(radius))).ceil() as u32).max(2)
}

#[cfg(test)]
mod draw_rule_tests {
    use super::*;

    /// Computed once with `arcMath.ts` `segmentsPerTurn`; `arcMath.test.ts`
    /// asserts the same table.
    const PARITY: [(f64, u32); 12] = [
        (1e-4, 24),
        (1e-3, 24),
        (5e-3, 24),
        (0.01, 24),
        (0.05, 24),
        (0.1, 32),
        (0.3, 56),
        (1.0, 96),
        (3.0, 96),
        (10.0, 96),
        (100.0, 96),
        (0.0, 24),
    ];

    #[test]
    fn segments_per_turn_matches_the_drawing_tools() {
        for (r, n) in PARITY {
            assert_eq!(draw_segments_per_turn(r), n, "radius {r}");
        }
    }

    #[test]
    fn arc_segments_matches_the_drawing_tools() {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};
        for (r, phi, n) in [
            (0.01, FRAC_PI_2, 6),
            (0.1, FRAC_PI_2, 8),
            (1.0, PI, 48),
            (0.003, 0.1, 2),
            (3.0, TAU, 96),
        ] {
            assert_eq!(arc_segments(r, phi), n, "radius {r}, sweep {phi}");
        }
    }
}
