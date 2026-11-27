//! Executable specs for Lock (OW-1), the model half: the flag, the reads,
//! the undo entry and the file format. Refusing edits on a frozen node is
//! specified with the enforcement, not here.
//!
//! The contract under test:
//! - a lock belongs to a shape, not one copy: Objects and groups carry
//!   their own; a component instance stands for its definition, so
//!   locking any copy locks every copy and unlocking any copy unlocks all;
//! - `set_nodes_locked` is ONE undo entry ("Lock" / "Unlock") for the
//!   whole call, pushes nothing when nothing changes, and undo/redo
//!   restore each record's previous flag exactly;
//! - `node_frozen` names the locked node behind a freeze: the node itself,
//!   a locked group above it, or a placement of a locked shape it belongs
//!   to (and a shape placed inside something frozen is frozen through it);
//! - Make Unique on a locked copy gives the new shape a lock of its own;
//! - the locks survive save and load (manifest v33 `objects[].locked`,
//!   `groups[].locked`, `components[].locked`); a manifest declaring an
//!   older version that sets one is refused; nothing locked writes no key
//!   and hashes as before. DEVELOPMENT.md rule 3: the properties at the end.

use std::num::NonZeroU32;

use kernel::{
    ComponentId, CompoundMeta, Document, DocumentError, EntityRef, InstanceId, LinkPolicy, NodeId,
    ObjectId, Plane, Point3, Transform, Vec3,
};
use proptest::prelude::*;

// ------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A unit box at `x` on the ground.
fn board(doc: &mut Document, x: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    doc.extrude_region(s, region, 1.0).expect("extrude").0
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

fn ins(n: NodeId) -> InstanceId {
    match n {
        NodeId::Instance(i) => i,
        other => panic!("expected an instance, got {other:?}"),
    }
}

/// A board made a set of four linked copies: the four placements, the
/// shared definition, and the board (now its member).
fn four_legs(doc: &mut Document) -> (Vec<InstanceId>, ComponentId, ObjectId) {
    let o = board(doc, 0.0);
    let (copies, change) = doc
        .duplicate_nodes_array(
            &[NodeId::Object(o)],
            &shift(2.0),
            NonZeroU32::new(3).unwrap(),
            LinkPolicy::Linked,
        )
        .expect("linked copies");
    let first = change.linked[0].instance;
    let def = change.linked[0].definition;
    let mut all = vec![first];
    all.extend(copies.into_iter().map(ins));
    assert_eq!(all.len(), 4);
    (all, def, o)
}

fn undo_depth(doc: &Document) -> usize {
    doc.history_entries().undo.len()
}

fn last_label(doc: &Document) -> String {
    doc.history_entries()
        .undo
        .last()
        .expect("an entry")
        .label
        .clone()
}

fn manifest(bytes: &[u8]) -> serde_json::Value {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// `bytes` with its manifest edited by `patch`, everything else unchanged.
fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut m = manifest(bytes);
    patch(&mut m);
    let text = serde_json::to_vec_pretty(&m).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        if name == "manifest.json" {
            buf = text.clone();
        }
        out.start_file(name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// The manifest's raw text.
fn manifest_text(bytes: &[u8]) -> String {
    use std::io::Read as _;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

// ----------------------------------------------------- lock and undo

#[test]
fn locking_an_object_is_one_undoable_step() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let n = NodeId::Object(o);
    assert!(!doc.node_locked(n));
    assert_eq!(doc.node_frozen(n), None);

    let depth = undo_depth(&doc);
    doc.set_nodes_locked(&[n], true).unwrap();
    assert!(doc.node_locked(n));
    assert_eq!(
        doc.node_frozen(n),
        Some(n),
        "a locked node is its own cause"
    );
    assert_eq!(undo_depth(&doc), depth + 1);
    assert_eq!(last_label(&doc), "Lock");

    doc.undo().unwrap();
    assert!(!doc.node_locked(n), "undo unlocks");
    assert_eq!(doc.node_frozen(n), None);
    doc.redo().unwrap();
    assert!(doc.node_locked(n), "redo locks again");

    doc.set_nodes_locked(&[n], false).unwrap();
    assert!(!doc.node_locked(n));
    assert_eq!(last_label(&doc), "Unlock");
    doc.undo().unwrap();
    assert!(doc.node_locked(n), "undoing the unlock locks");
}

#[test]
fn a_call_that_changes_nothing_pushes_no_entry() {
    let mut doc = Document::new();
    let o = NodeId::Object(board(&mut doc, 0.0));
    let depth = undo_depth(&doc);
    doc.set_nodes_locked(&[o], false).unwrap();
    assert_eq!(undo_depth(&doc), depth, "already unlocked");
    doc.set_nodes_locked(&[o], true).unwrap();
    doc.set_nodes_locked(&[o], true).unwrap();
    assert_eq!(undo_depth(&doc), depth + 1, "already locked");
    assert!(doc.unlock_all().is_ok());
    assert_eq!(undo_depth(&doc), depth + 2);
    doc.unlock_all().unwrap();
    assert_eq!(undo_depth(&doc), depth + 2, "nothing left to unlock");
}

#[test]
fn several_nodes_lock_as_one_entry_and_undo_restores_each() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 2.0));
    let c = NodeId::Object(board(&mut doc, 4.0));
    let (g, _) = doc.group_nodes(&[c]).unwrap();
    let g = NodeId::Group(g);
    doc.set_nodes_locked(&[a], true).unwrap();
    let depth = undo_depth(&doc);
    // `a` is already locked: only `b` and the group flip.
    doc.set_nodes_locked(&[a, b, g, b], true).unwrap();
    assert_eq!(undo_depth(&doc), depth + 1, "one entry for the call");
    assert!(doc.node_locked(a) && doc.node_locked(b) && doc.node_locked(g));
    doc.undo().unwrap();
    assert!(doc.node_locked(a), "a was locked before the call");
    assert!(!doc.node_locked(b) && !doc.node_locked(g));
    doc.redo().unwrap();
    assert!(doc.node_locked(a) && doc.node_locked(b) && doc.node_locked(g));
}

#[test]
fn bad_input_is_refused_and_changes_nothing() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let gone = board(&mut doc, 2.0);
    doc.delete_node(NodeId::Object(gone)).unwrap();
    assert_eq!(
        doc.set_nodes_locked(&[], true).unwrap_err(),
        DocumentError::EmptySelection
    );
    let depth = undo_depth(&doc);
    assert_eq!(
        doc.set_nodes_locked(&[NodeId::Object(o), NodeId::Object(gone)], true)
            .unwrap_err(),
        DocumentError::UnknownObject
    );
    assert!(!doc.node_locked(NodeId::Object(o)), "nothing applied");
    assert_eq!(undo_depth(&doc), depth);
    assert!(!doc.node_locked(NodeId::Object(gone)));
    assert_eq!(doc.node_frozen(NodeId::Object(gone)), None);
}

#[test]
fn a_lock_inside_an_aborted_transaction_leaves_nothing() {
    let mut doc = Document::new();
    let o = NodeId::Object(board(&mut doc, 0.0));
    let depth = undo_depth(&doc);
    let txn = doc.begin_transaction();
    doc.set_nodes_locked(&[o], true).unwrap();
    assert!(doc.node_locked(o));
    doc.abort_transaction(txn);
    assert!(!doc.node_locked(o));
    assert_eq!(undo_depth(&doc), depth);

    // Committed, it is one entry that undoes whole.
    let txn = doc.begin_transaction();
    doc.set_nodes_locked(&[o], true).unwrap();
    doc.commit_transaction(
        txn,
        CompoundMeta {
            label: "Lock it".to_string(),
            origin: kernel::HistoryOrigin::User,
            coalesce: None,
        },
    )
    .unwrap();
    assert!(doc.node_locked(o));
    doc.undo().unwrap();
    assert!(!doc.node_locked(o));
}

#[test]
fn unlock_all_unlocks_objects_groups_and_shapes_in_one_entry() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 10.0));
    let (g, _) = doc.group_nodes(&[b]).unwrap();
    let g = NodeId::Group(g);
    let (legs, def, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[a], true).unwrap();
    doc.set_nodes_locked(&[g], true).unwrap();
    doc.set_nodes_locked(&[NodeId::Instance(legs[2])], true)
        .unwrap();
    let depth = undo_depth(&doc);
    doc.unlock_all().unwrap();
    assert_eq!(undo_depth(&doc), depth + 1);
    assert_eq!(last_label(&doc), "Unlock");
    assert!(!doc.node_locked(a) && !doc.node_locked(g) && !doc.component_locked(def));
    doc.undo().unwrap();
    assert!(doc.node_locked(a) && doc.node_locked(g) && doc.component_locked(def));
}

#[test]
fn the_change_names_the_record_whose_lock_flipped() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    doc.set_nodes_locked(&[NodeId::Object(o)], true).unwrap();
    let rec = doc
        .change_records_since_epoch(0)
        .expect("covered")
        .last()
        .expect("a record")
        .clone();
    let modified: Vec<EntityRef> = rec
        .changes
        .modified
        .iter()
        .map(|r| r.entity.clone())
        .collect();
    assert_eq!(modified, vec![EntityRef::Object(o)]);
    assert!(rec.changes.added.is_empty() && rec.changes.removed.is_empty());

    let (legs, def, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[1])], true)
        .unwrap();
    let rec = doc
        .change_records_since_epoch(0)
        .expect("covered")
        .last()
        .expect("a record")
        .clone();
    let modified: Vec<EntityRef> = rec
        .changes
        .modified
        .iter()
        .map(|r| r.entity.clone())
        .collect();
    assert_eq!(
        modified,
        vec![EntityRef::Component(def)],
        "a copy's lock is its shape's"
    );
}

// ------------------------------------------------- one lock per shape

#[test]
fn locking_one_of_four_linked_copies_locks_all_four() {
    let mut doc = Document::new();
    let (legs, def, member) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[2])], true)
        .unwrap();
    assert!(doc.component_locked(def), "the lock is on the shape");
    for &leg in &legs {
        let n = NodeId::Instance(leg);
        assert!(doc.node_locked(n), "every copy is locked");
        assert_eq!(doc.node_frozen(n), Some(n));
    }
    // The shape's contents are frozen, named by a placement of it.
    let cause = doc.node_frozen(NodeId::Object(member)).expect("frozen");
    assert!(legs.contains(&ins(cause)), "{cause:?}");
    assert!(
        !doc.node_locked(NodeId::Object(member)),
        "a member is frozen, not locked"
    );

    // Unlock from any other copy unlocks them all.
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], false)
        .unwrap();
    assert!(!doc.component_locked(def));
    for &leg in &legs {
        assert!(!doc.node_locked(NodeId::Instance(leg)));
        assert_eq!(doc.node_frozen(NodeId::Instance(leg)), None);
    }
    assert_eq!(doc.node_frozen(NodeId::Object(member)), None);
}

#[test]
fn a_component_placement_stands_for_its_definition() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (cid, iid, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (other, _) = doc.place_instance(cid, shift(5.0)).unwrap();
    doc.set_nodes_locked(&[NodeId::Instance(iid)], true)
        .unwrap();
    assert!(doc.component_locked(cid));
    assert!(doc.node_locked(NodeId::Instance(other)));
    // A copy placed after the lock is locked too: it is the same shape.
    let (late, _) = doc.place_instance(cid, shift(9.0)).unwrap();
    assert!(doc.node_locked(NodeId::Instance(late)));
}

#[test]
fn everything_inside_a_locked_group_is_frozen_by_it() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 2.0));
    let (inner, _) = doc.group_nodes(&[b]).unwrap();
    let (outer, _) = doc.group_nodes(&[a, NodeId::Group(inner)]).unwrap();
    let outer = NodeId::Group(outer);
    let inner = NodeId::Group(inner);
    doc.set_nodes_locked(&[outer], true).unwrap();
    for n in [a, b, inner] {
        assert!(!doc.node_locked(n), "{n:?} is not locked itself");
        assert_eq!(
            doc.node_frozen(n),
            Some(outer),
            "{n:?} is frozen by the group"
        );
    }
    // The nearest lock is named first.
    doc.set_nodes_locked(&[inner], true).unwrap();
    assert_eq!(doc.node_frozen(b), Some(inner));
    assert_eq!(doc.node_frozen(a), Some(outer));
    doc.set_nodes_locked(&[outer], false).unwrap();
    assert_eq!(doc.node_frozen(a), None);
    assert_eq!(doc.node_frozen(b), Some(inner));
}

#[test]
fn a_shape_placed_inside_a_locked_group_is_frozen_but_its_other_copies_still_move() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (cid, inside, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (outside, _) = doc.place_instance(cid, shift(5.0)).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(inside)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    assert!(!doc.component_locked(cid), "the shape itself is not locked");
    assert_eq!(
        doc.node_frozen(NodeId::Instance(inside)),
        Some(NodeId::Group(g))
    );
    assert_eq!(
        doc.node_frozen(NodeId::Instance(outside)),
        None,
        "the copy outside can still be moved"
    );
    assert_eq!(
        doc.node_frozen(NodeId::Object(o)),
        Some(NodeId::Group(g)),
        "its contents would change the locked group"
    );
}

#[test]
fn make_unique_on_a_locked_copy_gives_the_new_shape_its_own_lock() {
    let mut doc = Document::new();
    let (legs, def, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], true)
        .unwrap();
    let (fresh, _) = doc.make_unique(legs[3]).unwrap();
    assert_ne!(fresh, def);
    assert!(
        doc.component_locked(fresh),
        "the split-off shape stays locked"
    );
    assert!(doc.component_locked(def));

    // Unlocking it touches nothing else.
    doc.set_nodes_locked(&[NodeId::Instance(legs[3])], false)
        .unwrap();
    assert!(!doc.node_locked(NodeId::Instance(legs[3])));
    for &leg in &legs[..3] {
        assert!(
            doc.node_locked(NodeId::Instance(leg)),
            "the other three stay locked"
        );
    }
    // Undo each step restores the one before.
    doc.undo().unwrap();
    assert!(doc.node_locked(NodeId::Instance(legs[3])));
    doc.undo().unwrap();
    assert_eq!(doc.instance_def(legs[3]), Some(def), "back in the set");
    assert!(doc.node_locked(NodeId::Instance(legs[3])));
    doc.undo().unwrap();
    for &leg in &legs {
        assert!(!doc.node_locked(NodeId::Instance(leg)));
    }
}

#[test]
fn a_separate_copy_is_not_locked() {
    let mut doc = Document::new();
    let o = NodeId::Object(board(&mut doc, 0.0));
    doc.set_nodes_locked(&[o], true).unwrap();
    let (copy, _) = doc
        .duplicate_node(o, &shift(3.0), LinkPolicy::Independent)
        .unwrap();
    assert!(!doc.node_locked(copy));
    assert!(doc.node_locked(o));
}

// --------------------------------------------------------- the file

#[test]
fn locks_survive_save_and_load() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 2.0));
    let (g, _) = doc.group_nodes(&[b]).unwrap();
    let (legs, _, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[a, NodeId::Group(g), NodeId::Instance(legs[1])], true)
        .unwrap();
    let bytes = doc.save();
    let m = manifest(&bytes);
    assert_eq!(m["format_version"], 33);
    let count = |key: &str| {
        m[key]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e.get("locked") == Some(&serde_json::json!(true)))
            .count()
    };
    assert_eq!(count("objects"), 1);
    assert_eq!(count("groups"), 1);
    assert_eq!(count("components"), 1);
    assert!(
        m["instances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i.get("locked").is_none()),
        "an instance carries no lock of its own"
    );

    let loaded = Document::load(&bytes).expect("loads");
    assert_eq!(loaded.save(), bytes, "a load saves back identically");
    let locked_objects = loaded
        .visible_object_ids()
        .into_iter()
        .filter(|&o| loaded.node_locked(NodeId::Object(o)))
        .count();
    assert_eq!(locked_objects, 1);
    let groups: Vec<NodeId> = loaded
        .top_level_nodes()
        .into_iter()
        .filter(|n| matches!(n, NodeId::Group(_)))
        .collect();
    assert_eq!(groups.len(), 1);
    assert!(loaded.node_locked(groups[0]));
    let instances: Vec<NodeId> = loaded
        .top_level_nodes()
        .into_iter()
        .filter(|n| matches!(n, NodeId::Instance(_)))
        .collect();
    assert_eq!(instances.len(), 4);
    for i in instances {
        assert!(loaded.node_locked(i), "every copy reopens locked");
    }
}

#[test]
fn nothing_locked_writes_no_key_and_hashes_as_before() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let (g, _) = doc.group_nodes(&[a]).unwrap();
    four_legs(&mut doc);
    let before = doc.save();
    let hash = doc.state_hash();
    let text = manifest_text(&before);
    assert!(!text.contains("\"locked\""), "no locked key anywhere");

    // Locking moves the hash; unlocking brings back the same bytes.
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    assert_ne!(doc.state_hash(), hash);
    doc.set_nodes_locked(&[NodeId::Group(g)], false).unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.save(), before);
}

#[test]
fn an_older_file_loads_with_nothing_locked() {
    let mut doc = Document::new();
    board(&mut doc, 0.0);
    let v32 = patch_manifest(&doc.save(), |m| m["format_version"] = serde_json::json!(32));
    let loaded = Document::load(&v32).expect("a v32 file loads");
    let o = loaded.visible_object_ids()[0];
    assert!(!loaded.node_locked(NodeId::Object(o)));
}

#[test]
fn a_lock_is_refused_in_an_older_manifest() {
    let mut doc = Document::new();
    let a = NodeId::Object(board(&mut doc, 0.0));
    let b = NodeId::Object(board(&mut doc, 2.0));
    let (g, _) = doc.group_nodes(&[b]).unwrap();
    let (legs, _, _) = four_legs(&mut doc);
    for (node, key) in [
        (a, "object"),
        (NodeId::Group(g), "group"),
        (NodeId::Instance(legs[0]), "component"),
    ] {
        doc.set_nodes_locked(&[node], true).unwrap();
        let old = patch_manifest(&doc.save(), |m| m["format_version"] = serde_json::json!(32));
        match Document::load(&old) {
            Err(kernel::LoadError::MalformedManifest { what }) => {
                assert!(what.contains("locked") && what.contains(key), "{what}");
            }
            other => panic!("expected a malformed manifest for {key}, got {other:?}"),
        }
        doc.set_nodes_locked(&[node], false).unwrap();
    }
}

// ------------------------------------------------------- properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Any sequence of lock and unlock calls over objects, a group and a
    /// linked set: each call is at most one entry, undoing every entry
    /// leaves nothing locked, redoing them all comes back to the same
    /// state, and a save and load keeps every node's lock.
    #[test]
    fn lock_sequences_undo_redo_and_round_trip(
        steps in prop::collection::vec((prop::collection::vec(0usize..7, 1..4), any::<bool>()), 1..12)
    ) {
        let mut doc = Document::new();
        let a = NodeId::Object(board(&mut doc, 0.0));
        let b = NodeId::Object(board(&mut doc, 2.0));
        let c = NodeId::Object(board(&mut doc, 4.0));
        let (g, _) = doc.group_nodes(&[c]).unwrap();
        let (legs, _, member) = four_legs(&mut doc);
        let nodes = [
            a,
            b,
            NodeId::Group(g),
            NodeId::Instance(legs[0]),
            NodeId::Instance(legs[1]),
            NodeId::Instance(legs[2]),
            NodeId::Instance(legs[3]),
        ];
        let base = undo_depth(&doc);
        let snapshot = |doc: &Document| -> Vec<(bool, Option<NodeId>)> {
            nodes
                .iter()
                .chain([c, NodeId::Object(member)].iter())
                .map(|&n| (doc.node_locked(n), doc.node_frozen(n)))
                .collect()
        };
        for (pick, locked) in &steps {
            let before = undo_depth(&doc);
            let targets: Vec<NodeId> = pick.iter().map(|&i| nodes[i]).collect();
            doc.set_nodes_locked(&targets, *locked).unwrap();
            prop_assert!(undo_depth(&doc) <= before + 1);
            for t in &targets {
                prop_assert_eq!(doc.node_locked(*t), *locked);
            }
            // Copies share one lock.
            let shared = doc.node_locked(NodeId::Instance(legs[0]));
            for &leg in &legs {
                prop_assert_eq!(doc.node_locked(NodeId::Instance(leg)), shared);
            }
            // The group's member is frozen exactly when the group is locked.
            prop_assert_eq!(
                doc.node_frozen(c).is_some(),
                doc.node_locked(NodeId::Group(g))
            );
        }
        let end = snapshot(&doc);
        let bytes = doc.save();
        let loaded = Document::load(&bytes).unwrap();
        prop_assert_eq!(loaded.save(), bytes);

        let entries = undo_depth(&doc) - base;
        for _ in 0..entries {
            doc.undo().unwrap();
        }
        for (locked, frozen) in snapshot(&doc) {
            prop_assert!(!locked);
            prop_assert_eq!(frozen, None);
        }
        for _ in 0..entries {
            doc.redo().unwrap();
        }
        prop_assert_eq!(snapshot(&doc), end);
    }
}
