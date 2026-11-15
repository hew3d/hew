//! Property tests for `kernel::triangulate` (DEVELOPMENT.md rule 3),
//! mirroring the `resolve` ≡ `resolve_linear` suite in crates/inference: on
//! random polygons the pruned ear clipping must return exactly the
//! reference triangulation, plus invariants of `triangulate_face`. Public
//! surface only; the tests that need the clipper's private counters are the
//! unit tests inside `src/triangulate.rs`.

use kernel::triangulate::{Pt5, bridge_holes, ear_clip, ear_clip_reference, triangulate_face};
use kernel::{Point3, Vec3};
use proptest::prelude::*;

/// Outer rectangle and CW rectangular hole loops of an "architectural
/// wall": each hole entry is `[x0, y0, x1, y1]`; holes wind CW in
/// (x, y), opposite the CCW outer, matching kernel inner loops.
fn wall_shape(width: f64, height: f64, holes: &[[f64; 4]]) -> (Vec<[f64; 3]>, Vec<Vec<[f64; 3]>>) {
    let outer = vec![
        [0.0, 0.0, 0.0],
        [width, 0.0, 0.0],
        [width, height, 0.0],
        [0.0, height, 0.0],
    ];
    let holes_3d: Vec<Vec<[f64; 3]>> = holes
        .iter()
        .map(|&[x0, y0, x1, y1]| vec![[x0, y0, 0.0], [x0, y1, 0.0], [x1, y1, 0.0], [x1, y0, 0.0]])
        .collect();
    (outer, holes_3d)
}

/// Builds the bridged "architectural wall" polygon from [`wall_shape`]
/// loops, holes bridged into a single weakly simple polygon — the
/// profiled pathological ear-clipping input.
fn wall_polygon(width: f64, height: f64, holes: &[[f64; 4]]) -> Vec<Pt5> {
    let (outer, holes_3d) = wall_shape(width, height, holes);
    // u × v = +Z (right-handed), so u2d = x and v2d = y.
    bridge_holes(
        &outer,
        &holes_3d,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .expect("wall holes must be bridgeable")
}

/// Random star-shaped polygon: vertices at strictly increasing polar
/// angles that wind around the origin exactly once, with every
/// angular gap below π (gaps are drawn from (0.75, 1.0) and
/// normalized to 2π, so the largest is at most 2π/2.25 < π). Every
/// ray from the origin then crosses exactly one edge, which makes
/// the polygon simple and CCW by construction — sorting arbitrary
/// angles is NOT enough (a polygon whose angles don't wrap the
/// origin can self-intersect on the closing edge).
fn arb_star_polygon() -> impl Strategy<Value = Vec<[f64; 5]>> {
    prop::collection::vec((0.75f64..1.0, 0.2f64..10.0), 3..48).prop_map(|pts| {
        let total: f64 = pts.iter().map(|&(gap, _)| gap).sum();
        let mut angle = 0.0f64;
        pts.iter()
            .map(|&(gap, radius)| {
                let x = radius * angle.cos();
                let y = radius * angle.sin();
                angle += std::f64::consts::TAU * gap / total;
                [x, y, 0.0, x, y]
            })
            .collect()
    })
}

/// Doubles a polygon by inserting each edge's midpoint — exactly
/// collinear runs, the configuration the conservative band in
/// `must_containment_test` exists for.
fn subdivide(poly: &[[f64; 5]]) -> Vec<[f64; 5]> {
    let n = poly.len();
    let mut out = Vec::with_capacity(2 * n);
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        out.push(a);
        out.push([
            (a[0] + b[0]) / 2.0,
            (a[1] + b[1]) / 2.0,
            (a[2] + b[2]) / 2.0,
            (a[3] + b[3]) / 2.0,
            (a[4] + b[4]) / 2.0,
        ]);
    }
    out
}

/// Random wall holes, one per unit cell so they are always disjoint
/// and strictly inside the outer rectangle.
fn arb_wall_holes() -> impl Strategy<Value = Vec<[f64; 4]>> {
    prop::collection::vec(
        (0.05f64..0.45, 0.05f64..0.45, 0.55f64..0.95, 0.55f64..0.95),
        1..8,
    )
    .prop_map(|cells| {
        cells
            .iter()
            .enumerate()
            .map(|(i, &(x0, y0, x1, y1))| [i as f64 + x0, y0, i as f64 + x1, y1])
            .collect()
    })
}

/// Lattice wall holes: rectangles snapped to a 0.3 grid, one per
/// 3×3 cell of a `3n × 3` wall. Quantization makes bridge
/// corridors, hole corners, and ear diagonals collide EXACTLY —
/// collinear blockers and coincident crossings that the
/// continuous-float generators above cannot produce (three random
/// f64 coordinates are essentially never collinear).
fn arb_lattice_wall_holes() -> impl Strategy<Value = Vec<[f64; 4]>> {
    prop::collection::vec((1u32..=8, 1u32..=8, 1u32..=8, 1u32..=8), 1..8).prop_map(|cells| {
        cells
            .iter()
            .enumerate()
            .map(|(i, &(a, b, c, d))| {
                let x0 = 0.3 * a.min(b) as f64;
                let x1 = 0.3 * (a.max(b) + 1) as f64;
                let y0 = 0.3 * c.min(d) as f64;
                let y1 = 0.3 * (c.max(d) + 1) as f64;
                [3.0 * i as f64 + x0, y0, 3.0 * i as f64 + x1, y1]
            })
            .collect()
    })
}

proptest! {
    /// Random simple polygons: pruned ≡ reference, exactly.
    #[test]
    fn pruned_ear_clip_equals_reference_on_simple_polygons(
        poly in arb_star_polygon(),
    ) {
        prop_assert_eq!(ear_clip(&poly), ear_clip_reference(&poly).0);
    }

    /// Random simple polygons with exactly-collinear runs: the
    /// on-the-band vertices must stay in the containment-test set,
    /// so the outputs still match exactly.
    #[test]
    fn pruned_ear_clip_equals_reference_with_collinear_runs(
        poly in arb_star_polygon(),
    ) {
        let poly = subdivide(&poly);
        prop_assert_eq!(ear_clip(&poly), ear_clip_reference(&poly).0);
    }

    /// Random hole-bridged polygons (duplicate bridge vertices):
    /// pruned ≡ reference, exactly.
    #[test]
    fn pruned_ear_clip_equals_reference_on_bridged_polygons(
        holes in arb_wall_holes(),
    ) {
        let poly = wall_polygon(holes.len() as f64, 1.0, &holes);
        prop_assert_eq!(ear_clip(&poly), ear_clip_reference(&poly).0);
    }

    /// Lattice-quantized bridged polygons (exact collinearity):
    /// pruned ≡ reference, exactly. This is the class where
    /// reflex-only pruning of the on-diagonal blocker diverged.
    #[test]
    fn pruned_ear_clip_equals_reference_on_lattice_walls(
        holes in arb_lattice_wall_holes(),
    ) {
        let poly = wall_polygon(3.0 * holes.len() as f64, 3.0, &holes);
        prop_assert_eq!(ear_clip(&poly), ear_clip_reference(&poly).0);
    }

    /// Random convex polygons with up to 2 interior rectangular holes: every
    /// output index is below the input point count, the triangle count is
    /// `n_outer + Σ n_hole + 2·holes − 2`, and the total area is the
    /// polygon's minus the holes' (relative 1e-12).
    #[test]
    fn triangulate_face_keeps_indices(case in arb_face_case()) {
        let (outer, holes) = (&case.outer, &case.holes);
        let tris = triangulate_face(outer, holes, Vec3::new(0.0, 0.0, 1.0))
            .expect("disjoint interior holes bridge");
        let all: Vec<Point3> = outer.iter().chain(holes.iter().flatten()).copied().collect();
        let n_total = all.len();
        for t in &tris {
            for &i in t {
                prop_assert!(i < n_total, "index {} out of range {}", i, n_total);
            }
        }
        prop_assert_eq!(tris.len(), n_total + 2 * holes.len() - 2);
        let area: f64 = tris.iter().map(|t| tri_area_z(&all, t)).sum();
        let expected = ring_area_z(outer) + holes.iter().map(|h| ring_area_z(h)).sum::<f64>();
        prop_assert!(
            (area - expected).abs() <= 1e-12 * expected.abs(),
            "area {} vs expected {}", area, expected
        );
    }

    /// Every output triangle winds CCW about the face normal.
    #[test]
    fn triangulate_face_winding(case in arb_face_case()) {
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let tris = triangulate_face(&case.outer, &case.holes, normal)
            .expect("disjoint interior holes bridge");
        let all: Vec<Point3> = case
            .outer
            .iter()
            .chain(case.holes.iter().flatten())
            .copied()
            .collect();
        for t in &tris {
            let (a, b, c) = (all[t[0]], all[t[1]], all[t[2]]);
            let n = (b - a).cross(c - a);
            prop_assert!(n.dot(normal) > 0.0, "triangle {:?} not CCW about the normal", t);
        }
    }
}

// ─────────────────────────────── triangulate_face fixtures

/// A planar face on z = 0: CCW convex outer ring and CW rectangular holes.
#[derive(Debug, Clone)]
struct FaceCase {
    outer: Vec<Point3>,
    holes: Vec<Vec<Point3>>,
}

/// Signed area (about +Z) of a ring on z = 0.
fn ring_area_z(ring: &[Point3]) -> f64 {
    let n = ring.len();
    let mut s = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        s += ring[i].x * ring[j].y - ring[j].x * ring[i].y;
    }
    0.5 * s
}

/// Signed area (about +Z) of the triangle `t` of `pts`.
fn tri_area_z(pts: &[Point3], t: &[usize; 3]) -> f64 {
    ring_area_z(&[pts[t[0]], pts[t[1]], pts[t[2]]])
}

/// Random convex face: a regular n-gon (n in 3..=12, circumradius in
/// 1..=5, random rotation) around the origin, so it contains the disc of
/// radius 0.5 about the origin; plus up to 2 rectangular holes placed in
/// disjoint cells (x < 0 and x > 0) inside the box |x| <= 0.3, |y| <= 0.1,
/// whose corners lie within 0.32 of the origin.
fn arb_face_case() -> impl Strategy<Value = FaceCase> {
    let hole_params = || {
        (
            -0.3f64..-0.15,
            -0.10f64..-0.02,
            -0.10f64..-0.02,
            0.02f64..0.10,
        )
    };
    (
        3usize..=12,
        1.0f64..5.0,
        0.0f64..std::f64::consts::TAU,
        0usize..=2,
        hole_params(),
        hole_params(),
    )
        .prop_map(|(n, radius, rot, n_holes, ha, hb)| {
            let outer: Vec<Point3> = (0..n)
                .map(|i| {
                    let a = rot + std::f64::consts::TAU * i as f64 / n as f64;
                    Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
                })
                .collect();
            // Hole A on the -x side; hole B mirrors B's x-range to +x.
            let rect = |x0: f64, x1: f64, y0: f64, y1: f64| -> Vec<Point3> {
                // CW in (x, y), opposite the CCW outer.
                vec![
                    Point3::new(x0, y0, 0.0),
                    Point3::new(x0, y1, 0.0),
                    Point3::new(x1, y1, 0.0),
                    Point3::new(x1, y0, 0.0),
                ]
            };
            let mut holes = Vec::new();
            if n_holes >= 1 {
                holes.push(rect(ha.0, ha.1, ha.2, ha.3));
            }
            if n_holes >= 2 {
                holes.push(rect(-hb.1, -hb.0, hb.2, hb.3));
            }
            FaceCase { outer, holes }
        })
}
