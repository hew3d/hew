//! Deterministic repro distilled from `op_fuzz.rs`: the first cut's endpoint
//! subdivides an edge shared with a second face; the second cut then runs
//! collinear along that face's boundary, passing through the vertex the first
//! cut created. Before the fix this degenerate cut was accepted, minting a
//! zero-area sliver face that poisoned later ops
//! (`TopologyError::EdgeHalfEdgeMismatch` inside a subsequent `push_pull`).
//! `split_face` must refuse it as `PathNotSimple`: a cut whose interior
//! touches the boundary does not produce exactly two faces.

use kernel::{History, KernelOp, KernelOpError, Object, Point3, StickyError};

/// Straight boundary-to-boundary cut across the `face_sel`-th live face, from
/// parameter `ta` on boundary edge `edge_a` to `tb` on `edge_b` (all selectors
/// taken modulo the live counts, mirroring the fuzz harness's `resolve`).
fn split_op(
    object: &Object,
    face_sel: usize,
    edge_a: usize,
    edge_b: usize,
    ta: f64,
    tb: f64,
) -> KernelOp {
    let face = object
        .faces()
        .keys()
        .nth(face_sel % object.faces().len())
        .unwrap();
    let boundary: Vec<Point3> = object
        .loop_positions(object.faces()[face].outer_loop)
        .collect();
    let sides = boundary.len();
    let point_on = |i: usize, t: f64| {
        let p = boundary[i % sides];
        let q = boundary[(i + 1) % sides];
        p + (q - p) * t
    };
    KernelOp::SplitFace {
        face,
        path: vec![point_on(edge_a % sides, ta), point_on(edge_b % sides, tb)],
        restore: None,
    }
}

#[test]
fn split_split_keeps_valid_topology() {
    let d = 0.5;
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(d, 0.0, 0.0),
        Point3::new(d, d, 0.0),
        Point3::new(0.0, d, 0.0),
        Point3::new(0.0, 0.0, d),
        Point3::new(d, 0.0, d),
        Point3::new(d, d, d),
        Point3::new(0.0, d, d),
    ];
    let f = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let mut object = Object::from_polygons(&v, &f).unwrap();
    let mut history = History::new();

    let op1 = split_op(
        &object,
        3145299265929052019,
        15766876595271498524,
        17728904028236275183,
        0.5601091566745782,
        0.41793440649599584,
    );
    let _ = history.apply(&mut object, op1);
    object.validate().expect("valid after op 1 (split)");

    let op2 = split_op(
        &object,
        12207525668406599104,
        17658350389635835705,
        637483992341533911,
        0.4822363305983528,
        0.69686830221714,
    );
    let result = history.apply(&mut object, op2);
    assert_eq!(
        result.unwrap_err(),
        KernelOpError::Sticky(StickyError::PathNotSimple),
        "a cut collinear with the face boundary must be refused"
    );
    object.validate().expect("valid after refused op 2 (split)");
}
