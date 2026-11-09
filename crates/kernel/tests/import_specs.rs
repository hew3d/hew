// Test-only: the set below is an order-independent equality assertion on
// visible-object ids, not kernel output, so HashSet is fine here. Suppress the
// workspace clippy.toml ban for this integration-test crate.
#![allow(clippy::disallowed_types)]

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
        extra: Default::default(),
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
        extra: Default::default(),
    }
}

fn empty_scene(roots: Vec<ImportNode>) -> ImportScene {
    ImportScene {
        materials: vec![],
        defs: vec![],
        roots,
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
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

/// `MeshRecipe::check_builds` answers, without a document, exactly what
/// `ingest` does with the mesh: a mesh it builds checks `Ok`, and a mesh it
/// skips checks as the error whose text is the skip's reason — a degenerate
/// face, and a quad too small for its plane to be found.
#[test]
fn check_builds_agrees_with_ingest() {
    let mut tiny = box_recipe("tiny");
    for p in &mut tiny.positions {
        *p = Point3::new(p.x * 1e-7, p.y * 1e-7, p.z * 1e-7);
    }
    for (recipe, builds) in [
        (box_recipe("good"), true),
        (open_box_recipe("open"), true),
        (degenerate_recipe("bad"), false),
        (tiny, false),
    ] {
        let name = recipe.name.clone();
        let check = recipe.check_builds();
        let mut doc = Document::new();
        let (report, _) = doc
            .ingest(empty_scene(vec![ImportNode::Mesh(recipe)]), vec![])
            .unwrap();
        assert_eq!(check.is_ok(), builds, "{name}: {check:?}");
        match check {
            Ok(()) => assert!(report.skipped.is_empty(), "{name}"),
            Err(e) => {
                assert_eq!(report.skipped.len(), 1, "{name}");
                assert_eq!(report.skipped[0].reason, e.to_string(), "{name}");
            }
        }
    }
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
                hidden: false,
                key: None,
            },
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
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
        defs: vec![DefRecipe::from_meshes(None, vec![box_recipe("def_mesh")])],
        roots: vec![
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::translation(kernel::Vec3::new(2.0, 0.0, 0.0)),
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
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
        defs: vec![DefRecipe::from_meshes(
            Some("MyComponent".to_string()),
            vec![box_recipe("def_box")],
        )],
        roots: vec![
            ImportNode::Group {
                name: "MyGroup".to_string(),
                children: vec![ImportNode::Mesh(box_recipe("MyBox"))],
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
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

/// Imported construction guides ride the same atomic undo step as the
/// geometry: one undo hides them, one redo restores the same ids; degenerate
/// guide inputs are skipped and reported, never fixed up (rule 4).
#[test]
fn ingest_guides_are_atomic_with_the_import() {
    use kernel::{ImportGuide, Vec3};
    let mut doc = Document::new();

    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(box_recipe("m1"))],
        guides: vec![
            ImportGuide::Line {
                dash: kernel::GuideDash::default(),
                origin: Point3::ORIGIN,
                direction: Vec3::new(0.0, 0.0, 3.0), // non-unit: ingest normalizes
                tags: Vec::new(),
                span: kernel::GuideSpan::INFINITE,
            },
            ImportGuide::Point {
                position: Point3::new(1.0, 2.0, 3.0),
                tags: Vec::new(),
            },
            ImportGuide::Line {
                dash: kernel::GuideDash::default(),
                origin: Point3::ORIGIN,
                direction: Vec3::new(0.0, 0.0, 0.0), // degenerate: skipped + reported
                tags: Vec::new(),
                span: kernel::GuideSpan::INFINITE,
            },
        ],
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let (report, change) = doc.ingest(scene, vec![]).unwrap();

    let guide_ids = doc.guide_ids();
    assert_eq!(guide_ids.len(), 2, "two valid guides created");
    assert_eq!(change.guides_touched.len(), 2);
    assert_eq!(
        report.skipped.len(),
        1,
        "the zero-direction guide is reported, not repaired"
    );
    assert!(report.skipped[0].reason.contains("guide"));

    // One undo removes geometry AND guides; one redo restores the same ids.
    doc.undo().expect("undo import");
    assert_eq!(doc.visible_object_ids().len(), 0);
    assert_eq!(doc.guide_ids().len(), 0);
    doc.redo().expect("redo import");
    assert_eq!(doc.visible_object_ids().len(), 1);
    assert_eq!(
        doc.guide_ids(),
        guide_ids,
        "guide ids stable across undo/redo"
    );
}

/// An `ImportNode::Instance` carrying its own name (native `.skp`
/// instance names) surfaces it as the instance's display name; `None` still
/// falls back to the definition's name.
#[test]
fn ingest_instance_own_name_wins_over_def_name() {
    let mut doc = Document::new();

    let scene = ImportScene {
        materials: vec![],
        defs: vec![DefRecipe::from_meshes(
            Some("Wall".to_string()),
            vec![box_recipe("def_mesh")],
        )],
        roots: vec![
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: Some("Front Wall".to_string()),
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::translation(kernel::Vec3::new(2.0, 0.0, 0.0)),
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    doc.ingest(scene, vec![]).unwrap();

    let names: Vec<Option<String>> = doc
        .instance_ids()
        .into_iter()
        .map(|iid| doc.instance_name(iid).map(str::to_string))
        .collect();
    assert!(
        names.contains(&Some("Front Wall".to_string())),
        "own instance name stored; got {names:?}"
    );
    assert!(
        names.contains(&None),
        "unnamed instance stays None (UI falls back to the def name); got {names:?}"
    );
}

// ───────────────────────────── nested definitions (docs/design v15) ──────────

/// A definition whose children include an `Instance` of another definition
/// builds a real nested def: the inner instance is definition-owned, the
/// expansion walks through it, and the document round-trips at v15.
#[test]
fn ingest_builds_nested_definitions() {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![
            // Outer FIRST in recipe order but referencing def 1 — build
            // order must not care about recipe order.
            DefRecipe {
                name: Some("Outer".to_string()),
                children: vec![
                    ImportNode::Mesh(box_recipe("outer_body")),
                    ImportNode::Instance {
                        def: 1,
                        pose: Transform::translation(kernel::Vec3::new(2.0, 0.0, 0.0)),
                        name: None,
                        tags: Vec::new(),
                        hidden: false,
                        key: None,
                    },
                ],
            },
            DefRecipe::from_meshes(Some("Inner".to_string()), vec![box_recipe("inner_body")]),
        ],
        roots: vec![ImportNode::Instance {
            def: 0,
            pose: Transform::translation(kernel::Vec3::new(10.0, 0.0, 0.0)),
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        }],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();
    assert_eq!(report.objects_created, 2);
    assert!(report.skipped.is_empty());

    // One world instance; the nested one is definition-owned, not a root.
    assert_eq!(doc.instance_ids().len(), 1);
    let world = doc.instance_ids()[0];
    let placements = doc.expanded_placements();
    assert_eq!(
        placements.len(),
        2,
        "outer body + inner body through nesting"
    );
    assert!(placements.iter().all(|(_, _, outer)| *outer == world));
    // The inner body's composed pose carries both translations.
    assert!(placements.iter().any(|(_, pose, _)| {
        pose.apply_point(Point3::ORIGIN)
            .approx_eq(Point3::new(12.0, 0.0, 0.0), 1e-9)
    }));

    // Round trip: the first organically-built nested document.
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("nested ingest round-trips");
    assert_eq!(re.save(), bytes);
    assert_eq!(re.expanded_placements().len(), 2);
}

/// A cyclic definition recipe is refused typed, before any mutation.
#[test]
fn ingest_refuses_cyclic_definition_recipes() {
    let inst = |def: usize| ImportNode::Instance {
        def,
        pose: Transform::IDENTITY,
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![
            DefRecipe {
                name: None,
                children: vec![ImportNode::Mesh(box_recipe("a")), inst(1)],
            },
            DefRecipe {
                name: None,
                children: vec![ImportNode::Mesh(box_recipe("b")), inst(0)],
            },
        ],
        roots: vec![],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    assert!(matches!(
        doc.ingest(scene, vec![]),
        Err(kernel::DocumentError::ComponentCycle)
    ));
    // Refused before any mutation: the document is untouched.
    assert!(doc.component_ids().is_empty());
    assert_eq!(doc.visible_object_ids().len(), 0);
}

/// A nested instance whose target definition ends up empty (every mesh
/// rejected) is skipped like a world instance would be — never a dangling
/// reference.
#[test]
fn ingest_skips_nested_instances_of_failed_defs() {
    let mut doc = Document::new();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![
            DefRecipe {
                name: Some("Outer".to_string()),
                children: vec![
                    ImportNode::Mesh(box_recipe("outer_body")),
                    ImportNode::Instance {
                        def: 1,
                        pose: Transform::IDENTITY,
                        name: None,
                        tags: Vec::new(),
                        hidden: false,
                        key: None,
                    },
                ],
            },
            DefRecipe::from_meshes(Some("Broken".to_string()), vec![degenerate_recipe("bad")]),
        ],
        roots: vec![ImportNode::Instance {
            def: 0,
            pose: Transform::IDENTITY,
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        }],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let (report, _change) = doc.ingest(scene, vec![]).unwrap();
    assert_eq!(report.skipped.len(), 1);
    // Only Outer survives, with its one real member.
    assert_eq!(doc.component_ids().len(), 1);
    assert_eq!(doc.expanded_placements().len(), 1);
    let bytes = doc.save();
    Document::load(&bytes).expect("still a valid document");
}

/// The width bound at ingest: a recipe DAG that doubles for 21 levels
/// (2^21 placements) refuses typed BEFORE any document mutation.
#[test]
fn ingest_refuses_over_wide_definition_recipes() {
    let inst = |def: usize| ImportNode::Instance {
        def,
        pose: Transform::IDENTITY,
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    // defs[0] is the leaf; defs[k] holds two instances of defs[k-1].
    let mut defs = vec![DefRecipe::from_meshes(None, vec![box_recipe("leaf")])];
    for k in 1..=21 {
        defs.push(DefRecipe {
            name: None,
            children: vec![inst(k - 1), inst(k - 1)],
        });
    }
    let scene = ImportScene {
        materials: vec![],
        defs,
        roots: vec![inst(21)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut doc = Document::new();
    assert!(matches!(
        doc.ingest(scene, vec![]),
        Err(kernel::DocumentError::ComponentExpansionExceeded)
    ));
    assert!(doc.component_ids().is_empty(), "refused before mutation");
}

// ── Scene recipes ────────────────────────────────────────────────────────────

#[test]
fn ingest_creates_scenes_from_recipes_keyed_to_the_nodes_it_makes() {
    use kernel::{
        CameraProjection, CameraState, ImportSceneRecipe, ImportTag, Point3, SectionPlaneState,
        Vec3,
    };
    let camera = |x: f64| {
        CameraState::framed(
            CameraProjection::Perspective,
            35.0,
            Point3::new(x, -5.0, 3.0),
            Point3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        )
    };
    let section = SectionPlaneState {
        origin: Point3::new(0.0, 0.0, 0.5),
        normal: Vec3::new(0.0, 0.0, -1.0),
        active: true,
    };
    let make = || ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![
            ImportNode::Group {
                name: "roof".into(),
                children: vec![ImportNode::Mesh(box_recipe("roof-box"))],
                tags: vec![],
                hidden: false,
                key: Some(7),
            },
            ImportNode::Mesh(box_recipe("floor")),
        ],
        guides: vec![],
        tags: vec![ImportTag {
            path: vec!["Trees".into()],
            hidden: false,
        }],
        camera: Some(camera(10.0)),
        scenes: vec![
            ImportSceneRecipe {
                view_style: None,
                transition_s: None,
                delay_s: None,
                name: "Plan".into(),
                description: "cut".into(),
                camera: Some(camera(1.0)),
                // 7 = the roof group; 99 names nothing Hew makes a node for
                hidden_keys: Some(vec![7, 99]),
                hidden_tags: Some(vec![vec!["Trees".into()], vec!["Missing".into()]]),
                section: Some(Some(section)),
                display: None,
            },
            ImportSceneRecipe {
                view_style: None,
                transition_s: None,
                delay_s: None,
                name: "Plan".into(),
                description: String::new(),
                camera: None,
                hidden_keys: None,
                hidden_tags: None,
                section: None,
                display: None,
            },
        ],
        section: Some(section),
        view_style: None,
        extra: Default::default(),
        geo: None,
    };

    let mut doc = Document::new();
    doc.ingest(make(), vec![]).unwrap();
    // The saved view and its section cut land in the empty document.
    assert!(same_view(doc.camera_state(), camera(10.0)));
    let scenes = doc.scenes();
    assert_eq!(scenes.len(), 2);
    let plan = &scenes[0];
    assert_eq!(plan.name, "Plan");
    assert_eq!(plan.description, "cut");
    assert!(same_view(plan.camera, camera(1.0)));
    assert_eq!(plan.section, Some(Some(section)));
    let hidden = plan.hidden_nodes.as_ref().expect("captured");
    assert_eq!(
        hidden.len(),
        1,
        "key 7 resolved to the roof group; 99 dropped"
    );
    let roof_sid = doc
        .top_level_nodes()
        .into_iter()
        .find_map(|n| match n {
            kernel::NodeId::Group(g) => doc.sid_of(&kernel::EntityRef::Group(g)),
            _ => None,
        })
        .unwrap();
    assert!(hidden.contains(&roof_sid));
    let tags = plan.hidden_tags.as_ref().expect("captured");
    assert_eq!(
        tags.len(),
        1,
        "the declared tag resolved; the unknown path dropped"
    );
    // Duplicate names get a suffix; uncaptured properties stay None.
    let second = &scenes[1];
    assert_eq!(second.name, "Plan (2)");
    assert!(
        second.camera.is_none() && second.hidden_nodes.is_none() && second.hidden_tags.is_none()
    );
    assert!(second.section.is_none());

    // A second import into the now non-empty document keeps the camera.
    doc.ingest(make(), vec![]).unwrap();
    assert!(same_view(doc.camera_state(), camera(10.0)));
    assert_eq!(doc.scenes().len(), 4);
    assert_eq!(doc.scenes()[2].name, "Plan (3)");
}

/// A SketchUp camera's target says only where it looks: the file's may sit
/// a hand's width from the eye or in mid-air. Ingest moves it to where the
/// view line meets the model (the file camera and every scene's), keeping
/// the eye and direction and re-measuring a perspective view's height
/// there; with nothing on the view line it lands at the depth of the
/// model's center, and with no model at all it stays.
#[test]
fn ingest_puts_a_file_cameras_target_on_the_model() {
    use kernel::{CameraProjection, CameraState, ImportSceneRecipe, ImportTag, Point3, Vec3};
    // Looking straight down at the unit box from 10 m up, target 0.2 m away.
    let down = CameraState::framed(
        CameraProjection::Perspective,
        35.0,
        Point3::new(0.5, 0.5, 10.0),
        Point3::new(0.5, 0.5, 9.8),
        Vec3::new(0.0, 1.0, 0.0),
    );
    // Looking past the box: nothing on the line, the center's depth it is.
    let past = CameraState::framed(
        CameraProjection::Parallel,
        35.0,
        Point3::new(-5.0, 0.5, 0.5),
        Point3::new(-4.0, 0.5, 2.5),
        Vec3::new(0.0, 0.0, 1.0),
    );
    let recipe = |cam: CameraState, name: &str| ImportSceneRecipe {
        view_style: None,
        transition_s: None,
        delay_s: None,
        name: name.into(),
        description: String::new(),
        camera: Some(cam),
        hidden_keys: None,
        hidden_tags: None,
        section: None,
        display: None,
    };
    // A second box, hidden by its tag, sits right above the first on the
    // way down: what is not on view is not what the camera looks at.
    let scene = ImportScene {
        materials: vec![],
        defs: vec![kernel::DefRecipe {
            name: Some("hidden box".into()),
            children: vec![ImportNode::Mesh(box_recipe("hidden"))],
        }],
        roots: vec![
            ImportNode::Mesh(box_recipe("box")),
            ImportNode::Instance {
                def: 0,
                pose: kernel::Transform::translation(Vec3::new(0.0, 0.0, 2.0)),
                name: None,
                tags: vec![vec!["Hidden".into()]],
                hidden: false,
                key: None,
            },
        ],
        guides: vec![],
        tags: vec![ImportTag {
            path: vec!["Hidden".into()],
            hidden: true,
        }],
        camera: Some(down),
        scenes: vec![recipe(down, "Down"), recipe(past, "Past")],
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut doc = Document::new();
    doc.ingest(scene, vec![]).unwrap();
    let cam = doc.camera_state().expect("adopted");
    assert!((cam.eye - down.eye).length() < 1e-9, "the eye stays");
    assert!(
        (cam.target.z - 1.0).abs() < 1e-9,
        "on the box's top: {:?}",
        cam.target
    );
    assert!(
        (cam.parallel_height - CameraState::height_at_target(cam.eye, cam.target, 35.0)).abs()
            < 1e-9,
        "the height is re-measured at the new target"
    );
    let scenes = doc.scenes();
    let s_down = scenes
        .iter()
        .find(|s| s.name == "Down")
        .unwrap()
        .camera
        .unwrap();
    assert!((s_down.target.z - 1.0).abs() < 1e-9);
    let s_past = scenes
        .iter()
        .find(|s| s.name == "Past")
        .unwrap()
        .camera
        .unwrap();
    let dir = (past.target - past.eye).normalized().unwrap();
    // (The hidden box is not among the model the center is taken from.)
    let expect = (Point3::new(0.5, 0.5, 0.5) - past.eye).dot(dir);
    assert!(
        ((s_past.target - s_past.eye).length() - expect).abs() < 1e-9,
        "the center's depth: {:?}",
        s_past.target
    );
    assert!(
        (s_past.parallel_height - past.parallel_height).abs() < 1e-12,
        "a parallel view keeps its height"
    );

    // No model: the camera is left as the file has it.
    let empty = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![],
        guides: vec![],
        tags: vec![],
        camera: Some(down),
        scenes: vec![],
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut doc = Document::new();
    doc.ingest(empty, vec![]).unwrap();
    assert_eq!(doc.camera_state().unwrap().target, down.target);
}

/// The view `a` is the file's `b`: same eye, direction, up and lens. (The
/// target's depth is Hew's own — ingest moves it onto the model.)
fn same_view(a: Option<kernel::CameraState>, b: kernel::CameraState) -> bool {
    let Some(a) = a else { return false };
    let da = (a.target - a.eye).normalized().unwrap();
    let db = (b.target - b.eye).normalized().unwrap();
    a.projection == b.projection
        && a.fov_deg == b.fov_deg
        && (a.eye - b.eye).length() < 1e-9
        && (a.up - b.up).length() < 1e-9
        && (da - db).length() < 1e-9
}

// ───────────────────────────────────────────── an import built in steps ──────

/// An import built a few items at a time is the import built at once, and
/// says how far it has come: the definitions first (a definition that
/// places another builds after it), then the top-level nodes. Undo takes
/// the stepped import out whole, as it does the one built at once.
#[test]
fn an_import_built_in_steps_is_the_import_built_at_once() {
    use kernel::{DefRecipe, IngestProgress, Vec3};
    let scene = || ImportScene {
        materials: vec![],
        defs: vec![
            DefRecipe {
                name: Some("table".into()),
                children: vec![
                    ImportNode::Mesh(box_recipe("top")),
                    ImportNode::Instance {
                        def: 1,
                        pose: Transform::translation(Vec3::new(0.0, 0.0, -1.0)),
                        name: None,
                        tags: Vec::new(),
                        hidden: false,
                        key: None,
                    },
                ],
            },
            DefRecipe {
                name: Some("leg".into()),
                children: vec![ImportNode::Mesh(box_recipe("leg"))],
            },
        ],
        roots: vec![
            ImportNode::Mesh(box_recipe("floor")),
            ImportNode::Group {
                name: "shelf".into(),
                children: vec![ImportNode::Mesh(box_recipe("board"))],
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
            ImportNode::Instance {
                def: 0,
                pose: Transform::translation(Vec3::new(3.0, 0.0, 1.0)),
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut at_once = Document::new();
    let (report_once, _) = at_once.ingest(scene(), vec![]).unwrap();

    let mut stepped = Document::new();
    let mut job = stepped.ingest_begin(scene(), vec![]).unwrap();
    assert_eq!(job.progress(), IngestProgress { done: 0, total: 5 });
    assert!(!job.progress().finished());
    let mut seen = Vec::new();
    loop {
        let p = stepped.ingest_step(&mut job, 1);
        seen.push(p.done);
        assert_eq!(p.total, 5);
        if p.finished() {
            break;
        }
    }
    assert_eq!(
        seen,
        vec![1, 2, 3, 4, 5],
        "one item per step, every step counted"
    );
    // A step past the end builds nothing more.
    assert_eq!(
        stepped.ingest_step(&mut job, 1),
        IngestProgress { done: 5, total: 5 }
    );
    let (report_stepped, _) = stepped.ingest_finish(job);

    assert_eq!(report_stepped.objects_created, report_once.objects_created);
    assert_eq!(
        report_stepped.objects_created, 4,
        "floor, board, and the table's own two"
    );
    assert_eq!(stepped.state_hash(), at_once.state_hash());
    stepped.undo().expect("undo the import");
    assert_eq!(stepped.visible_object_ids().len(), 0);
    stepped.redo().expect("redo the import");
    assert_eq!(stepped.state_hash(), at_once.state_hash());
}

/// Finishing an import that was never stepped builds everything first.
#[test]
fn finishing_an_unstepped_import_builds_it_whole() {
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![
            ImportNode::Mesh(box_recipe("a")),
            ImportNode::Mesh(box_recipe("b")),
        ],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut doc = Document::new();
    let job = doc.ingest_begin(scene, vec![]).unwrap();
    let (report, _) = doc.ingest_finish(job);
    assert_eq!(report.objects_created, 2);
    assert_eq!(doc.visible_object_ids().len(), 2);
}
