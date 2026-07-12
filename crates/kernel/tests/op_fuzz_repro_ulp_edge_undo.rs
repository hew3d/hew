//! Regression repro distilled from `op_fuzz.rs`: undoing a coplanar-aware
//! push whose junction vertices translate back with ulp-level rounding
//! (`fl(fl(z + d) - d)` lands one ulp off the sibling plane) reshaped a
//! straddling side wall into a boundary whose two top-edge pieces differ by
//! that ulp. `segments_intersect`'s proper-crossing branch tested raw sign
//! flips with no epsilon, so orientation values of ~1e-16 — seven orders of
//! magnitude inside the collinearity tolerance the very next branch applies —
//! counted as a crossing, `polygon_is_simple` declared the wall self-
//! intersecting, and the recorded inverse was refused typed
//! (`NonManifoldResult`) although the restored geometry is fine.
//!
//! Scene: a hexagonal prism, three cap splits (with an undo/redo mixed in),
//! a recess and a raise on two cap pieces, then a full unwind. Before the
//! orientation-epsilon fix the first undo (the raise's collapse) failed.
use kernel::{History, KernelOp, Object, Plane, Point3, Profile, Vec3};

fn nth_face(object: &Object, sel: usize) -> kernel::FaceId {
    object
        .faces()
        .keys()
        .nth(sel % object.faces().len())
        .unwrap()
}

fn split_op(
    object: &Object,
    face_sel: usize,
    edge_a: usize,
    edge_b: usize,
    ta: f64,
    tb: f64,
) -> Option<KernelOp> {
    let face = nth_face(object, face_sel);
    let boundary: Vec<Point3> = object
        .loop_positions(object.faces()[face].outer_loop)
        .collect();
    let sides = boundary.len();
    let (a, b) = (edge_a % sides, edge_b % sides);
    if a == b {
        return None;
    }
    let point_on = |i: usize, t: f64| {
        let p = boundary[i % sides];
        let q = boundary[(i + 1) % sides];
        p + (q - p) * t
    };
    Some(KernelOp::SplitFace {
        face,
        path: vec![point_on(a, ta), point_on(b, tb)],
        restore: None,
    })
}

#[test]
fn undo_survives_ulp_noise_on_straddling_wall_edges() {
    let (r, h) = (1.616283188561696f64, 6.969601897325285f64);
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let outer: Vec<Point3> = (0..6)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 6.0;
            Point3::new(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect();
    let profile = Profile::new(plane, outer, vec![]).unwrap();
    let mut object = Object::from_extrusion(&profile, h).unwrap();
    let mut history = History::new();

    if let Some(op) = split_op(
        &object,
        10863642120842647449,
        6683968343016945843,
        3786353811171308394,
        0.6225008602296737,
        0.4329470012194594,
    ) {
        let _ = history.apply(&mut object, op);
    }
    if history.can_undo() {
        history.undo(&mut object).expect("in-sequence undo");
    }
    if history.can_redo() {
        history.redo(&mut object).expect("in-sequence redo");
    }
    if let Some(op) = split_op(
        &object,
        3221568473471075728,
        629896748907071378,
        2426514873486761384,
        0.45066374636079704,
        0.27034671059609205,
    ) {
        let _ = history.apply(&mut object, op);
    }
    if let Some(op) = split_op(
        &object,
        5496760160763061189,
        7957996194491930701,
        15538967169842082718,
        0.25,
        0.4214887670217793,
    ) {
        let _ = history.apply(&mut object, op);
    }
    let op = KernelOp::PushPull {
        face: nth_face(&object, 11886399562821017915),
        distance: -3.3353707894308293,
    };
    let _ = history.apply(&mut object, op);
    let op = KernelOp::PushPull {
        face: nth_face(&object, 7410866372729772422),
        distance: 1.4643346391050072,
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    // Contract: every recorded inverse succeeds (DEVELOPMENT.md rule 9).
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut object)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        object.validate().expect("valid after undo");
        n += 1;
    }
}
