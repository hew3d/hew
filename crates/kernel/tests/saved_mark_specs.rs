//! Executable specs for the document's SAVED MARK and its labeled history
//! listing — the kernel side of "undo back to the saved state leaves
//! nothing to save" and of the session changelog.
//!
//! Contract under test:
//! - [`Document::mark_saved`] records the current undo depth as the clean
//!   depth; [`Document::at_saved_mark`] is true exactly when the undo stack
//!   sits at that depth again — after any number of undo/redo steps.
//! - A NEW action committed while the saved depth lies in the redo branch
//!   discards that branch, and with it the only path back to the saved
//!   state: [`Document::at_saved_mark`] stays false (and
//!   [`Document::saved_depth`] is `None`) until the next `mark_saved`.
//! - A fresh or loaded document is clean at depth 0.
//! - [`Document::history_entries`] labels every entry from the action
//!   itself; a transaction's own label wins when it has one.

use kernel::{
    CompoundMeta, Document, HistoryOrigin, KernelOp, NodeId, Object, Plane, Point3, Transform, Vec3,
};

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

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

fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

fn top_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("a top face exists")
}

/// Draw a rectangle (one gesture entry) and extrude it (one entry): two
/// undo entries, one solid.
fn build_box(doc: &mut Document, x0: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    draw_rect(doc, s, x0, 0.0, x0 + 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end gesture");
    let r = only_region(doc, s);
    doc.extrude_region(s, r, 0.5).expect("extrude").0
}

fn labels(doc: &Document) -> Vec<String> {
    doc.history_entries()
        .undo
        .into_iter()
        .map(|e| e.label)
        .collect()
}

// ------------------------------------------------------------ saved mark

#[test]
fn a_fresh_document_is_clean_at_depth_zero() {
    let doc = Document::default();
    assert!(doc.at_saved_mark());
    assert_eq!(doc.saved_depth(), Some(0));
    assert_eq!(doc.history_entries().saved_depth, Some(0));
}

#[test]
fn an_edit_dirties_and_undoing_it_restores_clean() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    assert!(!doc.at_saved_mark(), "two entries past the mark");
    doc.undo().expect("undo extrude");
    assert!(!doc.at_saved_mark(), "one entry past the mark");
    doc.undo().expect("undo draw");
    assert!(doc.at_saved_mark(), "back at depth 0 — nothing to save");
    doc.redo().expect("redo draw");
    assert!(!doc.at_saved_mark());
    doc.redo().expect("redo extrude");
    assert!(!doc.at_saved_mark());
}

#[test]
fn mark_saved_makes_the_current_depth_clean_and_undo_past_it_dirties() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    doc.mark_saved();
    assert!(doc.at_saved_mark());
    assert_eq!(doc.saved_depth(), Some(2));

    // Undo BELOW the saved depth: dirty (the file has the box).
    doc.undo().expect("undo extrude");
    assert!(!doc.at_saved_mark());
    doc.redo().expect("redo extrude");
    assert!(doc.at_saved_mark(), "redo back to the saved depth is clean");

    // Edit ABOVE the saved depth: dirty; undo it: clean again.
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move");
    assert!(!doc.at_saved_mark());
    doc.undo().expect("undo move");
    assert!(doc.at_saved_mark());
    // The redo branch above the mark is still intact: redoing dirties again.
    doc.redo().expect("redo move");
    assert!(!doc.at_saved_mark());
}

#[test]
fn a_new_action_that_discards_the_redo_branch_holding_the_save_invalidates_the_mark() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move");
    doc.mark_saved(); // saved at depth 3 (draw, extrude, move)
    doc.undo().expect("undo move"); // depth 2, saved state reachable via redo
    assert!(!doc.at_saved_mark());

    // A different edit here throws the "move" branch away — and the saved
    // state with it.
    doc.transform_object(a, &Transform::translation(Vec3::new(0.0, 1.0, 0.0)))
        .expect("other move");
    assert_eq!(doc.saved_depth(), None);
    assert!(!doc.at_saved_mark());
    // Depth 3 again, but it is NOT the saved state: still dirty.
    assert_eq!(doc.undo_depth(), 3);
    doc.undo().expect("undo other move");
    assert!(!doc.at_saved_mark(), "depth 2 was never clean");
    doc.undo().expect("undo extrude");
    doc.undo().expect("undo draw");
    assert!(
        !doc.at_saved_mark(),
        "even depth 0 is dirty: the saved file has a box"
    );

    // Only a new mark recovers.
    doc.mark_saved();
    assert!(doc.at_saved_mark());
}

#[test]
fn a_new_action_with_an_empty_redo_stack_keeps_the_mark_valid() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    doc.mark_saved();
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move");
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move again");
    assert_eq!(doc.saved_depth(), Some(2));
    doc.undo().expect("undo");
    doc.undo().expect("undo");
    assert!(doc.at_saved_mark());
}

#[test]
fn a_transaction_counts_as_one_step_for_the_mark() {
    let mut doc = Document::default();
    doc.mark_saved();
    let txn = doc.begin_transaction();
    build_box(&mut doc, 0.0);
    doc.commit_transaction(
        txn,
        CompoundMeta {
            label: "Box".to_string(),
            origin: HistoryOrigin::Connection("test".to_string()),
        },
    )
    .expect("commit");
    assert_eq!(doc.undo_depth(), 1);
    assert!(!doc.at_saved_mark());
    doc.undo().expect("undo transaction");
    assert!(doc.at_saved_mark());
}

#[test]
fn a_save_load_round_trip_is_clean_with_an_empty_history() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");
    assert!(loaded.at_saved_mark());
    assert_eq!(loaded.undo_depth(), 0);
    assert!(loaded.history_entries().undo.is_empty());
}

// --------------------------------------------------------------- labels

#[test]
fn entries_are_labeled_from_their_actions_oldest_first() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    let top = top_face(doc.object(a).unwrap());
    doc.apply_object_op(
        a,
        KernelOp::PushPull {
            face: top,
            distance: 0.25,
        },
    )
    .expect("push/pull");
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move");
    doc.transform_object(
        a,
        &Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.3).expect("rotation"),
    )
    .expect("rotate");
    doc.transform_object(a, &Transform::uniform_scale(2.0))
        .expect("scale");
    doc.set_node_name(NodeId::Object(a), Some("Base".to_string()))
        .expect("rename");
    doc.set_node_name(NodeId::Object(a), Some("Plinth".to_string()))
        .expect("rename again");
    doc.delete_node(NodeId::Object(a)).expect("delete");

    assert_eq!(
        labels(&doc),
        vec![
            "Draw",
            "Push/Pull",
            "Push/Pull",
            "Move 1 object",
            "Rotate 1 object",
            "Scale 1 object",
            "Rename to 'Base'",
            "Rename 'Base' → 'Plinth'",
            "Delete 'Plinth'",
        ]
    );
    let entries = doc.history_entries();
    assert!(entries.undo.iter().all(|e| e.origin == HistoryOrigin::User));
    assert!(entries.redo.is_empty());
}

#[test]
fn per_object_op_labels_resolve_against_the_objects_own_op_stack() {
    // Two push/pulls on one object, an unrelated entry between them: each
    // ObjectOp entry must name ITS op, not just the top one.
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    let top = top_face(doc.object(a).unwrap());
    doc.apply_object_op(
        a,
        KernelOp::PushPull {
            face: top,
            distance: 0.25,
        },
    )
    .expect("push/pull 1");
    doc.set_node_name(NodeId::Object(a), Some("A".to_string()))
        .expect("rename");
    let top = top_face(doc.object(a).unwrap());
    doc.apply_object_op(
        a,
        KernelOp::PushPull {
            face: top,
            distance: 0.25,
        },
    )
    .expect("push/pull 2");
    assert_eq!(
        labels(&doc),
        vec![
            "Draw",
            "Push/Pull",
            "Push/Pull",
            "Rename to 'A'",
            "Push/Pull"
        ]
    );
    // Undo the last: it moves to the redo list, still labeled.
    doc.undo().expect("undo");
    let entries = doc.history_entries();
    assert_eq!(entries.undo.len(), 4);
    assert_eq!(entries.redo.len(), 1);
    assert_eq!(entries.redo[0].label, "Push/Pull");
}

#[test]
fn a_transactions_own_label_and_origin_win() {
    let mut doc = Document::default();
    let txn = doc.begin_transaction();
    build_box(&mut doc, 0.0);
    doc.commit_transaction(
        txn,
        CompoundMeta {
            label: "Table leg".to_string(),
            origin: HistoryOrigin::Connection("hew-cli:7".to_string()),
        },
    )
    .expect("commit");
    let entries = doc.history_entries();
    assert_eq!(entries.undo.len(), 1);
    assert_eq!(entries.undo[0].label, "Table leg");
    assert_eq!(
        entries.undo[0].origin,
        HistoryOrigin::Connection("hew-cli:7".to_string())
    );
}

#[test]
fn redo_entries_list_in_replay_order() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move");
    doc.undo().expect("undo move");
    doc.undo().expect("undo extrude");
    let entries = doc.history_entries();
    assert_eq!(entries.undo.len(), 1);
    assert_eq!(
        entries
            .redo
            .iter()
            .map(|e| e.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Push/Pull", "Move 1 object"],
        "the next redo comes first"
    );
}

// ------------------------------------------------- label coverage table

/// One label per action kind the UI produces in ordinary use: every op
/// below is run against a real document and the entry it pushed is
/// checked by exact string, so a wording change here is a deliberate
/// change to what the Changes panel and the Undo menu say.
#[test]
fn every_common_action_gets_the_label_the_ui_shows() {
    use kernel::{Anchor, AttrTarget, AttrValue, BooleanOp, EntityRef, Material, Rgba8};
    use std::num::NonZeroU32;

    let mut doc = Document::default();
    let last = |doc: &Document| {
        doc.history_entries()
            .undo
            .last()
            .map(|e| e.label.clone())
            .unwrap()
    };

    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    let c = build_box(&mut doc, 6.0);
    let d = build_box(&mut doc, 9.0);
    let e = build_box(&mut doc, 12.0);

    // Grouping.
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    assert_eq!(last(&doc), "Group");
    doc.ungroup(g).expect("ungroup");
    assert_eq!(last(&doc), "Ungroup");

    // Copies.
    let shift = Transform::translation(Vec3::new(0.0, 5.0, 0.0));
    doc.duplicate_node(NodeId::Object(a), &shift).expect("copy");
    assert_eq!(last(&doc), "Copy");
    doc.duplicate_nodes_array(&[NodeId::Object(a)], &shift, NonZeroU32::new(3).unwrap())
        .expect("array copy");
    assert_eq!(last(&doc), "Copy ×3");

    // Materials.
    let m = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let top = top_face(doc.object(a).unwrap());
    doc.paint_face(a, top, Some(m)).expect("paint");
    assert_eq!(last(&doc), "Paint face");
    doc.set_object_material(b, Some(m)).expect("base");
    assert_eq!(last(&doc), "Paint object");
    doc.set_material_alpha(m, 100).expect("alpha");
    assert_eq!(last(&doc), "Change material opacity");

    // Booleans name their op.
    doc.boolean(BooleanOp::Union, a, b).expect("union");
    assert_eq!(last(&doc), "Union");
    // `d` starts three units clear of `c`; a Subtract that removes nothing
    // is refused, so move the cutter into the target first.
    doc.transform_object(d, &Transform::translation(Vec3::new(-2.5, 0.5, 0.25)))
        .expect("move cutter into target");
    doc.boolean(BooleanOp::Subtract, c, d).expect("subtract");
    assert_eq!(last(&doc), "Subtract");

    // Components.
    let (cid, _inst, _) = doc.make_component(&[NodeId::Object(e)]).expect("component");
    assert!(last(&doc).starts_with("Make component '"), "{}", last(&doc));
    doc.place_instance(cid, shift).expect("place");
    assert_eq!(last(&doc), "Place instance");
    let prev = doc.component_name(cid).unwrap().to_string();
    doc.set_component_name(cid, Some("Chair".to_string()))
        .expect("rename def");
    assert_eq!(last(&doc), format!("Rename component '{prev}' → 'Chair'"));

    // Tags, guides, annotations, attributes.
    let x = build_box(&mut doc, 15.0);
    doc.add_node_tag(NodeId::Object(x), vec!["Walls".to_string()])
        .expect("tag");
    assert_eq!(last(&doc), "Tag");
    doc.remove_node_tag(NodeId::Object(x), &["Walls".to_string()])
        .expect("untag");
    assert_eq!(last(&doc), "Untag");
    let guide = doc
        .add_guide_line(Point3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .expect("guide");
    assert_eq!(last(&doc), "Guide");
    doc.delete_guide(guide).expect("delete guide");
    assert_eq!(last(&doc), "Delete guide");
    let dim = doc
        .add_linear_dimension(
            Anchor {
                node: None,
                point: Point3::new(0.0, 0.0, 0.0),
            },
            Anchor {
                node: None,
                point: Point3::new(1.0, 0.0, 0.0),
            },
            Vec3::new(0.0, 1.0, 0.0),
            ground(),
            None,
        )
        .expect("dimension");
    assert_eq!(last(&doc), "Dimension");
    doc.add_leader_text(
        Anchor {
            node: None,
            point: Point3::new(0.0, 0.0, 0.0),
        },
        Vec3::new(1.0, 1.0, 0.0),
        "note".to_string(),
    )
    .expect("text");
    assert_eq!(last(&doc), "Text");
    doc.delete_annotation(dim).expect("delete dimension");
    assert_eq!(last(&doc), "Delete dimension");
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(x)),
        "com.example",
        "kind",
        AttrValue::Int(1),
    )
    .expect("attr");
    assert_eq!(last(&doc), "Set attribute com.example.kind");

    // Whole-selection transforms name the kind when it is uniform.
    let y = build_box(&mut doc, 18.0);
    doc.transform_selection(&[NodeId::Object(x), NodeId::Object(y)], &[], &shift)
        .expect("move selection");
    assert_eq!(last(&doc), "Move 2 objects");
    let (_, inst2, _) = doc
        .make_component(&[NodeId::Object(y)])
        .expect("component 2");
    doc.transform_selection(&[NodeId::Object(x), NodeId::Instance(inst2)], &[], &shift)
        .expect("move mixed");
    assert_eq!(last(&doc), "Move 2 items");

    // Model-wide edits.
    doc.rescale_document(2.0).expect("rescale");
    assert_eq!(last(&doc), "Resize model ×2");
    doc.set_axes(
        Point3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
    )
    .expect("axes");
    assert_eq!(last(&doc), "Move drawing axes");

    // Deletes name what they removed.
    doc.set_node_name(NodeId::Object(x), Some("Plinth".to_string()))
        .expect("name");
    doc.delete_node(NodeId::Object(x)).expect("delete named");
    assert_eq!(last(&doc), "Delete 'Plinth'");
    let z = build_box(&mut doc, 21.0);
    let w = build_box(&mut doc, 24.0);
    let (g2, _) = doc
        .group_nodes(&[NodeId::Object(z), NodeId::Object(w)])
        .expect("group 2");
    doc.delete_node(NodeId::Group(g2)).expect("delete group");
    assert_eq!(last(&doc), "Delete group");
}

// ---------------------------------------------- session bookkeeping

/// Entering and leaving a group edit is undoable, but it is not a change
/// to what a save writes: the mark ignores it, the change count ignores
/// it, and the listing flags it so a changelog can hide it.
#[test]
fn opening_and_closing_a_group_session_is_bookkeeping_not_a_change() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    doc.mark_saved();
    assert!(doc.at_saved_mark());
    let content = doc.content_depth();

    doc.open_group_session(g).expect("open session");
    assert!(doc.at_saved_mark(), "entering a group edit dirties nothing");
    assert_eq!(doc.content_depth(), content);
    let entries = doc.history_entries();
    assert!(entries.undo.last().unwrap().bookkeeping);
    assert_eq!(entries.undo.last().unwrap().label, "Edit group");
    assert_eq!(entries.saved_depth, Some(content));

    // A real edit inside the session dirties; undoing it cleans again.
    doc.transform_object(a, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move inside");
    assert!(!doc.at_saved_mark());
    assert_eq!(doc.content_depth(), content + 1);
    doc.undo().expect("undo move");
    assert!(doc.at_saved_mark());

    doc.close_group_session().expect("close session");
    assert!(
        doc.at_saved_mark(),
        "leaving the edit dirties nothing either"
    );
    assert!(doc.history_entries().undo.last().unwrap().bookkeeping);
    assert!(
        doc.history_entries()
            .undo
            .iter()
            .filter(|e| !e.bookkeeping)
            .count()
            == content
    );

    // Undoing the bookkeeping entries keeps the document clean throughout.
    doc.undo().expect("undo close");
    assert!(doc.at_saved_mark());
    doc.undo().expect("undo open");
    assert!(doc.at_saved_mark());
}

/// A saved mark inside a redo branch is still invalidated by a new content
/// action, and a new BOOKKEEPING action discards that branch just the same.
#[test]
fn bookkeeping_pushes_still_discard_a_redo_branch_holding_the_save() {
    let mut doc = Document::default();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    doc.transform_group(g, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("move group");
    doc.mark_saved();
    doc.undo().expect("undo move"); // saved state now lives in the redo branch
    doc.open_group_session(g)
        .expect("open session discards redo");
    assert_eq!(doc.saved_depth(), None);
    assert!(!doc.at_saved_mark());
    doc.close_group_session().expect("close");
    assert!(!doc.at_saved_mark(), "still dirty: the saved state is gone");
}
