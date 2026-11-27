//! A dense flat area lists quickly: a slab whose top is thousands of
//! coplanar faces (an imported mesh's floor) — every edge between them is
//! drawing — must not take minutes to list (the listing fed each drawing
//! edge to the sketch one at a time, and looked for each region's face
//! among every face of the object).

use kernel::{Document, ImportNode, ImportScene, MeshRecipe, Plane, Point3, Vec3};
use std::time::Instant;

fn slab(n: usize) -> MeshRecipe {
    let mut positions = Vec::new();
    let at = |i: usize, j: usize| j * (n + 1) + i;
    for j in 0..=n {
        for i in 0..=n {
            positions.push(Point3::new(i as f64, j as f64, 1.0));
        }
    }
    let base = positions.len();
    for &(x, y) in &[
        (0.0, 0.0),
        (n as f64, 0.0),
        (n as f64, n as f64),
        (0.0, n as f64),
    ] {
        positions.push(Point3::new(x, y, 0.0));
    }
    let mut faces = Vec::new();
    for j in 0..n {
        for i in 0..n {
            faces.push(vec![at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    // Bottom, then the four walls, each through every top point on its side.
    faces.push(vec![base, base + 3, base + 2, base + 1]);
    let side = |pts: Vec<usize>, b0: usize, b1: usize| {
        let mut f = vec![b0, b1];
        f.extend(pts.into_iter().rev());
        f
    };
    faces.push(side((0..=n).map(|i| at(i, 0)).collect(), base, base + 1));
    faces.push(side(
        (0..=n).map(|j| at(n, j)).collect(),
        base + 1,
        base + 2,
    ));
    faces.push(side(
        (0..=n).rev().map(|i| at(i, n)).collect(),
        base + 2,
        base + 3,
    ));
    faces.push(side(
        (0..=n).rev().map(|j| at(0, j)).collect(),
        base + 3,
        base,
    ));
    let k = faces.len();
    MeshRecipe {
        name: "Floor".into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; k],
        face_uv_frames: vec![None; k],
        face_holes: vec![Vec::new(); k],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

#[test]
fn a_floor_of_thousands_of_faces_lists_quickly() {
    let n = 50;
    let mut doc = Document::new();
    doc.ingest(
        ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![ImportNode::Mesh(slab(n))],
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            geo: None,
            extra: Default::default(),
        },
        Vec::new(),
    )
    .unwrap();
    let id = doc.visible_object_ids()[0];
    let top =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!(
        doc.object_drawing_planes(id)
            .iter()
            .any(|p| (p.signed_distance(Point3::new(0.0, 0.0, 1.0))).abs() < 1e-9)
    );
    let t = Instant::now();
    let units = doc.plane_units(top, None, &[id]).unwrap();
    let took = t.elapsed();
    assert_eq!(units.regions.len(), n * n);
    // Every region is a face of the floor, found as that face.
    assert!(
        units
            .regions
            .iter()
            .all(|r| matches!(r.over, Some((o, Some(_))) if o == id))
    );
    assert!(took.as_secs_f64() < 30.0, "listing took {took:?}");
}
