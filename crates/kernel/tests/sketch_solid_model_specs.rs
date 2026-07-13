//! Executable specs for the sketch–solid relationship (Model D,
//! docs/design/sketch-solid-model.md): "consumption is becoming".
//!
//! One spec per catalogued failure mode Z1–Z11 of the retired footprint
//! model, each asserting the NEW, correct behavior: extrusion deletes the
//! region's scaffolding (nothing hidden survives, nothing ever resurrects).
//! The standing-solid gate was dropped as inconsistent with Hew's
//! freely-interpenetrating-solids model, so re-extruding occupied ground is
//! allowed exactly like every other overlap — the former gate scenarios now
//! succeed instead of refusing (see the interpenetration section).

use kernel::{Document, DocumentError, NodeId, Plane, Point3, Transform, Vec3};

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

/// Draw an axis-aligned rectangle into `doc`'s sketch `s` at z = 0.
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

/// The single extrudable region of a sketch (panics unless exactly one).
fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// Draw `rect` on a fresh ground sketch and extrude it up by 1.
fn extrude_ground_rect(
    doc: &mut Document,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> (kernel::SketchId, kernel::ObjectId) {
    let s = doc.add_sketch(ground());
    draw_rect(doc, s, x0, y0, x1, y1);
    let r = only_region(doc, s);
    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude rect");
    (s, id)
}

/// Attempt to extrude `rect` drawn on a fresh ground sketch; the result of
/// the extrude call (the region must exist).
fn try_extrude_ground_rect(
    doc: &mut Document,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> Result<kernel::ObjectId, DocumentError> {
    let s = doc.add_sketch(ground());
    draw_rect(doc, s, x0, y0, x1, y1);
    let regions = doc
        .sketch(s)
        .expect("sketch is live")
        .regions()
        .keys()
        .collect::<Vec<_>>();
    assert_eq!(regions.len(), 1, "the drawn rectangle closes one region");
    doc.extrude_region(s, regions[0], 1.0).map(|(id, _)| id)
}

// ------------- re-extrusion over a standing solid (interpenetration allowed)
// The standing-solid gate was dropped (docs/design/sketch-solid-model.md):
// Hew's solids interpenetrate freely, so re-extruding occupied ground is
// allowed exactly like every other overlap — never refused. These specs pin
// the NEW behavior for the former gate scenarios: each now SUCCEEDS,
// producing an interpenetrating second solid rather than a typed refusal.

/// Z10 — redrawing a standing solid's base outline on a FRESH sketch on the
/// same plane now extrudes, producing a coincident second solid. The former
/// per-sketch launder "hole" is moot: there is no gate to launder past. A
/// partially overlapping base extrudes too.
#[test]
fn z10_redrawn_base_on_a_fresh_sketch_extrudes_a_coincident_solid() {
    let mut doc = Document::new();
    let (_s1, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    // Same base, fresh sketch: a second coincident solid, no refusal.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0)
        .expect("a coincident redraw extrudes — interpenetration is allowed");
    // A partially overlapping base extrudes as well.
    try_extrude_ground_rect(&mut doc, 0.5, 0.5, 1.5, 1.5)
        .expect("a partially overlapping base extrudes");
    assert_eq!(
        doc.visible_object_ids().len(),
        3,
        "three solids now stand on the plane"
    );
}

/// Z10 (persistence half) — reloading introduces no phantom gate: a fresh
/// sketch drawn over a reloaded solid's base extrudes, and the file carries
/// no stored claim data at all (see z11).
#[test]
fn z10_redraw_over_a_reloaded_base_extrudes() {
    let mut doc = Document::new();
    let (_s1, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    let bytes = doc.save();
    let mut doc2 = Document::load(&bytes).expect("reload");

    try_extrude_ground_rect(&mut doc2, 0.0, 0.0, 1.0, 1.0)
        .expect("a reloaded solid does not block its base");
    assert_eq!(doc2.visible_object_ids().len(), 2);
}

/// Z3(ii) — moving a solid claims nothing: neither its vacated birth ground
/// nor its landing refuses a later extrusion. The footprint model stranded a
/// claim here and the gate later derived one; both are gone.
#[test]
fn z3_moving_a_solid_claims_nothing() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    doc.transform_object(solid, &Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .expect("move the solid");

    // The landing extrudes (interpenetrating the moved solid)…
    try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0).expect("landing extrudes");
    // …and so does the vacated birth ground.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("vacated ground extrudes");
}

/// Z9 — a copy claims nothing: extruding beneath a duplicated solid
/// interpenetrates freely, like every other overlap in Hew.
#[test]
fn z9_a_copy_claims_nothing() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    doc.duplicate_node(
        NodeId::Object(solid),
        &Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
    )
    .expect("duplicate the solid");

    try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0)
        .expect("extruding beneath the copy interpenetrates freely");
}

/// Z8 — components claim nothing: an instance standing on the plane does not
/// block extrusion beneath it, through its pose or otherwise.
#[test]
fn z8_instances_claim_nothing() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    let (_def, instance, _) = doc
        .make_component(&[NodeId::Object(solid)])
        .expect("make a component of the solid");

    // Extruding under the identity-posed instance succeeds.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("an instance claims nothing");

    // Moving the instance changes nothing about extrudability.
    doc.transform_instance(instance, &Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .expect("move the instance");
    try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0)
        .expect("extruding under the moved instance succeeds");
}

// ------------------------------------------------ consumption is becoming

/// Z1 — nothing resurrects on delete: extruding a sketch's only content
/// deletes the outline and the emptied sketch with it; deleting the solid
/// later leaves the sketch GONE (deleting a solid deletes a solid) while
/// the ground becomes redrawable and extrudable. Undo is the only way back
/// to the outline.
#[test]
fn z1_deleting_a_solid_resurrects_nothing() {
    let mut doc = Document::new();
    let (s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);
    assert!(
        doc.sketch(s).is_none(),
        "the larval sketch became the solid"
    );

    doc.delete_node(NodeId::Object(solid)).expect("delete");
    assert!(
        doc.sketch(s).is_none(),
        "no outline reappears — nothing hidden existed to resurrect"
    );
    assert!(doc.sketch_ids().is_empty());

    // The ground is free again, via redraw.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("redraw extrudes");

    // Undo is the way back: undo the redraw-extrude and the delete, then
    // the original extrusion — the outline returns only then. (The redraw
    // used unbracketed segments, so it recorded no gesture step.)
    doc.undo().expect("undo redraw extrude");
    doc.undo().expect("undo delete");
    assert!(
        doc.sketch(s).is_none(),
        "solid restored, outline still gone"
    );
    doc.undo().expect("undo the original extrusion");
    assert_eq!(
        doc.sketch(s).expect("restored").edges().len(),
        4,
        "only undoing the extrusion itself brings the outline back"
    );
}

/// Z2 — booleans have no sketch side effects: subtracting a cutter that
/// was born from the same sketch resurrects no outline (the scaffolding
/// was deleted at extrusion, not hidden), and undoing the subtract changes
/// no sketch either.
#[test]
fn z2_booleans_resurrect_no_outlines() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 4.0, 4.0);
    draw_rect(&mut doc, s, 5.0, 0.0, 6.0, 1.0);
    let regions = doc.extrudable_regions(s).expect("live");
    assert_eq!(regions.len(), 2);
    let (a, _) = doc.extrude_region(s, regions[0], 2.0).expect("extrude a");
    let (b, _) = doc.extrude_region(s, regions[1], 1.0).expect("extrude b");
    assert!(doc.sketch(s).is_none(), "both regions consumed the sketch");

    // Move the cutter into the kept solid and subtract.
    doc.transform_object(b, &Transform::translation(Vec3::new(-4.5, 0.0, 0.0)))
        .expect("move cutter");
    doc.boolean(kernel::ops::BooleanOp::Subtract, a, b)
        .expect("subtract");
    assert!(
        doc.sketch_ids().is_empty(),
        "the subtract touches solids, never sketches"
    );

    doc.undo().expect("undo subtract");
    assert!(
        doc.sketch_ids().is_empty(),
        "undoing the subtract resurrects nothing either"
    );
}

/// Z4 — moving a sketch moves only what the user sees: the surviving
/// geometry travels; nothing hidden rides along or frees. (The deletion
/// model this pins is unchanged by dropping the gate; a surviving region
/// carried back over the standing solid now extrudes, interpenetrating it.)
#[test]
fn z4_moving_a_sketch_moves_only_visible_geometry() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 3.0, 0.0, 4.0, 1.0);
    let regions = doc.extrudable_regions(s).expect("live");
    let (_solid, _) = doc.extrude_region(s, regions[0], 1.0).expect("extrude");

    // The sketch holds exactly the surviving rectangle.
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 4);

    // Slide the sketch so the surviving rect lands on free ground: fine.
    doc.transform_sketch(s, &Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .expect("move sketch");
    assert_eq!(
        doc.sketch(s).expect("live").edges().len(),
        4,
        "nothing hidden materialized or vanished with the move"
    );
    assert_eq!(doc.extrudable_regions(s).expect("live").len(), 1);

    // Slide it back so the surviving rect sits over the standing solid
    // (from x∈[6,7] to x∈[0,1] — the solid's base): it extrudes anyway,
    // interpenetrating the solid.
    doc.transform_sketch(s, &Transform::translation(Vec3::new(-6.0, 0.0, 0.0)))
        .expect("move sketch back over the solid");
    let r = doc
        .sketch(s)
        .expect("live")
        .regions()
        .keys()
        .next()
        .expect("region");
    doc.extrude_region(s, r, 1.0)
        .expect("a region over a standing solid extrudes (interpenetration)");
}

/// Z5 — islands contain only real geometry: after extruding the left of
/// two edge-sharing rectangles, the survivor is ONE island of exactly its
/// own edges, and moving it drags no phantom scaffolding along.
#[test]
fn z5_island_moves_drag_no_invisible_geometry() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 1.0, 0.0, 2.0, 1.0); // shares the x=1 wall
    let regions = doc.extrudable_regions(s).expect("live");
    doc.extrude_region(s, regions[0], 1.0)
        .expect("extrude left");

    let island = {
        let sk = doc.sketch(s).expect("live");
        assert_eq!(sk.edges().len(), 4, "the survivor keeps its 4 walls");
        assert_eq!(sk.islands().len(), 1, "one island — no hidden hangers-on");
        let (id, isl) = sk.islands().iter().next().expect("island");
        assert_eq!(isl.edges.len(), 4);
        id
    };

    doc.transform_sketch_island(s, island, &Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .expect("move the surviving shape");
    let sk = doc.sketch(s).expect("live");
    assert_eq!(sk.edges().len(), 4, "exactly the selected shape moved");
    assert!(
        sk.vertices().values().all(|v| v.position.x >= 5.0 - 1e-9),
        "no phantom geometry materialized at the old location"
    );
}

/// Z6 — a region spanning a standing solid's base and free ground now
/// extrudes wholesale, producing one solid that overlaps the standing one.
/// (In the footprint model this area silently disappeared; under the dropped
/// gate it refused until split. Interpenetration makes it simply extrude —
/// every edge visible, nothing hidden, no split required.)
#[test]
fn z6_a_region_spanning_occupied_and_free_ground_extrudes() {
    let mut doc = Document::new();
    let (_s1, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 2.0);

    // A fresh rect spanning the solid's base AND free ground.
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 4.0, 2.0);
    let merged = doc
        .sketch(s)
        .expect("live")
        .regions()
        .keys()
        .next()
        .expect("region");
    doc.extrude_region(s, merged, 1.0)
        .expect("the spanning region extrudes, overlapping the standing solid");
    assert_eq!(doc.visible_object_ids().len(), 2);
}

/// Z7 — a combined solid claims nothing: after an intersect, the overlap
/// strip the result stands on AND both operands' vacated birth strips all
/// extrude freely. There is no inherited bookkeeping to over-claim.
#[test]
fn z7_boolean_results_claim_nothing() {
    let mut doc = Document::new();
    let (_s1, a) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 1.0);
    let (_s2, b) = extrude_ground_rect(&mut doc, 3.0, 0.0, 5.0, 1.0);
    // Overlap them: b moves to x∈[1,3] — the overlap with a is x∈[1,2].
    doc.transform_object(b, &Transform::translation(Vec3::new(-2.0, 0.0, 0.0)))
        .expect("move b");

    doc.boolean(kernel::ops::BooleanOp::Intersect, a, b)
        .expect("intersect");

    // The overlap strip the result stands on extrudes…
    try_extrude_ground_rect(&mut doc, 1.0, 0.0, 2.0, 1.0).expect("overlap strip extrudes");
    // …and so do both operands' vacated strips.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("a's vacated strip extrudes");
    try_extrude_ground_rect(&mut doc, 2.0, 0.0, 3.0, 1.0).expect("b's vacated strip extrudes");
}

/// Z11 — no claim outlives its solid in a file: a v11 save carries no
/// `consumed` or `footprints` data at all (asserted on the manifest JSON),
/// so there is nothing a reload could freeze forever. (Loading older
/// files' stored claims is covered in serialize_specs: they are ignored.)
#[test]
fn z11_saved_files_store_no_claims() {
    let mut doc = Document::new();
    let (_s, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);
    let bytes = doc.save();

    // Pull the manifest out of the container and inspect the raw JSON.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).expect("container");
    let mut manifest = String::new();
    std::io::Read::read_to_string(
        &mut zip.by_name("manifest.json").expect("manifest"),
        &mut manifest,
    )
    .expect("utf-8 manifest");
    assert!(
        !manifest.contains("\"consumed\""),
        "v11 writes no consumed list"
    );
    assert!(
        !manifest.contains("\"footprints\""),
        "v11 writes no footprint polygons"
    );
    assert!(
        !manifest.contains("\"source\""),
        "v11 writes no source provenance"
    );
}

/// Undoing an extrusion restores the deleted scaffolding by RE-INSERTION,
/// merging with whatever the sketch holds now — it must not clobber edits
/// made after the extrusion (a whole-sketch snapshot would). Unbracketed
/// edits (a script, the wasm shim outside a gesture) are exactly the kind
/// that record no undo step of their own.
#[test]
fn undoing_an_extrusion_preserves_interleaved_sketch_edits() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);
    doc.extrude_region(s, r, 1.0).expect("extrude");
    assert!(doc.sketch(s).is_none(), "the emptied sketch is gone");

    // …the extrusion emptied the sketch, so interleave on ANOTHER sketch
    // dimension: undo must restore the outline into a sketch that has
    // since gained unrecorded geometry. Rebuild the scenario with a
    // sketch that stays alive.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 5.0, 0.0, 6.0, 1.0); // keeps the sketch alive
    let regions = doc.extrudable_regions(s).expect("live");
    let left = regions
        .iter()
        .copied()
        .find(|&r| {
            let sk = doc.sketch(s).unwrap();
            sk.regions()[r]
                .outer
                .iter()
                .all(|&v| sk.vertices()[v].position.x <= 1.0 + 1e-9)
        })
        .expect("left region");
    doc.extrude_region(s, left, 1.0).expect("extrude left");
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 4);

    // Unbracketed interleaved edit: a third rectangle, no undo entry.
    draw_rect(&mut doc, s, 3.0, 0.0, 4.0, 1.0);
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 8);

    // Undo the extrusion: the outline returns AND the interleaved
    // rectangle survives.
    doc.undo().expect("undo the extrusion");
    let sk = doc.sketch(s).expect("live");
    assert_eq!(
        sk.edges().len(),
        12,
        "outline restored, interleaved drawing intact"
    );
    assert_eq!(sk.regions().len(), 3, "all three rectangles close");
    assert!(doc.visible_object_ids().is_empty(), "the solid is gone");

    // Redo removes exactly the restored scaffolding again.
    doc.redo().expect("redo the extrusion");
    let sk = doc.sketch(s).expect("live");
    assert_eq!(sk.edges().len(), 8, "outline consumed again");
    assert_eq!(sk.regions().len(), 2);
    assert_eq!(doc.visible_object_ids().len(), 1);
}

/// When geometry drawn after the extrusion crosses where the outline was,
/// undo cannot re-insert it faithfully: it fails typed and leaves the
/// document untouched — never a silent merge, never a clobber. Clearing
/// the conflict lets the same undo succeed.
#[test]
fn undoing_an_extrusion_refuses_typed_on_conflicting_edits() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 5.0, 0.0, 6.0, 1.0); // keeps the sketch alive
    let left = {
        let sk = doc.sketch(s).unwrap();
        sk.regions()
            .iter()
            .find(|(_, r)| {
                r.outer
                    .iter()
                    .all(|&v| sk.vertices()[v].position.x <= 1.0 + 1e-9)
            })
            .map(|(id, _)| id)
            .expect("left region")
    };
    let (obj, _) = doc.extrude_region(s, left, 1.0).expect("extrude left");

    // Unbracketed edit crossing the old outline's left wall at (0, 0.5).
    let crossing = {
        let sk = doc.sketch_mut(s).expect("live");
        sk.add_segment(Point3::new(-0.5, 0.5, 0.0), Point3::new(0.5, 0.5, 0.0))
            .expect("crossing segment")
            .new_edges[0]
    };

    // Undo refuses typed; the document is untouched.
    let err = doc.undo().expect_err("re-insertion conflicts");
    assert!(
        matches!(
            err,
            DocumentError::Sketch(kernel::SketchError::RestoreConflicts)
        ),
        "typed conflict, got {err:?}"
    );
    assert_eq!(
        doc.visible_object_ids(),
        vec![obj],
        "the solid still stands"
    );
    assert_eq!(
        doc.sketch(s).expect("live").edges().len(),
        5,
        "the conflicting edit is untouched"
    );
    assert!(doc.can_undo(), "the extrusion stays on the undo stack");

    // Clear the conflict: the same undo now succeeds.
    doc.sketch_mut(s)
        .expect("live")
        .remove_edge(crossing)
        .expect("erase the conflicting line");
    doc.undo().expect("undo succeeds once the conflict is gone");
    assert!(doc.visible_object_ids().is_empty());
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 8);
}

/// Z7, slice half: slice pieces claim nothing — both halves extrude freely
/// after the cut (there was never a whole-source claim to inherit, and now
/// no per-piece claim either).
#[test]
fn z7_slice_pieces_claim_nothing() {
    let mut doc = Document::new();
    let (_s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 1.0);
    let plane = Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .expect("cut plane");
    let ((_pos, _neg), _) = doc.slice_node(solid, &plane).expect("slice at x = 1");

    // Both the positive (x∈[1,2]) and negative (x∈[0,1]) halves extrude.
    try_extrude_ground_rect(&mut doc, 0.1, 0.1, 0.9, 0.9).expect("negative half extrudes");
    try_extrude_ground_rect(&mut doc, 1.1, 0.1, 1.9, 0.9).expect("positive half extrudes");
}

/// Z7, push-through half: a through-hole result claims nothing — both the
/// opened hole and the surviving ring extrude freely beneath them.
#[test]
fn z7_push_through_results_claim_nothing() {
    let mut doc = Document::new();
    let (_s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 4.0, 4.0);
    let top = doc
        .object(solid)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("top face");
    let sub = match doc
        .apply_object_op(
            solid,
            kernel::KernelOp::SplitFaceInner {
                face: top,
                loop_path: vec![
                    Point3::new(1.0, 1.0, 1.0),
                    Point3::new(3.0, 1.0, 1.0),
                    Point3::new(3.0, 3.0, 1.0),
                    Point3::new(1.0, 3.0, 1.0),
                ],
                restore: None,
                curve: None,
            },
        )
        .expect("imprint")
        .0
    {
        kernel::KernelOpReport::FaceSplitInner(r) => r.sub_face,
        other => panic!("unexpected report {other:?}"),
    };
    let (results, _) = doc
        .push_pull_through(solid, sub, -1.5)
        .expect("cut a through-hole");
    assert_eq!(results.len(), 1, "the ring stays one solid");

    // Both inside the hole and on the ring, the ground extrudes freely.
    try_extrude_ground_rect(&mut doc, 1.2, 1.2, 2.8, 2.8).expect("the hole ground extrudes");
    try_extrude_ground_rect(&mut doc, 0.1, 0.1, 0.9, 0.9).expect("the ring ground extrudes");
}
