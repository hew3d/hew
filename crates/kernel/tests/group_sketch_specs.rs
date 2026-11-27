//! Sketches held by groups (`Document::set_sketch_group`): drawing that
//! belongs to a group the way a `.skp` group of edges does — shown with
//! the group, deleted with it, released by an ungroup, copied with a
//! definition made unique, and saved as the sketch's `owner_group`
//! (HEW_FILE_FORMAT.md §4.6, manifest v19).

use kernel::{Document, DocumentError, NodeId, Plane, Point3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// A unit box at `x`, from its own sketch (extruded, so the sketch is
/// consumed and the box stands alone).
fn box_at(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let p = |dx: f64, dy: f64| Point3::new(x + dx, dy, 0.0);
    let corners = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..4 {
        sk.add_segment(corners[i], corners[(i + 1) % 4]).unwrap();
    }
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap().0
}

/// A free line on the ground, well away from any box.
fn line(doc: &mut Document, y: f64) -> kernel::SketchId {
    let s = doc.add_sketch(ground());
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(10.0, y, 0.0), Point3::new(12.0, y, 0.0))
        .unwrap();
    s
}

#[test]
fn a_sketch_put_in_a_group_is_shown_with_the_group_and_goes_with_it() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let s = line(&mut doc, 0.0);
    assert_eq!(doc.sketch_owner_group(s), None);
    assert!(doc.sketch_shown(s));

    doc.set_sketch_group(s, Some(g)).unwrap();
    assert_eq!(doc.sketch_owner_group(s), Some(g));
    assert_eq!(doc.group_member_sketches(g), vec![s]);
    // Still a world sketch: drawn with the rest, picked with the rest.
    assert!(doc.sketch_ids().contains(&s));
    assert!(doc.visible_sketch_ids().contains(&s));

    // Hiding the group hides its drawing; showing it brings it back.
    doc.set_node_user_hidden(NodeId::Group(g), true);
    assert!(!doc.sketch_shown(s));
    assert!(!doc.visible_sketch_ids().contains(&s));
    assert!(doc.sketch_ids().contains(&s), "hidden, not gone");
    doc.set_node_user_hidden(NodeId::Group(g), false);
    assert!(doc.sketch_shown(s));

    // The membership is one undo step.
    doc.undo().unwrap();
    assert_eq!(doc.sketch_owner_group(s), None);
    doc.redo().unwrap();
    assert_eq!(doc.sketch_owner_group(s), Some(g));

    // Deleting the group takes its drawing with it, in the same step.
    doc.delete_node(NodeId::Group(g)).unwrap();
    assert!(
        !doc.sketch_ids().contains(&s),
        "the line went with its group"
    );
    doc.undo().unwrap();
    assert!(
        doc.sketch_ids().contains(&s),
        "undo brings group and line back"
    );
    assert_eq!(doc.sketch_owner_group(s), Some(g));
    doc.redo().unwrap();
    assert!(!doc.sketch_ids().contains(&s));
}

#[test]
fn ungrouping_releases_the_groups_drawing() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();

    doc.ungroup(g).unwrap();
    assert_eq!(doc.sketch_owner_group(s), None, "free drawing again");
    assert!(doc.sketch_shown(s));
    doc.undo().unwrap();
    assert_eq!(
        doc.sketch_owner_group(s),
        Some(g),
        "undo re-groups the line"
    );
    doc.redo().unwrap();
    assert_eq!(doc.sketch_owner_group(s), None);
}

#[test]
fn a_sketch_cannot_join_a_group_in_another_frame() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    // Fold the group into a component: the group is now in that
    // definition's frame, the line is a world sketch.
    let (def, _, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    let s = line(&mut doc, 0.0);
    let members = doc.def_member_nodes(def).unwrap();
    let [NodeId::Group(inner)] = members.as_slice() else {
        panic!("the group is the definition's member");
    };
    assert!(matches!(
        doc.set_sketch_group(s, Some(*inner)),
        Err(DocumentError::SketchOwnerMismatch)
    ));
    assert!(matches!(
        doc.set_sketch_group(s, Some(g)),
        Err(DocumentError::UnknownGroup) | Err(DocumentError::SketchOwnerMismatch)
    ));
}

#[test]
fn a_groups_drawing_round_trips_through_save_and_load() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let s = line(&mut doc, 0.0);
    let free = line(&mut doc, 5.0);
    doc.set_sketch_group(s, Some(g)).unwrap();

    let bytes = doc.save();
    let loaded = Document::load(&bytes).unwrap();
    let groups = loaded.group_ids();
    assert_eq!(groups.len(), 1);
    let held = loaded.group_member_sketches(groups[0]);
    assert_eq!(held.len(), 1, "one sketch held by the group");
    let other: Vec<_> = loaded
        .sketch_ids()
        .into_iter()
        .filter(|&x| loaded.sketch_owner_group(x).is_none())
        .collect();
    assert_eq!(other.len(), 1, "the free line stays free");
    let _ = free;
    // Byte-identical on a second save (determinism).
    assert_eq!(loaded.save(), bytes);
}

#[test]
fn a_held_sketch_is_copied_with_its_group_when_a_definition_is_made_unique() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    let (def, inst, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    // The line went into the definition with its group.
    assert_eq!(
        doc.sketch_owner_component(s),
        Some(def),
        "{:?}",
        doc.sketch_owner_component(s)
    );
    let (second, _) = doc
        .place_instance(
            def,
            kernel::Transform::translation(kernel::Vec3::new(0.0, 20.0, 0.0)),
        )
        .unwrap();
    let _ = inst;
    let (copy, _) = doc.make_unique(second).unwrap();
    let copied_group = match doc.def_member_nodes(copy).unwrap().as_slice() {
        [NodeId::Group(cg)] => *cg,
        other => panic!("the copy holds the copied group: {other:?}"),
    };
    let held = doc.group_member_sketches(copied_group);
    assert_eq!(
        held.len(),
        1,
        "the copy's group holds its own copy of the line"
    );
    assert_ne!(held[0], s);
    assert_eq!(doc.sketch_owner_component(held[0]), Some(copy));
}

#[test]
fn moving_a_group_takes_the_drawing_it_holds_along() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    let ys = |doc: &Document| -> Vec<i64> {
        doc.sketch(s)
            .unwrap()
            .vertices()
            .values()
            .map(|v| (v.position.y * 1000.0).round() as i64)
            .collect()
    };
    let shift = kernel::Transform::translation(kernel::Vec3::new(0.0, 10.0, 0.0));
    doc.transform_group(g, &shift).unwrap();
    assert!(ys(&doc).iter().all(|&y| y == 10_000), "{:?}", ys(&doc));
    // As one node of a selection, too.
    doc.transform_selection(&[NodeId::Group(g)], &[], &shift)
        .unwrap();
    assert!(ys(&doc).iter().all(|&y| y == 20_000), "{:?}", ys(&doc));
    // Undo brings the drawing back with the group.
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert!(ys(&doc).iter().all(|&y| y == 0), "{:?}", ys(&doc));
}

#[test]
fn exploding_an_instance_keeps_a_held_sketch_with_the_groups_copy() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    let (_def, inst, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    doc.explode_instance(inst).unwrap();
    let copy = match doc.top_level_nodes().as_slice() {
        [NodeId::Group(copy)] => *copy,
        other => panic!("the exploded group stands at the top: {other:?}"),
    };
    let held = doc.group_member_sketches(copy);
    assert_eq!(held.len(), 1, "the copy holds its own copy of the line");
    assert_ne!(held[0], s);
    assert!(doc.sketch_shown(held[0]));
    assert_eq!(doc.sketch_owner_component(held[0]), None);
}

#[test]
fn editing_a_group_keeps_its_drawing_and_lines_drawn_inside_join_it() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    doc.open_group_session(g).unwrap();
    // The held line is still there and on view while the group is open.
    assert!(
        doc.sketch_ids().contains(&s),
        "the drawing is not deleted by the open"
    );
    assert!(
        doc.sketch_shown(s),
        "the drawing stays on view while editing"
    );
    // An edit inside the session leaves it alone too.
    doc.transform_object(
        a,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 1.0)),
    )
    .unwrap();
    assert!(doc.sketch_ids().contains(&s));
    assert_eq!(doc.sketch_owner_group(s), Some(g));
    // A line drawn while the group is open (the app's way: a fresh sketch,
    // its first segment in a gesture) joins it at the close.
    let drawn = doc.add_sketch(ground());
    doc.begin_sketch_gesture(drawn).unwrap();
    doc.sketch_mut(drawn)
        .unwrap()
        .add_segment(Point3::new(10.0, 2.0, 0.0), Point3::new(10.0, 3.0, 0.0))
        .unwrap();
    doc.end_sketch_gesture(drawn).unwrap();
    assert_eq!(doc.sketch_owner_group(drawn), None, "free until the close");
    doc.close_group_session().unwrap();
    assert_eq!(doc.sketch_owner_group(drawn), Some(g));
    assert_eq!(doc.sketch_owner_group(s), Some(g));
    assert_eq!(doc.group_member_sketches(g).len(), 2);
    assert!(doc.sketch_shown(drawn));
}

#[test]
fn duplicating_a_group_copies_the_drawing_it_holds() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    let shift = kernel::Transform::translation(kernel::Vec3::new(0.0, 10.0, 0.0));
    let (root, _) = doc
        .duplicate_node(NodeId::Group(g), &shift, kernel::LinkPolicy::Independent)
        .unwrap();
    let NodeId::Group(copy) = root else {
        panic!("a group copies as a group: {root:?}");
    };
    let held = doc.group_member_sketches(copy);
    assert_eq!(held.len(), 1, "the copy holds a copy of the line");
    assert_ne!(held[0], s);
    let ys: Vec<i64> = doc
        .sketch(held[0])
        .unwrap()
        .vertices()
        .values()
        .map(|v| (v.position.y * 1000.0).round() as i64)
        .collect();
    assert!(ys.iter().all(|&y| y == 10_000), "{ys:?}");
    assert_eq!(
        doc.group_member_sketches(g),
        vec![s],
        "the original keeps its own"
    );
    // Undo takes the copy and its drawing away together.
    doc.undo().unwrap();
    assert!(!doc.sketch_ids().contains(&held[0]));
    assert!(doc.sketch_ids().contains(&s));
}

#[test]
fn ungrouping_a_nested_group_releases_its_drawing_to_the_group_around_it() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(inner)).unwrap();
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(b)])
        .unwrap();
    doc.ungroup(inner).unwrap();
    assert_eq!(
        doc.sketch_owner_group(s),
        Some(outer),
        "released one level up, not to the top"
    );
    assert!(doc.sketch_shown(s));
    doc.undo().unwrap();
    assert_eq!(doc.sketch_owner_group(s), Some(inner));
}

#[test]
fn a_group_of_lines_alone_survives_being_opened_and_closed() {
    // A `.skp` group of edges arrives as a group with no members holding
    // its drawing (import_specs); entering and leaving it must not delete it.
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    doc.delete_node(NodeId::Object(a)).unwrap();
    assert!(doc.group_members(g).unwrap().is_empty());
    assert!(
        doc.group_ids().contains(&g),
        "a group holding only drawing is live"
    );
    doc.open_group_session(g).unwrap();
    doc.close_group_session().unwrap();
    assert!(
        doc.group_ids().contains(&g),
        "entering and leaving keeps the group"
    );
    assert!(doc.sketch_ids().contains(&s));
    assert_eq!(doc.sketch_owner_group(s), Some(g));
    assert!(doc.sketch_shown(s));
}

/// A free line on the ground drawn the app's way (a fresh sketch, its
/// segment in a recorded gesture), so it is history, not a fresh sketch.
fn drawn_line(doc: &mut Document, x: f64) -> kernel::SketchId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(x, 20.0, 0.0), Point3::new(x, 21.0, 0.0))
        .unwrap();
    doc.end_sketch_gesture(s).unwrap();
    s
}

/// **A group frame's own drawing** (`session_frame_sketches`): while a
/// group session is innermost it is the drawing the group holds, the
/// drawing of a group nested in it, a member's owned sketch, a line drawn
/// since the open (recorded or still fresh), and a nested group's drawing
/// a mid-session ungroup released — exactly what the close keeps in or
/// folds into the group. Never a free line drawn before the open, nor
/// another group's drawing. Empty once closed.
#[test]
fn a_group_frames_own_drawing_is_what_the_group_holds_and_folds_in() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let b = box_at(&mut doc, 3.0);
    let c = box_at(&mut doc, 6.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();
    let in_inner = drawn_line(&mut doc, 1.0);
    doc.set_sketch_group(in_inner, Some(inner)).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Group(inner)])
        .unwrap();
    let held = drawn_line(&mut doc, 2.0);
    doc.set_sketch_group(held, Some(g)).unwrap();
    let (other, _) = doc.group_nodes(&[NodeId::Object(c)]).unwrap();
    let in_other = drawn_line(&mut doc, 3.0);
    doc.set_sketch_group(in_other, Some(other)).unwrap();
    let outside = drawn_line(&mut doc, 4.0);
    // A sketch member A owns, on its top face (z = 1).
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    let owned = doc.add_sketch(top);
    doc.set_sketch_object_owner(owned, Some(a)).unwrap();
    assert!(doc.session_frame_sketches().is_empty(), "no frame open");

    doc.open_group_session(g).unwrap();
    let drawn = drawn_line(&mut doc, 5.0);
    let fresh = doc.add_sketch(ground());
    let frame = doc.session_frame_sketches();
    for s in [held, in_inner, owned, drawn, fresh] {
        assert!(
            frame.contains(&s),
            "{s:?} is the frame's drawing: {frame:?}"
        );
    }
    for s in [outside, in_other] {
        assert!(!frame.contains(&s), "{s:?} is context: {frame:?}");
    }

    // A mid-session ungroup releases the nested group's drawing to the
    // top; it folds into the open group at the close, so it stays in.
    doc.ungroup(inner).unwrap();
    assert_eq!(doc.sketch_owner_group(in_inner), None, "released");
    assert!(doc.session_frame_sketches().contains(&in_inner));

    doc.close_group_session().unwrap();
    assert!(doc.session_frame_sketches().is_empty(), "closed");
    assert_eq!(doc.sketch_owner_group(drawn), Some(g), "folded in");
    assert_eq!(doc.sketch_owner_group(in_inner), Some(g), "folded in");
}

/// A group holding drawing and no solid (a `.skp` group of edges): its
/// frame's drawing is that drawing.
#[test]
fn a_group_of_lines_alone_has_its_lines_as_its_frames_drawing() {
    let mut doc = Document::new();
    let a = box_at(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let s = drawn_line(&mut doc, 0.0);
    doc.set_sketch_group(s, Some(g)).unwrap();
    doc.delete_node(NodeId::Object(a)).unwrap();
    doc.open_group_session(g).unwrap();
    assert_eq!(doc.session_frame_sketches(), vec![s]);
}
