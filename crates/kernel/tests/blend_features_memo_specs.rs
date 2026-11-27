//! Executable specs for the memory behind [`Document::blend_features`]:
//! a repeat call on an unchanged object answers without running
//! [`kernel::Object::blend_features`] again, and after every kind of
//! change the answer is the one a fresh, uncached call finds.

use crate::common;

use common::curves::*;
use kernel::{
    BlendProfile, BlendSpec, CompoundMeta, CornerStyle, Document, EdgeId, HistoryOrigin, NodeId,
    ObjectId, Point3, Transform, Vec3,
};

fn build_box(doc: &mut Document, x: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(x, 0.0), (x + 1.0, 0.0), (x + 1.0, 1.0), (x, 1.0)];
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

/// The top edge of the box at `x` along y = `y`.
fn top_edge(doc: &Document, o: ObjectId, x: f64, y: f64) -> EdgeId {
    edge_between(doc.object(o).unwrap(), p3(x, y, 1.0), p3(x + 1.0, y, 1.0))
}

/// The document's answer, held to a fresh computation on the object.
fn assert_fresh(doc: &Document, o: ObjectId, what: &str) {
    let answered = doc.blend_features(o).unwrap();
    let fresh = doc.object(o).unwrap().blend_features();
    assert_eq!(answered, fresh, "{what}");
}

/// After a change: the document's answer is the fresh one, and differs
/// from `before` (what the document answered just before the change, still
/// in its memory), so a stale answer could not pass.
fn assert_changed(doc: &Document, o: ObjectId, before: &[kernel::BlendFeature], what: &str) {
    let fresh = doc.object(o).unwrap().blend_features();
    assert_ne!(fresh, before, "{what}: the change changes the answer");
    assert_fresh(doc, o, what);
}

/// The document's answer now, which it then remembers.
fn primed(doc: &Document, o: ObjectId) -> Vec<kernel::BlendFeature> {
    doc.blend_features(o).unwrap()
}

/// How many times the object's blends were computed while `f` ran.
fn scans<T>(f: impl FnOnce() -> T) -> (usize, T) {
    let before = kernel::blend_feature_scans_for_test();
    let out = f();
    (kernel::blend_feature_scans_for_test() - before, out)
}

/// A box with one fillet, and a second plain box beside it.
fn two_boxes() -> (Document, ObjectId, ObjectId) {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    let e = top_edge(&doc, a, 0.0, 0.0);
    doc.blend_edges(a, &round(vec![e], 0.1)).unwrap();
    (doc, a, b)
}

#[test]
fn a_repeat_call_on_an_unchanged_object_does_not_recompute() {
    let (doc, a, _) = two_boxes();
    let (n, first) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 1);
    assert_eq!(first.len(), 1);
    for _ in 0..3 {
        let (n, again) = scans(|| doc.blend_features(a).unwrap());
        assert_eq!(n, 0, "a repeat call answers from memory");
        assert_eq!(again, first);
    }
}

#[test]
fn an_edit_of_the_object_is_seen() {
    let (mut doc, a, _) = two_boxes();
    assert_fresh(&doc, a, "before");
    let e = top_edge(&doc, a, 0.0, 1.0);
    doc.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    let (n, _) = scans(|| assert_fresh(&doc, a, "after a second fillet"));
    assert_eq!(n, 2, "the edit is a miss (and the check's own fresh call)");
    assert_eq!(doc.blend_features(a).unwrap().len(), 2);
    let id = doc.blend_features(a).unwrap()[0].id;
    doc.remove_blend(a, id).unwrap();
    assert_fresh(&doc, a, "after a remove");
    assert_eq!(doc.blend_features(a).unwrap().len(), 1);
    let id = doc.blend_features(a).unwrap()[0].id;
    doc.resize_blend(
        a,
        id,
        &BlendProfile::Round {
            radius: 0.02,
            segments: 4,
        },
    )
    .unwrap();
    assert_fresh(&doc, a, "after a resize");
    // A move changes no blend's answer, so it is held to a recount: the
    // moved object is a miss.
    doc.transform_object(a, &Transform::translation(Vec3::new(0.0, 0.0, 0.5)))
        .unwrap();
    let (n, _) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 1, "a moved object is computed afresh");
    assert_fresh(&doc, a, "after a move");
    // A scale changes the size.
    let before = primed(&doc, a);
    doc.transform_object(a, &Transform::uniform_scale(2.0))
        .unwrap();
    assert_changed(&doc, a, &before, "after a scale");
}

#[test]
fn an_edit_of_another_object_keeps_the_answer_right() {
    let (mut doc, a, b) = two_boxes();
    assert_fresh(&doc, a, "before");
    // The other object's blends differ from the first's in count and size.
    let e = top_edge(&doc, b, 3.0, 0.0);
    doc.blend_edges(b, &round(vec![e], 0.05)).unwrap();
    let e = top_edge(&doc, b, 3.0, 1.0);
    doc.blend_edges(b, &round(vec![e], 0.03)).unwrap();
    assert_ne!(
        doc.object(a).unwrap().blend_features(),
        doc.object(b).unwrap().blend_features()
    );
    // Asking about the other object in between takes the one entry.
    assert_fresh(&doc, b, "the edited object");
    assert_fresh(&doc, a, "the first object, after the other's edit");
    let (n, _) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 0, "the untouched object answers from memory again");
}

#[test]
fn undo_and_redo_are_seen() {
    let (mut doc, a, _) = two_boxes();
    let e = top_edge(&doc, a, 0.0, 1.0);
    doc.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    assert_fresh(&doc, a, "two fillets");
    assert_eq!(doc.blend_features(a).unwrap().len(), 2);
    doc.undo().unwrap();
    assert_fresh(&doc, a, "after undo");
    assert_eq!(doc.blend_features(a).unwrap().len(), 1);
    doc.redo().unwrap();
    assert_fresh(&doc, a, "after redo");
    assert_eq!(doc.blend_features(a).unwrap().len(), 2);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_fresh(&doc, a, "after undoing both");
    assert!(doc.blend_features(a).unwrap().is_empty());
}

/// A replaced or reloaded document brings its own memory, never the old
/// one's: the memory is part of the `Document` value, so this holds by
/// construction (the stamp is not what keeps it right).
#[test]
fn a_replaced_or_reloaded_document_brings_its_own_memory() {
    let (doc, a, _) = two_boxes();
    assert_fresh(&doc, a, "before");
    let mut other = Document::new();
    let o = build_box(&mut other, 0.0);
    assert_eq!(o, a, "the new document reuses the object's handle");
    assert_fresh(&other, o, "a different document under the same handle");
    assert!(other.blend_features(o).unwrap().is_empty());
    let loaded = Document::load(&doc.save()).unwrap();
    let la = loaded.live_object_ids()[0];
    assert_fresh(&loaded, la, "a reloaded document");
    // A host replacing its document in place: the old one's answer goes
    // with it.
    let mut host = doc;
    assert_fresh(&host, a, "the first document");
    let _old = std::mem::replace(&mut host, other);
    assert_fresh(&host, a, "the replacement under the same handle");
    assert!(host.blend_features(a).unwrap().is_empty());
}

#[test]
fn rolled_back_brackets_and_refusals_are_seen() {
    let (mut doc, a, _) = two_boxes();
    assert_fresh(&doc, a, "before");
    // A transaction that blends, then aborts.
    let txn = doc.begin_transaction();
    let e = top_edge(&doc, a, 0.0, 1.0);
    doc.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    assert_fresh(&doc, a, "inside the transaction");
    assert_eq!(doc.blend_features(a).unwrap().len(), 2);
    doc.abort_transaction(txn);
    assert_fresh(&doc, a, "after the abort");
    assert_eq!(doc.blend_features(a).unwrap().len(), 1);
    // A compound that blends, then aborts.
    let token = doc.begin_compound(CompoundMeta {
        label: "Blend".into(),
        origin: HistoryOrigin::User,
        coalesce: None,
    });
    doc.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    assert_fresh(&doc, a, "inside the compound");
    doc.abort_compound(token).unwrap();
    assert_fresh(&doc, a, "after the compound's abort");
    assert_eq!(doc.blend_features(a).unwrap().len(), 1);
    // A dry run on a clone leaves the original's answer alone.
    let mut rehearsal = doc.clone();
    rehearsal.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    assert_fresh(&rehearsal, a, "the rehearsal");
    assert_fresh(&doc, a, "the original after a rehearsal");
    assert_eq!(doc.blend_features(a).unwrap().len(), 1);
    // A refused blend changes nothing, and the answer stands.
    assert!(doc.blend_edges(a, &round(vec![e], 5.0)).is_err());
    let (n, _) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 0, "a refusal leaves the object as it was");
    assert_fresh(&doc, a, "after a refusal");
}

#[test]
fn a_deleted_object_is_refused_as_before() {
    let (mut doc, a, _) = two_boxes();
    // Remembered with one fillet; then a second, unasked, and a delete.
    let before = primed(&doc, a);
    let e = top_edge(&doc, a, 0.0, 1.0);
    doc.blend_edges(a, &round(vec![e], 0.05)).unwrap();
    doc.delete_node(NodeId::Object(a)).unwrap();
    assert!(doc.blend_features(a).is_err(), "the gate still runs first");
    doc.undo().unwrap();
    assert_changed(&doc, a, &before, "after the delete is undone");
}

/// The operations that put a whole new Object in an object's place within
/// the same document (a rescale, a rebase or transform of a definition's
/// members, an explode session's close, and their undo and redo): each
/// answer is the fresh one.
#[test]
fn objects_replaced_in_place_are_seen() {
    // A world object scaled with the document, then the undo and redo.
    let (mut doc, a, _) = two_boxes();
    let before = primed(&doc, a);
    doc.rescale_document(2.0).unwrap();
    assert_changed(&doc, a, &before, "after a rescale");
    let before = primed(&doc, a);
    doc.undo().unwrap();
    assert_changed(&doc, a, &before, "after the rescale is undone");
    let before = primed(&doc, a);
    doc.redo().unwrap();
    assert_changed(&doc, a, &before, "after the rescale is redone");
}

/// A definition whose member `a` carries a fillet, placed once plainly and
/// once at twice the size.
fn blended_component() -> (Document, kernel::ComponentId, kernel::InstanceId, ObjectId) {
    let (mut doc, a, _) = two_boxes();
    let (comp, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (big, _) = doc
        .place_instance(comp, Transform::uniform_scale(2.0))
        .unwrap();
    (doc, comp, big, a)
}

#[test]
fn definition_members_replaced_in_place_are_seen() {
    let (mut doc, comp, _, a) = blended_component();
    // Scaling the member through the definition, then the undo and redo.
    let before = primed(&doc, a);
    doc.transform_def_members(comp, &[NodeId::Object(a)], &Transform::uniform_scale(2.0))
        .unwrap();
    assert_changed(&doc, a, &before, "after the member is scaled");
    let before = primed(&doc, a);
    doc.undo().unwrap();
    assert_changed(&doc, a, &before, "after the member's scale is undone");
    let before = primed(&doc, a);
    doc.redo().unwrap();
    assert_changed(&doc, a, &before, "after the member's scale is redone");
    // A rebase turns the member without changing its blends: held to a
    // recount, the rebased member is a miss, as is its undo.
    primed(&doc, a);
    let turn = std::f64::consts::FRAC_PI_2;
    doc.rebase_definition(
        comp,
        Point3::new(0.5, 0.5, 0.0),
        Vec3::new(kernel::trig::cos(turn), kernel::trig::sin(turn), 0.0),
        Vec3::new(-kernel::trig::sin(turn), kernel::trig::cos(turn), 0.0),
    )
    .unwrap();
    let (n, _) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 1, "a rebased member is computed afresh");
    assert_fresh(&doc, a, "after a rebase");
    doc.undo().unwrap();
    let (n, _) = scans(|| doc.blend_features(a).unwrap());
    assert_eq!(n, 1, "an undone rebase is computed afresh");
    assert_fresh(&doc, a, "after the rebase is undone");
}

#[test]
fn an_explode_session_and_its_close_are_seen() {
    let (mut doc, _, big, a) = blended_component();
    // Opening the larger placement bakes its pose into the member: twice
    // the size.
    let before = primed(&doc, a);
    doc.open_explode_session(big).unwrap();
    assert_changed(&doc, a, &before, "inside the session");
    // Closing with the member untouched puts its pristine copy back.
    let before = primed(&doc, a);
    doc.close_explode_session().unwrap();
    assert_changed(&doc, a, &before, "after the session closes");
    // And the session's undo, step by step.
    let before = primed(&doc, a);
    doc.undo().unwrap();
    assert_changed(&doc, a, &before, "after the close is undone");
    let before = primed(&doc, a);
    doc.undo().unwrap();
    assert_changed(&doc, a, &before, "after the open is undone");
}
