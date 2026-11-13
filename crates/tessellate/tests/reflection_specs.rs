//! A solid with a reflection baked into it renders like the unreflected
//! one: same triangle count, every triangle wound and shaded outward.

use kernel::{Document, MaterialPalette, Plane, Point3, Transform, Vec3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground")
}

fn boxed() -> (Document, kernel::ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(3.0, 0.0, 0.0),
        Point3::new(3.0, 1.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
    ];
    for i in 0..4 {
        doc.sketch_mut(s)
            .unwrap()
            .add_segment(c[i], c[(i + 1) % 4])
            .unwrap();
    }
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (id, _) = doc.extrude_region(s, r, 2.0).unwrap();
    (doc, id)
}

#[test]
fn reflected_object_tessellates_outward() {
    let (mut doc, id) = boxed();
    let plain = tessellate::tessellate(doc.object(id).unwrap(), &MaterialPalette::default())
        .expect("plain tessellates");
    doc.transform_object(id, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .expect("mirror");
    let mesh = tessellate::tessellate(doc.object(id).unwrap(), &MaterialPalette::default())
        .expect("mirrored tessellates");
    assert_eq!(mesh.indices.len(), plain.indices.len());

    let o = doc.object(id).unwrap();
    let n = o.vertices().len() as f64;
    let (mut cx, mut cy, mut cz) = (0.0, 0.0, 0.0);
    for v in o.vertices().values() {
        cx += v.position.x / n;
        cy += v.position.y / n;
        cz += v.position.z / n;
    }
    for tri in mesh.indices.chunks(3) {
        let p = |k: u32| {
            let i = k as usize * 3;
            [
                f64::from(mesh.positions[i]),
                f64::from(mesh.positions[i + 1]),
                f64::from(mesh.positions[i + 2]),
            ]
        };
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        // Winding normal of the triangle.
        let w = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let out = [a[0] - cx, a[1] - cy, a[2] - cz];
        assert!(
            w[0] * out[0] + w[1] * out[1] + w[2] * out[2] > 0.0,
            "a triangle winds into the solid"
        );
        // And the per-vertex shading normal agrees with the winding.
        let i = tri[0] as usize * 3;
        let nrm = [
            f64::from(mesh.normals[i]),
            f64::from(mesh.normals[i + 1]),
            f64::from(mesh.normals[i + 2]),
        ];
        assert!(nrm[0] * w[0] + nrm[1] * w[1] + nrm[2] * w[2] > 0.0);
    }
}
