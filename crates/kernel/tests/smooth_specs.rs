//! WP19: the Smooth level, creases and Bake through the Document.

use crate::common;

use common::curves::p3;
use kernel::{
    BooleanOp, Document, DocumentError, EdgeSmoothness, KernelOp, ObjectId, Plane, ShapeError,
    Transform, Vec3, WatertightState,
};

fn ground() -> Plane {
    Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

fn doc_box(doc: &mut Document, x0: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(x0, 0.0), (x0 + 1.0, 0.0), (x0 + 1.0, 1.0), (x0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

fn top(doc: &Document, o: ObjectId) -> kernel::FaceId {
    doc.object(o)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .unwrap()
}

fn bottom_edges(doc: &Document, o: ObjectId) -> Vec<kernel::EdgeId> {
    let obj = doc.object(o).unwrap();
    obj.edges()
        .keys()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            p.z.abs() < 1e-12 && q.z.abs() < 1e-12
        })
        .collect()
}

#[test]
fn level_set_read_and_cleared_with_undo() {
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    assert_eq!(doc.smooth_level(o), 0);
    let h0 = doc.state_hash();
    let change = doc.set_smooth_level(o, 2).unwrap();
    assert!(change.objects_touched.contains(&o), "the display changes");
    assert_eq!(doc.smooth_level(o), 2);
    assert!(doc.has_smoothed_objects());
    doc.set_smooth_level(o, 0).unwrap();
    assert_eq!(doc.smooth_level(o), 0);
    let undo = doc.undo().unwrap();
    assert!(
        undo.objects_touched.contains(&o),
        "undo refreshes the mesh too"
    );
    assert_eq!(doc.smooth_level(o), 2);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h0);
    assert_eq!(
        doc.set_smooth_level(o, 4).err(),
        Some(DocumentError::Shape(ShapeError::AmountOutOfRange))
    );
}

#[test]
fn a_cage_too_dense_for_a_level_refuses_typed_and_names_the_counts() {
    // An imported slab whose top and bottom are 180 × 180 grids: 64,800
    // quads plus the sides, past the 4,000,000-face surface cap at level 3
    // (× 64) and under it at level 2 (× 16).
    use kernel::{DefRecipe, ImportNode, ImportScene, MeshRecipe, Point3};
    let n = 180usize;
    let step = 1.0 / n as f64;
    let mut positions: Vec<Point3> = Vec::new();
    let mut index = std::collections::BTreeMap::new();
    let mut vid = |p: Point3, positions: &mut Vec<Point3>| -> usize {
        let key = (
            (p.x * 1e6).round() as i64,
            (p.y * 1e6).round() as i64,
            (p.z * 1e6).round() as i64,
        );
        *index.entry(key).or_insert_with(|| {
            positions.push(p);
            positions.len() - 1
        })
    };
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut quad = |a: Point3, b: Point3, c: Point3, d: Point3, positions: &mut Vec<Point3>| {
        let f = vec![
            vid(a, positions),
            vid(b, positions),
            vid(c, positions),
            vid(d, positions),
        ];
        faces.push(f);
    };
    for i in 0..n {
        for j in 0..n {
            let (x0, y0) = (i as f64 * step, j as f64 * step);
            let (x1, y1) = (x0 + step, y0 + step);
            quad(
                p3(x0, y0, 0.1),
                p3(x1, y0, 0.1),
                p3(x1, y1, 0.1),
                p3(x0, y1, 0.1),
                &mut positions,
            );
            quad(
                p3(x0, y0, 0.0),
                p3(x0, y1, 0.0),
                p3(x1, y1, 0.0),
                p3(x1, y0, 0.0),
                &mut positions,
            );
        }
    }
    for i in 0..n {
        let (a, b) = (i as f64 * step, (i + 1) as f64 * step);
        quad(
            p3(a, 0.0, 0.0),
            p3(b, 0.0, 0.0),
            p3(b, 0.0, 0.1),
            p3(a, 0.0, 0.1),
            &mut positions,
        );
        quad(
            p3(b, 1.0, 0.0),
            p3(a, 1.0, 0.0),
            p3(a, 1.0, 0.1),
            p3(b, 1.0, 0.1),
            &mut positions,
        );
        quad(
            p3(0.0, b, 0.0),
            p3(0.0, a, 0.0),
            p3(0.0, a, 0.1),
            p3(0.0, b, 0.1),
            &mut positions,
        );
        quad(
            p3(1.0, a, 0.0),
            p3(1.0, b, 0.0),
            p3(1.0, b, 0.1),
            p3(1.0, a, 0.1),
            &mut positions,
        );
    }
    let count = faces.len();
    let mesh = MeshRecipe {
        name: "slab".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; count],
        face_uv_frames: vec![None; count],
        face_holes: vec![Vec::new(); count],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    let scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::<DefRecipe>::new(),
        roots: vec![ImportNode::Mesh(mesh)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        geo: None,
        extra: Default::default(),
    };
    let mut doc = Document::new();
    doc.ingest(scene, Vec::new()).unwrap();
    let o = doc.visible_object_ids()[0];
    let h0 = doc.state_hash();
    match doc.set_smooth_level(o, 3).err() {
        Some(DocumentError::Shape(ShapeError::SmoothTooDense { faces, cap })) => {
            assert_eq!(cap, kernel::subdiv::MAX_SUBDIV_FACES);
            assert!(faces > cap);
        }
        other => panic!("expected SmoothTooDense, got {other:?}"),
    }
    assert_eq!(doc.state_hash(), h0);
    doc.set_smooth_level(o, 2).unwrap();
    assert_eq!(doc.smooth_level(o), 2);
    // A level the cap would refuse can still reach a cage (a file written
    // before the cap, a cage that grew after its level was set): Bake
    // refuses it the same way instead of a bare range error.
    doc.attr_set(
        kernel::AttrTarget::Entity(kernel::EntityRef::Object(o)),
        kernel::smooth::SMOOTH_NS,
        kernel::smooth::SMOOTH_LEVEL_KEY,
        kernel::AttrValue::Int(3),
    )
    .unwrap();
    assert_eq!(doc.smooth_level(o), 3);
    let h1 = doc.state_hash();
    match doc.bake(o).err() {
        Some(DocumentError::Shape(ShapeError::SmoothTooDense { faces, cap })) => {
            assert_eq!(cap, kernel::subdiv::MAX_SUBDIV_FACES);
            assert!(faces > cap);
        }
        other => panic!("expected SmoothTooDense from bake, got {other:?}"),
    }
    assert_eq!(doc.state_hash(), h1);
    assert_eq!(doc.smooth_level(o), 3);
}

#[test]
fn level_survives_push_pull_and_move() {
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    doc.set_smooth_level(o, 1).unwrap();
    let face = top(&doc, o);
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .unwrap();
    assert_eq!(doc.smooth_level(o), 1);
    doc.transform_object(o, &Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(doc.smooth_level(o), 1);
}

#[test]
fn boolean_copies_a_level_to_the_result() {
    let mut doc = Document::new();
    let a = doc_box(&mut doc, 0.0);
    let b = doc_box(&mut doc, 0.5);
    doc.set_smooth_level(a, 3).unwrap();
    let (r, _) = doc.boolean(BooleanOp::Union, a, b).unwrap();
    assert_eq!(doc.smooth_level(r), 3);
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.smooth_level(r), 3, "redo restores the carried level");
    // B's level does not carry.
    let mut doc = Document::new();
    let a = doc_box(&mut doc, 0.0);
    let b = doc_box(&mut doc, 0.5);
    doc.set_smooth_level(b, 2).unwrap();
    let (r, _) = doc.boolean(BooleanOp::Union, a, b).unwrap();
    assert_eq!(doc.smooth_level(r), 0);
}

#[test]
fn crease_toggle_undo_redo() {
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    let edges = bottom_edges(&doc, o);
    assert_eq!(edges.len(), 4);
    let h0 = doc.state_hash();
    doc.set_crease(o, &edges, true).unwrap();
    let creased = |doc: &Document| {
        let obj = doc.object(o).unwrap();
        edges.iter().filter(|&&e| obj.edges()[e].crease).count()
    };
    assert_eq!(creased(&doc), 4);
    let h1 = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h0);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), h1);
    doc.set_crease(o, &edges, false).unwrap();
    assert_eq!(creased(&doc), 0);
}

#[test]
fn crease_on_soft_edge_refused() {
    // A drawn circle extruded: its wall facets meet in soft seams.
    let mut doc2 = Document::new();
    let s = doc2.add_sketch(ground());
    doc2.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc2.sketch_mut(s).unwrap();
        common::curves::add_circle(sk, p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), 0.5, 24);
    }
    doc2.end_sketch_gesture(s).unwrap();
    let r = doc2.extrudable_regions(s).unwrap()[0];
    let c = doc2.extrude_region(s, r, 1.0).unwrap().0;
    let obj = doc2.object(c).unwrap();
    let seam = obj
        .edges()
        .keys()
        .find(|&e| obj.edge_smoothness(e) == EdgeSmoothness::Seam)
        .expect("a cylinder wall has soft seams");
    assert_eq!(
        doc2.set_crease(c, &[seam], true).err(),
        Some(DocumentError::Shape(ShapeError::CreaseOnSoftEdge))
    );
}

#[test]
fn bake_is_valid_and_one_undo_restores_cage_and_level() {
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    let edges = bottom_edges(&doc, o);
    doc.set_crease(o, &edges, true).unwrap();
    doc.set_smooth_level(o, 2).unwrap();
    let h0 = doc.state_hash();
    let faces0 = doc.object(o).unwrap().faces().len();
    doc.bake(o).unwrap();
    assert_eq!(doc.smooth_level(o), 0, "the level is cleared");
    let baked = doc.object(o).unwrap();
    baked.validate().unwrap();
    assert_eq!(baked.watertight(), WatertightState::Watertight);
    assert!(baked.faces().len() >= 6 * 16);
    let (creased, soft) = baked.edges().values().fold((0, 0), |(c, s), e| {
        (c + usize::from(e.crease), s + usize::from(e.soft))
    });
    assert!(
        creased > 0 && soft > 0,
        "creases stay sharp, the rest is soft"
    );
    assert!(
        baked.edges().values().all(|e| !(e.crease && e.soft)),
        "never both"
    );
    // The bottom stays flat: every point at z ≤ 0 + tol is on the ground.
    for v in baked.vertices().values() {
        assert!(v.position.z >= -1e-12);
    }
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h0, "one undo restores the cage and level");
    assert_eq!(doc.object(o).unwrap().faces().len(), faces0);
    assert_eq!(doc.smooth_level(o), 2);
    doc.redo().unwrap();
    assert_eq!(doc.smooth_level(o), 0);
    // Nothing to bake.
    assert_eq!(
        doc.bake(o).err(),
        Some(DocumentError::Shape(ShapeError::AmountOutOfRange))
    );
}

#[test]
fn save_load_keeps_level_and_creases() {
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    let edges = bottom_edges(&doc, o);
    doc.set_crease(o, &edges, true).unwrap();
    doc.set_smooth_level(o, 3).unwrap();
    let bytes = doc.save();
    let back = Document::load(&bytes).unwrap();
    let id = back.visible_object_ids()[0];
    assert_eq!(back.smooth_level(id), 3);
    let creased = back
        .object(id)
        .unwrap()
        .edges()
        .values()
        .filter(|e| e.crease)
        .count();
    assert_eq!(creased, 4);
}

#[test]
fn crease_on_a_blends_soft_joint_refused() {
    // A rolled corner's border with its strips is soft and reads as a
    // tangent joint (a structure line); it is still a soft edge, so it
    // cannot be creased.
    let mut doc = Document::new();
    let o = doc_box(&mut doc, 0.0);
    let edges: Vec<_> = doc.object(o).unwrap().edges().keys().collect();
    doc.blend_edges(
        o,
        &kernel::BlendSpec {
            edges,
            profile: kernel::BlendProfile::Round {
                radius: 0.1,
                segments: 6,
            },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        },
    )
    .unwrap();
    let obj = doc.object(o).unwrap();
    let role = |f| obj.faces()[f].blend.map(|t| t.role);
    let border = obj
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_faces_for_test(e).unwrap();
            let mut pair = [role(a), role(b)];
            pair.sort_by_key(|r| r.map(|r| r as u8));
            pair == [
                Some(kernel::BlendRole::Strip),
                Some(kernel::BlendRole::Corner),
            ]
        })
        .expect("a rolled corner borders a strip");
    assert!(obj.edges()[border].soft);
    assert_eq!(obj.edge_smoothness(border), EdgeSmoothness::Tangent);
    assert_eq!(
        doc.set_crease(o, &[border], true).err(),
        Some(DocumentError::Shape(ShapeError::CreaseOnSoftEdge))
    );
}
