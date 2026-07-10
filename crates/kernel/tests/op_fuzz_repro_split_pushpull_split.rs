//! Deterministic repro distilled from `op_fuzz.rs` (forward ops only, no
//! undo): split a box face, push/pull one of the resulting faces, then split
//! again. The final split corrupts the half-edge structure — a loop's
//! `first_half_edge` dangles — which `check_invariants` catches in debug
//! builds (`validate.rs`: "dangling handle: loop first half-edge").

use kernel::{History, KernelOp, Object, Point3};

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
    }
}

#[test]
fn split_pushpull_split_keeps_valid_topology() {
    let (dx, dy, dz) = (0.5, 7.246204574368695, 0.5);
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(dx, 0.0, 0.0),
        Point3::new(dx, dy, 0.0),
        Point3::new(0.0, dy, 0.0),
        Point3::new(0.0, 0.0, dz),
        Point3::new(dx, 0.0, dz),
        Point3::new(dx, dy, dz),
        Point3::new(0.0, dy, dz),
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

    // Op 1: split (raw selectors from the shrunk fuzz case).
    let op1 = split_op(
        &object,
        17784677413216664215,
        5348687560746595292,
        6939555044569115859,
        0.5688110062031128,
        0.3536490156018881,
    );
    let _ = history.apply(&mut object, op1);
    object.validate().expect("valid after op 1 (split)");

    // Op 2: push/pull.
    let face = object
        .faces()
        .keys()
        .nth(5140628813309177490usize % object.faces().len())
        .unwrap();
    let _ = history.apply(
        &mut object,
        KernelOp::PushPull {
            face,
            distance: 3.4489937287644037,
        },
    );
    object.validate().expect("valid after op 2 (push/pull)");

    // Op 3: split again. In debug builds this currently panics inside
    // check_invariants; the object is corrupted mid-mutation.
    let op3 = split_op(
        &object,
        2478978223319388793,
        1800845116828371340,
        13964960310677115469,
        0.5234727493696296,
        0.3147766100447447,
    );
    let _ = history.apply(&mut object, op3);
    object.validate().expect("valid after op 3 (split)");
}
