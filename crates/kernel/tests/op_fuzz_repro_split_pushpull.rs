//! Deterministic repro distilled from `op_fuzz.rs`: splitting a box face with
//! a straight boundary-to-boundary cut and then push/pulling one of the
//! resulting sub-faces leaves a loop whose `first_half_edge` dangles
//! (`TopologyError::DanglingHandle { context: "loop first half-edge" }`),
//! caught by `check_invariants` in debug builds.

use kernel::{History, KernelOp, Object, Point3};

#[test]
fn split_then_push_pull_keeps_valid_topology() {
    // 0.5 x 7.246... x 0.5 box at the origin (shrunk fuzz case; the long
    // dimension is not load-bearing, but keep the case verbatim).
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

    // Split face #5 straight across: from the midpoint-ish (t = 0.25) of its
    // boundary edge #2 to t = 0.25 of its boundary edge #3.
    let face = object.faces().keys().nth(5).unwrap();
    let boundary: Vec<Point3> = object
        .loop_positions(object.faces()[face].outer_loop)
        .collect();
    let point_on = |i: usize, t: f64| {
        let p = boundary[i];
        let q = boundary[(i + 1) % boundary.len()];
        p + (q - p) * t
    };
    history
        .apply(
            &mut object,
            KernelOp::SplitFace {
                face,
                path: vec![point_on(2, 0.25), point_on(3, 0.25)],
                restore: None,
            },
        )
        .expect("split is valid");
    object.validate().expect("valid after split");

    // Push/pull the 6th live face outward. In debug builds this currently
    // panics via check_invariants; in release it returns Ok with corrupted
    // topology, which validate() then reports.
    let target = object.faces().keys().nth(5).unwrap();
    let result = history.apply(
        &mut object,
        KernelOp::PushPull {
            face: target,
            distance: 3.4489937287644037,
        },
    );

    match result {
        Ok(_) => object.validate().expect("valid after push/pull"),
        Err(_) => object
            .validate()
            .expect("rejected op must leave the object untouched and valid"),
    }

    // Unwind: undo the push/pull, then undo the split (a MergeFaces of the
    // cut edge). The topology must stay valid at every step.
    while history.can_undo() {
        history.undo(&mut object).expect("undo must succeed");
        object.validate().expect("valid after undo");
    }

    // Replay: redo the split, then redo the push/pull, via the re-anchored
    // ops derived at undo time.
    while history.can_redo() {
        history.redo(&mut object).expect("redo must succeed");
        object.validate().expect("valid after redo");
    }
}
