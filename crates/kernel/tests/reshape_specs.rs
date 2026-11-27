//! Executable specs for the snapshot undo every in-place true-curves
//! operation shares (`DocAction::Reshaped`, decision I-6), driven through
//! its test spec (a uniform scale about the origin).

use kernel::{Document, DocumentError, NodeId, ObjectId, Plane, Point3, Transform, Vec3};

fn ground() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

fn build_box(doc: &mut Document, x0: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(x0, 0.0), (x0 + 1.0, 0.0), (x0 + 1.0, 1.0), (x0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

#[test]
fn reshape_undo_restores_bit_exact() {
    let mut doc = Document::new();
    let o = build_box(&mut doc, 0.3);
    let before = doc.state_hash();
    let change = doc.reshape_scale_for_test(o, 1.7).unwrap();
    assert_eq!(change.objects_touched, vec![o]);
    let after = doc.state_hash();
    assert_ne!(before, after);
    doc.undo().unwrap();
    assert_eq!(
        doc.state_hash(),
        before,
        "undo restores the snapshot verbatim"
    );
}

#[test]
fn reshape_redo_reproduces_bit_exact() {
    let mut doc = Document::new();
    let o = build_box(&mut doc, 0.3);
    doc.reshape_scale_for_test(o, 1.7).unwrap();
    let after = doc.state_hash();
    for _ in 0..3 {
        doc.undo().unwrap();
        doc.redo().unwrap();
        assert_eq!(
            doc.state_hash(),
            after,
            "redo re-runs the spec on the snapshot"
        );
    }
}

#[test]
fn reshape_on_definition_member_marks_instances_touched() {
    let mut doc = Document::new();
    let o = build_box(&mut doc, 0.0);
    let (def, inst, _) = doc.make_component(&[NodeId::Object(o)]).expect("component");
    // The object becomes the definition's member, keeping its handle.
    let change = doc.reshape_scale_for_test(o, 2.0).unwrap();
    assert!(change.components_touched.contains(&def));
    assert!(change.instances_touched.contains(&inst));
    let _ = Transform::IDENTITY;
}

#[test]
fn reshape_refuses_hidden_object() {
    let mut doc = Document::new();
    let o = build_box(&mut doc, 0.0);
    doc.delete_node(NodeId::Object(o)).unwrap();
    let before = doc.state_hash();
    assert!(matches!(
        doc.reshape_scale_for_test(o, 2.0),
        Err(DocumentError::UnknownObject)
    ));
    assert_eq!(doc.state_hash(), before);
}
