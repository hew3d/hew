//! Executable specs pinning DEVELOPMENT.md rule 9's "undo restores the
//! accepted state exactly" for the document-level TRANSFORM family
//! (`transform_object`, `transform_sketch`, `transform_group`,
//! `transform_selection`, `transform_def_member`).
//!
//! Before this suite these producers recorded a recomputed `inverse` matrix
//! and baked THAT into geometry for undo — the classic float trap
//! (`(p+d)-d != p` in general; `Transform::inverse`'s general
//! adjugate/determinant computation is not even an irrational-rotation's own
//! exact orthogonal inverse). That left every one of them drifting by ~1 ULP
//! per undo, confirmed directly against bare `kernel::Document` (see the now-
//! removed carve-outs in `crates/api/tests/conformance.rs`). The fix mirrors
//! `DocAction::Rescale`'s existing posture (see
//! `rescale_document_undo_redo_is_bit_exact_never_drifts` in
//! `document_specs.rs`): record the exact PRE-transform snapshot and restore
//! it verbatim; redo re-applies the forward transform to that same snapshot,
//! which reproduces the original commit bit-for-bit by construction
//! (deterministic float ops on identical operands).
//!
//! `transform_instance` is deliberately absent here — it already stored the
//! exact prior pose (never a recomputed inverse) before this change, so it
//! never drifted; see `transform_instance_composes_pose_and_undo_is_exact`
//! in `document_specs.rs`.

use kernel::{Document, NodeId, Plane, Point3, Transform, Vec3};
use std::f64::consts::FRAC_PI_2;

// ----------------------------------------------------------------- helpers

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// An axis-aligned box swept from `z` up by `h`. Coordinates are always
/// offset by the deliberately NON-ROUND `NX0`/`NY0`/`NX1`/`NY1` below — the
/// generic-translation gap this suite pins closed needs arithmetic that does
/// not happen to cancel exactly; round corners would hide it (mirrors
/// `crates/api/tests/conformance.rs`'s now-removed carve-out (d) fixture).
fn extrude_box(
    doc: &mut Document,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    z: f64,
    h: f64,
) -> kernel::ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z),
        Point3::new(1.0, 0.0, z),
        Point3::new(0.0, 1.0, z),
    ])
    .expect("offset plane is well-defined");
    let s = doc.add_sketch(plane);
    let corners = [
        (Point3::new(x0, y0, z), Point3::new(x1, y0, z)),
        (Point3::new(x1, y0, z), Point3::new(x1, y1, z)),
        (Point3::new(x1, y1, z), Point3::new(x0, y1, z)),
        (Point3::new(x0, y1, z), Point3::new(x0, y0, z)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
    let r = only_region(doc, s);
    doc.extrude_region(s, r, h).expect("extrude box").0
}

/// Draw an axis-aligned rectangle into `doc`'s sketch `s`.
fn draw_rect(doc: &mut Document, s: kernel::SketchId, x0: f64, y0: f64, x1: f64, y1: f64) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
}

const NX0: f64 = 5.0;
const NY0: f64 = 0.101_150_854_603_034_57;
const NX1: f64 = 5.4;
const NY1: f64 = 0.501_150_854_603_034_6;

/// A rotation by `angle` about `axis` through an off-center `pivot` —
/// translate to the pivot, rotate, translate back (the same
/// translate/transform/translate-back shape `rescale_transform` uses for an
/// anchored scale in `document.rs`).
fn rotation_about(axis: Vec3, angle: f64, pivot: Point3) -> Transform {
    Transform::translation(Vec3::new(-pivot.x, -pivot.y, -pivot.z))
        .then(&Transform::rotation(axis, angle).expect("axis is non-zero"))
        .then(&Transform::translation(Vec3::new(
            pivot.x, pivot.y, pivot.z,
        )))
}

/// Runs `cycles` full undo -> redo round trips over the most recently
/// committed action, asserting `doc.save()` reproduces `before`/`after`
/// byte-for-byte every time. Generalizes
/// `rescale_document_undo_redo_is_bit_exact_never_drifts`'s shape (see
/// `document_specs.rs`) to every call site in this file.
fn assert_undo_redo_bit_exact(
    doc: &mut Document,
    before: &[u8],
    after: &[u8],
    cycles: usize,
    what: &str,
) {
    for cycle in 0..cycles {
        doc.undo()
            .unwrap_or_else(|e| panic!("{what}: cycle {cycle}: undo failed: {e:?}"));
        assert_eq!(
            doc.save(),
            before,
            "{what}: cycle {cycle}: undo did not restore byte-identical bytes"
        );
        doc.redo()
            .unwrap_or_else(|e| panic!("{what}: cycle {cycle}: redo failed: {e:?}"));
        assert_eq!(
            doc.save(),
            after,
            "{what}: cycle {cycle}: redo did not reproduce byte-identical bytes"
        );
    }
}

// ------------------------------------------------------- transform_object

/// `transform_object` undo/redo never drifts, across a generic translation,
/// a rotation by `FRAC_PI_2` about an off-center pivot, and a non-uniform
/// scale.
#[test]
fn transform_object_undo_redo_is_bit_exact_never_drifts() {
    // Generic translation.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        doc.transform_object(
            id,
            &Transform::translation(Vec3::new(0.1, 0.0, -0.454_510_442_911_682_36)),
        )
        .expect("translate");
        let after = doc.save();
        assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_object translation");
    }
    // Rotation by FRAC_PI_2 about an off-center pivot.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        let rot = rotation_about(
            Vec3::new(0.0, 0.0, 1.0),
            FRAC_PI_2,
            Point3::new(1.7, -0.6, 0.3),
        );
        doc.transform_object(id, &rot)
            .expect("rotate about an off-center pivot");
        let after = doc.save();
        assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_object rotation");
    }
    // Non-uniform scale.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        doc.transform_object(id, &Transform::scale(Vec3::new(1.3, 0.7, 2.1)))
            .expect("non-uniform scale");
        let after = doc.save();
        assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_object scale");
    }
}

// ------------------------------------------------------- transform_sketch

/// `transform_sketch` undo/redo never drifts.
#[test]
fn transform_sketch_undo_redo_is_bit_exact_never_drifts() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, NX0, NY0, NX1, NY1);
    let before = doc.save();
    let rot = rotation_about(
        Vec3::new(0.0, 0.0, 1.0),
        FRAC_PI_2,
        Point3::new(-2.3, 4.1, 0.0),
    );
    doc.transform_sketch(s, &rot)
        .expect("rotate the sketch about an off-center pivot");
    let after = doc.save();
    assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_sketch rotation");
}

// -------------------------------------------------------- transform_group

/// `transform_group` undo/redo never drifts across its leaf objects.
#[test]
fn transform_group_undo_redo_is_bit_exact_never_drifts() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
    let b = extrude_box(&mut doc, NX0 + 2.0, NY0, NX1 + 2.0, NY1, 0.0, 0.1);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let before = doc.save();
    doc.transform_group(g, &Transform::scale(Vec3::new(1.3, 0.7, 2.1)))
        .expect("non-uniform scale the group");
    let after = doc.save();
    assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_group scale");
}

// -------------------------------------------------- transform_def_member

/// `transform_def_member` (component-edit-parity.md phase K2) shares
/// `DocAction::Transform` with `transform_object`/`transform_group`, so it
/// takes the same fix; pinned directly rather than assumed.
#[test]
fn transform_def_member_undo_redo_is_bit_exact_never_drifts() {
    let mut doc = Document::new();
    let gs = doc.add_sketch(ground());
    draw_rect(&mut doc, gs, NX0, NY0, NX1, NY1);
    let r = only_region(&doc, gs);
    let (id, _) = doc.extrude_region(gs, r, 0.1).expect("extrude box");
    let (comp, inst, _) = doc
        .make_component(&[NodeId::Object(id)])
        .expect("make_component");
    let member = doc.def_members(comp).expect("live component")[0];

    let before = doc.save();
    let rot = rotation_about(
        Vec3::new(0.0, 0.0, 1.0),
        FRAC_PI_2,
        Point3::new(0.9, -1.4, 0.0),
    );
    doc.transform_def_member(inst, member, &rot)
        .expect("bake a world gesture into the def member");
    let after = doc.save();
    assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_def_member");
}

// ----------------------------------------------------- transform_selection

/// `transform_selection` undo/redo never drifts, across a generic
/// translation, a rotation by `FRAC_PI_2` about an off-center pivot, a
/// non-uniform scale, and a genuinely MIXED selection (a bare object, a
/// group, a component instance, and a free-standing sketch, all baked/
/// composed by one call and cycled as one undo step).
#[test]
fn transform_selection_undo_redo_is_bit_exact_never_drifts() {
    // Generic translation.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        doc.transform_selection(
            &[NodeId::Object(id)],
            &[],
            &Transform::translation(Vec3::new(0.1, 0.0, -0.454_510_442_911_682_36)),
        )
        .expect("translate the selection");
        let after = doc.save();
        assert_undo_redo_bit_exact(
            &mut doc,
            &before,
            &after,
            3,
            "transform_selection translation",
        );
    }
    // Rotation by FRAC_PI_2 about an off-center pivot.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        let rot = rotation_about(
            Vec3::new(0.0, 0.0, 1.0),
            FRAC_PI_2,
            Point3::new(1.7, -0.6, 0.3),
        );
        doc.transform_selection(&[NodeId::Object(id)], &[], &rot)
            .expect("rotate the selection about an off-center pivot");
        let after = doc.save();
        assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_selection rotation");
    }
    // Non-uniform scale.
    {
        let mut doc = Document::new();
        let id = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let before = doc.save();
        doc.transform_selection(
            &[NodeId::Object(id)],
            &[],
            &Transform::scale(Vec3::new(1.3, 0.7, 2.1)),
        )
        .expect("non-uniform scale the selection");
        let after = doc.save();
        assert_undo_redo_bit_exact(&mut doc, &before, &after, 3, "transform_selection scale");
    }
    // A genuinely mixed selection: a bare object, a group, a component
    // instance, and a free-standing sketch.
    {
        let mut doc = Document::new();
        let a = extrude_box(&mut doc, NX0, NY0, NX1, NY1, 0.0, 0.1);
        let b = extrude_box(&mut doc, NX0 + 2.0, NY0, NX1 + 2.0, NY1, 0.0, 0.1);
        let c = extrude_box(&mut doc, NX0 + 4.0, NY0, NX1 + 4.0, NY1, 0.0, 0.1);
        let d = extrude_box(&mut doc, NX0 + 6.0, NY0, NX1 + 6.0, NY1, 0.0, 0.1);
        let (g, _) = doc
            .group_nodes(&[NodeId::Object(b), NodeId::Object(c)])
            .unwrap();
        let (_comp, inst, _) = doc.make_component(&[NodeId::Object(d)]).unwrap();
        let s = doc.add_sketch(ground());
        draw_rect(&mut doc, s, NX0 + 8.0, NY0, NX1 + 8.0, NY1);

        let before = doc.save();
        let t = rotation_about(
            Vec3::new(0.0, 0.0, 1.0),
            0.213_478_2,
            Point3::new(-0.8, 2.6, 0.0),
        )
        .then(&Transform::translation(Vec3::new(
            0.372_910_4,
            -0.194_003_1,
            0.083_5,
        )));
        doc.transform_selection(
            &[NodeId::Object(a), NodeId::Group(g), NodeId::Instance(inst)],
            &[s],
            &t,
        )
        .expect("transform the mixed selection");
        let after = doc.save();
        assert_undo_redo_bit_exact(
            &mut doc,
            &before,
            &after,
            3,
            "transform_selection mixed selection",
        );
    }
}

// ----------------------------------------------- transform_sketch_island

/// The island of `s` whose lowest edge's `from` vertex has x-coordinate
/// below `x_threshold` — the "left" shape of a two-island fixture built by
/// two calls to `draw_rect` at increasing x.
fn island_left_of(doc: &Document, s: kernel::SketchId, x_threshold: f64) -> kernel::SketchIslandId {
    let sk = doc.sketch(s).expect("live");
    sk.islands()
        .iter()
        .find(|(_, isl)| sk.vertices()[sk.edges()[isl.edges[0]].from].position.x < x_threshold)
        .map(|(id, _)| id)
        .expect("an island left of the threshold")
}

/// `transform_sketch_island` undo/redo never drifts, across a generic
/// translation and an irrational in-plane rotation about an off-center
/// pivot, on a sketch holding TWO separate islands (only one is touched).
///
/// Before the fix, `DocAction::TransformSketchIsland` recorded a recomputed
/// `inverse` matrix and baked it into the CURRENT island on undo — the same
/// float trap the rest of the transform family had (`(p+d)-d != p` in
/// general). The fix mirrors `DocAction::TransformSketch`: record the whole
/// sketch's exact pre-transform snapshot and restore it verbatim.
#[test]
fn transform_sketch_island_undo_redo_is_bit_exact_never_drifts() {
    // Generic translation.
    {
        let mut doc = Document::new();
        let s = doc.add_sketch(ground());
        draw_rect(&mut doc, s, NX0, NY0, NX1, NY1);
        draw_rect(&mut doc, s, NX0 + 3.0, NY0, NX1 + 3.0, NY1);
        let left = island_left_of(&doc, s, NX0 + 2.0);

        let before = doc.save();
        doc.transform_sketch_island(
            s,
            left,
            &Transform::translation(Vec3::new(0.1, 0.0, -0.454_510_442_911_682_36)),
        )
        .expect("translate the island");
        let after = doc.save();
        assert_undo_redo_bit_exact(
            &mut doc,
            &before,
            &after,
            3,
            "transform_sketch_island translation",
        );
    }
    // Irrational in-plane rotation about an off-center pivot.
    {
        let mut doc = Document::new();
        let s = doc.add_sketch(ground());
        draw_rect(&mut doc, s, NX0, NY0, NX1, NY1);
        draw_rect(&mut doc, s, NX0 + 3.0, NY0, NX1 + 3.0, NY1);
        let left = island_left_of(&doc, s, NX0 + 2.0);

        let before = doc.save();
        let rot = rotation_about(
            Vec3::new(0.0, 0.0, 1.0),
            0.373_291_2,
            Point3::new(NX0 - 0.4, NY0 + 0.2, 0.0),
        );
        doc.transform_sketch_island(s, left, &rot)
            .expect("rotate the island in-plane about an off-center pivot");
        let after = doc.save();
        assert_undo_redo_bit_exact(
            &mut doc,
            &before,
            &after,
            3,
            "transform_sketch_island rotation",
        );
    }
}

/// The reason per-island transforms exist: transforming ONE island leaves a
/// sibling island free to keep changing afterward, and undoing the FIRST
/// island's transform must not clobber the second island's later edit.
/// Whole-sketch-snapshot restoration makes this easy to get wrong (a naive
/// "undo restores my snapshot" could stomp a sibling's later state) — this
/// pins that undoing island A's transform, after island B was ALSO
/// transformed, leaves B's edit intact and reverses only A's.
#[test]
fn transform_sketch_island_undo_leaves_other_islands_later_edits_intact() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, NX0, NY0, NX1, NY1);
    draw_rect(&mut doc, s, NX0 + 3.0, NY0, NX1 + 3.0, NY1);
    let left = island_left_of(&doc, s, NX0 + 2.0);
    let before = doc.save();

    // Transform the LEFT island only.
    doc.transform_sketch_island(s, left, &Transform::translation(Vec3::new(0.0, 5.0, 0.0)))
        .expect("move the left island");
    let after_left = doc.save();

    // A LATER, independent transform of the RIGHT island.
    let right = doc
        .sketch(s)
        .expect("live")
        .islands()
        .keys()
        .find(|&id| id != left)
        .expect("a second island survives");
    doc.transform_sketch_island(s, right, &Transform::translation(Vec3::new(0.0, -3.0, 0.0)))
        .expect("move the right island");
    let after_both = doc.save();

    // Undo the RIGHT island's transform, then the LEFT island's: each step
    // must land on the EXACT state recorded at that point in forward
    // history — undoing the left transform's whole-sketch snapshot must
    // not disturb the right transform's own undo, which already ran first
    // (undo is strictly LIFO).
    doc.undo().expect("undo the right island transform");
    assert_eq!(
        doc.save(),
        after_left,
        "undoing the right transform lands exactly on the post-left-transform state"
    );
    doc.undo().expect("undo the left island transform");
    assert_eq!(
        doc.save(),
        before,
        "undoing the left transform lands exactly on the original, pre-transform state"
    );

    // Redo both: the right island's transform was committed against the
    // ALREADY-baked left transform, so redoing both must reproduce that
    // exact combination — proving the left transform's redo (re-applying
    // `forward` to `prior`) didn't disturb what the right transform built
    // on top of it.
    doc.redo().expect("redo the left island transform");
    assert_eq!(
        doc.save(),
        after_left,
        "redoing the left transform reproduces the post-left-transform state exactly"
    );
    doc.redo().expect("redo the right island transform");
    assert_eq!(
        doc.save(),
        after_both,
        "redoing the right transform reproduces the final state exactly"
    );
}

// ------------------------------------------------- transform_def_sketch_island

/// A minimal boxed component: one extruded object, made into a component
/// with one instance. Mirrors `component_edit_k2_specs.rs`'s
/// `boxed_component` (integration test binaries share no code).
fn boxed_component(doc: &mut Document) -> kernel::InstanceId {
    let gs = doc.add_sketch(ground());
    draw_rect(doc, gs, 0.0, 0.0, 4.0, 2.0);
    let r = only_region(doc, gs);
    let (id, _) = doc.extrude_region(gs, r, 1.0).expect("extrude box");
    let (_comp, inst, _) = doc
        .make_component(&[NodeId::Object(id)])
        .expect("make_component");
    inst
}

/// `transform_def_sketch_island` (the definition-owned analogue, reached
/// through an instance's world-gesture conjugation) shares
/// `DocAction::TransformSketchIsland` with the free-standing
/// `transform_sketch_island`, so it takes the same fix; pinned directly
/// rather than assumed.
#[test]
fn transform_def_sketch_island_undo_redo_is_bit_exact_never_drifts() {
    let mut doc = Document::new();
    let instance = boxed_component(&mut doc);
    let (s, _) = doc
        .begin_sketch_on_plane_in_instance(instance, ground())
        .expect("definition sketch");
    draw_rect(&mut doc, s, NX0, NY0, NX1, NY1);
    draw_rect(&mut doc, s, NX0 + 3.0, NY0, NX1 + 3.0, NY1);
    let left = island_left_of(&doc, s, NX0 + 2.0);

    let before = doc.save();
    let rot = rotation_about(
        Vec3::new(0.0, 0.0, 1.0),
        0.373_291_2,
        Point3::new(NX0 - 0.4, NY0 + 0.2, 0.0),
    );
    doc.transform_def_sketch_island(instance, s, left, &rot)
        .expect("rotate the definition-owned island in-plane");
    let after = doc.save();
    assert_undo_redo_bit_exact(
        &mut doc,
        &before,
        &after,
        3,
        "transform_def_sketch_island rotation",
    );
}
