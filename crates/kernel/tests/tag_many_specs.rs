//! Executable specs for `Document::add_node_tag_many` /
//! `remove_node_tag_many` — multi-selection tagging as ONE labeled undo
//! step (the Object Info panel's "tag everything selected").

use kernel::{Document, DocumentError, NodeId, ObjectId, Plane, Point3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

fn extrude_box(doc: &mut Document, x0: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(x0, 0.0, 0.0), Point3::new(x0 + 1.0, 0.0, 0.0)),
        (
            Point3::new(x0 + 1.0, 0.0, 0.0),
            Point3::new(x0 + 1.0, 1.0, 0.0),
        ),
        (Point3::new(x0 + 1.0, 1.0, 0.0), Point3::new(x0, 1.0, 0.0)),
        (Point3::new(x0, 1.0, 0.0), Point3::new(x0, 0.0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("segment");
    }
    let r = doc.extrudable_regions(s).expect("regions")[0];
    doc.extrude_region(s, r, 1.0).expect("extrude").0
}

fn tag(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[test]
fn add_many_tags_every_node_in_one_undo_step_and_undo_reverts_all() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 2.0);
    let c = extrude_box(&mut doc, 4.0);
    // b already carries the tag: left alone, no double entry.
    doc.add_node_tag(NodeId::Object(b), tag("Hardware"))
        .unwrap();
    let depth = doc.undo_depth();

    let nodes = [NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)];
    let change = doc
        .add_node_tag_many(&nodes, tag("Hardware"), "Tag 3 objects")
        .expect("bulk tag");
    assert_eq!(change.objects_touched.len(), 3);
    for n in nodes {
        assert_eq!(doc.node_tags(n), &[tag("Hardware")]);
    }
    assert_eq!(doc.undo_depth(), depth + 1, "one compound entry");
    assert_eq!(
        doc.peek_undo_meta().map(|m| m.label.as_str()),
        Some("Tag 3 objects")
    );

    doc.undo().expect("undo");
    assert!(doc.node_tags(NodeId::Object(a)).is_empty());
    assert_eq!(
        doc.node_tags(NodeId::Object(b)),
        &[tag("Hardware")],
        "b's prior tag survives"
    );
    assert!(doc.node_tags(NodeId::Object(c)).is_empty());
}

#[test]
fn remove_many_strips_the_tag_from_every_node_in_one_step() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 2.0);
    doc.add_node_tag(NodeId::Object(a), tag("Hardware"))
        .unwrap();
    doc.add_node_tag(NodeId::Object(a), tag("Oak")).unwrap();
    doc.add_node_tag(NodeId::Object(b), tag("Hardware"))
        .unwrap();
    let depth = doc.undo_depth();
    doc.remove_node_tag_many(
        &[NodeId::Object(a), NodeId::Object(b)],
        &tag("Hardware"),
        "Untag 2 objects",
    )
    .unwrap();
    assert_eq!(doc.node_tags(NodeId::Object(a)), &[tag("Oak")]);
    assert!(doc.node_tags(NodeId::Object(b)).is_empty());
    assert_eq!(doc.undo_depth(), depth + 1);
    doc.undo().unwrap();
    assert_eq!(doc.node_tags(NodeId::Object(b)), &[tag("Hardware")]);
}

#[test]
fn a_stale_handle_aborts_the_whole_batch_with_nothing_applied() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let gone = extrude_box(&mut doc, 2.0);
    doc.delete_node(NodeId::Object(gone)).unwrap();
    let depth = doc.undo_depth();
    let err = doc
        .add_node_tag_many(
            &[NodeId::Object(a), NodeId::Object(gone)],
            tag("X"),
            "Tag 2 objects",
        )
        .expect_err("stale handle refuses");
    assert_eq!(err, DocumentError::UnknownObject);
    assert!(
        doc.node_tags(NodeId::Object(a)).is_empty(),
        "nothing applied"
    );
    assert_eq!(doc.undo_depth(), depth);
}

#[test]
fn a_no_op_batch_records_no_undo_entry() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    doc.add_node_tag(NodeId::Object(a), tag("Hardware"))
        .unwrap();
    let depth = doc.undo_depth();
    doc.add_node_tag_many(&[NodeId::Object(a)], tag("Hardware"), "Tag 1 object")
        .unwrap();
    assert_eq!(doc.undo_depth(), depth);
}
