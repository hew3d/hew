//! Executable specs for transforming and copying a selection that mixes
//! construction guides with nodes: `transform_mixed_selection`,
//! `duplicate_mixed_placements` / `duplicate_mixed_array`, `flip_mixed` and
//! `flip_mixed_copy`.
//!
//! The oracle throughout is a twin document that reaches the same state
//! through the single-kind operations that already exist
//! (`transform_selection` then `transform_guide` per guide, the node copy
//! then `duplicate_guide_array`): the mixed operation must land exactly
//! there, in ONE undo step that undo and redo reproduce bit for bit
//! (DEVELOPMENT.md rule 9). A refusal anywhere in the batch leaves the
//! document and its history untouched. The property at the end checks the
//! same over random selections and transforms (DEVELOPMENT.md rule 3).

use kernel::{
    Anchor, Annotation, Document, DocumentError, FlipAbout, Guide, GuideId, LinkPolicy, NodeId,
    ObjectId, Plane, Point3, SketchId, Transform, Vec3,
};
use proptest::prelude::*;
use std::num::NonZeroU32;

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

/// A box with x in [x0, x1], y in [y0, y1], z in [0, h].
fn make_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    draw_loop(doc, s, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// Two boxes, a guide point and a guide line: the scene every spec starts
/// from, built the same way in a document and in its twin.
struct Scene {
    doc: Document,
    a: ObjectId,
    b: ObjectId,
    point: GuideId,
    line: GuideId,
}

fn scene() -> Scene {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = make_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let point = doc
        .add_guide_point(Point3::new(5.0, 2.0, 0.0))
        .expect("guide point");
    let line = doc
        .add_guide_line(Point3::new(0.0, 4.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .expect("guide line");
    Scene {
        doc,
        a,
        b,
        point,
        line,
    }
}

fn guide(doc: &Document, g: GuideId) -> Guide {
    *doc.guide(g).expect("guide is live")
}

fn last_kind(doc: &Document) -> kernel::HistoryKind {
    doc.history_entries()
        .undo
        .last()
        .expect("an undo entry")
        .kind
        .clone()
}

fn last_label(doc: &Document) -> String {
    doc.history_entries()
        .undo
        .last()
        .expect("an undo entry")
        .label
        .clone()
}

/// Undo restores `before` exactly, and redo the state the op committed.
fn assert_undo_redo_exact(doc: &mut Document, before: u64) {
    let after = doc.state_hash();
    doc.undo().expect("undo");
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
    doc.redo().expect("redo");
    assert_eq!(doc.state_hash(), after, "one redo reproduces the commit");
}

/// The null key: never a live guide.
fn stale_guide() -> GuideId {
    GuideId::default()
}

fn rotation_z(angle: f64) -> Transform {
    Transform::rotation(Vec3::new(0.0, 0.0, 1.0), angle).expect("unit axis")
}

// ------------------------------------------------- transform_mixed_selection

/// Nodes and guides move together as ONE undo step, landing exactly where
/// a node transform followed by one guide transform each would put them.
#[test]
fn a_mixed_transform_carries_guides_with_the_nodes_in_one_step() {
    let Scene {
        mut doc,
        a,
        b,
        point,
        line,
    } = scene();
    let Scene { doc: mut twin, .. } = scene();
    let t = rotation_z(0.6).then(&Transform::translation(Vec3::new(2.0, -1.0, 0.5)));
    let depth = doc.undo_depth();
    let before = doc.state_hash();

    let change = doc
        .transform_mixed_selection(&[NodeId::Object(a)], &[], &[point, line], &t)
        .expect("mixed transform");

    twin.transform_selection(&[NodeId::Object(a)], &[], &t)
        .expect("node half");
    twin.transform_guide(point, &t).expect("point");
    twin.transform_guide(line, &t).expect("line");
    assert_eq!(
        doc.state_hash(),
        twin.state_hash(),
        "the same state as the single-kind ops"
    );
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(change.objects_touched, vec![a]);
    assert_eq!(change.guides_touched, vec![point, line]);
    assert!(
        doc.visible_object_ids().contains(&b),
        "an unlisted node stays"
    );
    assert_undo_redo_exact(&mut doc, before);
}

/// A rotation turns a line guide's direction and carries its origin; a
/// point guide moves like any point.
#[test]
fn a_mixed_transform_turns_a_line_guide_and_moves_a_point_guide() {
    let Scene {
        mut doc,
        a,
        point,
        line,
        ..
    } = scene();
    let t = rotation_z(std::f64::consts::FRAC_PI_2);
    doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point, line], &t)
        .expect("rotate");

    let Guide::Point { position } = guide(&doc, point) else {
        panic!("still a point");
    };
    assert!((position - Point3::new(-2.0, 5.0, 0.0)).length() < 1e-12);
    let Guide::Line {
        origin, direction, ..
    } = guide(&doc, line)
    else {
        panic!("still a line");
    };
    assert!((origin - Point3::new(-4.0, 0.0, 0.0)).length() < 1e-12);
    assert!((direction - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-12);
}

/// Guides with no node listed are a selection of their own: one step,
/// named for its guides.
#[test]
fn a_transform_of_guides_alone_is_one_step_named_for_guides() {
    let Scene {
        mut doc,
        point,
        line,
        ..
    } = scene();
    let t = Transform::translation(Vec3::new(0.0, 1.0, 0.0));
    let depth = doc.undo_depth();
    let before = doc.state_hash();

    doc.transform_mixed_selection(&[], &[], &[point, line], &t)
        .expect("guides alone");
    assert_eq!(doc.undo_depth(), depth + 1);
    let kind = last_kind(&doc);
    assert_eq!(kind.id, "transform");
    assert_eq!(kind.text_arg("verb"), Some("move"));
    assert_eq!(kind.text_arg("noun"), Some("guide"));
    assert_eq!(kind.count_arg("count"), Some(2));
    assert_eq!(last_label(&doc), "Move 2 guides");
    assert_undo_redo_exact(&mut doc, before);
}

/// The step's count takes in the guides, and a mix of kinds is "items".
#[test]
fn a_mixed_transform_counts_its_guides_in_the_label() {
    let Scene {
        mut doc,
        a,
        b,
        point,
        ..
    } = scene();
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    doc.transform_mixed_selection(&[NodeId::Object(a), NodeId::Object(b)], &[], &[point], &t)
        .expect("move");
    let kind = last_kind(&doc);
    assert_eq!(kind.text_arg("noun"), Some("item"));
    assert_eq!(kind.count_arg("count"), Some(3));
    assert_eq!(last_label(&doc), "Move 3 items");
}

/// A node-only transform through the mixed entry point is exactly a
/// `transform_selection`: same state, same step name.
#[test]
fn a_mixed_transform_without_guides_is_a_selection_transform() {
    let Scene { mut doc, a, b, .. } = scene();
    let Scene { doc: mut twin, .. } = scene();
    let t = Transform::translation(Vec3::new(0.0, 0.0, 2.0));
    let nodes = [NodeId::Object(a), NodeId::Object(b)];
    doc.transform_mixed_selection(&nodes, &[], &[], &t)
        .expect("mixed, no guides");
    twin.transform_selection(&nodes, &[], &t).expect("plain");
    assert_eq!(doc.state_hash(), twin.state_hash());
    assert_eq!(last_kind(&doc), last_kind(&twin));
}

/// A stale guide anywhere in the batch refuses it whole: nothing moves and
/// nothing is recorded.
#[test]
fn a_stale_guide_refuses_the_whole_transform() {
    let Scene {
        mut doc, a, point, ..
    } = scene();
    let before = (doc.state_hash(), doc.history_generation());
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point, stale_guide()], &t),
        Err(DocumentError::UnknownGuide)
    );
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// A deleted guide is stale too.
#[test]
fn a_deleted_guide_refuses_the_whole_transform() {
    let Scene {
        mut doc,
        a,
        point,
        line,
        ..
    } = scene();
    doc.delete_guide(line).expect("delete");
    let before = (doc.state_hash(), doc.history_generation());
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point, line], &t),
        Err(DocumentError::UnknownGuide)
    );
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// A guide whose posed geometry is not a guide (here a coordinate pushed
/// past the largest float) refuses the whole batch before the nodes move.
#[test]
fn a_degenerate_guide_refuses_the_whole_transform() {
    let Scene {
        mut doc, a, point, ..
    } = scene();
    let far = doc
        .add_guide_point(Point3::new(f64::MAX, 0.0, 0.0))
        .expect("a far but finite point");
    let before = (doc.state_hash(), doc.history_generation());
    let t = Transform::translation(Vec3::new(f64::MAX, 0.0, 0.0));
    assert_eq!(
        doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point, far], &t),
        Err(DocumentError::DegenerateGuide)
    );
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// A singular map refuses before any guide is looked at.
#[test]
fn a_singular_transform_refuses_a_mixed_selection() {
    let Scene {
        mut doc, a, point, ..
    } = scene();
    let before = (doc.state_hash(), doc.history_generation());
    let flat = Transform::scale(Vec3::new(1.0, 1.0, 0.0));
    assert!(matches!(
        doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point], &flat),
        Err(DocumentError::Transform(_))
    ));
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// A guide listed twice moves once, as a node reached twice does.
#[test]
fn a_guide_listed_twice_moves_once() {
    let Scene { mut doc, point, .. } = scene();
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    let change = doc
        .transform_mixed_selection(&[], &[], &[point, point], &t)
        .expect("move");
    assert_eq!(
        guide(&doc, point),
        Guide::Point {
            position: Point3::new(6.0, 2.0, 0.0)
        }
    );
    assert_eq!(change.guides_touched, vec![point]);
    assert_eq!(last_kind(&doc).count_arg("count"), Some(1));
}

/// Nothing listed is still the empty-selection refusal, untouched.
#[test]
fn a_mixed_transform_of_nothing_refuses_empty_selection() {
    let Scene { mut doc, .. } = scene();
    let before = (doc.state_hash(), doc.history_generation());
    assert_eq!(
        doc.transform_mixed_selection(&[], &[], &[], &Transform::IDENTITY),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// An annotation anchored to a moving node follows it exactly once, guides
/// in the batch or not; one undo returns it with the node.
#[test]
fn an_annotation_anchored_to_a_moved_node_follows_it_once() {
    let Scene {
        mut doc, a, point, ..
    } = scene();
    let note = doc
        .add_leader_text(
            Anchor {
                node: Some(NodeId::Object(a)),
                point: Point3::new(1.0, 1.0, 1.0),
            },
            Vec3::new(0.5, 0.5, 0.0),
            "corner".to_string(),
        )
        .expect("note");
    let before = doc.state_hash();
    let t = Transform::translation(Vec3::new(2.0, 0.0, 0.0));
    doc.transform_mixed_selection(&[NodeId::Object(a)], &[], &[point], &t)
        .expect("move");
    let Some(Annotation::LeaderText { anchor, .. }) = doc.annotation(note) else {
        panic!("still a leader text");
    };
    assert_eq!(anchor.point, Point3::new(3.0, 1.0, 1.0), "moved once");
    assert_undo_redo_exact(&mut doc, before);
}

// ------------------------------------------------- duplicate_mixed_*

/// An array copies nodes and guides in ONE undo step, landing where a node
/// array followed by a guide array would; the sources stay where they are.
#[test]
fn a_mixed_array_copies_guides_with_the_nodes_in_one_step() {
    let Scene {
        mut doc,
        a,
        point,
        line,
        ..
    } = scene();
    let Scene { doc: mut twin, .. } = scene();
    let step = Transform::translation(Vec3::new(0.0, 3.0, 0.0));
    let count = NonZeroU32::new(3).expect("nonzero");
    let depth = doc.undo_depth();
    let before = doc.state_hash();

    let (roots, copies, change) = doc
        .duplicate_mixed_array(
            &[NodeId::Object(a)],
            &[point, line],
            &step,
            count,
            LinkPolicy::Independent,
        )
        .expect("mixed array");

    twin.duplicate_nodes_array(&[NodeId::Object(a)], &step, count, LinkPolicy::Independent)
        .expect("node half");
    // The twin copies one guide at a time, so its copies land guide-major;
    // the mixed array lands them placement-major. The geometry must agree
    // as a set, and the ids are compared by position below.
    twin.duplicate_guide_array(point, &step, count)
        .expect("point copies");
    twin.duplicate_guide_array(line, &step, count)
        .expect("line copies");
    let geometry = |d: &Document| {
        let mut gs: Vec<String> = d
            .guide_ids()
            .into_iter()
            .map(|g| format!("{:?}", d.guide(g)))
            .collect();
        gs.sort();
        gs
    };
    assert_eq!(geometry(&doc), geometry(&twin));
    assert_eq!(
        doc.visible_object_ids().len(),
        twin.visible_object_ids().len()
    );

    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(roots.len(), 3);
    assert_eq!(copies.len(), 6);
    assert_eq!(change.guides_touched, copies);
    // Placement-major: copy k of every guide, then copy k + 1.
    for (k, pair) in copies.chunks(2).enumerate() {
        let dy = 3.0 * (k as f64 + 1.0);
        assert_eq!(
            guide(&doc, pair[0]),
            Guide::Point {
                position: Point3::new(5.0, 2.0 + dy, 0.0)
            }
        );
        let Guide::Line { origin, .. } = guide(&doc, pair[1]) else {
            panic!("a line copy");
        };
        assert_eq!(origin, Point3::new(0.0, 4.0 + dy, 0.0));
    }
    assert_eq!(
        guide(&doc, point),
        Guide::Point {
            position: Point3::new(5.0, 2.0, 0.0)
        },
        "the source stays"
    );
    let kind = last_kind(&doc);
    assert_eq!(kind.id, "copy-array");
    assert_eq!(
        kind.count_arg("count"),
        Some(9),
        "3 node copies + 6 guide copies"
    );
    assert_undo_redo_exact(&mut doc, before);
}

/// Undo hides every copy, guides included; redo brings them back with the
/// same handles.
#[test]
fn undoing_a_mixed_array_hides_its_guide_copies() {
    let Scene {
        mut doc, a, point, ..
    } = scene();
    let step = Transform::translation(Vec3::new(0.0, 3.0, 0.0));
    let (_, copies, _) = doc
        .duplicate_mixed_array(
            &[NodeId::Object(a)],
            &[point],
            &step,
            NonZeroU32::new(2).expect("nonzero"),
            LinkPolicy::Independent,
        )
        .expect("array");
    doc.undo().expect("undo");
    assert!(copies.iter().all(|&g| doc.guide(g).is_none()));
    assert!(doc.guide(point).is_some(), "the source stays");
    doc.redo().expect("redo");
    assert!(copies.iter().all(|&g| doc.guide(g).is_some()));
}

/// A grid (explicit placements) is placement-major for guides too.
#[test]
fn mixed_placements_copy_each_guide_once_per_placement() {
    let Scene {
        mut doc, b, point, ..
    } = scene();
    let placements = [
        Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
        Transform::translation(Vec3::new(0.0, 10.0, 0.0)),
    ];
    let (roots, copies, _) = doc
        .duplicate_mixed_placements(
            &[NodeId::Object(b)],
            &[point],
            &placements,
            LinkPolicy::Independent,
        )
        .expect("grid");
    assert_eq!(roots.len(), 2);
    assert_eq!(
        copies.iter().map(|&g| guide(&doc, g)).collect::<Vec<_>>(),
        vec![
            Guide::Point {
                position: Point3::new(15.0, 2.0, 0.0)
            },
            Guide::Point {
                position: Point3::new(5.0, 12.0, 0.0)
            },
        ]
    );
}

/// Linked copies link the nodes only: every guide copy is an ordinary,
/// independent guide, and the whole copy is still one step.
#[test]
fn a_linked_mixed_array_copies_guides_independently() {
    let Scene {
        mut doc, a, line, ..
    } = scene();
    let step = Transform::translation(Vec3::new(0.0, 3.0, 0.0));
    let depth = doc.undo_depth();
    let before = doc.state_hash();
    let (roots, copies, change) = doc
        .duplicate_mixed_array(
            &[NodeId::Object(a)],
            &[line],
            &step,
            NonZeroU32::new(2).expect("nonzero"),
            LinkPolicy::Linked,
        )
        .expect("linked array");
    assert_eq!(
        doc.undo_depth(),
        depth + 1,
        "wraps, copies and guides: one step"
    );
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().all(|r| matches!(r, NodeId::Instance(_))));
    assert!(!change.linked.is_empty(), "the node was wrapped");
    assert_eq!(copies.len(), 2);
    assert!(change.guides_touched.iter().all(|g| copies.contains(g)));
    // Moving one copy leaves the other and the source alone: nothing links
    // a guide.
    let source = guide(&doc, line);
    let other = guide(&doc, copies[1]);
    doc.transform_guide(copies[0], &Transform::translation(Vec3::new(0.0, 0.0, 1.0)))
        .expect("move one copy");
    assert_eq!(guide(&doc, line), source);
    assert_eq!(guide(&doc, copies[1]), other);
    doc.undo().expect("undo the move");
    assert_undo_redo_exact(&mut doc, before);
}

/// Guides with no node listed copy as a guide array would: one
/// `copy-guides` step, under either policy.
#[test]
fn a_copy_of_guides_alone_is_a_guide_array() {
    for policy in [LinkPolicy::Independent, LinkPolicy::Linked] {
        let Scene {
            mut doc,
            point,
            line,
            ..
        } = scene();
        let step = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
        let before = doc.state_hash();
        let (roots, copies, _) = doc
            .duplicate_mixed_array(
                &[],
                &[point, line],
                &step,
                NonZeroU32::new(2).expect("nonzero"),
                policy,
            )
            .expect("guides alone");
        assert!(roots.is_empty());
        assert_eq!(copies.len(), 4);
        let kind = last_kind(&doc);
        assert_eq!(kind.id, "copy-guides");
        assert_eq!(kind.count_arg("count"), Some(4));
        assert_undo_redo_exact(&mut doc, before);
    }
}

/// A stale guide, a guide listed twice, or a degenerate copy refuses the
/// whole copy, nodes included, with nothing inserted.
#[test]
fn a_bad_guide_refuses_the_whole_copy() {
    let step = Transform::translation(Vec3::new(f64::MAX, 0.0, 0.0));
    let one = NonZeroU32::new(1).expect("nonzero");
    for policy in [LinkPolicy::Independent, LinkPolicy::Linked] {
        let Scene {
            mut doc, a, point, ..
        } = scene();
        let far = doc
            .add_guide_point(Point3::new(f64::MAX, 0.0, 0.0))
            .expect("far point");
        let before = (doc.state_hash(), doc.history_generation());
        let nodes = [NodeId::Object(a)];
        let shift = Transform::translation(Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(
            doc.duplicate_mixed_array(&nodes, &[point, stale_guide()], &shift, one, policy)
                .map(|_| ()),
            Err(DocumentError::UnknownGuide)
        );
        assert_eq!(
            doc.duplicate_mixed_array(&nodes, &[point, point], &shift, one, policy)
                .map(|_| ()),
            Err(DocumentError::DuplicateMember)
        );
        assert_eq!(
            doc.duplicate_mixed_array(&nodes, &[far], &step, one, policy)
                .map(|_| ()),
            Err(DocumentError::DegenerateGuide)
        );
        assert_eq!((doc.state_hash(), doc.history_generation()), before);
    }
}

/// No placements, or nothing listed, is the empty-selection refusal.
#[test]
fn a_mixed_copy_of_nothing_refuses_empty_selection() {
    let Scene { mut doc, point, .. } = scene();
    assert_eq!(
        doc.duplicate_mixed_placements(&[], &[], &[Transform::IDENTITY], LinkPolicy::Independent)
            .map(|_| ()),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!(
        doc.duplicate_mixed_placements(&[], &[point], &[], LinkPolicy::Independent)
            .map(|_| ()),
        Err(DocumentError::EmptySelection)
    );
}

// ------------------------------------------------- flip_mixed*

/// The mirror plane comes from the nodes alone; the guides are reflected
/// across it in the same step.
#[test]
fn a_mixed_flip_reflects_guides_across_the_nodes_center() {
    let Scene {
        mut doc,
        a,
        point,
        line,
        ..
    } = scene();
    let Scene { doc: mut twin, .. } = scene();
    let x = Vec3::new(1.0, 0.0, 0.0);
    let depth = doc.undo_depth();
    let before = doc.state_hash();
    doc.flip_mixed(
        &[NodeId::Object(a)],
        &[],
        &[point, line],
        x,
        FlipAbout::Center,
    )
    .expect("flip");
    assert_eq!(doc.undo_depth(), depth + 1);
    // Box a spans x in [0, 1]: the plane is x = 0.5, so the guide point at
    // x = 5 lands at x = -4, whatever the guide line's endless extent.
    assert_eq!(
        guide(&doc, point),
        Guide::Point {
            position: Point3::new(-4.0, 2.0, 0.0)
        }
    );
    let m = twin
        .flip_matrix(&[NodeId::Object(a)], &[], x, FlipAbout::Center)
        .expect("matrix");
    twin.flip_nodes(&[NodeId::Object(a)], &[], x, FlipAbout::Center)
        .expect("node half");
    twin.transform_guide(point, &m).expect("point");
    twin.transform_guide(line, &m).expect("line");
    assert_eq!(doc.state_hash(), twin.state_hash());
    assert_undo_redo_exact(&mut doc, before);
}

/// Guides alone have nothing to center a mirror on: both flips refuse
/// with the empty-selection error, untouched.
#[test]
fn a_flip_of_guides_alone_refuses_empty_selection() {
    let Scene {
        mut doc,
        point,
        line,
        ..
    } = scene();
    let before = (doc.state_hash(), doc.history_generation());
    let x = Vec3::new(1.0, 0.0, 0.0);
    assert_eq!(
        doc.flip_mixed(&[], &[], &[point, line], x, FlipAbout::Center),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!(
        doc.flip_mixed_copy(&[], &[point], x, FlipAbout::Origin, LinkPolicy::Independent)
            .map(|_| ()),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!((doc.state_hash(), doc.history_generation()), before);
}

/// A mirrored copy gives each guide one independent mirrored copy, in the
/// same step as the nodes' copy; the source guide stays.
#[test]
fn a_mixed_mirrored_copy_copies_each_guide_once() {
    for policy in [LinkPolicy::Independent, LinkPolicy::Linked] {
        let Scene {
            mut doc, a, point, ..
        } = scene();
        let depth = doc.undo_depth();
        let before = doc.state_hash();
        let (roots, copies, _) = doc
            .flip_mixed_copy(
                &[NodeId::Object(a)],
                &[point],
                Vec3::new(1.0, 0.0, 0.0),
                FlipAbout::Center,
                policy,
            )
            .expect("mirrored copy");
        assert_eq!(doc.undo_depth(), depth + 1);
        assert_eq!(roots.len(), 1);
        assert_eq!(copies.len(), 1);
        assert_eq!(
            guide(&doc, copies[0]),
            Guide::Point {
                position: Point3::new(-4.0, 2.0, 0.0)
            }
        );
        assert_eq!(
            guide(&doc, point),
            Guide::Point {
                position: Point3::new(5.0, 2.0, 0.0)
            }
        );
        assert_undo_redo_exact(&mut doc, before);
    }
}

// ------------------------------------------------------------- property

fn arb_transform() -> impl Strategy<Value = Transform> {
    let coord = -5.0f64..5.0;
    prop_oneof![
        (coord.clone(), coord.clone(), coord.clone())
            .prop_map(|(x, y, z)| Transform::translation(Vec3::new(x, y, z))),
        (-3.0f64..3.0, coord.clone(), coord.clone()).prop_map(|(angle, x, y)| {
            rotation_z(angle).then(&Transform::translation(Vec3::new(x, y, 0.0)))
        }),
        (0.25f64..4.0, 0.25f64..4.0, 0.25f64..4.0)
            .prop_map(|(x, y, z)| Transform::scale(Vec3::new(x, y, z))),
        coord.prop_map(|x| {
            Transform::reflection(Point3::new(x, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
                .expect("unit normal")
        }),
    ]
}

proptest! {
    /// Any selection of the scene's two boxes and two guides, through any
    /// move, rotation, scale or reflection: the mixed transform lands where
    /// the single-kind ops do, in one step that undo and redo reproduce
    /// exactly; and an array copy of the same selection is one step too.
    #[test]
    fn mixed_transforms_and_copies_round_trip(
        mask in 1u8..16,
        t in arb_transform(),
        count in 1u32..4,
        linked in any::<bool>(),
    ) {
        let Scene { mut doc, a, b, point, line } = scene();
        let Scene { doc: mut twin, .. } = scene();
        let nodes: Vec<NodeId> = [(1, a), (2, b)]
            .into_iter()
            .filter(|(bit, _)| mask & bit != 0)
            .map(|(_, o)| NodeId::Object(o))
            .collect();
        let guides: Vec<GuideId> = [(4, point), (8, line)]
            .into_iter()
            .filter(|(bit, _)| mask & bit != 0)
            .map(|(_, g)| g)
            .collect();

        let depth = doc.undo_depth();
        let before = doc.state_hash();
        doc.transform_mixed_selection(&nodes, &[], &guides, &t)
            .expect("a generated transform is valid");
        prop_assert_eq!(doc.undo_depth(), depth + 1);
        if !nodes.is_empty() {
            twin.transform_selection(&nodes, &[], &t).expect("node half");
        }
        for &g in &guides {
            twin.transform_guide(g, &t).expect("guide half");
        }
        prop_assert_eq!(doc.state_hash(), twin.state_hash());
        let after = doc.state_hash();
        doc.undo().expect("undo");
        prop_assert_eq!(doc.state_hash(), before);
        doc.redo().expect("redo");
        prop_assert_eq!(doc.state_hash(), after);

        let policy = if linked { LinkPolicy::Linked } else { LinkPolicy::Independent };
        let step = Transform::translation(Vec3::new(0.0, 6.0, 0.0));
        let count = NonZeroU32::new(count).expect("nonzero");
        let depth = doc.undo_depth();
        let (_, copies, _) = doc
            .duplicate_mixed_array(&nodes, &guides, &step, count, policy)
            .expect("array");
        prop_assert_eq!(doc.undo_depth(), depth + 1);
        prop_assert_eq!(copies.len(), guides.len() * count.get() as usize);
        let copied = doc.state_hash();
        doc.undo().expect("undo the copy");
        prop_assert_eq!(doc.state_hash(), after);
        doc.redo().expect("redo the copy");
        prop_assert_eq!(doc.state_hash(), copied);
    }
}
