//! Executable specs for editing definition members through an occurrence
//! path (docs/agents/HEW_API.md §7, the `{path}` forms of
//! `hew.entity.move`/`rotate`/`scale` and `hew.solid.subtract`/`slice`):
//! `Document::path_pose`, `transform_def_members`,
//! `duplicate_def_members_array`, `boolean_through_placement`, and
//! `slice_through_placement`. Each changes the definition, so every
//! placement follows; each is one undo entry.

use std::num::NonZeroU32;

use kernel::{
    BooleanOp, ComponentId, Document, DocumentError, GroupId, InstanceId, NodeId, ObjectId, Plane,
    Point3, SketchId, Transform, Vec3,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane_z(z: f64) -> Plane {
    Plane::from_polygon(&[p(0.0, 0.0, z), p(1.0, 0.0, z), p(0.0, 1.0, z)])
        .expect("horizontal plane")
}

/// An axis-aligned box `[x0, x1] × [y0, y1] × [0, h]`.
fn make_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(plane_z(0.0));
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let c = [
            p(x0, y0, 0.0),
            p(x1, y0, 0.0),
            p(x1, y1, 0.0),
            p(x0, y1, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).expect("rect segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// The lowest-x, lowest-y, lowest-z vertex of an object, in its own frame
/// (a box's minimum corner).
fn min_corner(doc: &Document, o: ObjectId) -> Point3 {
    let obj = doc.object(o).expect("object is live");
    obj.vertices()
        .values()
        .map(|v| v.position)
        .fold(p(f64::MAX, f64::MAX, f64::MAX), |a, b| {
            p(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z))
        })
}

fn approx(a: Point3, b: Point3) -> bool {
    a.approx_eq(b, 1e-9)
}

fn vec_approx(a: Vec3, b: Vec3) -> bool {
    (a - b).length_squared() < 1e-18
}

/// The shared fixture: a two-box component `comp` (members `a`, `b`)
/// placed three times — `i1` plainly, `i2` mirrored across x, and `i3`
/// inside a group `h` inside another component `outer`, whose placement
/// `w1` is lifted 5 along z.
struct Fixture {
    doc: Document,
    comp: ComponentId,
    a: ObjectId,
    b: ObjectId,
    i1: InstanceId,
    i2: InstanceId,
    i3: InstanceId,
    h: GroupId,
    outer: ComponentId,
    w1: InstanceId,
}

fn mirror() -> Transform {
    Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = make_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(a), Some("Leg".to_string()))
        .expect("name a");
    let (comp, i1, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("component");
    let (i2, _) = doc
        .place_instance(comp, mirror())
        .expect("mirrored placement");
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .expect("rotation")
        .then(&Transform::translation(Vec3::new(0.0, 10.0, 0.0)));
    let (i3, _) = doc.place_instance(comp, turn).expect("third placement");
    let (h, _) = doc.group_nodes(&[NodeId::Instance(i3)]).expect("group");
    let (outer, w1, _) = doc
        .make_component(&[NodeId::Group(h)])
        .expect("outer component");
    doc.transform_instance(w1, &Transform::translation(Vec3::new(0.0, 0.0, 5.0)))
        .expect("lift outer");
    Fixture {
        doc,
        comp,
        a,
        b,
        i1,
        i2,
        i3,
        h,
        outer,
        w1,
    }
}

/// The world position of `o`'s minimum corner (its own frame) under the
/// placement `path`, through `hew.query.occurrences`' rows.
fn world_corner(doc: &Document, path: &[InstanceId], o: ObjectId) -> Point3 {
    let rows = doc.occurrences(None, true).expect("walk");
    let row = rows
        .iter()
        .find(|r| r.path == path && r.node == NodeId::Object(o))
        .unwrap_or_else(|| panic!("no row for {path:?} / {o:?}"));
    row.pose.apply_point(min_corner(doc, o))
}

/// The definition-frame transform a world gesture `t` becomes through a
/// placement whose composed pose is `pose`: `pose⁻¹ · t · pose`, applied as
/// `pose` first, then `t`, then `pose⁻¹` (the bus handler's computation).
fn conjugate(pose: &Transform, t: &Transform) -> Transform {
    pose.then(t)
        .then(&pose.inverse().expect("a placement pose inverts"))
}

// ------------------------------------------------------------ path_pose

#[test]
fn path_pose_composes_innermost_first_and_matches_the_render_walk() {
    let f = fixture();
    assert_eq!(f.doc.path_pose(&[]), Some(Transform::IDENTITY));
    assert_eq!(f.doc.path_pose(&[f.i2]), Some(mirror()));
    let nested = f
        .doc
        .path_pose(&[f.w1, f.i3])
        .expect("nested path resolves");
    let expected = f
        .doc
        .instance_pose(f.i3)
        .unwrap()
        .then(&f.doc.instance_pose(f.w1).unwrap());
    assert_eq!(nested, expected);
    // The render walk places member `a` through `w1` at the same pose.
    let placed = f
        .doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, outermost)| *o == f.a && *outermost == f.w1)
        .expect("a renders through w1");
    assert_eq!(placed.1, nested);
    // A chain that does not nest, or an inner instance alone, is None.
    assert_eq!(f.doc.path_pose(&[f.i1, f.i3]), None);
    assert_eq!(f.doc.path_pose(&[f.i3]), None);
}

// ------------------------------------------------------------ transforms

#[test]
fn member_moves_in_definition_space_and_every_placement_updates() {
    let mut f = fixture();
    let paths: [Vec<InstanceId>; 3] = [vec![f.i1], vec![f.i2], vec![f.w1, f.i3]];
    let before: Vec<Point3> = paths
        .iter()
        .map(|path| world_corner(&f.doc, path, f.a))
        .collect();
    let d = Vec3::new(0.25, 0.5, 0.0);
    let change = f
        .doc
        .transform_def_members(f.comp, &[NodeId::Object(f.a)], &Transform::translation(d))
        .expect("member moves");
    assert_eq!(change.components_touched, vec![f.comp]);
    for &i in &[f.i1, f.i2, f.w1] {
        assert!(change.instances_touched.contains(&i), "{i:?} re-resolves");
    }
    for (path, was) in paths.iter().zip(&before) {
        let pose = f.doc.path_pose(path).unwrap();
        let moved = world_corner(&f.doc, path, f.a);
        assert!(
            vec_approx(moved - *was, pose.apply_vector(d)),
            "{path:?}: moved {:?}, expected {:?}",
            moved - *was,
            pose.apply_vector(d)
        );
    }
    // The sibling member did not move.
    assert!(approx(min_corner(&f.doc, f.b), p(2.0, 0.0, 0.0)));
}

#[test]
fn world_intent_through_a_mirrored_path_lands_where_asked() {
    let mut f = fixture();
    let world_t = Transform::translation(Vec3::new(0.1, 0.0, 0.0));
    let pose = f.doc.path_pose(&[f.i2]).unwrap();
    let local = conjugate(&pose, &world_t);
    let mirrored_before = world_corner(&f.doc, &[f.i2], f.a);
    let plain_before = world_corner(&f.doc, &[f.i1], f.a);
    f.doc
        .transform_def_members(f.comp, &[NodeId::Object(f.a)], &local)
        .expect("member moves");
    // Under the mirrored placement it moved exactly as asked, +x; the
    // plain placement shows the mirror image of the move, −x.
    assert!(vec_approx(
        world_corner(&f.doc, &[f.i2], f.a) - mirrored_before,
        Vec3::new(0.1, 0.0, 0.0)
    ));
    assert!(vec_approx(
        world_corner(&f.doc, &[f.i1], f.a) - plain_before,
        Vec3::new(-0.1, 0.0, 0.0)
    ));
}

#[test]
fn conjugation_order_is_pose_then_t_then_inverse() {
    // Under the mirror, a world +y move is a local +y move and a world +x
    // move is a local −x move.
    let local = conjugate(&mirror(), &Transform::translation(Vec3::new(0.0, 1.0, 0.0)));
    assert!(approx(
        local.apply_point(p(0.0, 0.0, 0.0)),
        p(0.0, 1.0, 0.0)
    ));
    let tx = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    let local = conjugate(&mirror(), &tx);
    assert!(approx(
        local.apply_point(p(0.0, 0.0, 0.0)),
        p(-1.0, 0.0, 0.0)
    ));
    // A known point pins the order: under a quarter turn about z followed
    // by +5 along x, local (0,0,0) sits at world (5,0,0); a world +x move
    // takes it to world (6,0,0), which is local (0,−1,0).
    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(5.0, 0.0, 0.0)));
    assert!(approx(pose.apply_point(p(0.0, 0.0, 0.0)), p(5.0, 0.0, 0.0)));
    let local = conjugate(&pose, &tx);
    assert!(approx(
        local.apply_point(p(0.0, 0.0, 0.0)),
        p(0.0, -1.0, 0.0)
    ));
    // The other order (inverse first) lands elsewhere.
    let wrong = pose
        .inverse()
        .unwrap()
        .then(&tx)
        .then(&pose)
        .apply_point(p(0.0, 0.0, 0.0));
    assert!(!approx(wrong, p(0.0, -1.0, 0.0)));
}

#[test]
fn member_instance_composes_its_pose() {
    let mut f = fixture();
    let prev = f.doc.instance_pose(f.i3).unwrap();
    let a_before = min_corner(&f.doc, f.a);
    let t = Transform::translation(Vec3::new(0.0, 0.0, 2.0));
    let change = f
        .doc
        .transform_def_members(f.outer, &[NodeId::Instance(f.i3)], &t)
        .expect("member instance moves");
    assert_eq!(f.doc.instance_pose(f.i3).unwrap(), prev.then(&t));
    // Composed, never baked: the placed definition's geometry is untouched.
    assert!(approx(min_corner(&f.doc, f.a), a_before));
    assert_eq!(change.components_touched, vec![f.outer]);
    assert!(change.instances_touched.contains(&f.w1));
}

#[test]
fn member_group_flattens_to_leaves_once() {
    let mut f = fixture();
    // The member group and the instance inside it: composed once.
    let prev = f.doc.instance_pose(f.i3).unwrap();
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    f.doc
        .transform_def_members(f.outer, &[NodeId::Group(f.h), NodeId::Instance(f.i3)], &t)
        .expect("group and its member move");
    assert_eq!(f.doc.instance_pose(f.i3).unwrap(), prev.then(&t));

    // A member group of objects: its leaf listed beside it bakes once.
    let mut doc = Document::new();
    let c = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let d = make_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(c), NodeId::Object(d)])
        .expect("group");
    let (comp, _, _) = doc.make_component(&[NodeId::Group(g)]).expect("component");
    let change = doc
        .transform_def_members(comp, &[NodeId::Group(g), NodeId::Object(c)], &t)
        .expect("moves");
    assert!(approx(min_corner(&doc, c), p(1.0, 0.0, 0.0)));
    assert!(approx(min_corner(&doc, d), p(3.0, 0.0, 0.0)));
    assert_eq!(change.groups_touched, vec![g]);
}

#[test]
fn face_sketch_rides_with_its_member() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = make_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let s: SketchId = doc.add_sketch(plane_z(1.0));
    doc.begin_sketch_gesture(s).expect("begin");
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(p(0.2, 0.2, 1.0), p(0.4, 0.3, 1.0))
        .expect("segment");
    doc.end_sketch_gesture(s).expect("end");
    doc.set_sketch_object_owner(s, Some(a)).expect("own");
    let (comp, _, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("component");
    assert_eq!(doc.sketch_owner_component(s), Some(comp));
    let t = Transform::translation(Vec3::new(0.5, 0.0, 0.0));
    let change = doc
        .transform_def_members(comp, &[NodeId::Object(a)], &t)
        .expect("moves");
    assert!(change.sketches_touched.contains(&s));
    let verts: Vec<Point3> = doc
        .sketch(s)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    assert!(verts.iter().any(|&v| approx(v, p(0.7, 0.2, 1.0))));
    assert!(verts.iter().any(|&v| approx(v, p(0.9, 0.3, 1.0))));
}

// ------------------------------------------------------------- refusals

#[test]
fn non_member_refuses_untouched() {
    let mut f = fixture();
    let world = make_box(&mut f.doc, 40.0, 0.0, 41.0, 1.0, 1.0);
    let before = f.doc.save();
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        f.doc
            .transform_def_members(f.comp, &[NodeId::Object(world)], &t)
            .unwrap_err(),
        DocumentError::UnknownObject
    );
    // A member of another definition is not a member of this one.
    assert_eq!(
        f.doc
            .transform_def_members(f.comp, &[NodeId::Instance(f.i3)], &t)
            .unwrap_err(),
        DocumentError::UnknownInstance
    );
    assert_eq!(
        f.doc
            .transform_def_members(f.comp, &[NodeId::Group(f.h)], &t)
            .unwrap_err(),
        DocumentError::UnknownGroup
    );
    assert_eq!(
        f.doc
            .duplicate_def_members_array(f.comp, &[NodeId::Object(world)], &t, NonZeroU32::MIN)
            .unwrap_err(),
        DocumentError::UnknownObject
    );
    // A singular transform refuses typed too.
    assert!(matches!(
        f.doc.transform_def_members(
            f.comp,
            &[NodeId::Object(f.a)],
            &Transform::scale(Vec3::new(0.0, 1.0, 1.0))
        ),
        Err(DocumentError::Transform(_))
    ));
    assert_eq!(
        f.doc.save(),
        before,
        "refusals leave the document untouched"
    );
}

#[test]
fn on_loan_definition_refuses_explode_session_scope() {
    let mut f = fixture();
    f.doc
        .open_explode_session(f.i1)
        .expect("open the definition");
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        f.doc
            .transform_def_members(f.comp, &[NodeId::Object(f.a)], &t)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        f.doc
            .duplicate_def_members_array(f.comp, &[NodeId::Object(f.a)], &t, NonZeroU32::MIN)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert!(f.doc.path_on_loan(&[f.i2]));
    assert!(f.doc.path_on_loan(&[f.w1, f.i3]));
    assert!(!f.doc.path_on_loan(&[f.w1]));
}

#[test]
fn empty_or_all_hidden_refuses_empty_selection() {
    let mut f = fixture();
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        f.doc.transform_def_members(f.comp, &[], &t).unwrap_err(),
        DocumentError::EmptySelection
    );
    // A member group whose only content is deleted holds nothing visible.
    let mut doc = Document::new();
    let c = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let d = make_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(c)]).expect("group");
    let (comp, _, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(d)])
        .expect("component");
    doc.delete_def_member(comp, c)
        .expect("delete the grouped member");
    assert_eq!(
        doc.transform_def_members(comp, &[NodeId::Group(g)], &t)
            .unwrap_err(),
        DocumentError::EmptySelection
    );
}

// --------------------------------------------------------------- history

#[test]
fn undo_restores_bitwise_and_redo_reproduces() {
    let mut f = fixture();
    let before = f.doc.save();
    let depth = f.doc.undo_depth();
    let generation = f.doc.history_generation();
    let t = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.3)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.1, 0.2, 0.3)));
    f.doc
        .transform_def_members(f.comp, &[NodeId::Object(f.a), NodeId::Object(f.b)], &t)
        .expect("moves");
    assert_eq!(f.doc.undo_depth(), depth + 1, "one undo entry");
    // The bus change set names the definition as modified.
    let changes = f
        .doc
        .changes_since_generation(generation)
        .expect("the ring reaches back");
    assert!(
        changes
            .modified
            .iter()
            .any(|r| r.entity == kernel::EntityRef::Component(f.comp)),
        "{changes:?}"
    );
    let after = f.doc.save();
    let undo = f.doc.undo().expect("undo");
    assert_eq!(f.doc.save(), before, "undo restores the snapshots verbatim");
    assert_eq!(undo.components_touched, vec![f.comp]);
    for &i in &[f.i1, f.i2, f.w1] {
        assert!(undo.instances_touched.contains(&i), "undo names {i:?}");
    }
    let redo = f.doc.redo().expect("redo");
    assert_eq!(f.doc.save(), after, "redo reproduces bit for bit");
    assert_eq!(redo.components_touched, vec![f.comp]);
    assert!(redo.instances_touched.contains(&f.w1));

    // A world transform still reports no definition.
    let world = make_box(&mut f.doc, 40.0, 0.0, 41.0, 1.0, 1.0);
    f.doc
        .transform_selection(&[NodeId::Object(world)], &[], &t)
        .expect("world move");
    assert!(f.doc.undo().unwrap().components_touched.is_empty());
}

// ---------------------------------------------------------------- copies

#[test]
fn copies_land_in_the_definition_and_show_in_every_placement() {
    let mut f = fixture();
    let members_before = f.doc.def_member_nodes(f.comp).unwrap();
    let step = Transform::translation(Vec3::new(0.0, 2.0, 0.0));
    let (roots, change) = f
        .doc
        .duplicate_def_members_array(
            f.comp,
            &[NodeId::Object(f.a)],
            &step,
            NonZeroU32::new(2).unwrap(),
        )
        .expect("copies");
    assert_eq!(roots.len(), 2);
    assert_eq!(change.components_touched, vec![f.comp]);
    assert!(change.instances_touched.contains(&f.w1));
    let members = f.doc.def_member_nodes(f.comp).unwrap();
    assert_eq!(members.len(), members_before.len() + 2);
    assert_eq!(&members[members_before.len()..], &roots[..]);
    let copies: Vec<ObjectId> = roots
        .iter()
        .map(|r| match r {
            NodeId::Object(o) => *o,
            other => panic!("an object copy, got {other:?}"),
        })
        .collect();
    assert!(approx(min_corner(&f.doc, copies[0]), p(0.0, 2.0, 0.0)));
    assert!(approx(min_corner(&f.doc, copies[1]), p(0.0, 4.0, 0.0)));
    assert_eq!(f.doc.object_owner_component(copies[0]), Some(f.comp));
    // Every placement shows both copies.
    for &outermost in &[f.i1, f.i2, f.w1] {
        for &c in &copies {
            assert!(
                f.doc
                    .expanded_placements()
                    .iter()
                    .any(|(o, _, i)| *o == c && *i == outermost),
                "{c:?} renders through {outermost:?}"
            );
        }
    }

    let undo = f.doc.undo().expect("undo");
    assert_eq!(undo.components_touched, vec![f.comp]);
    assert!(undo.instances_touched.contains(&f.i2));
    assert_eq!(
        f.doc.def_member_nodes(f.comp).unwrap().len(),
        members_before.len() + 2,
        "undo hides the copies, members still lists them"
    );
    assert!(
        !f.doc
            .expanded_placements()
            .iter()
            .any(|(o, _, _)| copies.contains(o)),
        "hidden copies do not render"
    );
    f.doc.redo().expect("redo");
    assert!(
        f.doc
            .expanded_placements()
            .iter()
            .any(|(o, _, _)| *o == copies[1])
    );
}

#[test]
fn a_copy_inside_a_member_group_lands_in_that_group() {
    let mut f = fixture();
    let step = Transform::translation(Vec3::new(0.0, 3.0, 0.0));
    let (roots, _) = f
        .doc
        .duplicate_def_members_array(f.outer, &[NodeId::Instance(f.i3)], &step, NonZeroU32::MIN)
        .expect("copy the nested placement");
    let NodeId::Instance(copy) = roots[0] else {
        panic!("an instance copy")
    };
    assert!(f.doc.group_members(f.h).unwrap().contains(&roots[0]));
    assert_eq!(
        f.doc.instance_pose(copy).unwrap(),
        f.doc.instance_pose(f.i3).unwrap().then(&step)
    );
    // The definition renders once more through the outer placement.
    assert!(f.doc.path_pose(&[f.w1, copy]).is_some());
    // Listing a node twice is refused; so is a copy while a session is open.
    assert_eq!(
        f.doc
            .duplicate_def_members_array(
                f.comp,
                &[NodeId::Object(f.a), NodeId::Object(f.a)],
                &step,
                NonZeroU32::MIN
            )
            .unwrap_err(),
        DocumentError::DuplicateMember
    );
}

// ------------------------------------------------- booleans and slices

#[test]
fn subtract_through_a_mirrored_placement_cuts_every_placement() {
    let mut f = fixture();
    // A world cutter over the mirrored placement's image of member `a`:
    // `a` spans local x ∈ [0, 1], so world x ∈ [19, 20] under the mirror.
    let tool = make_box(&mut f.doc, 19.5, 0.25, 21.0, 0.75, 2.0);
    let depth = f.doc.undo_depth();
    let before = f.doc.save();
    let (result, change) = f
        .doc
        .boolean_through_placement(&[f.i2], f.a, tool, BooleanOp::Subtract)
        .expect("cut through the placement");
    assert_eq!(f.doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(change.components_touched, vec![f.comp]);
    assert!(change.instances_touched.contains(&f.w1));
    assert_eq!(f.doc.object_owner_component(result), Some(f.comp));
    assert!(!f.doc.is_world_object(tool), "the world tool is consumed");
    assert_eq!(f.doc.object(f.a).map(|_| ()), None, "the host is consumed");
    assert_eq!(
        f.doc.object_name(result),
        Some("Leg"),
        "the result keeps the host's name"
    );
    // In the definition's frame the cut removed local x ∈ [0, 0.5].
    let obj = f.doc.object(result).unwrap();
    let min_x = obj
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!((min_x - 0.0).abs() < 1e-9);
    assert!(
        obj.vertices()
            .values()
            .any(|v| v.position.approx_eq(p(0.5, 0.25, 0.0), 1e-9)),
        "the notch wall sits at local x = 0.5"
    );
    // Every placement shows the cut member.
    for &outermost in &[f.i1, f.i2, f.w1] {
        assert!(
            f.doc
                .expanded_placements()
                .iter()
                .any(|(o, _, i)| *o == result && *i == outermost)
        );
    }
    f.doc.undo().expect("undo");
    assert_eq!(f.doc.save(), before, "undo restores host and tool");
}

#[test]
fn boolean_and_slice_through_a_placement_refuse_typed() {
    let mut f = fixture();
    let tool = make_box(&mut f.doc, 0.5, 0.25, 0.75, 0.75, 2.0);
    let far = make_box(&mut f.doc, 50.0, 0.0, 51.0, 1.0, 1.0);
    let before = f.doc.save();
    let sub = BooleanOp::Subtract;
    assert_eq!(
        f.doc
            .boolean_through_placement(&[], f.a, tool, sub)
            .unwrap_err(),
        DocumentError::UnknownInstance
    );
    assert_eq!(
        f.doc
            .boolean_through_placement(&[f.i1], tool, far, sub)
            .unwrap_err(),
        DocumentError::UnknownObject,
        "a world host is not a member"
    );
    assert_eq!(
        f.doc
            .boolean_through_placement(&[f.i1], f.a, f.b, sub)
            .unwrap_err(),
        DocumentError::UnknownObject,
        "a member tool is not a world object"
    );
    assert!(matches!(
        f.doc.boolean_through_placement(&[f.i1], f.a, far, sub),
        Err(DocumentError::Boolean(_))
    ));
    assert_eq!(f.doc.save(), before);

    // The slice through the nested path: a world plane at the placement's
    // image of local x = 0.5.
    let pose = f.doc.path_pose(&[f.w1, f.i3]).unwrap();
    let origin = pose.apply_point(p(0.5, 0.5, 0.5));
    let normal = pose.apply_vector(Vec3::new(1.0, 0.0, 0.0));
    let plane = Plane::from_point_normal(origin, normal).unwrap();
    let ((pos, neg), change) = f
        .doc
        .slice_through_placement(&[f.w1, f.i3], f.a, &plane)
        .expect("slice through the nested path");
    assert_eq!(change.components_touched, vec![f.comp]);
    assert!(approx(min_corner(&f.doc, pos), p(0.5, 0.0, 0.0)));
    assert!(approx(min_corner(&f.doc, neg), p(0.0, 0.0, 0.0)));

    f.doc.open_explode_session(f.i1).expect("open");
    assert_eq!(
        f.doc
            .boolean_through_placement(&[f.i2], pos, tool, sub)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        f.doc
            .slice_through_placement(&[f.i2], pos, &plane)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
}

#[test]
fn union_or_intersect_with_a_world_tool_refuses_untouched() {
    // Addendum D §5 scopes the helper to subtract: a union or intersect
    // would move world material into every placement, which unlinking
    // is for.
    let mut f = fixture();
    let tool = make_box(&mut f.doc, 0.5, 0.25, 1.5, 0.75, 2.0);
    let before = f.doc.save();
    let depth = f.doc.undo_depth();
    for op in [BooleanOp::Union, BooleanOp::Intersect] {
        assert_eq!(
            f.doc
                .boolean_through_placement(&[f.i1], f.a, tool, op)
                .unwrap_err(),
            DocumentError::BooleanOperandHasInstance,
            "{op:?}"
        );
    }
    assert_eq!(f.doc.save(), before);
    assert_eq!(f.doc.undo_depth(), depth);
    assert!(f.doc.is_world_object(tool), "the tool is not consumed");
}

#[test]
fn a_boolean_between_two_members_through_a_path_runs_in_the_definition() {
    let mut f = fixture();
    let depth = f.doc.undo_depth();
    let (result, change) = f
        .doc
        .boolean_members_through_placement(&[f.w1, f.i3], f.a, f.b, BooleanOp::Union)
        .expect("union of two members through the nested path");
    assert_eq!(f.doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(f.doc.object_owner_component(result), Some(f.comp));
    assert_eq!(change.components_touched, vec![f.comp]);
    for &i in &[f.i1, f.i2] {
        assert!(change.instances_touched.contains(&i), "{i:?}");
    }
}

#[test]
fn a_boolean_between_members_refuses_while_any_session_is_open() {
    // A session on an unrelated definition is open: its close folds every
    // node created since its open into the open container, so the new
    // member would land under the wrong owner.
    let mut f = fixture();
    let other = make_box(&mut f.doc, 40.0, 0.0, 41.0, 1.0, 1.0);
    let (_, other_inst, _) = f
        .doc
        .make_component(&[NodeId::Object(other)])
        .expect("an unrelated component");
    f.doc.open_explode_session(other_inst).expect("open");
    let before = f.doc.save();
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        assert_eq!(
            f.doc
                .boolean_members_through_placement(&[f.i1], f.a, f.b, op)
                .unwrap_err(),
            DocumentError::ExplodeSessionScope,
            "{op:?}"
        );
    }
    assert_eq!(f.doc.save(), before);
}

#[test]
fn a_boolean_between_members_refuses_a_grouped_operand() {
    // Definition D: a member group G holding `a`, and a direct member `b`.
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let c = make_box(&mut doc, 5.0, 0.0, 6.0, 1.0, 1.0);
    let b = make_box(&mut doc, 0.5, 0.0, 1.5, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(c)])
        .expect("group");
    let (_comp, inst, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(b)])
        .expect("component");
    let before = doc.save();
    for (x, y) in [(a, b), (b, a)] {
        assert_eq!(
            doc.boolean_members_through_placement(&[inst], x, y, BooleanOp::Subtract)
                .unwrap_err(),
            DocumentError::GroupedOperand
        );
    }
    assert_eq!(doc.save(), before);
}

#[test]
fn undo_of_a_drawing_only_group_move_names_the_definition() {
    // A member group whose objects are all deleted, so it holds only a
    // drawing: moving it moves just the sketch.
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let keep = make_box(&mut doc, 5.0, 0.0, 6.0, 1.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let s = doc.add_sketch(plane_z(0.0));
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(p(10.0, 0.0, 0.0), p(12.0, 0.0, 0.0))
        .unwrap();
    doc.set_sketch_group(s, Some(g)).expect("hold the line");
    let (comp, i1, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(keep)])
        .expect("component");
    let (i2, _) = doc.place_instance(comp, mirror()).expect("second");
    doc.delete_def_member(comp, a)
        .expect("empty the group of solids");

    let t = Transform::translation(Vec3::new(0.0, 3.0, 0.0));
    let change = doc
        .transform_def_members(comp, &[NodeId::Group(g)], &t)
        .expect("the drawing moves");
    assert_eq!(change.sketches_touched, vec![s]);
    assert!(change.objects_touched.is_empty());

    let generation = doc.history_generation();
    let undo = doc.undo().expect("undo");
    assert_eq!(undo.components_touched, vec![comp]);
    for i in [i1, i2] {
        assert!(undo.instances_touched.contains(&i), "undo names {i:?}");
    }
    let redo = doc.redo().expect("redo");
    assert_eq!(redo.components_touched, vec![comp]);
    for i in [i1, i2] {
        assert!(redo.instances_touched.contains(&i), "redo names {i:?}");
    }
    let changes = doc
        .changes_since_generation(generation)
        .expect("the ring reaches back");
    assert!(
        changes
            .modified
            .iter()
            .any(|r| r.entity == kernel::EntityRef::Component(comp)),
        "{changes:?}"
    );
}

#[test]
fn sole_member_object_names_why_no_part_is_taken() {
    let f = fixture();
    assert_eq!(f.doc.sole_member_object(f.comp), Ok(None), "two parts");
    assert_eq!(
        f.doc.sole_member_object(f.outer),
        Err(DocumentError::GroupedOperand),
        "the one part is a group"
    );

    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (solo, inst, _) = doc.make_component(&[NodeId::Object(a)]).expect("solo");
    assert_eq!(doc.sole_member_object(solo), Ok(Some(a)));
    let (nest, _, _) = doc
        .make_component(&[NodeId::Instance(inst)])
        .expect("nested");
    assert_eq!(
        doc.sole_member_object(nest),
        Err(DocumentError::BooleanOperandHasInstance)
    );
    doc.undo().expect("undo the nesting");
    assert_eq!(
        doc.sole_member_object(nest),
        Err(DocumentError::UnknownComponent),
        "a stale definition"
    );
}
