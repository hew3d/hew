//! Acceptance specs for `Document::ingest` (DEVELOPMENT.md rule 3).
//!
//! Tests are written stub-first per : they were un-ignored once
//! `ingest` was implemented in the same PR.

use kernel::{DefRecipe, Document, ImportNode, ImportScene, MeshRecipe, Point3, Transform};

// ─────────────────────────────────────────────────────────── helpers ─────────

/// Closed axis-aligned box as a `MeshRecipe`.
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
            vec![0, 3, 2, 1], // bottom (−Z)
            vec![4, 5, 6, 7], // top    (+Z)
            vec![0, 1, 5, 4], // front  (−Y)
            vec![1, 2, 6, 5], // right  (+X)
            vec![2, 3, 7, 6], // back   (+Y)
            vec![3, 0, 4, 7], // left   (−X)
        ],
        face_materials: vec![kernel::NO_MATERIAL; 6],
        face_uv_frames: vec![None; 6],
        face_holes: vec![Vec::new(); 6],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
    }
}

/// Open box (missing top face) as a `MeshRecipe`.
fn open_box_recipe(name: &str) -> MeshRecipe {
    let mut r = box_recipe(name);
    r.faces.remove(1); // remove the top face
    r.face_materials.remove(1);
    r.face_holes.remove(1);
    r
}

/// A recipe with a degenerate face (only 2 vertices — will fail `from_polygons`).
fn degenerate_recipe(name: &str) -> MeshRecipe {
    MeshRecipe {
        name: name.to_string(),
        positions: vec![Point3::ORIGIN, Point3::new(1.0, 0.0, 0.0)],
        faces: vec![vec![0, 1]], // degenerate: 2 points
        face_materials: vec![kernel::NO_MATERIAL],
        face_uv_frames: vec![None],
        face_holes: vec![Vec::new()],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
    }
}

fn empty_scene(roots: Vec<ImportNode>) -> ImportScene {
    ImportScene {
        materials: vec![],
        defs: vec![],
        roots,
    }
}

// ─────────────────────────────────────────────── specs ───────────────────────

/// Single closed-box recipe → one watertight object, nothing skipped.
#[test]
fn ingest_box_recipe_creates_one_watertight_object() {
    let mut doc = Document::new();
    let before = doc.visible_object_ids().len();

    let scene = empty_scene(vec![ImportNode::Mesh(box_recipe("box"))]);
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();

    assert_eq!(doc.visible_object_ids().len(), before + 1);
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.watertight, 1);
    assert_eq!(report.leaky, 0);
    assert!(report.skipped.is_empty());
    assert!(report.textures_missing.is_empty());
}

/// Open box recipe → leaky object present, not skipped.
#[test]
fn ingest_open_shell_is_leaky() {
    let mut doc = Document::new();
    let scene = empty_scene(vec![ImportNode::Mesh(open_box_recipe("open"))]);
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();

    assert_eq!(report.leaky, 1);
    assert_eq!(report.watertight, 0);
    assert!(report.skipped.is_empty());
    assert_eq!(doc.visible_object_ids().len(), 1);
}

/// Recipe with one good + one degenerate mesh → Ok, good created, one skipped.
#[test]
fn ingest_degenerate_mesh_is_skipped_not_fatal() {
    let mut doc = Document::new();
    let scene = empty_scene(vec![
        ImportNode::Mesh(box_recipe("good")),
        ImportNode::Mesh(degenerate_recipe("bad")),
    ]);
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();

    assert_eq!(doc.visible_object_ids().len(), 1, "good object created");
    assert_eq!(report.objects_created, 1);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].name, "bad");
}

/// Multi-node recipe → ingest → undo hides all → redo brings all back.
/// Ids are stable across the cycle.
#[test]
fn ingest_is_atomic_undo_hides_all() {
    let mut doc = Document::new();

    // A scene with: one mesh root + one group containing a mesh.
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![
            ImportNode::Mesh(box_recipe("m1")),
            ImportNode::Group {
                name: "g1".to_string(),
                children: vec![ImportNode::Mesh(box_recipe("m2"))],
                tags: Vec::new(),
            },
        ],
    };
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();
    assert_eq!(report.objects_created, 2);
    let obj_ids: Vec<_> = doc.visible_object_ids();
    assert_eq!(obj_ids.len(), 2);
    let grp_ids: Vec<_> = doc.group_ids();
    assert_eq!(grp_ids.len(), 1);

    // Undo → all created nodes hidden.
    doc.undo().expect("undo import");
    assert_eq!(doc.visible_object_ids().len(), 0);
    assert_eq!(doc.group_ids().len(), 0);

    // Redo → all back.
    doc.redo().expect("redo import");
    assert_eq!(doc.visible_object_ids().len(), 2);
    assert_eq!(doc.group_ids().len(), 1);

    // Ids are exactly the same.
    let after_redo: std::collections::HashSet<_> = doc.visible_object_ids().into_iter().collect();
    for &oid in &obj_ids {
        assert!(after_redo.contains(&oid), "same id after redo");
    }
}

/// Pre-populated doc with one object → ingest another → both visible.
#[test]
fn ingest_merges_not_replaces() {
    let mut doc = Document::new();

    // Pre-existing object via extrude.
    let plane = kernel::Plane::from_polygon(&[
        Point3::ORIGIN,
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    let sk = doc.sketch_mut(s).unwrap();
    sk.add_segment(Point3::ORIGIN, Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    sk.add_segment(Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0))
        .unwrap();
    sk.add_segment(Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0))
        .unwrap();
    sk.add_segment(Point3::new(0.0, 1.0, 0.0), Point3::ORIGIN)
        .unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap();

    let before = doc.visible_object_ids().len();
    assert_eq!(before, 1);

    // Ingest one more.
    let scene = empty_scene(vec![ImportNode::Mesh(box_recipe("imported"))]);
    let (report, _) = doc.ingest(scene, vec![]).unwrap();

    assert_eq!(report.objects_created, 1);
    assert_eq!(doc.visible_object_ids().len(), 2);
}

/// Recipe with 2 instances of 1 def → correct component sharing.
#[test]
fn ingest_instance_shares_one_def() {
    let mut doc = Document::new();

    let scene = ImportScene {
        materials: vec![],
        defs: vec![DefRecipe {
            name: None,
            meshes: vec![box_recipe("def_mesh")],
        }],
        roots: vec![
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                tags: Vec::new(),
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::translation(kernel::Vec3::new(2.0, 0.0, 0.0)),
                tags: Vec::new(),
            },
        ],
    };
    let (report, _) = doc.ingest(scene, vec![]).unwrap();

    // The def member itself is created (counts in objects_created).
    assert_eq!(report.objects_created, 1);

    // Two instances placed.
    let inst_ids = doc.instance_ids();
    assert_eq!(inst_ids.len(), 2);

    // Both reference the same component.
    let cid0 = doc.instance_def(inst_ids[0]).unwrap();
    let cid1 = doc.instance_def(inst_ids[1]).unwrap();
    assert_eq!(cid0, cid1, "both instances share one def");

    // Def has exactly one member.
    let members = doc.def_members(cid0).unwrap();
    assert_eq!(members.len(), 1);
}

/// textures_missing is passed through unchanged.
#[test]
fn ingest_passes_through_textures_missing() {
    let mut doc = Document::new();
    let scene = empty_scene(vec![]);
    let missing = vec!["textures/wood.png".to_string(), "uv_map.jpg".to_string()];
    let (report, _) = doc.ingest(scene, missing.clone()).unwrap();
    assert_eq!(report.textures_missing, missing);
}

/// Ingested names land on the document records and survive a save→load round
/// trip (manifest v2). An unnamed instance resolves to its def's name.
#[test]
fn ingest_then_save_load_preserves_names() {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![DefRecipe {
            name: Some("MyComponent".to_string()),
            meshes: vec![box_recipe("def_box")],
        }],
        roots: vec![
            ImportNode::Group {
                name: "MyGroup".to_string(),
                children: vec![ImportNode::Mesh(box_recipe("MyBox"))],
                tags: Vec::new(),
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                tags: Vec::new(),
            },
        ],
    };
    doc.ingest(scene, vec![]).unwrap();

    // Names are present right after ingest.
    let assert_names = |doc: &Document| {
        let group_named = doc
            .group_ids()
            .iter()
            .any(|&g| doc.group_name(g) == Some("MyGroup"));
        assert!(group_named, "a group must be named MyGroup");

        let box_named = doc
            .visible_object_ids()
            .iter()
            .any(|&o| doc.object_name(o) == Some("MyBox"));
        assert!(box_named, "a world object must be named MyBox");

        // The instance is unnamed; its def carries the component name.
        let inst = doc.instance_ids();
        assert_eq!(inst.len(), 1, "one instance");
        assert!(
            doc.instance_name(inst[0]).is_none(),
            "instance itself unnamed"
        );
        let cid = doc.instance_def(inst[0]).unwrap();
        assert_eq!(
            doc.component_name(cid),
            Some("MyComponent"),
            "def carries the component name"
        );
    };
    assert_names(&doc);

    // Round-trip through the native file format.
    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load must succeed");
    assert_names(&loaded);
}
