//! Regression repro distilled from `op_fuzz.rs` (surfaced by the rule-9
//! replay proof): `merge_faces`' boundary scar healing iterated two FIXED
//! pre-heal half-edge pairs, and skipped the second endpoint whenever the
//! first heal had consumed the half-edges its pair referenced (a triangular
//! neighbor's single non-shared half-edge). Which endpoint came first
//! followed from which side of the merge edge was `face_a` — the edge's
//! primary-half-edge orientation, internal representation state — so the
//! same geometric merge healed one scar or both depending on hidden state.
//! Forward apply and its redo (dispatched via a re-anchored edge whose
//! primary can land on the other side) then produced different geometry:
//! the redo failed the recorded-state proof with `InverseDiverged`.
//!
//! Healing now re-derives each endpoint's live half-edges at heal time, so
//! every healable endpoint heals regardless of order or orientation, and
//! the sequence below round-trips: two splits, two merges, full unwind
//! (restores the seed), full replay (reproduces the forward result), twice.

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

fn polygons(o: &Object) -> Vec<Vec<Point3>> {
    let (pts, fs) = o.to_polygons();
    fs.into_iter()
        .map(|f| f.into_iter().map(|i| pts[i]).collect())
        .collect()
}

/// Multiset equality of face rings up to rotation (winding preserved).
fn same_rings(a: &[Vec<Point3>], b: &[Vec<Point3>]) -> bool {
    fn cyclic(a: &[Point3], b: &[Point3]) -> bool {
        a.len() == b.len()
            && (0..a.len()).any(|shift| {
                a.iter()
                    .enumerate()
                    .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], 1e-9))
            })
    }
    if a.len() != b.len() {
        return false;
    }
    let mut unmatched: Vec<&Vec<Point3>> = b.iter().collect();
    for ring in a {
        let Some(i) = unmatched.iter().position(|c| cyclic(ring, c)) else {
            return false;
        };
        unmatched.swap_remove(i);
    }
    true
}

#[test]
fn merge_heal_survives_undo_redo_cycles() {
    let s = 0.5;
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(s, 0.0, 0.0),
        Point3::new(s, s, 0.0),
        Point3::new(0.0, s, 0.0),
        Point3::new(0.0, 0.0, s),
        Point3::new(s, 0.0, s),
        Point3::new(s, s, s),
        Point3::new(0.0, s, s),
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
    let original = polygons(&object);
    let mut history = History::new();

    for (fs, ea, eb) in [
        (
            13695086383745519444usize,
            9458722942108286377usize,
            7151938534045946387usize,
        ),
        (
            3732565518912063819,
            8296618985924052586,
            9350451374055745315,
        ),
    ] {
        if let Some(op) = split_op(&object, fs, ea, eb, 0.25, 0.25) {
            let _ = history.apply(&mut object, op);
        }
    }
    for es in [2328960287675854673usize, 15360641641247688354] {
        let n = object.edges().len();
        let edge = object.edges().keys().nth(es % n).unwrap();
        let _ = history.apply(&mut object, KernelOp::MergeFaces { edge });
    }
    object.validate().expect("valid after forward ops");
    let end_state = polygons(&object);

    // Two full unwind/replay cycles. Every dispatch is verified against its
    // recorded state by the History itself (rule 9); the ring comparisons
    // here pin the endpoints.
    for cycle in 0..2 {
        let mut n = 0;
        while history.can_undo() {
            history
                .undo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle}, undo #{n}: {e}"));
            object.validate().expect("valid after undo");
            n += 1;
        }
        assert!(
            same_rings(&original, &polygons(&object)),
            "cycle {cycle}: full undo did not restore the seed"
        );
        let mut k = 0;
        while history.can_redo() {
            history
                .redo(&mut object)
                .unwrap_or_else(|e| panic!("cycle {cycle}, redo #{k}: {e}"));
            object.validate().expect("valid after redo");
            k += 1;
        }
        assert!(
            same_rings(&end_state, &polygons(&object)),
            "cycle {cycle}: full redo did not reproduce the forward result"
        );
    }
}
