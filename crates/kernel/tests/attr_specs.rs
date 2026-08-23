//! Executable specs for attribute dictionaries (docs/agents/HEW_API.md §8;
//! manifest v14+): namespaced client data the kernel stores, round-trips,
//! and never interprets. Writes are ordinary, undoable mutations; values
//! are JSON minus non-finite numbers; dictionaries follow their entity
//! through copies and survive save/load byte-exactly.

use kernel::{
    AttrTarget, AttrValue, Document, DocumentError, EntityRef, LoadError, NodeId, Plane, Point3,
    Transform, Vec3,
};
use std::collections::BTreeMap;

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn build_box(doc: &mut Document, x0: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let (y0, x1, y1) = (0.0, x0 + 1.0, 1.0);
        for (a, b) in [
            (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
            (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
            (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
            (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude").0
}

fn obj_target(obj: kernel::ObjectId) -> AttrTarget {
    AttrTarget::Entity(EntityRef::Object(obj))
}

// ------------------------------------------------------------------ specs

/// Set → get round-trips the exact value; delete removes it; each is one
/// undo entry and undo/redo restore the dictionary exactly (byte-identical
/// save at every step of the walk-back).
#[test]
fn set_get_delete_round_trip_with_exact_undo() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    let bytes_clean = doc.save();
    let depth0 = doc.undo_depth();

    let value = AttrValue::Map(BTreeMap::from([
        (
            "part_no".to_string(),
            AttrValue::Text("SHLF-204".to_string()),
        ),
        ("load_kg".to_string(), AttrValue::Float(35.5)),
    ]));
    doc.attr_set(
        obj_target(obj),
        "com.example.shelving",
        "spec",
        value.clone(),
    )
    .expect("set");
    assert_eq!(doc.undo_depth(), depth0 + 1, "one undo entry per write");
    let bytes_set = doc.save();
    assert_ne!(bytes_clean, bytes_set, "attributes serialize");

    let dict = doc
        .attr_get(&obj_target(obj))
        .expect("live target")
        .expect("dict exists");
    assert_eq!(dict["com.example.shelving"]["spec"], value);

    doc.attr_delete(obj_target(obj), "com.example.shelving", Some("spec"))
        .expect("delete");
    assert!(doc.attr_get(&obj_target(obj)).expect("live").is_none());
    assert_eq!(doc.save(), bytes_clean, "delete tidied the table exactly");

    // Walk the whole history back and forward.
    doc.undo().expect("undo delete");
    assert_eq!(doc.save(), bytes_set);
    doc.undo().expect("undo set");
    assert_eq!(doc.save(), bytes_clean);
    doc.redo().expect("redo set");
    assert_eq!(doc.save(), bytes_set);
    doc.redo().expect("redo delete");
    assert_eq!(doc.save(), bytes_clean);
}

/// Deleting a whole namespace removes every key at once and undo restores
/// them all.
#[test]
fn whole_namespace_delete_round_trips() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    doc.attr_set(obj_target(obj), "ns", "a", AttrValue::Int(1))
        .expect("set a");
    doc.attr_set(obj_target(obj), "ns", "b", AttrValue::Int(2))
        .expect("set b");
    let bytes_full = doc.save();

    doc.attr_delete(obj_target(obj), "ns", None)
        .expect("delete ns");
    assert!(doc.attr_get(&obj_target(obj)).expect("live").is_none());
    doc.undo().expect("undo namespace delete");
    assert_eq!(doc.save(), bytes_full);
}

/// The document itself carries dictionaries.
#[test]
fn document_attrs_round_trip_through_save_load() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    doc.attr_set(
        AttrTarget::Document,
        "com.example.project",
        "site",
        AttrValue::Text("Bergen".to_string()),
    )
    .expect("set");
    let bytes = doc.save();
    let reloaded = Document::load(&bytes).expect("round trip");
    assert_eq!(reloaded.save(), bytes, "save→load→save fixed point");
    let dict = reloaded
        .attr_get(&AttrTarget::Document)
        .expect("document is always a live target")
        .expect("dict survived");
    assert_eq!(
        dict["com.example.project"]["site"],
        AttrValue::Text("Bergen".to_string())
    );
}

/// Refusals: empty names, non-finite numbers, dead targets, and deleting
/// what does not exist — all typed, all leaving the document untouched.
#[test]
fn refusals_are_typed_and_leave_the_document_untouched() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    let bytes = doc.save();

    assert_eq!(
        doc.attr_set(obj_target(obj), "", "k", AttrValue::Null),
        Err(DocumentError::InvalidAttrName)
    );
    assert_eq!(
        doc.attr_set(obj_target(obj), "ns", "", AttrValue::Null),
        Err(DocumentError::InvalidAttrName)
    );
    assert_eq!(
        doc.attr_set(
            obj_target(obj),
            "ns",
            "k",
            AttrValue::List(vec![AttrValue::Float(f64::NAN)])
        ),
        Err(DocumentError::NonFiniteAttrValue)
    );
    assert_eq!(
        doc.attr_delete(obj_target(obj), "ns", Some("missing")),
        Err(DocumentError::UnknownAttr)
    );
    assert_eq!(
        doc.attr_set(
            AttrTarget::Entity(EntityRef::Tag(vec!["NoSuch".to_string()])),
            "ns",
            "k",
            AttrValue::Null
        ),
        Err(DocumentError::UnknownTag)
    );
    assert_eq!(
        doc.save(),
        bytes,
        "every refusal left the document untouched"
    );
}

/// A duplicated entity carries its source's dictionaries; the copies then
/// evolve independently.
#[test]
fn duplicates_carry_copied_dictionaries() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    doc.attr_set(obj_target(obj), "ns", "k", AttrValue::Int(7))
        .expect("set");
    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(obj),
            &Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
        )
        .expect("duplicate");
    let NodeId::Object(copy_obj) = copy else {
        panic!("object duplicate is an object");
    };
    let dict = doc
        .attr_get(&obj_target(copy_obj))
        .expect("live")
        .expect("copied");
    assert_eq!(dict["ns"]["k"], AttrValue::Int(7));

    // Independence: editing the copy leaves the source alone.
    doc.attr_set(obj_target(copy_obj), "ns", "k", AttrValue::Int(8))
        .expect("set on copy");
    assert_eq!(
        doc.attr_get(&obj_target(obj)).expect("live").expect("dict")["ns"]["k"],
        AttrValue::Int(7)
    );
}

/// Deleting an entity and undoing the delete restores its dictionaries —
/// they follow the entity's tombstone, exactly like its stable id.
#[test]
fn delete_then_undo_restores_dictionaries() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    doc.attr_set(obj_target(obj), "ns", "k", AttrValue::Bool(true))
        .expect("set");
    let bytes = doc.save();
    doc.delete_node(NodeId::Object(obj)).expect("delete");
    doc.undo().expect("undo delete");
    assert_eq!(doc.save(), bytes);
}

/// Attribute writes are transaction payload: a bracketed set commits as
/// one stamped entry with everything else.
#[test]
fn attr_writes_ride_transactions() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    let depth = doc.undo_depth();
    let txn = doc.begin_transaction();
    doc.attr_set(obj_target(obj), "ns", "k", AttrValue::Int(1))
        .expect("set");
    doc.attr_set(obj_target(obj), "ns", "k2", AttrValue::Int(2))
        .expect("set");
    doc.commit_transaction(
        txn,
        kernel::CompoundMeta {
            label: "annotate".to_string(),
            origin: kernel::HistoryOrigin::Connection("test".to_string()),
        },
    )
    .expect("commit");
    assert_eq!(doc.undo_depth(), depth + 1);
    doc.undo().expect("one undo drops both writes");
    assert!(doc.attr_get(&obj_target(obj)).expect("live").is_none());
}

/// Attrs smuggled under a pre-v14 declared version are rejected.
#[test]
fn attrs_smuggled_into_a_v13_file_are_rejected() {
    use std::io::{Cursor, Read as _, Write as _};
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    doc.attr_set(obj_target(obj), "ns", "k", AttrValue::Int(1))
        .expect("set");
    let bytes = doc.save();

    // Downgrade the version, strip sids (their own gate), keep attrs.
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    manifest["format_version"] = serde_json::json!(13);
    for arr_key in [
        "objects",
        "sketches",
        "materials",
        "groups",
        "components",
        "instances",
        "guides",
        "tags",
    ] {
        if let Some(arr) = manifest.get_mut(arr_key).and_then(|v| v.as_array_mut()) {
            for entry in arr.iter_mut() {
                if let Some(m) = entry.as_object_mut() {
                    m.remove("sid");
                }
            }
        }
    }
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();
    let mut new_zip = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched).unwrap();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name() == "manifest.json" {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        new_zip.start_file(&name, opts).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    let smuggled = new_zip.finish().unwrap().into_inner();

    let err = Document::load(&smuggled).expect_err("smuggled attrs must refuse");
    assert!(matches!(err, LoadError::MalformedManifest { .. }));
}

/// A value nested beyond the depth bound is refused typed — never a
/// stack overflow (the review-found crash: unbounded recursion through a
/// caller-built tree).
#[test]
fn too_deep_values_refuse_typed_instead_of_crashing() {
    let mut doc = Document::default();
    let obj = build_box(&mut doc, 0.0);
    let mut v = AttrValue::Null;
    for _ in 0..kernel::attr::MAX_ATTR_DEPTH {
        v = AttrValue::List(vec![v]);
    }
    assert_eq!(
        doc.attr_set(obj_target(obj), "ns", "k", v),
        Err(DocumentError::AttrValueTooDeep)
    );
}

/// Deleting a tag retires its identity: a same-path tag created later is
/// a NEW entity — fresh stable id, empty dictionaries. Undoing the delete
/// restores the ORIGINAL id and dictionaries.
#[test]
fn tag_delete_retires_identity_and_undo_restores_it() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    let path = vec!["Structure".to_string()];
    let tag = EntityRef::Tag(path.clone());
    doc.set_tag_hidden(path.clone(), false);
    doc.attr_set(
        AttrTarget::Entity(tag.clone()),
        "ns",
        "k",
        AttrValue::Int(1),
    )
    .expect("set");
    let original_sid = doc.sid_of(&tag).expect("minted");

    doc.delete_tag(&path).expect("delete");
    assert_eq!(doc.sid_of(&tag), None, "identity retired with the tag");

    // Recreate the same path: a NEW entity.
    doc.set_tag_hidden(path.clone(), true);
    let fresh_sid = doc.sid_of(&tag).expect("fresh mint");
    assert_ne!(fresh_sid, original_sid, "recreation is not resurrection");
    assert!(
        doc.attr_get(&AttrTarget::Entity(tag.clone()))
            .expect("live")
            .is_none(),
        "no dictionary resurrection"
    );

    // Fresh registration survives the old delete's undo (its identity
    // wins). set_tag_hidden is non-undoable view state, so the top of the
    // undo stack after the recreation is still the TagDeleted entry.
    doc.undo().expect("undo the tag delete");
    assert_eq!(
        doc.sid_of(&tag),
        Some(fresh_sid),
        "a re-registered path keeps its fresh identity through the undo"
    );
}

/// Without an intervening recreation, undoing a tag delete restores the
/// original stable id and dictionaries exactly.
#[test]
fn tag_delete_undo_restores_original_identity() {
    let mut doc = Document::default();
    build_box(&mut doc, 0.0);
    let path = vec!["Structure".to_string()];
    let tag = EntityRef::Tag(path.clone());
    doc.set_tag_hidden(path.clone(), false);
    doc.attr_set(
        AttrTarget::Entity(tag.clone()),
        "ns",
        "k",
        AttrValue::Int(7),
    )
    .expect("set");
    let sid = doc.sid_of(&tag).expect("minted");
    let bytes = doc.save();

    doc.delete_tag(&path).expect("delete");
    doc.undo().expect("undo the delete");
    assert_eq!(doc.sid_of(&tag), Some(sid));
    assert_eq!(
        doc.save(),
        bytes,
        "byte-exact restore, dictionaries included"
    );
}
