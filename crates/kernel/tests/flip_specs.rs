//! Executable specs for Flip: `Transform::reflection`, the selection's
//! world bounds, the flip matrix, flipping a selection in place, and a
//! mirrored copy (linked or independent). DEVELOPMENT.md rule 3: the
//! property at the end.

use kernel::{
    Document, DocumentError, FlipAbout, LinkPolicy, NodeId, Object, ObjectId, Plane, Point3,
    SketchId, Transform, TransformError, Vec3, WatertightState,
};
use proptest::prelude::*;

// ------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

fn draw_loop(doc: &mut Document, s: SketchId, pts: &[(f64, f64)]) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..pts.len() {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % pts.len()];
        sk.add_segment(Point3::new(ax, ay, 0.0), Point3::new(bx, by, 0.0))
            .expect("segment");
    }
}

fn extrude_loop(doc: &mut Document, pts: &[(f64, f64)], h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    draw_loop(doc, s, pts);
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// A box with x in [x0, x1], y in [y0, y1], z in [0, h].
fn make_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    extrude_loop(doc, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], h)
}

/// An L-shaped board: a 4 x 3 footprint with a 2 x 2 notch at (2..4, 1..3),
/// 0.5 thick, so it has no mirror symmetry across x or y.
fn l_board(doc: &mut Document) -> ObjectId {
    extrude_loop(
        doc,
        &[
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 1.0),
            (2.0, 1.0),
            (2.0, 3.0),
            (0.0, 3.0),
        ],
        0.5,
    )
}

fn positions(o: &Object) -> Vec<Point3> {
    let mut v: Vec<Point3> = o.vertices().values().map(|v| v.position).collect();
    v.sort_by(|a, b| {
        (a.x, a.y, a.z)
            .partial_cmp(&(b.x, b.y, b.z))
            .expect("finite positions")
    });
    v
}

fn near(a: Point3, b: Point3, tol: f64) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.z - b.z).abs() <= tol
}

/// Multiset equality within `tol`: every point of `a` pairs with its own
/// distinct point of `b`, and nothing is left over.
fn same_set(a: &[Point3], b: &[Point3], tol: f64) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut used = vec![false; b.len()];
    a.iter().all(
        |p| match (0..b.len()).find(|&k| !used[k] && near(*p, b[k], tol)) {
            Some(k) => {
                used[k] = true;
                true
            }
            None => false,
        },
    )
}

fn mirrored(ps: &[Point3], m: &Transform) -> Vec<Point3> {
    ps.iter().map(|&p| m.apply_point(p)).collect()
}

fn assert_solid(o: &Object, volume: f64) {
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!(o.signed_volume() > 0.0, "the solid is right side out");
    assert!((o.signed_volume() - volume).abs() < 1e-9, "volume kept");
    o.validate()
        .expect("the validator accepts the flipped solid");
}

fn affine_near(a: &Transform, b: &Transform, tol: f64) -> bool {
    a.to_affine()
        .iter()
        .zip(b.to_affine().iter())
        .all(|(x, y)| (x - y).abs() < tol)
}

fn obj(n: NodeId) -> ObjectId {
    match n {
        NodeId::Object(o) => o,
        other => panic!("expected an object, got {other:?}"),
    }
}

fn ins(n: NodeId) -> kernel::InstanceId {
    match n {
        NodeId::Instance(i) => i,
        other => panic!("expected an instance, got {other:?}"),
    }
}

const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

// ------------------------------------------------------------- the matrix

#[test]
fn reflection_has_determinant_minus_one_and_fixes_its_plane() {
    let point = Point3::new(1.0, -2.0, 0.5);
    let normal = Vec3::new(1.0, 2.0, -2.0); // length 3, not unit
    let m = Transform::reflection(point, normal).expect("a plane");
    assert!((m.determinant() + 1.0).abs() < 1e-12);
    // Points on the plane stay put.
    let u = Vec3::new(2.0, -1.0, 0.0); // u · n = 0
    let w = Vec3::new(2.0, 0.0, 1.0); // w · n = 0
    for (a, b) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-3.0, 2.5)] {
        let p = point + u * a + w * b;
        assert!(near(m.apply_point(p), p, 1e-12), "a plane point moved");
    }
    // The normal reverses; a point off the plane lands at the same
    // distance on the other side.
    let n = normal.normalized().unwrap();
    assert!((m.apply_vector(n) + n).length() < 1e-12);
    let off = point + n * 0.75;
    assert!(near(m.apply_point(off), point - n * 0.75, 1e-12));
    // An involution.
    assert!(affine_near(&m.then(&m), &Transform::IDENTITY, 1e-12));
}

#[test]
fn reflection_with_a_zero_normal_is_singular() {
    assert_eq!(
        Transform::reflection(Point3::ORIGIN, Vec3::ZERO),
        Err(TransformError::Singular)
    );
    assert_eq!(
        Transform::reflection(Point3::ORIGIN, Vec3::new(f64::NAN, 0.0, 1.0)),
        Err(TransformError::Singular)
    );
    let mut doc = Document::new();
    let b = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let before = doc.state_hash();
    assert_eq!(
        doc.flip_nodes(&[NodeId::Object(b)], &[], Vec3::ZERO, FlipAbout::Center),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert_eq!(doc.state_hash(), before);
}

// ------------------------------------------------------------- in place

#[test]
fn reflection_scales_huge_and_tiny_normals_before_normalizing() {
    let p = Point3::new(1.0, 2.0, 3.0);
    // Squared length overflows: still a reflection across the plane x = -y.
    let m = Transform::reflection(Point3::ORIGIN, Vec3::new(1e200, 1e200, 0.0)).expect("a plane");
    assert!((m.determinant() + 1.0).abs() < 1e-12);
    assert!(near(m.apply_point(p), Point3::new(-2.0, -1.0, 3.0), 1e-12));
    // Squared length underflows: still a reflection across x = 0.
    let m = Transform::reflection(Point3::ORIGIN, Vec3::new(1e-200, 0.0, 0.0)).expect("a plane");
    assert!(near(m.apply_point(p), Point3::new(-1.0, 2.0, 3.0), 0.0));
    let m =
        Transform::reflection(Point3::ORIGIN, Vec3::new(0.0, f64::MAX, f64::MAX)).expect("a plane");
    assert!(near(m.apply_point(p), Point3::new(1.0, -3.0, -2.0), 1e-12));
    // Infinite or exactly zero normals name no plane.
    assert_eq!(
        Transform::reflection(Point3::ORIGIN, Vec3::new(f64::INFINITY, 0.0, 0.0)),
        Err(TransformError::Singular)
    );
    assert_eq!(
        Transform::reflection(Point3::ORIGIN, Vec3::new(0.0, -0.0, 0.0)),
        Err(TransformError::Singular)
    );
}

#[test]
fn flip_copy_refuses_a_group_listed_with_its_own_member() {
    for policy in [LinkPolicy::Linked, LinkPolicy::Independent] {
        let mut doc = Document::new();
        let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
        let b = make_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
        let (g, _) = doc
            .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
            .unwrap();
        let hash = doc.state_hash();
        let depth = doc.undo_depth();
        for nodes in [
            [NodeId::Group(g), NodeId::Object(a)],
            [NodeId::Object(a), NodeId::Group(g)],
        ] {
            assert_eq!(
                doc.flip_copy(&nodes, X, FlipAbout::Center, policy)
                    .map(|_| ()),
                Err(DocumentError::DuplicateMember),
                "{policy:?}"
            );
        }
        assert_eq!(doc.state_hash(), hash);
        assert_eq!(doc.undo_depth(), depth);
    }
}

#[test]
fn flip_an_object_bakes_the_reflection_and_keeps_it_a_solid() {
    let mut doc = Document::new();
    let b = l_board(&mut doc);
    let volume = doc.object(b).unwrap().signed_volume();
    let before = positions(doc.object(b).unwrap());
    let m = doc
        .flip_matrix(&[NodeId::Object(b)], &[], Y, FlipAbout::Center)
        .unwrap();
    let change = doc
        .flip_nodes(&[NodeId::Object(b)], &[], Y, FlipAbout::Center)
        .expect("flip");
    assert_eq!(change.objects_touched, vec![b]);
    let o = doc.object(b).unwrap();
    assert_solid(o, volume);
    // Across y = 1.5: (x, y, z) -> (x, 3 - y, z).
    assert!(same_set(&positions(o), &mirrored(&before, &m), 1e-12));
    assert!(
        o.vertices()
            .values()
            .any(|v| near(v.position, Point3::new(4.0, 3.0, 0.0), 1e-12))
    );
    assert!(
        !o.vertices()
            .values()
            .any(|v| near(v.position, Point3::new(4.0, 0.0, 0.0), 1e-12))
    );
}

#[test]
fn flip_a_sketch_mirrors_it_with_its_normal_kept() {
    // Across a plane it does not lie in, a sketch keeps facing the side it
    // faced (the normal is kept); across its own plane it turns over.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_loop(
        &mut doc,
        s,
        &[(1.0, 0.0), (2.0, 0.0), (2.0, 3.0), (1.0, 3.0)],
    );
    let n0 = doc.sketch(s).unwrap().plane().normal();
    let regions: Vec<_> = doc.sketch(s).unwrap().regions().keys().collect();
    doc.flip_nodes(&[], &[s], X, FlipAbout::Origin)
        .expect("flip");
    let sk = doc.sketch(s).unwrap();
    assert!(
        (sk.plane().normal() - n0).length() < 1e-12,
        "the sketch faces the same side"
    );
    let xs: Vec<f64> = sk.vertices().values().map(|v| v.position.x).collect();
    assert!(
        xs.iter()
            .all(|&x| (-2.0 - 1e-12..=-1.0 + 1e-12).contains(&x))
    );
    let after: Vec<_> = sk.regions().keys().collect();
    assert_eq!(regions, after, "the region keeps its id");
    // Across its own plane the sketch stays where it is and turns over:
    // its normal reverses, so it faces the other side.
    let s3 = doc.add_sketch(ground());
    draw_loop(&mut doc, s3, &[(5.0, 5.0), (6.0, 5.0), (6.0, 6.0)]);
    let before3: Vec<Point3> = doc
        .sketch(s3)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    doc.flip_nodes(&[], &[s3], Z, FlipAbout::Origin)
        .expect("flip");
    let sk3 = doc.sketch(s3).unwrap();
    let after3: Vec<Point3> = sk3.vertices().values().map(|v| v.position).collect();
    assert!(
        same_set(&after3, &before3, 0.0),
        "points on the mirror stay put"
    );
    assert!(
        (sk3.plane().normal() + n0).length() < 1e-12,
        "flipped across its own plane, the sketch faces the other way"
    );
    // Center of a sketch alone: its own vertex bounds.
    let s2 = doc.add_sketch(ground());
    draw_loop(
        &mut doc,
        s2,
        &[(5.0, 0.0), (7.0, 0.0), (7.0, 1.0), (5.0, 1.0)],
    );
    let m = doc.flip_matrix(&[], &[s2], X, FlipAbout::Center).unwrap();
    assert!(near(
        m.apply_point(Point3::new(5.0, 0.0, 0.0)),
        Point3::new(7.0, 0.0, 0.0),
        1e-12
    ));
}

#[test]
fn flip_an_instance_keeps_a_negative_determinant_pose() {
    let mut doc = Document::new();
    let b = l_board(&mut doc);
    let member_before = positions(doc.object(b).unwrap());
    let (_, i, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    doc.transform_instance(i, &Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .unwrap();
    let change = doc
        .flip_nodes(&[NodeId::Instance(i)], &[], X, FlipAbout::Center)
        .expect("flip");
    assert_eq!(change.instances_touched, vec![i]);
    let pose = doc.instance_pose(i).unwrap();
    assert!(pose.determinant() < 0.0, "the placement reads as mirrored");
    // The shared member is untouched.
    assert!(same_set(
        &positions(doc.object(b).unwrap()),
        &member_before,
        0.0
    ));
    // The world footprint stays in x in [10, 14], mirrored about x = 12.
    let (lo, hi) = doc.world_bounds(&[NodeId::Instance(i)]).unwrap();
    assert!((lo.x - 10.0).abs() < 1e-12 && (hi.x - 14.0).abs() < 1e-12);
    assert!(near(
        pose.apply_point(Point3::new(0.0, 0.0, 0.0)),
        Point3::new(14.0, 0.0, 0.0),
        1e-12
    ));
}

#[test]
fn flip_a_group_bakes_into_its_leaves_and_composes_into_its_instances() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let c = make_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Instance(i)])
        .unwrap();
    let change = doc
        .flip_nodes(&[NodeId::Group(g)], &[], X, FlipAbout::Center)
        .expect("flip");
    assert!(change.objects_touched.contains(&a));
    assert!(change.instances_touched.contains(&i));
    // The group spans x in [0, 4]: center 2; the loose box lands at [3, 4].
    let xs: Vec<f64> = doc
        .object(a)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position.x)
        .collect();
    assert!(
        xs.iter()
            .all(|&x| (x - 3.0).abs() < 1e-12 || (x - 4.0).abs() < 1e-12)
    );
    assert_solid(doc.object(a).unwrap(), 1.0);
    // The instance's pose is mirrored; its box now covers [0, 1].
    let pose = doc.instance_pose(i).unwrap();
    assert!(pose.determinant() < 0.0);
    let (lo, hi) = doc.world_bounds(&[NodeId::Instance(i)]).unwrap();
    assert!(lo.x.abs() < 1e-12 && (hi.x - 1.0).abs() < 1e-12);
    // The member geometry itself never moved.
    let member_xs: Vec<f64> = doc
        .object(c)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position.x)
        .collect();
    assert!(
        member_xs
            .iter()
            .all(|&x| (x - 3.0).abs() < 1e-12 || (x - 4.0).abs() < 1e-12)
    );
}

#[test]
fn flip_about_center_uses_the_selection_bounds_center() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 1.0, 0.0, 3.0, 1.0, 1.0);
    let b = make_box(&mut doc, 5.0, 2.0, 6.0, 4.0, 2.0);
    let nodes = [NodeId::Object(a), NodeId::Object(b)];
    let (lo, hi) = doc.world_bounds(&nodes).unwrap();
    assert!(near(lo, Point3::new(1.0, 0.0, 0.0), 0.0));
    assert!(near(hi, Point3::new(6.0, 4.0, 2.0), 0.0));
    // Center (3.5, 2, 1): each axis's plane passes through it.
    for (n, p, q) in [
        (X, Point3::new(1.0, 0.0, 0.0), Point3::new(6.0, 0.0, 0.0)),
        (Y, Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 4.0, 0.0)),
        (Z, Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 0.0, 2.0)),
    ] {
        let m = doc.flip_matrix(&nodes, &[], n, FlipAbout::Center).unwrap();
        assert!(near(m.apply_point(p), q, 1e-12));
    }
    // An instance's bounds come from its expansion through its pose,
    // a hidden member's geometry included.
    let (_, i, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    doc.transform_instance(i, &Transform::translation(Vec3::new(0.0, 0.0, 10.0)))
        .unwrap();
    doc.set_node_user_hidden(NodeId::Object(a), true);
    let (lo, hi) = doc
        .world_bounds(&[NodeId::Object(a), NodeId::Instance(i)])
        .unwrap();
    assert!(near(lo, Point3::new(1.0, 0.0, 0.0), 0.0));
    assert!(near(hi, Point3::new(6.0, 4.0, 12.0), 0.0));
    assert_eq!(doc.world_bounds(&[]), None);
}

#[test]
fn flip_about_origin_and_about_a_point() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 1.0, 0.0, 3.0, 1.0, 1.0);
    let n = [NodeId::Object(a)];
    let m = doc.flip_matrix(&n, &[], X, FlipAbout::Origin).unwrap();
    assert!(near(
        m.apply_point(Point3::new(1.0, 2.0, 3.0)),
        Point3::new(-1.0, 2.0, 3.0),
        1e-12
    ));
    let m = doc
        .flip_matrix(&n, &[], Z, FlipAbout::Point(Point3::new(0.0, 0.0, 5.0)))
        .unwrap();
    assert!(near(
        m.apply_point(Point3::new(1.0, 2.0, 3.0)),
        Point3::new(1.0, 2.0, 7.0),
        1e-12
    ));
    doc.flip_nodes(&n, &[], X, FlipAbout::Point(Point3::new(4.0, 0.0, 0.0)))
        .unwrap();
    let xs: Vec<f64> = doc
        .object(a)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position.x)
        .collect();
    assert!(
        xs.iter()
            .all(|&x| (x - 5.0).abs() < 1e-12 || (x - 7.0).abs() < 1e-12)
    );
}

#[test]
fn flip_twice_is_the_identity_for_objects_and_instances() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    let c = make_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
    let start = Transform::rotation(Z, 0.3)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.0, 2.0, 0.0)));
    doc.transform_instance(i, &start).unwrap();
    let pose0 = doc.instance_pose(i).unwrap();
    let before = positions(doc.object(a).unwrap());
    let nodes = [NodeId::Object(a), NodeId::Instance(i)];
    let n = Vec3::new(1.0, 1.0, 0.0);
    doc.flip_nodes(&nodes, &[], n, FlipAbout::Center).unwrap();
    doc.flip_nodes(&nodes, &[], n, FlipAbout::Center).unwrap();
    assert!(same_set(&positions(doc.object(a).unwrap()), &before, 1e-9));
    let pose = doc.instance_pose(i).unwrap();
    assert!(pose.determinant() > 0.0);
    assert!(affine_near(&pose, &pose0, 1e-9));
    assert_solid(doc.object(a).unwrap(), 4.0);
}

#[test]
fn flip_in_place_is_one_undo_entry_and_exact() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    let c = make_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (_, i, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
    let s = doc.add_sketch(ground());
    draw_loop(&mut doc, s, &[(9.0, 0.0), (10.0, 0.0), (10.0, 1.0)]);
    let depth = doc.undo_depth();
    let hash = doc.state_hash();
    doc.flip_nodes(
        &[NodeId::Object(a), NodeId::Instance(i)],
        &[s],
        Y,
        FlipAbout::Center,
    )
    .unwrap();
    let flipped = doc.state_hash();
    assert_ne!(flipped, hash);
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash, "undo restores the document exactly");
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), flipped, "redo lands the same flip");
}

// ------------------------------------------------------------- copies

#[test]
fn flip_copy_linked_makes_a_mirrored_placement_and_bakes_nothing() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    let before = positions(doc.object(a).unwrap());
    let (roots, change) = doc
        .flip_copy(
            &[NodeId::Object(a)],
            X,
            FlipAbout::Center,
            LinkPolicy::Linked,
        )
        .expect("copy");
    assert_eq!(roots.len(), 1);
    let copy = ins(roots[0]);
    assert_eq!(change.linked.len(), 1);
    let wrap = change.linked[0];
    assert_eq!(wrap.source, NodeId::Object(a));
    assert_eq!(doc.instance_def(copy), Some(wrap.definition));
    assert_eq!(doc.instance_pose(wrap.instance), Some(Transform::IDENTITY));
    let pose = doc.instance_pose(copy).unwrap();
    assert!(pose.determinant() < 0.0, "the copy is a mirrored placement");
    // Nothing baked: the shared member is exactly the board it was.
    assert!(same_set(&positions(doc.object(a).unwrap()), &before, 0.0));
    assert_solid(
        doc.object(a).unwrap(),
        doc.object(a).unwrap().signed_volume(),
    );
    // The copy covers the board's own box (a flip about its center).
    let (lo, hi) = doc.world_bounds(&[NodeId::Instance(copy)]).unwrap();
    assert!(near(lo, Point3::new(0.0, 0.0, 0.0), 1e-12));
    assert!(near(hi, Point3::new(4.0, 3.0, 0.5), 1e-12));
}

#[test]
fn flip_copy_independent_bakes_a_mirrored_copy() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    let volume = doc.object(a).unwrap().signed_volume();
    let before = positions(doc.object(a).unwrap());
    let m = doc
        .flip_matrix(&[NodeId::Object(a)], &[], X, FlipAbout::Origin)
        .unwrap();
    let (roots, change) = doc
        .flip_copy(
            &[NodeId::Object(a)],
            X,
            FlipAbout::Origin,
            LinkPolicy::Independent,
        )
        .expect("copy");
    assert!(change.linked.is_empty());
    let copy = obj(roots[0]);
    assert_ne!(copy, a);
    assert_solid(doc.object(copy).unwrap(), volume);
    assert!(same_set(
        &positions(doc.object(copy).unwrap()),
        &mirrored(&before, &m),
        1e-12
    ));
    assert!(
        same_set(&positions(doc.object(a).unwrap()), &before, 0.0),
        "the original stays"
    );
}

#[test]
fn flip_copy_of_an_instance_composes_the_pose() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    let (def, i, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.transform_instance(i, &Transform::translation(Vec3::new(0.0, 5.0, 0.0)))
        .unwrap();
    let pose = doc.instance_pose(i).unwrap();
    let m = doc
        .flip_matrix(&[NodeId::Instance(i)], &[], Y, FlipAbout::Center)
        .unwrap();
    for policy in [LinkPolicy::Linked, LinkPolicy::Independent] {
        let (roots, change) = doc
            .flip_copy(&[NodeId::Instance(i)], Y, FlipAbout::Center, policy)
            .expect("copy");
        assert!(change.linked.is_empty(), "an instance is never wrapped");
        let copy = ins(roots[0]);
        assert_eq!(doc.instance_def(copy), Some(def));
        assert!(affine_near(
            &doc.instance_pose(copy).unwrap(),
            &pose.then(&m),
            1e-12
        ));
        assert_eq!(
            doc.instance_pose(i),
            Some(pose),
            "the source placement stays"
        );
        doc.undo().unwrap();
    }
}

#[test]
fn flip_copy_is_one_undo_entry_and_restores_the_loose_object() {
    let mut doc = Document::new();
    let a = l_board(&mut doc);
    doc.set_node_name(NodeId::Object(a), Some("Side".to_string()))
        .unwrap();
    let depth = doc.undo_depth();
    let hash = doc.state_hash();
    let (_, change) = doc
        .flip_copy(
            &[NodeId::Object(a)],
            Z,
            FlipAbout::Center,
            LinkPolicy::Linked,
        )
        .unwrap();
    assert_eq!(change.linked.len(), 1);
    assert!(
        !doc.top_level_nodes().contains(&NodeId::Object(a)),
        "the board is wrapped"
    );
    let copied = doc.state_hash();
    assert_eq!(
        doc.undo_depth(),
        depth + 1,
        "wrap and copy are one undo entry"
    );
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert!(
        doc.top_level_nodes().contains(&NodeId::Object(a)),
        "the loose board is back"
    );
    assert_eq!(doc.object_name(a), Some("Side"));
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), copied);
}

#[test]
fn flip_an_empty_or_stale_selection_refuses_untouched() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let gone = make_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    doc.delete_node(NodeId::Object(gone)).unwrap();
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    assert_eq!(
        doc.flip_nodes(&[], &[], X, FlipAbout::Center),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!(
        doc.flip_copy(&[], X, FlipAbout::Center, LinkPolicy::Linked)
            .map(|_| ()),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!(
        doc.flip_nodes(
            &[NodeId::Object(a), NodeId::Object(gone)],
            &[],
            X,
            FlipAbout::Center
        ),
        Err(DocumentError::UnknownObject)
    );
    for policy in [LinkPolicy::Linked, LinkPolicy::Independent] {
        assert_eq!(
            doc.flip_copy(&[NodeId::Object(gone)], X, FlipAbout::Center, policy)
                .map(|_| ()),
            Err(DocumentError::UnknownObject)
        );
    }
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.undo_depth(), depth);
}

#[test]
fn flip_across_the_selections_own_center_plane_leaves_a_symmetric_part_in_place() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 1.0, 2.0, 3.5, 3.0, 0.75);
    let before = positions(doc.object(a).unwrap());
    for n in [X, Y, Z] {
        doc.flip_nodes(&[NodeId::Object(a)], &[], n, FlipAbout::Center)
            .unwrap();
        assert!(
            same_set(&positions(doc.object(a).unwrap()), &before, 0.0),
            "a box is its own mirror across its center planes"
        );
        assert_solid(doc.object(a).unwrap(), 2.5 * 0.75);
    }
}

// ------------------------------------------------------------- property

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Flipping a selection across an axis plane through its center, then
    /// flipping it back, restores the document bit for bit (positions on
    /// a half-unit grid keep every reflection exact).
    #[test]
    fn flip_then_flip_back_restores_the_state_hash(
        boxes in prop::collection::vec((0i32..8, 0i32..8, 1i32..4, 1i32..4, 1i32..4), 1..4),
        axis in 0usize..3,
        about in 0usize..3,
        wrap_first in any::<bool>(),
    ) {
        let mut doc = Document::new();
        let mut nodes = Vec::new();
        for (k, (x, y, w, d, h)) in boxes.iter().enumerate() {
            let x0 = f64::from(*x) * 0.5 + 10.0 * k as f64;
            let y0 = f64::from(*y) * 0.5;
            let o = make_box(&mut doc, x0, y0, x0 + f64::from(*w) * 0.5, y0 + f64::from(*d) * 0.5, f64::from(*h) * 0.5);
            nodes.push(NodeId::Object(o));
        }
        if wrap_first {
            let (_, i, _) = doc.make_component(&[nodes[0]]).unwrap();
            nodes[0] = NodeId::Instance(i);
        }
        let n = [X, Y, Z][axis];
        let about = [
            FlipAbout::Center,
            FlipAbout::Origin,
            FlipAbout::Point(Point3::new(1.5, -0.5, 2.0)),
        ][about];
        let hash = doc.state_hash();
        doc.flip_nodes(&nodes, &[], n, about).unwrap();
        doc.flip_nodes(&nodes, &[], n, about).unwrap();
        prop_assert_eq!(doc.state_hash(), hash);
    }
}
