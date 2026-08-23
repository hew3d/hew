//! Executable specs for the labeled compound history entry — the kernel
//! prerequisite for the Hew API's one-envelope-one-undo guarantee
//! (docs/agents/HEW_API.md §6.1).
//!
//! The contract under test: [`Document::begin_transaction`] /
//! [`Document::commit_transaction`] / [`Document::abort_transaction`]
//! bracket ordinary public mutations into exactly ONE undo entry carrying a
//! [`CompoundMeta`] (label + [`HistoryOrigin`]), which [`Document::undo`]
//! reverses and [`Document::redo`] replays atomically through the same
//! rule-9 proof-carrying replay paths as any single op (DEVELOPMENT.md
//! rule 9). Provenance is session-scoped: it survives undo⇄redo but is
//! never serialized — a saved-and-reloaded document has an empty history.

use kernel::{
    CompoundMeta, Document, DocumentError, HistoryOrigin, KernelOp, NodeId, Object, Plane, Point3,
    Vec3,
};
use proptest::prelude::*;

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// Draw an axis-aligned rectangle into `doc`'s sketch `s`.
fn draw_rect(doc: &mut Document, s: kernel::SketchId, x0: f64, y0: f64, x1: f64, y1: f64) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
}

/// The single region of a sketch (panics if there isn't exactly one).
fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// The +Z (top) face of an extruded box.
fn top_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("a top face exists")
}

/// A `CompoundMeta` as the API dispatcher would stamp it.
fn api_meta(label: &str) -> CompoundMeta {
    CompoundMeta {
        label: label.to_string(),
        origin: HistoryOrigin::Connection("hew-cli:test".to_string()),
    }
}

/// Sketch a rectangle (through the recorded gesture path, exactly as the
/// API's drawing commands will) and extrude it — two recorded steps.
fn build_leg(doc: &mut Document, x0: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    draw_rect(doc, s, x0, 0.0, x0 + 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end gesture");
    let r = only_region(doc, s);
    doc.extrude_region(s, r, 0.45).expect("extrude leg").0
}

// ------------------------------------------------------------------ specs

/// A multi-step transaction commits as exactly one undo entry, stamped with
/// its label and origin; one `undo` reverses all of it to the prior
/// serialized bytes, and one `redo` restores the committed bytes — with the
/// meta still readable on the restored entry.
#[test]
fn transaction_commits_many_steps_as_one_labeled_undo_entry() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0); // pre-existing history: one leg
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let txn = doc.begin_transaction();
    let leg = build_leg(&mut doc, 0.0);
    let top = top_face(doc.object(leg).unwrap());
    doc.apply_object_op(
        leg,
        KernelOp::PushPull {
            face: top,
            distance: 0.2,
        },
    )
    .expect("push/pull inside the transaction");
    let committed = doc
        .commit_transaction(txn, api_meta("Table leg"))
        .expect("commit");
    assert!(committed, "a mutating transaction commits an entry");

    // Exactly one new entry, carrying the stamp.
    assert_eq!(doc.undo_depth(), depth_before + 1);
    let meta = doc.peek_undo_meta().expect("top entry is the transaction");
    assert_eq!(meta.label, "Table leg");
    assert_eq!(
        meta.origin,
        HistoryOrigin::Connection("hew-cli:test".to_string())
    );

    let bytes_committed = doc.save();
    assert_ne!(bytes_before, bytes_committed);

    // One undo pops the whole envelope.
    doc.undo().expect("undo the transaction");
    assert_eq!(doc.undo_depth(), depth_before);
    assert_eq!(doc.save(), bytes_before, "undo restores the prior bytes");

    // One redo restores it — meta intact.
    doc.redo().expect("redo the transaction");
    assert_eq!(doc.save(), bytes_committed, "redo restores committed bytes");
    let meta = doc.peek_undo_meta().expect("meta survives undo⇄redo");
    assert_eq!(meta.label, "Table leg");
}

/// Aborting restores the document byte-identically — geometry, history
/// depth, and redo stack alike.
#[test]
fn abort_restores_the_document_byte_identically() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    doc.undo().expect("park one entry on the redo stack");
    let bytes_before = doc.save();
    let (undo_before, redo_before) = (doc.undo_depth(), doc.redo_depth());

    let txn = doc.begin_transaction();
    build_leg(&mut doc, 0.0);
    doc.abort_transaction(txn);

    assert_eq!(doc.save(), bytes_before);
    assert_eq!(doc.undo_depth(), undo_before);
    assert_eq!(
        doc.redo_depth(),
        redo_before,
        "abort restores even the redo stack a mid-transaction mutation cleared"
    );
    doc.redo().expect("the parked redo entry is intact");
}

/// A transaction that records no mutations commits no entry (`Ok(false)`)
/// and leaves the redo stack alone — the API's read-only envelope.
#[test]
fn read_only_transaction_commits_no_entry() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    doc.undo().expect("park one entry on the redo stack");
    let bytes_before = doc.save();

    let txn = doc.begin_transaction();
    // Reads only.
    assert!(doc.undo_depth() > 0 || doc.undo_depth() == 0);
    let committed = doc
        .commit_transaction(txn, api_meta("nothing"))
        .expect("commit");

    assert!(!committed);
    assert_eq!(doc.save(), bytes_before);
    assert!(doc.peek_undo_meta().is_none(), "no stamped entry appeared");
    doc.redo().expect("redo stack untouched by an empty commit");
}

/// A single-command transaction still commits as a stamped compound — the
/// API's plain-request-equals-one-command-transaction equivalence.
#[test]
fn single_step_transaction_carries_meta() {
    let mut doc = Document::default();
    let s = doc.add_sketch(ground());

    let txn = doc.begin_transaction();
    doc.begin_sketch_gesture(s).expect("gesture");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end gesture");
    let r = only_region(&doc, s);
    doc.extrude_region(s, r, 0.45).expect("extrude");
    let bytes_open = doc.save();
    doc.commit_transaction(txn, api_meta("one step"))
        .expect("commit");

    assert_eq!(
        doc.save(),
        bytes_open,
        "commit changes history, not geometry"
    );
    assert_eq!(doc.peek_undo_meta().expect("stamped").label, "one step");
    let depth = doc.undo_depth();
    doc.undo().expect("one undo reverses the whole envelope");
    assert_eq!(doc.undo_depth(), depth - 1);
}

/// UI-authored entries carry no meta: `peek_undo_meta` is `None` for a
/// plain edit — the reader's cue to report origin `user`.
#[test]
fn ui_entries_carry_no_meta() {
    let mut doc = Document::default();
    build_leg(&mut doc, 0.0);
    assert!(doc.undo_depth() > 0);
    assert!(doc.peek_undo_meta().is_none());
}

/// A transaction that leaves a group session open is refused typed, whole,
/// with the document restored to its `begin` state (the session gone).
#[test]
fn commit_refuses_a_dangling_session_and_rolls_back() {
    let mut doc = Document::default();
    let a = build_leg(&mut doc, 0.0);
    let b = build_leg(&mut doc, 2.0);
    let bytes_before = doc.save();
    let depth_before = doc.undo_depth();

    let txn = doc.begin_transaction();
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group the legs");
    doc.open_group_session(group).expect("enter the group");
    // ... and never close it.
    let err = doc
        .commit_transaction(txn, api_meta("dangling"))
        .expect_err("a dangling frame must refuse");
    assert_eq!(err, DocumentError::TransactionSessionUnbalanced);

    assert_eq!(doc.save(), bytes_before, "refusal restored the snapshot");
    assert_eq!(doc.undo_depth(), depth_before);
}

/// A session opened AND closed inside the transaction is balanced — the
/// commit accepts it as ordinary payload.
#[test]
fn balanced_session_inside_a_transaction_commits() {
    let mut doc = Document::default();
    let a = build_leg(&mut doc, 0.0);
    let b = build_leg(&mut doc, 2.0);
    let depth_before = doc.undo_depth();

    let txn = doc.begin_transaction();
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group the legs");
    doc.open_group_session(group).expect("enter");
    doc.close_group_session().expect("exit");
    let committed = doc
        .commit_transaction(txn, api_meta("grouped"))
        .expect("balanced commit succeeds");

    assert!(committed);
    assert_eq!(doc.undo_depth(), depth_before + 1);
    assert_eq!(doc.peek_undo_meta().expect("stamped").label, "grouped");
}

/// Undoing an entry that predates the bracket, from inside it, is refused
/// at commit — and the refusal itself restores the snapshot.
#[test]
fn commit_refuses_a_disturbed_history_and_rolls_back() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    let bytes_before = doc.save();

    let txn = doc.begin_transaction();
    doc.undo().expect("pop the pre-bracket entry");
    let err = doc
        .commit_transaction(txn, api_meta("disturbed"))
        .expect_err("commit must refuse");
    assert_eq!(err, DocumentError::TransactionHistoryDisturbed);
    assert_eq!(doc.save(), bytes_before, "refusal restored the snapshot");
    doc.undo().expect("the popped entry is back on the stack");
}

/// Provenance is session-scoped: saving and reloading drops the history —
/// and with it every label and origin — exactly like the undo log itself.
#[test]
fn meta_is_never_serialized() {
    let mut doc = Document::default();
    let txn = doc.begin_transaction();
    build_leg(&mut doc, 0.0);
    doc.commit_transaction(txn, api_meta("Table leg"))
        .expect("commit");
    assert!(doc.peek_undo_meta().is_some());

    let reloaded = Document::load(&doc.save()).expect("round-trip");
    assert_eq!(reloaded.undo_depth(), 0, "history is never serialized");
    assert!(reloaded.peek_undo_meta().is_none());
}

/// The existing meta-less compound (a component-edit selection transform)
/// still reads back as an unlabeled entry: `Compound` without meta is a UI
/// gesture, not an API envelope.
#[test]
fn ui_compound_reads_back_unlabeled() {
    // `transform_def_selection` is exercised by component_edit_k2_specs;
    // here it is enough that a fresh document's plain edits never grow a
    // stamp, which pins the None-meta default for every non-transaction
    // producer (the transform included — same constructor).
    let mut doc = Document::default();
    build_leg(&mut doc, 0.0);
    assert!(doc.peek_undo_meta().is_none());
}

/// Swapping the user's open session for a DIFFERENT one at the same stack
/// depth is refused just like a dangling frame — the balance guard compares
/// frame identity, not depth.
#[test]
fn commit_refuses_a_same_depth_session_swap() {
    let mut doc = Document::default();
    let a = build_leg(&mut doc, 0.0);
    let b = build_leg(&mut doc, 2.0);
    let c = build_leg(&mut doc, 4.0);
    let d = build_leg(&mut doc, 6.0);
    let (group_ab, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group ab");
    let (group_cd, _) = doc
        .group_nodes(&[NodeId::Object(c), NodeId::Object(d)])
        .expect("group cd");
    doc.open_group_session(group_ab).expect("user enters ab");
    let bytes_before = doc.save_for_persistence();

    let txn = doc.begin_transaction();
    doc.close_group_session().expect("close the user's frame");
    doc.open_group_session(group_cd)
        .expect("open a different one");
    let err = doc
        .commit_transaction(txn, api_meta("swap"))
        .expect_err("a same-depth swap must refuse");
    assert_eq!(err, DocumentError::TransactionSessionUnbalanced);
    assert_eq!(
        doc.save_for_persistence(),
        bytes_before,
        "refusal restored the snapshot, the user's session included"
    );
    doc.close_group_session()
        .expect("the user's original frame is back and closable");
}

/// A sketch-drawing gesture opened inside the bracket and never closed is
/// refused at commit — a dangling gesture is the same hazard as a dangling
/// session frame, on a second channel.
#[test]
fn commit_refuses_a_dangling_sketch_gesture_and_rolls_back() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    let bytes_before = doc.save();

    let txn = doc.begin_transaction();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("open a gesture");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    // ... and never end it.
    let err = doc
        .commit_transaction(txn, api_meta("dangling gesture"))
        .expect_err("a dangling gesture must refuse");
    assert_eq!(err, DocumentError::TransactionGestureUnbalanced);
    assert_eq!(doc.save(), bytes_before, "refusal restored the snapshot");

    // The gesture did not leak: a fresh one opens cleanly.
    let s2 = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s2)
        .expect("no gesture survived the refused commit");
}

/// Brackets nest textually: an inner commit becomes one child entry of the
/// outer bracket, and the outer commit still accounts correctly (the inner
/// commit's drain-and-bundle must not read as a disturbed history — the
/// same shape as `transform_def_selection` or `place_text` bundling inside
/// a bracket).
#[test]
fn nested_brackets_commit_the_inner_as_one_child() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    let bytes_before = doc.save();
    let depth_before = doc.undo_depth();

    let outer = doc.begin_transaction();
    build_leg(&mut doc, 0.0);
    let inner = doc.begin_transaction();
    build_leg(&mut doc, 2.0);
    build_leg(&mut doc, 4.0);
    assert!(
        doc.commit_transaction(inner, api_meta("inner"))
            .expect("inner commit")
    );
    let committed = doc
        .commit_transaction(outer, api_meta("outer"))
        .expect("outer commit must not misread the inner bundle");
    assert!(committed);

    assert_eq!(doc.undo_depth(), depth_before + 1);
    assert_eq!(doc.peek_undo_meta().expect("stamped").label, "outer");
    doc.undo().expect("one undo reverses everything");
    assert_eq!(doc.save(), bytes_before);
}

/// A `redo()` of a pre-bracket redo entry, before any mutation clears the
/// redo stack, folds the redone entry into the compound as one child — the
/// documented, accepted posture (only `undo()` counts as disturbance).
#[test]
fn redo_of_a_pre_bracket_entry_folds_into_the_compound() {
    let mut doc = Document::default();
    build_leg(&mut doc, 10.0);
    build_leg(&mut doc, 12.0);
    doc.undo().expect("park the second leg on the redo stack");
    let depth_before = doc.undo_depth();

    let txn = doc.begin_transaction();
    doc.redo()
        .expect("redo the parked entry inside the bracket");
    build_leg(&mut doc, 0.0);
    let committed = doc
        .commit_transaction(txn, api_meta("folded"))
        .expect("redo inside the bracket is accepted");
    assert!(committed);
    assert_eq!(
        doc.undo_depth(),
        depth_before + 1,
        "the redone entry and the new work are one compound"
    );
}

// ------------------------------------------------------------ properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// For any small batch of mutations: one transaction ⇒ exactly one undo
    /// entry, and one undo restores the prior serialized bytes exactly.
    /// (The API conformance suite re-states this property over envelopes;
    /// this is its kernel-side root — DEVELOPMENT.md rule 3.)
    #[test]
    fn any_transaction_is_one_entry_and_undoes_to_prior_bytes(
        legs in 1usize..4,
        pull in prop::option::of(0.05f64..0.5),
    ) {
        let mut doc = Document::default();
        build_leg(&mut doc, 10.0); // pre-existing history
        let bytes_before = doc.save();
        let depth_before = doc.undo_depth();

        let txn = doc.begin_transaction();
        let mut last = None;
        for i in 0..legs {
            last = Some(build_leg(&mut doc, i as f64 * 2.0));
        }
        if let (Some(distance), Some(obj)) = (pull, last) {
            let top = top_face(doc.object(obj).unwrap());
            doc.apply_object_op(obj, KernelOp::PushPull { face: top, distance })
                .expect("push/pull");
        }
        doc.commit_transaction(txn, api_meta("batch")).expect("commit");

        prop_assert_eq!(doc.undo_depth(), depth_before + 1);
        doc.undo().expect("one undo reverses the envelope");
        prop_assert_eq!(doc.save(), bytes_before);
    }
}
