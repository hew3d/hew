//! Executable specs for **group editing sessions**
//! (docs/design/group-session.md).
//!
//! The contract under test:
//! - `open_group_session` applies the document's ungroup posture to a
//!   live, top-level group — every direct member's parent cleared to the
//!   top level, the group hidden with its member list intact — so the
//!   ordinary plain-object tool set works on the members unmodified, the
//!   replacing ops included (nothing is a `GroupedOperand` mid-session).
//!   No pose is involved: nothing bakes, nothing drifts.
//! - `close_group_session` re-homes the survivors in their original
//!   order, folds in every node created mid-session that is still live
//!   and top-level, drops members deleted or consumed mid-session, and
//!   deletes the group outright when nothing survived (the emptied
//!   posture, matching the component close and plain ungroup).
//! - sessions stack LIFO; a component frame is always innermost; a nested
//!   group refuses a direct open (drill down through its ancestors).
//! - open/close are ordinary, granular history actions; persistence
//!   mid-session serializes as if the whole stack were closed.
//! - whole-document rescale refuses while any frame is open.

use kernel::{BooleanOp, Document, DocumentError, NodeId, ObjectId, Plane, Point3};

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// Draws an axis-aligned rectangle directly into `s` (bypassing gestures —
/// undo grouping is not what these specs are about), returning its sole
/// region.
fn draw_rect(
    doc: &mut Document,
    s: kernel::SketchId,
    corners: [Point3; 4],
) -> kernel::SketchRegionId {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        sk.add_segment(corners[i], corners[(i + 1) % 4])
            .expect("rectangle segment");
    }
    doc.extrudable_regions(s).expect("sketch is live")[0]
}

/// Builds an axis-aligned box `[x0,x1] x [y0,y1] x [z0,z1]` as a fresh world
/// object via a ground sketch + extrusion, returning its `ObjectId`.
fn build_box(doc: &mut Document, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let region = draw_rect(
        doc,
        s,
        [
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
        ],
    );
    let (id, _) = doc.extrude_region(s, region, z1 - z0).expect("extrude box");
    id
}

/// Two disjoint boxes grouped — the common fixture. Returns
/// `(group, box_a, box_b)`.
fn two_box_group(doc: &mut Document) -> (kernel::GroupId, ObjectId, ObjectId) {
    let a = build_box(doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group the boxes");
    (group, a, b)
}

// --------------------------------------------------------------- spec 1

/// **Open = the ungroup posture; close = the exact regroup.** Members
/// surface at the top level for the session's duration (same ids — a
/// move, not a copy), the group disappears from the tree, and the close
/// restores membership in the original order with nothing else changed.
#[test]
fn open_applies_the_ungroup_posture_and_close_restores_membership() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(group));

    doc.open_group_session(group).expect("open");
    assert_eq!(doc.session_stack(), vec![NodeId::Group(group)]);
    assert_eq!(doc.group_session_group(), Some(group));
    assert_eq!(doc.node_parent(NodeId::Object(a)), None);
    assert_eq!(doc.node_parent(NodeId::Object(b)), None);
    let top = doc.top_level_nodes();
    assert!(top.contains(&NodeId::Object(a)) && top.contains(&NodeId::Object(b)));
    assert!(
        !top.contains(&NodeId::Group(group)),
        "the open group is hidden for the session's duration"
    );

    doc.close_group_session().expect("close");
    assert!(doc.session_stack().is_empty());
    assert_eq!(
        doc.group_members(group).expect("group is back"),
        vec![NodeId::Object(a), NodeId::Object(b)],
        "membership and order restored exactly"
    );
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(group));
    assert_eq!(doc.node_parent(NodeId::Object(b)), Some(group));
}

// --------------------------------------------------------------- spec 2

/// **The replacing ops work mid-session** (the stress-test defect this
/// design exists for): a boolean between two members — a `GroupedOperand`
/// refusal outside a session — succeeds mid-session, and its result folds
/// into the group at close, replacing the consumed operands.
#[test]
fn boolean_between_members_mid_session_folds_the_result_into_the_group() {
    let mut doc = Document::new();
    // Overlapping boxes so the union is a genuine merge.
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(&mut doc, 1.0, 0.0, 0.0, 3.0, 2.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group the boxes");

    // Outside a session the kernel refuses the grouped operands.
    assert_eq!(
        doc.boolean(BooleanOp::Union, a, b).unwrap_err(),
        DocumentError::GroupedOperand
    );

    doc.open_group_session(group).expect("open");
    let (result, _) = doc.boolean(BooleanOp::Union, a, b).expect("union works");
    doc.close_group_session().expect("close");

    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Object(result)],
        "consumed operands drop from membership; the result folds in"
    );
    assert_eq!(doc.node_parent(NodeId::Object(result)), Some(group));
}

// --------------------------------------------------------------- spec 3

/// **What you draw while editing goes into the group**: an object built
/// mid-session (sketch + extrude) folds in at close, after the surviving
/// original members.
#[test]
fn a_drawn_object_folds_into_the_group_at_close() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);

    doc.open_group_session(group).expect("open");
    let fresh = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    doc.close_group_session().expect("close");

    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Object(a), NodeId::Object(b), NodeId::Object(fresh)],
        "survivors keep their order; the fold-in appends"
    );
    assert_eq!(doc.node_parent(NodeId::Object(fresh)), Some(group));
}

// --------------------------------------------------------------- spec 4

/// **A group created mid-session folds in as one node**: grouping two
/// members re-homes them under the new group, so the close sees them as
/// non-top-level (they stay put) and folds the new group itself in.
#[test]
fn group_nodes_mid_session_folds_the_new_group_not_its_members() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(&mut doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let c = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)])
        .expect("group all three");

    doc.open_group_session(group).expect("open");
    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("grouping session members is legal in a group session");
    doc.close_group_session().expect("close");

    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Object(c), NodeId::Group(inner)],
        "the survivor keeps its slot; the new group folds in as one node"
    );
    assert_eq!(doc.node_parent(NodeId::Group(inner)), Some(group));
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(inner));
}

// --------------------------------------------------------------- spec 5

/// **An emptied group does not survive its close** — every member deleted
/// mid-session, nothing folded in: the group is deleted outright
/// (tombstoned, id stable), exactly like the component close's emptied
/// definition and plain ungroup.
#[test]
fn deleting_every_member_deletes_the_group_at_close() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);

    doc.open_group_session(group).expect("open");
    doc.delete_node(NodeId::Object(a)).expect("delete a");
    doc.delete_node(NodeId::Object(b)).expect("delete b");
    doc.close_group_session().expect("close");

    assert_eq!(doc.group_members(group), None, "the group is gone");
    assert!(!doc.top_level_nodes().contains(&NodeId::Group(group)));

    // Fully undoable: undo the close (re-opens the session), then the two
    // deletes — the members are live top-level session geometry again.
    doc.undo().expect("undo close");
    assert_eq!(doc.session_stack(), vec![NodeId::Group(group)]);
    doc.undo().expect("undo delete b");
    doc.undo().expect("undo delete a");
    assert_eq!(doc.node_parent(NodeId::Object(a)), None);
    doc.close_group_session().expect("close again");
    assert_eq!(
        doc.group_members(group).expect("group is back"),
        vec![NodeId::Object(a), NodeId::Object(b)]
    );
}

// --------------------------------------------------------------- spec 6

/// **Sessions nest and close LIFO**: opening an outer group frees its
/// nested group to the top level, which can then open its own session;
/// each close folds into its own group only.
#[test]
fn nested_group_sessions_close_lifo() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(&mut doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("inner group");
    let c = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .expect("outer group");

    // A nested group refuses a direct open — drill down.
    assert_eq!(
        doc.open_group_session(inner).unwrap_err(),
        DocumentError::ExplodeSessionNestedGroup
    );

    doc.open_group_session(outer).expect("open outer");
    doc.open_group_session(inner)
        .expect("inner is top-level inside the outer session");
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Group(outer), NodeId::Group(inner)]
    );

    // Draw inside the INNER session; it must fold into `inner`, not
    // `outer` — the inner close re-homes it first, so the outer close's
    // top-level filter never sees it.
    let fresh = build_box(&mut doc, 12.0, 0.0, 0.0, 13.0, 1.0, 1.0);
    doc.close_group_session().expect("close inner");
    assert_eq!(
        doc.group_members(inner).expect("inner survives"),
        vec![NodeId::Object(a), NodeId::Object(b), NodeId::Object(fresh)]
    );

    doc.close_group_session().expect("close outer");
    assert_eq!(
        doc.group_members(outer).expect("outer survives"),
        vec![NodeId::Group(inner), NodeId::Object(c)],
        "the outer close folds nothing extra — the inner close already re-homed it"
    );
    assert!(doc.session_stack().is_empty());
}

// --------------------------------------------------------------- spec 7

/// **A component frame is always innermost**: a component session opens
/// on top of a group session, and while it is open nothing else opens;
/// whole-document rescale refuses while ANY frame is open.
#[test]
fn component_frames_stack_innermost_and_rescale_refuses_throughout() {
    let mut doc = Document::new();
    let (group, _a, _b) = two_box_group(&mut doc);
    let lone = build_box(&mut doc, 10.0, 0.0, 0.0, 11.0, 1.0, 1.0);
    let (_component, instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("make a component from the lone box");
    let (other_group, _) = {
        let d = build_box(&mut doc, 14.0, 0.0, 0.0, 15.0, 1.0, 1.0);
        doc.group_nodes(&[NodeId::Object(d)]).expect("other group")
    };

    doc.open_group_session(group).expect("open the group");
    assert_eq!(
        doc.rescale_document(2.0).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "whole-document rescale refuses under a group frame"
    );

    doc.open_explode_session(instance)
        .expect("a component session opens on top of a group frame");
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Group(group), NodeId::Instance(instance)]
    );
    assert_eq!(
        doc.open_group_session(other_group).unwrap_err(),
        DocumentError::ExplodeSessionOpen,
        "nothing opens inside a component frame"
    );
    assert_eq!(
        doc.rescale_document(2.0).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );

    doc.close_innermost_session().expect("close the component");
    assert_eq!(doc.session_stack(), vec![NodeId::Group(group)]);
    doc.close_innermost_session().expect("close the group");
    assert!(doc.session_stack().is_empty());
    doc.rescale_document(2.0)
        .expect("rescale works again once the stack is empty");
}

// --------------------------------------------------------------- spec 8

/// **Open and close are granular history actions**: undo steps back
/// through the close (re-entering the session), through the mid-session
/// edit, and through the open; redo replays all three to the identical
/// end state.
#[test]
fn undo_steps_granularly_through_the_session_boundary() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);

    doc.open_group_session(group).expect("open");
    let fresh = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    doc.close_group_session().expect("close");
    let closed_members = doc.group_members(group).expect("closed membership");
    assert_eq!(
        closed_members,
        vec![NodeId::Object(a), NodeId::Object(b), NodeId::Object(fresh)]
    );

    doc.undo().expect("undo close");
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Group(group)],
        "undoing a close re-enters the session"
    );
    assert_eq!(doc.node_parent(NodeId::Object(fresh)), None);

    // The extrusion is two recorded steps (sketch gesture + extrude in
    // this test idiom, or one compound) — walk back until the fresh box is
    // gone, then one more for the open.
    while doc.visible_object_ids().contains(&fresh) {
        doc.undo().expect("undo mid-session step");
    }
    doc.undo().expect("undo open");
    assert!(doc.session_stack().is_empty());
    assert_eq!(
        doc.group_members(group).expect("group restored"),
        vec![NodeId::Object(a), NodeId::Object(b)]
    );

    while doc.redo().is_ok() {}
    assert!(doc.session_stack().is_empty(), "redo replays the close too");
    assert_eq!(
        doc.group_members(group).expect("group is back"),
        closed_members,
        "redo converges on the identical end state"
    );
}

// --------------------------------------------------------------- spec 9

/// **Persistence mid-stack serializes as-if-closed** without touching the
/// live document — with TWO frames open, both fold transparently.
#[test]
fn save_for_persistence_mid_stack_matches_a_closed_save() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("inner");
    let (outer, _) = doc.group_nodes(&[NodeId::Group(inner)]).expect("outer");

    doc.open_group_session(outer).expect("open outer");
    doc.open_group_session(inner).expect("open inner");
    let fresh = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);

    let bytes_mid_stack = doc.save_for_persistence();

    // Live document untouched: both frames still open, the fresh box
    // still loose at the top level.
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Group(outer), NodeId::Group(inner)]
    );
    assert_eq!(doc.node_parent(NodeId::Object(fresh)), None);

    // Reference: a clone actually closed the ordinary way.
    let mut closed = doc.clone();
    closed.close_group_session().expect("close inner");
    closed.close_group_session().expect("close outer");
    assert_eq!(
        bytes_mid_stack,
        closed.save(),
        "persistence bytes are exactly the fully-closed document's bytes"
    );

    // And the real closes still work afterwards.
    doc.close_group_session().expect("close inner");
    doc.close_group_session().expect("close outer");
    assert_eq!(
        doc.group_members(inner).expect("inner survives"),
        vec![NodeId::Object(a), NodeId::Object(fresh)]
    );
}

// -------------------------------------------------------------- spec 10

/// **Structural creation is legal in a group session and folds in**:
/// `make_component` consumes a member and folds its new instance back
/// into the group — the gate that refuses this under a COMPONENT frame
/// does not fire under a group frame.
#[test]
fn make_component_mid_session_folds_the_instance_into_the_group() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);

    doc.open_group_session(group).expect("open");
    let (_component, instance, _) = doc
        .make_component(&[NodeId::Object(a)])
        .expect("make_component is legal in a group session");
    doc.close_group_session().expect("close");

    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Object(b), NodeId::Instance(instance)],
        "the consumed member drops; its replacement instance folds in"
    );
    assert_eq!(doc.node_parent(NodeId::Instance(instance)), Some(group));
}

// -------------------------------------------------------------- spec 11

/// **The kernel backstop under a group frame**: an operand OUTSIDE the
/// session refuses typed — a boolean against a dimmed outside object, and
/// a duplicate of one, would fold foreign-derived geometry into the group
/// at close.
#[test]
fn outside_operands_refuse_under_a_group_frame() {
    let mut doc = Document::new();
    // Overlap the outside box with member `a` so only the scope check can
    // be the refusal.
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (group, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let outside = build_box(&mut doc, 1.0, 0.0, 0.0, 3.0, 2.0, 1.0);

    doc.open_group_session(group).expect("open");
    assert_eq!(
        doc.boolean(BooleanOp::Union, a, outside).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.duplicate_node(NodeId::Object(outside), &kernel::Transform::IDENTITY)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    // The member itself duplicates fine, and the copy folds in.
    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(a),
            &kernel::Transform::translation(kernel::Vec3::new(10.0, 0.0, 0.0)),
        )
        .expect("duplicate a member");
    doc.close_group_session().expect("close");
    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Object(a), copy]
    );
}

// -------------------------------------------------------------- spec 12

/// **A grouped component instance opens through the group session** — the
/// case the old fallback existed for: `open_explode_session` refuses a
/// grouped placement, but inside the group's session the instance is
/// genuinely top-level, so the component session opens on top of the
/// group frame, the definition edit lands, and both closes re-home
/// everything.
#[test]
fn a_grouped_instance_opens_through_the_group_session() {
    let mut doc = Document::new();
    let lone = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (_component, instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("make_component");
    let (group, _) = doc
        .group_nodes(&[NodeId::Instance(instance)])
        .expect("group the instance");

    // Directly: refused (the placement is grouped).
    assert_eq!(
        doc.open_explode_session(instance).unwrap_err(),
        DocumentError::ExplodeSessionGroupedInstance
    );

    // Through the group session: the instance is top-level mid-session.
    doc.open_group_session(group).expect("open the group");
    doc.open_explode_session(instance)
        .expect("the grouped instance opens through the group frame");
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Group(group), NodeId::Instance(instance)]
    );

    doc.close_innermost_session().expect("close the component");
    doc.close_innermost_session().expect("close the group");
    assert_eq!(
        doc.group_members(group).expect("group survives"),
        vec![NodeId::Instance(instance)],
        "the instance is back inside its group"
    );
}

// ----------------------------------------------------- scoped rescale specs

/// Maximum deviation of `current`'s vertices from `expected(v)` applied to
/// `orig`'s — the anchored-scale correctness probe.
fn max_mapped_deviation(
    orig: &kernel::Object,
    current: &kernel::Object,
    expected: impl Fn(Point3) -> Point3,
) -> f64 {
    orig.vertices()
        .iter()
        .map(|(vid, v)| {
            let cur = current
                .vertices()
                .get(vid)
                .expect("vertex ids stable across a rescale");
            (cur.position - expected(v.position)).length()
        })
        .fold(0.0, f64::max)
}

// -------------------------------------------------------------- spec 13

/// **A group-frame rescale scales exactly the session's contents about
/// the anchor** — members and mid-session guides move, the anchor point
/// holds, and outside geometry (plus a pre-session guide) is untouched.
#[test]
fn rescale_session_scales_group_contents_about_the_anchor_and_nothing_else() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);
    let outside = build_box(&mut doc, 10.0, 10.0, 0.0, 12.0, 12.0, 1.0);
    let pre_guide = doc
        .add_guide_point(Point3::new(20.0, 0.0, 0.0))
        .expect("pre-session guide");

    // No session open: the scoped rescale refuses typed.
    assert_eq!(
        doc.rescale_session(2.0, Point3::new(0.0, 0.0, 0.0))
            .unwrap_err(),
        DocumentError::ExplodeSessionNotOpen
    );

    doc.open_group_session(group).expect("open");
    let mid_guide = doc
        .add_guide_point(Point3::new(4.0, 0.0, 0.0))
        .expect("mid-session guide");
    let pre_a = doc.object(a).expect("a live").clone();
    let pre_b = doc.object(b).expect("b live").clone();
    let pre_outside = doc.object(outside).expect("outside live").clone();

    assert_eq!(
        doc.rescale_session(f64::NAN, Point3::new(0.0, 0.0, 0.0))
            .unwrap_err(),
        DocumentError::InvalidRescaleFactor
    );

    let anchor = Point3::new(2.0, 2.0, 1.0);
    doc.rescale_session(2.0, anchor).expect("scoped rescale");

    let scaled = |p: Point3| anchor + (p - anchor) * 2.0;
    assert!(
        max_mapped_deviation(&pre_a, doc.object(a).expect("a live"), scaled) < 1e-12,
        "member a scales about the anchor"
    );
    assert!(
        max_mapped_deviation(&pre_b, doc.object(b).expect("b live"), scaled) < 1e-12,
        "member b scales about the anchor"
    );
    assert!(
        max_mapped_deviation(&pre_outside, doc.object(outside).expect("live"), |p| p) == 0.0,
        "outside geometry is untouched"
    );
    match doc.guide(mid_guide).expect("mid-session guide live") {
        kernel::Guide::Point { position } => {
            assert!((*position - scaled(Point3::new(4.0, 0.0, 0.0))).length() < 1e-12)
        }
        _ => panic!("guide kind changed"),
    }
    match doc.guide(pre_guide).expect("pre-session guide live") {
        kernel::Guide::Point { position } => {
            assert_eq!(*position, Point3::new(20.0, 0.0, 0.0))
        }
        _ => panic!("guide kind changed"),
    }

    // Exact undo: the recorded snapshots restore verbatim.
    doc.undo().expect("undo the rescale");
    assert!(
        max_mapped_deviation(&pre_a, doc.object(a).expect("a live"), |p| p) == 0.0,
        "undo restores member a bit-for-bit"
    );

    doc.close_group_session().expect("close");
}

// -------------------------------------------------------------- spec 14

/// **A member instance scales via its pose** under a group frame — the
/// shared definition stays at its authored size, so other placements of
/// the same component elsewhere in the model are untouched.
#[test]
fn rescale_session_composes_into_member_instance_poses() {
    let mut doc = Document::new();
    let lone = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (component, instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("make_component");
    let (group, _) = doc
        .group_nodes(&[NodeId::Instance(instance)])
        .expect("group the instance");
    let member_object = doc.def_members(component).expect("def live")[0];
    let pre_def = doc.object(member_object).expect("def member").clone();
    let pre_pose = doc.instance_pose(instance).expect("pose");

    doc.open_group_session(group).expect("open");
    let anchor = Point3::new(1.0, 1.0, 0.0);
    doc.rescale_session(3.0, anchor).expect("scoped rescale");

    assert!(
        max_mapped_deviation(&pre_def, doc.object(member_object).expect("live"), |p| p) == 0.0,
        "the shared definition stays at its authored size"
    );
    let post_pose = doc.instance_pose(instance).expect("pose");
    assert_ne!(pre_pose, post_pose, "the pose absorbed the scale");
    // The pose maps definition-local corner (2,2,1) to the anchored image
    // of its previous world position.
    let prev_world = pre_pose.apply_point(Point3::new(2.0, 2.0, 1.0));
    let want = anchor + (prev_world - anchor) * 3.0;
    assert!(
        (post_pose.apply_point(Point3::new(2.0, 2.0, 1.0)) - want).length() < 1e-12,
        "the composed pose scales the instance about the anchor"
    );

    doc.close_group_session().expect("close");
}

// -------------------------------------------------------------- spec 15

/// **A component-frame rescale reaches the definition at close**: the
/// scaled members dirty-mark, the close unbakes them, and every other
/// placement of the definition resizes about its own image of the anchor.
#[test]
fn rescale_session_in_a_component_frame_rescales_the_definition_at_close() {
    let mut doc = Document::new();
    let lone = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (component, instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("make_component");
    let member_object = doc.def_members(component).expect("def live")[0];
    let pre_def = doc.object(member_object).expect("def member").clone();

    doc.open_explode_session(instance).expect("open");
    // Identity pose: definition-local space IS world space here, so the
    // anchor's definition-local image is the anchor itself.
    let anchor = Point3::new(2.0, 2.0, 1.0);
    doc.rescale_session(0.5, anchor).expect("scoped rescale");
    doc.close_explode_session().expect("close");

    let scaled = |p: Point3| anchor + (p - anchor) * 0.5;
    assert!(
        max_mapped_deviation(&pre_def, doc.object(member_object).expect("live"), scaled) < 1e-12,
        "the definition itself carries the anchored scale after the close"
    );
}

// ------------------------------------------- adversarial-review fix specs

// -------------------------------------------------------------- spec 16

/// **Ungrouping a nested member group mid-session releases its contents
/// INTO the open context** (SketchUp's explode-inside-a-context
/// semantic): the released members join the session's scope — scoped ops
/// accept them — and fold into the session group at close. Before the
/// fix they were invisible to both walks: locked out mid-session, leaked
/// to the model root at close, and an only-member ungroup deleted the
/// session group through the emptied branch.
#[test]
fn ungrouping_a_nested_member_group_releases_its_contents_into_the_session() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(&mut doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("inner group");
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner)])
        .expect("outer group");

    doc.open_group_session(outer).expect("open outer");
    doc.ungroup(inner).expect("ungroup the nested member group");

    // The released members are session geometry now: a scoped op accepts
    // them (this refused ExplodeSessionScope before the fix).
    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(a),
            &kernel::Transform::translation(kernel::Vec3::new(10.0, 0.0, 0.0)),
        )
        .expect("released member duplicates mid-session");

    doc.close_group_session().expect("close");
    assert_eq!(
        doc.group_members(outer)
            .expect("outer SURVIVES — not emptied"),
        vec![NodeId::Object(a), NodeId::Object(b), copy],
        "released members and the mid-session copy fold into the session group"
    );
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(outer));
}

// -------------------------------------------------------------- spec 17

/// **Outside operands refuse across every structural op under a group
/// frame** — grouping, component creation, instance explode, and ungroup
/// of a group that is not session geometry would all steal outside
/// nodes into (or leak session structure out of) the open group at
/// close, so each refuses typed.
#[test]
fn structural_ops_refuse_outside_operands_under_a_group_frame() {
    let mut doc = Document::new();
    let (group, _a, _b) = two_box_group(&mut doc);
    let x = build_box(&mut doc, 10.0, 10.0, 0.0, 12.0, 12.0, 1.0);
    let y = build_box(&mut doc, 14.0, 10.0, 0.0, 15.0, 11.0, 1.0);
    let (outside_group, _) = doc
        .group_nodes(&[NodeId::Object(y)])
        .expect("outside group");
    let lone = build_box(&mut doc, 20.0, 0.0, 0.0, 21.0, 1.0, 1.0);
    let (_c, outside_instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("outside component");

    doc.open_group_session(group).expect("open");

    assert_eq!(
        doc.group_nodes(&[NodeId::Object(x)]).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "grouping an outside node would move it inside the session group"
    );
    assert_eq!(
        doc.make_component(&[NodeId::Object(x)]).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "an outside make_component's instance would fold into the session group"
    );
    assert_eq!(
        doc.explode_instance(outside_instance).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "an outside instance's baked objects would fold into the session group"
    );
    assert_eq!(
        doc.ungroup(outside_group).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "an outside ungroup's released members would fold into the session group"
    );

    doc.close_group_session().expect("close");
    // Nothing leaked in: membership is exactly the original two members.
    assert_eq!(doc.group_members(group).expect("live").len(), 2);
}

// -------------------------------------------------------------- spec 18

/// **A nested member group's internals are session geometry**: scoped
/// ops accept a node inside a member subtree (its results land inside
/// that nested container, never outside), so opening a session must not
/// REVOKE capability that existed without one.
#[test]
fn nested_member_internals_stay_operable_under_a_group_frame() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("inner");
    let (outer, _) = doc.group_nodes(&[NodeId::Group(inner)]).expect("outer");

    doc.open_group_session(outer).expect("open outer");
    // `a` sits inside nested member group `inner` — not a direct member,
    // but session-owned; the duplicate lands inside `inner`.
    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(a),
            &kernel::Transform::translation(kernel::Vec3::new(5.0, 0.0, 0.0)),
        )
        .expect("nested member internals duplicate mid-session");
    match copy {
        NodeId::Object(_) => {}
        _ => panic!("object copy expected"),
    }
    assert_eq!(
        doc.node_parent(copy),
        Some(inner),
        "the copy stays inside the nested group"
    );
    doc.close_group_session().expect("close");
    assert_eq!(
        doc.group_members(outer).expect("outer survives"),
        vec![NodeId::Group(inner)],
        "nothing extra folds in — the copy already lives inside the member subtree"
    );
}

// -------------------------------------------------------------- spec 19

/// **Exploding a MEMBER instance folds its baked objects in; exploding
/// any instance during a COMPONENT session refuses** (the baked copies
/// would fold into the open definition).
#[test]
fn explode_instance_scoping_per_frame_kind() {
    let mut doc = Document::new();
    let lone = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let (_comp, member_instance, _) = doc
        .make_component(&[NodeId::Object(lone)])
        .expect("component");
    let (group, _) = doc
        .group_nodes(&[NodeId::Instance(member_instance)])
        .expect("group the instance");

    doc.open_group_session(group).expect("open");
    let (baked, _) = doc
        .explode_instance(member_instance)
        .expect("a member instance explodes mid-session");
    doc.close_group_session().expect("close");
    assert_eq!(
        doc.group_members(group).expect("group survives"),
        baked.iter().map(|&o| NodeId::Object(o)).collect::<Vec<_>>(),
        "the baked objects replace the exploded member instance"
    );

    // Component frame: refuses outright.
    let other = build_box(&mut doc, 10.0, 0.0, 0.0, 12.0, 1.0, 1.0);
    let (_c2, inst2, _) = doc.make_component(&[NodeId::Object(other)]).expect("c2");
    let extra = build_box(&mut doc, 20.0, 0.0, 0.0, 21.0, 1.0, 1.0);
    let (_c3, inst3, _) = doc.make_component(&[NodeId::Object(extra)]).expect("c3");
    doc.open_explode_session(inst2)
        .expect("open component session");
    assert_eq!(
        doc.explode_instance(inst3).unwrap_err(),
        DocumentError::ExplodeSessionScope,
        "exploding any instance during a component session would fold foreign objects into the open definition"
    );
    doc.close_explode_session().expect("close");
}

// -------------------------------------------------------------- spec 20

/// **`session_direct_members` reports kernel truth for the innermost
/// frame** — original members, minus deletions, plus surfacings — the
/// query the app's Outliner/pick scope reads instead of any app-side
/// open-time snapshot.
#[test]
fn session_direct_members_tracks_the_live_scope() {
    let mut doc = Document::new();
    let (group, a, b) = two_box_group(&mut doc);
    assert_eq!(doc.session_direct_members(), None);

    doc.open_group_session(group).expect("open");
    assert_eq!(
        doc.session_direct_members().expect("open frame"),
        vec![NodeId::Object(a), NodeId::Object(b)]
    );
    let fresh = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    doc.delete_node(NodeId::Object(b)).expect("delete b");
    assert_eq!(
        doc.session_direct_members().expect("open frame"),
        vec![NodeId::Object(a), NodeId::Object(fresh)],
        "deletions drop out; creations join"
    );
    doc.close_group_session().expect("close");
    assert_eq!(doc.session_direct_members(), None);
}
