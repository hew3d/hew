//! Acceptance spec distilled from `op_fuzz.rs` for a since-resolved contract
//! gap: undoing a recess whose stretched side walls carry subdivision
//! vertices (here: from a raised step bordering the pocket, spliced in by an
//! earlier coplanar-aware push) was refused by `push_pull`'s
//! interior-obstruction guard, which misread those wall vertices as fixed
//! obstructions strictly in front of the translate-back sweep. Undo failed
//! typed with `NonManifoldResult` instead of restoring the object (never
//! corrupting it; the inverse property was what broke).
//!
//! Resolved by DEVELOPMENT.md rule 9 (ARCHITECTURE.md §5.7): history replay
//! is guard-exempt and proof-carrying, so the heuristic cannot refuse the
//! recorded inverse. The translate-back surgery itself restores the
//! subdivided walls exactly (they un-stretch to the shapes they were built
//! with — no wall degenerates and no collapse detection is involved), and
//! the replay's result is verified against the recorded pre-op state's
//! fingerprint before committing. Generalizing `find_collapse_plans` beyond
//! quad walls was NOT needed for this class: the failing inverse was never a
//! collapse, only a guarded translate.

use kernel::{History, KernelOp, Object, Point3};

fn nth_face(object: &Object, sel: usize) -> kernel::FaceId {
    object
        .faces()
        .keys()
        .nth(sel % object.faces().len())
        .unwrap()
}

/// Mirrors the fuzz harness's SplitFace resolution.
fn split_op(
    object: &Object,
    face_sel: usize,
    edge_a: usize,
    edge_b: usize,
    ta: f64,
    tb: f64,
) -> KernelOp {
    let face = nth_face(object, face_sel);
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

/// Mirrors the fuzz harness's SplitFaceInner resolution.
fn imprint_op(object: &Object, face_sel: usize, shrink: f64) -> KernelOp {
    let face = nth_face(object, face_sel);
    let boundary: Vec<Point3> = object
        .loop_positions(object.faces()[face].outer_loop)
        .collect();
    let inv = 1.0 / boundary.len() as f64;
    let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
        Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
    });
    let loop_path: Vec<Point3> = boundary.iter().map(|&p| c + (p - c) * shrink).collect();
    KernelOp::SplitFaceInner {
        face,
        loop_path,
        restore: None,
        curve: None,
    }
}

#[test]
fn undo_closes_pocket_whose_walls_were_subdivided() {
    let (dx, dy, dz) = (3.4732755870644763, 0.5, 0.5);
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

    let op = split_op(
        &object,
        1205648497134094197,
        2868677407249015972,
        5701500092296864303,
        0.25,
        0.25,
    );
    let _ = history.apply(&mut object, op);
    let op = split_op(
        &object,
        5922937443923893501,
        8960320271775487091,
        4842985451379951130,
        0.25,
        0.25,
    );
    let _ = history.apply(&mut object, op);
    let op = imprint_op(&object, 8631195973038952049, 0.3);
    let _ = history.apply(&mut object, op);
    let op = KernelOp::PushPull {
        face: nth_face(&object, 13829145998056071474),
        distance: 0.07923999262641618,
    };
    let _ = history.apply(&mut object, op);
    let op = KernelOp::PushPull {
        face: nth_face(&object, 8345945516403286721),
        distance: -0.633792794888971,
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    // Contract: every recorded inverse succeeds. Before rule 9 the first
    // undo failed with InverseFailed(PushPull(NonManifoldResult)).
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut object)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        object.validate().expect("valid after undo");
        n += 1;
    }
}
