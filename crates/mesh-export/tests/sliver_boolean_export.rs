//! A boolean result that keeps faces under the old area-only floor (KB-25:
//! faces down to 10 nm wide now keep a plane) tessellates with every face
//! drawn and outward-wound, and exports as a closed binary STL; and so does
//! one whose cut slivers under 0.1 µm the boolean merged away.
//!
//! The geometry is the kernel's sliver spec: a 10 mm box and a tetrahedron
//! whose apex sits `h` above the middle of the box top, so the cut leaves
//! a cap about `h` across. At 0.3 µm the cap is kept; at 50 nm and 5 nm it
//! is merged into whichever solid it pokes through, and the tetrahedron
//! minus the box is nothing at all.

use std::collections::BTreeMap;

use kernel::{BooleanOp, Document, ImportNode, ImportScene, MeshRecipe, ObjectId, Point3};

const MM: f64 = 1e-3;
const BOX: f64 = 10.0 * MM;

/// A named mesh: positions and outward-wound face index lists.
type Mesh<'a> = (&'a str, Vec<Point3>, Vec<Vec<usize>>);

fn mesh_doc(meshes: Vec<Mesh>) -> Document {
    let roots = meshes
        .into_iter()
        .map(|(name, positions, faces)| {
            let count = faces.len();
            ImportNode::Mesh(MeshRecipe {
                name: name.into(),
                positions,
                faces,
                face_materials: vec![kernel::serialize::NO_MATERIAL; count],
                face_uv_frames: vec![None; count],
                face_holes: vec![vec![]; count],
                base_material: kernel::serialize::NO_MATERIAL,
                tags: vec![],
                extra: Default::default(),
            })
        })
        .collect();
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots,
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
    doc
}

fn box_mesh() -> (Vec<Point3>, Vec<Vec<usize>>) {
    let p = |i: usize| {
        Point3::new(
            if i & 1 == 0 { 0.0 } else { BOX },
            if i & 2 == 0 { 0.0 } else { BOX },
            if i & 4 == 0 { 0.0 } else { BOX },
        )
    };
    let faces = vec![
        vec![0, 2, 3, 1],
        vec![4, 5, 7, 6],
        vec![0, 1, 5, 4],
        vec![2, 6, 7, 3],
        vec![0, 4, 6, 2],
        vec![1, 3, 7, 5],
    ];
    ((0..8).map(p).collect(), faces)
}

fn tet_mesh(h: f64) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let (cx, cy, r, z) = (BOX / 2.0, BOX / 2.0, 5.0 * MM / 3f64.sqrt(), 5.0 * MM);
    let mut pts: Vec<Point3> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|a| {
            Point3::new(
                cx + r * a.to_radians().cos(),
                cy + r * a.to_radians().sin(),
                z,
            )
        })
        .collect();
    pts.push(Point3::new(cx, cy, BOX + h));
    (
        pts,
        vec![vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3]],
    )
}

/// The document holding only the boolean's result.
fn result_doc(h: f64, op: BooleanOp, tet_first: bool) -> (Document, ObjectId) {
    let (bp, bf) = box_mesh();
    let (tp, tf) = tet_mesh(h);
    let mut doc = mesh_doc(vec![("Box", bp, bf), ("Tet", tp, tf)]);
    let ids = doc.visible_object_ids();
    let (a, b) = if tet_first {
        (ids[1], ids[0])
    } else {
        (ids[0], ids[1])
    };
    let (id, _) = doc
        .boolean(op, a, b)
        .unwrap_or_else(|e| panic!("{op:?} h={h:e}: {e:?}"));
    assert_eq!(doc.visible_object_ids(), vec![id]);
    (doc, id)
}

type DirectedEdges = BTreeMap<([u32; 3], [u32; 3]), usize>;

/// Triangle count, enclosed volume (mm³) and directed-edge use counts of a
/// binary STL, asserting each stored normal agrees with its winding.
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

const CASES: [(BooleanOp, bool, &str); 4] = [
    (BooleanOp::Union, false, "union"),
    (BooleanOp::Intersect, false, "intersect"),
    (BooleanOp::Subtract, false, "box - tet"),
    (BooleanOp::Subtract, true, "tet - box"),
];

/// Cap heights: one kept, two merged away.
const HEIGHTS: [f64; 3] = [3e-7, 5e-8, 5e-9];

/// The cases with a result at `h`: under the feature floor the cap alone
/// (the tetrahedron minus the box) merges into nothing.
fn cases_at(h: f64) -> impl Iterator<Item = (BooleanOp, bool, &'static str)> {
    CASES
        .into_iter()
        .filter(move |&(_, tet_first, _)| !(tet_first && h < kernel::tol::FEATURE_MIN))
}

#[test]
fn the_cap_alone_under_the_feature_floor_is_nothing() {
    for h in [5e-8, 5e-9] {
        let (bp, bf) = box_mesh();
        let (tp, tf) = tet_mesh(h);
        let mut doc = mesh_doc(vec![("Box", bp, bf), ("Tet", tp, tf)]);
        let ids = doc.visible_object_ids();
        assert!(
            matches!(
                doc.boolean(BooleanOp::Subtract, ids[1], ids[0]),
                Err(kernel::DocumentError::Boolean(
                    kernel::BooleanError::EmptyResult
                ))
            ),
            "h={h:e}"
        );
    }
}

#[test]
fn band_faces_tessellate_outward_with_every_face_drawn() {
    for h in HEIGHTS {
        for (op, tet_first, name) in cases_at(h) {
            let what = format!("{name} h={h:e}");
            let (doc, id) = result_doc(h, op, tet_first);
            let obj = doc.object(id).unwrap();
            let mesh = tessellate::tessellate(obj, doc.materials())
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            let pos = |i: u32| {
                let i = i as usize * 3;
                [0, 1, 2].map(|k| f64::from(mesh.positions[i + k]))
            };
            let mut drawn = vec![0usize; mesh.face_ranges.len()];
            for tri in mesh.front_triangles() {
                let [a, b, c] = [pos(tri[0]), pos(tri[1]), pos(tri[2])];
                let u = [0, 1, 2].map(|k| b[k] - a[k]);
                let w = [0, 1, 2].map(|k| c[k] - a[k]);
                let cross = [
                    u[1] * w[2] - u[2] * w[1],
                    u[2] * w[0] - u[0] * w[2],
                    u[0] * w[1] - u[1] * w[0],
                ];
                let n = tri[0] as usize * 3;
                let normal = [0, 1, 2].map(|k| f64::from(mesh.normals[n + k]));
                let along: f64 = (0..3).map(|k| cross[k] * normal[k]).sum();
                assert!(along > 0.0, "{what}: a triangle winds against its face");
                let face = mesh
                    .face_ranges
                    .iter()
                    .position(|&(_, base, len)| tri[0] >= base && tri[0] < base + len)
                    .unwrap();
                drawn[face] += 1;
            }
            assert!(
                drawn.iter().all(|&n| n > 0),
                "{what}: every face draws at least one triangle ({drawn:?})"
            );
        }
    }
}

#[test]
fn band_results_export_a_closed_stl() {
    for h in HEIGHTS {
        for (op, tet_first, name) in cases_at(h) {
            let what = format!("{name} h={h:e}");
            let (doc, id) = result_doc(h, op, tet_first);
            let bytes = mesh_export::export_stl(&doc, 0, true, false)
                .unwrap_or_else(|e| panic!("{what}: {}", e.message()));
            let (count, volume_mm3, edges) = decode(&bytes);
            assert!(count >= 4, "{what}");
            for ((a, b), uses) in &edges {
                assert_eq!(*uses, 1, "{what}: one triangle per directed edge");
                assert_eq!(
                    edges.get(&(*b, *a)),
                    Some(&1),
                    "{what}: every edge has a twin"
                );
            }
            let obj = doc.object(id).unwrap();
            let expected = obj.volume().unwrap() * 1e9;
            // The file stores f32 millimeters: near 10 mm one step is about
            // 9.5e-7 mm, so the volume can move by the surface area times a
            // step or two (it dominates for the sub-micron cap, which is
            // only tens of steps across).
            let step = f64::from(f32::EPSILON) * 8.0;
            let rounding = obj.surface_area() * 1e6 * 2.0 * step;
            assert!(
                (volume_mm3 - expected).abs() <= expected * 1e-5 + rounding,
                "{what}: STL volume {volume_mm3} vs {expected}"
            );
        }
    }
}
