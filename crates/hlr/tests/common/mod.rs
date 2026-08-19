//! Shared builders for the hlr specs: kernel-built solids, tessellated.
#![allow(dead_code)]

use kernel::{CurveGeom, Point3, Vec3};
use tessellate::RenderMesh;

/// An axis-aligned box `[x0,x1]×[y0,y1]×[z0,z1]` built the way the app does
/// (rectangle sketch + extrude), so its edges/faces are exactly the kernel's.
pub fn box_mesh(
    doc: &mut kernel::Document,
    x0: f64,
    y0: f64,
    z0: f64,
    x1: f64,
    y1: f64,
    z1: f64,
) -> (kernel::ObjectId, RenderMesh) {
    let plane = kernel::Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z0),
        Point3::new(1.0, 0.0, z0),
        Point3::new(0.0, 1.0, z0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let corners = [
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
        ];
        for i in 0..4 {
            sk.add_segment(corners[i], corners[(i + 1) % 4]).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (obj, _) = doc.extrude_region(s, region, z1 - z0).unwrap();
    let mesh = tessellate::tessellate(doc.object(obj).unwrap(), doc.materials()).unwrap();
    (obj, mesh)
}

/// A vertical cylinder (a real curved wall: soft facet seams).
pub fn cylinder_mesh(
    doc: &mut kernel::Document,
    cx: f64,
    cy: f64,
    z0: f64,
    radius: f64,
    height: f64,
    n: usize,
) -> (kernel::ObjectId, RenderMesh) {
    let plane = kernel::Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z0),
        Point3::new(1.0, 0.0, z0),
        Point3::new(0.0, 1.0, z0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let center = Point3::new(cx, cy, z0);
        sk.begin_curve_with(CurveGeom { center, radius }).unwrap();
        let ring: Vec<Point3> = (0..n)
            .map(|i| {
                let a = (i as f64) / (n as f64) * std::f64::consts::TAU;
                Point3::new(cx + radius * a.cos(), cy + radius * a.sin(), z0)
            })
            .collect();
        for i in 0..n {
            sk.add_segment(ring[i], ring[(i + 1) % n]).unwrap();
        }
        sk.end_curve();
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (obj, _) = doc.extrude_region(s, region, height).unwrap();
    let mesh = tessellate::tessellate(doc.object(obj).unwrap(), doc.materials()).unwrap();
    (obj, mesh)
}

pub fn parallel(eye: [f64; 3], target: [f64; 3], up: [f64; 3]) -> hlr::Camera {
    hlr::Camera {
        eye: Point3::new(eye[0], eye[1], eye[2]),
        target: Point3::new(target[0], target[1], target[2]),
        up: Vec3::new(up[0], up[1], up[2]),
        projection: hlr::Projection::Parallel,
    }
}

pub fn perspective(eye: [f64; 3], target: [f64; 3], up: [f64; 3]) -> hlr::Camera {
    hlr::Camera {
        projection: hlr::Projection::Perspective,
        ..parallel(eye, target, up)
    }
}

/// Total length of segments of a kind.
pub fn total_len(d: &hlr::LineDrawing, kind: hlr::Kind) -> f64 {
    d.segs
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| ((s.b[0] - s.a[0]).powi(2) + (s.b[1] - s.a[1]).powi(2)).sqrt())
        .sum()
}

pub fn count(d: &hlr::LineDrawing, kind: hlr::Kind) -> usize {
    d.segs.iter().filter(|s| s.kind == kind).count()
}
