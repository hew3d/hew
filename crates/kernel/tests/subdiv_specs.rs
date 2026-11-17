//! WP19: Catmull–Clark subdivision with sharp creases.

use crate::common;

use std::collections::BTreeMap;

use common::curves::*;
use kernel::subdiv::{SubdivMesh, corner_normals, subdivide};
use kernel::{Object, Point3, ShapeError, Vec3};

fn cube() -> Object {
    box_xyz(1.0, 1.0, 1.0)
}

fn crease_all(obj: &Object) -> Object {
    let mut o = obj.clone();
    let edges: Vec<_> = o.edges().keys().collect();
    for e in edges {
        o.set_edge_crease_for_test(e, true);
    }
    o.validate().unwrap();
    o
}

/// Every undirected edge is used by exactly two faces.
fn closed(m: &SubdivMesh) -> bool {
    let mut uses: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for f in &m.faces {
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    uses.values().all(|&n| n == 2)
}

fn volume(m: &SubdivMesh) -> f64 {
    let mut v = 0.0;
    for f in &m.faces {
        let o = m.points[f[0] as usize].to_vec();
        for k in 1..f.len() - 1 {
            let (a, b) = (
                m.points[f[k] as usize].to_vec(),
                m.points[f[k + 1] as usize].to_vec(),
            );
            v += o.dot(a.cross(b)) / 6.0;
        }
    }
    v
}

#[test]
fn cube_level1_counts() {
    let m = subdivide(&cube(), 1).unwrap();
    assert_eq!(m.faces.len(), 24);
    assert_eq!(m.points.len(), 26);
    assert!(closed(&m));
    let c = p3(0.5, 0.5, 0.5);
    // The 8 corners move inward (toward the center) and stay symmetric.
    let corners: Vec<f64> = m.points[..8].iter().map(|p| (*p - c).length()).collect();
    for &d in &corners {
        assert!(d < (3.0f64).sqrt() / 2.0, "a corner moved inward");
        assert!((d - corners[0]).abs() < 1e-12, "symmetric");
    }
}

#[test]
fn cube_level2_converges() {
    let (m1, m2) = (
        subdivide(&cube(), 1).unwrap(),
        subdivide(&cube(), 2).unwrap(),
    );
    for p in &m2.points {
        for x in [p.x, p.y, p.z] {
            assert!((0.0..=1.0).contains(&x), "inside the cube: {p:?}");
        }
    }
    assert!(volume(&m1) < 1.0 && volume(&m2) < volume(&m1));
    assert!(closed(&m2));
}

#[test]
fn creased_cube_stays_box_along_creases() {
    let m = subdivide(&crease_all(&cube()), 1).unwrap();
    // Every point lies on the cube's surface and faces stay planar.
    for p in &m.points {
        let on = [p.x, p.y, p.z]
            .iter()
            .any(|&x| x.abs() < 1e-12 || (x - 1.0).abs() < 1e-12);
        assert!(on, "{p:?} is on the cube");
    }
    assert!((volume(&m) - 1.0).abs() < 1e-12);
}

#[test]
fn cylinder_rim_creased_keeps_rim() {
    let c = cylinder(0.5, 1.0, 24);
    let mut o = c.clone();
    let rim: Vec<_> = o
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p.z - 1.0).abs() < 1e-12 && (q.z - 1.0).abs() < 1e-12
        })
        .collect();
    assert_eq!(rim.len(), 24);
    for e in rim {
        o.set_edge_crease_for_test(e, true);
    }
    let m = subdivide(&o, 2).unwrap();
    // Points on the creased rim stay in the top plane and close to the
    // rim polygon (crease edges subdivide as curves: (a + 6v + b)/8).
    // The top cap's own points lie in z = 1 too; the rim's are the ones
    // past r = 0.45.
    let rim_pts: Vec<&Point3> = m
        .points
        .iter()
        .filter(|p| (p.z - 1.0).abs() < 1e-12 && (p.x * p.x + p.y * p.y).sqrt() > 0.45)
        .collect();
    assert!(
        rim_pts.len() >= 24 * 4,
        "the rim survives: {}",
        rim_pts.len()
    );
    // A creased rim subdivides toward the cubic B-spline of the rim
    // polygon: in its plane, and round, within about 1 % of the drawn
    // radius (the spline's lowest point, between two vertices, is at
    // r·(46·cos 7.5° + 2·cos 22.5°)/48 ≈ 0.4945).
    for p in rim_pts {
        let r = (p.x * p.x + p.y * p.y).sqrt();
        assert!((0.494..=0.5 + 1e-12).contains(&r), "{p:?} at r {r}");
    }
}

#[test]
fn holed_face_partitioned() {
    let p = plate_with_hole(1.0, 0.1, 0.2, 16);
    let m = subdivide(&p, 1).unwrap();
    assert!(
        closed(&m),
        "the holed faces split into triangles, and the result closes"
    );
    assert_eq!(
        kernel::subdiv::subdivided_face_count(&p, 1),
        m.faces.len(),
        "the size guard counts exactly what subdivision makes"
    );
}

#[test]
fn deterministic() {
    let c = cylinder(0.5, 1.0, 24);
    assert_eq!(subdivide(&c, 2).unwrap(), subdivide(&c, 2).unwrap());
}

#[test]
fn size_guard() {
    let c = cylinder(0.5, 1.0, 24);
    assert_eq!(subdivide(&c, 0).err(), Some(ShapeError::AmountOutOfRange));
    assert_eq!(subdivide(&c, 4).err(), Some(ShapeError::AmountOutOfRange));
    // 64 facets: 66 faces × 4³ is far below the guard, so it passes.
    assert!(subdivide(&cylinder(0.5, 1.0, 64), 3).is_ok());
}

#[test]
fn corner_normals_split_at_creases() {
    let smooth = subdivide(&cube(), 1).unwrap();
    let n = corner_normals(&smooth);
    // Smooth: all corners at a point share one normal.
    let mut at: BTreeMap<u32, Vec<Vec3>> = BTreeMap::new();
    for (f, ns) in smooth.faces.iter().zip(&n) {
        for (&p, &v) in f.iter().zip(ns) {
            at.entry(p).or_default().push(v);
        }
    }
    for vs in at.values() {
        assert!(vs.iter().all(|v| v.approx_eq(vs[0], 1e-12)));
    }
    // Fully creased: a cube corner has three distinct normals.
    let sharp = subdivide(&crease_all(&cube()), 1).unwrap();
    let n = corner_normals(&sharp);
    let mut corner0 = Vec::new();
    for (f, ns) in sharp.faces.iter().zip(&n) {
        for (&p, &v) in f.iter().zip(ns) {
            if p == 0 {
                corner0.push(v);
            }
        }
    }
    let mut distinct: Vec<Vec3> = Vec::new();
    for v in corner0 {
        if !distinct.iter().any(|d| d.approx_eq(v, 1e-9)) {
            distinct.push(v);
        }
    }
    assert_eq!(distinct.len(), 3);
}
