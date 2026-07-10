//! Deterministic repro distilled from `op_fuzz.rs`: push/pull a box face,
//! then make two straight cuts; the second cut corrupts an edge/half-edge
//! relationship (`TopologyError::EdgeHalfEdgeMismatch`), caught by
//! `check_invariants` in debug builds.

use kernel::{History, KernelOp, Object, Point3};

/// Mirrors the fuzz harness's `resolve` for SplitFace: straight cut across
/// the `face_sel`-th live face between parameters `ta`/`tb` of boundary edges
/// `edge_a`/`edge_b` (selectors modulo live counts).
fn split_op(
    object: &Object,
    face_sel: usize,
    edge_a: usize,
    edge_b: usize,
    ta: f64,
    tb: f64,
) -> Option<KernelOp> {
    let face = object.faces().keys().nth(face_sel % object.faces().len())?;
    let boundary: Vec<Point3> = object
        .loop_positions(object.faces()[face].outer_loop)
        .collect();
    let sides = boundary.len();
    if sides < 3 {
        return None;
    }
    let (a, b) = (edge_a % sides, edge_b % sides);
    if a == b {
        return None;
    }
    let point_on = |i: usize, t: f64| {
        let p = boundary[i];
        let q = boundary[(i + 1) % sides];
        p + (q - p) * t
    };
    Some(KernelOp::SplitFace {
        face,
        path: vec![point_on(a, ta), point_on(b, tb)],
    })
}

#[test]
fn pushpull_then_two_splits_keeps_valid_topology() {
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

    // Op 1: rejected push/pull (would vanish); must be a no-op.
    let face0 = object.faces().keys().next().unwrap();
    let _ = history.apply(
        &mut object,
        KernelOp::PushPull {
            face: face0,
            distance: -4.107814479564193,
        },
    );
    object.validate().expect("valid after op 1");

    // Op 2: push/pull face 0 outward.
    let face0 = object.faces().keys().next().unwrap();
    let _ = history.apply(
        &mut object,
        KernelOp::PushPull {
            face: face0,
            distance: 4.8045967137976175,
        },
    );
    object.validate().expect("valid after op 2");

    // Op 3: first cut.
    if let Some(op) = split_op(
        &object,
        3186759898719533190,
        3273424489668098496,
        640155907872384363,
        0.25,
        0.25,
    ) {
        let _ = history.apply(&mut object, op);
        object.validate().expect("valid after op 3");
    }

    // Op 4: second cut.
    if let Some(op) = split_op(
        &object,
        10570850251180526830,
        13975937294210189536,
        1292811894643448327,
        0.25,
        0.25,
    ) {
        let _ = history.apply(&mut object, op);
        object.validate().expect("valid after op 4");
    }

    // Round-trip the history twice (mirrors the fuzz harness). In debug
    // builds this currently panics inside check_invariants with
    // EdgeHalfEdgeMismatch during the unwind's merge_faces.
    for cycle in 0..2 {
        let mut n = 0;
        while history.can_undo() {
            history
                .undo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle} undo #{n}: {e}"));
            object.validate().expect("valid after undo");
            n += 1;
        }
        let mut n = 0;
        while history.can_redo() {
            history
                .redo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle} redo #{n}: {e}"));
            object.validate().expect("valid after redo");
            n += 1;
        }
    }
}
