//! Specs for [`Document::insert_as_definition`]: a whole item becomes one
//! unplaced, provenance-stamped component definition that later calls with
//! the same provenance reuse, as one undo step.

use kernel::{Document, DocumentError, LibraryProvenance, NodeId, Plane, Point3, Transform, Vec3};

fn add_box(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane");
    let s = doc.add_sketch(plane);
    let c = [(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 1.0), (x, 1.0)];
    let sk = doc.sketch_mut(s).expect("sketch");
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .expect("segment");
    }
    let r = doc.extrudable_regions(s).expect("regions");
    doc.extrude_region(s, r[0], 1.0).expect("extrude").0
}

fn prov(hash: &str) -> LibraryProvenance {
    LibraryProvenance {
        source_id: format!("import:{hash}"),
        content_hash: hash.to_string(),
    }
}

fn one_box_item() -> Document {
    let mut item = Document::new();
    add_box(&mut item, 0.0);
    item
}

#[test]
fn one_box_item_makes_one_unplaced_named_definition() {
    let mut doc = Document::new();
    let (cid, reused, _) = doc
        .insert_as_definition(&one_box_item(), Some("  Hinge "), prov("a"))
        .unwrap();
    assert!(!reused);
    assert_eq!(doc.component_name(cid), Some("Hinge"));
    assert_eq!(doc.def_members(cid).unwrap().len(), 1);
    assert!(doc.instance_ids().is_empty());
    assert!(doc.top_level_nodes().is_empty(), "no world node appears");
    assert_eq!(doc.undo_depth(), 1);
    Document::load(&doc.save()).expect("reopens");
}

#[test]
fn same_provenance_reuses_without_an_undo_entry() {
    let mut doc = Document::new();
    let (a, _, _) = doc
        .insert_as_definition(&one_box_item(), Some("A"), prov("a"))
        .unwrap();
    let hash = doc.state_hash();
    let (b, reused, change) = doc
        .insert_as_definition(&one_box_item(), Some("B"), prov("a"))
        .unwrap();
    assert!(reused);
    assert_eq!(a, b);
    assert_eq!(change, kernel::DocChange::default());
    assert_eq!(doc.undo_depth(), 1);
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.component_name(a), Some("A"), "first import names it");
}

#[test]
fn different_hash_makes_a_second_definition() {
    let mut doc = Document::new();
    let (a, _, _) = doc
        .insert_as_definition(&one_box_item(), None, prov("a"))
        .unwrap();
    let (b, reused, _) = doc
        .insert_as_definition(&one_box_item(), None, prov("b"))
        .unwrap();
    assert!(!reused);
    assert_ne!(a, b);
    assert_eq!(doc.component_ids().len(), 2);
}

#[test]
fn a_nested_definition_comes_along_as_a_member() {
    let mut item = Document::new();
    let o = add_box(&mut item, 0.0);
    item.make_component(&[NodeId::Object(o)]).unwrap();
    add_box(&mut item, 5.0);
    let mut doc = Document::new();
    let (wrapper, _, _) = doc.insert_as_definition(&item, None, prov("n")).unwrap();
    assert_eq!(doc.component_ids().len(), 2, "wrapper plus the nested one");
    assert_eq!(doc.def_members(wrapper).unwrap().len(), 1, "the loose box");
    assert_eq!(doc.expanded_total(), 0, "nothing is placed");
    Document::load(&doc.save()).expect("reopens");
}

#[test]
fn undo_removes_everything_and_redo_restores_it_bitwise() {
    let mut doc = Document::new();
    let mut item = Document::new();
    let o = add_box(&mut item, 0.0);
    item.make_component(&[NodeId::Object(o)]).unwrap();
    add_box(&mut item, 4.0);
    let before = doc.state_hash();
    doc.insert_as_definition(&item, Some("W"), prov("u"))
        .unwrap();
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert!(doc.component_ids().is_empty());
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    // After an undo a fresh import is not a reuse of the hidden definition.
    doc.undo().unwrap();
    let (_, reused, _) = doc.insert_as_definition(&item, None, prov("u")).unwrap();
    assert!(!reused);
}

#[test]
fn the_definition_places_like_any_other() {
    let mut doc = Document::new();
    let (cid, _, _) = doc
        .insert_as_definition(&one_box_item(), None, prov("p"))
        .unwrap();
    for i in 0..3 {
        doc.place_instance(
            cid,
            Transform::translation(Vec3::new(i as f64 * 2.0, 0.0, 0.0)),
        )
        .unwrap();
    }
    assert_eq!(doc.instance_ids().len(), 3);
    assert_eq!(doc.component_ids().len(), 1);
    assert_eq!(doc.expanded_placements().len(), 3);
}

#[test]
fn empty_item_refuses() {
    let mut doc = Document::new();
    let before = doc.state_hash();
    assert!(matches!(
        doc.insert_as_definition(&Document::new(), None, prov("e")),
        Err(DocumentError::EmptyGroup)
    ));
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn refuses_during_a_component_session() {
    let mut doc = Document::new();
    let a = add_box(&mut doc, 0.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.open_explode_session(inst).unwrap();
    assert!(matches!(
        doc.insert_as_definition(&one_box_item(), None, prov("s")),
        Err(DocumentError::ExplodeSessionScope)
    ));
}

fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    patch(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();
    let mut new_zip = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    new_zip.start_file("manifest.json", opts).unwrap();
    new_zip.write_all(&patched).unwrap();
    let mut zip2 = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..zip2.len() {
        let mut entry = zip2.by_index(i).unwrap();
        if entry.name().unwrap() == "manifest.json" {
            continue;
        }
        let name = entry.name().unwrap().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        new_zip.start_file(&name, opts).unwrap();
        new_zip.write_all(&buf).unwrap();
    }
    new_zip.finish().unwrap().into_inner()
}

#[test]
fn a_file_carried_sentinel_sid_cannot_stand_in_for_the_wrapper() {
    let mut item = Document::new();
    let o = add_box(&mut item, 0.0);
    item.make_component(&[NodeId::Object(o)]).unwrap();
    add_box(&mut item, 5.0);
    let bytes = patch_manifest(&item.save(), |m| {
        m["components"][0]["sid"] = serde_json::json!(u64::MAX);
    });
    let item = Document::load(&bytes).expect("a top-of-range sid loads without overflow");
    let mut doc = Document::new();
    let (wrapper, reused, _) = doc.insert_as_definition(&item, None, prov("s")).unwrap();
    assert!(!reused);
    let (again, reused, _) = doc.insert_as_definition(&item, None, prov("s")).unwrap();
    assert!(reused);
    assert_eq!(wrapper, again, "the wrapper, never the nested definition");
    assert_eq!(doc.def_members(wrapper).unwrap().len(), 1, "the loose box");
}

#[test]
fn the_items_tags_register_with_their_hidden_flags_and_undo_removes_them() {
    let mut item = Document::new();
    let o = add_box(&mut item, 0.0);
    item.add_node_tag(NodeId::Object(o), vec!["Ghost".to_string()])
        .unwrap();
    item.set_tag_hidden(vec!["Ghost".to_string()], true);
    let mut doc = Document::new();
    let before = doc.state_hash();
    doc.insert_as_definition(&item, None, prov("t")).unwrap();
    let path = vec!["Ghost".to_string()];
    assert!(doc.tag_meta().any(|(p, _)| p == path.as_slice()));
    assert!(doc.tag_hidden(&path), "the source's hidden flag carries");
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert!(doc.tag_meta().next().is_none());
}
