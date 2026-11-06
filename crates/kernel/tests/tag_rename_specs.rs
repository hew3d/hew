//! Executable specs for `Document::rename_tag` (`DocAction::TagRenamed`):
//! a tag keeps its identity across a rename, nested paths follow, node tag
//! lists are rewritten, collisions refuse, undo/redo round-trip.

use kernel::{Document, DocumentError, EntityRef, NodeId, ObjectId, Plane, Point3, SceneProps};

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

fn p(segs: &[&str]) -> Vec<String> {
    segs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn rename_moves_registry_identity_and_rewrites_nested_node_tags() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 2.0);
    doc.add_node_tag(NodeId::Object(a), p(&["Hardware"]))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), p(&["Hardware", "Screws"]))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), p(&["Oak"])).unwrap();
    // Node-carried tags are implicit; the REGISTRY (with stable ids) holds
    // only tags something registered — hiding, importing, or the API. Both
    // paths here get an entry so the rename has identities to carry.
    doc.set_tag_hidden(p(&["Hardware"]), false);
    doc.set_tag_hidden(p(&["Hardware", "Screws"]), true);
    let sid_root = doc.sid_of(&EntityRef::Tag(p(&["Hardware"]))).unwrap();
    let sid_child = doc
        .sid_of(&EntityRef::Tag(p(&["Hardware", "Screws"])))
        .unwrap();
    // A Scene capturing the hidden child tag references it by sid.
    let scene = doc
        .add_scene(None, SceneProps::ALL, None, None, None)
        .unwrap();

    let depth = doc.undo_depth();
    let change = doc
        .rename_tag(&p(&["Hardware"]), p(&["Fixings"]))
        .expect("rename");
    assert_eq!(change.objects_touched.len(), 2);
    assert_eq!(doc.undo_depth(), depth + 1);

    // Registry moved, identity + flags intact.
    assert!(
        !doc.tag_meta()
            .any(|(path, _)| path == p(&["Hardware"]).as_slice())
    );
    assert_eq!(doc.sid_of(&EntityRef::Tag(p(&["Fixings"]))), Some(sid_root));
    assert_eq!(
        doc.sid_of(&EntityRef::Tag(p(&["Fixings", "Screws"]))),
        Some(sid_child)
    );
    assert!(doc.tag_hidden(&p(&["Fixings", "Screws"])));
    // Nodes rewritten (nested too), untouched tags untouched.
    assert_eq!(doc.node_tags(NodeId::Object(a)), &[p(&["Fixings"])]);
    assert_eq!(
        doc.node_tags(NodeId::Object(b)),
        &[p(&["Fixings", "Screws"]), p(&["Oak"])]
    );
    // The Scene still resolves the (renamed) hidden tag.
    let resolved = doc.resolve_scene(scene).unwrap();
    assert_eq!(
        resolved.hidden_tag_paths,
        Some(vec![p(&["Fixings", "Screws"])])
    );

    // Undo restores everything; redo re-applies.
    doc.undo().unwrap();
    assert_eq!(
        doc.sid_of(&EntityRef::Tag(p(&["Hardware"]))),
        Some(sid_root)
    );
    assert_eq!(
        doc.node_tags(NodeId::Object(b)),
        &[p(&["Hardware", "Screws"]), p(&["Oak"])]
    );
    assert!(doc.tag_hidden(&p(&["Hardware", "Screws"])));
    doc.redo().unwrap();
    assert_eq!(doc.node_tags(NodeId::Object(a)), &[p(&["Fixings"])]);
    assert_eq!(
        doc.sid_of(&EntityRef::Tag(p(&["Fixings", "Screws"]))),
        Some(sid_child)
    );
}

#[test]
fn rename_refuses_collisions_and_invalid_targets_and_noops_the_same_name() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    let b = extrude_box(&mut doc, 2.0);
    doc.add_node_tag(NodeId::Object(a), p(&["Hardware"]))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), p(&["Oak"])).unwrap();
    let depth = doc.undo_depth();

    assert_eq!(
        doc.rename_tag(&p(&["Hardware"]), p(&["Oak"])),
        Err(DocumentError::DuplicateTag)
    );
    assert_eq!(
        doc.rename_tag(&p(&["Hardware"]), p(&["Hardware", "Nested"])),
        Err(DocumentError::InvalidTagPath)
    );
    assert_eq!(
        doc.rename_tag(&p(&["Hardware"]), p(&[])),
        Err(DocumentError::InvalidTagPath)
    );
    assert_eq!(
        doc.rename_tag(&p(&["Hardware"]), p(&["A", " "])),
        Err(DocumentError::InvalidTagPath)
    );
    doc.rename_tag(&p(&["Hardware"]), p(&["Hardware"])).unwrap();
    doc.rename_tag(&p(&["Nope"]), p(&["Whatever"])).unwrap();
    assert_eq!(
        doc.undo_depth(),
        depth,
        "refusals and no-ops record nothing"
    );
    assert_eq!(doc.node_tags(NodeId::Object(a)), &[p(&["Hardware"])]);
}

#[test]
fn rename_survives_save_and_load() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0);
    doc.add_node_tag(NodeId::Object(a), p(&["Hardware"]))
        .unwrap();
    doc.set_tag_hidden(p(&["Hardware"]), true);
    doc.rename_tag(&p(&["Hardware"]), p(&["Fixings"])).unwrap();
    let bytes = doc.save();
    let loaded = Document::load(&bytes).unwrap();
    let paths: Vec<Vec<String>> = loaded.tag_meta().map(|(p, _)| p.to_vec()).collect();
    assert_eq!(paths, vec![p(&["Fixings"])]);
    assert!(loaded.tag_hidden(&p(&["Fixings"])));
    let obj = loaded.visible_object_ids()[0];
    assert_eq!(loaded.node_tags(NodeId::Object(obj)), &[p(&["Fixings"])]);
    assert_eq!(loaded.save(), bytes);
}

/// A guide carries tags as a node does: a rename rewrites them, a delete
/// strips them, undo and redo replay both, and a hidden tag hides the
/// guide from drawing and snapping while a save keeps it.
#[test]
fn rename_and_delete_carry_a_guides_tags_with_undo() {
    use kernel::{ImportGuide, ImportScene, ImportTag, Vec3};
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: vec![
            ImportGuide::Line {
                dash: kernel::GuideDash::default(),
                origin: Point3::ORIGIN,
                direction: Vec3::new(1.0, 0.0, 0.0),
                tags: vec![p(&["Rig"]), p(&["Cameras"])],
                span: kernel::GuideSpan::INFINITE,
            },
            ImportGuide::Point {
                position: Point3::new(1.0, 2.0, 3.0),
                tags: vec![p(&["Cameras"])],
            },
        ],
        tags: vec![
            ImportTag {
                path: p(&["Rig"]),
                hidden: false,
            },
            ImportTag {
                path: p(&["Cameras"]),
                hidden: false,
            },
        ],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    doc.ingest(scene, Vec::new()).unwrap();
    let ids = doc.guide_ids();
    assert_eq!(ids.len(), 2);
    let (line, point) = (ids[0], ids[1]);
    assert_eq!(doc.guide_tags(line), &[p(&["Rig"]), p(&["Cameras"])]);

    // Hidden through a tag: in the document, not shown.
    doc.set_tag_hidden(p(&["Cameras"]), true);
    assert!(!doc.guide_shown(line) && !doc.guide_shown(point));
    assert_eq!(doc.guide_ids().len(), 2, "a save keeps a hidden guide");
    doc.set_tag_hidden(p(&["Cameras"]), false);
    assert!(doc.guide_shown(line));

    let change = doc.rename_tag(&p(&["Cameras"]), p(&["Views"])).unwrap();
    assert_eq!(change.guides_touched.len(), 2);
    assert_eq!(doc.guide_tags(line), &[p(&["Rig"]), p(&["Views"])]);
    assert_eq!(doc.guide_tags(point), &[p(&["Views"])]);
    doc.undo().unwrap();
    assert_eq!(doc.guide_tags(point), &[p(&["Cameras"])]);
    doc.redo().unwrap();
    assert_eq!(doc.guide_tags(point), &[p(&["Views"])]);

    let change = doc.delete_tag(&p(&["Views"])).unwrap();
    assert_eq!(change.guides_touched.len(), 2);
    assert_eq!(doc.guide_tags(line), &[p(&["Rig"])]);
    assert!(doc.guide_tags(point).is_empty());
    doc.undo().unwrap();
    assert_eq!(doc.guide_tags(point), &[p(&["Views"])]);
    doc.redo().unwrap();
    assert!(doc.guide_tags(point).is_empty());

    // The tags survive a save.
    let loaded = Document::load(&doc.save()).unwrap();
    let ids = loaded.guide_ids();
    assert_eq!(loaded.guide_tags(ids[0]), &[p(&["Rig"])]);
}
