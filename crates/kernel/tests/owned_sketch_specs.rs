//! Executable specs for object-owned sketches (unified drafting, Stage 1):
//! drawing that belongs to an Object rides with it — moves with it, lives
//! and dies with it, follows pushed faces, is carried through booleans and
//! slices, and is copied with duplicates, exploded instances, private
//! copies, and library items. Every spec runs under the kernel's debug
//! validator (owner live and same frame, a coplanar same-facing owner face
//! exists, no owned edge duplicates an owner edge).

use kernel::{
    BooleanOp, Document, DocumentError, FaceId, InsertOptions, KernelOp, NodeId, ObjectId, Plane,
    Point3, SketchId, Transform, Vec3, tol,
};

mod face_arrangement_support;
use face_arrangement_support::{draw_loop, draw_path};

// ----------------------------------------------------------------- helpers

fn plane_z(z: f64) -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z),
        Point3::new(1.0, 0.0, z),
        Point3::new(0.0, 1.0, z),
    ])
    .expect("horizontal plane")
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A box `[x0,x1] x [y0,y1] x [z0, z0+h]`, drawn on the plane `z = z0`.
fn make_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, z0: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(plane_z(z0));
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let c = [p(x0, y0, z0), p(x1, y0, z0), p(x1, y1, z0), p(x0, y1, z0)];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).expect("rect segment");
        }
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1);
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// The unit box `[0,1]^3`.
fn unit_box(doc: &mut Document) -> ObjectId {
    make_box(doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0)
}

/// The face of `obj` whose plane normal is `dir` and that contains `inside`.
fn face_at(doc: &Document, obj: ObjectId, dir: Vec3, inside: Point3) -> FaceId {
    let o = doc.object(obj).expect("object is live");
    o.faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION)
                && f.plane.signed_distance(inside).abs() < 1e-9
                && {
                    let ps: Vec<Point3> = o.loop_positions(f.outer_loop).collect();
                    let (lo, hi) = bounds(&ps);
                    inside.x >= lo.x - 1e-9
                        && inside.x <= hi.x + 1e-9
                        && inside.y >= lo.y - 1e-9
                        && inside.y <= hi.y + 1e-9
                        && inside.z >= lo.z - 1e-9
                        && inside.z <= hi.z + 1e-9
                }
        })
        .map(|(id, _)| id)
        .expect("face found")
}

fn bounds(ps: &[Point3]) -> (Point3, Point3) {
    let mut lo = p(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = p(f64::MIN, f64::MIN, f64::MIN);
    for q in ps {
        lo = p(lo.x.min(q.x), lo.y.min(q.y), lo.z.min(q.z));
        hi = p(hi.x.max(q.x), hi.y.max(q.y), hi.z.max(q.z));
    }
    (lo, hi)
}

fn top_face(doc: &Document, obj: ObjectId) -> FaceId {
    let o = doc.object(obj).expect("object is live");
    let (lo, hi) = bounds(
        &o.vertices()
            .values()
            .map(|v| v.position)
            .collect::<Vec<_>>(),
    );
    face_at(
        doc,
        obj,
        Vec3::new(0.0, 0.0, 1.0),
        p((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, hi.z),
    )
}

/// Draws `segs` into the committed sketch `s` as one undoable gesture.
fn draw(doc: &mut Document, s: SketchId, segs: &[(Point3, Point3)]) {
    doc.begin_sketch_gesture(s).expect("begin gesture");
    for &(a, b) in segs {
        doc.sketch_mut(s)
            .expect("sketch is live")
            .add_segment(a, b)
            .expect("segment");
    }
    doc.end_sketch_gesture(s).expect("end gesture");
}

/// A new sketch on `plane` holding `segs`, owned by `owner`.
fn owned_sketch(
    doc: &mut Document,
    owner: ObjectId,
    z: f64,
    segs: &[(Point3, Point3)],
) -> SketchId {
    let s = doc.add_sketch(plane_z(z));
    draw(doc, s, segs);
    doc.set_sketch_object_owner(s, Some(owner))
        .expect("own the sketch");
    s
}

/// Two loose, disjoint segments over the unit box's top: `near` (x < 0.5)
/// and `far` (x > 0.5).
fn near() -> (Point3, Point3) {
    (p(0.2, 0.2, 1.0), p(0.4, 0.3, 1.0))
}
fn far() -> (Point3, Point3) {
    (p(0.7, 0.5, 1.0), p(0.9, 0.6, 1.0))
}

fn verts(doc: &Document, s: SketchId) -> Vec<Point3> {
    doc.sketch(s)
        .expect("sketch is live")
        .vertices()
        .values()
        .map(|v| v.position)
        .collect()
}

/// Whether the two point sets match (as sets) within the merge tolerance.
fn same_points(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| b.iter().any(|y| x.approx_eq(*y, tol::POINT_MERGE)))
}

fn assert_moved(doc: &Document, s: SketchId, original: &[Point3], t: &Transform) {
    let expected: Vec<Point3> = original.iter().map(|&q| t.apply_point(q)).collect();
    let actual = verts(doc, s);
    assert!(
        same_points(&actual, &expected),
        "sketch vertices {actual:?} should be the originals {original:?} moved by the transform"
    );
}

fn live_sketches(doc: &Document) -> Vec<SketchId> {
    doc.sketch_ids()
}

// -------------------------------------------------------- set / clear owner

#[test]
fn set_and_clear_owner_round_trips_through_undo_and_redo() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = doc.add_sketch(plane_z(1.0));
    draw(&mut doc, s, &[near()]);
    assert_eq!(doc.sketch_owner_object(s), None);
    let bare = doc.save();

    let change = doc.set_sketch_object_owner(s, Some(b)).expect("own");
    assert_eq!(change.sketches_touched, vec![s]);
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    assert_eq!(doc.object_owned_sketches(b), vec![s]);
    let owned = doc.save();

    // Owning again is a no-op: no undo record.
    let before = doc.can_redo();
    assert!(
        doc.set_sketch_object_owner(s, Some(b))
            .expect("noop")
            .sketches_touched
            .is_empty()
    );
    assert_eq!(doc.can_redo(), before);

    doc.undo().expect("undo own");
    assert_eq!(doc.sketch_owner_object(s), None);
    assert!(doc.object_owned_sketches(b).is_empty());
    assert_eq!(doc.save(), bare, "undo restores the unowned file bytes");
    doc.redo().expect("redo own");
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    assert_eq!(doc.save(), owned, "redo restores the owned file bytes");

    doc.set_sketch_object_owner(s, None).expect("clear");
    assert_eq!(doc.sketch_owner_object(s), None);
    doc.undo().expect("undo clear");
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    doc.redo().expect("redo clear");
    assert_eq!(doc.sketch_owner_object(s), None);
    assert!(
        doc.sketch(s).is_some(),
        "a cleared sketch is a plain sketch"
    );
}

#[test]
fn set_owner_refusals_are_typed_and_leave_the_document_untouched() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);

    // A sketch on a plane no face of the box lies on.
    let off = doc.add_sketch(plane_z(5.0));
    draw(&mut doc, off, &[(p(0.2, 0.2, 5.0), p(0.4, 0.3, 5.0))]);
    // Same plane as the top but opposite facing is no match either.
    let flipped = doc.add_sketch(
        Plane::from_polygon(&[p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(1.0, 0.0, 1.0)]).unwrap(),
    );
    draw(&mut doc, flipped, &[near()]);
    // A definition sketch cannot belong to a world object.
    let moved = make_box(&mut doc, 5.0, 5.0, 6.0, 6.0, 0.0, 1.0);
    let (_comp, inst, _) = doc
        .make_component(&[NodeId::Object(moved)])
        .expect("component");
    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, plane_z(1.0))
        .expect("def sketch");
    draw(&mut doc, def_sketch, &[near()]);
    let bytes = doc.save();

    for s in [off, flipped, def_sketch] {
        assert!(matches!(
            doc.set_sketch_object_owner(s, Some(b)),
            Err(DocumentError::SketchOwnerMismatch)
        ));
        assert_eq!(doc.sketch_owner_object(s), None);
        assert_eq!(doc.save(), bytes, "a refusal leaves the bytes untouched");
    }

    // Stale handles.
    let dead = doc.add_sketch(plane_z(1.0));
    draw(&mut doc, dead, &[near()]);
    doc.delete_sketch(dead).expect("delete sketch");
    assert!(matches!(
        doc.set_sketch_object_owner(dead, Some(b)),
        Err(DocumentError::UnknownSketch)
    ));
    let live = doc.add_sketch(plane_z(1.0));
    draw(&mut doc, live, &[near()]);
    let bytes = doc.save();
    doc.delete_node(NodeId::Object(b)).expect("delete box");
    assert!(matches!(
        doc.set_sketch_object_owner(live, Some(b)),
        Err(DocumentError::UnknownObject)
    ));
    doc.undo().expect("undo delete");
    assert_eq!(doc.save(), bytes);
}

// ----------------------------------------------------- derived liveness

#[test]
fn deleting_the_owner_takes_the_sketch_and_undo_brings_it_back() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let with = doc.save();
    let original = verts(&doc, s);
    assert!(live_sketches(&doc).contains(&s));

    doc.delete_node(NodeId::Object(b)).expect("delete");
    assert!(!live_sketches(&doc).contains(&s));
    assert!(doc.sketch(s).is_none());
    assert!(doc.object_owned_sketches(b).is_empty());
    // Saving while deleted neither panics nor writes the dead sketch.
    let deleted = doc.save();
    let reloaded = Document::load(&deleted).expect("load while deleted");
    assert!(reloaded.sketch_ids().is_empty());

    doc.undo().expect("undo delete");
    assert!(live_sketches(&doc).contains(&s));
    assert!(same_points(&verts(&doc, s), &original));
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    assert_eq!(doc.save(), with);

    doc.redo().expect("redo delete");
    assert!(doc.sketch(s).is_none());
    assert_eq!(doc.save(), deleted);
}

// ------------------------------------------------------------- file format

#[test]
fn owner_object_round_trips_through_save_and_load() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let bytes = doc.save();

    let manifest = manifest_of(&bytes);
    assert_eq!(manifest["format_version"], serde_json::json!(29));
    let sk = &manifest["sketches"][0];
    assert!(sk["owner_object"].is_number(), "owner_object is written");
    assert!(sk.get("owner").is_none());

    let loaded = Document::load(&bytes).expect("load v17");
    let sids = loaded.sketch_ids();
    assert_eq!(sids.len(), 1);
    let owner = loaded.sketch_owner_object(sids[0]).expect("owner survives");
    assert!(loaded.is_world_object(owner));
    assert!(same_points(&verts(&loaded, sids[0]), &verts(&doc, s)));
    assert_eq!(loaded.save(), bytes, "re-saving is byte-stable");
}

#[test]
fn a_file_without_owner_object_loads_at_v16() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = doc.add_sketch(plane_z(1.0));
    draw(&mut doc, s, &[near()]);
    let _ = b;
    let bytes = doc.save();
    let v16 = patch_manifest(&bytes, |m| {
        m["format_version"] = serde_json::json!(16);
        for sk in m["sketches"].as_array_mut().unwrap() {
            sk.as_object_mut().unwrap().remove("owner_object");
        }
    });
    let loaded = Document::load(&v16).expect("a v16 file loads");
    assert_eq!(loaded.sketch_ids().len(), 1);
    assert_eq!(loaded.sketch_owner_object(loaded.sketch_ids()[0]), None);
}

#[test]
fn owner_object_in_a_v16_manifest_is_malformed() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    owned_sketch(&mut doc, b, 1.0, &[near()]);
    let smuggled = patch_manifest(&doc.save(), |m| {
        m["format_version"] = serde_json::json!(16);
    });
    assert!(matches!(
        Document::load(&smuggled),
        Err(kernel::LoadError::MalformedManifest { .. })
    ));
}

#[test]
fn out_of_range_owner_object_is_a_dangling_reference() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    owned_sketch(&mut doc, b, 1.0, &[near()]);
    let bad = patch_manifest(&doc.save(), |m| {
        m["sketches"][0]["owner_object"] = serde_json::json!(99);
    });
    assert!(matches!(
        Document::load(&bad),
        Err(kernel::LoadError::DanglingReference { .. })
    ));
}

#[test]
fn a_member_owner_without_owner_component_is_rejected() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(b)]).expect("component");
    let (s, _) = doc
        .begin_sketch_on_plane_in_instance(inst, plane_z(1.0))
        .expect("def sketch");
    draw(&mut doc, s, &[near()]);
    let member = doc.def_members(comp).expect("live")[0];
    doc.set_sketch_object_owner(s, Some(member)).expect("own");
    let bytes = doc.save();
    // Intact, the file loads and keeps the member ownership.
    let loaded = Document::load(&bytes).expect("member-owned def sketch loads");
    assert!(
        loaded
            .component_ids()
            .into_iter()
            .any(|c| loaded.def_member_sketches(c).is_some_and(|v| v.len() == 1))
    );
    // Dropping `owner` leaves the sketch a world sketch owned by a member.
    let bad = patch_manifest(&bytes, |m| {
        m["sketches"][0].as_object_mut().unwrap().remove("owner");
    });
    assert!(matches!(
        Document::load(&bad),
        Err(kernel::LoadError::MalformedManifest { .. })
    ));
}

// -------------------------------------------------------------- ride-along

fn pose() -> Transform {
    Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.6)
        .expect("rotation")
        .then(&Transform::translation(Vec3::new(3.0, -2.0, 0.5)))
}

#[test]
fn transform_object_moves_the_owned_sketch_with_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let original = verts(&doc, s);
    let before = doc.save();

    let change = doc.transform_object(b, &pose()).expect("transform");
    assert!(change.sketches_touched.contains(&s));
    assert_moved(&doc, s, &original, &pose());
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    let after = doc.save();

    doc.undo().expect("undo");
    assert!(same_points(&verts(&doc, s), &original));
    assert_eq!(doc.save(), before, "undo restores the exact bytes");
    doc.redo().expect("redo");
    assert_moved(&doc, s, &original, &pose());
    assert_eq!(doc.save(), after, "redo re-applies the exact bytes");
}

#[test]
fn transform_group_moves_each_members_owned_sketch() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let c = make_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let sa = owned_sketch(&mut doc, a, 1.0, &[near()]);
    let sc = owned_sketch(&mut doc, c, 1.0, &[(p(4.2, 0.2, 1.0), p(4.4, 0.3, 1.0))]);
    let (oa, oc) = (verts(&doc, sa), verts(&doc, sc));
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(c)])
        .expect("group");
    let before = doc.save();

    doc.transform_group(g, &pose()).expect("transform group");
    assert_moved(&doc, sa, &oa, &pose());
    assert_moved(&doc, sc, &oc, &pose());
    let after = doc.save();

    doc.undo().expect("undo");
    assert!(same_points(&verts(&doc, sa), &oa));
    assert!(same_points(&verts(&doc, sc), &oc));
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo");
    assert_eq!(doc.save(), after);
}

#[test]
fn transform_selection_moves_the_owned_sketch_of_a_selected_node() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let c = make_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let sa = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let sc = owned_sketch(&mut doc, c, 1.0, &[(p(4.2, 0.2, 1.0), p(4.4, 0.3, 1.0))]);
    let (oa, oc) = (verts(&doc, sa), verts(&doc, sc));
    let before = doc.save();

    doc.transform_selection(&[NodeId::Object(a)], &[], &pose())
        .expect("transform selection");
    assert_moved(&doc, sa, &oa, &pose());
    assert!(
        same_points(&verts(&doc, sc), &oc),
        "an unselected object's sketch stays"
    );
    let after = doc.save();

    doc.undo().expect("undo");
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo");
    assert_eq!(doc.save(), after);
}

#[test]
fn rescale_document_scales_owned_sketches_and_leaves_definition_sketches() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let original = verts(&doc, s);

    // A definition holding a box and a drawing of its own.
    let m = make_box(&mut doc, 5.0, 5.0, 6.0, 6.0, 0.0, 1.0);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(m)]).expect("component");
    let (ds, _) = doc
        .begin_sketch_on_plane_in_instance(inst, plane_z(1.0))
        .expect("def sketch");
    draw(&mut doc, ds, &[(p(5.2, 5.2, 1.0), p(5.4, 5.3, 1.0))]);
    let member = doc.def_members(comp).expect("live")[0];
    doc.set_sketch_object_owner(ds, Some(member)).expect("own");
    let def_before = verts(&doc, ds);
    let before = doc.save();

    let scale = Transform::uniform_scale(2.0);
    doc.rescale_document(2.0).expect("rescale");
    assert_moved(&doc, s, &original, &scale);
    assert!(
        same_points(&verts(&doc, ds), &def_before),
        "a definition sketch keeps its authored size (the instance pose absorbs the scale)"
    );
    let after = doc.save();

    doc.undo().expect("undo");
    assert!(same_points(&verts(&doc, s), &original));
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo");
    assert_eq!(doc.save(), after);
}

// ------------------------------------------------------------ push / pull

#[test]
fn pushing_the_whole_top_face_moves_the_owned_sketch_with_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let original = verts(&doc, s);
    let before = doc.save();

    let face = top_face(&doc, b);
    doc.apply_object_op(
        b,
        KernelOp::PushPull {
            face,
            distance: 0.5,
        },
    )
    .expect("push the top");
    let lift = Transform::translation(Vec3::new(0.0, 0.0, 0.5));
    assert_moved(&doc, s, &original, &lift);
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    assert_eq!(doc.object_owned_sketches(b), vec![s]);
    let after = doc.save();

    doc.undo()
        .expect("one undo restores the push and the drawing");
    assert!(same_points(&verts(&doc, s), &original));
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo");
    assert_eq!(doc.save(), after);
}

/// Imprints `[x0,x1] x [y0,y1]` on the unit box's top; returns the sub-face.
fn imprint_on_top(doc: &mut Document, b: ObjectId, x0: f64, y0: f64, x1: f64, y1: f64) -> FaceId {
    let top = top_face(doc, b);
    let ring = vec![
        p(x0, y0, 1.0),
        p(x1, y0, 1.0),
        p(x1, y1, 1.0),
        p(x0, y1, 1.0),
    ];
    let plane = face_arrangement_support::face_plane(doc, b, top);
    draw_loop(doc, None, b, top, &ring, None).expect("imprint");
    face_arrangement_support::region_face_in(doc, None, b, plane, &ring).expect("the imprint")
}

#[test]
fn raising_an_imprint_carries_only_the_edges_over_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let sub = imprint_on_top(&mut doc, b, 0.6, 0.1, 0.95, 0.9);
    let s = owned_sketch(&mut doc, b, 1.0, &[near(), far()]);
    let before = doc.save();

    doc.apply_object_op(
        b,
        KernelOp::ExtrudeSubFace {
            sub_face: sub,
            distance: 0.5,
        },
    )
    .expect("raise the imprint");

    // `near` stayed on the old top; `far` rode up into its own owned sketch.
    let owned = doc.object_owned_sketches(b);
    assert_eq!(owned.len(), 2, "the drawing split in two: {owned:?}");
    let (n, f) = (near(), far());
    let low = owned
        .iter()
        .copied()
        .find(|&o| same_points(&verts(&doc, o), &[n.0, n.1]))
        .expect("the staying edge keeps a sketch at the old plane");
    let high = owned
        .iter()
        .copied()
        .find(|&o| o != low)
        .expect("the riding edge");
    assert_eq!(low, s, "the staying part keeps the original sketch");
    let up = |q: Point3| p(q.x, q.y, q.z + 0.5);
    assert!(same_points(&verts(&doc, high), &[up(f.0), up(f.1)]));
    let after = doc.save();

    doc.undo().expect("a single undo restores the whole push");
    assert_eq!(doc.object_owned_sketches(b), vec![s]);
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo");
    assert_eq!(doc.save(), after);
    assert_eq!(doc.object_owned_sketches(b).len(), 2);
}

// -------------------------------------------------------------- consumers

/// The vertices of every live sketch `obj` owns, sorted for comparison.
fn owned_verts(doc: &Document, obj: ObjectId) -> Vec<Point3> {
    doc.object_owned_sketches(obj)
        .into_iter()
        .flat_map(|s| verts(doc, s))
        .collect()
}

#[test]
fn union_result_owns_a_copy_and_undo_restores_the_originals() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let c = make_box(&mut doc, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let original = verts(&doc, s);
    let before = doc.save();

    let (r, _) = doc.boolean(BooleanOp::Union, a, c).expect("union");
    assert!(doc.sketch(s).is_none(), "the operand's original is dead");
    assert!(same_points(&owned_verts(&doc, r), &original));
    let copies = doc.object_owned_sketches(r);
    assert_eq!(copies.len(), 1);
    assert_ne!(copies[0], s);
    let after = doc.save();

    doc.undo().expect("undo union");
    assert!(doc.sketch(s).is_some());
    assert_eq!(doc.sketch_owner_object(s), Some(a));
    assert!(
        doc.sketch(copies[0]).is_none(),
        "the copy dies with the result"
    );
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo union");
    assert!(doc.sketch(s).is_none());
    assert!(same_points(&owned_verts(&doc, r), &original));
    assert_eq!(doc.save(), after);
}

#[test]
fn subtract_keeps_the_surviving_edges_and_frees_the_cut_away_ones() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    // Removes everything with x > 0.5.
    let cutter = make_box(&mut doc, 0.5, -0.5, 1.5, 1.5, -0.5, 2.0);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let sketches_before = live_sketches(&doc);
    let before = doc.save();

    let (r, _) = doc
        .boolean(BooleanOp::Subtract, a, cutter)
        .expect("subtract");
    let (n, f) = (near(), far());
    assert!(
        same_points(&owned_verts(&doc, r), &[n.0, n.1]),
        "the edge over surviving material stays drawn on the result"
    );
    let free: Vec<SketchId> = live_sketches(&doc)
        .into_iter()
        .filter(|id| !sketches_before.contains(id))
        .filter(|&id| doc.sketch_owner_object(id).is_none())
        .collect();
    assert_eq!(free.len(), 1, "the cut-away edge lands in one free sketch");
    assert!(same_points(&verts(&doc, free[0]), &[f.0, f.1]));
    let after = doc.save();

    doc.undo().expect("undo subtract");
    assert!(
        doc.sketch(free[0]).is_none(),
        "undo removes the free sketch"
    );
    assert!(doc.sketch(s).is_some());
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo subtract");
    assert!(doc.sketch(free[0]).is_some());
    assert_eq!(doc.save(), after);
}

/// A drawn edge lying exactly along an edge the subtract leaves on the
/// result's top stays drawn on the result, whichever side the cut comes
/// from, and rides when that top is pushed. Its midpoint is on the face's
/// boundary, which the inside test alone answers by the edge's direction:
/// on two of these four sides the subtract used to free the edge into a
/// loose sketch.
#[test]
fn an_edge_along_a_new_boundary_stays_with_the_result_from_any_side() {
    let cases = [
        (
            "front",
            (p(1.0, 1.0, 1.0), p(2.0, 1.0, 1.0)),
            (0.5, 1.0, 3.5, 5.0),
        ),
        (
            "back",
            (p(1.0, 3.0, 1.0), p(2.0, 3.0, 1.0)),
            (0.5, -1.0, 3.5, 3.0),
        ),
        (
            "left",
            (p(1.0, 1.0, 1.0), p(1.0, 2.0, 1.0)),
            (1.0, 0.5, 5.0, 3.5),
        ),
        (
            "right",
            (p(3.0, 1.0, 1.0), p(3.0, 2.0, 1.0)),
            (-1.0, 0.5, 3.0, 3.5),
        ),
    ];
    for (side, seg, (x0, y0, x1, y1)) in cases {
        let mut doc = Document::new();
        let a = make_box(&mut doc, 0.0, 0.0, 4.0, 4.0, 0.0, 1.0);
        let cutter = make_box(&mut doc, x0, y0, x1, y1, 0.5, 2.0);
        let s = owned_sketch(&mut doc, a, 1.0, &[seg]);
        let sketches_before = live_sketches(&doc);

        let (r, _) = doc
            .boolean(BooleanOp::Subtract, a, cutter)
            .expect("subtract");
        assert!(
            same_points(&owned_verts(&doc, r), &[seg.0, seg.1]),
            "{side}: the edge stays drawn on the result"
        );
        assert!(
            live_sketches(&doc)
                .into_iter()
                .filter(|id| !sketches_before.contains(id))
                .all(|id| doc.sketch_owner_object(id).is_some()),
            "{side}: nothing is freed"
        );

        doc.undo().expect("undo subtract");
        assert_eq!(doc.sketch_owner_object(s), Some(a), "{side}");
        doc.redo().expect("redo subtract");
        assert!(
            same_points(&owned_verts(&doc, r), &[seg.0, seg.1]),
            "{side}"
        );

        let top = face_at(&doc, r, Vec3::new(0.0, 0.0, 1.0), p(0.25, 0.25, 1.0));
        doc.apply_object_op(
            r,
            KernelOp::PushPull {
                face: top,
                distance: 0.5,
            },
        )
        .expect("push the top");
        let up = Vec3::new(0.0, 0.0, 0.5);
        assert!(
            same_points(&owned_verts(&doc, r), &[seg.0 + up, seg.1 + up]),
            "{side}: the edge rides with the pushed top"
        );
    }
}

#[test]
fn slicing_gives_each_half_its_part_of_the_drawing() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let before = doc.save();

    let plane = Plane::from_point_normal(p(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let ((pos, neg), _) = doc.slice_node(a, &plane).expect("slice");
    let (n, f) = (near(), far());
    assert!(same_points(&owned_verts(&doc, pos), &[f.0, f.1]));
    assert!(same_points(&owned_verts(&doc, neg), &[n.0, n.1]));
    assert!(doc.sketch(s).is_none());
    let after = doc.save();

    doc.undo().expect("undo slice");
    assert!(doc.sketch(s).is_some());
    assert!(owned_verts(&doc, pos).is_empty());
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo slice");
    assert_eq!(doc.save(), after);
}

#[test]
fn push_pull_through_results_own_their_parts() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    // A strip across the whole top, pushed through the box: it severs the
    // solid into two pieces, each carrying the drawing that lay over it.
    let strip = imprint_strip_on_top(&mut doc, a);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let before = doc.save();

    let (results, _) = doc.push_pull_through(a, strip, -1.0).expect("push through");
    assert_eq!(results.len(), 2, "the strip severs the box");
    let (n, f) = (near(), far());
    let mut all: Vec<Point3> = Vec::new();
    for &r in &results {
        let v = owned_verts(&doc, r);
        assert!(
            same_points(&v, &[n.0, n.1]) || same_points(&v, &[f.0, f.1]),
            "each piece owns exactly the edge over it: {v:?}"
        );
        all.extend(v);
    }
    assert_eq!(all.len(), 4);
    assert!(doc.sketch(s).is_none());
    let after = doc.save();

    doc.undo().expect("undo push-through");
    assert!(doc.sketch(s).is_some());
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo push-through");
    assert_eq!(doc.save(), after);
}

/// Imprints a full-height strip `x in [0.45, 0.55]` that touches the top's
/// boundary at both ends (so it lands as a face split, not an island).
fn imprint_strip_on_top(doc: &mut Document, b: ObjectId) -> FaceId {
    let top = top_face(doc, b);
    draw_path(
        doc,
        None,
        b,
        top,
        &[p(0.45, 0.0, 1.0), p(0.45, 1.0, 1.0)],
        &[],
    )
    .expect("first cut");
    let top = face_at(doc, b, Vec3::new(0.0, 0.0, 1.0), p(0.7, 0.5, 1.0));
    draw_path(
        doc,
        None,
        b,
        top,
        &[p(0.55, 0.0, 1.0), p(0.55, 1.0, 1.0)],
        &[],
    )
    .expect("second cut");
    face_at(doc, b, Vec3::new(0.0, 0.0, 1.0), p(0.5, 0.5, 1.0))
}

// ------------------------------------------------------------- duplicates

#[test]
fn a_duplicate_owns_a_placed_copy_of_the_drawing() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let original = verts(&doc, s);
    let before = doc.save();

    let place = Transform::translation(Vec3::new(5.0, 2.0, 0.0));
    let (node, _) = doc.duplicate_node(NodeId::Object(a), &place).expect("copy");
    let NodeId::Object(copy) = node else {
        panic!("an object duplicates to an object");
    };
    let copies = doc.object_owned_sketches(copy);
    assert_eq!(copies.len(), 1);
    assert_moved(&doc, copies[0], &original, &place);
    assert!(
        same_points(&verts(&doc, s), &original),
        "the source drawing stays"
    );
    assert_eq!(doc.sketch_owner_object(s), Some(a));
    let after = doc.save();

    doc.undo().expect("undo duplicate");
    assert!(
        doc.sketch(copies[0]).is_none(),
        "the copy dies with its owner"
    );
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo duplicate");
    assert!(doc.sketch(copies[0]).is_some());
    assert_eq!(doc.save(), after);
}

// ------------------------------------------------------- explode sessions

/// A component whose single member owns a definition sketch.
fn component_with_owned_sketch(
    doc: &mut Document,
) -> (kernel::ComponentId, kernel::InstanceId, ObjectId, SketchId) {
    let b = unit_box(doc);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(b)]).expect("component");
    let (s, _) = doc
        .begin_sketch_on_plane_in_instance(inst, plane_z(1.0))
        .expect("def sketch");
    draw(doc, s, &[near(), far()]);
    let member = doc.def_members(comp).expect("live component")[0];
    doc.set_sketch_object_owner(s, Some(member)).expect("own");
    (comp, inst, member, s)
}

#[test]
fn closing_an_explode_session_folds_the_moved_owned_sketch_back_in() {
    let mut doc = Document::new();
    let (comp, inst, member, s) = component_with_owned_sketch(&mut doc);
    let original = verts(&doc, s);
    let before = doc.save();

    doc.open_explode_session(inst).expect("open session");
    let shift = Transform::translation(Vec3::new(0.0, 0.0, 0.25));
    doc.transform_object(member, &shift).expect("move member");
    assert_moved(&doc, s, &original, &shift);
    doc.close_explode_session().expect("close session");

    // Back in the definition, riding with its (moved) owner.
    assert_eq!(doc.sketch_owner_component(s), Some(comp));
    assert_eq!(doc.sketch_owner_object(s), Some(member));
    assert_moved(&doc, s, &original, &shift);
    assert!(
        doc.def_member_sketches(comp)
            .expect("live component")
            .contains(&s)
    );
    let after = doc.save();

    doc.undo().expect("undo close");
    doc.undo().expect("undo move");
    doc.undo().expect("undo open");
    assert!(same_points(&verts(&doc, s), &original));
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo open");
    doc.redo().expect("redo move");
    doc.redo().expect("redo close");
    assert_eq!(doc.save(), after);
    Document::load(&after).expect("the closed document reloads");
}

#[test]
fn a_member_duplicated_in_a_session_folds_its_sketch_in_at_close() {
    let mut doc = Document::new();
    let (comp, inst, member, s) = component_with_owned_sketch(&mut doc);
    let original = verts(&doc, s);

    doc.open_explode_session(inst).expect("open session");
    let place = Transform::translation(Vec3::new(3.0, 0.0, 0.0));
    let (node, _) = doc
        .duplicate_node(NodeId::Object(member), &place)
        .expect("duplicate in session");
    let NodeId::Object(copy) = node else {
        panic!("an object duplicates to an object");
    };
    let copy_sketch = doc.object_owned_sketches(copy)[0];
    doc.close_explode_session().expect("close session");

    let defs = doc.def_member_sketches(comp).expect("live component");
    assert_eq!(defs.len(), 2, "both drawings belong to the definition");
    assert_eq!(doc.sketch_owner_object(copy_sketch), Some(copy));
    assert_eq!(doc.sketch_owner_component(copy_sketch), Some(comp));
    assert_moved(&doc, copy_sketch, &original, &place);
    assert!(same_points(&verts(&doc, s), &original));
    let after = doc.save();
    Document::load(&after).expect("reloads with the folded-in sketch");

    doc.undo().expect("undo close");
    doc.redo().expect("redo close");
    assert_eq!(doc.save(), after);
}

// ----------------------------------------- explode / make unique / library

#[test]
fn explode_instance_owns_each_world_copy_by_the_baked_member() {
    let mut doc = Document::new();
    let (_comp, inst, _member, s) = component_with_owned_sketch(&mut doc);
    // A plain definition sketch too: it must stay a plain world sketch.
    let (plain, _) = doc
        .begin_sketch_on_plane_in_instance(inst, plane_z(1.0))
        .expect("plain def sketch");
    draw(&mut doc, plain, &[(p(0.1, 0.9, 1.0), p(0.3, 0.8, 1.0))]);
    doc.transform_instance(inst, &pose()).expect("pose");
    let (def_owned, def_plain) = (verts(&doc, s), verts(&doc, plain));
    let before = doc.save();

    let (created, change) = doc.explode_instance(inst).expect("explode");
    assert_eq!(created.len(), 1);
    let baked = created[0];
    let owned = doc.object_owned_sketches(baked);
    assert_eq!(owned.len(), 1, "the baked member owns its world copy");
    assert_moved(&doc, owned[0], &def_owned, &pose());
    assert_eq!(doc.sketch_owner_component(owned[0]), None);
    let plain_copy = change
        .sketches_touched
        .iter()
        .copied()
        .find(|&c| c != owned[0])
        .expect("the plain copy");
    assert_eq!(doc.sketch_owner_object(plain_copy), None);
    assert_moved(&doc, plain_copy, &def_plain, &pose());
    // The definition's own sketch is untouched for any sibling instance.
    assert!(same_points(&verts(&doc, s), &def_owned));
    let after = doc.save();
    Document::load(&after).expect("the exploded document reloads");

    doc.undo().expect("undo explode");
    assert!(
        doc.sketch(owned[0]).is_none(),
        "the world copy dies with its owner"
    );
    assert!(doc.sketch(plain_copy).is_none());
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo explode");
    assert_eq!(doc.save(), after);
}

#[test]
fn make_unique_owns_each_private_copy_by_the_private_member() {
    let mut doc = Document::new();
    let (comp, inst, member, s) = component_with_owned_sketch(&mut doc);
    let original = verts(&doc, s);
    let before = doc.save();

    let (new_def, _) = doc.make_unique(inst).expect("make unique");
    let new_member = doc.def_members(new_def).expect("live copy")[0];
    assert_ne!(new_member, member);
    let copies = doc.def_member_sketches(new_def).expect("live copy");
    assert_eq!(copies.len(), 1);
    assert_eq!(doc.sketch_owner_object(copies[0]), Some(new_member));
    assert!(same_points(&verts(&doc, copies[0]), &original));
    // The original keeps its own owner.
    assert_eq!(doc.sketch_owner_object(s), Some(member));
    assert_eq!(doc.sketch_owner_component(s), Some(comp));
    let after = doc.save();
    Document::load(&after).expect("the unique copy reloads");

    doc.undo().expect("undo make unique");
    assert!(
        doc.sketch(copies[0]).is_none(),
        "the private copy is hidden"
    );
    assert!(doc.sketch(s).is_some());
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo make unique");
    assert!(doc.sketch(copies[0]).is_some());
    assert_eq!(doc.save(), after);
}

#[test]
fn library_extract_and_insert_carry_the_owned_sketch_with_its_object() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let original = verts(&doc, s);

    // Extract re-origins the item to its content's bottom centre.
    let item = doc
        .extract_item(&[NodeId::Object(a)], false)
        .expect("extract");
    let item_sketches = item.sketch_ids();
    assert_eq!(item_sketches.len(), 1);
    let item_box = item.visible_object_ids()[0];
    assert_eq!(item.sketch_owner_object(item_sketches[0]), Some(item_box));
    let to_item = Transform::translation(Vec3::new(-0.5, -0.5, 0.0));
    assert_moved(&item, item_sketches[0], &original, &to_item);

    // Insert into a fresh document through a pose.
    let mut dest = Document::new();
    let before = dest.save();
    let place = Transform::translation(Vec3::new(10.0, 0.0, 0.0));
    let (report, change) = dest
        .insert_document(
            &item,
            &InsertOptions {
                pose: place,
                provenance: None,
                origin: None,
            },
        )
        .expect("insert");
    assert_eq!(
        report.world_sketches_skipped, 0,
        "owned sketches are not skipped"
    );
    let NodeId::Object(inserted) = report.roots[0] else {
        panic!("the item root is an object");
    };
    let owned = dest.object_owned_sketches(inserted);
    assert_eq!(owned.len(), 1);
    assert!(change.sketches_touched.contains(&owned[0]));
    assert_moved(&dest, owned[0], &original, &to_item.then(&place));
    let after = dest.save();
    Document::load(&after).expect("the destination reloads");

    dest.undo().expect("undo insert");
    assert!(
        dest.sketch(owned[0]).is_none(),
        "undo hides the inserted drawing"
    );
    assert_eq!(dest.save(), before);
    dest.redo().expect("redo insert");
    assert!(dest.sketch(owned[0]).is_some());
    assert_eq!(dest.save(), after);
}

#[test]
fn a_free_world_sketch_is_still_skipped_by_insert_but_an_owned_one_is_not() {
    let mut item = Document::new();
    let a = unit_box(&mut item);
    owned_sketch(&mut item, a, 1.0, &[near()]);
    let free = item.add_sketch(plane_z(1.0));
    draw(&mut item, free, &[far()]);

    let mut dest = Document::new();
    let (report, _) = dest
        .insert_document(
            &item,
            &InsertOptions {
                pose: Transform::IDENTITY,
                provenance: None,
                origin: None,
            },
        )
        .expect("insert");
    assert_eq!(report.world_sketches_skipped, 1);
    assert_eq!(dest.sketch_ids().len(), 1, "only the owned drawing arrived");
}

#[test]
fn a_component_item_carries_member_owned_sketches_into_the_new_definition() {
    // Instance form: the definition (with its member-owned sketch) copies
    // through `library_copy_def`.
    let mut doc = Document::new();
    let (_comp, inst, _member, s) = component_with_owned_sketch(&mut doc);
    let original = verts(&doc, s);
    let item = doc
        .extract_item(&[NodeId::Instance(inst)], false)
        .expect("extract instance");
    assert_component_item_lands_owned(&item, &original, &Transform::IDENTITY);

    // Wrapped form: a bare world object with an owned sketch becomes a
    // component item whose member owns the (baked) sketch.
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, a, 1.0, &[near(), far()]);
    let original = verts(&doc, s);
    let item = doc
        .extract_item(&[NodeId::Object(a)], true)
        .expect("extract wrapped");
    assert_component_item_lands_owned(&item, &original, &Transform::IDENTITY);
}

/// Inserts `item` (one component, one member owning one definition sketch)
/// and checks the definition's sketch is owned by its new member and, baked
/// back through the instance pose, still draws `original` (modulo the
/// item's re-origin `shift`).
fn assert_component_item_lands_owned(item: &Document, original: &[Point3], _shift: &Transform) {
    let mut dest = Document::new();
    let (report, change) = dest
        .insert_document(
            item,
            &InsertOptions {
                pose: Transform::IDENTITY,
                provenance: None,
                origin: None,
            },
        )
        .expect("insert component item");
    assert_eq!(report.world_sketches_skipped, 0);
    let comp = *dest.component_ids().first().expect("a definition arrived");
    let member = dest.def_members(comp).expect("live def")[0];
    let sketches = dest.def_member_sketches(comp).expect("live def");
    assert_eq!(sketches.len(), 1, "the definition's drawing arrived once");
    assert_eq!(dest.sketch_owner_object(sketches[0]), Some(member));
    assert_eq!(dest.sketch_owner_component(sketches[0]), Some(comp));
    assert!(change.sketches_touched.contains(&sketches[0]));
    // The inserted instance maps the def-local drawing to the same world
    // positions as the original (up to the item's own re-origin).
    let inst = dest.instance_ids()[0];
    let pose = dest.instance_pose(inst).expect("live instance");
    let world: Vec<Point3> = verts(&dest, sketches[0])
        .into_iter()
        .map(|q| pose.apply_point(q))
        .collect();
    let (lo, _) = bounds(original);
    let (lo_w, _) = bounds(&world);
    let (hi_o, hi_w) = (bounds(original).1, bounds(&world).1);
    assert!(((hi_o.x - lo.x) - (hi_w.x - lo_w.x)).abs() < 1e-9);
    assert!(((hi_o.y - lo.y) - (hi_w.y - lo_w.y)).abs() < 1e-9);
    let saved = dest.save();
    Document::load(&saved).expect("the destination reloads");

    dest.undo().expect("undo insert");
    assert!(dest.sketch(sketches[0]).is_none());
    dest.redo().expect("redo insert");
    assert!(dest.sketch(sketches[0]).is_some());
    assert_eq!(dest.save(), saved);
}

fn manifest_of(bytes: &[u8]) -> serde_json::Value {
    use std::io::{Cursor, Read as _};
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut buf = Vec::new();
    zip.by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();
    serde_json::from_slice(&buf).unwrap()
}

/// Rewrites a saved container's manifest through `patch`.
fn patch_manifest(bytes: &[u8], patch: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    use std::io::{Cursor, Read as _, Write as _};
    let mut manifest = manifest_of(bytes);
    patch(&mut manifest);
    let patched = serde_json::to_vec_pretty(&manifest).unwrap();
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    out.start_file("manifest.json", opts).unwrap();
    out.write_all(&patched).unwrap();
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        if entry.name().unwrap() == "manifest.json" {
            continue;
        }
        let name = entry.name().unwrap().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        out.start_file(&name, opts).unwrap();
        out.write_all(&buf).unwrap();
    }
    out.finish().unwrap().into_inner()
}

// ---- review findings ----

/// Make Component folds an object's owned sketch into the new definition
/// with it (identity pose), so the saved file reloads; undo returns both
/// to the world, redo folds both again.
#[test]
fn make_component_folds_the_owned_sketch_with_its_object() {
    let mut doc = Document::new();
    let obj = unit_box(&mut doc);
    let sk = owned_sketch(&mut doc, obj, 1.0, &[near(), far()]);
    let before = doc.save();
    doc.make_component(&[kernel::NodeId::Object(obj)])
        .expect("fold");
    assert!(
        doc.sketch_owner_component(sk).is_some(),
        "the sketch folded in"
    );
    let saved = doc.save();
    let reloaded = kernel::Document::load(&saved).expect("the file reloads");
    assert_eq!(reloaded.save(), saved);
    doc.undo().expect("undo");
    assert_eq!(doc.save(), before);
    assert!(doc.sketch_owner_component(sk).is_none());
    doc.redo().expect("redo");
    assert_eq!(doc.save(), saved);
}

/// A whole-sketch move that takes an owned sketch off every face of its
/// owner leaves it as free drawing, in the same undo step.
#[test]
fn moving_an_owned_sketch_off_its_owner_frees_it() {
    let mut doc = Document::new();
    let obj = unit_box(&mut doc);
    let sk = owned_sketch(&mut doc, obj, 1.0, &[near(), far()]);
    let before = doc.save();
    doc.transform_sketch(
        sk,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 5.0)),
    )
    .expect("move");
    assert_eq!(doc.sketch_owner_object(sk), None, "freed");
    doc.undo().expect("undo");
    assert_eq!(doc.save(), before);
    assert_eq!(doc.sketch_owner_object(sk), Some(obj));
}

// ------------------------------------------- owned drawing stays true to its owner

fn edge_count(doc: &Document, s: SketchId) -> usize {
    doc.sketch(s).expect("sketch is live").edges().len()
}

#[test]
fn an_imprint_along_an_owned_edge_drops_it_in_the_same_step() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[(p(0.2, 0.2, 1.0), p(0.4, 0.2, 1.0))]);
    let before = doc.save();
    let top = top_face(&doc, b);

    draw_loop(
        &mut doc,
        None,
        b,
        top,
        &[
            p(0.2, 0.2, 1.0),
            p(0.4, 0.2, 1.0),
            p(0.4, 0.4, 1.0),
            p(0.2, 0.4, 1.0),
        ],
        None,
    )
    .expect("imprint");
    assert!(
        doc.sketch(s).is_none(),
        "the face holds that line now, and the emptied sketch is gone"
    );
    let after = doc.save();

    doc.undo().expect("undo imprint");
    assert_eq!(
        doc.save(),
        before,
        "one step restores the line and the face"
    );
    assert_eq!(edge_count(&doc, s), 1);
    doc.redo().expect("redo imprint");
    assert_eq!(doc.save(), after);
}

#[test]
fn pushing_a_side_face_onto_an_owned_edge_drops_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(
        &mut doc,
        b,
        1.0,
        &[(p(0.5, 0.0, 1.0), p(0.5, 1.0, 1.0)), near()],
    );
    let before = doc.save();
    let side = face_at(&doc, b, Vec3::new(1.0, 0.0, 0.0), p(1.0, 0.5, 0.5));

    doc.apply_object_op(
        b,
        KernelOp::PushPull {
            face: side,
            distance: -0.5,
        },
    )
    .expect("push the side in");
    assert_eq!(
        edge_count(&doc, s),
        1,
        "the line the top face's edge landed on is dropped; the other stays"
    );
    let after = doc.save();
    doc.undo().expect("undo push");
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo push");
    assert_eq!(doc.save(), after);
}

#[test]
fn moving_an_owned_edge_onto_an_owner_edge_drops_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(
        &mut doc,
        b,
        1.0,
        &[
            (p(0.0, 0.5, 1.0), p(1.0, 0.5, 1.0)),
            (p(0.3, 0.7, 1.0), p(0.6, 0.8, 1.0)),
        ],
    );
    let before = doc.save();

    doc.transform_sketch(s, &Transform::translation(Vec3::new(0.0, -0.5, 0.0)))
        .expect("move the drawing");
    assert_eq!(
        edge_count(&doc, s),
        1,
        "the line on the top's edge is dropped"
    );
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    doc.undo().expect("undo move");
    assert_eq!(doc.save(), before);
}

#[test]
fn a_selection_move_that_lifts_an_owned_sketch_off_its_owner_frees_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(&mut doc, b, 1.0, &[near()]);
    let before = doc.save();

    doc.transform_selection(&[], &[s], &Transform::translation(Vec3::new(0.0, 0.0, 0.5)))
        .expect("lift the drawing");
    assert_eq!(
        doc.sketch_owner_object(s),
        None,
        "no face of the box is under it"
    );
    assert!(doc.sketch(s).is_some(), "it stays as free drawing");
    let after = doc.save();
    doc.undo().expect("undo lift");
    assert_eq!(doc.sketch_owner_object(s), Some(b));
    assert_eq!(doc.save(), before);
    doc.redo().expect("redo lift");
    assert_eq!(doc.save(), after);
}

#[test]
fn moving_island_of_an_owned_sketch_onto_an_owner_edge_drops_it() {
    let mut doc = Document::new();
    let b = unit_box(&mut doc);
    let s = owned_sketch(
        &mut doc,
        b,
        1.0,
        &[
            (p(0.0, 0.5, 1.0), p(1.0, 0.5, 1.0)),
            (p(0.3, 0.7, 1.0), p(0.6, 0.8, 1.0)),
        ],
    );
    let sk = doc.sketch(s).expect("live");
    let long = sk
        .edges()
        .iter()
        .find(|(_, e)| (sk.vertices()[e.from].position.y - 0.5).abs() < 1e-12)
        .map(|(id, _)| id)
        .expect("the long line");
    let island = sk.island_of_edge(long).expect("its island");
    doc.transform_sketch_islands(
        s,
        &[island],
        &Transform::translation(Vec3::new(0.0, -0.5, 0.0)),
    )
    .expect("move the island");
    assert_eq!(
        edge_count(&doc, s),
        1,
        "the line moved onto the top's edge is dropped"
    );
}

#[test]
fn changes_report_the_drawing_their_objects_own() {
    let mut doc = Document::new();
    let a = unit_box(&mut doc);
    let c = make_box(&mut doc, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0);
    let s = owned_sketch(
        &mut doc,
        a,
        1.0,
        &[near(), (p(1.2, 0.5, 1.0), p(1.5, 0.5, 1.0))],
    );

    let (r, mut change) = doc.boolean(BooleanOp::Union, a, c).expect("union");
    doc.complete_change(&mut change);
    let copies = doc.object_owned_sketches(r);
    assert!(!copies.is_empty());
    assert!(change.sketches_touched.contains(&s), "the dead original");
    for k in &copies {
        assert!(change.sketches_touched.contains(k), "each carried copy");
    }

    let undo = doc.undo().expect("undo union");
    assert!(undo.sketches_touched.contains(&s), "the revived original");
    for k in &copies {
        assert!(undo.sketches_touched.contains(k), "each copy that died");
    }
    let redo = doc.redo().expect("redo union");
    assert!(redo.sketches_touched.contains(&s));

    let mut deleted = doc.delete_node(NodeId::Object(r)).expect("delete");
    doc.complete_change(&mut deleted);
    for k in &copies {
        assert!(
            deleted.sketches_touched.contains(k),
            "the deleted owner's drawing"
        );
    }
    let undo = doc.undo().expect("undo delete");
    for k in &copies {
        assert!(
            undo.sketches_touched.contains(k),
            "the restored owner's drawing"
        );
    }
}
