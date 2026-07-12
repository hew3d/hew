//! Acceptance spec distilled from `op_fuzz.rs` for a since-resolved contract
//! gap: the interior-obstruction guards differ in fidelity across ops that
//! are inverses of each other. Here a sub-face of a tetrahedron is recessed
//! via `push_pull` (vertex-heuristic guard: permissive — no tetra vertex sits
//! in front of the sweep, so a recess deeper than the material at the
//! centroid is accepted), then flattened via `collapse_sub_face`. The
//! collapse's recorded inverse dispatches as `extrude_sub_face`, whose
//! centroid-RAY guard saw the opposite face closer than the recess depth and
//! refused — undo failed typed with `NonManifoldResult` although the object
//! was valid and untouched.
//!
//! Resolved by DEVELOPMENT.md rule 9 (ARCHITECTURE.md §5.7): history replay
//! is guard-exempt and proof-carrying — the recorded inverse dispatches with
//! the heuristic guards skipped and its result is verified against the
//! recorded pre-op state's fingerprint before committing.

use kernel::{History, KernelOp, Object, Point3};

fn nth_face(object: &Object, sel: usize) -> kernel::FaceId {
    object
        .faces()
        .keys()
        .nth(sel % object.faces().len())
        .unwrap()
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
fn undo_of_collapse_survives_guard_regime_change() {
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(0.0, 0.0, -4.7793259739871035),
        Point3::new(0.0, 9.701425358426723, 0.0),
        Point3::new(5.553016263797945, 0.0, 0.0),
    ];
    let f = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 1, 3], vec![1, 2, 3]];
    let mut object = Object::from_polygons(&v, &f).unwrap();
    let mut history = History::new();

    let op = imprint_op(&object, 8865050982547732, 0.3);
    let _ = history.apply(&mut object, op);
    let op = imprint_op(&object, 5903517105019682981, 0.3);
    let _ = history.apply(&mut object, op);
    let op = imprint_op(&object, 1727731450235832212, 0.3);
    let _ = history.apply(&mut object, op);
    if history.can_undo() {
        history.undo(&mut object).expect("in-sequence undo");
    }
    let op = KernelOp::PushPull {
        face: nth_face(&object, 10170221171977651246),
        distance: -4.6414162357680455,
    };
    let _ = history.apply(&mut object, op);
    let op = KernelOp::CollapseSubFace {
        sub_face: nth_face(&object, 10926515914467240316),
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    // Contract: every recorded inverse succeeds. Before rule 9 the collapse's
    // inverse (extrude_sub_face) was refused by the centroid-ray guard.
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut object)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        object.validate().expect("valid after undo");
        n += 1;
    }
}
