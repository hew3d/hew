//! Deterministic repro distilled from `op_fuzz.rs`: a push that swings a
//! neighboring face's cut-edge boundary can sweep that boundary ACROSS an
//! imprinted ring on the face, leaving the ring poking outside its parent's
//! outer polygon — planar and twin-consistent, so the validator cannot see
//! it, and the ring's own inverses fail afterwards. `refit_face_plane` now
//! refuses the push (inner loops must stay strictly inside the reshaped
//! outer boundary), so this sequence's push fails typed and the history
//! unwinds cleanly.

use kernel::{History, KernelOp, Object, Point3};

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
        let p = boundary[i];
        let q = boundary[(i + 1) % sides];
        p + (q - p) * t
    };
    Some(KernelOp::SplitFace {
        face,
        path: vec![point_on(a, ta), point_on(b, tb)],
    })
}

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
fn ring_kept_inside_reshaped_boundary() {
    let dxl = 6.043949932478207;
    let d = 0.5;
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(dxl, 0.0, 0.0),
        Point3::new(dxl, d, 0.0),
        Point3::new(0.0, d, 0.0),
        Point3::new(0.0, 0.0, d),
        Point3::new(dxl, 0.0, d),
        Point3::new(dxl, d, d),
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

    if let Some(op) = split_op(
        &object,
        5425739042031116221,
        15125578936822122142,
        4923581626061743039,
        0.25,
        0.25,
    ) {
        let _ = history.apply(&mut object, op);
    }
    if let Some(op) = split_op(
        &object,
        9042925749594819542,
        1583511761662585154,
        1862093078217424973,
        0.25,
        0.25,
    ) {
        let _ = history.apply(&mut object, op);
    }
    let op = imprint_op(&object, 1815147360634333537, 0.6040787726061253);
    let _ = history.apply(&mut object, op);
    let op = KernelOp::PushPull {
        face: nth_face(&object, 108380143547104622),
        distance: 0.9335101119512089,
    };
    let _ = history.apply(&mut object, op);
    if std::env::var("PROBE").is_ok() {
        for (fid, f) in object.faces() {
            if f.plane.normal().z.abs() > 0.5 {
                let pts: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
                let xy: Vec<(f64, f64)> = pts.iter().map(|p| (p.x, p.y)).collect();
                eprintln!(
                    "POST-PUSH face {fid:?} nz={} loop={xy:?}",
                    f.plane.normal().z
                );
            }
        }
    }
    let op = KernelOp::MergeInnerFace {
        sub_face: nth_face(&object, 2212764678860837318),
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    if std::env::var("PROBE").is_ok() {
        if let Some(kernel::KernelOp::SplitFaceInner { loop_path, .. }) = history.peek_undo() {
            eprintln!("pending re-imprint loop: {loop_path:?}");
        }
        for (fid, f) in object.faces() {
            let pts: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
            eprintln!("face {fid:?} n={:?} outer={pts:?}", f.plane.normal());
        }
    }
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut object)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        object.validate().expect("valid after undo");
        n += 1;
    }
}
