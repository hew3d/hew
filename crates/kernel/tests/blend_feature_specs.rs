//! Executable specs for asking about one blend ([`Object::blend_feature`],
//! [`Document::blend_feature`]): the answer is exactly the row the whole
//! list ([`Object::blend_features`]) has for that blend, it verifies that
//! blend alone, and the document's memory of it is dropped by every change
//! that drops the list's.

use crate::common;

use common::curves::*;
use kernel::{
    BlendFeature, BlendId, BlendProfile, BlendSpec, CornerStyle, Document, EdgeId, KernelOp,
    Object, ObjectId, Point3, Vec3,
};

fn round(edges: Vec<EdgeId>, radius: f64) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Round {
            radius,
            segments: 4,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn chamfer(edges: Vec<EdgeId>, size: f64) -> BlendSpec {
    BlendSpec {
        profile: BlendProfile::Chamfer {
            setback_a: size,
            setback_b: size,
        },
        ..round(edges, size)
    }
}

fn blended(obj: &Object, spec: &BlendSpec) -> Object {
    obj.blend_edges(spec).unwrap().0
}

/// A unit box's top edge along y = `y`.
fn top_edge(obj: &Object, y: f64) -> EdgeId {
    edge_between(obj, p3(0.0, y, 1.0), p3(1.0, y, 1.0))
}

/// A unit box with one round on its front top edge.
fn one_round() -> Object {
    let b = unit_box();
    blended(&b, &round(vec![top_edge(&b, 0.0)], 0.1))
}

/// A unit box with a round on its front top edge and a chamfer on the left
/// top edge, meeting at a corner.
fn round_and_chamfer() -> Object {
    let a = one_round();
    let e = edge_between(&a, p3(0.0, 0.0 + 0.1, 1.0), p3(0.0, 1.0, 1.0));
    blended(&a, &chamfer(vec![e], 0.05))
}

/// Two rounds on opposite top edges of a unit box.
fn two_rounds() -> Object {
    let a = one_round();
    blended(&a, &round(vec![top_edge(&a, 1.0)], 0.05))
}

/// A claimed cylinder of `n` facets with both rims filleted: the later
/// fillet trims the walls the first one trimmed, so verifying the first
/// takes the second off and puts it back.
fn stacked_rims(n: usize) -> Object {
    let c = claimed_cylinder(n);
    let one = blended(&c, &round(sharp_edges_at_height(&c, 0.1), 0.01));
    blended(&one, &round(sharp_edges_at_height(&one, 0.0), 0.01))
}

/// A unit box with a 200 mm round on its front top edge, then its top face
/// pushed down 900 mm: the round no longer fits, so the blend is no longer
/// intact.
fn pushed_through() -> Object {
    let b = unit_box();
    let mut r = blended(&b, &round(vec![top_edge(&b, 0.0)], 0.2));
    let top = face_with_normal(&r, Vec3::new(0.0, 0.0, 1.0));
    r.push_pull(top, -0.9).unwrap();
    r
}

fn fixtures() -> Vec<(&'static str, Object)> {
    vec![
        ("plain box", unit_box()),
        ("one round", one_round()),
        ("round and chamfer", round_and_chamfer()),
        ("two rounds", two_rounds()),
        ("stacked rims", stacked_rims(64)),
        ("pushed through", pushed_through()),
    ]
}

/// The ids no face of `obj` carries, next to the ones it does.
fn absent_ids(list: &[BlendFeature]) -> Vec<BlendId> {
    let top = list.iter().map(|f| f.id.0).max().unwrap_or(0);
    vec![
        BlendId(0),
        BlendId(top + 1),
        BlendId(top + 7),
        BlendId(u32::MAX),
    ]
    .into_iter()
    .filter(|id| list.iter().all(|f| f.id != *id))
    .collect()
}

#[test]
fn one_blend_is_its_row_of_the_list() {
    let mut saw_not_intact = false;
    let mut saw_stacked = false;
    for (name, obj) in fixtures() {
        let list = obj.blend_features();
        for row in &list {
            let one = obj.blend_feature(row.id);
            assert_eq!(one.as_ref(), Some(row), "{name}: blend {:?}", row.id);
            saw_not_intact |= !row.intact;
        }
        for id in absent_ids(&list) {
            assert_eq!(obj.blend_feature(id), None, "{name}: absent {id:?}");
        }
        saw_stacked |= name == "stacked rims" && list.len() == 2 && list.iter().all(|f| f.intact);
    }
    assert!(saw_not_intact, "a fixture has a blend that is not intact");
    assert!(saw_stacked, "the stacked rims are both intact");
}

/// A removed blend's id is gone from the single query as it is from the
/// list.
#[test]
fn a_removed_blend_is_none() {
    let (mut doc, o, a, b) = doc_two_rounds();
    doc.remove_blend(o, a).unwrap();
    let after = doc.object(o).unwrap();
    assert_eq!(after.blend_feature(a), None);
    let list = after.blend_features();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, b);
    assert_eq!(after.blend_feature(b).as_ref(), Some(&list[0]));
}

/// How many blends were verified while `f` ran.
fn verified<T>(f: impl FnOnce() -> T) -> (usize, T) {
    let before = kernel::blend_verifications_for_test();
    let out = f();
    (kernel::blend_verifications_for_test() - before, out)
}

#[test]
fn one_blend_verifies_that_blend_alone() {
    for (name, obj) in [
        ("two rounds", two_rounds()),
        ("round and chamfer", round_and_chamfer()),
        ("stacked rims", stacked_rims(64)),
    ] {
        let (n, list) = verified(|| obj.blend_features());
        assert_eq!(n, list.len(), "{name}: the list verifies every blend");
        assert!(list.len() >= 2, "{name}");
        for row in &list {
            let (n, _) = verified(|| obj.blend_feature(row.id));
            assert_eq!(n, 1, "{name}: blend {:?} alone", row.id);
        }
        let (n, none) = verified(|| obj.blend_feature(BlendId(u32::MAX)));
        assert_eq!(
            (n, none),
            (0, None),
            "{name}: an absent id verifies nothing"
        );
    }
}

// --- the document's memory ---------------------------------------------

fn build_box(doc: &mut Document) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

/// A document box with rounds on two opposite top edges.
fn doc_two_rounds() -> (Document, ObjectId, BlendId, BlendId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let e = top_edge(doc.object(o).unwrap(), 0.0);
    let (a, _) = doc.blend_edges(o, &round(vec![e], 0.1)).unwrap();
    let e = top_edge(doc.object(o).unwrap(), 1.0);
    let (b, _) = doc.blend_edges(o, &round(vec![e], 0.05)).unwrap();
    (doc, o, a.blends[0], b.blends[0])
}

/// The document's single answer, held to a fresh one on the object.
fn assert_fresh(doc: &Document, o: ObjectId, id: BlendId, what: &str) {
    let answered = doc.blend_feature(o, id).unwrap();
    let fresh = doc.object(o).unwrap().blend_feature(id);
    assert_eq!(answered, fresh, "{what}");
}

#[test]
fn the_document_answers_one_blend_from_memory() {
    let (doc, o, a, b) = doc_two_rounds();
    let (n, first) = verified(|| doc.blend_feature(o, a).unwrap());
    assert_eq!(n, 1, "the first ask verifies the one blend");
    assert!(first.as_ref().is_some_and(|f| f.intact));
    let (n, again) = verified(|| doc.blend_feature(o, a).unwrap());
    assert_eq!((n, &again), (0, &first), "a repeat ask answers from memory");
    let (n, other) = verified(|| doc.blend_feature(o, b).unwrap());
    assert_eq!(n, 1, "another blend is verified on its own");
    let (n, _) = verified(|| doc.blend_feature(o, a).unwrap());
    assert_eq!(n, 0, "the first blend is still remembered");
    // An absent id is remembered too.
    let (n, none) = verified(|| doc.blend_feature(o, BlendId(u32::MAX)).unwrap());
    assert_eq!((n, none), (0, None));
    // The list is not served from the single answers, but once found it
    // answers every single blend.
    let (n, list) = verified(|| doc.blend_features(o).unwrap());
    assert_eq!(n, 2, "the list verifies every blend");
    assert_eq!(list, vec![first.unwrap(), other.unwrap()]);
    let (n, _) = verified(|| doc.blend_features(o).unwrap());
    assert_eq!(n, 0, "the list is remembered");
    for row in &list {
        let (n, one) = verified(|| doc.blend_feature(o, row.id).unwrap());
        assert_eq!(
            (n, one.as_ref()),
            (0, Some(row)),
            "a row of the remembered list"
        );
    }
}

#[test]
fn a_remembered_list_answers_one_blend() {
    let (doc, o, a, b) = doc_two_rounds();
    let list = doc.blend_features(o).unwrap();
    for id in [a, b] {
        let (n, one) = verified(|| doc.blend_feature(o, id).unwrap());
        assert_eq!(n, 0);
        assert_eq!(one.as_ref(), list.iter().find(|f| f.id == id));
    }
    let (n, none) = verified(|| doc.blend_feature(o, BlendId(u32::MAX)).unwrap());
    assert_eq!((n, none), (0, None), "an id the remembered list lacks");
}

#[test]
fn edits_undo_and_redo_drop_the_remembered_blend() {
    let (mut doc, o, a, b) = doc_two_rounds();
    let before = doc.blend_feature(o, a).unwrap().unwrap();
    // A resize of the blend changes its answer.
    doc.resize_blend(
        o,
        a,
        &BlendProfile::Round {
            radius: 0.02,
            segments: 4,
        },
    )
    .unwrap();
    let fresh = doc.object(o).unwrap().blend_feature(a);
    assert_ne!(
        fresh.as_ref(),
        Some(&before),
        "the resize changes the answer"
    );
    let (n, _) = verified(|| assert_fresh(&doc, o, a, "after a resize"));
    assert_eq!(n, 2, "a miss (and the check's own fresh call)");
    let resized = doc.blend_feature(o, a).unwrap().unwrap();
    // Undo brings the first answer back, redo the resized one.
    doc.undo().unwrap();
    assert_fresh(&doc, o, a, "after undo");
    assert_eq!(doc.blend_feature(o, a).unwrap().as_ref(), Some(&before));
    doc.redo().unwrap();
    assert_fresh(&doc, o, a, "after redo");
    assert_eq!(doc.blend_feature(o, a).unwrap().as_ref(), Some(&resized));
    // A remove makes the id absent; its undo brings it back.
    doc.remove_blend(o, b).unwrap();
    assert_fresh(&doc, o, b, "after a remove");
    assert_eq!(doc.blend_feature(o, b).unwrap(), None);
    doc.undo().unwrap();
    assert_fresh(&doc, o, b, "after the remove is undone");
    assert!(doc.blend_feature(o, b).unwrap().is_some());
    // A push that takes the blend's intactness away is seen, and so are
    // its undo and redo.
    let (mut doc, o, a) = doc_big_round();
    assert!(doc.blend_feature(o, a).unwrap().unwrap().intact);
    let top = face_with_normal(doc.object(o).unwrap(), Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face: top,
            distance: -0.9,
        },
    )
    .unwrap();
    assert_fresh(&doc, o, a, "after the push");
    assert!(!doc.blend_feature(o, a).unwrap().unwrap().intact);
    doc.undo().unwrap();
    assert_fresh(&doc, o, a, "after the push is undone");
    assert!(doc.blend_feature(o, a).unwrap().unwrap().intact);
    doc.redo().unwrap();
    assert_fresh(&doc, o, a, "after the push is redone");
    assert!(!doc.blend_feature(o, a).unwrap().unwrap().intact);
}

/// A document box with a 200 mm round on its front top edge.
fn doc_big_round() -> (Document, ObjectId, BlendId) {
    let mut doc = Document::new();
    let o = build_box(&mut doc);
    let e = top_edge(doc.object(o).unwrap(), 0.0);
    let (r, _) = doc.blend_edges(o, &round(vec![e], 0.2)).unwrap();
    (doc, o, r.blends[0])
}

#[test]
fn a_single_answer_is_not_served_for_another_object_or_a_deleted_one() {
    let (mut doc, o, a, _) = doc_two_rounds();
    let other = build_box(&mut doc);
    assert!(doc.blend_feature(o, a).unwrap().is_some());
    assert_eq!(
        doc.blend_feature(other, a).unwrap(),
        None,
        "the other object has no blends"
    );
    assert_fresh(&doc, o, a, "back on the first object");
    doc.delete_node(kernel::NodeId::Object(o)).unwrap();
    assert!(
        doc.blend_feature(o, a).is_err(),
        "the gate still runs first"
    );
    doc.undo().unwrap();
    assert_fresh(&doc, o, a, "after the delete is undone");
}
