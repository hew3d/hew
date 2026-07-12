//! Document-level fuzz harness (DEVELOPMENT.md rule 3): random document ops —
//! object edits, booleans, transforms, duplicate/delete, group/ungroup,
//! components and instances, interleaved undo/redo — over a document seeded
//! with sketch-extruded boxes, with torture mode on (the always-on validator
//! panics at the offending op instead of committing a violation).
//!
//! Invariants:
//! - after every op (applied or refused typed) every visible object validates
//!   and stays watertight;
//! - every undo/redo dispatch succeeds (DEVELOPMENT.md rule 9 — history
//!   replay is guard-exempt with proof; no failure signature is tolerated);
//! - `save()` is deterministic, `load(save())` reproduces the same
//!   `state_hash`, at the post-sequence and maximal states;
//! - fully unwinding the document log and replaying it twice reproduces the
//!   same states up to tolerance-aware equivalence (canonical fingerprints).
//!   The canonical geometry writer makes save bytes independent of slot
//!   allocation (see `doc_replay_diverge_repro.rs`), but cycles are compared
//!   by fingerprint, not bytes, because baked translations round-trip with
//!   ulp noise (`fl(fl(x + d) - d) != x` — the DEVELOPMENT.md fp trap), and
//!   that noise is exactly what tolerance-aware equivalence exists for.

use kernel::{BooleanOp, Document, KernelOp, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use proptest::prelude::*;

/// Abstract document op; selectors resolve against live ids at apply time.
#[derive(Debug, Clone)]
enum DocOp {
    /// Push/pull the `face_sel`-th face of the `obj_sel`-th visible object.
    PushPull {
        obj_sel: usize,
        face_sel: usize,
        distance: f64,
    },
    /// Straight cut across a face of a visible object.
    SplitFace {
        obj_sel: usize,
        face_sel: usize,
        edge_a: usize,
        edge_b: usize,
        ta: f64,
        tb: f64,
    },
    /// Imprint a loop into a face of a visible object: a shrunk boundary
    /// copy, or (`staple: true`, quad faces) a concave U-shaped loop whose
    /// vertex average lies outside it — the hole-reassignment trap shape.
    SplitFaceInner {
        obj_sel: usize,
        face_sel: usize,
        shrink: f64,
        staple: bool,
    },
    /// Boolean between two distinct visible objects.
    Boolean {
        kind: u8,
        a_sel: usize,
        b_sel: usize,
    },
    /// Translate a visible object.
    Translate {
        obj_sel: usize,
        offset: (f64, f64, f64),
    },
    /// Duplicate a visible object with a translation placement.
    Duplicate {
        obj_sel: usize,
        offset: (f64, f64, f64),
    },
    /// Delete the `node_sel`-th top-level node.
    Delete {
        node_sel: usize,
    },
    /// Group the first `count` top-level nodes.
    Group {
        count: usize,
    },
    /// Ungroup the `group_sel`-th group.
    Ungroup {
        group_sel: usize,
    },
    /// Make a component from the `node_sel`-th top-level node.
    MakeComponent {
        node_sel: usize,
    },
    /// Place another instance of the `comp_sel`-th component, translated.
    PlaceInstance {
        comp_sel: usize,
        offset: (f64, f64, f64),
    },
    /// Explode the `inst_sel`-th instance back into loose objects.
    ExplodeInstance {
        inst_sel: usize,
    },
    Undo,
    Redo,
}

fn arb_offset() -> impl Strategy<Value = (f64, f64, f64)> {
    (-8.0..8.0f64, -8.0..8.0f64, -4.0..4.0f64)
}

fn arb_doc_op() -> impl Strategy<Value = DocOp> {
    let distance = || (-4.0..4.0f64).prop_filter("degenerate distance", |d| d.abs() >= 0.05);
    prop_oneof![
        3 => (any::<usize>(), any::<usize>(), distance()).prop_map(|(obj_sel, face_sel, distance)| {
            DocOp::PushPull { obj_sel, face_sel, distance }
        }),
        2 => (any::<usize>(), any::<usize>(), any::<usize>(), any::<usize>(), 0.25..0.75f64, 0.25..0.75f64)
            .prop_map(|(obj_sel, face_sel, edge_a, edge_b, ta, tb)| DocOp::SplitFace {
                obj_sel, face_sel, edge_a, edge_b, ta, tb,
            }),
        2 => (any::<usize>(), any::<usize>(), 0.3..0.7f64, proptest::bool::ANY).prop_map(
            |(obj_sel, face_sel, shrink, staple)| {
                DocOp::SplitFaceInner { obj_sel, face_sel, shrink, staple }
            }
        ),
        3 => (0u8..3, any::<usize>(), any::<usize>()).prop_map(|(kind, a_sel, b_sel)| {
            DocOp::Boolean { kind, a_sel, b_sel }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Translate { obj_sel, offset }
        }),
        2 => (any::<usize>(), arb_offset()).prop_map(|(obj_sel, offset)| {
            DocOp::Duplicate { obj_sel, offset }
        }),
        1 => any::<usize>().prop_map(|node_sel| DocOp::Delete { node_sel }),
        1 => (2usize..4).prop_map(|count| DocOp::Group { count }),
        1 => any::<usize>().prop_map(|group_sel| DocOp::Ungroup { group_sel }),
        1 => any::<usize>().prop_map(|node_sel| DocOp::MakeComponent { node_sel }),
        1 => (any::<usize>(), arb_offset()).prop_map(|(comp_sel, offset)| {
            DocOp::PlaceInstance { comp_sel, offset }
        }),
        1 => any::<usize>().prop_map(|inst_sel| DocOp::ExplodeInstance { inst_sel }),
        2 => Just(DocOp::Undo),
        1 => Just(DocOp::Redo),
    ]
}

/// Seeds one box by sketching a rectangle on the ground plane and extruding.
fn add_box(doc: &mut Document, x: f64, y: f64, dx: f64, dy: f64, h: f64) -> ObjectId {
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("unit normal");
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).expect("gesture opens");
    {
        let sk = doc.sketch_mut(s).expect("sketch exists");
        let p = [
            Point3::new(x, y, 0.0),
            Point3::new(x + dx, y, 0.0),
            Point3::new(x + dx, y + dy, 0.0),
            Point3::new(x, y + dy, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(p[k], p[(k + 1) % 4]).expect("segment adds");
        }
    }
    doc.end_sketch_gesture(s).expect("gesture closes");
    let region = doc
        .sketch(s)
        .expect("sketch exists")
        .regions()
        .keys()
        .next()
        .expect("rectangle closes one region");
    let (oid, _) = doc.extrude_region(s, region, h).expect("box extrudes");
    oid
}

/// Every visible object validates and is watertight.
fn check_doc(doc: &Document, step: usize, what: &str) -> Result<(), TestCaseError> {
    for oid in doc.visible_object_ids() {
        let obj = doc.object(oid).expect("visible id resolves");
        if let Err(e) = obj.validate() {
            return Err(TestCaseError::fail(format!(
                "step {step} ({what}): object {oid:?} invalid: {e}"
            )));
        }
        prop_assert_eq!(
            obj.watertight(),
            kernel::WatertightState::Watertight,
            "step {} ({}): object {:?} opened up",
            step,
            what,
            oid
        );
    }
    Ok(())
}

/// `save` determinism and `load(save())` state-hash fidelity.
fn check_persistence(doc: &Document, what: &str) -> Result<(), TestCaseError> {
    let bytes = doc.save();
    prop_assert_eq!(&bytes, &doc.save(), "{}: save is not deterministic", what);
    let reloaded = match Document::load(&bytes) {
        Ok(d) => d,
        Err(e) => {
            return Err(TestCaseError::fail(format!(
                "{what}: load(save(doc)) failed: {e}"
            )));
        }
    };
    prop_assert_eq!(
        reloaded.state_hash(),
        doc.state_hash(),
        "{}: state hash changed across save/load",
        what
    );
    Ok(())
}

/// Canonical, slot-order-independent document fingerprint: per visible
/// object, its sorted vertex positions (rounded well below any op tolerance)
/// and sorted face sizes; plus node/instance/component counts. Two documents
/// with equal fingerprints are the same model up to handle renaming.
fn doc_fingerprint(doc: &Document) -> String {
    let q = |c: f64| (c * 1e7).round() as i64;
    let object_print = |oid| {
        let obj = doc.object(oid).expect("live id resolves");
        let (pts, faces) = obj.to_polygons();
        let mut vs: Vec<(i64, i64, i64)> = pts.iter().map(|p| (q(p.x), q(p.y), q(p.z))).collect();
        vs.sort_unstable();
        let mut sizes: Vec<usize> = faces.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        format!("{vs:?}|{sizes:?}")
    };
    let mut objects: Vec<String> = doc
        .visible_object_ids()
        .into_iter()
        .map(object_print)
        .collect();
    objects.sort_unstable();
    // Definition members are not world objects; fingerprint them per
    // component, and instances by their (quantized) poses, or a replay that
    // corrupts a definition or restores a wrong pose would compare equal.
    let mut defs: Vec<String> = doc
        .component_ids()
        .into_iter()
        .map(|cid| {
            let mut members: Vec<String> = doc
                .def_members(cid)
                .expect("live component resolves")
                .into_iter()
                .map(object_print)
                .collect();
            members.sort_unstable();
            format!("{members:?}")
        })
        .collect();
    defs.sort_unstable();
    let mut poses: Vec<Vec<i64>> = doc
        .instance_ids()
        .into_iter()
        .map(|iid| {
            doc.instance_pose(iid)
                .expect("live instance resolves")
                .to_affine()
                .iter()
                .map(|&c| q(c))
                .collect()
        })
        .collect();
    poses.sort_unstable();
    format!(
        "objs={objects:?} defs={defs:?} poses={poses:?} groups={}",
        doc.group_ids().len()
    )
}

fn nth<T: Copy>(items: &[T], sel: usize) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[sel % items.len()])
    }
}

/// Applies one abstract op.
fn apply_doc_op(doc: &mut Document, step: usize, op: &DocOp) -> Result<(), TestCaseError> {
    match op {
        DocOp::PushPull {
            obj_sel,
            face_sel,
            distance,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(());
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(());
            };
            let _ = doc.apply_object_op(
                oid,
                KernelOp::PushPull {
                    face,
                    distance: *distance,
                },
            );
        }
        DocOp::SplitFace {
            obj_sel,
            face_sel,
            edge_a,
            edge_b,
            ta,
            tb,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(());
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(());
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            let sides = boundary.len();
            if sides < 3 {
                return Ok(());
            }
            let (a, b) = (edge_a % sides, edge_b % sides);
            if a == b {
                return Ok(());
            }
            let point_on = |i: usize, t: f64| {
                let p = boundary[i];
                let q = boundary[(i + 1) % sides];
                p + (q - p) * t
            };
            let path = vec![point_on(a, *ta), point_on(b, *tb)];
            let _ = doc.apply_object_op(
                oid,
                KernelOp::SplitFace {
                    face,
                    path,
                    restore: None,
                },
            );
        }
        DocOp::SplitFaceInner {
            obj_sel,
            face_sel,
            shrink,
            staple,
        } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(());
            };
            let obj = doc.object(oid).expect("visible id resolves");
            let Some(face) = obj.faces().keys().nth(face_sel % obj.faces().len()) else {
                return Ok(());
            };
            let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
            if boundary.len() < 3 {
                return Ok(());
            }
            let loop_path: Vec<Point3> = if *staple {
                // Concave staple in the quad's bilinear frame (quad faces
                // only; skewed quads may still reject typed — fine).
                if boundary.len() != 4 {
                    return Ok(());
                }
                let (o, ua, vb) = (boundary[0], boundary[1], boundary[3]);
                let at = |a: f64, b: f64| o + (ua - o) * a + (vb - o) * b;
                vec![
                    at(0.2, 0.6),
                    at(0.4, 0.6),
                    at(0.4, 0.8),
                    at(0.6, 0.8),
                    at(0.6, 0.6),
                    at(0.8, 0.6),
                    at(0.8, 0.9),
                    at(0.2, 0.9),
                ]
            } else {
                let inv = 1.0 / boundary.len() as f64;
                let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
                    Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
                });
                boundary.iter().map(|&p| c + (p - c) * *shrink).collect()
            };
            let _ = doc.apply_object_op(
                oid,
                KernelOp::SplitFaceInner {
                    face,
                    loop_path,
                    restore: None,
                    curve: None,
                },
            );
        }
        DocOp::Boolean { kind, a_sel, b_sel } => {
            let ids = doc.visible_object_ids();
            let (Some(a), Some(b)) = (nth(&ids, *a_sel), nth(&ids, *b_sel)) else {
                return Ok(());
            };
            let op = match kind {
                0 => BooleanOp::Union,
                1 => BooleanOp::Subtract,
                _ => BooleanOp::Intersect,
            };
            let _ = doc.boolean(op, a, b);
        }
        DocOp::Translate { obj_sel, offset } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(());
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.transform_object(oid, &t);
        }
        DocOp::Duplicate { obj_sel, offset } => {
            let Some(oid) = nth(&doc.visible_object_ids(), *obj_sel) else {
                return Ok(());
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.duplicate_node(NodeId::Object(oid), &t);
        }
        DocOp::Delete { node_sel } => {
            let Some(node) = nth(&doc.top_level_nodes(), *node_sel) else {
                return Ok(());
            };
            let _ = doc.delete_node(node);
        }
        DocOp::Group { count } => {
            let nodes = doc.top_level_nodes();
            if nodes.len() < 2 {
                return Ok(());
            }
            let members: Vec<NodeId> = nodes.into_iter().take(*count).collect();
            let _ = doc.group_nodes(&members);
        }
        DocOp::Ungroup { group_sel } => {
            let Some(gid) = nth(&doc.group_ids(), *group_sel) else {
                return Ok(());
            };
            let _ = doc.ungroup(gid);
        }
        DocOp::MakeComponent { node_sel } => {
            let Some(node) = nth(&doc.top_level_nodes(), *node_sel) else {
                return Ok(());
            };
            let _ = doc.make_component(&[node]);
        }
        DocOp::PlaceInstance { comp_sel, offset } => {
            let Some(cid) = nth(&doc.component_ids(), *comp_sel) else {
                return Ok(());
            };
            let t = Transform::translation(Vec3::new(offset.0, offset.1, offset.2));
            let _ = doc.place_instance(cid, t);
        }
        DocOp::ExplodeInstance { inst_sel } => {
            let Some(iid) = nth(&doc.instance_ids(), *inst_sel) else {
                return Ok(());
            };
            let _ = doc.explode_instance(iid);
        }
        DocOp::Undo => {
            if doc.can_undo()
                && let Err(e) = doc.undo()
            {
                return Err(TestCaseError::fail(format!(
                    "step {step}: document undo failed: {e}"
                )));
            }
        }
        DocOp::Redo => {
            if doc.can_redo()
                && let Err(e) = doc.redo()
            {
                return Err(TestCaseError::fail(format!(
                    "step {step}: document redo failed: {e}"
                )));
            }
        }
    }
    Ok(())
}

proptest! {
    /// Random document-op sequences keep every visible object valid, survive
    /// save/load, and the document log unwinds and replays reproducibly.
    #[test]
    fn document_sequences_preserve_invariants_and_roundtrip(
        seeds in proptest::collection::vec(
            ((-6.0..6.0f64, -6.0..6.0f64), (1.0..6.0f64, 1.0..6.0f64), 0.5..5.0f64),
            2..4,
        ),
        ops in proptest::collection::vec(arb_doc_op(), 1..14),
    ) {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        for ((x, y), (dx, dy), h) in seeds {
            add_box(&mut doc, x, y, dx, dy, h);
        }
        check_doc(&doc, 0, "seed")?;

        if std::env::var("FUZZ_TRACE").is_ok() {
            eprintln!("DOC CASE ops={ops:?}");
        }

        for (step, op) in ops.iter().enumerate() {
            apply_doc_op(&mut doc, step, op)?;
            check_doc(&doc, step, "apply")?;
        }

        check_persistence(&doc, "post-sequence")?;

        // Unwind the whole document log, replay it, and do both again; both
        // ends must reproduce the same canonical fingerprint, and every
        // undo/redo must succeed (rule 9 — no failure signature is
        // tolerated). Fingerprints, not save bytes: baked-transform and
        // sweep round-trips carry ulp noise, which the fingerprint's
        // quantization absorbs and byte comparison would not.
        let unwind = |doc: &mut Document, label: &str| -> Result<(), TestCaseError> {
            let mut n = 0usize;
            while doc.can_undo() {
                if let Err(e) = doc.undo() {
                    return Err(TestCaseError::fail(format!("{label}, undo #{n}: {e}")));
                }
                check_doc(doc, n, label)?;
                n += 1;
            }
            Ok(())
        };
        let replay = |doc: &mut Document, label: &str| -> Result<(), TestCaseError> {
            let mut n = 0usize;
            while doc.can_redo() {
                if let Err(e) = doc.redo() {
                    return Err(TestCaseError::fail(format!("{label}, redo #{n}: {e}")));
                }
                check_doc(doc, n, label)?;
                n += 1;
            }
            Ok(())
        };

        unwind(&mut doc, "first unwind")?;
        let empty_print = doc_fingerprint(&doc);

        replay(&mut doc, "first replay")?;
        let maximal_print = doc_fingerprint(&doc);
        check_persistence(&doc, "maximal")?;

        unwind(&mut doc, "second unwind")?;
        prop_assert_eq!(
            doc_fingerprint(&doc),
            empty_print,
            "second full undo did not reproduce the fully-unwound state"
        );

        replay(&mut doc, "second replay")?;
        prop_assert_eq!(
            doc_fingerprint(&doc),
            maximal_print,
            "second full redo did not reproduce the maximal state"
        );
    }
}
