//! Regression repro distilled from `op_fuzz.rs` (surfaced by the rule-9
//! replay proof): a lens cut — a multi-segment `split_face` path whose BOTH
//! endpoints land in the interior of the same boundary edge — depended on
//! the path's orientation. The first endpoint's `split_boundary_edge`
//! consumes the shared half-edge, so the second endpoint re-resolves its
//! containing sub-edge by projection; that projection was unclamped, and an
//! extrapolation onto the carrier line has zero distance from a point that
//! actually lies on the COLLINEAR SIBLING sub-edge. Whichever sub-edge came
//! first in iteration order won, and the wrong one wired a self-overlapping
//! (validator-invisible) ring: the boundary visited the two split vertices
//! in swapped order.
//!
//! Contract: the two path orientations describe the same cut, so both must
//! produce the same geometry — and that geometry must place the split
//! vertices in carrier order along the boundary.

use kernel::{Object, Point3};

fn cube(s: f64) -> Object {
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(s, 0.0, 0.0),
        Point3::new(s, s, 0.0),
        Point3::new(0.0, s, 0.0),
        Point3::new(0.0, 0.0, s),
        Point3::new(s, 0.0, s),
        Point3::new(s, s, s),
        Point3::new(0.0, s, s),
    ];
    let f = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons(&v, &f).unwrap()
}

fn rings(o: &Object) -> Vec<Vec<Point3>> {
    let (pts, fs) = o.to_polygons();
    fs.into_iter()
        .map(|f| f.into_iter().map(|i| pts[i]).collect())
        .collect()
}

/// Multiset equality of face rings up to rotation (winding preserved).
fn same_rings(a: &[Vec<Point3>], b: &[Vec<Point3>]) -> bool {
    fn cyclic(a: &[Point3], b: &[Point3]) -> bool {
        a.len() == b.len()
            && (0..a.len()).any(|shift| {
                a.iter()
                    .enumerate()
                    .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], 1e-9))
            })
    }
    if a.len() != b.len() {
        return false;
    }
    let mut unmatched: Vec<&Vec<Point3>> = b.iter().collect();
    for ring in a {
        let Some(i) = unmatched.iter().position(|c| cyclic(ring, c)) else {
            return false;
        };
        unmatched.swap_remove(i);
    }
    true
}

#[test]
fn lens_cut_is_orientation_independent() {
    // Both endpoints lie inside the front face's right boundary edge
    // (x = 0.5, y = 0, z in (0, 0.5)); the interior point bulges into the
    // face. Forward and reversed paths describe the same cut.
    let a = Point3::new(0.5, 0.0, 0.03125);
    let w = Point3::new(0.375, 0.0, 0.1875);
    let b = Point3::new(0.5, 0.0, 0.125);

    let mut results = Vec::new();
    for path in [vec![a, w, b], vec![b, w, a]] {
        let mut o = cube(0.5);
        let front = o
            .faces()
            .iter()
            .find(|(_, f)| f.plane.normal().y < -0.9)
            .map(|(id, _)| id)
            .expect("front face exists");
        o.split_face(front, &path).expect("lens cut applies");
        o.validate().expect("valid after lens cut");
        results.push(rings(&o));
    }
    assert!(
        same_rings(&results[0], &results[1]),
        "the two orientations of the same lens cut produced different geometry:\n{:?}\nvs\n{:?}",
        results[0],
        results[1]
    );

    // The right face gained both split vertices, in carrier order: its ring
    // must contain the sequence ... 0.5 -> 0.125 -> 0.03125 -> 0.0 ... along
    // the shared edge (descending z), never a doubled-back order.
    let right = results[0]
        .iter()
        .find(|r| r.len() == 6 && r.iter().all(|p| (p.x - 0.5).abs() < 1e-9))
        .expect("right face is the hexagon");
    let zs: Vec<f64> = right
        .iter()
        .filter(|p| p.y.abs() < 1e-9)
        .map(|p| p.z)
        .collect();
    // The 4 vertices on the front-right carrier edge, in ring order starting
    // anywhere: as a cycle they must be monotone (one descending run).
    let start = zs
        .iter()
        .position(|&z| (z - 0.5).abs() < 1e-9)
        .expect("carrier top vertex present");
    let ordered: Vec<f64> = (0..zs.len()).map(|k| zs[(start + k) % zs.len()]).collect();
    let descending = ordered.windows(2).all(|w| w[0] > w[1]);
    let ascending = ordered.windows(2).all(|w| w[0] < w[1]);
    assert!(
        descending || ascending,
        "split vertices out of carrier order on the right face: {ordered:?}"
    );
}
