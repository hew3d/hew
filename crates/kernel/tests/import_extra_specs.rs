//! Acceptance specs for the import extras `Document::ingest` carries beyond
//! the node tree (DEVELOPMENT.md rule 3): source claims about a mesh's
//! edges and faces ([`kernel::MeshExtra`]), drawn sketches, attribute
//! dictionaries, scene display state, and the importer-key report the
//! `.skp` re-save builds its provenance from.

use std::collections::BTreeMap;

use kernel::{
    AttrValue, CurveAnalytic, CurveGeom, DefRecipe, DisplayState, Document, EntityRef, FaceId,
    ImportAttrTarget, ImportNode, ImportScene, ImportSceneRecipe, ImportSketch, ImportTag,
    KernelOp, Material, MaterialScope, MeshExtra, MeshRecipe, NodeId, Plane, Point3, Rgba8,
    SketchCurveKind, SurfaceRef, Transform, Vec3,
};

const N: usize = 12;

fn ring(i: usize, z: f64) -> Point3 {
    let t = std::f64::consts::TAU * i as f64 / N as f64;
    Point3::new(t.cos(), t.sin(), z)
}

/// A closed 12-sided prism of radius 1 and height 1 about the Z axis: the
/// faceted cylinder a `.skp` holds for a drawn circle pushed up.
fn prism(extra: MeshExtra) -> MeshRecipe {
    let mut positions = Vec::new();
    for i in 0..N {
        positions.push(ring(i, 0.0));
    }
    for i in 0..N {
        positions.push(ring(i, 1.0));
    }
    let mut faces: Vec<Vec<usize>> = Vec::new();
    faces.push((0..N).rev().collect()); // bottom, facing −Z
    faces.push((N..2 * N).collect()); // top, facing +Z
    for i in 0..N {
        let j = (i + 1) % N;
        faces.push(vec![i, j, N + j, N + i]);
    }
    let n = faces.len();
    MeshRecipe {
        name: "Post".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra,
    }
}

fn scene(roots: Vec<ImportNode>) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
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

fn axis() -> SurfaceRef {
    SurfaceRef::Cylinder {
        axis_point: Point3::ORIGIN,
        axis: Vec3::new(0.0, 0.0, 1.0),
        radius: 1.0,
    }
}

fn only_object(doc: &Document) -> &kernel::Object {
    let ids = doc.visible_object_ids();
    assert_eq!(ids.len(), 1);
    doc.object(ids[0]).unwrap()
}

#[test]
fn soft_edges_rims_and_surfaces_are_adopted_where_they_hold() {
    let seams: Vec<[Point3; 2]> = (0..N).map(|i| [ring(i, 0.0), ring(i, 1.0)]).collect();
    let mut curves: Vec<([Point3; 2], CurveGeom)> = Vec::new();
    for (z, c) in [(0.0, Point3::ORIGIN), (1.0, Point3::new(0.0, 0.0, 1.0))] {
        for i in 0..N {
            curves.push((
                [ring(i, z), ring((i + 1) % N, z)],
                CurveGeom {
                    center: c,
                    radius: 1.0,
                },
            ));
        }
    }
    let mut doc = Document::new();
    doc.ingest(
        scene(vec![ImportNode::Mesh(prism(MeshExtra {
            key: Some(7),
            soft_edges: seams,
            edge_curves: curves,
            surfaces: vec![axis()],
            hidden: false,
            face_back_materials: Vec::new(),
        }))]),
        Vec::new(),
    )
    .unwrap();
    let obj = only_object(&doc);
    let soft = obj.edges().iter().filter(|(_, e)| e.soft).count();
    let claimed = obj
        .edges()
        .iter()
        .filter(|(_, e)| e.curve.is_some())
        .count();
    let walls = obj
        .faces()
        .iter()
        .filter(|(_, f)| f.surface.is_some())
        .count();
    assert_eq!(
        (soft, claimed, walls),
        (N, 2 * N, N),
        "every seam, rim chord and wall"
    );
}

#[test]
fn a_claim_the_geometry_contradicts_is_dropped_not_forced() {
    let mut doc = Document::new();
    doc.ingest(
        scene(vec![ImportNode::Mesh(prism(MeshExtra {
            key: None,
            // Not an edge of the mesh (a diagonal).
            soft_edges: vec![[ring(0, 0.0), ring(1, 1.0)]],
            // The right chord, the wrong circle.
            edge_curves: vec![(
                [ring(0, 0.0), ring(1, 0.0)],
                CurveGeom {
                    center: Point3::ORIGIN,
                    radius: 2.0,
                },
            )],
            // A cylinder no face lies on.
            surfaces: vec![SurfaceRef::Cylinder {
                axis_point: Point3::ORIGIN,
                axis: Vec3::new(0.0, 0.0, 1.0),
                radius: 3.0,
            }],
            hidden: false,
            face_back_materials: Vec::new(),
        }))]),
        Vec::new(),
    )
    .unwrap();
    let obj = only_object(&doc);
    assert!(
        obj.edges()
            .iter()
            .all(|(_, e)| !e.soft && e.curve.is_none())
    );
    assert!(obj.faces().iter().all(|(_, f)| f.surface.is_none()));
}

#[test]
fn the_report_maps_every_key_and_definition() {
    let mut s = scene(vec![
        ImportNode::Mesh(prism(MeshExtra {
            key: Some(1),
            ..Default::default()
        })),
        ImportNode::Instance {
            def: 0,
            pose: Transform::translation(Vec3::new(5.0, 0.0, 0.0)),
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: Some(2),
        },
    ]);
    s.defs.push(DefRecipe::from_meshes(
        Some("Part".into()),
        vec![prism(MeshExtra {
            key: Some(3),
            ..Default::default()
        })],
    ));
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    assert!(matches!(report.keys[&1][..], [NodeId::Object(_)]));
    assert!(matches!(report.keys[&2][..], [NodeId::Instance(_)]));
    assert!(
        matches!(report.keys[&3][..], [NodeId::Object(_)]),
        "a definition member keyed too"
    );
    let def = report.defs[0].expect("the definition was built");
    assert_eq!(doc.component_name(def), Some("Part"));
}

#[test]
fn sketches_are_built_in_the_import_and_undone_with_it() {
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let square = [
        [Point3::new(3.0, 0.0, 0.0), Point3::new(4.0, 0.0, 0.0)],
        [Point3::new(4.0, 0.0, 0.0), Point3::new(4.0, 1.0, 0.0)],
        [Point3::new(4.0, 1.0, 0.0), Point3::new(3.0, 1.0, 0.0)],
        [Point3::new(3.0, 1.0, 0.0), Point3::new(3.0, 0.0, 0.0)],
    ];
    let circle: Vec<[Point3; 2]> = (0..N)
        .map(|i| {
            let p = |k: usize| {
                let r = ring(k % N, 0.0);
                Point3::new(r.x * 0.5 + 8.0, r.y * 0.5, 0.0)
            };
            [p(i), p(i + 1)]
        })
        .collect();
    let mut segments = square.to_vec();
    segments.extend(circle.iter().copied());
    let mut s = scene(Vec::new());
    s.extra.sketches.push(ImportSketch {
        plane: ground,
        def: None,
        group: None,
        tags: Vec::new(),
        segments,
        curves: vec![(
            (4..4 + N).collect(),
            Some(CurveAnalytic {
                geom: CurveGeom {
                    center: Point3::new(8.0, 0.0, 0.0),
                    radius: 0.5,
                },
                kind: SketchCurveKind::Circle,
            }),
        )],
    });
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let id = report.sketches[0].expect("built");
    let sk = doc.sketch(id).unwrap();
    assert_eq!(sk.regions().len(), 2, "the square and the circle close");
    let chains: std::collections::BTreeSet<_> =
        sk.edges().iter().filter_map(|(_, e)| e.curve).collect();
    assert_eq!(chains.len(), 1, "the circle is one welded chain");
    let c = *chains.iter().next().unwrap();
    assert_eq!(
        sk.curve_analytic(c).map(|a| a.kind),
        Some(SketchCurveKind::Circle)
    );
    doc.undo().unwrap();
    assert!(
        doc.sketch_ids().is_empty(),
        "undoing the import removes its sketches"
    );
    doc.redo().unwrap();
    assert_eq!(doc.sketch_ids().len(), 1);
}

#[test]
fn a_sketch_off_its_plane_is_refused_and_reported() {
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let mut s = scene(Vec::new());
    s.extra.sketches.push(ImportSketch {
        plane: ground,
        def: None,
        group: None,
        tags: Vec::new(),
        segments: vec![[Point3::ORIGIN, Point3::new(1.0, 0.0, 0.5)]],
        curves: Vec::new(),
    });
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    assert_eq!(report.sketches, vec![None]);
    assert_eq!(report.skipped.len(), 1);
    assert!(doc.sketch_ids().is_empty());
}

#[test]
fn attribute_dictionaries_land_on_their_targets() {
    let dict = |ns: &str, v: AttrValue| {
        let mut entries = BTreeMap::new();
        entries.insert("k".to_string(), v);
        let mut d = kernel::AttrDict::new();
        d.insert(ns.to_string(), entries);
        d
    };
    let mut s = scene(vec![ImportNode::Mesh(prism(MeshExtra {
        key: Some(9),
        ..Default::default()
    }))]);
    s.materials
        .push(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
    s.tags.push(ImportTag {
        path: vec!["Walls".into()],
        hidden: false,
    });
    s.extra.attrs = vec![
        (
            ImportAttrTarget::Document,
            dict("project", AttrValue::Text("x".into())),
        ),
        (ImportAttrTarget::Node(9), dict("shop", AttrValue::Int(3))),
        (
            ImportAttrTarget::Material(0),
            dict("mat", AttrValue::Bool(true)),
        ),
        (
            ImportAttrTarget::Tag(vec!["Walls".into()]),
            dict("tag", AttrValue::Float(1.5)),
        ),
        // A key no node carries, and a value Hew refuses: neither lands.
        (ImportAttrTarget::Node(99), dict("ghost", AttrValue::Null)),
        (
            ImportAttrTarget::Node(9),
            dict("bad", AttrValue::Float(f64::NAN)),
        ),
    ];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let get = |t: kernel::AttrTarget, ns: &str| {
        doc.attr_get(&t)
            .unwrap()
            .and_then(|d| d.get(ns))
            .and_then(|e| e.get("k"))
            .cloned()
    };
    assert_eq!(
        get(kernel::AttrTarget::Document, "project"),
        Some(AttrValue::Text("x".into()))
    );
    let NodeId::Object(o) = report.keys[&9][0] else {
        panic!()
    };
    let obj = kernel::AttrTarget::Entity(EntityRef::Object(o));
    assert_eq!(get(obj.clone(), "shop"), Some(AttrValue::Int(3)));
    assert_eq!(get(obj, "bad"), None);
    let mat = doc.materials().iter().next().unwrap().0;
    assert_eq!(
        get(kernel::AttrTarget::Entity(EntityRef::Material(mat)), "mat"),
        Some(AttrValue::Bool(true))
    );
    assert_eq!(
        get(
            kernel::AttrTarget::Entity(EntityRef::Tag(vec!["Walls".into()])),
            "tag"
        ),
        Some(AttrValue::Float(1.5))
    );
    assert!(report.skipped.iter().any(|s| s.name.contains("bad")));
}

#[test]
fn a_scene_keeps_its_display_toggles() {
    let mut s = scene(vec![ImportNode::Mesh(prism(MeshExtra::default()))]);
    let display = DisplayState {
        grid: false,
        axes: true,
        guides: false,
    };
    s.scenes.push(ImportSceneRecipe {
        view_style: None,
        transition_s: None,
        delay_s: None,
        name: "Plan".into(),
        description: String::new(),
        camera: None,
        hidden_keys: None,
        hidden_tags: None,
        section: None,
        display: Some(display),
    });
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    assert_eq!(doc.scenes()[0].display, Some(display));
}

#[test]
fn forgetting_history_after_an_open_leaves_nothing_to_undo() {
    let mut doc = Document::new();
    doc.ingest(
        scene(vec![ImportNode::Mesh(prism(MeshExtra::default()))]),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(doc.undo_depth(), 1);
    doc.forget_history().unwrap();
    assert_eq!((doc.undo_depth(), doc.redo_depth()), (0, 0));
    assert!(doc.at_saved_mark());
    assert!(doc.undo().is_err() || doc.visible_object_ids().len() == 1);
    assert_eq!(
        doc.visible_object_ids().len(),
        1,
        "the opened content stays"
    );
}

#[test]
fn annotations_are_built_in_the_import_and_undone_with_it() {
    use kernel::{Anchor, Annotation};
    let free = |x: f64| Anchor {
        node: None,
        point: Point3::new(x, 0.0, 0.0),
    };
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let mut s = scene(Vec::new());
    s.extra.annotations = vec![
        Annotation::LinearDimension {
            a: free(0.0),
            b: free(1.0),
            offset: Vec3::new(0.0, -0.2, 0.0),
            plane: ground,
            text_override: None,
        },
        Annotation::LeaderText {
            anchor: free(2.0),
            offset: Vec3::new(0.1, 0.1, 0.0),
            text: "Note".into(),
        },
        // Zero length: refused and reported.
        Annotation::LinearDimension {
            a: free(3.0),
            b: free(3.0),
            offset: Vec3::new(0.0, 0.1, 0.0),
            plane: ground,
            text_override: None,
        },
    ];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    assert_eq!(doc.annotation_ids().len(), 2);
    assert_eq!(report.skipped.len(), 1);
    doc.undo().unwrap();
    assert!(doc.annotation_ids().is_empty());
    doc.redo().unwrap();
    assert_eq!(doc.annotation_ids().len(), 2);
}

#[test]
fn undoing_an_import_puts_its_attributes_back_as_they_were() {
    use kernel::AttrTarget;
    let dict = |ns: &str, v: i64| {
        let mut entries = BTreeMap::new();
        entries.insert("k".to_string(), AttrValue::Int(v));
        let mut d = kernel::AttrDict::new();
        d.insert(ns.to_string(), entries);
        d
    };
    let mut doc = Document::new();
    // The document already holds a namespace the import overwrites.
    let mut s0 = scene(Vec::new());
    s0.extra.attrs = vec![(ImportAttrTarget::Document, dict("project", 1))];
    doc.ingest(s0, Vec::new()).unwrap();
    let mut s = scene(vec![ImportNode::Mesh(prism(MeshExtra::default()))]);
    s.tags.push(ImportTag {
        path: vec!["Walls".into()],
        hidden: false,
    });
    s.extra.attrs = vec![
        (ImportAttrTarget::Document, dict("project", 2)),
        (ImportAttrTarget::Document, dict("other", 3)),
        (ImportAttrTarget::Tag(vec!["Walls".into()]), dict("tag", 4)),
    ];
    doc.ingest(s, Vec::new()).unwrap();
    let get = |doc: &Document, t: AttrTarget, ns: &str| {
        doc.attr_get(&t)
            .ok()
            .flatten()
            .and_then(|d| d.get(ns))
            .and_then(|e| e.get("k"))
            .cloned()
    };
    assert_eq!(
        get(&doc, AttrTarget::Document, "project"),
        Some(AttrValue::Int(2))
    );
    doc.undo().unwrap();
    assert_eq!(
        get(&doc, AttrTarget::Document, "project"),
        Some(AttrValue::Int(1)),
        "the prior value is back"
    );
    assert_eq!(get(&doc, AttrTarget::Document, "other"), None);
    // A tag of that name made later starts clean.
    doc.set_tag_hidden(vec!["Walls".into()], false);
    assert_eq!(
        get(
            &doc,
            AttrTarget::Entity(EntityRef::Tag(vec!["Walls".into()])),
            "tag"
        ),
        None
    );
}

#[test]
fn a_hidden_mesh_imports_as_a_hidden_object() {
    let mut doc = Document::new();
    doc.ingest(
        scene(vec![ImportNode::Mesh(prism(MeshExtra {
            hidden: true,
            ..Default::default()
        }))]),
        Vec::new(),
    )
    .unwrap();
    let o = doc.visible_object_ids()[0];
    assert!(doc.node_user_hidden(NodeId::Object(o)));
}

/// A unit box whose top is two triangles split along the diagonal (4)-(6).
fn split_top_box(extra: MeshExtra) -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    let faces: Vec<Vec<usize>> = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6],
        vec![4, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces.len();
    MeshRecipe {
        name: "Box".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra,
    }
}

#[test]
fn a_soft_split_of_a_flat_face_is_not_drawing() {
    // The same split, drawn: the top is a drawing plane, its diagonal a
    // drawing edge.
    let mut drawn = Document::new();
    drawn
        .ingest(
            scene(vec![ImportNode::Mesh(split_top_box(MeshExtra::default()))]),
            Vec::new(),
        )
        .unwrap();
    let o = drawn.live_object_ids()[0];
    assert_eq!(drawn.object_drawing_planes(o).len(), 1, "a drawn split");

    // Soft in the source (a terrain's flat triangles, a hidden seam): part
    // of the solid like a paint boundary, so no drawing plane, and the
    // top's drawing holds no edge.
    let mut soft = Document::new();
    soft.ingest(
        scene(vec![ImportNode::Mesh(split_top_box(MeshExtra {
            soft_edges: vec![[Point3::new(0.0, 0.0, 1.0), Point3::new(1.0, 1.0, 1.0)]],
            ..Default::default()
        }))]),
        Vec::new(),
    )
    .unwrap();
    let o = soft.live_object_ids()[0];
    assert!(soft.object_drawing_planes(o).is_empty());
    let top =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let units = soft.plane_units(top, None, &[o]).unwrap();
    assert!(units.units.is_empty(), "{:?}", units.units);
    assert_eq!(
        units.regions.len(),
        2,
        "the two triangles still bound regions"
    );
}

#[test]
fn a_hidden_member_is_hidden_in_every_placement() {
    // "Frame": a post. "Ceiling": a panel on the hidden tag "Ceiling" and a
    // nested Frame placed hidden, plus a plain post. Placed twice.
    let mesh = |name: &str, tags: Vec<Vec<String>>| {
        let mut m = prism(MeshExtra::default());
        m.name = name.into();
        m.tags = tags;
        ImportNode::Mesh(m)
    };
    let mut s = scene(vec![]);
    s.tags = vec![ImportTag {
        path: vec!["Ceiling".into()],
        hidden: true,
    }];
    s.defs = vec![
        DefRecipe {
            name: Some("Frame".into()),
            children: vec![mesh("post", Vec::new())],
        },
        DefRecipe {
            name: Some("Ceiling".into()),
            children: vec![
                mesh("panel", vec![vec!["Ceiling".into()]]),
                ImportNode::Instance {
                    def: 0,
                    pose: Transform::IDENTITY,
                    name: None,
                    tags: Vec::new(),
                    hidden: true,
                    key: None,
                },
                mesh("plain", Vec::new()),
            ],
        },
    ];
    let place = |x: f64| ImportNode::Instance {
        def: 1,
        pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    s.roots = vec![place(0.0), place(5.0)];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let ceiling = doc
        .component_ids()
        .into_iter()
        .find(|&c| doc.component_name(c) == Some("Ceiling"))
        .unwrap();
    let names = |doc: &Document| -> Vec<(String, bool)> {
        doc.expanded_def_placements(ceiling)
            .iter()
            .zip(doc.expanded_def_placements_shown(ceiling))
            .map(|((o, _), shown)| (doc.object_name(*o).unwrap_or("").to_string(), shown))
            .collect()
    };
    let mut got = names(&doc);
    got.sort();
    assert_eq!(
        got,
        [
            ("panel".to_string(), false),
            ("plain".to_string(), true),
            ("post".to_string(), false),
        ]
    );
    // Showing the tag shows the panel; hiding the plain post hides it.
    doc.set_tag_hidden(vec!["Ceiling".into()], false);
    let plain = doc
        .def_member_nodes(ceiling)
        .unwrap()
        .into_iter()
        .find(|n| matches!(n, NodeId::Object(o) if doc.object_name(*o) == Some("plain")))
        .unwrap();
    doc.set_node_user_hidden(plain, true);
    let mut got = names(&doc);
    got.sort();
    assert_eq!(
        got,
        [
            ("panel".to_string(), true),
            ("plain".to_string(), false),
            ("post".to_string(), false),
        ]
    );
}

/// Replace Material repaints a face's back too. Undoing it after a split
/// of that face and the split's undo (which rebuilds the face under a new
/// handle) finds the face by its geometry and restores the back; redo
/// repaints it again.
#[test]
fn replace_material_undo_restores_a_back_on_a_face_a_split_undo_re_minted() {
    let no = kernel::NO_MATERIAL;
    let recipe = split_top_box(MeshExtra {
        face_back_materials: vec![0, no, no, no, no, no, no],
        ..Default::default()
    });
    let mut s = scene(vec![ImportNode::Mesh(recipe)]);
    s.materials = vec![Material::solid("Blue", Rgba8::rgba(0, 0, 255, 255))];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let oid = doc.visible_object_ids()[0];
    let bottom = |doc: &Document| {
        let faces: Vec<FaceId> = doc
            .object(oid)
            .unwrap()
            .faces()
            .iter()
            .filter(|(_, f)| f.plane.normal().z < -1.0 + 1e-9)
            .map(|(id, _)| id)
            .collect();
        assert_eq!(faces.len(), 1);
        faces[0]
    };
    let back = |doc: &Document| doc.object(oid).unwrap().faces()[bottom(doc)].back_material;
    let blue = back(&doc).expect("the import painted the bottom's back");
    let green = doc.add_material(Material::solid("Green", Rgba8::rgba(0, 255, 0, 255)));

    doc.replace_material(MaterialScope::Document, Some(blue), Some(green))
        .unwrap();
    assert_eq!(back(&doc), Some(green));
    let f = bottom(&doc);
    doc.apply_object_op(
        oid,
        KernelOp::SplitFace {
            face: f,
            path: vec![Point3::new(0.5, 0.0, 0.0), Point3::new(0.5, 1.0, 0.0)],
            restore: None,
            curves: Vec::new(),
        },
    )
    .expect("split the bottom");
    doc.undo().expect("undo the split");
    assert_ne!(
        bottom(&doc),
        f,
        "the merge rebuilt the bottom under a fresh id"
    );

    doc.undo()
        .expect("the replace undoes onto the rebuilt bottom");
    assert_eq!(back(&doc), Some(blue));
    doc.redo().expect("and redoes onto it");
    assert_eq!(back(&doc), Some(green));
}

#[test]
fn a_back_painted_face_keeps_its_back_through_save_and_load() {
    // The split-top box with its first top triangle painted red in front
    // and blue behind, the second blue behind only.
    let mut recipe = split_top_box(MeshExtra {
        face_back_materials: vec![
            kernel::NO_MATERIAL,
            1,
            1,
            kernel::NO_MATERIAL,
            kernel::NO_MATERIAL,
            kernel::NO_MATERIAL,
            kernel::NO_MATERIAL,
        ],
        ..Default::default()
    });
    recipe.face_materials[1] = 0;
    let mut s = scene(vec![ImportNode::Mesh(recipe)]);
    s.materials = vec![
        Material::solid("Red", Rgba8::rgba(255, 0, 0, 255)),
        Material::solid("Blue", Rgba8::rgba(0, 0, 255, 255)),
    ];
    let mut doc = Document::new();
    doc.ingest(s, Vec::new()).unwrap();
    let paints = |doc: &Document| -> Vec<(Option<String>, Option<String>)> {
        let obj = only_object(doc);
        let name =
            |m: Option<kernel::MaterialId>| m.and_then(|m| doc.material(m)).map(|m| m.name.clone());
        let mut v: Vec<_> = obj
            .faces()
            .values()
            .map(|f| (name(f.material), name(f.back_material)))
            .collect();
        v.sort();
        v
    };
    let painted = paints(&doc);
    assert!(
        painted.contains(&(Some("Red".into()), Some("Blue".into()))),
        "{painted:?}"
    );
    assert!(
        painted.contains(&(None, Some("Blue".into()))),
        "{painted:?}"
    );
    let loaded = Document::load(&doc.save()).unwrap();
    assert_eq!(paints(&loaded), painted);

    // Deleting the back's material clears it, and undo brings it back.
    let blue = doc
        .material_ids()
        .into_iter()
        .find(|&m| doc.material(m).is_some_and(|m| m.name == "Blue"))
        .unwrap();
    assert_eq!(doc.material_usage(blue), 2);
    assert!(!doc.unused_materials().contains(&blue));
    doc.delete_material(blue).unwrap();
    assert!(paints(&doc).iter().all(|(_, b)| b.is_none()));
    doc.undo().unwrap();
    assert_eq!(paints(&doc), painted);
}

#[test]
fn a_tagged_annotation_hides_with_its_tag_and_keeps_it_through_save() {
    let dim = |x: f64| kernel::Annotation::LeaderText {
        anchor: kernel::Anchor {
            node: None,
            point: Point3::new(x, 0.0, 0.0),
        },
        offset: Vec3::new(0.0, 0.0, 1.0),
        text: format!("at {x}"),
    };
    let mut s = scene(vec![]);
    s.tags = vec![ImportTag {
        path: vec!["dimensions".into(), "footings".into()],
        hidden: true,
    }];
    s.extra.annotations = vec![dim(0.0), dim(1.0)];
    s.extra.annotation_tags = vec![vec![vec!["dimensions".into(), "footings".into()]]];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    assert_eq!(report.annotations.len(), 2);
    let ids: Vec<kernel::AnnotationId> = report.annotations.iter().map(|a| a.unwrap()).collect();
    assert!(!doc.annotation_shown(ids[0]), "on a hidden tag");
    assert!(doc.annotation_shown(ids[1]), "untagged");
    // A hidden parent tag hides it too.
    doc.set_tag_hidden(vec!["dimensions".into(), "footings".into()], false);
    assert!(doc.annotation_shown(ids[0]));
    doc.set_tag_hidden(vec!["dimensions".into()], true);
    assert!(!doc.annotation_shown(ids[0]));

    let loaded = Document::load(&doc.save()).unwrap();
    let tagged: Vec<_> = loaded
        .annotation_ids()
        .into_iter()
        .map(|a| loaded.annotation_tags(a).to_vec())
        .collect();
    assert!(tagged.contains(&vec![vec![
        "dimensions".to_string(),
        "footings".to_string()
    ]]));
    assert!(tagged.contains(&Vec::new()));
}

/// Three unit squares in a row on the ground, each painted Blue behind only.
fn three_squares() -> ImportScene {
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let positions = vec![
        p(0.0, 0.0),
        p(1.0, 0.0),
        p(2.0, 0.0),
        p(3.0, 0.0),
        p(0.0, 1.0),
        p(1.0, 1.0),
        p(2.0, 1.0),
        p(3.0, 1.0),
    ];
    let faces: Vec<Vec<usize>> = vec![vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6]];
    let mut s = scene(vec![ImportNode::Mesh(MeshRecipe {
        name: "Strip".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; 3],
        face_uv_frames: vec![None; 3],
        face_holes: vec![Vec::new(); 3],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: MeshExtra {
            face_back_materials: vec![0, 0, 0],
            ..Default::default()
        },
    })]);
    s.materials = vec![Material::solid("Blue", Rgba8::rgba(0, 0, 255, 255))];
    s
}

fn backs(doc: &Document, o: kernel::ObjectId) -> Vec<Option<String>> {
    doc.object(o)
        .unwrap()
        .faces()
        .values()
        .map(|f| {
            f.back_material
                .and_then(|m| doc.material(m))
                .map(|m| m.name.clone())
        })
        .collect()
}

#[test]
fn back_paint_survives_a_library_copy_and_a_split() {
    // A library copy resolves every material a face uses, its back too.
    let mut doc = Document::new();
    doc.ingest(three_squares(), Vec::new()).unwrap();
    let o = doc.live_object_ids()[0];
    let item = doc.extract_item(&[NodeId::Object(o)], false).unwrap();
    let io = item.live_object_ids()[0];
    assert!(
        backs(&item, io)
            .iter()
            .all(|b| b.as_deref() == Some("Blue"))
    );

    // Deleting the middle square splits the strip in two; both pieces keep
    // their backs.
    let middle = doc
        .object(o)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| {
            let c: Vec<Point3> = doc
                .object(o)
                .unwrap()
                .loop_positions(f.outer_loop)
                .collect();
            c.iter().all(|p| (1.0..=2.0).contains(&p.x))
        })
        .map(|(id, _)| id)
        .unwrap();
    let (split_off, _) = doc.delete_faces(o, &[middle]).unwrap();
    assert_eq!(split_off.len(), 1);
    for piece in [o, split_off[0]] {
        assert_eq!(backs(&doc, piece), vec![Some("Blue".to_string())]);
    }
}

#[test]
fn renaming_or_deleting_a_tag_carries_its_annotations() {
    let text = kernel::Annotation::LeaderText {
        anchor: kernel::Anchor {
            node: None,
            point: Point3::ORIGIN,
        },
        offset: Vec3::new(0.0, 0.0, 1.0),
        text: "dim".into(),
    };
    let mut s = scene(vec![]);
    s.tags = vec![ImportTag {
        path: vec!["Dims".into()],
        hidden: true,
    }];
    s.extra.annotations = vec![text];
    s.extra.annotation_tags = vec![vec![vec!["Dims".into()]]];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let a = report.annotations[0].unwrap();
    assert!(!doc.annotation_shown(a));

    // Renamed: the annotation follows, still hidden; undo moves it back.
    doc.rename_tag(&["Dims".to_string()], vec!["Notes".into()])
        .unwrap();
    assert_eq!(doc.annotation_tags(a), [vec!["Notes".to_string()]]);
    assert!(!doc.annotation_shown(a));
    doc.undo().unwrap();
    assert_eq!(doc.annotation_tags(a), [vec!["Dims".to_string()]]);

    // Deleted: the annotation loses the tag (and shows); undo restores it.
    doc.delete_tag(&["Dims".to_string()]).unwrap();
    assert!(doc.annotation_tags(a).is_empty());
    assert!(doc.annotation_shown(a));
    doc.undo().unwrap();
    assert_eq!(doc.annotation_tags(a), [vec!["Dims".to_string()]]);
    assert!(!doc.annotation_shown(a));
    doc.redo().unwrap();
    assert!(doc.annotation_tags(a).is_empty());
}

/// A tagged drafting sketch (a `.skp` edge's layer): hidden with its tag or
/// a tag above it, kept through save and load, and carried by a tag rename
/// or delete the way a node's tags are — with undo and redo.
#[test]
fn a_tagged_sketch_hides_with_its_tag_and_follows_tag_renames() {
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let line = |x: f64| ImportSketch {
        plane: ground,
        def: None,
        group: None,
        segments: vec![[Point3::new(x, 0.0, 0.0), Point3::new(x, 1.0, 0.0)]],
        curves: Vec::new(),
        tags: if x > 0.0 {
            vec![vec!["dimensions".into(), "site".into()]]
        } else {
            Vec::new()
        },
    };
    let mut s = scene(vec![]);
    s.tags = vec![ImportTag {
        path: vec!["dimensions".into(), "site".into()],
        hidden: true,
    }];
    s.extra.sketches = vec![line(0.0), line(1.0)];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let ids: Vec<kernel::SketchId> = report.sketches.iter().map(|s| s.unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(doc.sketch_tags(ids[0]).is_empty());
    assert_eq!(
        doc.sketch_tags(ids[1]),
        [vec!["dimensions".to_string(), "site".to_string()]]
    );
    assert!(doc.sketch_shown(ids[0]), "untagged");
    assert!(!doc.sketch_shown(ids[1]), "on a hidden tag");
    assert_eq!(doc.visible_sketch_ids(), vec![ids[0]]);
    // A hidden parent tag hides it too.
    doc.set_tag_hidden(vec!["dimensions".into(), "site".into()], false);
    assert!(doc.sketch_shown(ids[1]));
    doc.set_tag_hidden(vec!["dimensions".into()], true);
    assert!(!doc.sketch_shown(ids[1]));
    doc.set_tag_hidden(vec!["dimensions".into()], false);

    // Through save and load.
    let loaded = Document::load(&doc.save()).unwrap();
    let mut tagged: Vec<Vec<Vec<String>>> = loaded
        .sketch_ids()
        .into_iter()
        .map(|s| loaded.sketch_tags(s).to_vec())
        .collect();
    tagged.sort();
    assert_eq!(
        tagged,
        vec![
            Vec::new(),
            vec![vec!["dimensions".to_string(), "site".to_string()]]
        ]
    );

    // Renamed: the sketch follows; undo moves it back.
    let change = doc
        .rename_tag(&["dimensions".to_string()], vec!["dims".into()])
        .unwrap();
    assert_eq!(change.sketches_touched, vec![ids[1]]);
    assert_eq!(
        doc.sketch_tags(ids[1]),
        [vec!["dims".to_string(), "site".to_string()]]
    );
    doc.undo().unwrap();
    assert_eq!(
        doc.sketch_tags(ids[1]),
        [vec!["dimensions".to_string(), "site".to_string()]]
    );

    // Deleted: the sketch loses the tag (and shows while the tag is
    // hidden); undo restores it, redo strips it again.
    doc.set_tag_hidden(vec!["dimensions".into(), "site".into()], true);
    assert!(!doc.sketch_shown(ids[1]));
    let change = doc
        .delete_tag(&["dimensions".to_string(), "site".to_string()])
        .unwrap();
    assert_eq!(change.sketches_touched, vec![ids[1]]);
    assert!(doc.sketch_tags(ids[1]).is_empty());
    assert!(doc.sketch_shown(ids[1]));
    doc.undo().unwrap();
    assert_eq!(
        doc.sketch_tags(ids[1]),
        [vec!["dimensions".to_string(), "site".to_string()]]
    );
    assert!(!doc.sketch_shown(ids[1]));
    doc.redo().unwrap();
    assert!(doc.sketch_tags(ids[1]).is_empty());
}

/// A definition of drawing alone (a wire-frame component, a light rig of
/// edges) has no mesh to build and is a definition all the same: its
/// instance is placed, its sketches belong to it, a loop its edges close
/// is a loop with no face, and closing an edit of it with its lines still
/// there keeps it — as a group of lines alone is a group.
#[test]
fn a_definition_of_drawing_alone_is_placed_with_its_sketches() {
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let mut s = scene(vec![ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(5.0, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    }]);
    s.defs = vec![DefRecipe {
        name: Some("Bare Wire Component".into()),
        children: Vec::new(),
    }];
    s.extra.sketches = vec![ImportSketch {
        plane: ground,
        def: Some(0),
        group: None,
        segments: vec![
            [Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)],
            [Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)],
            [Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)],
            [Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)],
        ],
        curves: Vec::new(),
        tags: Vec::new(),
    }];
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let cid = report.defs[0].expect("the definition survives");
    assert_eq!(doc.instance_ids().len(), 1, "its instance is placed");
    let sk = report.sketches[0].expect("the sketch is built");
    assert_eq!(doc.sketch_owner_component(sk), Some(cid));
    assert_eq!(
        doc.component_ids(),
        vec![cid],
        "an empty definition would have been removed"
    );
    let sketch = doc.sketch(sk).expect("live");
    assert_eq!(sketch.regions().len(), 1);
    assert!(
        sketch.regions().values().all(|r| r.faceless),
        "a loop of bare edges has no face"
    );
    assert!(
        doc.extrudable_regions(sk).unwrap().is_empty(),
        "nothing to extrude"
    );
    // The state survives a save.
    let loaded = Document::load(&doc.save()).unwrap();
    let cid2 = loaded.component_ids()[0];
    let owned = loaded.def_member_sketches(cid2).unwrap();
    assert_eq!(owned.len(), 1);
    assert!(
        loaded
            .sketch(owned[0])
            .unwrap()
            .regions()
            .values()
            .all(|r| r.faceless)
    );
}
