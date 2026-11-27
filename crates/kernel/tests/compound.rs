//! Executable specs for compound brackets, coalesced commits, and the
//! document epoch (docs/agents/PLUGINS.md "Compounds";
//! docs/agents/HEW_API.md §6.1, §6.5).
//!
//! A compound is a held transaction bracket a host opens around a
//! forwarded mutating plugin command: every transaction committed inside
//! it becomes one child of a single undo entry (PLUGINS.md invariant 4),
//! and aborting it restores the whole snapshot taken at `begin_compound`.
//! `document_epoch` is the session counter that never runs backwards,
//! unlike `history_generation`, which a restore rewinds.

use kernel::{
    AttrTarget, AttrValue, CompoundMeta, CompoundToken, Document, DocumentError, HistoryOrigin,
    Material, NodeId, Plane, Point3, Rgba8, SceneProps,
};
use proptest::prelude::*;

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn meta(label: &str, origin: &str, coalesce: Option<&str>) -> CompoundMeta {
    CompoundMeta {
        label: label.to_string(),
        origin: HistoryOrigin::Connection(origin.to_string()),
        coalesce: coalesce.map(str::to_string),
    }
}

fn plugin_meta(label: &str) -> CompoundMeta {
    meta(label, "plugin:com.example.parts", None)
}

/// One mutation a property test drives. `Rect` and `Extrude` and `Attr`
/// record undo entries; `Hide` and `Material` are the non-undoable
/// registry and view-state writes HEW_API §6.4 lists.
#[derive(Debug, Clone)]
enum Op {
    Rect,
    Extrude,
    Attr(i64),
    Hide,
    Material,
}

fn undoable(op: &Op) -> bool {
    matches!(op, Op::Rect | Op::Extrude | Op::Attr(_))
}

/// What the property tests need between ops: where the next rectangle
/// goes and which sketch still waits for an extrusion.
#[derive(Default)]
struct World {
    rects: u32,
    pending: Option<kernel::SketchId>,
}

fn apply(doc: &mut Document, world: &mut World, op: &Op) {
    match op {
        Op::Rect => {
            let x = f64::from(world.rects) * 3.0;
            world.rects += 1;
            let s = doc.add_sketch(ground());
            doc.begin_sketch_gesture(s).expect("gesture");
            let sk = doc.sketch_mut(s).expect("sketch is live");
            let corners = [
                (Point3::new(x, 0.0, 0.0), Point3::new(x + 1.0, 0.0, 0.0)),
                (
                    Point3::new(x + 1.0, 0.0, 0.0),
                    Point3::new(x + 1.0, 1.0, 0.0),
                ),
                (Point3::new(x + 1.0, 1.0, 0.0), Point3::new(x, 1.0, 0.0)),
                (Point3::new(x, 1.0, 0.0), Point3::new(x, 0.0, 0.0)),
            ];
            for (a, b) in corners {
                sk.add_segment(a, b).expect("rectangle segment");
            }
            doc.end_sketch_gesture(s).expect("end gesture");
            world.pending = Some(s);
        }
        Op::Extrude => {
            // Always records something: with no rectangle waiting, one is
            // drawn first.
            if world.pending.is_none() {
                apply(doc, world, &Op::Rect);
            }
            let s = world.pending.take().expect("a rectangle waits");
            let regions = doc.extrudable_regions(s).expect("sketch is live");
            doc.extrude_region(s, regions[0], 0.5).expect("extrude");
        }
        Op::Attr(v) => {
            doc.attr_set(
                AttrTarget::Document,
                "com.example.t",
                "k",
                AttrValue::Int(*v),
            )
            .expect("attr_set");
        }
        Op::Hide => {
            if let Some(&o) = doc.live_object_ids().first() {
                let node = NodeId::Object(o);
                let hidden = doc.user_hidden_nodes().contains(&node);
                doc.set_node_user_hidden(node, !hidden);
            }
        }
        Op::Material => {
            doc.add_material(Material::solid("M", Rgba8::rgb(10, 20, 30)));
        }
    }
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Rect),
        2 => Just(Op::Extrude),
        3 => (0..1000i64).prop_map(Op::Attr),
        1 => Just(Op::Hide),
        1 => Just(Op::Material),
    ]
}

/// Children for the undo property: everything but the palette addition,
/// whose stable id makes the restored bytes differ by construction (it is
/// pinned on its own by `compound_with_material_create_keeps_the_palette_after_undo`).
fn arb_child_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Rect),
        2 => Just(Op::Extrude),
        3 => (0..1000i64).prop_map(Op::Attr),
        1 => Just(Op::Hide),
    ]
}

fn arb_undoable_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Rect),
        2 => Just(Op::Extrude),
        3 => (0..1000i64).prop_map(Op::Attr),
    ]
}

/// Runs `ops` as one committed transaction (a child envelope).
fn transaction(doc: &mut Document, world: &mut World, ops: &[Op], label: &str) -> bool {
    let txn = doc.begin_transaction();
    for op in ops {
        apply(doc, world, op);
    }
    doc.commit_transaction(txn, meta(label, "plugin:com.example.parts", None))
        .expect("commit")
}

/// A document with some history of its own and a parked redo entry.
fn seeded() -> (Document, World) {
    let mut doc = Document::new();
    let mut world = World::default();
    apply(&mut doc, &mut world, &Op::Rect);
    apply(&mut doc, &mut world, &Op::Extrude);
    apply(&mut doc, &mut world, &Op::Attr(1));
    doc.undo().expect("park one entry on the redo stack");
    (doc, world)
}

/// Everything `abort_restores_everything` compares.
#[derive(Debug, PartialEq)]
struct Observed {
    hash: u64,
    undo: usize,
    redo: usize,
    clean: bool,
    generation: u64,
    tags: Vec<(Vec<String>, bool)>,
    hidden: Vec<NodeId>,
    scenes: usize,
}

fn observe(doc: &Document) -> Observed {
    Observed {
        hash: doc.state_hash(),
        undo: doc.undo_depth(),
        redo: doc.redo_depth(),
        clean: doc.at_saved_mark(),
        generation: doc.history_generation(),
        tags: doc.tag_meta().map(|(p, h)| (p.to_vec(), h)).collect(),
        hidden: doc.user_hidden_nodes(),
        scenes: doc.scenes().len(),
    }
}

/// Undo restores this; the non-undoable writes stay out of it.
fn undoable_state(doc: &Document) -> u64 {
    let mut probe = doc.clone();
    for node in probe.user_hidden_nodes() {
        probe.set_node_user_hidden(node, false);
    }
    probe.state_hash()
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    }
}

// --------------------------------------------------------------- properties

proptest! {
    #![proptest_config(config())]

    /// N committed transactions inside one compound are exactly one undo
    /// entry, labeled and attributed as the compound.
    #[test]
    fn compound_of_n_transactions_is_one_entry(
        txns in prop::collection::vec(prop::collection::vec(arb_undoable_op(), 1..3), 1..5),
    ) {
        let (mut doc, mut world) = seeded();
        let before = doc.undo_depth();
        let token = doc.begin_compound(plugin_meta("Cut list"));
        for (i, ops) in txns.iter().enumerate() {
            transaction(&mut doc, &mut world, ops, &format!("child {i}"));
        }
        let outcome = doc.end_compound(token).expect("end");
        prop_assert!(outcome.committed);
        prop_assert_eq!(doc.undo_depth(), before + 1);
        let top = doc.peek_undo_meta().expect("the compound is the top entry");
        prop_assert_eq!(&top.label, "Cut list");
        prop_assert_eq!(&top.origin, &HistoryOrigin::Connection("plugin:com.example.parts".into()));
        prop_assert_eq!(doc.redo_depth(), 0);
        prop_assert!(!doc.compound_open());
        prop_assert_eq!(outcome.generation, doc.history_generation());
        prop_assert_eq!(outcome.epoch, doc.document_epoch());
    }

    /// One undo of the compound restores the undoable state the document
    /// had before it began (the non-undoable writes stay, by contract).
    #[test]
    fn undo_of_compound_restores_state_hash_for_undoable_children(
        txns in prop::collection::vec(prop::collection::vec(arb_child_op(), 1..3), 1..5),
    ) {
        let (mut doc, mut world) = seeded();
        let expected = doc.clone();
        let token = doc.begin_compound(plugin_meta("Parts"));
        let mut any_undoable = false;
        for (i, ops) in txns.iter().enumerate() {
            any_undoable |= ops.iter().any(undoable);
            transaction(&mut doc, &mut world, ops, &format!("child {i}"));
        }
        let outcome = doc.end_compound(token).expect("end");
        prop_assume!(any_undoable && outcome.committed);
        // Hidden flags are view state, which `undoable_state` leaves out.
        doc.undo().expect("undo the compound");
        prop_assert_eq!(undoable_state(&doc), undoable_state(&expected));
    }

    /// Abort leaves the document exactly as it was at begin: geometry,
    /// both stacks, the saved mark, the generation, tag and hidden view
    /// state, the scene list. Only the epoch moves, forward.
    #[test]
    fn abort_restores_everything(
        txns in prop::collection::vec(prop::collection::vec(arb_op(), 1..3), 0..5),
        views in 0..4u8,
    ) {
        let (mut doc, mut world) = seeded();
        doc.mark_saved();
        let before = observe(&doc);
        let epoch = doc.document_epoch();
        let token = doc.begin_compound(plugin_meta("Parts"));
        for (i, ops) in txns.iter().enumerate() {
            transaction(&mut doc, &mut world, ops, &format!("child {i}"));
        }
        // View-state writes the aborted plugin made meanwhile.
        for v in 0..views {
            doc.set_tag_hidden(vec![format!("Tag {v}")], v % 2 == 0);
            doc.add_scene(Some(format!("Scene {v}")), SceneProps::ALL, None, None, None)
                .expect("scene");
        }
        doc.abort_compound(token).expect("abort");
        prop_assert_eq!(observe(&doc), before);
        prop_assert!(doc.document_epoch() > epoch);
        prop_assert!(!doc.compound_open());
    }

    /// A nested compound collapses into one child of the outer one.
    #[test]
    fn nested_compound_collapses_into_outer(
        outer_ops in prop::collection::vec(arb_undoable_op(), 1..3),
        inner_ops in prop::collection::vec(arb_undoable_op(), 1..3),
    ) {
        let (mut doc, mut world) = seeded();
        let before = doc.undo_depth();
        let outer = doc.begin_compound(plugin_meta("Outer"));
        transaction(&mut doc, &mut world, &outer_ops, "a");
        let inner = doc.begin_compound(meta("Inner", "plugin:com.example.lib", None));
        prop_assert_eq!(doc.compound_depth(), 2);
        prop_assert_eq!(doc.innermost_compound(), Some(inner));
        transaction(&mut doc, &mut world, &inner_ops, "b");
        prop_assert!(doc.end_compound(inner).expect("inner").committed);
        prop_assert_eq!(doc.innermost_compound(), Some(outer));
        let outcome = doc.end_compound(outer).expect("outer");
        prop_assert!(outcome.committed);
        prop_assert_eq!(doc.undo_depth(), before + 1);
        prop_assert_eq!(&doc.peek_undo_meta().expect("top").label, "Outer");
        let entries = doc.history_entries();
        prop_assert_eq!(entries.undo.len(), before + 1);
    }

    /// Keyed commits, compounds, and undo interleaved at random: whatever
    /// a compound held always ends as at most one entry right above the
    /// depth it began at, so no keyed commit inside one ever extends an
    /// older entry.
    #[test]
    fn coalesce_never_leaks_out_of_a_compound(steps in prop::collection::vec(0..6u8, 1..16)) {
        let (mut doc, mut world) = seeded();
        // Open compounds: token, depth at begin, whether anything was
        // committed inside.
        let mut open: Vec<(CompoundToken, usize, bool)> = Vec::new();
        for step in steps {
            match step {
                0 | 1 => {
                    coalesced(&mut doc, &mut world, "plugin:a", Some(if step == 0 { "k" } else { "j" }));
                    if let Some(top) = open.last_mut() {
                        top.2 = true;
                    }
                }
                2 => open.push((doc.begin_compound(meta("c", "plugin:a", None)), doc.undo_depth(), false)),
                3 => {
                    if let Some((token, depth, wrote)) = open.pop() {
                        match doc.end_compound(token) {
                            Ok(outcome) => {
                                prop_assert_eq!(outcome.committed, wrote, "a commit inside is the compound's");
                                prop_assert_eq!(doc.undo_depth(), depth + usize::from(wrote));
                                if wrote && let Some(outer) = open.last_mut() {
                                    outer.2 = true;
                                }
                            }
                            Err(e) => {
                                // An undo inside disturbed it: restored, so
                                // its writes are gone from the outer too.
                                prop_assert_eq!(e, DocumentError::TransactionHistoryDisturbed);
                            }
                        }
                    }
                }
                4 => { let _ = doc.undo(); }
                _ => {
                    // A redo inside lands as one of the compound's children.
                    if doc.redo().is_ok() && let Some(top) = open.last_mut() {
                        top.2 = true;
                    }
                }
            }
        }
    }

    /// The epoch never runs backwards, whatever happens.
    #[test]
    fn document_epoch_strictly_increases(steps in prop::collection::vec(0..7u8, 1..12)) {
        let (mut doc, mut world) = seeded();
        let mut last = doc.document_epoch();
        let mut open: Vec<CompoundToken> = Vec::new();
        for step in steps {
            match step {
                0 => { transaction(&mut doc, &mut world, &[Op::Attr(7)], "t"); }
                1 => { let _ = doc.undo(); }
                2 => { let _ = doc.redo(); }
                3 => open.push(doc.begin_compound(plugin_meta("c"))),
                4 => {
                    if let Some(t) = open.pop() {
                        doc.abort_compound(t).expect("abort");
                    } else {
                        // A refused undo (nothing to undo) still moves it.
                        while doc.can_undo() { doc.undo().expect("undo"); }
                        let _ = doc.undo();
                    }
                }
                5 => {
                    if let Some(t) = open.pop() {
                        let _ = doc.end_compound(t);
                    }
                }
                _ => {
                    let at = doc.document_epoch() + 5;
                    doc.raise_document_epoch_to(at);
                    prop_assert_eq!(doc.document_epoch(), at);
                    doc.raise_document_epoch_to(1);
                    prop_assert_eq!(doc.document_epoch(), at, "raise never lowers");
                    last = at - 1;
                }
            }
            if step == 3 {
                // Opening a compound is not a change.
                prop_assert!(doc.document_epoch() >= last);
                last = doc.document_epoch();
                continue;
            }
            if step == 5 && doc.document_epoch() == last {
                // An empty compound ends without a change.
                continue;
            }
            prop_assert!(doc.document_epoch() > last, "step {} did not move the epoch", step);
            last = doc.document_epoch();
        }
    }
}

// ------------------------------------------------------------------- specs

/// A palette addition inside a compound is not reversed by undoing it
/// (spec §19.11: undo does not reverse registry writes, compound or not).
#[test]
fn compound_with_material_create_keeps_the_palette_after_undo() {
    let (mut doc, mut world) = seeded();
    let palette = doc.materials().len();
    let token = doc.begin_compound(plugin_meta("Paint"));
    transaction(&mut doc, &mut world, &[Op::Material, Op::Attr(3)], "a");
    assert!(doc.end_compound(token).expect("end").committed);
    assert_eq!(doc.materials().len(), palette + 1);
    doc.undo().expect("undo");
    assert_eq!(
        doc.materials().len(),
        palette + 1,
        "the material stays in the palette"
    );
}

#[test]
fn end_with_wrong_token_is_mismatch_and_changes_nothing() {
    let (mut doc, mut world) = seeded();
    let outer = doc.begin_compound(plugin_meta("Outer"));
    transaction(&mut doc, &mut world, &[Op::Attr(4)], "a");
    let inner = doc.begin_compound(plugin_meta("Inner"));
    let hash = doc.state_hash();
    let (undo, epoch) = (doc.undo_depth(), doc.document_epoch());
    assert_eq!(
        doc.end_compound(outer),
        Err(DocumentError::CompoundTokenMismatch)
    );
    assert_eq!(
        doc.abort_compound(outer),
        Err(DocumentError::CompoundTokenMismatch)
    );
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.undo_depth(), undo);
    assert_eq!(doc.document_epoch(), epoch);
    assert_eq!(doc.compound_depth(), 2);
    assert_eq!(doc.innermost_compound(), Some(inner));
    // Nothing open at all is a mismatch too.
    doc.end_compound(inner).expect("inner");
    doc.end_compound(outer).expect("outer");
    assert_eq!(
        doc.end_compound(outer),
        Err(DocumentError::CompoundTokenMismatch)
    );
}

/// An undo inside the compound disturbs it: ending refuses and restores.
#[test]
fn undo_inside_a_compound_aborts_it_at_end() {
    let (mut doc, mut world) = seeded();
    let before = observe(&doc);
    let token = doc.begin_compound(plugin_meta("Parts"));
    transaction(&mut doc, &mut world, &[Op::Attr(5)], "a");
    doc.undo().expect("undo inside");
    assert_eq!(
        doc.end_compound(token),
        Err(DocumentError::TransactionHistoryDisturbed)
    );
    assert_eq!(observe(&doc), before);
    assert!(!doc.compound_open());
}

#[test]
fn empty_compound_commits_nothing() {
    let (mut doc, _) = seeded();
    let before = observe(&doc);
    let token = doc.begin_compound(plugin_meta("Nothing"));
    let outcome = doc.end_compound(token).expect("end");
    assert!(!outcome.committed);
    assert_eq!(observe(&doc), before);
}

/// Ten coalesced commits by one origin under one key are one entry.
#[test]
fn coalesce_merges_same_origin_and_key() {
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    for i in 0..10 {
        let txn = doc.begin_transaction();
        apply(&mut doc, &mut world, &Op::Attr(i));
        let label = format!("Edit {i}");
        doc.commit_transaction(txn, meta(&label, "plugin:a", Some("k")))
            .expect("commit");
    }
    assert_eq!(doc.undo_depth(), before + 1);
    assert_eq!(
        doc.peek_undo_meta().expect("top").label,
        "Edit 0",
        "the label is the first commit's"
    );
    doc.undo().expect("one undo reverses all ten");
    assert_eq!(doc.undo_depth(), before);
}

fn coalesced(doc: &mut Document, world: &mut World, origin: &str, key: Option<&str>) {
    let txn = doc.begin_transaction();
    apply(doc, world, &Op::Attr(9));
    doc.commit_transaction(txn, meta("Edit", origin, key))
        .expect("commit");
}

#[test]
fn coalesce_stops_at_other_origin() {
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    coalesced(&mut doc, &mut world, "plugin:b", Some("k"));
    assert_eq!(doc.undo_depth(), before + 2);
}

#[test]
fn coalesce_stops_at_other_key() {
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    coalesced(&mut doc, &mut world, "plugin:a", Some("j"));
    coalesced(&mut doc, &mut world, "plugin:a", None);
    coalesced(&mut doc, &mut world, "plugin:a", None);
    assert_eq!(doc.undo_depth(), before + 4);
}

#[test]
fn coalesce_stops_after_undo() {
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    doc.undo().expect("undo");
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 1);
    assert_eq!(doc.redo_depth(), 0);
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(
        doc.undo_depth(),
        before + 1,
        "merges again after the new entry"
    );

    // X(k), Y(j), undo Y, Z(k): X is on top again, but the key expired
    // when Y landed.
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    coalesced(&mut doc, &mut world, "plugin:a", Some("j"));
    doc.undo().expect("undo Y");
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 2);

    // X(k), undo, redo, Z(k): the same entry is on top, but the history
    // moved.
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    doc.undo().expect("undo X");
    doc.redo().expect("redo X");
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 2);
}

#[test]
fn coalesce_stops_at_saved_mark() {
    let (mut doc, mut world) = seeded();
    let before = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    doc.mark_saved();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 2);
    assert!(!doc.at_saved_mark(), "the edit after the save is unsaved");
    doc.undo().expect("undo");
    assert!(doc.at_saved_mark(), "undoing just the new edit is clean");
}

/// A keyed commit inside a compound never extends an entry from before
/// the compound began: the plugin command's work stays its own entry.
#[test]
fn coalesce_inside_a_compound_stays_inside_it() {
    let (mut doc, mut world) = seeded();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    let before = doc.undo_depth();
    let token = doc.begin_compound(meta("Parts", "plugin:a", None));
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 1, "a child of the compound");
    // Within the compound, keyed commits still coalesce with each other.
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), before + 1);
    let outcome = doc.end_compound(token).expect("end");
    assert!(outcome.committed);
    assert_eq!(doc.undo_depth(), before + 1);
    assert_eq!(doc.peek_undo_meta().expect("top").label, "Parts");
    doc.undo().expect("undo the compound");
    assert_eq!(doc.undo_depth(), before);

    // A nested compound's floor is its own start.
    let (mut doc, mut world) = seeded();
    let outer = doc.begin_compound(meta("Outer", "plugin:a", None));
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    let depth = doc.undo_depth();
    let inner = doc.begin_compound(meta("Inner", "plugin:a", None));
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), depth + 1);
    assert!(doc.end_compound(inner).expect("inner").committed);
    assert!(doc.end_compound(outer).expect("outer").committed);
}

#[test]
fn coalesce_bumps_generation() {
    let (mut doc, mut world) = seeded();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    let g = doc.history_generation();
    let e = doc.document_epoch();
    let depth = doc.undo_depth();
    coalesced(&mut doc, &mut world, "plugin:a", Some("k"));
    assert_eq!(doc.undo_depth(), depth, "merged");
    assert!(doc.history_generation() > g);
    assert!(doc.document_epoch() > e);
}

/// No snapshot holds another compound's snapshot: two compounds and a
/// transaction open, and every held snapshot sees no compound.
#[test]
fn snapshots_do_not_nest() {
    let (mut doc, _) = seeded();
    let a = doc.begin_compound(plugin_meta("A"));
    let b = doc.begin_compound(plugin_meta("B"));
    let txn = doc.begin_transaction();
    assert_eq!(doc.compound_snapshot_depths(), vec![0, 0]);
    assert_eq!(txn.snapshot_compound_depth(), 0);
    doc.abort_transaction(txn);
    assert_eq!(doc.compound_depth(), 2, "an abort keeps the open compounds");
    doc.abort_compound(b).expect("b");
    doc.abort_compound(a).expect("a");
    assert_eq!(doc.compound_depth(), 0);
}

/// A transaction aborted inside a compound restores to its own begin and
/// keeps the compound open.
#[test]
fn transaction_abort_inside_compound_keeps_the_compound() {
    let (mut doc, mut world) = seeded();
    let token = doc.begin_compound(plugin_meta("Parts"));
    transaction(&mut doc, &mut world, &[Op::Attr(1)], "a");
    let mid = doc.state_hash();
    let txn = doc.begin_transaction();
    apply(&mut doc, &mut world, &Op::Attr(2));
    doc.abort_transaction(txn);
    assert_eq!(doc.state_hash(), mid);
    assert_eq!(doc.innermost_compound(), Some(token));
    assert!(doc.end_compound(token).expect("end").committed);
}
