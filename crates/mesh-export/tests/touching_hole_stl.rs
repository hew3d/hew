//! STL export of a solid whose face holds a hole touching its outer loop
//! at one vertex (a circle drawn tangent to the face's edge, then pushed
//! into a pocket or left as a disc): the triangle soup is closed — every
//! edge of every triangle is matched by the reverse edge of another — and
//! encloses the solid's volume, at the stored facet count and re-faceted.

use std::collections::BTreeMap;

use kernel::{CurveGeom, Document, KernelOp, ObjectId, Plane, Point3};

fn boxed_with_tangent_circle() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let ground = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let s = doc.add_sketch(ground);
    let c = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    for i in 0..4 {
        doc.sketch_mut(s)
            .unwrap()
            .add_segment(c[i], c[(i + 1) % 4])
            .unwrap();
    }
    let r = doc.extrudable_regions(s).unwrap()[0];
    let (id, _) = doc.extrude_region(s, r, 1.0).unwrap();
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    let mut ring: Vec<Point3> = (0..72)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 72.0;
            Point3::new(1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin(), 1.0)
        })
        .collect();
    ring[0] = Point3::new(1.0, 0.0, 1.0);
    doc.draft(top, None, &[id], "Circle", |sk| {
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(1.0, 0.5, 1.0),
            radius: 0.5,
        })?;
        for i in 0..ring.len() {
            sk.add_segment(ring[i], ring[(i + 1) % ring.len()])?;
        }
        sk.end_curve();
        Ok(())
    })
    .unwrap();
    (doc, id)
}

/// The triangles of a binary STL.
fn triangles(stl: &[u8]) -> Vec<[[f32; 3]; 3]> {
    let n = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl.len(), 84 + 50 * n, "a well-formed binary STL");
    (0..n)
        .map(|t| {
            let at = 84 + 50 * t + 12;
            let f =
                |k: usize| f32::from_le_bytes(stl[at + 4 * k..at + 4 * k + 4].try_into().unwrap());
            [[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]]
        })
        .collect()
}

/// Checks the soup is closed (each directed edge, by exact endpoints,
/// matched by the reverse edge of another triangle) and returns its
/// volume in cubic metres (STL coordinates are millimetres).
fn assert_closed_and_volume(tris: &[[[f32; 3]; 3]]) -> f64 {
    // Directed edge counts by exact f32 endpoints.
    let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
    let mut edges: BTreeMap<([u32; 3], [u32; 3]), i64> = BTreeMap::new();
    let mut volume = 0.0;
    for t in tris {
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            *edges.entry((a, b)).or_default() += 1;
            *edges.entry((b, a)).or_default() -= 1;
        }
        let v = |p: [f32; 3]| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    let open: Vec<_> = edges.iter().filter(|(_, n)| **n != 0).collect();
    assert!(
        open.is_empty(),
        "{} unmatched directed edges",
        open.len() / 2
    );
    volume * 1e-9
}

#[test]
fn a_box_with_a_touching_hole_exports_a_closed_stl() {
    let (doc, _) = boxed_with_tangent_circle();
    for segments in [72, 24, 96] {
        let stl = mesh_export::export_stl(&doc, segments, true, false).expect("exports");
        let v = assert_closed_and_volume(&triangles(&stl));
        assert!((v - 4.0).abs() < 1e-4, "{segments} facets: volume {v}");
    }
}

#[test]
fn a_pocket_from_a_touching_hole_exports_a_closed_stl() {
    let (mut doc, id) = boxed_with_tangent_circle();
    let obj = doc.object(id).unwrap();
    let disc = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9 && f.inner_loops.is_empty())
        .map(|(f, _)| f)
        .expect("the disc");
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: disc,
            distance: -0.4,
        },
    )
    .expect("the pocket");
    let stl = mesh_export::export_stl(&doc, 72, true, false).expect("exports");
    let v = assert_closed_and_volume(&triangles(&stl));
    let disc_area = 0.5 * 72.0 * 0.25 * (std::f64::consts::TAU / 72.0).sin();
    assert!((v - (4.0 - 0.4 * disc_area)).abs() < 1e-4, "volume {v}");
}
