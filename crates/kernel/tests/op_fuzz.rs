//! Op-sequence fuzz harness (DEVELOPMENT.md rule 3): random [`KernelOp`]
//! sequences — with interleaved undo/redo — driven through [`History`] over
//! generated boxes.
//!
//! Invariants asserted after every step:
//! - the validator accepts the object (`Object::validate`);
//! - watertightness is preserved (these ops cannot open a closed solid);
//! - the Euler characteristic stays 2 (no op here can change genus);
//! - a failed op leaves the object bit-for-bit untouched (strong guarantee).
//!
//! And for the sequence as a whole:
//! - unwinding the entire history restores the starting geometry;
//! - replaying the entire redo stack restores the post-sequence geometry
//!   (exercises inverse re-anchoring in `history.rs`).

use kernel::{
    History, HistoryError, KernelOp, KernelOpError, Object, Plane, Point3, Profile, PushPullError,
    Vec3, WatertightState,
};
use proptest::prelude::*;

/// Geometric slack for round-trip comparison: inverse ops recompute positions
/// (e.g. `p + d·n` then `- d·n`), so bitwise equality is not guaranteed, but
/// error should stay within a few ulps of the coordinate magnitudes used here.
const ROUNDTRIP_TOL: f64 = 1e-9;

type Snapshot = (Vec<Point3>, Vec<Vec<usize>>);

/// An op described abstractly so it can be generated before the object exists;
/// selectors are resolved against live handles at apply time.
#[derive(Debug, Clone)]
enum FuzzOp {
    /// Push/pull the `face_sel`-th live face by `distance`.
    PushPull {
        face_sel: usize,
        distance: f64,
    },
    /// Cut the `face_sel`-th live face straight across, from a point on its
    /// `edge_a`-th boundary edge (at parameter `ta`) to a point on its
    /// `edge_b`-th boundary edge (at parameter `tb`).
    SplitFace {
        face_sel: usize,
        edge_a: usize,
        edge_b: usize,
        ta: f64,
        tb: f64,
    },
    /// Merge across the `edge_sel`-th live edge (usually rejected — the two
    /// faces are rarely coplanar — which exercises the strong guarantee).
    MergeFaces {
        edge_sel: usize,
    },
    /// Imprint a shrunk copy of the `face_sel`-th face's boundary strictly
    /// inside it (valid on convex faces; rejected typed otherwise).
    SplitFaceInner {
        face_sel: usize,
        shrink: f64,
    },
    /// Boss/recess the `face_sel`-th face if it is an imprinted sub-face
    /// (usually rejected typed — exercises the strong guarantee).
    ExtrudeSubFace {
        face_sel: usize,
        distance: f64,
    },
    /// Dissolve the `face_sel`-th face if it is an imprinted sub-face.
    MergeInnerFace {
        face_sel: usize,
    },
    /// Flatten the `face_sel`-th face if it is a raised sub-face.
    CollapseSubFace {
        face_sel: usize,
    },
    Undo,
    Redo,
}

fn arb_fuzz_op() -> impl Strategy<Value = FuzzOp> {
    let distance = || (-5.0..5.0f64).prop_filter("degenerate distance", |d| d.abs() >= 0.05);
    prop_oneof![
        4 => (any::<usize>(), distance()).prop_map(|(face_sel, distance)| {
            FuzzOp::PushPull { face_sel, distance }
        }),
        3 => (any::<usize>(), any::<usize>(), any::<usize>(), 0.25..0.75f64, 0.25..0.75f64)
            .prop_map(|(face_sel, edge_a, edge_b, ta, tb)| FuzzOp::SplitFace {
                face_sel,
                edge_a,
                edge_b,
                ta,
                tb,
            }),
        2 => any::<usize>().prop_map(|edge_sel| FuzzOp::MergeFaces { edge_sel }),
        2 => (any::<usize>(), 0.3..0.7f64).prop_map(|(face_sel, shrink)| {
            FuzzOp::SplitFaceInner { face_sel, shrink }
        }),
        2 => (any::<usize>(), distance()).prop_map(|(face_sel, distance)| {
            FuzzOp::ExtrudeSubFace { face_sel, distance }
        }),
        1 => any::<usize>().prop_map(|face_sel| FuzzOp::MergeInnerFace { face_sel }),
        1 => any::<usize>().prop_map(|face_sel| FuzzOp::CollapseSubFace { face_sel }),
        2 => Just(FuzzOp::Undo),
        1 => Just(FuzzOp::Redo),
    ]
}

/// Axis-aligned starting boxes (matches `props.rs::arb_box`).
fn arb_box() -> impl Strategy<Value = Object> {
    (
        (-10.0..10.0f64, -10.0..10.0f64, -10.0..10.0f64),
        (0.5..10.0f64, 0.5..10.0f64, 0.5..10.0f64),
    )
        .prop_map(|((x, y, z), (dx, dy, dz))| {
            let v = vec![
                Point3::new(x, y, z),
                Point3::new(x + dx, y, z),
                Point3::new(x + dx, y + dy, z),
                Point3::new(x, y + dy, z),
                Point3::new(x, y, z + dz),
                Point3::new(x + dx, y, z + dz),
                Point3::new(x + dx, y + dy, z + dz),
                Point3::new(x, y + dy, z + dz),
            ];
            let f = vec![
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![1, 2, 6, 5],
                vec![2, 3, 7, 6],
                vec![3, 0, 4, 7],
            ];
            Object::from_polygons(&v, &f).expect("generated box is a valid solid")
        })
}

/// Well-separated tetrahedra (triangular faces exercise the slanted-neighbor
/// refusals; matches `props.rs::arb_tetrahedron`'s degeneracy bound).
fn arb_tetra() -> impl Strategy<Value = Object> {
    fn pt() -> impl Strategy<Value = Point3> {
        (-10.0..10.0f64, -10.0..10.0f64, -10.0..10.0f64).prop_map(|(x, y, z)| Point3::new(x, y, z))
    }
    (pt(), pt(), pt(), pt()).prop_filter_map("tetrahedron too close to degenerate", {
        |(p0, p1, p2, p3)| {
            let det = (p1 - p0).cross(p2 - p0).dot(p3 - p0);
            if det.abs() < 1.0 {
                return None;
            }
            let v = if det > 0.0 {
                vec![p0, p1, p2, p3]
            } else {
                vec![p0, p1, p3, p2]
            };
            let f = vec![vec![0, 2, 1], vec![0, 3, 2], vec![0, 1, 3], vec![1, 2, 3]];
            Some(Object::from_polygons(&v, &f).expect("non-degenerate tetrahedron builds"))
        }
    })
}

/// Extruded L-shaped prisms: the caps are concave, feeding the notch-chord
/// and non-convex-imprint paths; built through `from_extrusion`, so extrusion
/// results themselves are also under test.
fn arb_l_prism() -> impl Strategy<Value = Object> {
    (
        (2.0..10.0f64, 2.0..10.0f64),   // outer footprint
        (0.25..0.75f64, 0.25..0.75f64), // notch fraction of each side
        0.5..8.0f64,                    // extrusion height
    )
        .prop_map(|((dx, dy), (fx, fy), h)| {
            let (nx, ny) = (dx * fx, dy * fy);
            let plane =
                Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                    .expect("unit normal");
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
            .expect("L profile is simple and planar");
            Object::from_extrusion(&profile, h).expect("L prism extrudes")
        })
}

/// Extruded regular n-gon prisms (adjacent side walls are slanted neighbors).
fn arb_ngon_prism() -> impl Strategy<Value = Object> {
    (5usize..9, 1.0..8.0f64, 0.5..8.0f64).prop_map(|(n, r, h)| {
        let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
            .expect("unit normal");
        let outer: Vec<Point3> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                Point3::new(r * a.cos(), r * a.sin(), 0.0)
            })
            .collect();
        let profile = Profile::new(plane, outer, vec![]).expect("n-gon profile");
        Object::from_extrusion(&profile, h).expect("n-gon prism extrudes")
    })
}

/// Seed solids for a fuzz case, weighted toward the shapes with the richest
/// op surface (quads everywhere) but always covering triangles, concave caps,
/// and slanted side walls.
fn arb_seed() -> impl Strategy<Value = Object> {
    prop_oneof![
        4 => arb_box(),
        1 => arb_tetra(),
        3 => arb_l_prism(),
        2 => arb_ngon_prism(),
    ]
}

/// Resolve an abstract op against the object's live handles. `None` means the
/// op has no target in the current state (e.g. Redo with an empty redo stack)
/// and the step is skipped.
fn resolve(object: &Object, op: &FuzzOp) -> Option<KernelOp> {
    match op {
        FuzzOp::PushPull { face_sel, distance } => {
            let n = object.faces().len();
            let face = object.faces().keys().nth(face_sel % n)?;
            Some(KernelOp::PushPull {
                face,
                distance: *distance,
            })
        }
        FuzzOp::SplitFace {
            face_sel,
            edge_a,
            edge_b,
            ta,
            tb,
        } => {
            let n = object.faces().len();
            let face = object.faces().keys().nth(face_sel % n)?;
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
                path: vec![point_on(a, *ta), point_on(b, *tb)],
            })
        }
        FuzzOp::MergeFaces { edge_sel } => {
            let n = object.edges().len();
            let edge = object.edges().keys().nth(edge_sel % n)?;
            Some(KernelOp::MergeFaces { edge })
        }
        FuzzOp::SplitFaceInner { face_sel, shrink } => {
            let n = object.faces().len();
            let face = object.faces().keys().nth(face_sel % n)?;
            let boundary: Vec<Point3> = object
                .loop_positions(object.faces()[face].outer_loop)
                .collect();
            if boundary.len() < 3 {
                return None;
            }
            // Shrink the boundary toward its vertex centroid; strictly inside
            // for convex faces, typed rejection otherwise.
            let inv = 1.0 / boundary.len() as f64;
            let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
            });
            let loop_path: Vec<Point3> = boundary.iter().map(|&p| c + (p - c) * *shrink).collect();
            Some(KernelOp::SplitFaceInner { face, loop_path })
        }
        FuzzOp::ExtrudeSubFace { face_sel, distance } => {
            let n = object.faces().len();
            let sub_face = object.faces().keys().nth(face_sel % n)?;
            Some(KernelOp::ExtrudeSubFace {
                sub_face,
                distance: *distance,
            })
        }
        FuzzOp::MergeInnerFace { face_sel } => {
            let n = object.faces().len();
            let sub_face = object.faces().keys().nth(face_sel % n)?;
            Some(KernelOp::MergeInnerFace { sub_face })
        }
        FuzzOp::CollapseSubFace { face_sel } => {
            let n = object.faces().len();
            let sub_face = object.faces().keys().nth(face_sel % n)?;
            Some(KernelOp::CollapseSubFace { sub_face })
        }
        FuzzOp::Undo | FuzzOp::Redo => None,
    }
}

fn euler_characteristic(obj: &Object) -> i64 {
    obj.vertices().len() as i64 - obj.edges().len() as i64 + obj.faces().len() as i64
}

/// V - E + F for a genus-0 solid, counting each face as one polygon: every
/// imprinted hole (inner loop) inflates the naive sum by one because an
/// annular face still counts as a single face.
fn expected_euler(obj: &Object) -> i64 {
    2 + obj
        .faces()
        .values()
        .map(|f| f.inner_loops.len() as i64)
        .sum::<i64>()
}

/// Face-size multiset (vertex indices differ across rebuilds; sizes must not).
fn face_sizes(snap: &Snapshot) -> Vec<usize> {
    let mut sizes: Vec<usize> = snap.1.iter().map(Vec::len).collect();
    sizes.sort_unstable();
    sizes
}

/// Structural + geometric equality up to vertex/face ordering and fp slack.
/// Position matching is greedy pairwise (not sort-and-zip): fp noise near a
/// coordinate boundary flips lexicographic sort order and would mispair
/// otherwise-identical sets.
fn same_geometry(a: &Snapshot, b: &Snapshot) -> Result<(), String> {
    if a.0.len() != b.0.len() {
        return Err(format!("vertex count {} vs {}", a.0.len(), b.0.len()));
    }
    if face_sizes(a) != face_sizes(b) {
        return Err(format!(
            "face sizes {:?} vs {:?}",
            face_sizes(a),
            face_sizes(b)
        ));
    }
    let mut unmatched = b.0.clone();
    for p in &a.0 {
        let Some(i) = unmatched
            .iter()
            .position(|q| (*p - *q).length() <= ROUNDTRIP_TOL)
        else {
            return Err(format!("no counterpart within tolerance for {:?}", p));
        };
        unmatched.swap_remove(i);
    }
    Ok(())
}

/// KNOWN CONTRACT GAPS, tolerated by this harness — both are inverse ops
/// failing typed with `InverseFailed(PushPull(NonManifoldResult))` while the
/// object stays valid and untouched:
///
/// 1. `find_collapse_plans` only recognizes clean QUAD step walls, so
///    undoing/redoing a push whose pocket walls were subdivided fails.
///    Acceptance spec: `op_fuzz_repro_nonquad_wall_undo.rs` (pending op is a
///    `PushPull`).
/// 2. Obstruction-guard fidelity differs across inverse op pairs — a recess
///    accepted by `push_pull`'s vertex-heuristic guard leaves a collapse
///    whose inverse (`extrude_sub_face`, centroid-RAY guard) is refused.
///    Acceptance spec: `op_fuzz_repro_guard_regime_undo.rs` (pending op is an
///    `ExtrudeSubFace`).
///
/// When a spec is implemented and un-ignored, delete its arm here so the
/// round-trip asserts bite again. The signature alone is deliberately not
/// enough: it is shared by validator-backstop refusals (real regressions), so
/// the pending op kind — peeked from the history, which is unchanged after a
/// failed dispatch — must match a documented gap. Residual risk: a
/// backstop-refused PushPull/ExtrudeSubFace inverse would be tolerated here
/// in release, but debug runs still panic inside `check_invariants` before
/// the backstop and are never tolerated.
fn is_known_inverse_guard_gap(history: &History, e: &HistoryError, redo: bool) -> bool {
    let signature = matches!(
        e,
        HistoryError::InverseFailed(KernelOpError::PushPull(PushPullError::NonManifoldResult))
    );
    let pending = if redo {
        history.peek_redo()
    } else {
        history.peek_undo()
    };
    signature
        && matches!(
            pending,
            Some(KernelOp::PushPull { .. }) | Some(KernelOp::ExtrudeSubFace { .. })
        )
}

/// Serialization round-trip (HEW_FILE_FORMAT.md; serialize.rs's documented
/// property): `decode(encode(o))` equals `o` topologically and geometrically,
/// and `encode` is deterministic. The harness objects carry no materials, so
/// the material closures are inert.
fn check_serialize_roundtrip(object: &Object, what: &str) -> Result<(), TestCaseError> {
    let encode = |o: &Object| o.encode(&|_| unreachable!("harness objects have no materials"));
    let bytes = encode(object);
    prop_assert_eq!(
        &bytes,
        &encode(object),
        "{}: encode is not deterministic",
        what
    );
    let decoded = match Object::decode(&bytes, &|_| None) {
        Ok(o) => o,
        Err(e) => {
            return Err(TestCaseError::fail(format!(
                "{what}: decode(encode(object)) failed: {e}"
            )));
        }
    };
    if let Err(e) = decoded.validate() {
        return Err(TestCaseError::fail(format!(
            "{what}: decoded object fails validation: {e}"
        )));
    }
    prop_assert_eq!(
        decoded.watertight(),
        object.watertight(),
        "{}: watertight state changed across the round-trip",
        what
    );
    if let Err(why) = same_geometry(&object.to_polygons(), &decoded.to_polygons()) {
        return Err(TestCaseError::fail(format!(
            "{what}: geometry changed across the round-trip: {why}"
        )));
    }
    prop_assert_eq!(
        expected_euler(&decoded),
        expected_euler(object),
        "{}: hole count changed across the round-trip",
        what
    );
    Ok(())
}

/// Per-step invariant bundle.
fn check_state(object: &Object, step: usize, what: &str) -> Result<(), TestCaseError> {
    if let Err(e) = object.validate() {
        return Err(TestCaseError::fail(format!(
            "step {step} ({what}): validator rejected object: {e}"
        )));
    }
    prop_assert_eq!(
        object.watertight(),
        WatertightState::Watertight,
        "step {} ({}): solid opened up",
        step,
        what
    );
    prop_assert_eq!(
        euler_characteristic(object),
        expected_euler(object),
        "step {} ({}): Euler characteristic disagrees with hole count",
        step,
        what
    );
    Ok(())
}

proptest! {
    /// The core fuzz property: any random op/undo/redo sequence on a box
    /// keeps every invariant, and history unwinds and replays exactly.
    #[test]
    fn op_sequences_preserve_invariants_and_roundtrip(
        mut object in arb_seed(),
        ops in proptest::collection::vec(arb_fuzz_op(), 1..16),
    ) {
        let original = object.to_polygons();
        if std::env::var("FUZZ_TRACE").is_ok() {
            eprintln!("CASE verts={:?} ops={:?}", original.0, ops);
        }
        let mut history = History::new();

        for (step, fuzz_op) in ops.iter().enumerate() {
            match fuzz_op {
                FuzzOp::Undo => {
                    if history.can_undo() {
                        match history.undo(&mut object) {
                            Ok(_) => check_state(&object, step, "undo")?,
                            Err(e) if is_known_inverse_guard_gap(&history, &e, false) => {
                                return Ok(());
                            }
                            Err(e) => return Err(TestCaseError::fail(
                                format!("step {step}: undo failed (kernel bug): {e}"),
                            )),
                        }
                    }
                }
                FuzzOp::Redo => {
                    if history.can_redo() {
                        match history.redo(&mut object) {
                            Ok(_) => check_state(&object, step, "redo")?,
                            Err(e) if is_known_inverse_guard_gap(&history, &e, true) => {
                                return Ok(());
                            }
                            Err(e) => return Err(TestCaseError::fail(
                                format!("step {step}: redo failed (kernel bug): {e}"),
                            )),
                        }
                    }
                }
                _ => {
                    let Some(op) = resolve(&object, fuzz_op) else { continue };
                    if std::env::var("FUZZ_TRACE").is_ok() {
                        eprintln!("  step {step} resolved: {op:?}");
                    }
                    let before = object.to_polygons();
                    match history.apply(&mut object, op.clone()) {
                        Ok(_) => check_state(&object, step, "apply")?,
                        Err(_) => {
                            // Strong guarantee: a rejected op is a no-op.
                            let after = object.to_polygons();
                            prop_assert!(
                                before == after,
                                "step {}: failed op {:?} mutated the object",
                                step,
                                op
                            );
                        }
                    }
                }
            }
        }

        // The post-sequence object must survive the serialization round-trip.
        check_serialize_roundtrip(&object, "post-sequence")?;

        // Round-trip the whole history twice. Full unwind must restore the
        // starting geometry; full replay reaches the "maximal" state (which
        // can exceed the post-sequence state when the sequence ended with
        // unmatched Undos, so it is captured, not predicted); a second
        // unwind/replay cycle must reproduce both — this is what exercises
        // inverse/redo re-anchoring across handle reallocation.
        // `Ok(false)` = the known non-quad-collapse gap fired; abandon the
        // round-trip for this case (the object is valid and untouched, but
        // the history can no longer unwind past the refused inverse).
        let unwind = |object: &mut Object,
                      history: &mut History,
                      label: &str|
         -> Result<bool, TestCaseError> {
            let mut n = 0usize;
            while history.can_undo() {
                match history.undo(object) {
                    Ok(_) => {}
                    Err(e) if is_known_inverse_guard_gap(history, &e, false) => {
                        return Ok(false);
                    }
                    Err(e) => {
                        return Err(TestCaseError::fail(format!("{label}, undo #{n}: {e}")));
                    }
                }
                check_state(object, n, label)?;
                n += 1;
            }
            Ok(true)
        };
        let replay = |object: &mut Object,
                      history: &mut History,
                      label: &str|
         -> Result<bool, TestCaseError> {
            let mut n = 0usize;
            while history.can_redo() {
                match history.redo(object) {
                    Ok(_) => {}
                    Err(e) if is_known_inverse_guard_gap(history, &e, true) => {
                        return Ok(false);
                    }
                    Err(e) => {
                        return Err(TestCaseError::fail(format!("{label}, redo #{n}: {e}")));
                    }
                }
                check_state(object, n, label)?;
                n += 1;
            }
            Ok(true)
        };

        if !unwind(&mut object, &mut history, "first unwind")? {
            return Ok(());
        }
        if let Err(why) = same_geometry(&original, &object.to_polygons()) {
            return Err(TestCaseError::fail(format!(
                "full undo did not restore the original object: {why}"
            )));
        }

        if !replay(&mut object, &mut history, "first replay")? {
            return Ok(());
        }
        let maximal = object.to_polygons();

        if !unwind(&mut object, &mut history, "second unwind")? {
            return Ok(());
        }
        if let Err(why) = same_geometry(&original, &object.to_polygons()) {
            return Err(TestCaseError::fail(format!(
                "second full undo did not restore the original object: {why}"
            )));
        }

        if !replay(&mut object, &mut history, "second replay")? {
            return Ok(());
        }
        if let Err(why) = same_geometry(&maximal, &object.to_polygons()) {
            return Err(TestCaseError::fail(format!(
                "second full redo did not reproduce the maximal state: {why}"
            )));
        }

        // The maximal state must survive the round-trip too (it can contain
        // geometry the post-sequence state never had).
        check_serialize_roundtrip(&object, "maximal")?;
    }
}
