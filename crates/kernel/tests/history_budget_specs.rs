//! Executable specs for undo on dense meshes: copy-on-write snapshots
//! (`kernel::shared`) and the undo-memory budget
//! (`Document::enforce_history_budget`, `Document::history_memory`).
//!
//! The contract: a step costs the tables it wrote, not the document; an
//! ordinary model is never trimmed; over budget the oldest undo steps go
//! (never below `min_steps`, never inside a transaction bracket, never an
//! open session's opening entry); and what stays undoable undoes and
//! redoes bit-for-bit as before.

use kernel::repair::{MendPlan, default_weld_distance};
use kernel::trig;
use kernel::{
    CompoundMeta, DEFAULT_HISTORY_BUDGET_BYTES, DEFAULT_HISTORY_MIN_STEPS, Document, DocumentError,
    HistoryBudget, HistoryOrigin, ImportNode, ImportScene, KernelOp, MeshRecipe, NodeId, ObjectId,
    Point3, Transform, Vec3,
};

// ───────────────────────────────────────────────────────────── helpers

/// A UV sphere of radius 1 m, `n` bands by `2n` sectors, triangulated, with
/// one triangle left out (an open scan Mend can close).
fn scan(n: usize) -> MeshRecipe {
    let mut positions = vec![Point3::new(0.0, 0.0, 1.0)];
    for i in 1..n {
        let th = std::f64::consts::PI * i as f64 / n as f64;
        for j in 0..2 * n {
            let ph = std::f64::consts::PI * j as f64 / n as f64;
            positions.push(Point3::new(
                trig::sin(th) * trig::cos(ph),
                trig::sin(th) * trig::sin(ph),
                trig::cos(th),
            ));
        }
    }
    positions.push(Point3::new(0.0, 0.0, -1.0));
    let south = positions.len() - 1;
    let ring = |i: usize, j: usize| 1 + (i - 1) * 2 * n + j % (2 * n);
    let mut faces = Vec::new();
    for j in 0..2 * n {
        faces.push(vec![0, ring(1, j), ring(1, j + 1)]);
        faces.push(vec![south, ring(n - 1, j + 1), ring(n - 1, j)]);
    }
    for i in 1..n - 1 {
        for j in 0..2 * n {
            faces.push(vec![ring(i, j), ring(i + 1, j), ring(i + 1, j + 1)]);
            faces.push(vec![ring(i, j), ring(i + 1, j + 1), ring(i, j + 1)]);
        }
    }
    faces.remove(faces.len() / 2);
    recipe("Scan", positions, faces)
}

/// A closed 1 m cube.
fn cube() -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x + 5.0, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    recipe("Cube", positions, faces)
}

fn recipe(name: &str, positions: Vec<Point3>, faces: Vec<Vec<usize>>) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

/// A document holding `meshes`, with the import's own step dropped so the
/// stacks start empty. Returns the objects in recipe order.
fn doc_with(meshes: Vec<MeshRecipe>) -> (Document, Vec<ObjectId>) {
    let mut doc = Document::new();
    let (_, change) = doc
        .ingest(
            ImportScene {
                materials: Vec::new(),
                defs: Vec::new(),
                roots: meshes.into_iter().map(ImportNode::Mesh).collect(),
                guides: Vec::new(),
                tags: Vec::new(),
                camera: None,
                scenes: Vec::new(),
                section: None,
                view_style: None,
                geo: None,
                extra: Default::default(),
            },
            Vec::new(),
        )
        .expect("the fixture imports");
    let ids = change.objects_touched.clone();
    (doc, ids)
}

fn nudge(doc: &mut Document, object: ObjectId) {
    doc.transform_object(object, &Transform::translation(Vec3::new(0.01, 0.002, 0.0)))
        .expect("a move commits");
}

fn mend(doc: &mut Document, object: ObjectId) {
    let weld = default_weld_distance(doc.object(object).unwrap());
    doc.mend(object, &MendPlan::all(weld))
        .expect("the scan mends");
}

fn top_face(doc: &Document, object: ObjectId) -> kernel::FaceId {
    doc.object(object)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .expect("the cube has a top")
}

fn meta() -> CompoundMeta {
    CompoundMeta {
        label: "Script".into(),
        origin: HistoryOrigin::User,
        coalesce: None,
    }
}

/// Undo every step left, checking each state against the hash recorded when
/// it was first reached (`hashes[d]` = the state at undo depth `d`), then
/// redo every step, checking again.
fn undo_redo_exact(doc: &mut Document, hashes: &[u64]) {
    let depth = doc.undo_depth();
    let base = hashes.len() - 1 - depth;
    assert_eq!(doc.state_hash(), hashes[base + depth]);
    for d in (0..depth).rev() {
        doc.undo().expect("a kept step undoes");
        assert_eq!(doc.state_hash(), hashes[base + d], "undo to depth {d}");
    }
    assert!(matches!(doc.undo(), Err(DocumentError::NothingToUndo)));
    for d in 1..=depth {
        doc.redo().expect("an undone step redoes");
        assert_eq!(doc.state_hash(), hashes[base + d], "redo to depth {d}");
    }
}

// ───────────────────────────────────────────────────────────── specs

/// The default budget: 1 GiB, at least three steps.
#[test]
fn the_default_budget_is_a_gib_and_three_steps() {
    let doc = Document::new();
    assert_eq!(
        doc.history_budget(),
        HistoryBudget {
            max_bytes: DEFAULT_HISTORY_BUDGET_BYTES,
            min_steps: DEFAULT_HISTORY_MIN_STEPS,
        }
    );
    assert_eq!(DEFAULT_HISTORY_BUDGET_BYTES, 1 << 30);
    let m = doc.history_memory();
    assert_eq!((m.undo_steps, m.redo_steps, m.trimmed_total), (0, 0, 0));
}

/// `min_steps` is at least one: the step just committed stays undoable.
#[test]
fn min_steps_is_at_least_one() {
    let mut doc = Document::new();
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 0,
    });
    assert_eq!(doc.history_budget().min_steps, 1);
}

/// A Move writes positions and planes; the snapshot it records shares the
/// rest of the mesh with the live object, so it holds well under what a
/// Mend's snapshot (which rewrites every table) holds.
#[test]
fn a_move_holds_only_the_tables_it_wrote() {
    let (mut doc, ids) = doc_with(vec![scan(40)]);
    doc.enforce_history_budget();
    let empty = doc.history_memory().bytes;
    nudge(&mut doc, ids[0]);
    let after_move = doc.history_memory().bytes - empty;
    let (mut doc2, ids2) = doc_with(vec![scan(40)]);
    let empty2 = doc2.history_memory().bytes;
    mend(&mut doc2, ids2[0]);
    let after_mend = doc2.history_memory().bytes - empty2;
    assert!(
        after_move * 10 < after_mend * 6,
        "a Move holds {after_move} bytes, a Mend {after_mend}"
    );
}

/// A paint step holds the fingerprint of the face it painted (the face's
/// outer ring), and the tally counts it.
#[test]
fn a_paint_step_counts_its_face_fingerprint() {
    fn prism(n: usize) -> MeshRecipe {
        let ring = |z: f64| {
            (0..n).map(move |i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                Point3::new(trig::cos(a), trig::sin(a), z)
            })
        };
        let positions: Vec<Point3> = ring(0.0).chain(ring(1.0)).collect();
        let mut faces = vec![(0..n).rev().collect::<Vec<_>>(), (n..2 * n).collect()];
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(vec![i, j, n + j, n + i]);
        }
        recipe("Prism", positions, faces)
    }
    let paint_bytes = |n: usize| {
        let (mut doc, ids) = doc_with(vec![prism(n)]);
        let m = doc.add_material(kernel::Material::solid(
            "Red",
            kernel::Rgba8::rgb(220, 30, 30),
        ));
        let top = top_face(&doc, ids[0]);
        let before = doc.history_memory().bytes;
        doc.paint_face(ids[0], top, Some(m)).unwrap();
        doc.history_memory().bytes - before
    };
    let (small, large) = (paint_bytes(4), paint_bytes(64));
    assert!(
        large >= small + 60 * std::mem::size_of::<Point3>(),
        "painting a 64-gon holds {large} bytes, a quad {small}"
    );
}

/// Undo restores the recorded snapshot by sharing it: the redo step then
/// holds next to nothing the live document does not.
#[test]
fn undo_shares_the_restored_snapshot() {
    let (mut doc, ids) = doc_with(vec![scan(40)]);
    let base = doc.history_memory().bytes;
    nudge(&mut doc, ids[0]);
    let after_move = doc.history_memory().bytes - base;
    doc.undo().unwrap();
    let redo_held = doc.history_memory().bytes - base;
    assert!(
        redo_held * 20 < after_move,
        "the redo step holds {redo_held} bytes after a {after_move}-byte Move"
    );
}

/// An ordinary model never reaches the budget: fifty steps, no trim.
#[test]
fn an_ordinary_model_is_never_trimmed() {
    let (mut doc, ids) = doc_with(vec![cube()]);
    let start = doc.undo_depth();
    for _ in 0..50 {
        nudge(&mut doc, ids[0]);
        assert_eq!(doc.enforce_history_budget(), None);
    }
    let m = doc.history_memory();
    assert_eq!(m.undo_steps, start + 50);
    assert_eq!(m.trimmed_total, 0);
    assert!(m.bytes < 1 << 20, "fifty cube moves hold {} bytes", m.bytes);
}

/// The stress spec: many steps on a dense mesh stay under budget (or at
/// `min_steps`), each trim drops the oldest steps and reports what is
/// kept, and every kept step undoes and redoes bit-for-bit.
#[test]
fn many_steps_on_a_dense_mesh_stay_under_budget_and_undo_exactly() {
    let (mut doc, ids) = doc_with(vec![scan(40), cube()]);
    let base = doc.history_memory().bytes;
    nudge(&mut doc, ids[0]);
    let one_move = doc.history_memory().bytes - base;
    // Room for about four Moves.
    let budget = HistoryBudget {
        max_bytes: base + one_move * 4 + one_move / 2,
        min_steps: 2,
    };
    doc.set_history_budget(budget);
    let mut hashes = vec![0; doc.undo_depth()];
    hashes.push(doc.state_hash());
    let mut trims = 0;
    for step in 0..14 {
        match step % 7 {
            3 => {
                let top = top_face(&doc, ids[1]);
                doc.apply_object_op(
                    ids[1],
                    KernelOp::PushPull {
                        face: top,
                        distance: 0.1,
                    },
                )
                .unwrap();
            }
            5 if step == 5 => mend(&mut doc, ids[0]),
            _ => nudge(&mut doc, ids[0]),
        }
        let before = doc.undo_depth();
        if let Some(trim) = doc.enforce_history_budget() {
            trims += 1;
            assert_eq!(trim.kept, doc.undo_depth());
            assert_eq!(trim.dropped + trim.kept, before);
            assert!(trim.kept >= budget.min_steps);
            // The dropped steps' states are no longer reachable.
            hashes.drain(..trim.dropped);
        }
        let m = doc.history_memory();
        assert!(
            m.bytes <= budget.max_bytes || m.undo_steps == budget.min_steps,
            "step {step}: {} bytes over a {}-byte budget with {} steps",
            m.bytes,
            budget.max_bytes,
            m.undo_steps
        );
        hashes.push(doc.state_hash());
        assert_eq!(hashes.len(), doc.undo_depth() + 1);
    }
    assert!(trims > 0, "the budget never trimmed");
    assert!(doc.history_memory().trimmed_total > 0);
    undo_redo_exact(&mut doc, &hashes);
}

/// Over budget, `min_steps` still stay — however large they are.
#[test]
fn min_steps_stay_whatever_they_hold() {
    let (mut doc, ids) = doc_with(vec![scan(20)]);
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 2,
    });
    let mut hashes = vec![0; doc.undo_depth()];
    hashes.push(doc.state_hash());
    for _ in 0..5 {
        nudge(&mut doc, ids[0]);
        if let Some(trim) = doc.enforce_history_budget() {
            hashes.drain(..trim.dropped);
        }
        hashes.push(doc.state_hash());
    }
    assert_eq!(doc.undo_depth(), 2);
    undo_redo_exact(&mut doc, &hashes);
}

/// A trim leaves the redo stack and the history generation alone.
#[test]
fn a_trim_records_nothing() {
    let (mut doc, ids) = doc_with(vec![scan(20)]);
    for _ in 0..4 {
        nudge(&mut doc, ids[0]);
    }
    doc.undo().unwrap();
    let generation = doc.history_generation();
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 1,
    });
    assert!(doc.enforce_history_budget().is_some());
    assert_eq!(doc.history_generation(), generation);
    assert_eq!(doc.redo_depth(), 1);
    assert_eq!(doc.undo_depth(), 1);
    doc.redo().unwrap();
}

/// Inside a transaction bracket nothing is trimmed; the commit folds the
/// bracket into one step as always, and the next trim may then drop it.
#[test]
fn no_trim_inside_a_bracket() {
    let (mut doc, ids) = doc_with(vec![scan(20)]);
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 1,
    });
    nudge(&mut doc, ids[0]);
    let txn = doc.begin_transaction();
    for _ in 0..4 {
        nudge(&mut doc, ids[0]);
        assert_eq!(doc.enforce_history_budget(), None);
    }
    let depth = doc.undo_depth();
    assert!(doc.commit_transaction(txn, meta()).unwrap());
    assert_eq!(doc.undo_depth(), depth - 3);
    let trim = doc
        .enforce_history_budget()
        .expect("over budget after the bracket");
    assert_eq!(trim.kept, 1);
    assert_eq!(
        doc.peek_undo_meta().map(|m| m.label.as_str()),
        Some("Script")
    );
    // An aborted bracket closes too.
    let txn = doc.begin_transaction();
    nudge(&mut doc, ids[0]);
    doc.abort_transaction(txn);
    nudge(&mut doc, ids[0]);
    assert!(doc.enforce_history_budget().is_some());
}

/// The saved mark moves down with the stack, and is lost (dirty until the
/// next save) when the saved state was among the dropped steps.
#[test]
fn the_saved_mark_follows_a_trim() {
    let (mut doc, ids) = doc_with(vec![scan(20)]);
    for _ in 0..3 {
        nudge(&mut doc, ids[0]);
    }
    doc.mark_saved();
    for _ in 0..3 {
        nudge(&mut doc, ids[0]);
    }
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 4,
    });
    let trim = doc.enforce_history_budget().unwrap();
    assert_eq!(trim.kept, 4);
    for _ in 0..3 {
        doc.undo().unwrap();
    }
    assert!(
        doc.at_saved_mark(),
        "the saved state is still three undos away"
    );
    for _ in 0..3 {
        doc.redo().unwrap();
    }
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 1,
    });
    doc.enforce_history_budget().unwrap();
    doc.undo().unwrap();
    assert!(!doc.at_saved_mark());
    assert_eq!(doc.saved_depth(), None);
}

/// A trim never drops the opening entry of an open editing session, so the
/// session closes and undoes as always.
#[test]
fn an_open_sessions_opening_entry_stays() {
    let (mut doc, ids) = doc_with(vec![scan(20)]);
    nudge(&mut doc, ids[0]);
    let (group, _) = doc.group_nodes(&[NodeId::Object(ids[0])]).unwrap();
    doc.open_group_session(group).unwrap();
    let opened_at = doc.undo_depth();
    for _ in 0..4 {
        nudge(&mut doc, ids[0]);
    }
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 1,
    });
    let trim = doc.enforce_history_budget().unwrap();
    assert_eq!(
        trim.dropped,
        opened_at - 1,
        "everything below the opening entry"
    );
    let mut hashes = Vec::new();
    doc.close_group_session().unwrap();
    // Undo the close, the four moves and the open, then redo them all.
    let depth = doc.undo_depth();
    for _ in 0..depth {
        hashes.push(doc.state_hash());
        doc.undo().unwrap();
    }
    hashes.push(doc.state_hash());
    hashes.reverse();
    for h in hashes.iter().skip(1) {
        doc.redo().unwrap();
        assert_eq!(doc.state_hash(), *h);
    }
    // While its closing entry stays, the session's opening entry does too
    // (undoing the close reopens the session at its recorded depth)...
    assert_eq!(doc.enforce_history_budget(), None);
    // ...and once a later step lets both go, they go like any other.
    nudge(&mut doc, ids[0]);
    assert!(doc.enforce_history_budget().is_some());
    assert_eq!(doc.undo_depth(), 1);
}

/// A trimmed push/pull's per-object entry goes with it; later push/pulls
/// on the same object still undo and redo exactly, and the labels read as
/// before.
#[test]
fn a_trimmed_object_op_takes_its_object_entry_along() {
    let (mut doc, ids) = doc_with(vec![cube(), scan(20)]);
    let mut hashes = vec![0; doc.undo_depth()];
    hashes.push(doc.state_hash());
    for i in 0..4 {
        let top = top_face(&doc, ids[0]);
        doc.apply_object_op(
            ids[0],
            KernelOp::PushPull {
                face: top,
                distance: 0.1 + i as f64 * 0.01,
            },
        )
        .unwrap();
        hashes.push(doc.state_hash());
        nudge(&mut doc, ids[1]);
        hashes.push(doc.state_hash());
    }
    doc.set_history_budget(HistoryBudget {
        max_bytes: 0,
        min_steps: 3,
    });
    let trim = doc.enforce_history_budget().unwrap();
    hashes.drain(..trim.dropped);
    let labels: Vec<String> = doc
        .history_entries()
        .undo
        .into_iter()
        .map(|e| e.label)
        .collect();
    assert_eq!(labels.len(), 3);
    assert!(labels[1].contains("Push/Pull"), "labels: {labels:?}");
    undo_redo_exact(&mut doc, &hashes);
}
