//! Executable specs for Lock (OW-1), the enforcement half: what a lock
//! refuses, what it allows, and how it travels with copies.
//!
//! The contract under test (docs/agents/ARCHITECTURE.md, node model):
//! - a node is FROZEN when it is locked (an instance by its definition's
//!   lock), when a group above it is frozen, or when it is part of a shape
//!   whose contents are frozen (the shape is locked, or one of its
//!   placements is frozen — a copy placed inside a locked group);
//! - every edit that would modify or remove a frozen node refuses with
//!   `DocumentError::LockedNode { node, via_shared_shape }`, naming the
//!   locked node that is the cause, before any work: the document is
//!   untouched (same state hash, same history depth);
//! - `via_shared_shape` is set when the lock reaches the edit only through
//!   a shared shape (the cause is neither the target nor above it);
//! - allowed on a frozen node: reading, hiding and showing, copying,
//!   drawing on its face (callers open a face draw detached, a separate
//!   shape that does not cut it; a plane draft leaves it out), and Make
//!   Unique on a locked copy (a new shape with a lock of its own);
//! - a linked copy of a locked Object or group locks the set: the lock
//!   moves onto the new definition, so every placement carries it, and
//!   undo puts it back on the node;
//! - every committed edit passes the commit audit in a debug build (the
//!   audit's own specs live beside it in `document.rs`).
//!
//! DEVELOPMENT.md rule 3: the properties at the end.

use std::num::NonZeroU32;

use kernel::{
    AttrTarget, AttrValue, Axis, BooleanOp, ComponentId, Document, DocumentError, EntityRef,
    FaceId, FlipAbout, InstanceId, KernelOp, LinkPolicy, Material, MaterialScope, NodeId, ObjectId,
    Plane, Point3, Rgba8, Transform, Vec3,
};
use proptest::prelude::*;

// ------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A unit box at `x` on the ground.
fn board(doc: &mut Document, x: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    doc.extrude_region(s, region, 1.0).expect("extrude").0
}

fn shift(x: f64) -> Transform {
    Transform::translation(Vec3::new(x, 0.0, 0.0))
}

fn obj(o: ObjectId) -> NodeId {
    NodeId::Object(o)
}

fn ins(n: NodeId) -> InstanceId {
    match n {
        NodeId::Instance(i) => i,
        other => panic!("expected an instance, got {other:?}"),
    }
}

/// The box's top face (+z).
fn top_face(doc: &Document, o: ObjectId) -> FaceId {
    let object = doc.object(o).expect("live");
    object
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .expect("a box has a top face")
}

fn top_plane(doc: &Document, o: ObjectId) -> Plane {
    let f = top_face(doc, o);
    doc.object(o).expect("live").faces()[f].plane
}

fn face_count(doc: &Document, o: ObjectId) -> usize {
    doc.object(o).expect("live").faces().len()
}

/// Every vertex `node` renders, in its own frame (through placements),
/// sorted: a lock's "nothing changes on it", as geometry.
fn points(doc: &Document, node: NodeId) -> Vec<[i64; 3]> {
    let mut out = Vec::new();
    let mut stack = vec![(node, Transform::IDENTITY)];
    while let Some((n, xf)) = stack.pop() {
        match n {
            NodeId::Object(o) => {
                if let Some(object) = doc.object(o) {
                    for v in object.vertices().values() {
                        let p = xf.apply_point(v.position);
                        out.push([
                            (p.x * 1e6).round() as i64,
                            (p.y * 1e6).round() as i64,
                            (p.z * 1e6).round() as i64,
                        ]);
                    }
                }
            }
            NodeId::Group(g) => {
                for m in doc.group_members(g).unwrap_or_default() {
                    stack.push((m, xf));
                }
            }
            NodeId::Instance(i) => {
                if let (Some(pose), Some(def)) = (doc.instance_pose(i), doc.instance_def(i)) {
                    for m in doc.def_member_nodes(def).unwrap_or_default() {
                        stack.push((m, pose.then(&xf)));
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Runs `edit`, which must refuse with `LockedNode` naming `cause`
/// (`via` its indirect flag), and checks the document is untouched.
#[track_caller]
fn refused<T: std::fmt::Debug>(
    doc: &mut Document,
    cause: NodeId,
    via: bool,
    edit: impl FnOnce(&mut Document) -> Result<T, DocumentError>,
) {
    let hash = doc.state_hash();
    let depth = doc.undo_depth();
    match edit(doc) {
        Err(DocumentError::LockedNode {
            node,
            via_shared_shape,
        }) => {
            assert_eq!(node, cause, "the refusal names the cause");
            assert_eq!(via_shared_shape, via, "via_shared_shape");
        }
        other => panic!("expected LockedNode naming {cause:?}, got {other:?}"),
    }
    assert_eq!(
        doc.state_hash(),
        hash,
        "a refused edit leaves the document as it was"
    );
    assert_eq!(doc.undo_depth(), depth, "a refused edit records nothing");
}

/// A locked box: the document, the box, its node.
fn locked_box() -> (Document, ObjectId, NodeId) {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    doc.set_nodes_locked(&[obj(o)], true).unwrap();
    (doc, o, obj(o))
}

/// A board made a set of four linked copies: the four placements, the
/// shared definition, and the board (now its member).
fn four_legs(doc: &mut Document) -> (Vec<InstanceId>, ComponentId, ObjectId) {
    let o = board(doc, 0.0);
    let (copies, change) = doc
        .duplicate_nodes_array(
            &[obj(o)],
            &shift(2.0),
            NonZeroU32::new(3).unwrap(),
            LinkPolicy::Linked,
        )
        .expect("linked copies");
    let mut all = vec![change.linked[0].instance];
    all.extend(copies.into_iter().map(ins));
    (all, change.linked[0].definition, o)
}

/// A component placed twice: one copy inside a group that is then locked,
/// one outside. The shape is frozen through the inside copy.
struct SharedShape {
    doc: Document,
    def: ComponentId,
    member: ObjectId,
    inside: InstanceId,
    outside: InstanceId,
    group: NodeId,
}

fn shape_in_locked_group() -> SharedShape {
    let mut doc = Document::new();
    let member = board(&mut doc, 0.0);
    let (def, inside, _) = doc.make_component(&[obj(member)]).unwrap();
    let (outside, _) = doc.place_instance(def, shift(5.0)).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(inside)]).unwrap();
    let group = NodeId::Group(g);
    doc.set_nodes_locked(&[group], true).unwrap();
    SharedShape {
        doc,
        def,
        member,
        inside,
        outside,
        group,
    }
}

// --------------------------------------------- refused: placement

#[test]
fn a_locked_object_does_not_move_rotate_scale_or_flip() {
    let (mut doc, o, n) = locked_box();
    refused(&mut doc, n, false, |d| d.transform_object(o, &shift(1.0)));
    refused(&mut doc, n, false, |d| {
        d.transform_selection(&[n], &[], &shift(1.0))
    });
    refused(&mut doc, n, false, |d| {
        d.transform_selection(&[n], &[], &Transform::uniform_scale(2.0))
    });
    refused(&mut doc, n, false, |d| {
        d.flip_nodes(&[n], &[], Vec3::new(1.0, 0.0, 0.0), FlipAbout::Center)
    });
}

#[test]
fn a_locked_component_copy_does_not_move_or_flip() {
    let mut doc = Document::new();
    let (legs, _, _) = four_legs(&mut doc);
    let leg = NodeId::Instance(legs[1]);
    doc.set_nodes_locked(&[leg], true).unwrap();
    refused(&mut doc, leg, false, |d| {
        d.transform_instance(legs[1], &shift(1.0))
    });
    refused(&mut doc, leg, false, |d| d.flip_instance(legs[1], Axis::X));
    // Every copy is locked with it: another copy refuses, named itself.
    let other = NodeId::Instance(legs[3]);
    refused(&mut doc, other, false, |d| {
        d.transform_instance(legs[3], &shift(1.0))
    });
}

#[test]
fn an_unlocked_group_holding_a_locked_member_does_not_move_and_names_the_member() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (g, _) = doc.group_nodes(&[obj(a), obj(b)]).unwrap();
    doc.set_nodes_locked(&[obj(b)], true).unwrap();
    refused(&mut doc, obj(b), false, |d| {
        d.transform_group(g, &shift(1.0))
    });
    refused(&mut doc, obj(b), false, |d| {
        d.transform_selection(&[NodeId::Group(g)], &[], &shift(1.0))
    });
    // Its free member still moves on its own.
    doc.transform_object(a, &shift(1.0)).unwrap();
}

#[test]
fn whole_document_rescale_is_refused_while_anything_is_locked() {
    let (mut doc, _, n) = locked_box();
    refused(&mut doc, n, false, |d| d.rescale_document(2.0));
    doc.set_nodes_locked(&[n], false).unwrap();
    doc.rescale_document(2.0).expect("nothing locked: rescales");
}

// --------------------------------------------- refused: geometry

#[test]
fn a_locked_object_refuses_every_geometry_edit() {
    let (mut doc, o, n) = locked_box();
    let other = board(&mut doc, 0.5);
    let face = top_face(&doc, o);
    refused(&mut doc, n, false, |d| {
        d.apply_object_op(
            o,
            KernelOp::PushPull {
                face,
                distance: 0.5,
            },
        )
    });
    refused(&mut doc, n, false, |d| d.push_pull_through(o, face, -2.0));
    refused(&mut doc, n, false, |d| {
        d.boolean(BooleanOp::Union, other, o)
    });
    refused(&mut doc, n, false, |d| {
        d.boolean_nodes(BooleanOp::Subtract, obj(other), n)
    });
    let cut =
        Plane::from_point_normal(Point3::new(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    refused(&mut doc, n, false, |d| d.slice_node(o, &cut));
    refused(&mut doc, n, false, |d| d.remove_degenerate_faces(o));
    refused(&mut doc, n, false, |d| d.delete_faces(o, &[face]));
    refused(&mut doc, n, false, |d| d.set_smooth_level(o, 1));
    refused(&mut doc, n, false, |d| d.soften_by_angle(o, 0.1));
    let edges: Vec<_> = doc.object(o).unwrap().edges().keys().take(1).collect();
    refused(&mut doc, n, false, |d| d.set_crease(o, &edges, true));
}

#[test]
fn a_frozen_shapes_member_refuses_a_definition_edit() {
    let mut doc = Document::new();
    let (legs, def, member) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], true)
        .unwrap();
    let face = top_face(&doc, member);
    let cause = doc.node_frozen(obj(member)).expect("frozen");
    refused(&mut doc, cause, false, |d| {
        d.apply_def_op(
            def,
            member,
            KernelOp::PushPull {
                face,
                distance: 0.5,
            },
        )
    });
    refused(&mut doc, NodeId::Instance(legs[2]), false, |d| {
        d.transform_def_member(legs[2], member, &shift(0.5))
    });
    refused(&mut doc, cause, false, |d| {
        d.set_component_name(def, Some("Leg".into()))
    });
    refused(&mut doc, cause, false, |d| {
        d.rebase_definition(
            def,
            Point3::ORIGIN,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
        )
    });
}

/// A rebase re-expresses every member in the new frame, so a member that
/// carries its own lock refuses it, though the shape itself is not locked
/// (found by the document fuzz: the commit audit caught the rebase).
#[test]
fn a_rebase_is_refused_when_a_member_carries_its_own_lock() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (def, _, _) = doc.make_component(&[obj(a), obj(b)]).unwrap();
    doc.set_nodes_locked(&[obj(b)], true).unwrap();
    assert!(doc.def_frozen(def).is_none(), "the shape is not locked");
    refused(&mut doc, obj(b), false, |d| {
        d.rebase_definition(
            def,
            Point3::ORIGIN,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
        )
    });
}

/// Two boxes made a component, the second member locked on its own: the
/// document, the placement, the free member and the locked one. The shape
/// itself is not frozen, so only the member's own lock can refuse an edit.
fn component_with_a_locked_member() -> (Document, InstanceId, ObjectId, ObjectId) {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (def, inst, _) = doc.make_component(&[obj(a), obj(b)]).unwrap();
    doc.set_nodes_locked(&[obj(b)], true).unwrap();
    assert!(doc.def_frozen(def).is_none(), "the shape is not locked");
    (doc, inst, a, b)
}

/// A slice of a definition member consumes it, so a member that carries
/// its own lock refuses the slice, by placement and by path, though its
/// shape and placement are free (found by the document fuzz: the commit
/// audit caught `slice_def_member` cutting the locked member).
#[test]
fn a_locked_definition_member_refuses_a_slice() {
    let (mut doc, inst, a, b) = component_with_a_locked_member();
    let before = points(&doc, obj(b));
    let cut =
        Plane::from_point_normal(Point3::new(2.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    refused(&mut doc, obj(b), false, |d| {
        d.slice_def_member(inst, b, &cut)
    });
    refused(&mut doc, obj(b), false, |d| {
        d.slice_through_placement(&[inst], b, &cut)
    });
    assert_eq!(
        points(&doc, obj(b)),
        before,
        "the locked member is unchanged"
    );
    // The free member beside it still slices.
    let free_cut =
        Plane::from_point_normal(Point3::new(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    doc.slice_def_member(inst, a, &free_cut)
        .expect("a free member slices");
}

/// A subtract through a placement consumes its host member, so a host that
/// carries its own lock refuses it, with a world tool and with a transient
/// cutter alike.
#[test]
fn a_locked_definition_member_refuses_a_cut_through_its_placement() {
    let (mut doc, inst, _, b) = component_with_a_locked_member();
    let tool = board(&mut doc, 2.5);
    let cutter = doc.object(tool).unwrap().clone();
    let before = points(&doc, obj(b));
    refused(&mut doc, obj(b), false, |d| {
        d.boolean_through_placement(&[inst], b, tool, BooleanOp::Subtract)
    });
    refused(&mut doc, obj(b), false, |d| {
        d.subtract_solid_through_placement(&[inst], b, &cutter)
    });
    assert_eq!(
        points(&doc, obj(b)),
        before,
        "the locked member is unchanged"
    );
    doc.set_nodes_locked(&[obj(b)], false).unwrap();
    doc.boolean_through_placement(&[inst], b, tool, BooleanOp::Subtract)
        .expect("unlocked, the cut goes through");
}

/// An Object that is not a member of the addressed definition answers
/// `UnknownObject`, as the docs promise, even when it is locked: the
/// membership test comes before the member's own lock check.
#[test]
fn a_locked_non_member_is_unknown_not_locked() {
    let (mut doc, inst, _, _) = component_with_a_locked_member();
    let stranger = board(&mut doc, 6.0);
    doc.set_nodes_locked(&[obj(stranger)], true).unwrap();
    let cut =
        Plane::from_point_normal(Point3::new(6.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    assert!(matches!(
        doc.slice_def_member(inst, stranger, &cut),
        Err(DocumentError::UnknownObject)
    ));
    assert!(matches!(
        doc.slice_through_placement(&[inst], stranger, &cut),
        Err(DocumentError::UnknownObject)
    ));
    let cutter = doc.object(stranger).unwrap().clone();
    assert!(matches!(
        doc.subtract_solid_through_placement(&[inst], stranger, &cutter),
        Err(DocumentError::UnknownObject)
    ));
}

/// Painting a whole definition repaints every member Object, so a member
/// that carries its own lock, or sits in a locked member group, refuses
/// it though the shape itself is free (found by review: the commit audit
/// caught `set_definition_material` repainting the locked member).
#[test]
fn a_definition_paint_is_refused_by_a_locked_member_or_member_group() {
    let (mut doc, _, _, b) = component_with_a_locked_member();
    let def = doc.object_owner_component(b).unwrap();
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
    refused(&mut doc, obj(b), false, |d| {
        d.set_definition_material(def, Some(red))
    });
    doc.set_nodes_locked(&[obj(b)], false).unwrap();
    doc.set_definition_material(def, Some(red))
        .expect("unlocked, the shape repaints");

    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let c = board(&mut doc, 2.0);
    let (g, _) = doc.group_nodes(&[obj(c)]).unwrap();
    let group = NodeId::Group(g);
    let (def, _, _) = doc.make_component(&[obj(a), group]).unwrap();
    doc.set_nodes_locked(&[group], true).unwrap();
    assert!(doc.def_frozen(def).is_none(), "the shape is not locked");
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
    refused(&mut doc, group, false, |d| {
        d.set_definition_material(def, Some(red))
    });
}

// --------------------------------------------- refused: paint, names, tags, attributes

#[test]
fn a_locked_object_refuses_paint_rename_tags_and_attributes() {
    let (mut doc, o, n) = locked_box();
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
    let face = top_face(&doc, o);
    refused(&mut doc, n, false, |d| d.paint_face(o, face, Some(red)));
    refused(&mut doc, n, false, |d| d.set_object_material(o, Some(red)));
    refused(&mut doc, n, false, |d| {
        d.set_node_name(n, Some("Board".into()))
    });
    refused(&mut doc, n, false, |d| {
        d.add_node_tag(n, vec!["Walls".into()])
    });
    refused(&mut doc, n, false, |d| {
        d.attr_set(
            AttrTarget::Entity(EntityRef::Object(o)),
            "com.example",
            "k",
            AttrValue::Int(1),
        )
    });
    refused(&mut doc, n, false, |d| {
        d.face_attr_set(o, face, "com.example", "k", AttrValue::Int(1))
    });
}

#[test]
fn removing_a_tag_or_material_a_locked_thing_carries_is_refused() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let n = obj(o);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(200, 0, 0)));
    doc.set_object_material(o, Some(red)).unwrap();
    doc.add_node_tag(n, vec!["Walls".into()]).unwrap();
    doc.set_nodes_locked(&[n], true).unwrap();
    refused(&mut doc, n, false, |d| {
        d.remove_node_tag(n, &["Walls".to_string()])
    });
    refused(&mut doc, n, false, |d| d.delete_tag(&["Walls".to_string()]));
    refused(&mut doc, n, false, |d| {
        d.rename_tag(&["Walls".to_string()], vec!["Partitions".into()])
    });
    refused(&mut doc, n, false, |d| d.delete_material(red));
    refused(&mut doc, n, false, |d| {
        d.replace_material(MaterialScope::Document, Some(red), None)
    });
    // Editing the material in the palette is allowed.
    doc.set_material_alpha(red, 128)
        .expect("a palette edit is not an edit to the box");
}

// --------------------------------------------- refused: structure

#[test]
fn a_locked_object_refuses_delete_group_reparent_and_make_component() {
    let (mut doc, _, n) = locked_box();
    let free = obj(board(&mut doc, 3.0));
    let (g, _) = doc.group_nodes(&[free]).unwrap();
    refused(&mut doc, n, false, |d| d.delete_node(n));
    refused(&mut doc, n, false, |d| {
        d.delete_selection(&[free, n], &[], &[])
    });
    refused(&mut doc, n, false, |d| d.group_nodes(&[n]));
    refused(&mut doc, n, false, |d| d.reparent_nodes(&[n], Some(g)));
    refused(&mut doc, n, false, |d| d.make_component(&[n]));
}

#[test]
fn a_locked_group_refuses_ungroup_and_nothing_moves_in_or_out_of_it() {
    let mut doc = Document::new();
    let a = obj(board(&mut doc, 0.0));
    let b = obj(board(&mut doc, 3.0));
    let (g, _) = doc.group_nodes(&[a]).unwrap();
    let gn = NodeId::Group(g);
    doc.set_nodes_locked(&[gn], true).unwrap();
    refused(&mut doc, gn, false, |d| d.ungroup(g));
    refused(&mut doc, gn, false, |d| d.reparent_nodes(&[b], Some(g)));
    refused(&mut doc, gn, false, |d| d.reparent_nodes(&[a], None));
    refused(&mut doc, gn, false, |d| d.delete_node(a));
    refused(&mut doc, gn, false, |d| d.open_group_session(g));
    // An unlocked group holding a locked member is not dissolved either.
    doc.set_nodes_locked(&[gn], false).unwrap();
    doc.set_nodes_locked(&[a], true).unwrap();
    refused(&mut doc, a, false, |d| d.ungroup(g));
    refused(&mut doc, a, false, |d| d.delete_node(gn));
}

#[test]
fn a_locked_component_refuses_explode_open_delete_and_promote() {
    let mut doc = Document::new();
    let (legs, def, _) = four_legs(&mut doc);
    let leg = NodeId::Instance(legs[0]);
    doc.set_nodes_locked(&[leg], true).unwrap();
    refused(&mut doc, leg, false, |d| d.explode_instance(legs[0]));
    refused(&mut doc, leg, false, |d| d.open_explode_session(legs[0]));
    refused(&mut doc, leg, false, |d| d.delete_definition(def));
    refused(&mut doc, leg, false, |d| d.promote_linked(def));
    refused(&mut doc, leg, false, |d| d.make_component(&[leg]));
}

#[test]
fn join_linked_is_refused_whenever_a_lock_is_involved() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (set, _) = doc
        .duplicate_node(obj(o), &shift(2.0), LinkPolicy::Linked)
        .unwrap();
    let set = ins(set);
    let joiner = board(&mut doc, 5.0);
    // A locked joiner.
    doc.set_nodes_locked(&[obj(joiner)], true).unwrap();
    refused(&mut doc, obj(joiner), false, |d| {
        d.join_linked(set, joiner, shift(5.0))
    });
    doc.set_nodes_locked(&[obj(joiner)], false).unwrap();
    // A locked set.
    doc.set_nodes_locked(&[NodeId::Instance(set)], true)
        .unwrap();
    refused(&mut doc, NodeId::Instance(set), false, |d| {
        d.join_linked(set, joiner, shift(5.0))
    });
    doc.set_nodes_locked(&[NodeId::Instance(set)], false)
        .unwrap();
    // A set frozen through a placement inside a locked group.
    let (g, _) = doc.group_nodes(&[NodeId::Instance(set)]).unwrap();
    let other = doc
        .instances_of(doc.instance_def(set).unwrap())
        .into_iter()
        .find(|&i| i != set)
        .unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    refused(&mut doc, NodeId::Group(g), true, |d| {
        d.join_linked(other, joiner, shift(5.0))
    });
}

// --------------------------------------------- the shared shape (rulings, question A)

#[test]
fn an_edit_through_the_outside_copy_is_refused_naming_the_locked_group() {
    let SharedShape {
        mut doc,
        def,
        member,
        inside: _,
        outside,
        group,
    } = shape_in_locked_group();
    let face = top_face(&doc, member);
    refused(&mut doc, group, true, |d| {
        d.apply_def_op(
            def,
            member,
            KernelOp::PushPull {
                face,
                distance: 0.5,
            },
        )
    });
    refused(&mut doc, group, true, |d| {
        d.transform_def_member(outside, member, &shift(0.1))
    });
    // Opening it would let its members stop being the shape's.
    refused(&mut doc, group, true, |d| d.open_explode_session(outside));
    // Moving and deleting the outside copy work: the shape is untouched.
    doc.transform_instance(outside, &shift(1.0)).unwrap();
    doc.delete_node(NodeId::Instance(outside)).unwrap();
}

#[test]
fn make_unique_on_the_outside_copy_frees_it_and_leaves_the_group_unchanged() {
    let SharedShape {
        mut doc,
        def,
        member: _,
        inside: _,
        outside,
        group,
    } = shape_in_locked_group();
    let before = points(&doc, group);
    let (fresh, _) = doc.make_unique(outside).unwrap();
    assert_ne!(fresh, def);
    assert!(
        !doc.component_locked(fresh),
        "the outside copy was not locked"
    );
    assert_eq!(doc.node_frozen(NodeId::Instance(outside)), None);
    let copy = doc.def_members(fresh).unwrap()[0];
    let face = top_face(&doc, copy);
    doc.apply_def_op(
        fresh,
        copy,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .expect("its own shape now: edits");
    assert_eq!(points(&doc, group), before, "the locked group is unchanged");
}

#[test]
fn the_same_holds_for_a_shape_nested_in_a_locked_component() {
    let mut doc = Document::new();
    let member = board(&mut doc, 0.0);
    let (window, nested, _) = doc.make_component(&[obj(member)]).unwrap();
    let (outside, _) = doc.place_instance(window, shift(5.0)).unwrap();
    let (house, house_i, _) = doc.make_component(&[NodeId::Instance(nested)]).unwrap();
    let house_n = NodeId::Instance(house_i);
    doc.set_nodes_locked(&[house_n], true).unwrap();
    assert!(doc.component_locked(house));
    let face = top_face(&doc, member);
    refused(&mut doc, house_n, true, |d| {
        d.apply_def_op(
            window,
            member,
            KernelOp::PushPull {
                face,
                distance: 0.5,
            },
        )
    });
    let before = points(&doc, house_n);
    let (fresh, _) = doc.make_unique(outside).unwrap();
    let copy = doc.def_members(fresh).unwrap()[0];
    let face = top_face(&doc, copy);
    doc.apply_def_op(
        fresh,
        copy,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .unwrap();
    assert_eq!(points(&doc, house_n), before);
}

#[test]
fn locking_the_group_while_the_outside_copy_is_open_is_refused() {
    let mut doc = Document::new();
    let member = board(&mut doc, 0.0);
    let (def, inside, _) = doc.make_component(&[obj(member)]).unwrap();
    let (outside, _) = doc.place_instance(def, shift(5.0)).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(inside)]).unwrap();
    doc.open_explode_session(outside).unwrap();
    let hash = doc.state_hash();
    assert_eq!(
        doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(doc.state_hash(), hash);
    doc.close_explode_session().unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true)
        .expect("closed: locks");
}

#[test]
fn locking_inside_and_beside_an_open_group_is_allowed() {
    let mut doc = Document::new();
    let a = obj(board(&mut doc, 0.0));
    let b = obj(board(&mut doc, 3.0));
    let (g, _) = doc.group_nodes(&[a]).unwrap();
    doc.open_group_session(g).unwrap();
    // Locking a member of the open group, or something outside, is fine.
    doc.set_nodes_locked(&[a], true).unwrap();
    doc.set_nodes_locked(&[b], true).unwrap();
    doc.close_group_session().unwrap();
    assert!(doc.node_locked(a));
}

#[test]
fn make_unique_inside_a_locked_group_is_refused() {
    let SharedShape {
        mut doc,
        inside,
        group,
        ..
    } = shape_in_locked_group();
    refused(&mut doc, group, false, |d| d.make_unique(inside));
}

/// A window component placed inside a house component, the house placed
/// twice in the world. Nothing is locked yet.
struct NestedShape {
    doc: Document,
    window: ComponentId,
    /// The window's placement inside the house.
    nested: InstanceId,
    house: ComponentId,
    house_i: InstanceId,
    house_j: InstanceId,
}

fn nested_shape() -> NestedShape {
    let mut doc = Document::new();
    let member = board(&mut doc, 0.0);
    let (window, nested, _) = doc.make_component(&[obj(member)]).unwrap();
    let (house, house_i, _) = doc.make_component(&[NodeId::Instance(nested)]).unwrap();
    let (house_j, _) = doc.place_instance(house, shift(5.0)).unwrap();
    NestedShape {
        doc,
        window,
        nested,
        house,
        house_i,
        house_j,
    }
}

/// What a shape's records say: its member list and, for each member
/// placement, the definition it places and its pose. Make Unique must
/// leave a locked shape's records exactly as they were.
fn shape_records(doc: &Document, def: ComponentId) -> Vec<(NodeId, Option<ComponentId>, String)> {
    doc.def_member_nodes(def)
        .unwrap_or_default()
        .into_iter()
        .map(|m| match m {
            NodeId::Instance(i) => (
                m,
                doc.instance_def(i),
                format!("{:?}", doc.instance_pose(i)),
            ),
            _ => (m, None, String::new()),
        })
        .collect()
}

/// Make Unique by path never edits a locked shape's contents (review F4).
/// The locked copy itself may be made unique, deep included: everything
/// it repoints is a fresh copy made by the same call. A copy INSIDE a
/// locked shape is not a locked copy: a cascade that would repoint it
/// inside the shape's new copy is refused, as the in-place form is, so
/// the way out is the same as for a copy inside a locked group (unlock).
#[test]
fn make_unique_by_path_never_edits_a_locked_shapes_contents() {
    let NestedShape {
        mut doc,
        window,
        nested,
        house,
        house_i,
        house_j,
    } = nested_shape();
    doc.set_nodes_locked(&[NodeId::Instance(house_i)], true)
        .unwrap();
    let cause = NodeId::Instance(house_i);
    let records = shape_records(&doc, house);
    let other = points(&doc, NodeId::Instance(house_j));
    // The nested copy is inside the locked house: refused in every mode.
    for deep in [false, true] {
        refused(&mut doc, cause, false, |d| {
            d.make_unique_by_path(&[house_i, nested], true, deep)
        });
        refused(&mut doc, cause, false, |d| {
            d.make_unique_by_path(&[house_i, nested], false, deep)
        });
    }
    // The locked copy itself: allowed, one level and deep.
    let hash = doc.state_hash();
    for deep in [false, true] {
        let (fresh, leaf, remap, _) = doc
            .make_unique_by_path(&[house_i], true, deep)
            .expect("Make Unique works on a locked copy");
        assert_eq!(leaf, house_i);
        assert!(doc.component_locked(fresh), "the new shape keeps a lock");
        assert!(
            doc.component_locked(house),
            "the shape it left stays locked"
        );
        assert_eq!(
            shape_records(&doc, house),
            records,
            "the house is untouched"
        );
        assert_eq!(doc.instance_def(nested), Some(window));
        assert_eq!(points(&doc, NodeId::Instance(house_j)), other);
        let copy = remap.instances[&nested];
        assert_eq!(
            doc.instance_def(copy).is_some_and(|d| d != window),
            deep,
            "a deep call gives the copy a window of its own"
        );
        assert!(
            doc.node_frozen(NodeId::Instance(copy)).is_some(),
            "the copy inside the new locked shape is frozen"
        );
        doc.undo().unwrap();
        assert_eq!(doc.state_hash(), hash, "undo restores the document");
    }
}

/// A locked group inside a shape freezes the copy it holds: a cascade
/// through the shape is refused naming that group.
#[test]
fn make_unique_by_path_through_a_locked_member_group_is_refused() {
    let mut doc = Document::new();
    let member = board(&mut doc, 0.0);
    let (_, nested, _) = doc.make_component(&[obj(member)]).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(nested)]).unwrap();
    let (_, house_i, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    for (cascade, deep) in [(true, false), (true, true), (false, false)] {
        refused(&mut doc, NodeId::Group(g), false, |d| {
            d.make_unique_by_path(&[house_i, nested], cascade, deep)
        });
    }
}

/// The way out of a shape frozen through a copy in a locked group is
/// Make Unique on the outside copy (rulings, question A), and that holds
/// by path too: the cascade forks the outside copy's own house and
/// window, both free, and the locked group does not change. The in-place
/// form would edit the shared house, so it is refused naming the group.
#[test]
fn make_unique_by_path_through_the_outside_copy_of_a_frozen_shape_is_free() {
    let NestedShape {
        mut doc,
        nested,
        house,
        house_i,
        house_j,
        ..
    } = nested_shape();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(house_i)]).unwrap();
    let group = NodeId::Group(g);
    doc.set_nodes_locked(&[group], true).unwrap();
    refused(&mut doc, group, true, |d| {
        d.make_unique_by_path(&[house_j, nested], false, false)
    });
    let before = points(&doc, group);
    let records = shape_records(&doc, house);
    let (fresh, leaf, _, _) = doc
        .make_unique_by_path(&[house_j, nested], true, false)
        .expect("the outside copy is free");
    assert!(!doc.component_locked(fresh));
    assert!(doc.node_frozen(NodeId::Instance(leaf)).is_none());
    assert!(doc.node_frozen(NodeId::Instance(house_j)).is_none());
    assert_eq!(points(&doc, group), before, "the locked group is unchanged");
    assert_eq!(shape_records(&doc, house), records);
}

/// A locked linked set made unique placement by placement: each new
/// shape keeps the lock, and the last one takes the emptied set with it
/// (hidden, its lock kept), all through the commit audit; undo walks
/// back to the start.
#[test]
fn make_unique_by_path_on_every_copy_of_a_locked_linked_set() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (copy, change) = doc
        .duplicate_node(obj(o), &shift(2.0), LinkPolicy::Linked)
        .unwrap();
    let first = change.linked[0].instance;
    let def = change.linked[0].definition;
    doc.set_nodes_locked(&[copy], true).unwrap();
    let hash = doc.state_hash();
    let (a, _, _, _) = doc.make_unique_by_path(&[first], true, true).unwrap();
    let (b, _, _, _) = doc.make_unique_by_path(&[ins(copy)], true, false).unwrap();
    assert!(doc.component_locked(a) && doc.component_locked(b));
    assert!(doc.node_locked(NodeId::Instance(first)));
    assert!(doc.node_locked(copy));
    assert!(
        !doc.component_locked(def),
        "the emptied set went with its last copy"
    );
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), hash);
    assert!(doc.component_locked(def));
}

// --------------------------------------------- copies (rulings, question B)

#[test]
fn linked_copy_of_a_locked_object_locks_the_set() {
    let (mut doc, o, n) = locked_box();
    let hash = doc.state_hash();
    let (copy, change) = doc
        .duplicate_node(n, &shift(2.0), LinkPolicy::Linked)
        .unwrap();
    let original = change.linked[0].instance;
    let def = change.linked[0].definition;
    assert!(doc.component_locked(def), "the lock moved onto the shape");
    assert!(doc.node_locked(NodeId::Instance(original)));
    assert!(doc.node_locked(copy));
    assert!(!doc.node_locked(n), "the member carries no lock of its own");
    assert!(doc.node_frozen(n).is_some(), "the member is frozen");
    refused(&mut doc, NodeId::Instance(original), false, |d| {
        d.transform_instance(original, &shift(1.0))
    });
    refused(&mut doc, copy, false, |d| {
        d.transform_instance(ins(copy), &shift(1.0))
    });
    // Unlocking through the copy unlocks both, with no stranded lock.
    doc.set_nodes_locked(&[copy], false).unwrap();
    assert!(!doc.node_locked(NodeId::Instance(original)));
    assert_eq!(doc.node_frozen(n), None);
    doc.undo().unwrap();
    // Undo of the copy returns a standalone locked Object, same id.
    doc.undo().unwrap();
    assert!(doc.node_locked(n));
    assert_eq!(doc.owner_definition(n), None);
    assert_eq!(doc.state_hash(), hash, "the pre-copy state");
    doc.redo().unwrap();
    assert!(doc.component_locked(def));
    assert!(!doc.node_locked(n));
    let _ = o;
}

#[test]
fn linked_copy_of_a_locked_group_locks_the_set() {
    let mut doc = Document::new();
    let a = obj(board(&mut doc, 0.0));
    let (g, _) = doc.group_nodes(&[a]).unwrap();
    let gn = NodeId::Group(g);
    doc.set_nodes_locked(&[gn], true).unwrap();
    let hash = doc.state_hash();
    let (copy, change) = doc
        .duplicate_node(gn, &shift(2.0), LinkPolicy::Linked)
        .unwrap();
    let def = change.linked[0].definition;
    assert!(doc.component_locked(def));
    assert!(doc.node_locked(copy));
    assert!(!doc.node_locked(gn));
    assert!(doc.node_frozen(a).is_some());
    doc.undo().unwrap();
    assert!(doc.node_locked(gn));
    assert_eq!(doc.state_hash(), hash);
}

#[test]
fn an_array_and_a_flipped_copy_of_a_locked_object_lock_the_set() {
    let (mut doc, _, n) = locked_box();
    let (copies, change) = doc
        .duplicate_nodes_array(
            &[n],
            &shift(2.0),
            NonZeroU32::new(2).unwrap(),
            LinkPolicy::Linked,
        )
        .unwrap();
    let def = change.linked[0].definition;
    assert!(doc.component_locked(def));
    for c in copies {
        assert!(doc.node_locked(c));
    }
    let (mut doc, _, n) = locked_box();
    let (copies, change) = doc
        .flip_copy(
            &[n],
            Vec3::new(1.0, 0.0, 0.0),
            FlipAbout::Origin,
            LinkPolicy::Linked,
        )
        .unwrap();
    assert!(doc.component_locked(change.linked[0].definition));
    assert!(doc.node_locked(copies[0]));
}

#[test]
fn a_bare_wrap_then_a_placement_is_one_locked_set() {
    let (mut doc, _, n) = locked_box();
    let (def, inst, _) = doc.wrap_linked(n).unwrap();
    let (placed, _) = doc.place_instance(def, shift(3.0)).unwrap();
    assert!(doc.node_locked(NodeId::Instance(inst)));
    assert!(doc.node_locked(NodeId::Instance(placed)));
    assert!(!doc.node_locked(n));
}

#[test]
fn a_separate_copy_of_a_locked_object_is_not_locked() {
    let (mut doc, _, n) = locked_box();
    let (copy, _) = doc
        .duplicate_node(n, &shift(3.0), LinkPolicy::Independent)
        .unwrap();
    assert!(!doc.node_locked(copy));
    assert!(doc.node_locked(n));
}

#[test]
fn a_copy_of_a_node_inside_a_locked_group_is_refused_naming_the_group() {
    let mut doc = Document::new();
    let a = obj(board(&mut doc, 0.0));
    let (g, _) = doc.group_nodes(&[a]).unwrap();
    let gn = NodeId::Group(g);
    doc.set_nodes_locked(&[gn], true).unwrap();
    refused(&mut doc, gn, false, |d| {
        d.duplicate_node(a, &shift(2.0), LinkPolicy::Linked)
    });
    refused(&mut doc, gn, false, |d| {
        d.duplicate_node(a, &shift(2.0), LinkPolicy::Independent)
    });
    refused(&mut doc, gn, false, |d| d.wrap_linked(a));
}

#[test]
fn locking_a_linked_sets_member_by_its_path_locks_the_set() {
    let mut doc = Document::new();
    let (legs, def, member) = four_legs(&mut doc);
    doc.set_nodes_locked(&[obj(member)], true).unwrap();
    assert!(doc.component_locked(def));
    assert!(!doc.node_locked(obj(member)));
    for leg in legs {
        assert!(doc.node_locked(NodeId::Instance(leg)));
    }
    doc.set_nodes_locked(&[obj(member)], false).unwrap();
    assert!(!doc.component_locked(def));
}

#[test]
fn a_file_with_a_lock_on_a_linked_sets_member_opens_with_the_set_locked() {
    use std::io::{Cursor, Read as _, Write as _};
    let mut doc = Document::new();
    let (legs, _, _) = four_legs(&mut doc);
    let bytes = doc.save();
    // Put the flag on the member Object instead of the definition.
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let mut m: serde_json::Value = serde_json::from_str(&text).unwrap();
    let member_id = m["components"][0]["members"][0]["id"].clone();
    assert_eq!(m["components"][0]["members"][0]["kind"], "object");
    let objects = m["objects"].as_array_mut().unwrap();
    let member = objects
        .iter_mut()
        .find(|o| o["id"] == member_id)
        .expect("the set's member Object");
    member["locked"] = serde_json::json!(true);
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        if name == "manifest.json" {
            buf = serde_json::to_vec_pretty(&m).unwrap();
        }
        out.start_file(name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    let patched = out.finish().unwrap().into_inner();
    let loaded = Document::load(&patched).expect("loads");
    let placements: Vec<NodeId> = loaded
        .top_level_nodes()
        .into_iter()
        .filter(|n| matches!(n, NodeId::Instance(_)))
        .collect();
    assert_eq!(placements.len(), legs.len());
    for p in placements {
        assert!(loaded.node_locked(p), "the lock is hoisted onto the shape");
    }
}

// --------------------------------------------- allowed

#[test]
fn hiding_showing_and_reading_a_locked_object_work() {
    let (mut doc, o, n) = locked_box();
    doc.set_node_user_hidden(n, true);
    assert!(doc.node_user_hidden(n));
    doc.set_node_user_hidden(n, false);
    assert!(doc.object(o).is_some());
    assert!(doc.node_locked(n));
}

#[test]
fn drawing_on_a_locked_face_gives_a_free_drawing_that_does_not_cut_it() {
    let (mut doc, o, n) = locked_box();
    let faces = face_count(&doc, o);
    let plane = top_plane(&doc, o);
    let participants = doc.draft_participants(plane, None);
    assert!(!participants.contains(&o), "a frozen Object takes no part");
    doc.draft(plane, None, &participants, "Rectangle", |w| {
        let c = [
            Point3::new(0.25, 0.25, 1.0),
            Point3::new(0.75, 0.25, 1.0),
            Point3::new(0.75, 0.75, 1.0),
            Point3::new(0.25, 0.75, 1.0),
        ];
        for i in 0..4 {
            w.add_segment(c[i], c[(i + 1) % 4])?;
        }
        Ok(())
    })
    .expect("drawing on a locked face works");
    assert_eq!(face_count(&doc, o), faces, "the face is not cut");
    assert!(
        doc.object_owned_sketches(o).is_empty(),
        "nothing rides with it"
    );
    assert!(
        doc.sketch_ids()
            .iter()
            .any(|&s| { doc.sketch(s).is_some_and(|sk| !sk.edges().is_empty()) }),
        "a free drawing"
    );
    // Naming the locked Object as a participant is refused instead.
    refused(&mut doc, n, false, |d| {
        d.begin_draft(plane, None, &[o], "Line")
    });
}

#[test]
fn a_face_draw_detaches_on_a_locked_object_and_one_in_a_locked_group() {
    let (mut doc, o, n) = locked_box();
    assert!(doc.face_draw_detaches(o), "a locked Object");
    doc.set_nodes_locked(&[n], false).unwrap();
    assert!(!doc.face_draw_detaches(o), "unlocked, it is cut as usual");

    let inner = board(&mut doc, 3.0);
    let (g, _) = doc.group_nodes(&[obj(inner)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    assert!(doc.face_draw_detaches(inner), "an Object in a locked group");
    assert!(!doc.face_draw_detaches(o), "its unlocked neighbor");
    doc.set_nodes_locked(&[NodeId::Group(g)], false).unwrap();
    assert!(!doc.face_draw_detaches(inner), "after the group's unlock");
}

/// A rectangle drawn on a locked box's top face as callers open it there
/// (detached): the box is untouched; after unlock its face lists whole,
/// and the shape pulls into a new Object.
#[test]
fn a_detached_rectangle_on_a_locked_face_leaves_the_box_and_pulls_into_a_new_object() {
    // Strictly inside the face, and exactly on the face's own corners.
    for (lo, hi) in [(0.25, 0.75), (0.0, 1.0)] {
        let (mut doc, o, n) = locked_box();
        let plane = top_plane(&doc, o);
        let print0 = points(&doc, n);
        let faces = face_count(&doc, o);
        assert!(doc.face_draw_detaches(o));
        let (report, _) = doc
            .draft_detached_new(plane, None, "Rectangle", |w| {
                let c = [
                    Point3::new(lo, lo, 1.0),
                    Point3::new(hi, lo, 1.0),
                    Point3::new(hi, hi, 1.0),
                    Point3::new(lo, hi, 1.0),
                ];
                for i in 0..4 {
                    w.add_segment(c[i], c[(i + 1) % 4])?;
                }
                Ok(())
            })
            .expect("a detached draw on a locked face works");
        assert!(report.objects.is_empty(), "no object changes: {report:?}");
        assert_eq!(report.sketches.len(), 1, "one new sketch: {report:?}");
        let s = report.sketches[0];
        assert!(doc.sketch(s).is_some_and(|sk| sk.detached()));
        assert_eq!(points(&doc, n), print0, "the box is unchanged ({lo})");
        assert_eq!(face_count(&doc, o), faces, "the face is not cut ({lo})");
        assert!(doc.node_frozen(n).is_some(), "still locked");

        doc.set_nodes_locked(&[n], false).unwrap();
        let participants = doc.draft_participants(plane, None);
        assert_eq!(participants, vec![o]);
        let pu = doc.plane_units(plane, None, &participants).unwrap();
        assert_eq!(pu.regions.len(), 1, "the face alone ({lo}): {pu:?}");
        assert!(pu.regions[0].holes.is_empty(), "no hole in it ({lo})");
        assert!(
            pu.regions
                .iter()
                .all(|r| matches!(r.over, Some((ob, Some(_))) if ob == o)),
            "every region is the box's face ({lo}): {pu:?}"
        );

        let own = doc.detached_sketch_units(s).unwrap();
        assert_eq!(own.regions.len(), 1);
        let (sk, rid) = own.regions[0].extrude.expect("extrudes as its own");
        let (fresh, _) = doc.extrude_region(sk, rid, 0.5).expect("extrudes");
        assert_ne!(fresh, o, "a new Object");
        assert_eq!(doc.visible_object_ids().len(), 2, "two objects ({lo})");
        assert_eq!(points(&doc, n), print0, "the box is still unchanged");
        assert_eq!(face_count(&doc, o), faces);
    }
}

/// The edges of `sketch` as rounded endpoint pairs, sorted.
fn sketch_print(doc: &Document, sketch: kernel::SketchId) -> Vec<[i64; 6]> {
    let sk = doc.sketch(sketch).expect("live sketch");
    let q = |x: f64| (x * 1e6).round() as i64;
    let mut out: Vec<[i64; 6]> = sk
        .edges()
        .values()
        .map(|e| {
            let (a, b) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
            let (a, b) = if (a.x, a.y, a.z) <= (b.x, b.y, b.z) {
                (a, b)
            } else {
                (b, a)
            };
            [q(a.x), q(a.y), q(a.z), q(b.x), q(b.y), q(b.z)]
        })
        .collect();
    out.sort_unstable();
    out
}

/// A closed square of lines on the ground, from `lo` to `hi`, held by a
/// group (a group's own drawing).
fn grouped_square(doc: &mut Document, lo: f64, hi: f64) -> (kernel::SketchId, kernel::GroupId) {
    let anchor = board(doc, 10.0);
    let (g, _) = doc.group_nodes(&[obj(anchor)]).unwrap();
    let s = doc.add_sketch(ground());
    let c = [
        Point3::new(lo, lo, 0.0),
        Point3::new(hi, lo, 0.0),
        Point3::new(hi, hi, 0.0),
        Point3::new(lo, hi, 0.0),
    ];
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
    }
    doc.set_sketch_group(s, Some(g)).unwrap();
    (s, g)
}

/// The rectangle from `lo` to `hi` drawn on the ground as a plane draft.
fn ground_rect(doc: &mut Document, lo: f64, hi: f64) -> kernel::DraftReport {
    let participants = doc.draft_participants(ground(), None);
    doc.draft(ground(), None, &participants, "Rectangle", |w| {
        let c = [
            Point3::new(lo, lo, 0.0),
            Point3::new(hi, lo, 0.0),
            Point3::new(hi, hi, 0.0),
            Point3::new(lo, hi, 0.0),
        ];
        for i in 0..4 {
            w.add_segment(c[i], c[(i + 1) % 4])?;
        }
        Ok(())
    })
    .expect("a plane draft beside locked drawing works")
    .0
}

/// A locked group's own lines are frozen with it: a plane draft that
/// crosses them leaves them alone (never merged into or split against
/// them), the new drawing is a separate shape that closes as drawn, and
/// the plane's listing keeps the frozen lines apart as context so its
/// editable keys are the draft's. After unlock the next draft arranges
/// with them as before.
#[test]
fn a_plane_draft_across_a_locked_groups_lines_leaves_them_alone() {
    let mut doc = Document::new();
    let (held, g) = grouped_square(&mut doc, 0.0, 2.0);
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    let print0 = sketch_print(&doc, held);
    let regions0 = doc.sketch(held).unwrap().regions().len();

    let report = ground_rect(&mut doc, 1.0, 3.0);
    assert!(!report.sketches.contains(&held), "{report:?}");
    assert_eq!(sketch_print(&doc, held), print0, "the group's lines");
    assert_eq!(doc.sketch(held).unwrap().regions().len(), regions0);
    assert_eq!(doc.sketch_owner_group(held), Some(g));
    // The new drawing: one separate sketch, the rectangle whole, one face.
    assert_eq!(report.sketches.len(), 1, "{report:?}");
    let drawn = report.sketches[0];
    assert_eq!(sketch_print(&doc, drawn).len(), 4, "not split");
    assert_eq!(doc.sketch(drawn).unwrap().regions().len(), 1);

    // The plane's arrangement is the unfrozen drawing; the frozen lines
    // list apart, as context.
    let participants = doc.draft_participants(ground(), None);
    let plane = doc.plane_units(ground(), None, &participants).unwrap();
    assert_eq!(plane.units.len(), 1, "{plane:?}");
    assert_eq!(plane.units[0].edges.len(), 4, "{plane:?}");
    let listing = doc.free_plane_units(None, &Default::default());
    let on_ground: Vec<_> = listing
        .iter()
        .filter(|(l, _)| !matches!(l, kernel::PlaneListing::Detached { .. }))
        .collect();
    assert_eq!(on_ground.len(), 2, "{listing:?}");
    let (_, listed) = on_ground
        .iter()
        .find(|(l, _)| matches!(l, kernel::PlaneListing::Plane(_)))
        .expect("the draft's entry");
    assert_eq!(listed, &plane, "the listing's keys are the draft's");
    let (_, context) = on_ground
        .iter()
        .find(|(l, _)| l.is_context())
        .expect("the frozen lines' entry");
    assert_eq!(context.units[0].edges.len(), 4, "{context:?}");

    // Unlocked, a draft arranges with the group's lines again.
    doc.set_nodes_locked(&[NodeId::Group(g)], false).unwrap();
    let participants = doc.draft_participants(ground(), None);
    let joined = doc.plane_units(ground(), None, &participants).unwrap();
    let edges: usize = joined.units.iter().map(|u| u.edges.len()).sum();
    assert_eq!(edges, 12, "the crossings split both squares: {joined:?}");
}

/// A plane whose only free drawing is frozen still lists what a draft on
/// it lifts: an unlocked box's top face shares its plane with a locked
/// group's lines, and the plane's draft entry is the box face (what a
/// plain plane draft there cuts), the frozen lines apart as context.
#[test]
fn an_unlocked_face_beside_only_frozen_lines_lists_as_the_planes_draft() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let top = top_plane(&doc, o);
    // The locked group's square on the box's top plane, across the face.
    let anchor = board(&mut doc, 10.0);
    let (g, _) = doc.group_nodes(&[obj(anchor)]).unwrap();
    let held = doc.add_sketch(top);
    let c = [
        Point3::new(0.5, 0.5, 1.0),
        Point3::new(1.5, 0.5, 1.0),
        Point3::new(1.5, 1.5, 1.0),
        Point3::new(0.5, 1.5, 1.0),
    ];
    let sk = doc.sketch_mut(held).unwrap();
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
    }
    doc.set_sketch_group(held, Some(g)).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    let print0 = sketch_print(&doc, held);

    // The plane's entries, apart from detached ones.
    let on_top = |doc: &Document| -> Vec<(kernel::PlaneListing, kernel::PlaneUnits)> {
        doc.free_plane_units(None, &Default::default())
            .into_iter()
            .filter(|(l, _)| {
                !matches!(l, kernel::PlaneListing::Detached { .. })
                    && (l.plane().normal() - top.normal()).length() < 1e-9
                    && l.plane().signed_distance(Point3::new(0.0, 0.0, 1.0)).abs() < 1e-9
            })
            .collect()
    };
    let draft_entry = |doc: &Document| -> kernel::PlaneUnits {
        let participants = doc.draft_participants(top, None);
        doc.plane_units(top, None, &participants).unwrap()
    };

    // Locked: the draft's entry is the box face, keyed as the draft's.
    let participants = doc.draft_participants(top, None);
    assert_eq!(participants, vec![o], "the unlocked box takes part");
    let entries = on_top(&doc);
    assert_eq!(
        entries.len(),
        2,
        "the draft's and the frozen lines': {entries:?}"
    );
    let (_, listed) = entries
        .iter()
        .find(|(l, _)| matches!(l, kernel::PlaneListing::Plane(_)))
        .expect("the draft's entry, though the plane's only free drawing is frozen");
    assert_eq!(
        listed,
        &draft_entry(&doc),
        "the listing's keys are the draft's"
    );
    assert!(
        listed
            .regions
            .iter()
            .any(|r| matches!(r.over, Some((ob, Some(_))) if ob == o)),
        "the box face's region is listed: {listed:?}"
    );
    assert!(listed.units.is_empty(), "no frozen line in it: {listed:?}");
    let (_, context) = entries
        .iter()
        .find(|(l, _)| l.is_context())
        .expect("the frozen lines' entry");
    let frozen_edges: usize = context.units.iter().map(|u| u.edges.len()).sum();
    assert_eq!(frozen_edges, 4, "the square whole: {context:?}");

    // A plain plane draft over the box face still cuts it; the group's
    // lines are untouched.
    let faces0 = face_count(&doc, o);
    let (report, _) = doc
        .draft(top, None, &participants, "Rectangle", |w| {
            let c = [
                Point3::new(0.1, 0.1, 1.0),
                Point3::new(0.4, 0.1, 1.0),
                Point3::new(0.4, 0.4, 1.0),
                Point3::new(0.1, 0.4, 1.0),
            ];
            for i in 0..4 {
                w.add_segment(c[i], c[(i + 1) % 4])?;
            }
            Ok(())
        })
        .expect("a plane draft over the unlocked face works");
    assert!(report.objects.contains(&o), "{report:?}");
    assert!(!report.sketches.contains(&held), "{report:?}");
    assert_eq!(face_count(&doc, o), faces0 + 1, "the face is imprinted");
    assert_eq!(sketch_print(&doc, held), print0, "the group's lines");
    let entries = on_top(&doc);
    let (_, listed) = entries
        .iter()
        .find(|(l, _)| matches!(l, kernel::PlaneListing::Plane(_)))
        .expect("the draft's entry after the draft");
    assert_eq!(listed, &draft_entry(&doc), "still the draft's keys");

    // Unlocked: one arrangement, the group's lines with the box face.
    doc.set_nodes_locked(&[NodeId::Group(g)], false).unwrap();
    let entries = on_top(&doc);
    assert_eq!(entries.len(), 1, "no context left: {entries:?}");
    assert!(matches!(entries[0].0, kernel::PlaneListing::Plane(_)));
    assert_eq!(entries[0].1, draft_entry(&doc), "the draft's keys");
    let joined = &entries[0].1;
    let group_edges = joined
        .units
        .iter()
        .flat_map(|u| &u.edges)
        .filter(|e| e.store == kernel::DraftStore::Free(held))
        .count();
    assert_eq!(
        group_edges, 6,
        "the box face's edges split the square where they cross: {joined:?}"
    );
}

/// The same with the group inside a locked group: the outer lock freezes
/// the inner group's lines.
#[test]
fn a_plane_draft_leaves_lines_in_a_group_inside_a_locked_group_alone() {
    let mut doc = Document::new();
    let (held, g) = grouped_square(&mut doc, 0.0, 2.0);
    let other = board(&mut doc, 20.0);
    let (outer, _) = doc.group_nodes(&[NodeId::Group(g), obj(other)]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(outer)], true).unwrap();
    let print0 = sketch_print(&doc, held);
    let report = ground_rect(&mut doc, 1.0, 3.0);
    assert!(!report.sketches.contains(&held), "{report:?}");
    assert_eq!(sketch_print(&doc, held), print0);
}

#[test]
fn make_unique_on_a_locked_copy_unlocks_alone_and_undoes_step_by_step() {
    let mut doc = Document::new();
    let (legs, def, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], true)
        .unwrap();
    let (fresh, _) = doc.make_unique(legs[3]).unwrap();
    assert!(doc.component_locked(fresh) && doc.component_locked(def));
    doc.set_nodes_locked(&[NodeId::Instance(legs[3])], false)
        .unwrap();
    let copy = doc.def_members(fresh).unwrap()[0];
    let face = top_face(&doc, copy);
    let others: Vec<_> = legs[..3]
        .iter()
        .map(|&l| points(&doc, NodeId::Instance(l)))
        .collect();
    doc.apply_def_op(
        fresh,
        copy,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .expect("its own unlocked shape edits");
    for (k, &l) in legs[..3].iter().enumerate() {
        assert!(doc.node_locked(NodeId::Instance(l)));
        assert_eq!(points(&doc, NodeId::Instance(l)), others[k], "unchanged");
    }
    doc.undo().unwrap();
    assert!(!doc.node_locked(NodeId::Instance(legs[3])));
    doc.undo().unwrap();
    assert!(doc.node_locked(NodeId::Instance(legs[3])));
    doc.undo().unwrap();
    assert_eq!(doc.instance_def(legs[3]), Some(def));
}

/// Unlink is Make Unique for linked copies (OW-1): it works on a locked
/// copy, the copy becomes a plain Object with a lock of its own, and the
/// copies left behind keep theirs. A set left with one copy is no set: that
/// copy is a plain locked Object too.
#[test]
fn unlinking_a_locked_copy_gives_it_a_lock_of_its_own() {
    let mut doc = Document::new();
    let (legs, def, _) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], true)
        .unwrap();
    let before = doc.state_hash();
    let others: Vec<_> = legs[..3]
        .iter()
        .map(|&l| points(&doc, NodeId::Instance(l)))
        .collect();
    let (roots, _) = doc.unlink_copies(&[legs[3]]).unwrap();
    let [NodeId::Object(made)] = roots[..] else {
        panic!("one Object, got {roots:?}");
    };
    assert!(doc.node_locked(obj(made)), "locked, with its own lock");
    assert!(doc.component_locked(def), "the set keeps its lock");
    for (k, &l) in legs[..3].iter().enumerate() {
        assert!(doc.node_locked(NodeId::Instance(l)));
        assert_eq!(points(&doc, NodeId::Instance(l)), others[k], "unchanged");
    }
    // Unlocking it leaves the other three locked.
    doc.set_nodes_locked(&[obj(made)], false).unwrap();
    assert!(
        legs[..3]
            .iter()
            .all(|&l| doc.node_locked(NodeId::Instance(l)))
    );
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before, "undo restores the locked set");

    // Down to two, then one more: both are plain and locked.
    doc.unlink_copies(&[legs[3], legs[2]]).unwrap();
    let (roots, _) = doc.unlink_copies(&[legs[1]]).unwrap();
    assert_eq!(roots.len(), 1);
    assert!(doc.instance_ids().is_empty(), "no set of one is left");
    let all = doc.visible_object_ids();
    assert_eq!(all.len(), 4);
    assert!(
        all.iter().all(|&o| doc.node_locked(obj(o))),
        "each keeps the lock"
    );
    let re = Document::load(&doc.save()).unwrap();
    assert!(
        re.visible_object_ids()
            .iter()
            .all(|&o| re.node_locked(obj(o)))
    );
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

/// A copy of a locked set is never deleted or exploded (OW-1), so a delete
/// or an explode never reaches the last-copy bake with the set locked:
/// each is refused before any work, with two copies as with four, and so is
/// deleting a group that holds one.
#[test]
fn deleting_or_exploding_a_copy_of_a_locked_set_of_two_is_refused() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (copies, change) = doc
        .duplicate_nodes_array(
            &[obj(o)],
            &shift(2.0),
            NonZeroU32::new(1).unwrap(),
            LinkPolicy::Linked,
        )
        .unwrap();
    let first = change.linked[0].instance;
    let second = ins(copies[0]);
    let leg = NodeId::Instance(second);
    let (g, _) = doc.group_nodes(&[leg]).unwrap();
    doc.set_nodes_locked(&[NodeId::Instance(first)], true)
        .unwrap();
    refused(&mut doc, leg, false, |d| d.delete_node(leg));
    refused(&mut doc, leg, false, |d| {
        d.delete_selection(&[leg], &[], &[])
    });
    refused(&mut doc, leg, false, |d| d.explode_instance(second));
    refused(&mut doc, leg, false, |d| d.explode_instances(&[second]));
    refused(&mut doc, leg, false, |d| d.delete_node(NodeId::Group(g)));
    refused(&mut doc, NodeId::Instance(first), false, |d| {
        d.delete_node(NodeId::Instance(first))
    });
}

/// A set whose own lock is off can hold a locked node deeper in its shape
/// (a member group's member): deleting one of its two copies makes the
/// other plain, and the locked node's copy keeps the lock, as Unlink
/// carries it. Undo restores the set exactly.
#[test]
fn the_last_copy_made_plain_after_a_delete_keeps_a_locked_members_lock() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 0.0);
    doc.transform_object(b, &Transform::translation(Vec3::new(0.0, 0.0, 1.0)))
        .unwrap();
    let (g, _) = doc.group_nodes(&[obj(a), obj(b)]).unwrap();
    doc.set_nodes_locked(&[obj(b)], true).unwrap();
    let (copies, change) = doc
        .duplicate_nodes_array(
            &[NodeId::Group(g)],
            &shift(3.0),
            NonZeroU32::new(1).unwrap(),
            LinkPolicy::Linked,
        )
        .unwrap();
    let def = change.linked[0].definition;
    assert!(!doc.component_locked(def), "the set itself is not locked");
    let before = doc.state_hash();
    doc.delete_node(copies[0]).unwrap();
    assert!(doc.instance_ids().is_empty(), "no set of one is left");
    let locked: Vec<ObjectId> = doc
        .visible_object_ids()
        .into_iter()
        .filter(|&o| doc.node_locked(obj(o)))
        .collect();
    assert_eq!(locked.len(), 1, "the locked member's copy is locked");
    assert_eq!(doc.visible_object_ids().len(), 2);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
}

/// A copy inside a locked group is not "a locked copy": unlinking it
/// rewrites the group, so it refuses naming the group. The copy left
/// behind in a locked group when its sibling is unlinked stays where it is,
/// a placement of a set of one.
#[test]
fn a_copy_in_a_locked_group_is_not_unlinked_and_stays_behind() {
    let mut doc = Document::new();
    let (legs, def, _) = four_legs(&mut doc);
    let (g, _) = doc.group_nodes(&[NodeId::Instance(legs[0])]).unwrap();
    doc.set_nodes_locked(&[NodeId::Group(g)], true).unwrap();
    refused(&mut doc, NodeId::Group(g), false, |d| {
        d.unlink_copies(&[legs[0]])
    });
    let inside = points(&doc, NodeId::Group(g));
    doc.unlink_copies(&[legs[1], legs[2], legs[3]]).unwrap();
    assert_eq!(doc.instances_of(def), vec![legs[0]], "it stays where it is");
    assert_eq!(points(&doc, NodeId::Group(g)), inside);
    assert_eq!(doc.definition_usage(def), 1, "which reads as not linked");
}

/// Ctrl/Cmd Push/Pull (a new Object from a face) changes nothing that
/// exists, so it works on a locked box and through a locked copy, as
/// copying does (OW-1); the new Object is not locked.
#[test]
fn a_new_object_from_a_locked_face_leaves_the_locked_thing_alone() {
    let (mut doc, o, n) = locked_box();
    let print0 = points(&doc, n);
    let face = top_face(&doc, o);
    let (made, _) = doc
        .extrude_face_as_new_object(o, face, 0.5)
        .expect("a new Object from a locked face");
    assert_eq!(points(&doc, n), print0);
    assert!(doc.node_locked(n));
    assert!(doc.node_frozen(obj(made)).is_none(), "the new one is free");

    let mut doc = Document::new();
    let (legs, def, member) = four_legs(&mut doc);
    doc.set_nodes_locked(&[NodeId::Instance(legs[0])], true)
        .unwrap();
    let prints: Vec<_> = legs
        .iter()
        .map(|&l| points(&doc, NodeId::Instance(l)))
        .collect();
    let face = top_face(&doc, member);
    let (made, _) = doc
        .extrude_face_as_new_object_in_instance(legs[2], member, face, 0.5)
        .expect("a new Object through a locked copy");
    for (k, &l) in legs.iter().enumerate() {
        assert_eq!(points(&doc, NodeId::Instance(l)), prints[k]);
    }
    assert!(doc.component_locked(def));
    assert!(doc.node_frozen(obj(made)).is_none());
}

#[test]
fn exploding_an_unlocked_component_carries_its_members_locks() {
    // A component holding a locked member, locked by its path.
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 2.0);
    let (_, i, _) = doc.make_component(&[obj(a), obj(b)]).unwrap();
    doc.set_nodes_locked(&[obj(b)], true).unwrap();
    assert!(!doc.node_locked(NodeId::Instance(i)));
    let (created, _) = doc
        .explode_instance(i)
        .expect("an unfrozen placement explodes");
    let locked: Vec<_> = created
        .iter()
        .filter(|&&o| doc.node_locked(obj(o)))
        .collect();
    assert_eq!(locked.len(), 1, "the member's lock rides onto its copy");
}

#[test]
fn an_unfrozen_placement_of_a_frozen_shape_explodes() {
    let SharedShape {
        mut doc,
        outside,
        group,
        ..
    } = shape_in_locked_group();
    let before = points(&doc, group);
    doc.explode_instance(outside)
        .expect("the definition is untouched");
    assert_eq!(points(&doc, group), before);
}

#[test]
fn purge_and_delete_leave_a_definition_holding_a_lock() {
    let mut doc = Document::new();
    let o = board(&mut doc, 0.0);
    let (def, i, _) = doc.make_component(&[obj(o)]).unwrap();
    doc.delete_node(NodeId::Instance(i)).unwrap();
    // Unused now; its member is locked through its path.
    doc.set_nodes_locked(&[obj(o)], true).unwrap();
    refused(&mut doc, obj(o), false, |d| d.delete_definition(def));
    doc.purge_unused().unwrap();
    assert!(
        doc.def_members(def).is_some(),
        "a definition holding a lock is not purged"
    );
}

#[test]
fn the_refusal_says_how_to_get_out() {
    let (doc, _, n) = locked_box();
    let direct = DocumentError::LockedNode {
        node: n,
        via_shared_shape: false,
    };
    let indirect = DocumentError::LockedNode {
        node: n,
        via_shared_shape: true,
    };
    assert!(direct.to_string().contains("unlock"));
    assert!(indirect.to_string().contains("unique"));
    let _ = doc;
}

// ------------------------------------------------------- properties

/// The edits the property drives, each on a pick of the nodes.
#[derive(Debug, Clone)]
enum Step {
    Lock(usize, bool),
    Move(usize),
    Rename(usize),
    Delete(usize),
    PushPull(usize),
    CopyLinked(usize),
    CopySeparate(usize),
    Group(usize),
    Ungroup(usize),
    MakeUnique(usize),
    Undo,
    Redo,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0usize..16, any::<bool>()).prop_map(|(i, b)| Step::Lock(i, b)),
        (0usize..16).prop_map(Step::Move),
        (0usize..16).prop_map(Step::Rename),
        (0usize..16).prop_map(Step::Delete),
        (0usize..16).prop_map(Step::PushPull),
        (0usize..16).prop_map(Step::CopyLinked),
        (0usize..16).prop_map(Step::CopySeparate),
        (0usize..16).prop_map(Step::Group),
        (0usize..16).prop_map(Step::Ungroup),
        (0usize..16).prop_map(Step::MakeUnique),
        Just(Step::Undo),
        Just(Step::Redo),
    ]
}

/// What a lock keeps: geometry where it stands, name and tags.
fn keep(doc: &Document, n: NodeId) -> (Vec<[i64; 3]>, Option<String>, Vec<Vec<String>>) {
    let name = match n {
        NodeId::Object(o) => doc.object_name(o),
        NodeId::Group(g) => doc.group_name(g),
        NodeId::Instance(i) => doc.instance_name(i),
    };
    (
        points(doc, n),
        name.map(str::to_string),
        doc.node_tags(n).to_vec(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any sequence of edits over locked and unlocked boards, groups and
    /// linked copies: a refused edit is `LockedNode` and leaves the state
    /// hash as it was, and every node frozen before and after a forward
    /// step keeps its geometry, name and tags (undo and redo move between
    /// accepted states, so they are left out of the check).
    #[test]
    fn a_frozen_node_never_changes(steps in prop::collection::vec(step(), 1..24)) {
        let mut doc = Document::new();
        for k in 0..3 {
            board(&mut doc, 3.0 * k as f64);
        }
        for s in steps {
            let nodes = doc.top_level_nodes();
            if nodes.is_empty() {
                break;
            }
            let pick = |i: usize| nodes[i % nodes.len()];
            let frozen_before: Vec<(NodeId, _)> = doc
                .visible_object_ids()
                .into_iter()
                .map(NodeId::Object)
                .chain(nodes.iter().copied())
                .filter(|&n| doc.node_frozen(n).is_some())
                .map(|n| (n, keep(&doc, n)))
                .collect();
            let hash = doc.state_hash();
            let forward = !matches!(s, Step::Undo | Step::Redo);
            let s_kind = Some(matches!(s, Step::MakeUnique(_)));
            let result: Result<(), DocumentError> = match s {
                Step::Lock(i, b) => doc.set_nodes_locked(&[pick(i)], b).map(|_| ()),
                Step::Move(i) => doc.transform_selection(&[pick(i)], &[], &shift(0.5)).map(|_| ()),
                Step::Rename(i) => doc.set_node_name(pick(i), Some(format!("n{i}"))).map(|_| ()),
                Step::Delete(i) => doc.delete_node(pick(i)).map(|_| ()),
                Step::PushPull(i) => match pick(i) {
                    NodeId::Object(o) => {
                        let f = top_face(&doc, o);
                        doc.apply_object_op(o, KernelOp::PushPull { face: f, distance: 0.25 })
                            .map(|_| ())
                    }
                    _ => Ok(()),
                },
                Step::CopyLinked(i) => doc
                    .duplicate_node(pick(i), &shift(1.0), LinkPolicy::Linked)
                    .map(|_| ()),
                Step::CopySeparate(i) => doc
                    .duplicate_node(pick(i), &shift(1.0), LinkPolicy::Independent)
                    .map(|_| ()),
                Step::Group(i) => doc.group_nodes(&[pick(i)]).map(|_| ()),
                Step::Ungroup(i) => match pick(i) {
                    NodeId::Group(g) => doc.ungroup(g).map(|_| ()),
                    _ => Ok(()),
                },
                Step::MakeUnique(i) => match pick(i) {
                    NodeId::Instance(inst) => doc.make_unique(inst).map(|_| ()),
                    _ => Ok(()),
                },
                Step::Undo => doc.undo().map(|_| ()),
                Step::Redo => doc.redo().map(|_| ()),
            };
            if let Err(DocumentError::LockedNode { .. }) = result {
                prop_assert_eq!(doc.state_hash(), hash, "a refusal changes nothing");
            }
            if forward {
                // Make Unique promotes a copy's own name onto its new
                // shape: the one edit allowed on a locked copy.
                let unique = matches!(s_kind, Some(true));
                for (n, kept) in frozen_before {
                    if doc.node_frozen(n).is_some() {
                        let now = keep(&doc, n);
                        prop_assert_eq!(&now.0, &kept.0, "{:?} moved while frozen", n);
                        prop_assert_eq!(&now.2, &kept.2, "{:?} retagged while frozen", n);
                        if !(unique && matches!(n, NodeId::Instance(_))) {
                            prop_assert_eq!(&now.1, &kept.1, "{:?} renamed while frozen", n);
                        }
                    }
                }
            }
        }
    }
}
