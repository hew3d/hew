//! Regression repro distilled from `op_fuzz.rs`: replayed ops used to
//! COMMIT their recomputed geometry, so undo/redo cycles accumulated
//! floating-point noise (`fl(fl(x + d) - d) != x`, amplified through refit
//! normals by sweep-distance over face-extent per cycle). On this L-prism
//! sequence the second full replay's last redo saw a transverse neighbor
//! whose accumulated normal drift pushed |n1.n2| one hair past
//! `tol::NORMAL_DIRECTION`, classified it as a slanted neighbor, and
//! refused a push/pull the forward pass (and the first replay) had
//! accepted.
//!
//! Resolved by proof ALIGNMENT (rule 9): a replayed op's verified result is
//! aligned to the recorded coordinates before committing, so every replay
//! re-enters the exact bits its forward op produced and noise cannot
//! accumulate across cycles.
use kernel::{History, KernelOp, Object, Plane, Point3, Profile, Vec3};

fn nth_face(object: &Object, sel: usize) -> kernel::FaceId {
    object
        .faces()
        .keys()
        .nth(sel % object.faces().len())
        .unwrap()
}

#[test]
fn replay_cycles_do_not_accumulate_noise() {
    let (dx, dy) = (2.0, 3.578092432849931);
    let (nx, ny) = (0.5, 2.6557202516578204);
    let h = 0.5;
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(dx, 0.0, 0.0),
            Point3::new(dx, ny, 0.0),
            Point3::new(nx, ny, 0.0),
            Point3::new(nx, dy, 0.0),
            Point3::new(0.0, dy, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let mut object = Object::from_extrusion(&profile, h).unwrap();
    let mut history = History::new();

    let op = KernelOp::PushPull {
        face: nth_face(&object, 15873995167184219180),
        distance: -2.647936640654412,
    };
    let _ = history.apply(&mut object, op);
    {
        let face = nth_face(&object, 13130159919218771083);
        let boundary: Vec<Point3> = object
            .loop_positions(object.faces()[face].outer_loop)
            .collect();
        let sides = boundary.len();
        let (a, b) = (
            227270876956555411usize % sides,
            9199835075509323285usize % sides,
        );
        if a != b {
            let point_on = |i: usize, t: f64| {
                let p = boundary[i % sides];
                let q = boundary[(i + 1) % sides];
                p + (q - p) * t
            };
            let op = KernelOp::SplitFace {
                face,
                path: vec![point_on(a, 0.25), point_on(b, 0.745254278052551)],
                restore: None,
            };
            let _ = history.apply(&mut object, op);
        }
    }
    {
        let face = nth_face(&object, 17083764546689378);
        let boundary: Vec<Point3> = object
            .loop_positions(object.faces()[face].outer_loop)
            .collect();
        let inv = 1.0 / boundary.len() as f64;
        let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
            Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
        });
        let loop_path: Vec<Point3> = boundary.iter().map(|&p| c + (p - c) * 0.3).collect();
        let _ = history.apply(
            &mut object,
            KernelOp::SplitFaceInner {
                face,
                loop_path,
                restore: None,
                curve: None,
            },
        );
    }
    let op = KernelOp::PushPull {
        face: nth_face(&object, 7053327063461211258),
        distance: 2.9500675178426223,
    };
    let _ = history.apply(&mut object, op);
    object.validate().expect("valid after forward ops");

    // Contract: every recorded inverse and redo succeeds on EVERY cycle
    // (DEVELOPMENT.md rule 9); pre-alignment, cycle 1's last redo failed.
    for cycle in 0..3 {
        let mut n = 0;
        while history.can_undo() {
            history
                .undo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle} undo #{n}: {e}"));
            object.validate().expect("valid after undo");
            n += 1;
        }
        let mut k = 0;
        while history.can_redo() {
            history
                .redo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle} redo #{k}: {e}"));
            object.validate().expect("valid after redo");
            k += 1;
        }
    }
}
