//! `Object::volume` and `Object::surface_area`.

use kernel::{Document, ImportNode, ImportScene, MeshRecipe, Object, Point3};

fn cube(s: f64) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let v = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(s, 0.0, 0.0),
        Point3::new(s, s, 0.0),
        Point3::new(0.0, s, 0.0),
        Point3::new(0.0, 0.0, s),
        Point3::new(s, 0.0, s),
        Point3::new(s, s, s),
        Point3::new(0.0, s, s),
    ];
    let f = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    (v, f)
}

#[test]
fn a_closed_cube_has_volume_and_area() {
    let (v, f) = cube(2.0);
    let o = Object::from_polygons(&v, &f).unwrap();
    assert!((o.volume().unwrap() - 8.0).abs() < 1e-12);
    assert!((o.surface_area() - 24.0).abs() < 1e-12);
}

#[test]
fn an_open_shell_has_no_volume_but_has_area() {
    let (v, mut f) = cube(1.0);
    f.remove(1);
    let o = Object::from_polygons(&v, &f).unwrap();
    assert_eq!(o.volume(), None);
    assert!((o.surface_area() - 5.0).abs() < 1e-12);
}

/// A 2 x 2 x 1 block with a 1 x 1 hole through it: the top and bottom
/// faces each carry an inner loop. Volume 3, area 18.
fn holed_block_scene() -> ImportScene {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(2.0, 0.0, 1.0),
        p(2.0, 2.0, 1.0),
        p(0.0, 2.0, 1.0),
        p(0.5, 0.5, 0.0),
        p(1.5, 0.5, 0.0),
        p(1.5, 1.5, 0.0),
        p(0.5, 1.5, 0.0),
        p(0.5, 0.5, 1.0),
        p(1.5, 0.5, 1.0),
        p(1.5, 1.5, 1.0),
        p(0.5, 1.5, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        vec![8, 12, 13, 9],
        vec![9, 13, 14, 10],
        vec![10, 14, 15, 11],
        vec![11, 15, 12, 8],
    ];
    let n = faces.len();
    let mut face_holes = vec![Vec::new(); n];
    face_holes[0] = vec![vec![8, 9, 10, 11]];
    face_holes[1] = vec![vec![12, 15, 14, 13]];
    let recipe = MeshRecipe {
        name: "Holed".into(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes,
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    ImportScene {
        geo: None,
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(recipe)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    }
}

#[test]
fn a_holed_face_subtracts_its_hole_from_volume_and_area() {
    let mut doc = Document::new();
    doc.ingest(holed_block_scene(), Vec::new()).unwrap();
    let id = doc.visible_object_ids()[0];
    let o = doc.object(id).unwrap();
    assert!(o.faces().values().any(|f| !f.inner_loops.is_empty()));
    assert!((o.volume().unwrap() - 3.0).abs() < 1e-12);
    assert!((o.surface_area() - 18.0).abs() < 1e-12);
}
