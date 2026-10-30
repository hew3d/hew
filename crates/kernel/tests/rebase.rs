//! Specs for the model writes parts need (docs/agents/PLUGINS.md "Setting
//! axes"): `Document::rebase_definition` re-expresses a definition in a new
//! frame while every placement stays put, `Document::flip_instance` mirrors
//! one placement, and `Document::duplicate_nodes_placements` copies a
//! selection to explicit placements as one undo step.

use kernel::{
    AxesFrameError, Axis, ComponentId, Document, DocumentError, InstanceId, NodeId, ObjectId,
    Plane, Point3, Transform, Vec3,
};
use std::num::NonZeroU32;

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn extrude_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("segment");
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// The P2.1 occurrence fixture: a world box, a group of two boxes, and a
/// component `outer` holding a nested component instance (`i1`, placing
/// `inner`) and a nested group, placed three times — once plainly (`o1`),
/// once mirrored (`o2`), once inside another component (`o3`, placed by
/// `w1`).
struct Fixture {
    doc: Document,
    inner: ComponentId,
    outer: ComponentId,
    i1: InstanceId,
    o1: InstanceId,
    o2: InstanceId,
    o3: InstanceId,
    w1: InstanceId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let _a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 1.0);
    doc.group_nodes(&[NodeId::Object(b), NodeId::Object(c)])
        .unwrap();
    let d = extrude_box(&mut doc, 6.0, 0.0, 7.0, 2.0, 0.5);
    let (inner, i1, _) = doc.make_component(&[NodeId::Object(d)]).unwrap();
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.0, 3.0, 0.0)));
    doc.transform_instance(i1, &turn).unwrap();
    let e = extrude_box(&mut doc, 8.0, 0.0, 9.0, 1.0, 1.0);
    let (h, _) = doc.group_nodes(&[NodeId::Object(e)]).unwrap();
    let (outer, o1, _) = doc
        .make_component(&[NodeId::Instance(i1), NodeId::Group(h)])
        .unwrap();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(20.0, 0.0, 0.0)));
    let (o2, _) = doc.place_instance(outer, mirror).unwrap();
    let (o3, _) = doc
        .place_instance(outer, Transform::translation(Vec3::new(0.0, 10.0, 0.0)))
        .unwrap();
    let (_, w1, _) = doc.make_component(&[NodeId::Instance(o3)]).unwrap();
    doc.transform_instance(w1, &Transform::translation(Vec3::new(0.0, 0.0, 5.0)))
        .unwrap();
    Fixture {
        doc,
        inner,
        outer,
        i1,
        o1,
        o2,
        o3,
        w1,
    }
}

/// Every vertex the model renders, in world space, keyed by the leaf
/// placement and the vertex's place in its object.
fn world_vertices(doc: &Document) -> Vec<Point3> {
    let mut out = Vec::new();
    for o in doc.visible_object_ids() {
        out.extend(
            doc.object(o)
                .unwrap()
                .vertices()
                .values()
                .map(|v| v.position),
        );
    }
    for (o, pose, _) in doc.expanded_placements() {
        out.extend(
            doc.object(o)
                .unwrap()
                .vertices()
                .values()
                .map(|v| pose.apply_point(v.position)),
        );
    }
    out
}

fn assert_same_world(a: &[Point3], b: &[Point3]) {
    assert_eq!(a.len(), b.len());
    for (p, q) in a.iter().zip(b) {
        assert!((*p - *q).length() < 1e-9, "{p:?} vs {q:?}");
    }
}

/// A 90° turn about the local z: the new x is the old y.
fn quarter_turn() -> (Point3, Vec3, Vec3) {
    (
        Point3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
    )
}

/// A tilted frame with a moved origin.
fn tilted_moved() -> (Point3, Vec3, Vec3) {
    let r = Transform::rotation(Vec3::new(1.0, 2.0, 0.5), 0.7).unwrap();
    (
        Point3::new(6.5, 1.0, 0.25),
        r.apply_vector(Vec3::new(1.0, 0.0, 0.0)),
        r.apply_vector(Vec3::new(0.0, 1.0, 0.0)),
    )
}

#[test]
fn rebase_keeps_world_geometry() {
    for frame in [quarter_turn(), tilted_moved()] {
        for which in 0..2 {
            let mut f = fixture();
            let before = world_vertices(&f.doc);
            let component = if which == 0 { f.inner } else { f.outer };
            let (o, x, y) = frame;
            f.doc.rebase_definition(component, o, x, y).unwrap();
            assert_same_world(&before, &world_vertices(&f.doc));
        }
    }
}

#[test]
fn rebase_moves_the_definition_local_geometry() {
    let mut f = fixture();
    let (lo0, hi0) = f.doc.definition_bounds(f.inner).unwrap();
    let (o, x, y) = quarter_turn();
    f.doc.rebase_definition(f.inner, o, x, y).unwrap();
    let (lo, hi) = f.doc.definition_bounds(f.inner).unwrap();
    // The new x is the old y, the new y the old -x.
    assert!((lo.x - lo0.y).abs() < 1e-9 && (hi.x - hi0.y).abs() < 1e-9);
    assert!((lo.y + hi0.x).abs() < 1e-9 && (hi.y + lo0.x).abs() < 1e-9);
}

#[test]
fn rebase_undo_restores_state_hash_and_redo_reproduces() {
    let mut f = fixture();
    let h0 = f.doc.state_hash();
    let depth = f.doc.undo_depth();
    let (o, x, y) = tilted_moved();
    f.doc.rebase_definition(f.outer, o, x, y).unwrap();
    assert_eq!(f.doc.undo_depth(), depth + 1, "one undo entry");
    let h1 = f.doc.state_hash();
    let bytes1 = f.doc.save();
    assert_ne!(h0, h1);
    for _ in 0..3 {
        f.doc.undo().unwrap();
        assert_eq!(f.doc.state_hash(), h0);
        f.doc.redo().unwrap();
        assert_eq!(f.doc.state_hash(), h1);
        assert_eq!(
            f.doc.save(),
            bytes1,
            "redo reproduces the commit bit for bit"
        );
    }
}

#[test]
fn rebase_refuses_left_handed_and_non_unit_frames() {
    let mut f = fixture();
    let h0 = f.doc.state_hash();
    let depth = f.doc.undo_depth();
    let o = Point3::new(0.0, 0.0, 0.0);
    let cases = [
        (
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            AxesFrameError::NonOrthonormal,
        ),
        (
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.6, 0.8, 0.0),
            AxesFrameError::NonOrthonormal,
        ),
        (
            Vec3::new(f64::NAN, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            AxesFrameError::NonFinite,
        ),
    ];
    for (x, y, why) in cases {
        assert_eq!(
            f.doc.rebase_definition(f.outer, o, x, y),
            Err(DocumentError::InvalidFrame(why))
        );
        assert_eq!(f.doc.state_hash(), h0, "a refusal changes nothing");
        assert_eq!(f.doc.undo_depth(), depth);
    }
    // A left-handed basis cannot be stated: z is always x × y, so turning
    // x over gives a right-handed frame, and every pose keeps its sign.
    let before = world_vertices(&f.doc);
    let signs: Vec<bool> = [f.o1, f.o2, f.o3]
        .iter()
        .map(|&i| f.doc.instance_pose(i).unwrap().determinant() < 0.0)
        .collect();
    f.doc
        .rebase_definition(
            f.outer,
            o,
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
    assert_same_world(&before, &world_vertices(&f.doc));
    let after: Vec<bool> = [f.o1, f.o2, f.o3]
        .iter()
        .map(|&i| f.doc.instance_pose(i).unwrap().determinant() < 0.0)
        .collect();
    assert_eq!(signs, after);
}

#[test]
fn rebase_refuses_during_session() {
    let mut f = fixture();
    f.doc.open_explode_session(f.o1).unwrap();
    let h0 = f.doc.state_hash();
    let (o, x, y) = quarter_turn();
    assert_eq!(
        f.doc.rebase_definition(f.inner, o, x, y),
        Err(DocumentError::RebaseDuringSession)
    );
    assert_eq!(f.doc.state_hash(), h0);
}

#[test]
fn rebase_refuses_a_stale_definition() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (c, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.undo().unwrap();
    let (o, x, y) = quarter_turn();
    assert_eq!(
        doc.rebase_definition(c, o, x, y),
        Err(DocumentError::UnknownComponent)
    );
}

#[test]
fn rebase_updates_nested_and_placing_instances() {
    let mut f = fixture();
    let poses = |doc: &Document| -> Vec<Transform> {
        [f.i1, f.o1, f.o2, f.o3, f.w1]
            .iter()
            .map(|&i| doc.instance_pose(i).unwrap())
            .collect()
    };
    let before = poses(&f.doc);
    let (o, x, y) = tilted_moved();
    let change = f.doc.rebase_definition(f.outer, o, x, y).unwrap();
    let after = poses(&f.doc);
    // One nested member instance (i1) and three placings (o1, o2 at world,
    // o3 inside w1's definition) change; w1, which places the definition
    // that holds o3, does not.
    let changed: Vec<bool> = before.iter().zip(&after).map(|(a, b)| a != b).collect();
    assert_eq!(changed, vec![true, true, true, true, false]);
    assert_eq!(change.components_touched, vec![f.outer]);
    for i in [f.i1, f.o1, f.o2, f.o3, f.w1] {
        assert!(change.instances_touched.contains(&i), "{i:?} touched");
    }
    // The inner definition has one placing instance, nested in `outer`.
    let before = poses(&f.doc);
    let (o, x, y) = quarter_turn();
    f.doc.rebase_definition(f.inner, o, x, y).unwrap();
    let after = poses(&f.doc);
    let changed: Vec<bool> = before.iter().zip(&after).map(|(a, b)| a != b).collect();
    assert_eq!(changed, vec![true, false, false, false, false]);
}

/// Bitwise view of a pose, so `-0.0` and `0.0` differ.
fn bits(t: &Transform) -> Vec<u64> {
    t.to_affine().iter().map(|v| v.to_bits()).collect()
}

#[test]
fn flip_twice_is_identity() {
    let mut f = fixture();
    for i in [f.o1, f.o2, f.w1] {
        let pose = f.doc.instance_pose(i).unwrap();
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            f.doc.flip_instance(i, axis).unwrap();
            f.doc.flip_instance(i, axis).unwrap();
            assert_eq!(bits(&f.doc.instance_pose(i).unwrap()), bits(&pose));
        }
    }
}

#[test]
fn flip_refuses_nested_instance() {
    let mut f = fixture();
    let h0 = f.doc.state_hash();
    assert_eq!(
        f.doc.flip_instance(f.i1, Axis::X),
        Err(DocumentError::UnknownInstance)
    );
    assert_eq!(
        f.doc.flip_instance(f.o3, Axis::X),
        Err(DocumentError::UnknownInstance)
    );
    assert_eq!(f.doc.state_hash(), h0);
}

#[test]
fn flip_marks_mirrored() {
    let mut f = fixture();
    let before = world_vertices(&f.doc);
    assert!(f.doc.instance_pose(f.o1).unwrap().determinant() > 0.0);
    let depth = f.doc.undo_depth();
    let change = f.doc.flip_instance(f.o1, Axis::X).unwrap();
    assert_eq!(change.instances_touched, vec![f.o1]);
    assert_eq!(f.doc.undo_depth(), depth + 1);
    assert!(f.doc.instance_pose(f.o1).unwrap().determinant() < 0.0);
    // The mirrored placement flips about its own local x through its
    // origin: the pose's x column turns over, the others stay.
    let a = f.doc.instance_pose(f.o1).unwrap().to_affine();
    assert_eq!([a[0], a[4], a[8]], [-1.0, 0.0, 0.0]);
    assert_eq!([a[1], a[5], a[9]], [0.0, 1.0, 0.0]);
    f.doc.undo().unwrap();
    assert_same_world(&before, &world_vertices(&f.doc));
    // A mirrored placement flipped once more reads unmirrored.
    f.doc.flip_instance(f.o2, Axis::Y).unwrap();
    assert!(f.doc.instance_pose(f.o2).unwrap().determinant() > 0.0);
}

fn first_vertex(doc: &Document, node: NodeId) -> Point3 {
    let NodeId::Object(o) = node else {
        panic!("an object")
    };
    let obj = doc.object(o).unwrap();
    obj.vertices().values().next().unwrap().position
}

#[test]
fn placements_order_is_placement_major() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 0.0, 2.0, 1.0, 3.0, 1.0);
    let pa = first_vertex(&doc, NodeId::Object(a));
    let pb = first_vertex(&doc, NodeId::Object(b));
    let t1 = Transform::translation(Vec3::new(5.0, 0.0, 0.0));
    let t2 = Transform::translation(Vec3::new(0.0, 0.0, 7.0));
    let depth = doc.undo_depth();
    let (roots, _) = doc
        .duplicate_nodes_placements(&[NodeId::Object(a), NodeId::Object(b)], &[t1, t2])
        .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(roots.len(), 4);
    let want = [
        t1.apply_point(pa),
        t1.apply_point(pb),
        t2.apply_point(pa),
        t2.apply_point(pb),
    ];
    for (r, w) in roots.iter().zip(want) {
        assert_eq!(first_vertex(&doc, *r), w);
    }
    doc.undo().unwrap();
    assert_eq!(
        doc.visible_object_ids().len(),
        2,
        "one undo removes them all"
    );
    assert_eq!(
        doc.duplicate_nodes_placements(&[NodeId::Object(a)], &[]),
        Err(DocumentError::EmptySelection)
    );
}

#[test]
fn array_wrapper_matches_old_behavior() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let step = Transform::translation(Vec3::new(1.25, 0.1, 0.0));
    // The array op's own composition: copy k sits at `step` composed k
    // times, each from the previous.
    let mut expected = Vec::new();
    let mut placement = step;
    for k in 0..3 {
        if k > 0 {
            placement = placement.then(&step);
        }
        let mut obj = doc.object(a).unwrap().clone();
        obj.apply_transform(&placement).unwrap();
        expected.push(
            obj.vertices()
                .values()
                .map(|v| v.position)
                .collect::<Vec<_>>(),
        );
    }
    let (roots, _) = doc
        .duplicate_nodes_array(&[NodeId::Object(a)], &step, NonZeroU32::new(3).unwrap())
        .unwrap();
    assert_eq!(roots.len(), 3);
    for (r, want) in roots.iter().zip(expected) {
        let NodeId::Object(o) = r else {
            panic!("an object copy")
        };
        let got: Vec<Point3> = doc
            .object(*o)
            .unwrap()
            .vertices()
            .values()
            .map(|v| v.position)
            .collect();
        let bits = |v: &[Point3]| -> Vec<[u64; 3]> {
            v.iter()
                .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                .collect()
        };
        assert_eq!(bits(&got), bits(&want));
    }
}

/// A definition's own drawing (a def-owned sketch) is re-expressed with
/// its members: it stays where it is in the world.
#[test]
fn rebase_carries_def_owned_sketches() {
    let mut f = fixture();
    let ground = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("unit normal");
    let (sid, _) = f
        .doc
        .begin_sketch_on_plane_in_instance(f.o1, ground)
        .unwrap();
    let pose = f.doc.instance_pose(f.o1).unwrap();
    let pose_inv = pose.inverse().unwrap();
    let corners = [
        Point3::new(30.0, 0.0, 0.0),
        Point3::new(31.0, 0.0, 0.0),
        Point3::new(31.0, 1.0, 0.0),
        Point3::new(30.0, 1.0, 0.0),
    ]
    .map(|p| pose_inv.apply_point(p));
    f.doc.begin_sketch_gesture(sid).unwrap();
    {
        let sk = f.doc.sketch_mut(sid).unwrap();
        for k in 0..4 {
            sk.add_segment(corners[k], corners[(k + 1) % 4]).unwrap();
        }
    }
    f.doc.end_sketch_gesture(sid).unwrap();
    assert_eq!(f.doc.sketch_owner_component(sid), Some(f.outer));
    let world = |doc: &Document| -> Vec<Point3> {
        let pose = doc.instance_pose(f.o1).unwrap();
        doc.sketch(sid)
            .unwrap()
            .vertices()
            .values()
            .map(|v| pose.apply_point(v.position))
            .collect()
    };
    let before = world(&f.doc);
    let (o, x, y) = tilted_moved();
    let change = f.doc.rebase_definition(f.outer, o, x, y).unwrap();
    assert!(change.sketches_touched.contains(&sid));
    assert_same_world(&before, &world(&f.doc));
    let local_before = f.doc.sketch(sid).unwrap().clone();
    f.doc.undo().unwrap();
    f.doc.redo().unwrap();
    let local_after = f.doc.sketch(sid).unwrap();
    for (p, q) in local_before
        .vertices()
        .values()
        .zip(local_after.vertices().values())
    {
        assert_eq!(p.position, q.position);
    }
}

/// A copy that fails part way (the second placement reflects, which a
/// baked object refuses) leaves the document exactly as it was: the first
/// placement's object copy AND the copy of its owned sketch are removed
/// with their stable ids, owner links, and tags, and the stable-id counter
/// is rewound.
#[test]
fn a_failed_placement_rolls_back_owned_sketch_copies_too() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let top = Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("unit normal");
    let s = doc.add_sketch(top);
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(0.2, 0.2, 1.0), Point3::new(0.5, 0.3, 1.0))
        .unwrap();
    doc.set_sketch_object_owner(s, Some(a)).unwrap();
    assert_eq!(doc.object_owned_sketches(a), vec![s]);

    let control = doc.clone();
    let bytes = doc.save();
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    let placements = [
        Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
        Transform::scale(Vec3::new(-1.0, 1.0, 1.0)),
    ];
    assert!(matches!(
        doc.duplicate_nodes_placements(&[NodeId::Object(a)], &placements),
        Err(DocumentError::Transform(_))
    ));
    assert_eq!(doc.save(), bytes);
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.visible_object_ids(), control.visible_object_ids());
    assert_eq!(doc.object_owned_sketches(a), vec![s]);

    // The next entity gets the stable id it would have had with no failed
    // copy in between: the counter was rewound.
    let mut fresh = control;
    let next = |d: &mut Document| {
        let id = d.add_sketch(ground());
        d.sid_of(&kernel::EntityRef::Sketch(id)).unwrap()
    };
    assert_eq!(next(&mut doc), next(&mut fresh));
    assert_eq!(doc.save(), fresh.save());
}

/// Copies of a grouped node land in the same group, appended in
/// placement-major order after the existing members; the change names the
/// group once; one undo takes every copy out of the group again.
#[test]
fn placements_of_grouped_nodes_append_to_their_group() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 0.0, 2.0, 1.0, 3.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let before = doc.group_members(g).unwrap();
    let hash = doc.state_hash();
    let t1 = Transform::translation(Vec3::new(5.0, 0.0, 0.0));
    let t2 = Transform::translation(Vec3::new(10.0, 0.0, 0.0));
    let (roots, change) = doc
        .duplicate_nodes_placements(&[NodeId::Object(a), NodeId::Object(b)], &[t1, t2])
        .unwrap();
    assert_eq!(roots.len(), 4);
    let mut want = before.clone();
    want.extend(roots.iter().copied());
    assert_eq!(doc.group_members(g).unwrap(), want);
    assert_eq!(
        change.groups_touched.iter().filter(|&&x| x == g).count(),
        1,
        "the parent group is named once"
    );
    for r in &roots {
        assert_eq!(doc.node_parent(*r), Some(g));
    }
    // Placement-major: a@t1, b@t1, a@t2, b@t2.
    let pa = first_vertex(&doc, NodeId::Object(a));
    let pb = first_vertex(&doc, NodeId::Object(b));
    let wanted = [
        t1.apply_point(pa),
        t1.apply_point(pb),
        t2.apply_point(pa),
        t2.apply_point(pb),
    ];
    for (r, w) in roots.iter().zip(wanted) {
        assert_eq!(first_vertex(&doc, *r), w);
    }
    doc.undo().unwrap();
    assert_eq!(doc.group_members(g).unwrap(), before);
    assert_eq!(doc.state_hash(), hash);
    doc.redo().unwrap();
    assert_eq!(doc.group_members(g).unwrap(), want);
}
