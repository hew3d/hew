//! Tessellation of faces whose loops share a vertex (ARCHITECTURE §2.4:
//! every loop simple, loops of one face may touch at isolated points): a
//! circle tangent to the face's edge at one point leaves the face around it
//! with a hole sharing that vertex with its outer loop, and two circles
//! touching inside a face leave two holes sharing one vertex.
//!
//! Each such face triangulates with no gap and no overlap: its triangles
//! all wind with the face, none is degenerate, and their areas sum to the
//! face's net area (outer less holes). Property-tested over random circles
//! on a lattice that makes the touches exact.

use std::f64::consts::{PI, TAU};

use kernel::{CurveGeom, Document, FaceId, Object, ObjectId, Plane, Point3, Vec3};
use proptest::prelude::*;

/// Below this area (m²) a triangle is degenerate. A geometric threshold of
/// the test, far under any facet a drawn circle makes.
const DEGENERATE_AREA: f64 = 1e-14;

/// Relative error allowed between the triangles' summed area and the face's
/// net area: the f32 output buffers carry about 7 significant digits.
const AREA_REL_TOL: f64 = 1e-5;

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground")
}

/// A `w × d × 1` box, its top face's plane.
fn boxed(w: f64, d: f64) -> (Document, ObjectId, Plane) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(w, 0.0, 0.0),
        Point3::new(w, d, 0.0),
        Point3::new(0.0, d, 0.0),
    ];
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let r = doc.extrudable_regions(s).expect("regions")[0];
    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .expect("top");
    (doc, id, top)
}

/// `n` points of the circle about `(cx, cy)` on the top (z = 1), vertex `i`
/// at angle `phase + TAU i / n`.
fn ring(cx: f64, cy: f64, r: f64, n: usize, phase: f64) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = phase + TAU * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), 1.0)
        })
        .collect()
}

/// Draws `pts` as one closed curve on the top through one draft.
fn draw(doc: &mut Document, id: ObjectId, top: Plane, pts: &[Point3], cx: f64, cy: f64, r: f64) {
    doc.draft(top, None, &[id], "Circle", |sk| {
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(cx, cy, 1.0),
            radius: r,
        })?;
        for i in 0..pts.len() {
            sk.add_segment(pts[i], pts[(i + 1) % pts.len()])?;
        }
        sk.end_curve();
        Ok(())
    })
    .expect("the circle draws");
}

/// Whether some face holds two loops sharing a vertex.
fn has_touching_loops(obj: &Object) -> bool {
    obj.faces().values().any(|face| {
        let loops: Vec<Vec<kernel::VertexId>> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| {
                obj.loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect()
            })
            .collect();
        (0..loops.len())
            .any(|i| (i + 1..loops.len()).any(|j| loops[i].iter().any(|v| loops[j].contains(v))))
    })
}

fn signed_area(ring: &[Point3], n: Vec3) -> f64 {
    let mut acc = Vec3::ZERO;
    for i in 0..ring.len() {
        acc = acc + ring[i].to_vec().cross(ring[(i + 1) % ring.len()].to_vec());
    }
    0.5 * acc.dot(n)
}

/// Tessellates `obj` and checks every face: each triangle winds with the
/// face and is not degenerate, and the triangles' areas sum to the face's
/// net area.
fn assert_tessellates_exactly(obj: &Object) {
    let mesh = tessellate::tessellate(obj, &kernel::MaterialPalette::default())
        .expect("the object tessellates");
    let pos = |i: u32| {
        let k = i as usize * 3;
        Point3::new(
            f64::from(mesh.positions[k]),
            f64::from(mesh.positions[k + 1]),
            f64::from(mesh.positions[k + 2]),
        )
    };
    let face_of = |i: u32| -> FaceId {
        mesh.face_ranges
            .iter()
            .find(|&&(_, base, count)| i >= base && i < base + count)
            .map(|&(f, _, _)| f)
            .expect("a vertex of some face")
    };
    let mut sums: std::collections::BTreeMap<FaceId, f64> = std::collections::BTreeMap::new();
    for tri in mesh.front_triangles() {
        let f = face_of(tri[0]);
        assert!(
            tri.iter().all(|&i| face_of(i) == f),
            "a triangle spans two faces"
        );
        let n = obj.faces()[f].plane.normal();
        let (a, b, c) = (pos(tri[0]), pos(tri[1]), pos(tri[2]));
        let area = 0.5 * (b - a).cross(c - a).dot(n);
        assert!(
            area > DEGENERATE_AREA,
            "a degenerate or flipped triangle: {area}"
        );
        *sums.entry(f).or_default() += area;
    }
    for (f, face) in obj.faces() {
        let n = face.plane.normal();
        let net: f64 = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| signed_area(&obj.loop_positions(l).collect::<Vec<_>>(), n))
            .sum();
        let got = sums.get(&f).copied().unwrap_or(0.0);
        assert!(
            (got - net).abs() <= AREA_REL_TOL * net.abs(),
            "face {f:?}: triangles cover {got}, the face is {net}"
        );
    }
}

#[test]
fn a_hole_touching_the_outer_loop_tessellates_with_no_gap_or_overlap() {
    let (mut doc, id, top) = boxed(2.0, 2.0);
    // Rim on the bottom edge at (1, 0): vertex 0 at angle -pi/2.
    let pts = ring(1.0, 0.5, 0.5, 72, -PI / 2.0);
    draw(&mut doc, id, top, &pts, 1.0, 0.5, 0.5);
    let obj = doc.object(id).expect("box");
    assert!(has_touching_loops(obj));
    assert_tessellates_exactly(obj);
}

#[test]
fn two_holes_sharing_a_vertex_tessellate_with_no_gap_or_overlap() {
    let (mut doc, id, top) = boxed(2.0, 2.0);
    let a = ring(0.6, 1.0, 0.3, 24, 0.0);
    let mut b = ring(1.2, 1.0, 0.3, 24, 0.0);
    b[12] = a[0];
    draw(&mut doc, id, top, &a, 0.6, 1.0, 0.3);
    draw(&mut doc, id, top, &b, 1.2, 1.0, 0.3);
    let obj = doc.object(id).expect("box");
    assert!(has_touching_loops(obj));
    assert_tessellates_exactly(obj);
}

/// The lattice case the property test draws from: a `wq × dq` box (in
/// 0.1 m units) with a circle of radius `rq` (same units) and `4·n4`
/// facets, its rim snapped onto edge `side` (0 bottom, 1 right, 2 top, 3
/// left) at fraction `along`/10 of that edge's free run, and with `second`
/// a half-radius circle touching the first at its far point when it fits
/// clear of every edge. `None` when the circle does not fit the box.
fn touching_case(
    wq: u32,
    dq: u32,
    side: u8,
    rq: u32,
    along: u32,
    n4: usize,
    second: bool,
) -> Option<(Document, ObjectId)> {
    let q = 0.1;
    let (w, d) = (f64::from(wq) * q, f64::from(dq) * q);
    let r = f64::from(rq) * q;
    let n = n4 * 4;
    let t = f64::from(along) / 10.0;
    // Center one radius in from the chosen edge, the rim on it.
    let (cx, cy, phase) = match side {
        0 => (r + t * (w - 2.0 * r), r, -PI / 2.0),
        1 => (w - r, r + t * (d - 2.0 * r), 0.0),
        2 => (r + t * (w - 2.0 * r), d - r, PI / 2.0),
        _ => (r, r + t * (d - 2.0 * r), PI),
    };
    if !(w > 2.0 * r && d > 2.0 * r) {
        return None;
    }
    let (mut doc, id, top) = boxed(w, d);
    let mut a = ring(cx, cy, r, n, phase);
    // The rim exactly on the edge.
    a[0] = match side {
        0 => Point3::new(a[0].x, 0.0, 1.0),
        1 => Point3::new(w, a[0].y, 1.0),
        2 => Point3::new(a[0].x, d, 1.0),
        _ => Point3::new(0.0, a[0].y, 1.0),
    };
    draw(&mut doc, id, top, &a, cx, cy, r);
    if second {
        // A second, smaller circle outside the first, touching it at
        // the first's far point (vertex n/2, opposite the rim): its
        // vertex 0, at angle `phase` from its own center, is that point.
        let far = a[n / 2];
        let r2 = 0.5 * r;
        let (dx, dy) = (far.x - cx, far.y - cy);
        let len = dx.hypot(dy);
        let (c2x, c2y) = (far.x + dx / len * r2, far.y + dy / len * r2);
        // Clear of every edge by a margin, so the second circle touches only
        // the first.
        let m = r2 + 1e-3;
        let inside = |x: f64, y: f64| x > m && y > m && x < w - m && y < d - m;
        if inside(c2x, c2y) {
            let mut b = ring(c2x, c2y, r2, n, phase);
            b[0] = far;
            draw(&mut doc, id, top, &b, c2x, c2y, r2);
        }
    }
    Some((doc, id))
}

/// A 2.9 × 2.6 top with a 44-facet circle of radius 0.2 on its right edge
/// and a radius-0.1 circle touching it on the left. Ear clipping fans the
/// far top-left corner across the small circle's lower chords, and the
/// fan's first triangle sits at that corner's tangent to the circle: its
/// apex is 5.7e-8 m off a 2.44 m side, under the f32 quantum of the
/// coordinates, so the rounded buffers would wind it backwards. The
/// tessellation must leave such a triangle out rather than write it
/// inverted (the property test's first found counterexample).
#[test]
fn a_fan_tangent_to_a_hole_writes_no_inverted_sliver() {
    let (doc, id) = touching_case(29, 26, 1, 2, 9, 11, true).expect("the circles fit");
    let obj = doc.object(id).expect("box");
    obj.validate().expect("valid");
    assert!(has_touching_loops(obj));
    assert_tessellates_exactly(obj);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// A circle on a 0.1 m lattice inside a random box top, its rim snapped
    /// onto one of the four edges along an axis (an exact one-point touch),
    /// and optionally a second circle touching the first at its far side.
    #[test]
    fn touching_holes_tessellate_with_no_gap_or_overlap(
        wq in 10u32..=30, dq in 10u32..=30,
        side in 0u8..4, rq in 1u32..=4, along in 1u32..=9, n4 in 6usize..=18,
        second in proptest::bool::ANY,
    ) {
        let case = touching_case(wq, dq, side, rq, along, n4, second);
        prop_assume!(case.is_some());
        let (doc, id) = case.unwrap();
        let obj = doc.object(id).expect("box");
        obj.validate().expect("valid");
        prop_assert!(has_touching_loops(obj));
        assert_tessellates_exactly(obj);
    }
}
