//! Ear-clip triangulation of one face, holes included, for the boolean's
//! imported-operand pre-pass (`boolean::refine_operand`) and the tidy pass.
//! Separate from `crate::triangulate` (the renderer's bridged ear clip,
//! shared with the curve operations): this one guarantees what the boolean
//! relies on, that every ring vertex stays a triangle corner and no
//! diagonal passes through a vertex.
//!
//! An imported face can be flat only to the import tolerance. Where a cut
//! reaches such a face, the boolean replaces it with flat triangles through
//! the same vertices, bit for bit: subdivision, never a moved point
//! (DEVELOPMENT.md rule 4). Every vertex of the face is a corner of some
//! triangle and every triangle side is either a face edge or a diagonal
//! that passes through no vertex, so the triangles meet the face's
//! neighbors exactly where the face did.
//!
//! The method follows `tessellate`'s bridged ear clip (Eberly, "Triangulation
//! by Ear Clipping"): each hole is spliced into the outer ring by a
//! zero-width bridge to the nearest ring vertex it can see, then ears are
//! cut off one by one. Imported rings carry collinear vertices where
//! T-junctions were stitched, so an ear's tip must clear the chord between
//! its neighbors by more than [`tol::POINT_MERGE`]: a collinear vertex is
//! never a tip, only a corner. A face no ear can be found for is refused
//! (`None`), never forced.

use crate::geom2d::plane_axes;
use crate::math::{Point3, Vec3};
use crate::tol;

/// Triangles covering the face bounded by `outer` (counter-clockwise seen
/// from `normal`) less `holes` (clockwise), as index triples into the
/// rings laid end to end: `outer` first, then each hole in order. Every
/// triangle is counter-clockwise seen from `normal`. `None` when the rings
/// are wound the wrong way or no ear can be found.
pub(crate) fn triangulate(
    outer: &[Point3],
    holes: &[Vec<Point3>],
    normal: Vec3,
) -> Option<Vec<[usize; 3]>> {
    if outer.len() < 3 {
        return None;
    }
    let (u, v) = plane_axes(normal);
    // Relative to the first vertex, so a face far from the origin keeps
    // its precision in the projection.
    let origin = outer[0];
    let pts: Vec<[f64; 2]> = outer
        .iter()
        .chain(holes.iter().flatten())
        .map(|&p| {
            let d = p - origin;
            [d.dot(u), d.dot(v)]
        })
        .collect();

    let outer_ring: Vec<usize> = (0..outer.len()).collect();
    if ring_area(&pts, &outer_ring) <= 0.0 {
        return None;
    }
    let mut start = outer.len();
    let mut hole_rings: Vec<Vec<usize>> = Vec::with_capacity(holes.len());
    for h in holes {
        let ring: Vec<usize> = (start..start + h.len()).collect();
        start += h.len();
        if h.len() < 3 || ring_area(&pts, &ring) >= 0.0 {
            return None;
        }
        hole_rings.push(ring);
    }
    // Rightmost hole first, as in Eberly's construction (it keeps later
    // bridges short); ties keep the given order.
    let max_u = |r: &Vec<usize>| {
        r.iter()
            .map(|&i| pts[i][0])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    hole_rings.sort_by(|a, b| max_u(b).total_cmp(&max_u(a)));

    let mut ring = outer_ring;
    for k in 0..hole_rings.len() {
        let (hole, remaining) = hole_rings[k..].split_first().expect("k is in range");
        ring = bridge(&pts, ring, hole, remaining)?;
    }
    clip(&pts, ring)
}

/// Twice the signed area of a ring of `pts` indices (positive =
/// counter-clockwise), as a fan from its first vertex.
fn ring_area(pts: &[[f64; 2]], ring: &[usize]) -> f64 {
    let p0 = pts[ring[0]];
    (1..ring.len().saturating_sub(1))
        .map(|k| cross(p0, pts[ring[k]], pts[ring[k + 1]]))
        .sum()
}

/// `(b - a) × (c - a)`: twice the signed area of the triangle.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Twice the smallest area a triangle may have (square meters): a kernel
/// face needs a plane, and [`crate::math::Plane::from_polygon`] finds one
/// on area alone when twice the area reaches [`tol::NORMALIZE_MIN_LENGTH`].
/// Four times that leaves room for the plane's rounding in three
/// dimensions. Deliberately not the narrower width rule the plane fit also
/// accepts: this floor decides which ears the boolean's pre-pass cuts from
/// an imported face that is not flat, and the triangulations it gives are
/// pinned by the planarity specs.
const MIN_TWICE_AREA: f64 = 4.0 * tol::NORMALIZE_MIN_LENGTH;

/// Whether `p` projects strictly inside the segment `a`–`c`.
fn strictly_between(p: [f64; 2], a: [f64; 2], c: [f64; 2]) -> bool {
    let t = (p[0] - a[0]) * (c[0] - a[0]) + (p[1] - a[1]) * (c[1] - a[1]);
    t > 0.0 && t < (c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2)
}

/// How far `b` sits off the chord `a → c`, signed: positive on the left
/// (a convex corner of a counter-clockwise ring).
fn clearance(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let chord = (c[0] - a[0]).hypot(c[1] - a[1]);
    if chord == 0.0 {
        return 0.0;
    }
    cross(a, b, c) / chord
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// True if the segments `a`–`b` and `c`–`d` cross at a point interior to
/// both (each straddles the other's line strictly).
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Whether the segment leaving `apex` toward `q` starts into the region,
/// which lies to the left of the boundary path `prev → apex → next`
/// (O'Rourke's `InCone`).
fn in_cone(prev: [f64; 2], apex: [f64; 2], next: [f64; 2], q: [f64; 2]) -> bool {
    if cross(apex, next, prev) >= 0.0 {
        cross(apex, q, prev) > 0.0 && cross(q, apex, next) > 0.0
    } else {
        !(cross(apex, q, next) >= 0.0 && cross(q, apex, prev) >= 0.0)
    }
}

/// True if a bridge from ring position `k` to hole position `m` starts into
/// the region at both ends and meets no vertex or edge of the ring, the
/// hole, or a hole still to be bridged anywhere but at its own two ends.
fn bridge_is_valid(
    pts: &[[f64; 2]],
    ring: &[usize],
    k: usize,
    hole: &[usize],
    m: usize,
    remaining: &[Vec<usize>],
) -> bool {
    let p = pts[ring[k]];
    let q = pts[hole[m]];
    let (rp, rn) = corner(pts, ring, k);
    let (hp, hn) = corner(pts, hole, m);
    if !in_cone(rp, p, rn, q) || !in_cone(hp, q, hn, p) {
        return false;
    }
    let blocks = |r: &[usize]| {
        (0..r.len()).any(|i| {
            let (a, b) = (pts[r[i]], pts[r[(i + 1) % r.len()]]);
            (a != p && a != q && segment_distance(a, p, q) <= tol::POINT_MERGE)
                || segments_cross(p, q, a, b)
        })
    };
    !blocks(ring) && !blocks(hole) && !remaining.iter().any(|r| blocks(r))
}

/// The points before and after position `k` of `ring` that differ from
/// the point at `k`: the two sides of the corner there. A bridge at a
/// shared point leaves copies of it next to each other (a zero-length
/// edge), which say nothing about the corner. `k`'s own point when the
/// whole ring is that point.
fn corner(pts: &[[f64; 2]], ring: &[usize], k: usize) -> ([f64; 2], [f64; 2]) {
    let n = ring.len();
    let at = pts[ring[k]];
    let side = |step: usize| {
        (1..n)
            .map(|j| pts[ring[(k + j * step) % n]])
            .find(|&p| p != at)
            .unwrap_or(at)
    };
    (side(n - 1), side(1))
}

/// `ring` with `hole` spliced in by a zero-width bridge: to the ring vertex
/// at the hole's own position if the hole touches the ring there, else
/// from the hole's rightmost vertex to the nearest ring vertex it can
/// validly reach. `None` when there is none.
///
/// The ring may pass a point it shares with a hole more than once (an
/// earlier hole touching the ring, or another hole, there), and each pass
/// is a different corner. The splice goes to the first pass, in ring
/// order, whose corner holds both of the hole's sides at that point
/// (Eberly's rule for a vertex the ring visits twice): any other pass
/// would cross the ring's edges. Only when no pass holds them (a hole
/// edge running along a ring edge) does the first pass take it, as
/// before, and the ear clip refuses what does not triangulate.
fn bridge(
    pts: &[[f64; 2]],
    ring: Vec<usize>,
    hole: &[usize],
    remaining: &[Vec<usize>],
) -> Option<Vec<usize>> {
    let splice = |k: usize, m: usize| {
        let mut out = Vec::with_capacity(ring.len() + hole.len() + 2);
        out.extend_from_slice(&ring[..=k]);
        out.extend((0..=hole.len()).map(|j| hole[(m + j) % hole.len()]));
        out.extend_from_slice(&ring[k..]);
        out
    };
    let mut touch: Option<(usize, usize)> = None;
    for (m, &hv) in hole.iter().enumerate() {
        let at = pts[hv];
        let (hp, hn) = corner(pts, hole, m);
        for k in (0..ring.len()).filter(|&k| pts[ring[k]] == at) {
            let (rp, rn) = corner(pts, &ring, k);
            if in_cone(rp, at, rn, hp) && in_cone(rp, at, rn, hn) {
                return Some(splice(k, m));
            }
            touch.get_or_insert((k, m));
        }
    }
    if let Some((k, m)) = touch {
        return Some(splice(k, m));
    }
    let m = (0..hole.len())
        .max_by(|&i, &j| pts[hole[i]][0].total_cmp(&pts[hole[j]][0]).then(j.cmp(&i)))
        .expect("a hole has vertices");
    let q = pts[hole[m]];
    let dist2 = |k: usize| {
        let p = pts[ring[k]];
        (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
    };
    let mut order: Vec<usize> = (0..ring.len()).collect();
    order.sort_by(|&i, &j| dist2(i).total_cmp(&dist2(j)));
    let k = order
        .into_iter()
        .find(|&k| bridge_is_valid(pts, &ring, k, hole, m, remaining))?;
    Some(splice(k, m))
}

/// Ear-clips a weakly simple counter-clockwise ring.
fn clip(pts: &[[f64; 2]], mut ring: Vec<usize>) -> Option<Vec<[usize; 3]>> {
    let mut tris: Vec<[usize; 3]> = Vec::with_capacity(ring.len());
    // Where the search for the next ear starts: just before the last one
    // cut, where the ring changed, so a convex run is clipped in one sweep.
    let mut start = 0usize;
    loop {
        // A zero-length edge (a hole bridged at a point it shares with the
        // ring) carries nothing: one copy of the point stays.
        let mut i = 0;
        while ring.len() > 1 && i < ring.len() {
            let next = (i + 1) % ring.len();
            if pts[ring[i]] == pts[ring[next]] {
                ring.remove(i);
            } else {
                i += 1;
            }
        }
        let n = ring.len();
        if n < 3 {
            break;
        }
        let ear = (0..n).map(|k| (start + k) % n).find(|&i| {
            let (ia, ib, ic) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if clearance(a, b, c) <= tol::POINT_MERGE || cross(a, b, c) <= MIN_TWICE_AREA {
                return false;
            }
            // No other vertex in the closed triangle: strictly inside, or
            // on (within the merge tolerance of) one of its sides, where
            // the triangle's side would pass through it. Nor one so close to
            // the new diagonal that the triangle it must later make with
            // the diagonal's ends would be too small to have a plane.
            !ring.iter().any(|&j| {
                let p = pts[j];
                if p == a || p == b || p == c {
                    return false;
                }
                let inside = cross(a, b, p) > 0.0 && cross(b, c, p) > 0.0 && cross(c, a, p) > 0.0;
                inside
                    || segment_distance(p, a, c) <= tol::POINT_MERGE
                    || segment_distance(p, a, b) <= tol::POINT_MERGE
                    || segment_distance(p, b, c) <= tol::POINT_MERGE
                    || (cross(a, c, p).abs() <= MIN_TWICE_AREA && strictly_between(p, a, c))
            })
        });
        match ear {
            Some(i) => {
                tris.push([ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]]);
                ring.remove(i);
                start = (i + n - 2) % (n - 1);
            }
            None => {
                // What is left may be the zero-width corridor of a bridge,
                // every point of which is already a corner of a triangle.
                let spent = ring_area(pts, &ring).abs()
                    <= tol::POINT_MERGE * ring_perimeter(pts, &ring)
                    && ring
                        .iter()
                        .all(|&j| tris.iter().flatten().any(|&t| pts[t] == pts[j]));
                return spent.then_some(tris);
            }
        }
    }
    // The last one or two points must already be corners.
    ring.iter()
        .all(|&j| tris.iter().flatten().any(|&t| pts[t] == pts[j]))
        .then_some(tris)
}

fn ring_perimeter(pts: &[[f64; 2]], ring: &[usize]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (pts[ring[i]], pts[ring[(i + 1) % ring.len()]]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    fn pts(xy: &[(f64, f64)]) -> Vec<Point3> {
        xy.iter().map(|&(x, y)| Point3::new(x, y, 0.0)).collect()
    }

    /// Twice the area of every triangle, all positive, summing to twice the
    /// face's area; every vertex is a corner.
    fn check(outer: &[Point3], holes: &[Vec<Point3>], expect_area: f64) -> Vec<[usize; 3]> {
        let tris = triangulate(outer, holes, UP).expect("triangulates");
        let all: Vec<Point3> = outer
            .iter()
            .chain(holes.iter().flatten())
            .copied()
            .collect();
        let mut area = 0.0;
        for t in &tris {
            let (a, b, c) = (all[t[0]], all[t[1]], all[t[2]]);
            let a2 = (b - a).cross(c - a).z;
            assert!(a2 > 0.0, "triangle {t:?} is not counter-clockwise");
            area += a2 / 2.0;
        }
        assert!((area - expect_area).abs() < 1e-12, "area {area}");
        for (i, p) in all.iter().enumerate() {
            assert!(
                tris.iter().flatten().any(|&t| all[t] == *p),
                "vertex {i} is no corner"
            );
        }
        tris
    }

    #[test]
    fn a_convex_quad_is_two_triangles() {
        let q = pts(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        assert_eq!(check(&q, &[], 1.0).len(), 2);
    }

    #[test]
    fn a_concave_ring_triangulates() {
        // An L.
        let l = pts(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ]);
        assert_eq!(check(&l, &[], 3.0).len(), 4);
    }

    /// Collinear boundary vertices (T-junction stitching) are corners,
    /// never tips, and no triangle side passes through one.
    #[test]
    fn collinear_boundary_vertices_stay_corners() {
        let r = pts(&[
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (1.5, 1.0),
            (0.0, 1.0),
            (0.0, 0.5),
        ]);
        let tris = check(&r, &[], 3.0);
        let all = &r;
        // No triangle side carries another vertex in its interior.
        for t in &tris {
            for s in 0..3 {
                let (a, b) = (all[t[s]], all[t[(s + 1) % 3]]);
                for p in all {
                    if *p == a || *p == b {
                        continue;
                    }
                    let d = segment_distance([p.x, p.y], [a.x, a.y], [b.x, b.y]);
                    assert!(d > 1e-9, "side {a:?}-{b:?} passes through {p:?}");
                }
            }
        }
    }

    #[test]
    fn a_face_with_holes_triangulates() {
        let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        // Two square holes, wound clockwise.
        let h1 = pts(&[(0.5, 0.5), (0.5, 1.5), (1.5, 1.5), (1.5, 0.5)]);
        let h2 = pts(&[(2.5, 2.5), (2.5, 3.5), (3.5, 3.5), (3.5, 2.5)]);
        check(&outer, &[h1, h2], 14.0);
    }

    /// A hole touching the outer ring at one vertex splices there.
    #[test]
    fn a_hole_touching_the_ring_at_a_point_triangulates() {
        let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let hole = pts(&[(0.0, 2.0), (1.0, 3.0), (1.0, 1.0)]);
        check(&outer, &[hole], 15.0);
    }

    /// Two holes touching the outer ring at the same corner: the second
    /// splices into the pass of that corner its sides lie in, not the
    /// first pass in ring order (which would cross the first hole).
    #[test]
    fn holes_sharing_a_ring_corner_triangulate() {
        let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let h1 = pts(&[(0.0, 0.0), (0.9, 0.15), (0.7, 0.05)]);
        let h2 = pts(&[(0.0, 0.0), (1.1, 0.8), (1.4, 0.7)]);
        let area = 16.0 - tri_area(&h1) - tri_area(&h2);
        check_cover(&outer, &[h1.clone(), h2.clone()], area);
        check_cover(&outer, &[h2, h1], area);
        let a = pts(&[(0.0, 0.0), (2.0, 0.8), (0.5, 0.1)]);
        let b = pts(&[(0.0, 0.0), (0.1, 0.5), (0.8, 2.0)]);
        let area = 16.0 - tri_area(&a) - tri_area(&b);
        check_cover(&outer, &[a.clone(), b.clone()], area);
        check_cover(&outer, &[b, a], area);
    }

    /// Holes touching one another at a point inside the face, clear of
    /// the outer ring: the first bridges, the rest splice at the shared
    /// point's right pass.
    #[test]
    fn holes_sharing_an_inner_point_triangulate() {
        let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let c = (2.0, 2.0);
        let h1 = pts(&[c, (3.0, 2.5), (3.0, 1.5)]);
        let h2 = pts(&[c, (1.0, 1.5), (1.0, 2.5)]);
        let h3 = pts(&[c, (1.5, 3.0), (2.5, 3.0)]);
        let area = 16.0 - tri_area(&h1) - tri_area(&h2) - tri_area(&h3);
        check_cover(&outer, &[h1, h2, h3], area);
    }

    /// The area of a clockwise hole.
    fn tri_area(h: &[Point3]) -> f64 {
        -ring_twice_area(h) / 2.0
    }

    /// Twice the signed area of a ring in the z = 0 plane.
    fn ring_twice_area(r: &[Point3]) -> f64 {
        (0..r.len())
            .map(|i| {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                a.x * b.y - b.x * a.y
            })
            .sum()
    }

    /// Strictly inside the ring (crossing number; a point on it is out).
    fn inside(p: [f64; 2], r: &[Point3]) -> bool {
        let mut odd = false;
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if (a.y > p[1]) != (b.y > p[1]) {
                let x = a.x + (p[1] - a.y) / (b.y - a.y) * (b.x - a.x);
                if x > p[0] {
                    odd = !odd;
                }
            }
        }
        odd
    }

    /// [`check`], plus: no two triangles overlap (a separating side for
    /// every pair) and every triangle's centroid is in the face, off every
    /// hole.
    fn check_cover(outer: &[Point3], holes: &[Vec<Point3>], expect_area: f64) {
        let tris = triangulate(outer, holes, UP).expect("triangulates");
        let all: Vec<[f64; 2]> = outer
            .iter()
            .chain(holes.iter().flatten())
            .map(|p| [p.x, p.y])
            .collect();
        let mut area = 0.0;
        for t in &tris {
            let a2 = cross(all[t[0]], all[t[1]], all[t[2]]);
            assert!(a2 > 0.0, "triangle {t:?} is not counter-clockwise");
            area += a2 / 2.0;
            let c = [
                (all[t[0]][0] + all[t[1]][0] + all[t[2]][0]) / 3.0,
                (all[t[0]][1] + all[t[1]][1] + all[t[2]][1]) / 3.0,
            ];
            assert!(inside(c, outer), "triangle {t:?} leaves the face");
            assert!(
                holes.iter().all(|h| !inside(c, h)),
                "triangle {t:?} covers a hole"
            );
        }
        assert!(
            (area - expect_area).abs() <= 1e-9 * expect_area.max(1.0),
            "area {area}, expected {expect_area}"
        );
        // Convex triangles are interior-disjoint when some side of one has
        // the other wholly on its outer side.
        let separated = |s: &[usize; 3], o: &[usize; 3]| {
            (0..3).any(|e| {
                let (a, b) = (all[s[e]], all[s[(e + 1) % 3]]);
                o.iter().all(|&j| cross(a, b, all[j]) <= 1e-12)
            })
        };
        for (i, s) in tris.iter().enumerate() {
            for o in &tris[i + 1..] {
                assert!(
                    separated(s, o) || separated(o, s),
                    "triangles {s:?} and {o:?} overlap"
                );
            }
        }
        for (i, p) in all.iter().enumerate() {
            assert!(
                tris.iter().flatten().any(|&t| all[t] == *p),
                "vertex {i} is no corner"
            );
        }
    }

    /// A star-shaped hole about `c` (clockwise): from `c` out to the ray at
    /// angle `a1`, an optional middle corner, back from the ray at `a0`.
    fn sector_hole(c: (f64, f64), a0: f64, a1: f64, r: [f64; 3], mid: bool) -> Vec<Point3> {
        let at = |a: f64, r: f64| (c.0 + r * a.cos(), c.1 + r * a.sin());
        let mut h = vec![c, at(a1, r[0])];
        if mid {
            h.push(at((a0 + a1) / 2.0, r[1]));
        }
        h.push(at(a0, r[2]));
        pts(&h)
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2000))]

        /// Two or three holes meeting at one point (an outer corner, or a
        /// point inside the face), each in its own angular sector about
        /// it, triangulates every time: every triangle in the face and off
        /// the holes, none overlapping, their area the face's.
        #[test]
        fn holes_sharing_a_vertex_always_triangulate(
            at_corner in proptest::bool::ANY,
            count in 2usize..=3,
            cuts in proptest::collection::vec(0.05f64..0.95, 6),
            radii in proptest::collection::vec(0.3f64..1.0, 9),
            mids in proptest::collection::vec(proptest::bool::ANY, 3),
            order in 0usize..6,
        ) {
            use std::f64::consts::PI;
            let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
            let (c, span, reach) = if at_corner {
                ((0.0, 0.0), PI / 2.0, 3.5)
            } else {
                ((2.0, 2.0), 2.0 * PI, 1.8)
            };
            // `count` sectors, each under a half turn, separated by gaps.
            let slot = span / count as f64;
            let mut holes = Vec::new();
            for k in 0..count {
                let (u, w) = (cuts[2 * k].min(cuts[2 * k + 1]), cuts[2 * k].max(cuts[2 * k + 1]));
                let w = w.max(u + 0.03);
                let a0 = slot * (k as f64 + u);
                let a1 = (slot * (k as f64 + w)).min(a0 + 0.9 * PI);
                let r = [radii[3 * k] * reach, radii[3 * k + 1] * reach, radii[3 * k + 2] * reach];
                holes.push(sector_hole(c, a0, a1, r, mids[k]));
            }
            let n = holes.len();
            holes.rotate_left(order % n);
            if order >= 3 {
                holes.reverse();
            }
            let area = 16.0 + holes.iter().map(|h| ring_twice_area(h) / 2.0).sum::<f64>();
            for h in &holes {
                proptest::prop_assume!(ring_twice_area(h) < -1e-6);
            }
            check_cover(&outer, &holes, area);
        }
    }

    /// A quad three of whose corners are all but collinear (twice the
    /// area of the triangle they make is under the plane threshold): the
    /// diagonal must run from the fourth corner, never between the first
    /// and third of the three. From a sculpted STL.
    #[test]
    fn a_quad_with_three_nearly_collinear_corners_takes_the_other_diagonal() {
        let q = vec![
            Point3::new(
                0.013102142333984376,
                0.015635940551757814,
                0.017238874435424805,
            ),
            Point3::new(
                0.013149749755859376,
                0.015584716796875,
                0.017237722396850588,
            ),
            Point3::new(
                0.013092071533203126,
                0.01558807373046875,
                0.01724687957763672,
            ),
            Point3::new(
                0.01303436279296875,
                0.015591445922851563,
                0.017256034851074218,
            ),
        ];
        let normal = crate::math::Plane::from_polygon(&q).unwrap().normal();
        let tris = triangulate(&q, &[], normal).expect("triangulates");
        for t in &tris {
            let tri = [q[t[0]], q[t[1]], q[t[2]]];
            let plane = crate::math::Plane::from_polygon(&tri).expect("each triangle has a plane");
            assert!(plane.normal().dot(normal) > 0.0);
        }
        assert!(tris.iter().all(|t| t.contains(&0)), "{tris:?}");
    }

    #[test]
    fn rings_it_must_refuse() {
        // Wound clockwise.
        let cw = pts(&[(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)]);
        assert_eq!(triangulate(&cw, &[], UP), None);
        // Every vertex on one line: no ear clears its chord.
        let flat = pts(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]);
        assert_eq!(triangulate(&flat, &[], UP), None);
        // A self-crossing bow tie.
        let bow = pts(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]);
        assert_eq!(triangulate(&bow, &[], UP), None);
        // A hole wound like the outer ring.
        let outer = pts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        let ccw_hole = pts(&[(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)]);
        assert_eq!(triangulate(&outer, &[ccw_hole], UP), None);
    }

    /// Off the z = 0 plane and far from the origin, a slightly bent quad
    /// keeps every point bit for bit.
    #[test]
    fn a_bent_quad_far_from_the_origin_triangulates() {
        let o = 1.0e4;
        let q = vec![
            Point3::new(o, o, o),
            Point3::new(o + 1.0, o, o),
            Point3::new(o + 1.0, o + 1.0, o + 1e-6),
            Point3::new(o, o + 1.0, o),
        ];
        let tris = triangulate(&q, &[], UP).expect("triangulates");
        assert_eq!(tris.len(), 2);
    }
}
