//! Executable specs for change sets and the session change ring
//! (docs/agents/HEW_API.md §4.5, docs/agents/ARCHITECTURE.md "Change
//! ring"): one record per history change, named by what it touched, kept
//! across aborts, and collapsed with the bracket that drains its children.

use kernel::{
    AttrTarget, AttrValue, CHANGE_RING_CAP, ChangeKind, ChangeRecord, ChangeSet, CompoundMeta,
    DefRecipe, Document, EntityRef, HistoryOrigin, ImportNode, ImportScene, Material, MeshRecipe,
    NodeId, Plane, Point3, Rgba8, Transform, Vec3,
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

fn meta(label: &str) -> CompoundMeta {
    CompoundMeta {
        label: label.to_string(),
        origin: HistoryOrigin::Connection("plugin:com.example.t".to_string()),
        coalesce: None,
    }
}

/// Draws a closed rectangle at `x` as one gesture; returns its sketch.
fn rect(doc: &mut Document, x: f64) -> kernel::SketchId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    s
}

/// A box at `x`: a rectangle extruded half a meter.
fn boxed(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let s = rect(doc, x);
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude");
    *doc.live_object_ids().last().expect("an object")
}

fn last(doc: &Document) -> ChangeRecord {
    doc.change_records_since_epoch(0)
        .expect("covered")
        .last()
        .expect("a record")
        .clone()
}

fn sid(doc: &Document, e: EntityRef) -> u64 {
    doc.sid_of(&e).expect("entity has a stable id")
}

fn names(list: &[kernel::ChangeRef]) -> Vec<EntityRef> {
    list.iter().map(|r| r.entity.clone()).collect()
}

fn records_since(doc: &Document, epoch: u64) -> Vec<ChangeRecord> {
    doc.change_records_since_epoch(epoch)
        .expect("covered")
        .to_vec()
}

fn commits(records: &[ChangeRecord]) -> usize {
    records
        .iter()
        .filter(|r| r.kind == ChangeKind::Commit)
        .count()
}

fn box_recipe(name: &str) -> MeshRecipe {
    let (a, b) = (Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
    MeshRecipe {
        name: name.to_string(),
        positions: vec![
            Point3::new(a.x, a.y, a.z),
            Point3::new(b.x, a.y, a.z),
            Point3::new(b.x, b.y, a.z),
            Point3::new(a.x, b.y, a.z),
            Point3::new(a.x, a.y, b.z),
            Point3::new(b.x, a.y, b.z),
            Point3::new(b.x, b.y, b.z),
            Point3::new(a.x, b.y, b.z),
        ],
        faces: vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
        face_materials: vec![kernel::NO_MATERIAL; 6],
        face_uv_frames: vec![None; 6],
        face_holes: vec![Vec::new(); 6],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

// --------------------------------------------------------------- the specs

/// Each constructible variant group lands in the list its doc comment
/// says: created → added, deleted → removed, edited → modified; attribute
/// writes name only their (target, namespace); axes are document state;
/// an import is `full`; entering an edit is bookkeeping with no lists.
#[test]
fn every_action_variant_is_classified() {
    let mut doc = Document::new();

    // create: a drawn sketch, then an extrusion that consumes it whole.
    let s = rect(&mut doc, 0.0);
    let r = last(&doc);
    assert_eq!(r.kind, ChangeKind::Commit);
    assert_eq!(names(&r.changes.added), vec![EntityRef::Sketch(s)]);
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], 0.5).unwrap();
    let o = *doc.live_object_ids().last().unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.added), vec![EntityRef::Object(o)]);
    assert_eq!(names(&r.changes.removed), vec![EntityRef::Sketch(s)]);
    assert_eq!(r.changes.added[0].sid, sid(&doc, EntityRef::Object(o)));
    assert!(!r.changes.full && !r.changes.document);

    // transform
    doc.transform_object(o, &Transform::translation(Vec3::new(0.0, 0.0, 1.0)))
        .unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.modified), vec![EntityRef::Object(o)]);
    assert!(r.changes.added.is_empty() && r.changes.removed.is_empty());

    // paint
    let m = doc.add_material(Material::solid("Oak", Rgba8::rgb(1, 2, 3)));
    doc.set_object_material(o, Some(m)).unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.modified), vec![EntityRef::Object(o)]);

    // attr: the pair and nothing else.
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(o)),
        "com.example.t",
        "k",
        AttrValue::Int(1),
    )
    .unwrap();
    let r = last(&doc);
    assert_eq!(
        r.changes,
        ChangeSet {
            attrs: vec![kernel::AttrChange::entity(
                AttrTarget::Entity(EntityRef::Object(o)),
                "com.example.t"
            )],
            ..ChangeSet::default()
        }
    );

    // group: the group is new, its members moved into it.
    let o2 = boxed(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(o), NodeId::Object(o2)])
        .unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.added), vec![EntityRef::Group(g)]);
    assert!(r.changes.modified.contains(&kernel::ChangeRef {
        entity: EntityRef::Object(o),
        sid: sid(&doc, EntityRef::Object(o)),
    }));

    // session: entering an edit is bookkeeping.
    doc.open_group_session(g).unwrap();
    let r = last(&doc);
    assert!(r.bookkeeping);
    assert_eq!(r.changes, ChangeSet::default());
    doc.close_group_session().unwrap();
    assert!(last(&doc).bookkeeping);

    // component: definition and instance are new.
    let (c, i, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    let r = last(&doc);
    assert!(names(&r.changes.added).contains(&EntityRef::Component(c)));
    assert!(names(&r.changes.added).contains(&EntityRef::Instance(i)));

    // delete
    doc.delete_node(NodeId::Instance(i)).unwrap();
    let r = last(&doc);
    assert!(names(&r.changes.removed).contains(&EntityRef::Instance(i)));
    assert!(r.changes.added.is_empty());

    // axes
    doc.set_axes(
        Point3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .unwrap();
    let r = last(&doc);
    assert!(r.changes.document && !r.changes.full);
    assert!(r.changes.added.is_empty() && r.changes.modified.is_empty());

    // material rename and delete
    doc.set_material_name(m, "Ash".to_string()).unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.modified), vec![EntityRef::Material(m)]);
    doc.delete_material(m).unwrap();
    let r = last(&doc);
    assert_eq!(names(&r.changes.removed), vec![EntityRef::Material(m)]);

    // import: full, lists empty.
    let scene = ImportScene {
        geo: None,
        materials: vec![],
        defs: vec![DefRecipe::from_meshes(None, vec![box_recipe("d")])],
        roots: vec![ImportNode::Mesh(box_recipe("b"))],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(scene, vec![]).unwrap();
    let r = last(&doc);
    assert_eq!(r.changes, ChangeSet::everything());
}

/// An undo records the entry it moved with the entry's forward lists and
/// its label and origin; a redo records it again.
#[test]
fn undo_record_mirrors_commit() {
    let mut doc = Document::new();
    let txn = doc.begin_transaction();
    let s = rect(&mut doc, 0.0);
    doc.commit_transaction(txn, meta("Sketch it")).unwrap();
    let commit = last(&doc);
    assert_eq!(commit.label, "Sketch it");
    assert_eq!(
        commit.origin,
        HistoryOrigin::Connection("plugin:com.example.t".into())
    );
    doc.undo().unwrap();
    let undo = last(&doc);
    assert_eq!(undo.kind, ChangeKind::Undo);
    assert_eq!(undo.changes, commit.changes);
    assert_eq!(names(&undo.changes.added), vec![EntityRef::Sketch(s)]);
    assert_eq!(
        (undo.label.as_str(), &undo.origin),
        ("Sketch it", &commit.origin)
    );
    assert!(undo.epoch > commit.epoch);
    assert_eq!(undo.generation, doc.history_generation());
    doc.redo().unwrap();
    let redo = last(&doc);
    assert_eq!(redo.kind, ChangeKind::Redo);
    assert_eq!(redo.changes, commit.changes);
    assert_eq!(redo.label, "Sketch it");
    // A refused undo records nothing.
    let mut empty = Document::new();
    assert!(empty.undo().is_err());
    assert_eq!(empty.change_ring_len(), 0);
}

/// An aborted transaction leaves one `Abort` record with `full` in place
/// of its children's (they never happened, as far as history goes), and
/// the records made before it survive.
#[test]
fn abort_records_full() {
    let mut doc = Document::new();
    rect(&mut doc, 0.0);
    let before = doc.change_ring_len();
    let epoch = doc.document_epoch();
    let txn = doc.begin_transaction();
    rect(&mut doc, 3.0);
    doc.abort_transaction(txn);
    let r = last(&doc);
    assert_eq!(r.kind, ChangeKind::Abort);
    assert_eq!(r.changes, ChangeSet::everything());
    assert_eq!((r.label.as_str(), &r.origin), ("", &HistoryOrigin::User));
    assert_eq!(r.epoch, doc.document_epoch());
    // The child's record went; the abort stands for it.
    let since = records_since(&doc, epoch);
    assert_eq!(
        since.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![ChangeKind::Abort]
    );
    assert_eq!(doc.change_ring_len(), before + 1);
    // A compound abort too, its committed envelopes included.
    let epoch = doc.document_epoch();
    let token = doc.begin_compound(meta("Cmd"));
    let txn = doc.begin_transaction();
    rect(&mut doc, 6.0);
    doc.commit_transaction(txn, meta("Child")).unwrap();
    assert_eq!(doc.compound_start_epoch(), Some(epoch));
    doc.abort_compound(token).unwrap();
    assert_eq!(doc.compound_start_epoch(), None);
    let since = records_since(&doc, epoch);
    assert_eq!(
        since.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![ChangeKind::Abort]
    );
}

/// A bracket's children never push older history out of the ring, however
/// many there are: they collapse into one record first. Only the record
/// that collapses them trims the ring back to its cap.
#[test]
fn bracket_children_do_not_evict_history() {
    let mut doc = Document::new();
    let g = doc
        .add_guide_line(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    let attr = |doc: &mut Document, v: i64| {
        doc.attr_set(
            AttrTarget::Entity(EntityRef::Guide(g)),
            "com.example.t",
            "k",
            AttrValue::Int(v),
        )
        .unwrap();
    };
    for v in 0..200 {
        attr(&mut doc, v);
    }
    let oldest = doc.change_records_since_epoch(0).unwrap()[0].epoch;
    let generation = doc.history_generation();
    // A compound whose command commits 300 envelopes.
    let token = doc.begin_compound(meta("Many"));
    for v in 0..300 {
        let txn = doc.begin_transaction();
        attr(&mut doc, v);
        doc.commit_transaction(txn, meta("Child")).unwrap();
    }
    doc.end_compound(token).unwrap();
    // One transaction with 300 children.
    let txn = doc.begin_transaction();
    for v in 0..300 {
        attr(&mut doc, v);
    }
    doc.commit_transaction(txn, meta("Bulk")).unwrap();
    // Everything since the first record is still there: 201 + 2.
    let all = doc
        .change_records_since_epoch(0)
        .expect("nothing was evicted");
    assert_eq!(all.len(), 203);
    assert_eq!(all[0].epoch, oldest);
    assert!(doc.changes_since_generation(generation).is_some());
    // An ordinary commit past the cap still trims.
    for v in 0..60 {
        attr(&mut doc, v);
    }
    assert_eq!(doc.change_ring_len(), CHANGE_RING_CAP);
    assert_eq!(doc.change_records_since_epoch(0), None);
}

/// 3D Text folds the glyph gesture committed just before it into its one
/// entry: one record stands for the whole placement, the gesture's own
/// record included.
#[test]
fn place_text_leaves_one_record_for_its_entry() {
    let mut doc = Document::new();
    let epoch = doc.document_epoch();
    let depth = doc.undo_depth();
    let s = rect(&mut doc, 0.0);
    let regions = doc.extrudable_regions(s).unwrap();
    doc.place_text(s, &regions, 0.2, "Text".into(), None)
        .expect("place text");
    assert_eq!(doc.undo_depth(), depth + 1);
    let records = records_since(&doc, epoch);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].kind, ChangeKind::Commit);
    assert!(!records[0].changes.added.is_empty());
}

/// Opening and closing a group session re-decides annotations anchored to
/// the group, which a save writes: they come back as modified, though the
/// entry is bookkeeping.
#[test]
fn group_session_names_the_annotations_it_reanchors() {
    let mut doc = Document::new();
    let a = boxed(&mut doc, 0.0);
    let b = boxed(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let note = doc
        .add_leader_text(
            kernel::Anchor {
                node: Some(NodeId::Group(g)),
                point: Point3::new(0.5, 0.5, 0.5),
            },
            Vec3::new(0.0, 0.0, 1.0),
            "Note".into(),
        )
        .unwrap();
    let sid = doc.sid_of(&EntityRef::Annotation(note)).unwrap();
    doc.open_group_session(g).unwrap();
    let r = last(&doc);
    assert!(r.bookkeeping);
    assert!(
        r.changes.modified.contains(&kernel::ChangeRef {
            entity: EntityRef::Annotation(note),
            sid
        }),
        "{:?}",
        r.changes
    );
    doc.close_group_session().unwrap();
    let r = last(&doc);
    assert!(r.bookkeeping);
    assert_eq!(
        names(&r.changes.modified),
        vec![EntityRef::Annotation(note)]
    );
}

/// A session close that finds the definition emptied deletes the
/// definition, the entered instance, and the live sibling placements, and
/// detaches an annotation anchored to a sibling: the close's change set
/// names all of them (a subscriber hears about a delete the save writes).
#[test]
fn an_emptied_explode_close_names_what_it_deletes() {
    let mut doc = Document::new();
    let m = boxed(&mut doc, 0.0);
    let (component, instance, _) = doc.make_component(&[NodeId::Object(m)]).unwrap();
    let (sibling, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .unwrap();
    let note = doc
        .add_leader_text(
            kernel::Anchor {
                node: Some(NodeId::Instance(sibling)),
                point: Point3::new(10.0, 0.0, 1.0),
            },
            Vec3::new(0.2, 0.2, 0.0),
            "sibling note".into(),
        )
        .unwrap();
    doc.open_explode_session(instance).unwrap();
    doc.delete_node(NodeId::Object(m)).unwrap();
    doc.close_explode_session().unwrap();
    let r = last(&doc);
    let removed = names(&r.changes.removed);
    assert!(
        removed.contains(&EntityRef::Component(component)),
        "{removed:?}"
    );
    assert!(
        removed.contains(&EntityRef::Instance(instance)),
        "{removed:?}"
    );
    assert!(
        removed.contains(&EntityRef::Instance(sibling)),
        "{removed:?}"
    );
    assert_eq!(
        names(&r.changes.modified),
        vec![EntityRef::Annotation(note)],
        "{:?}",
        r.changes
    );
}

/// Exploding an instance inside a group rewrites the group's member list,
/// so the group is modified.
#[test]
fn explode_names_the_group_it_splices_into() {
    let mut doc = Document::new();
    let a = boxed(&mut doc, 0.0);
    let b = boxed(&mut doc, 3.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(i), NodeId::Object(b)])
        .unwrap();
    doc.explode_instance(i).unwrap();
    let r = last(&doc);
    assert!(names(&r.changes.removed).contains(&EntityRef::Instance(i)));
    assert!(names(&r.changes.modified).contains(&EntityRef::Group(g)));
    doc.undo().unwrap();
    assert!(names(&last(&doc).changes.modified).contains(&EntityRef::Group(g)));
}

/// A host replacing the document records one `Reset` with `full`, and the
/// epoch moves.
#[test]
fn reset_records_full() {
    let mut doc = Document::new();
    let epoch = doc.document_epoch();
    doc.record_reset();
    let r = last(&doc);
    assert_eq!(r.kind, ChangeKind::Reset);
    assert_eq!(r.changes, ChangeSet::everything());
    assert!(doc.document_epoch() > epoch);
    // Everything since any generation is now `full`.
    assert_eq!(
        doc.changes_since_generation(0),
        Some(ChangeSet::everything())
    );
}

/// The ring keeps `CHANGE_RING_CAP` records; a reader whose epoch is
/// older than a dropped record is told it is not covered.
#[test]
fn ring_caps_and_reports_uncovered() {
    let mut doc = Document::new();
    let g = doc
        .add_guide_line(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    for _ in 0..CHANGE_RING_CAP + 9 {
        doc.attr_set(
            AttrTarget::Entity(EntityRef::Guide(g)),
            "com.example.t",
            "k",
            AttrValue::Int(1),
        )
        .unwrap();
    }
    assert_eq!(doc.change_ring_len(), CHANGE_RING_CAP);
    let records = doc
        .change_records_since_epoch(doc.document_epoch() - 3)
        .unwrap();
    assert_eq!(records.len(), 3);
    let oldest = doc
        .change_records_since_epoch(0)
        .map(|r| r.first().map(|r| r.epoch));
    // Epoch 0 is older than the dropped records: not covered.
    assert_eq!(oldest, None);
    // The oldest kept record's predecessor is the newest dropped one: a
    // reader at its epoch missed nothing.
    let first = doc.document_epoch() - CHANGE_RING_CAP as u64 + 1;
    assert_eq!(
        doc.change_records_since_epoch(first - 1).map(<[_]>::len),
        Some(CHANGE_RING_CAP)
    );
    assert_eq!(doc.change_records_since_epoch(first - 2), None);
    // The generation side: the floor is covered, below it is not.
    let generation = doc.history_generation();
    assert!(doc.changes_since_generation(generation - 3).is_some());
    assert_eq!(doc.changes_since_generation(0), None);
}

/// The union since a generation: commits add, undo records contribute
/// swapped, `full` is sticky, and the current generation is empty.
#[test]
fn changes_since_generation_unions_and_swaps_undo() {
    let mut doc = Document::new();
    let g0 = doc.history_generation();
    let s = rect(&mut doc, 0.0);
    let g1 = doc.history_generation();
    let since = doc.changes_since_generation(g0).unwrap();
    assert_eq!(names(&since.added), vec![EntityRef::Sketch(s)]);
    assert_eq!(doc.changes_since_generation(g1), Some(ChangeSet::default()));

    doc.undo().unwrap();
    let after_undo = doc.changes_since_generation(g1).unwrap();
    assert_eq!(names(&after_undo.removed), vec![EntityRef::Sketch(s)]);
    assert!(after_undo.added.is_empty());
    let whole = doc.changes_since_generation(g0).unwrap();
    assert_eq!(names(&whole.added), vec![EntityRef::Sketch(s)]);
    assert_eq!(names(&whole.removed), vec![EntityRef::Sketch(s)]);
    assert!(!whole.full);

    // An abort makes everything since before it full.
    let g2 = doc.history_generation();
    let txn = doc.begin_transaction();
    rect(&mut doc, 3.0);
    doc.abort_transaction(txn);
    assert_eq!(
        doc.changes_since_generation(g0),
        Some(ChangeSet::everything())
    );
    // The generation the abort restored names the restored state.
    assert_eq!(doc.history_generation(), g2);
    assert_eq!(doc.changes_since_generation(g2), Some(ChangeSet::default()));
    // An unknown generation is not covered.
    assert_eq!(doc.changes_since_generation(g2 + 100), None);
}

/// A committed transaction leaves one record: its children's are gone.
#[test]
fn transaction_children_do_not_leave_records() {
    let mut doc = Document::new();
    let epoch = doc.document_epoch();
    let txn = doc.begin_transaction();
    let a = rect(&mut doc, 0.0);
    let b = rect(&mut doc, 3.0);
    doc.commit_transaction(txn, meta("Two")).unwrap();
    let records = records_since(&doc, epoch);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].label, "Two");
    assert_eq!(
        names(&records[0].changes.added),
        vec![EntityRef::Sketch(a), EntityRef::Sketch(b)]
    );
    // A coalesced commit records once more, with the whole extended entry.
    let keyed = CompoundMeta {
        coalesce: Some("k".into()),
        ..meta("Edit")
    };
    let epoch = doc.document_epoch();
    for v in 0..3 {
        let txn = doc.begin_transaction();
        doc.attr_set(
            AttrTarget::Document,
            "com.example.t",
            "k",
            AttrValue::Int(v),
        )
        .unwrap();
        doc.commit_transaction(txn, keyed.clone()).unwrap();
    }
    let records = records_since(&doc, epoch);
    assert_eq!(commits(&records), 3);
    assert_eq!(records.last().unwrap().label, "Edit");
}

/// A compound leaves one record however many envelopes committed inside
/// it, nested compounds included.
#[test]
fn compound_children_do_not_leave_records() {
    let mut doc = Document::new();
    let epoch = doc.document_epoch();
    let outer = doc.begin_compound(meta("Outer"));
    for x in [0.0, 3.0] {
        let txn = doc.begin_transaction();
        rect(&mut doc, x);
        doc.commit_transaction(txn, meta("Child")).unwrap();
    }
    let inner = doc.begin_compound(meta("Inner"));
    let txn = doc.begin_transaction();
    rect(&mut doc, 6.0);
    doc.commit_transaction(txn, meta("Grandchild")).unwrap();
    // An aborted envelope inside leaves nothing either, once the compound
    // ends.
    let txn = doc.begin_transaction();
    rect(&mut doc, 9.0);
    doc.abort_transaction(txn);
    doc.end_compound(inner).unwrap();
    doc.end_compound(outer).unwrap();
    let records = records_since(&doc, epoch);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].label, "Outer");
    assert_eq!(records[0].changes.added.len(), 3);
    assert_eq!(doc.undo_depth(), 1);
    // An empty compound records nothing.
    let epoch = doc.document_epoch();
    let token = doc.begin_compound(meta("Nothing"));
    doc.end_compound(token).unwrap();
    assert!(records_since(&doc, epoch).is_empty());
}

// ------------------------------------------------------ record accounting

#[derive(Debug, Clone)]
enum Op {
    Rect,
    Extrude,
    Attr(i64),
    Move,
    Delete,
    Group,
    Ungroup,
    Component,
    DeleteDefinition,
    Paint,
    RenameMaterial,
    DeleteMaterial,
    EnterGroup,
    ExitGroup,
    Guide,
    Purge,
    Undo,
    Redo,
    /// A committed API-style bracket around `n` rectangles.
    Transaction(u8),
    /// An aborted one.
    Aborted(u8),
    /// A compound around two committed brackets.
    Compound,
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Rect),
        3 => Just(Op::Extrude),
        2 => (0..9i64).prop_map(Op::Attr),
        2 => Just(Op::Move),
        2 => Just(Op::Delete),
        2 => Just(Op::Group),
        1 => Just(Op::Ungroup),
        1 => Just(Op::Component),
        1 => Just(Op::DeleteDefinition),
        1 => Just(Op::Paint),
        1 => Just(Op::RenameMaterial),
        1 => Just(Op::DeleteMaterial),
        1 => Just(Op::EnterGroup),
        1 => Just(Op::ExitGroup),
        1 => Just(Op::Guide),
        1 => Just(Op::Purge),
        3 => Just(Op::Undo),
        2 => Just(Op::Redo),
        2 => (1..3u8).prop_map(Op::Transaction),
        1 => (1..3u8).prop_map(Op::Aborted),
        1 => Just(Op::Compound),
    ]
}

#[derive(Default)]
struct World {
    x: f64,
    pending: Vec<kernel::SketchId>,
    material: Option<kernel::MaterialId>,
}

/// Runs `op`; ops whose preconditions fail are skipped (an error leaves
/// the document untouched, which the property also checks).
fn run(doc: &mut Document, w: &mut World, op: &Op) {
    let mut next_x = || {
        w.x += 3.0;
        w.x
    };
    match op {
        Op::Rect => {
            let x = next_x();
            let s = rect(doc, x);
            w.pending.push(s);
        }
        Op::Extrude => {
            if let Some(s) = w.pending.pop()
                && let Ok(regions) = doc.extrudable_regions(s)
                && let Some(&r) = regions.first()
            {
                let _ = doc.extrude_region(s, r, 0.5);
            }
        }
        Op::Attr(v) => {
            let _ = doc.attr_set(
                AttrTarget::Document,
                "com.example.t",
                "k",
                AttrValue::Int(*v),
            );
        }
        Op::Move => {
            if let Some(&o) = doc.live_object_ids().first() {
                let _ = doc.transform_object(o, &Transform::translation(Vec3::new(0.0, 0.0, 0.1)));
            }
        }
        Op::Delete => {
            if let Some(&n) = doc.top_level_nodes().last() {
                let _ = doc.delete_node(n);
            }
        }
        Op::Group => {
            let roots = doc.top_level_nodes();
            if roots.len() >= 2 {
                let _ = doc.group_nodes(&roots[roots.len() - 2..]);
            }
        }
        Op::Ungroup => {
            if let Some(&NodeId::Group(g)) = doc
                .top_level_nodes()
                .iter()
                .find(|n| matches!(n, NodeId::Group(_)))
            {
                let _ = doc.ungroup(g);
            }
        }
        Op::Component => {
            if let Some(&n) = doc.top_level_nodes().first() {
                let _ = doc.make_component(&[n]);
            }
        }
        Op::DeleteDefinition => {
            if let Some(&c) = doc.component_ids().first() {
                let _ = doc.delete_definition(c);
            }
        }
        Op::Paint => {
            let m = *w
                .material
                .get_or_insert_with(|| doc.add_material(Material::solid("M", Rgba8::rgb(9, 9, 9))));
            if let Some(&o) = doc.live_object_ids().first() {
                let _ = doc.set_object_material(o, Some(m));
            }
        }
        Op::RenameMaterial => {
            if let Some(m) = w.material {
                let _ = doc.set_material_name(m, format!("M{}", doc.history_generation()));
            }
        }
        Op::DeleteMaterial => {
            if let Some(m) = w.material.take() {
                let _ = doc.delete_material(m);
            }
        }
        Op::EnterGroup => {
            if let Some(&NodeId::Group(g)) = doc
                .top_level_nodes()
                .iter()
                .find(|n| matches!(n, NodeId::Group(_)))
            {
                let _ = doc.open_group_session(g);
            }
        }
        Op::ExitGroup => {
            let _ = doc.close_group_session();
        }
        Op::Guide => {
            let _ = doc.add_guide_line(Point3::ORIGIN, Vec3::new(0.0, 1.0, 0.0));
        }
        Op::Purge => {
            let _ = doc.purge_unused();
        }
        Op::Undo => {
            let _ = doc.undo();
        }
        Op::Redo => {
            let _ = doc.redo();
        }
        Op::Transaction(n) => {
            let txn = doc.begin_transaction();
            for _ in 0..*n {
                let x = next_x();
                rect(doc, x);
            }
            doc.commit_transaction(txn, meta("Tx")).unwrap();
        }
        Op::Aborted(n) => {
            let txn = doc.begin_transaction();
            for _ in 0..*n {
                let x = next_x();
                rect(doc, x);
            }
            doc.abort_transaction(txn);
        }
        Op::Compound => {
            let token = doc.begin_compound(meta("Cmd"));
            for _ in 0..2 {
                let txn = doc.begin_transaction();
                let x = next_x();
                rect(doc, x);
                doc.commit_transaction(txn, meta("Child")).unwrap();
            }
            doc.end_compound(token).unwrap();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// After each operation: a new entry is exactly one `Commit` record;
    /// an undo or redo that ran is exactly one record of its kind; an
    /// abort is one `Abort`; nothing else records. Epochs strictly
    /// increase along the ring.
    #[test]
    fn one_commit_record_per_visible_entry(ops in proptest::collection::vec(arb_op(), 1..40)) {
        let mut doc = Document::new();
        let mut w = World::default();
        for op in &ops {
            let depth = doc.undo_depth();
            let redo_depth = doc.redo_depth();
            let epoch = doc.document_epoch();
            run(&mut doc, &mut w, op);
            let new = records_since(&doc, epoch);
            let kinds: Vec<ChangeKind> = new.iter().map(|r| r.kind).collect();
            match op {
                Op::Undo if doc.redo_depth() == redo_depth + 1 => {
                    prop_assert_eq!(kinds, vec![ChangeKind::Undo], "{:?}", op);
                }
                Op::Redo if doc.undo_depth() == depth + 1 => {
                    prop_assert_eq!(kinds, vec![ChangeKind::Redo], "{:?}", op);
                }
                Op::Aborted(_) => {
                    prop_assert_eq!(kinds, vec![ChangeKind::Abort]);
                    prop_assert_eq!(doc.undo_depth(), depth);
                }
                _ => {
                    let grew = doc.undo_depth().saturating_sub(depth);
                    prop_assert_eq!(commits(&new), grew, "{:?}: {:?}", op, kinds);
                    prop_assert_eq!(new.len(), grew, "{:?}: {:?}", op, kinds);
                }
            }
            let all = records_since(&doc, 0);
            prop_assert!(all.windows(2).all(|p| p[0].epoch < p[1].epoch));
            if let Some(r) = all.last() {
                prop_assert!(r.epoch <= doc.document_epoch());
            }
        }
    }
}
