//! A solid whose facets are all far below the retired fixed area gate is
//! still a valid, representable mesh: binary STL keeps every triangle, so
//! the written file stays closed and oriented. The old writer silently
//! dropped such facets and left the solid open.

use std::collections::BTreeMap;

use kernel::{Document, ImportNode, ImportScene, MeshRecipe, Point3};

/// A cube `side` meters on a side, ingested as a mesh so the kernel's
/// extrusion size limits do not apply.
fn cube(side: f64) -> Document {
    let positions: Vec<Point3> = (0..8)
        .map(|i| {
            Point3::new(
                if i & 1 == 0 { 0.0 } else { side },
                if i & 2 == 0 { 0.0 } else { side },
                if i & 4 == 0 { 0.0 } else { side },
            )
        })
        .collect();
    // Outward-wound (counterclockwise from outside) quads.
    let faces = vec![
        vec![0, 2, 3, 1],
        vec![4, 5, 7, 6],
        vec![0, 1, 5, 4],
        vec![2, 6, 7, 3],
        vec![0, 4, 6, 2],
        vec![1, 3, 7, 5],
    ];
    let count = faces.len();
    let mesh = MeshRecipe {
        name: "Cube".into(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; count],
        face_uv_frames: vec![None; count],
        face_holes: vec![vec![]; count],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: vec![],
        extra: Default::default(),
    };
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(mesh)],
        guides: vec![],
        tags: vec![],
        camera: None,
        scenes: vec![],
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    let mut doc = Document::new();
    doc.ingest(scene, vec![]).unwrap();
    assert_eq!(doc.visible_object_ids().len(), 1);
    doc
}

type DirectedEdges = BTreeMap<([u32; 3], [u32; 3]), usize>;

fn decode(bytes: &[u8]) -> (usize, f64, DirectedEdges) {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 84 + 50 * count);
    let key = |p: [f32; 3]| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    let mut edges = BTreeMap::new();
    let mut six_volume = 0.0;
    for t in 0..count {
        let at = 84 + t * 50;
        let read = |o: usize| f32::from_le_bytes(bytes[at + o..at + o + 4].try_into().unwrap());
        let normal = [read(0), read(4), read(8)].map(f64::from);
        let v: [[f32; 3]; 3] =
            std::array::from_fn(|i| std::array::from_fn(|j| read(12 + i * 12 + j * 4)));
        let p = v.map(|c| c.map(f64::from));
        let u = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
        let w = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
        let cross = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let len = cross.iter().map(|c| c * c).sum::<f64>().sqrt();
        assert!(len > 0.0, "no collapsed triangle is written");
        let along: f64 = (0..3).map(|k| cross[k] * normal[k]).sum();
        assert!(along > 0.0, "stored normal agrees with the vertex winding");
        six_volume += p[0][0] * (p[1][1] * p[2][2] - p[1][2] * p[2][1])
            + p[0][1] * (p[1][2] * p[2][0] - p[1][0] * p[2][2])
            + p[0][2] * (p[1][0] * p[2][1] - p[1][1] * p[2][0]);
        for i in 0..3 {
            *edges.entry((key(v[i]), key(v[(i + 1) % 3]))).or_default() += 1;
        }
    }
    (count, six_volume / 6.0, edges)
}

#[test]
fn a_submicrometer_cube_keeps_all_of_its_facets_and_closes() {
    // 0.8 micrometer edges: every triangle's squared winding cross is
    // about 4e-13 mm^4, under the retired 1e-12 gate that dropped them.
    let side = 8e-7;
    let doc = cube(side);
    let bytes = mesh_export::export_stl(&doc, 0, true).unwrap();
    let (count, volume_mm3, edges) = decode(&bytes);
    assert_eq!(count, 12, "a box is twelve triangles; none are dropped");
    for ((a, b), uses) in &edges {
        assert_eq!(*uses, 1, "one triangle per directed edge");
        assert_eq!(edges.get(&(*b, *a)), Some(&1), "every edge has a twin");
    }
    let side_mm = side * 1000.0;
    let expected = side_mm * side_mm * side_mm;
    assert!(
        (volume_mm3 - expected).abs() <= expected * 1e-5,
        "decoded volume {volume_mm3} vs {expected}"
    );
}

#[test]
fn a_cube_pushed_past_f32_millimeter_resolution_is_refused_whole() {
    // At 100 km from the origin an f32 millimeter coordinate steps by
    // 8 mm, so a 1 mm cube cannot be written faithfully: no partial file.
    let mut doc = cube(0.001);
    let id = doc.visible_object_ids()[0];
    doc.transform_object(
        id,
        &kernel::Transform::translation(kernel::Vec3::new(100_000.0, 100_000.0, 100_000.0)),
    )
    .unwrap();
    let err = mesh_export::export_stl(&doc, 0, true).unwrap_err();
    assert_eq!(err.name(), "export_failed");
    assert!(err.message().contains("No file was produced"));
}
