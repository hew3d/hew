//! A document carrying attributes in a plugin's namespace opens and saves
//! byte for byte the same, with no notice, in every build. A production
//! build has no plugins, so this is how it must treat a file a plugin in a
//! preview build wrote: as data it keeps, not something it reports or
//! drops. Runs in both builds; it names nothing gated.

use kernel::{AttrTarget, AttrValue, Document, EntityRef, Plane, Point3, Vec3};
use serde_json::{Value, json};
use std::path::PathBuf;

/// A box carrying `com.example.cutlist` (a plugin's namespace) on the
/// document, the object, a face and an edge, with a fixed document id so
/// a save writes the same bytes.
fn plugin_attributed_document() -> Document {
    let mut doc = Document::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
            (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
            (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
            (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (id, _) = doc.extrude_region(s, region, 1.0).unwrap();
    let o = doc.object(id).unwrap();
    let top = o
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(f, _)| f)
        .unwrap();
    let edge = o.edges().keys().next().unwrap();
    let ns = "com.example.cutlist";
    let nested =
        AttrValue::from_json(&json!({ "part": "Shelf", "grain": [1, 0.5, null, true] })).unwrap();
    doc.attr_set(
        AttrTarget::Document,
        ns,
        "project",
        AttrValue::Text("Kitchen".into()),
    )
    .unwrap();
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(id)),
        ns,
        "spec",
        nested,
    )
    .unwrap();
    doc.face_attr_set(id, top, ns, "part", AttrValue::Text("Top".into()))
        .unwrap();
    doc.edge_attr_set(id, edge, ns, "band", AttrValue::Float(0.5))
        .unwrap();
    doc.ensure_document_id([7; 16]);
    doc
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hew-cli-plugin-attrs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn plugin_attributes_survive_open_and_save_byte_for_byte() {
    let dir = scratch();
    let original = dir.join("original.hew");
    let saved = dir.join("saved.hew");
    let bytes = plugin_attributed_document().save();
    std::fs::write(&original, &bytes).unwrap();

    let frames = [
        json!({"jsonrpc":"2.0","id":1,"method":"hew.meta.hello","params":{"protocol":1}}),
        json!({"jsonrpc":"2.0","id":2,"method":"hew.doc.open","params":{"path": original.to_str().unwrap()}}),
        json!({"jsonrpc":"2.0","id":3,"method":"hew.attr.get","params":{"target":"document"}}),
        json!({"jsonrpc":"2.0","id":4,"method":"hew.doc.save","params":{"path": saved.to_str().unwrap()}}),
    ];
    let script = dir.join("round_trip.jsonl");
    std::fs::write(
        &script,
        frames
            .iter()
            .map(|f| f.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let outcome = hew_cli::run::run_script(&script, None, None);
    assert_eq!(outcome.exit_code, 0, "{:?}", outcome.responses);
    let [_, open, attrs, save] = outcome.responses.as_slice() else {
        panic!("four replies: {:?}", outcome.responses);
    };

    // No notice: the open answers only the document, the save only what
    // its schema allows for a .hew (no `simplified`, no `writer_notes`).
    let open = &open["result"];
    let keys: Vec<&str> = open
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.iter()
            .all(|k| ["document", "already_open"].contains(k)),
        "open says more than the document: {open}"
    );
    let save = &save["result"];
    assert!(
        save.get("simplified").is_none() && save.get("writer_notes").is_none(),
        "{save}"
    );
    assert_eq!(
        attrs["result"]["com.example.cutlist"]["project"],
        Value::String("Kitchen".into()),
        "{attrs}"
    );

    assert_eq!(
        std::fs::read(&saved).unwrap(),
        bytes,
        "the saved file is byte for byte the file that was opened"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
