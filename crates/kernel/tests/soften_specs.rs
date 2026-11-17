//! WP20 Part 3: Soften by angle.

use crate::common;

use common::curves::*;
use kernel::{Document, DocumentError, Object, Plane, Point3, ShapeError, Vec3};

/// A 48-facet cylinder (r 0.05, h 0.1) as an STL brings it in: polygons,
/// no claims, no soft marks.
fn stl_cylinder() -> Object {
    let n = 48;
    let mut pts: Vec<Point3> = Vec::new();
    for z in [0.0, 0.1] {
        for k in 0..n {
            let t = k as f64 * std::f64::consts::TAU / n as f64;
            pts.push(p3(0.05 * t.cos(), 0.05 * t.sin(), z));
        }
    }
    let mut faces: Vec<Vec<usize>> = (0..n)
        .map(|k| vec![k, (k + 1) % n, n + (k + 1) % n, n + k])
        .collect();
    faces.push((0..n).rev().collect());
    faces.push((n..2 * n).collect());
    Object::from_polygons_import_for_test(&pts, &faces).expect("the cylinder imports")
}

fn soft(o: &Object) -> usize {
    o.edges().values().filter(|e| e.soft).count()
}

#[test]
fn stl_like_cylinder_softens() {
    let c = stl_cylinder();
    assert_eq!(soft(&c), 0);
    let (r, n) = c.soften_by_angle(20f64.to_radians()).unwrap();
    // The 48 wall seams bend 7.5° and soften; the 96 rim edges bend 90°.
    assert_eq!(n, 48);
    assert_eq!(soft(&r), 48);
    r.validate().unwrap();
    // A second pass finds nothing new.
    assert_eq!(r.soften_by_angle(20f64.to_radians()).unwrap().1, 0);
}

#[test]
fn coplanar_edges_untouched() {
    // A box whose top is split in two by a drawn line: the line is between
    // coplanar faces and stays hard.
    let pts = [
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
        p3(0.5, 0.0, 1.0),
        p3(0.5, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 8, 9, 7],
        vec![8, 5, 6, 9],
        vec![0, 1, 5, 8, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 9, 6],
        vec![3, 0, 4, 7],
    ];
    let b = Object::from_polygons(&pts, &faces).unwrap();
    // Every bend is 90° except the drawn line (0°): nothing softens even
    // at a wide angle.
    let (_, n) = b.soften_by_angle(80f64.to_radians()).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn creased_untouched() {
    let mut c = stl_cylinder();
    let seam = c
        .edges()
        .keys()
        .find(|&e| {
            let (p, q) = c.edge_endpoints(e).unwrap();
            (p.z - q.z).abs() > 0.05
        })
        .unwrap();
    c.set_edge_crease_for_test(seam, true);
    let (r, n) = c.soften_by_angle(20f64.to_radians()).unwrap();
    assert_eq!(n, 47);
    assert!(!r.edges()[seam].soft && r.edges()[seam].crease);
}

#[test]
fn angle_range_refused() {
    let c = stl_cylinder();
    for bad in [0.0, -1.0, std::f64::consts::PI, f64::NAN] {
        assert_eq!(
            c.soften_by_angle(bad).err(),
            Some(ShapeError::AmountOutOfRange)
        );
    }
}

#[test]
fn document_soften_is_one_undo_step() {
    let mut doc = Document::new();
    let ground = Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(ground);
    doc.begin_sketch_gesture(s).unwrap();
    {
        // A 12-gon drawn as plain lines (no circle), extruded: hard 30° seams.
        let sk = doc.sketch_mut(s).unwrap();
        let n = 12;
        let p = |k: usize| {
            let t = k as f64 * std::f64::consts::TAU / n as f64;
            p3(t.cos(), t.sin(), 0.0)
        };
        for k in 0..n {
            sk.add_segment(p(k), p(k + 1)).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let o = doc.extrude_region(s, r, 1.0).unwrap().0;
    let h0 = doc.state_hash();
    let (n, change) = doc.soften_by_angle(o, 40f64.to_radians()).unwrap();
    assert_eq!(n, 12);
    assert!(change.objects_touched.contains(&o));
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h0);
    // Below the seams' bend: nothing to do, nothing recorded.
    let (n, _) = doc.soften_by_angle(o, 10f64.to_radians()).unwrap();
    assert_eq!(n, 0);
    assert_eq!(doc.state_hash(), h0);
    assert_eq!(
        doc.soften_by_angle(o, 0.0).err(),
        Some(DocumentError::Shape(ShapeError::AmountOutOfRange))
    );
}
