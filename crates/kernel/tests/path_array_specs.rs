//! Executable specs for Array along a path (`Document::array_along_path`):
//! copies at even arc-length stations of a sketch path, turned to follow it,
//! with an optional uniform scale ramp, all as one undo step.

use std::f64::consts::FRAC_PI_2;
use std::num::NonZeroU32;

use kernel::{
    Document, DocumentError, FollowMePath, MAX_ARRAY_COUNT, NodeId, ObjectId, PathArraySpec,
    Point3, ShapeError, SketchEdgeId, SketchId, SurfaceRef, Transform, Vec3,
};

mod common;
use common::curves::{add_arc, add_circle, assert_valid_solid, claim_census, ground, p3};

fn spec(count: u32, from: f64, to: f64, align: bool) -> PathArraySpec {
    PathArraySpec {
        count,
        scale_from: from,
        scale_to: to,
        align,
    }
}

/// A unit box with its corner at `(x0, y0, 0)`.
fn build_box(doc: &mut Document, x0: f64, y0: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [
            (x0, y0),
            (x0 + 1.0, y0),
            (x0 + 1.0, y0 + 1.0),
            (x0, y0 + 1.0),
        ];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

/// A 12-facet cylinder of radius 0.5 and height 1 at the origin.
fn build_cylinder(doc: &mut Document) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    add_circle(
        doc.sketch_mut(s).unwrap(),
        p3(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        0.5,
        12,
    );
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

fn path_of(doc: &Document, sketch: SketchId) -> FollowMePath {
    let edges: Vec<SketchEdgeId> = doc.sketch(sketch).unwrap().edges().keys().collect();
    FollowMePath::SketchEdges { sketch, edges }
}

fn line_path(doc: &mut Document, a: Point3, b: Point3) -> FollowMePath {
    let s = doc.add_sketch(ground());
    doc.sketch_mut(s).unwrap().add_segment(a, b).unwrap();
    path_of(doc, s)
}

fn object_ids(roots: &[NodeId]) -> Vec<ObjectId> {
    roots
        .iter()
        .map(|r| match r {
            NodeId::Object(o) => *o,
            _ => panic!("an object copy is an object"),
        })
        .collect()
}

fn positions(doc: &Document, o: ObjectId) -> Vec<Point3> {
    doc.object(o)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect()
}

fn sorted(mut pts: Vec<Point3>) -> Vec<Point3> {
    pts.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    pts
}

fn same_vertex_set(a: &[Point3], b: &[Point3]) -> bool {
    let (a, b) = (sorted(a.to_vec()), sorted(b.to_vec()));
    a.len() == b.len() && a.iter().zip(&b).all(|(p, q)| p.approx_eq(*q, 1e-9))
}

fn extent_x(pts: &[Point3]) -> f64 {
    let lo = pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let hi = pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    hi - lo
}

#[test]
fn straight_path_equals_linear_array() {
    // Path array: 5 instances along a 1 m line, so 4 copies 0.25 m apart.
    let mut a = Document::new();
    let box_a = build_box(&mut a, 0.0, 0.5);
    let path = line_path(&mut a, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0));
    let (roots, _) = a
        .array_along_path(&[NodeId::Object(box_a)], &path, &spec(5, 1.0, 1.0, true))
        .unwrap();
    let path_copies = object_ids(&roots);
    assert_eq!(path_copies.len(), 4);

    let mut b = Document::new();
    let box_b = build_box(&mut b, 0.0, 0.5);
    let (roots, _) = b
        .duplicate_nodes_array(
            &[NodeId::Object(box_b)],
            &Transform::translation(Vec3::new(0.25, 0.0, 0.0)),
            NonZeroU32::new(4).unwrap(),
            kernel::LinkPolicy::Independent,
        )
        .unwrap();
    let linear_copies = object_ids(&roots);
    for (p, l) in path_copies.iter().zip(&linear_copies) {
        assert!(same_vertex_set(&positions(&a, *p), &positions(&b, *l)));
    }
}

#[test]
fn arc_path_aligned_rotates() {
    // A quarter circle of radius 2 about the origin, 32 facets, ccw from +x.
    let n: u32 = 32;
    let mut doc = Document::new();
    let box_id = build_box(&mut doc, 1.5, -0.5);
    let ps = doc.add_sketch(ground());
    add_arc(
        doc.sketch_mut(ps).unwrap(),
        p3(0.0, 0.0, 0.0),
        2.0,
        0.0,
        FRAC_PI_2,
        n as usize,
    );
    let path = path_of(&doc, ps);
    let (roots, _) = doc
        .array_along_path(&[NodeId::Object(box_id)], &path, &spec(4, 1.0, 1.0, true))
        .unwrap();
    let last = *object_ids(&roots).last().unwrap();

    // The copy at the end of the arc is turned about +z by the tangent
    // change between the first and the last facet: (n - 1) facet angles.
    let before = positions(&doc, box_id);
    let after = positions(&doc, last);
    let (i, j) = (0..before.len())
        .flat_map(|i| (0..before.len()).map(move |j| (i, j)))
        .find(|&(i, j)| {
            let d = before[j] - before[i];
            d.x.hypot(d.y) > 0.5
        })
        .unwrap();
    let (d0, d1) = (before[j] - before[i], after[j] - after[i]);
    let angle = d0.cross(d1).z.atan2(d0.dot(d1));
    let facet = FRAC_PI_2 / f64::from(n);
    assert!((angle - (f64::from(n) - 1.0) * facet).abs() < 1e-9);
    assert!(
        (angle - FRAC_PI_2).abs() <= facet + 1e-12,
        "about 90 degrees"
    );
    assert!(d0.cross(d1).x.abs() < 1e-9 && d0.cross(d1).y.abs() < 1e-9);
}

#[test]
fn ramp_scales() {
    // Original plus 3 copies; scales 1.0, 0.75, 0.5 (the original stays 1).
    let mut doc = Document::new();
    let box_id = build_box(&mut doc, 0.0, 0.0);
    let path = line_path(&mut doc, p3(0.0, -2.0, 0.0), p3(3.0, -2.0, 0.0));
    let (roots, _) = doc
        .array_along_path(&[NodeId::Object(box_id)], &path, &spec(4, 1.0, 0.5, true))
        .unwrap();
    let copies = object_ids(&roots);
    assert!((extent_x(&positions(&doc, box_id)) - 1.0).abs() < 1e-9);
    for (c, want) in copies.iter().zip([1.0, 0.75, 0.5]) {
        assert!((extent_x(&positions(&doc, *c)) - want).abs() < 1e-9);
        assert_valid_solid(doc.object(*c).unwrap());
    }
}

#[test]
fn claims_survive_uniform_scale() {
    let mut doc = Document::new();
    let cyl = build_cylinder(&mut doc);
    assert_eq!(
        claim_census(doc.object(cyl).unwrap()).get("cylinder"),
        Some(&12)
    );
    let path = line_path(&mut doc, p3(0.0, 3.0, 0.0), p3(4.0, 3.0, 0.0));
    let (roots, _) = doc
        .array_along_path(&[NodeId::Object(cyl)], &path, &spec(3, 1.0, 0.5, true))
        .unwrap();
    for (c, want) in object_ids(&roots).iter().zip([0.5, 0.25]) {
        let obj = doc.object(*c).unwrap();
        assert_valid_solid(obj);
        assert_eq!(claim_census(obj).get("cylinder"), Some(&12));
        for f in obj.faces().values() {
            if let Some(SurfaceRef::Cylinder { radius, .. }) = f.surface {
                assert!((radius - want).abs() < 1e-9, "radius {radius} vs {want}");
            }
        }
    }
}

#[test]
fn count_limits() {
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0);
    let path = line_path(&mut doc, p3(0.0, 2.0, 0.0), p3(1.0, 2.0, 0.0));
    let refused = DocumentError::Shape(ShapeError::AmountOutOfRange);
    for s in [
        spec(1, 1.0, 1.0, true),
        spec(0, 1.0, 1.0, true),
        spec(MAX_ARRAY_COUNT + 1, 1.0, 1.0, true),
        spec(3, 0.0, 1.0, true),
        spec(3, 1.0, -0.5, true),
        spec(3, f64::NAN, 1.0, true),
    ] {
        assert_eq!(
            doc.array_along_path(&[NodeId::Object(b)], &path, &s)
                .unwrap_err(),
            refused,
            "{s:?}"
        );
    }
    assert_eq!(doc.visible_object_ids().len(), 1, "nothing was added");
    doc.array_along_path(&[NodeId::Object(b)], &path, &spec(2, 1.0, 1.0, true))
        .expect("count 2 is the minimum");
}

#[test]
fn undo_one_step() {
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0);
    let path = line_path(&mut doc, p3(0.0, 2.0, 0.0), p3(5.0, 2.0, 0.0));
    let before = doc.state_hash();
    let (roots, _) = doc
        .array_along_path(&[NodeId::Object(b)], &path, &spec(6, 1.0, 0.4, true))
        .unwrap();
    assert_eq!(doc.visible_object_ids().len(), 6);
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.visible_object_ids(), vec![b], "one undo removes all");
    assert_eq!(doc.state_hash(), before);
    doc.redo().unwrap();
    assert_eq!(doc.visible_object_ids().len(), 6);
    assert_eq!(doc.state_hash(), after);
    assert_eq!(object_ids(&roots).len(), 5);
}

#[test]
fn closed_path_divides_the_loop_by_count() {
    // A 1 m square loop (L = 4); 4 instances sit a quarter loop apart, so the
    // copies land at s = 1, 2, 3 (the loop never repeats its start).
    let mut doc = Document::new();
    let b = build_box(&mut doc, 10.0, 10.0);
    let ps = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(ps).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, e) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(a.0, a.1, 0.0), p3(e.0, e.1, 0.0))
                .unwrap();
        }
    }
    let path = path_of(&doc, ps);
    let (roots, _) = doc
        .array_along_path(&[NodeId::Object(b)], &path, &spec(4, 1.0, 1.0, false))
        .unwrap();
    let first = positions(&doc, b)[0];
    let mut offsets: Vec<(f64, f64)> = object_ids(&roots)
        .iter()
        .map(|c| {
            let d = positions(&doc, *c)[0] - first;
            (d.x, d.y)
        })
        .collect();
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Whichever corner the sketch walks from, an unaligned copy only
    // translates, so the offsets are the other three corners relative to
    // that start corner.
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let matches_start = |s: usize| {
        let mut want: Vec<(f64, f64)> = (0..4)
            .filter(|&j| j != s)
            .map(|j| (corners[j].0 - corners[s].0, corners[j].1 - corners[s].1))
            .collect();
        want.sort_by(|a, b| a.partial_cmp(b).unwrap());
        want.iter()
            .zip(&offsets)
            .all(|(w, o)| (w.0 - o.0).abs() < 1e-9 && (w.1 - o.1).abs() < 1e-9)
    };
    assert!((0..4).any(matches_start), "{offsets:?}");
}
