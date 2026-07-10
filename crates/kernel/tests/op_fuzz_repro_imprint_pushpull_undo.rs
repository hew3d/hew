//! Deterministic repro distilled from `op_fuzz.rs`: imprint a sub-face, then
//! push/pull faces around it with an interleaved undo; unwinding the history
//! afterwards fails — the recorded inverse of a push/pull is refused with
//! `PushPullError::NonManifoldResult`, violating the history contract that a
//! recorded inverse always succeeds.

use kernel::{History, KernelOp, Object, Point3};

fn nth_face(object: &Object, sel: usize) -> kernel::FaceId {
    object
        .faces()
        .keys()
        .nth(sel % object.faces().len())
        .unwrap()
}

/// Mirrors the fuzz harness's SplitFaceInner resolution: shrink the face's
/// outer boundary toward its vertex centroid.
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
    KernelOp::SplitFaceInner { face, loop_path }
}

#[test]
fn imprint_pushpull_undo_history_unwinds() {
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

    let op = imprint_op(&object, 5596565772054426179, 0.3);
    let _ = history.apply(&mut object, op);
    let op = KernelOp::PushPull {
        face: nth_face(&object, 439059453726638533),
        distance: 3.391126511733209,
    };
    let _ = history.apply(&mut object, op);
    let op = imprint_op(&object, 8037413331546221040, 0.3);
    let _ = history.apply(&mut object, op);
    let op = imprint_op(&object, 0, 0.4535824240610475);
    let _ = history.apply(&mut object, op);
    if history.can_undo() {
        history.undo(&mut object).expect("in-sequence undo");
    }
    // Before the fix, this push was ACCEPTED: it drives a recess tunnel wall
    // outward through the opposite tunnel wall (self-intersecting geometry),
    // and the recorded inverse then failed against the inward obstruction
    // guard. The bidirectional guard now refuses the forward push instead.
    let op = KernelOp::PushPull {
        face: nth_face(&object, 1767307280586049375),
        distance: 1.5572371392648383,
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    // Unwind everything: every recorded inverse must succeed.
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut object)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        object.validate().expect("valid after undo");
        n += 1;
    }
}
