//! Executable specs for the sketch–solid relationship (Model D,
//! docs/design/sketch-solid-model.md): "consumption is becoming".
//!
//! One spec per catalogued failure mode Z1–Z11 of the retired footprint
//! model, each asserting the NEW, correct behavior: extrusion deletes the
//! region's scaffolding (nothing hidden survives, nothing ever resurrects),
//! and re-extrusion is refused by a gate derived live from visible solids'
//! coplanar face contact — global across sketches, kinematic by
//! construction (the claim is the solid's own face).

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

fn assert_blocked(result: Result<kernel::ObjectId, DocumentError>, by: NodeId) {
    match result {
        Err(DocumentError::RegionBlocked { by: b }) => {
            assert_eq!(b, by, "the refusal names the standing solid")
        }
        other => panic!("expected RegionBlocked, got {other:?}"),
    }
}

// ------------------------------------------------- the standing-solid gate

/// Z10 — the gate is global across sketches: redrawing a standing solid's
/// base outline on a FRESH sketch on the same plane refuses to extrude
/// while the solid stands, and frees the moment the solid dies. The
/// per-sketch launder hole (fresh sketch after save/load) is closed.
#[test]
fn z10_redrawn_base_on_a_fresh_sketch_refuses_while_the_solid_stands() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    // Same base, fresh sketch: refused, naming the standing solid.
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0),
        NodeId::Object(solid),
    );
    // A partially overlapping base refuses too (area overlap, not identity).
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.5, 0.5, 1.5, 1.5),
        NodeId::Object(solid),
    );

    // Delete the solid: the claim dies with it; the same redraw extrudes.
    doc.delete_node(NodeId::Object(solid)).expect("delete");
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("ground is free again");
}

/// Z10 (persistence half) — the gate holds across save/load: a fresh
/// sketch drawn after reload still refuses over the standing solid's base,
/// with no stored claim data in the file at all.
#[test]
fn z10_gate_survives_save_load_with_no_stored_claims() {
    let mut doc = Document::new();
    let (_s1, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    let bytes = doc.save();
    let mut doc2 = Document::load(&bytes).expect("reload");

    let result = try_extrude_ground_rect(&mut doc2, 0.0, 0.0, 1.0, 1.0);
    assert!(
        matches!(result, Err(DocumentError::RegionBlocked { .. })),
        "reloaded solid still blocks its base: {result:?}"
    );
}

/// Z3(ii) — the claim is kinematic: moving a solid claims its landing and
/// frees its birth ground, because the claim IS the solid's own base face.
#[test]
fn z3_moving_a_solid_moves_its_claim() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    doc.transform_object(solid, &Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .expect("move the solid");

    // The landing is claimed…
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0),
        NodeId::Object(solid),
    );
    // …and the birth ground is free.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("vacated ground extrudes");
}

/// Z9 — copies claim by standing, like everything else: a duplicated solid
/// blocks extrusion beneath its landing, and deleting the copy frees it.
#[test]
fn z9_a_copy_claims_where_it_stands() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    let (copy, _) = doc
        .duplicate_node(
            NodeId::Object(solid),
            &Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
        )
        .expect("duplicate the solid");

    assert_blocked(try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0), copy);

    doc.delete_node(copy).expect("delete the copy");
    try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0).expect("freed by deleting the copy");
}

/// Z8 — components claim by standing: an instance blocks through its pose,
/// the claim follows the pose, and a definition with no visible instance
/// claims nothing (deleting the last instance frees the ground even though
/// the definition member object still exists un-hidden).
#[test]
fn z8_instances_claim_through_their_pose_and_die_with_the_last_instance() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    let (_def, instance, _) = doc
        .make_component(&[NodeId::Object(solid)])
        .expect("make a component of the solid");

    // The identity-posed instance claims the birth area.
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0),
        NodeId::Instance(instance),
    );

    // Moving the instance moves the claim (pose, not bake).
    doc.transform_instance(instance, &Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .expect("move the instance");
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0),
        NodeId::Instance(instance),
    );
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("vacated birth ground extrudes");

    // Deleting the last visible instance frees the ground — the definition
    // member survives as library geometry but claims nothing.
    doc.delete_node(NodeId::Instance(instance))
        .expect("delete the instance");
    try_extrude_ground_rect(&mut doc, 3.0, 0.0, 4.0, 1.0).expect("no visible instance, no claim");
}

/// Hidden solids claim nothing: user-hiding a solid (directly or through an
/// ancestor group) lifts its claim, and unhiding restores it. What you see
/// is what blocks.
#[test]
fn hidden_solids_claim_nothing() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    doc.set_node_user_hidden(NodeId::Object(solid), true);
    let interloper =
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("hidden solid claims nothing");
    // Remove the solid that just proved the point, then unhide.
    doc.delete_node(NodeId::Object(interloper))
        .expect("remove the probe solid");

    doc.set_node_user_hidden(NodeId::Object(solid), false);
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0),
        NodeId::Object(solid),
    );

    // Hiding an ancestor group hides — and un-claims — the whole subtree.
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(solid)])
        .expect("group the solid");
    doc.set_node_user_hidden(NodeId::Group(group), true);
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0)
        .expect("a member of a hidden group claims nothing");
}

/// Boundary grazing is not overlap: a region sharing only an edge with a
/// standing solid's base extrudes freely (adjacent construction stays
/// possible), and a region inside a solid's base HOLE is likewise free.
#[test]
fn adjacency_is_not_blocked() {
    let mut doc = Document::new();
    let (_s1, _solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    // Shares the whole x = 1 edge with the standing solid's base.
    try_extrude_ground_rect(&mut doc, 1.0, 0.0, 2.0, 1.0).expect("adjacent region extrudes");
}

/// The gate reads every coplanar face of a visible solid, not just its
/// birth base: a free-standing sketch coinciding with a solid's TOP face is
/// blocked there too — extruding it would stack a coincident second solid,
/// exactly what the gate exists to refuse.
#[test]
fn a_solids_top_face_blocks_a_coincident_sketch_plane() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);

    // A sketch on the z = 1 plane, over the solid's top face.
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .expect("top plane");
    let s = doc.add_sketch(top);
    {
        let sk = doc.sketch_mut(s).expect("live");
        let corners = [
            (Point3::new(0.2, 0.2, 1.0), Point3::new(0.8, 0.2, 1.0)),
            (Point3::new(0.8, 0.2, 1.0), Point3::new(0.8, 0.8, 1.0)),
            (Point3::new(0.8, 0.8, 1.0), Point3::new(0.2, 0.8, 1.0)),
            (Point3::new(0.2, 0.8, 1.0), Point3::new(0.2, 0.2, 1.0)),
        ];
        for (a, b) in corners {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let regions: Vec<_> = doc.sketch(s).expect("live").regions().keys().collect();
    assert_eq!(regions.len(), 1);
    assert_blocked(
        doc.extrude_region(s, regions[0], 1.0).map(|(id, _)| id),
        NodeId::Object(solid),
    );
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
/// geometry travels; nothing hidden rides along or frees. A surviving
/// region carried onto free ground extrudes; carried under the standing
/// solid it refuses — the claim stays with the solid, not the ground.
#[test]
fn z4_moving_a_sketch_moves_only_visible_geometry() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 3.0, 0.0, 4.0, 1.0);
    let regions = doc.extrudable_regions(s).expect("live");
    let (solid, _) = doc.extrude_region(s, regions[0], 1.0).expect("extrude");

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

    // Slide it back so the surviving rect sits under the standing solid
    // (from x∈[6,7] to x∈[0,1] — the solid's base): the region refuses.
    doc.transform_sketch(s, &Transform::translation(Vec3::new(-6.0, 0.0, 0.0)))
        .expect("move sketch back over the solid");
    let r = doc
        .sketch(s)
        .expect("live")
        .regions()
        .keys()
        .next()
        .expect("region");
    assert_blocked(
        doc.extrude_region(s, r, 1.0).map(|(id, _)| id),
        NodeId::Object(solid),
    );
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

/// Z6 — deleted geometry cannot merge: a region redrawn across occupied
/// and free ground refuses to extrude whole (every edge visible, the
/// blocking solid physically present), and splitting it at the boundary
/// frees exactly the open half.
#[test]
fn z6_a_region_spanning_occupied_and_free_ground_refuses_until_split() {
    let mut doc = Document::new();
    let (_s1, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 2.0);

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
    assert_blocked(
        doc.extrude_region(s, merged, 1.0).map(|(id, _)| id),
        NodeId::Object(solid),
    );

    // Split at the solid's edge: the free half extrudes, the covered half
    // still refuses.
    {
        let sk = doc.sketch_mut(s).expect("live");
        sk.add_segment(Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 2.0, 0.0))
            .expect("split line");
    }
    let extrudable = doc.extrudable_regions(s).expect("live");
    assert_eq!(extrudable.len(), 1, "exactly the open half is free");
    doc.extrude_region(s, extrudable[0], 1.0)
        .expect("the free half extrudes");
}

/// Z7 — a combined solid claims exactly the area its actual geometry
/// stands on: an intersect result blocks only the overlap strip; the
/// operands' vacated birth areas extrude freely.
#[test]
fn z7_boolean_results_claim_exactly_where_they_stand() {
    let mut doc = Document::new();
    let (_s1, a) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 1.0);
    let (_s2, b) = extrude_ground_rect(&mut doc, 3.0, 0.0, 5.0, 1.0);
    // Overlap them: b moves to x∈[1,3] — the overlap with a is x∈[1,2].
    doc.transform_object(b, &Transform::translation(Vec3::new(-2.0, 0.0, 0.0)))
        .expect("move b");

    let (result, _) = doc
        .boolean(kernel::ops::BooleanOp::Intersect, a, b)
        .expect("intersect");

    // The overlap strip is claimed by the result…
    assert_blocked(try_extrude_ground_rect(&mut doc, 1.0, 0.0, 2.0, 1.0), {
        NodeId::Object(result)
    });
    // …and the non-overlap parts of both operands' bases are free.
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0).expect("a's vacated strip extrudes");
    try_extrude_ground_rect(&mut doc, 2.0, 0.0, 3.0, 1.0).expect("b's vacated strip extrudes");
}

/// A redrawn region overlapping TWO standing solids frees only when the
/// LAST of them goes — the derived gate needs no shared bookkeeping to get
/// multi-solid coverage right.
#[test]
fn area_under_two_solids_frees_with_the_last_one() {
    let mut doc = Document::new();
    let (_s1, a) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 2.0);
    let (_s2, b) = extrude_ground_rect(&mut doc, 2.0, 0.0, 4.0, 2.0);

    // A rect spanning both bases.
    assert!(matches!(
        try_extrude_ground_rect(&mut doc, 1.0, 0.5, 3.0, 1.5),
        Err(DocumentError::RegionBlocked { .. })
    ));

    doc.delete_node(NodeId::Object(a)).expect("delete a");
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 1.0, 0.5, 3.0, 1.5),
        NodeId::Object(b),
    );

    doc.delete_node(NodeId::Object(b)).expect("delete b");
    try_extrude_ground_rect(&mut doc, 1.0, 0.5, 3.0, 1.5).expect("last solid gone, area free");
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

/// Tag-hidden solids claim nothing, exactly like user-hidden ones: the
/// gate computes the same union the Tags panel does — a node is tag-hidden
/// iff any of its tag paths is at or under a hidden tag path, checked on
/// the node itself and on every ancestor group. Un-hiding the tag
/// re-blocks (the claim is derived live).
#[test]
fn tag_hidden_solids_claim_nothing() {
    let mut doc = Document::new();
    let (_s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);
    let tag = vec!["Structure".to_string(), "Roof".to_string()];
    doc.add_node_tag(NodeId::Object(solid), tag.clone())
        .expect("tag the solid");

    // Hiding the tag's PARENT path covers the child-tagged solid too.
    doc.set_tag_hidden(vec!["Structure".to_string()], true);
    let probe = try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0)
        .expect("a tag-hidden solid claims nothing");
    doc.delete_node(NodeId::Object(probe)).expect("clean up");

    // Un-hide: the standing solid blocks again.
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0),
        NodeId::Object(solid),
    );

    // A hidden tag on an ANCESTOR GROUP hides — and un-claims — members.
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(solid)])
        .expect("group the solid");
    doc.add_node_tag(NodeId::Group(group), vec!["Mock".to_string()])
        .expect("tag the group");
    doc.set_tag_hidden(vec!["Mock".to_string()], true);
    try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0)
        .expect("a member of a tag-hidden group claims nothing");
}

/// Tag hiding covers component instances through the same union: a
/// tag-hidden instance claims nothing; un-hiding restores the claim.
#[test]
fn tag_hidden_instances_claim_nothing() {
    let mut doc = Document::new();
    let (_s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0);
    let (_def, instance, _) = doc
        .make_component(&[NodeId::Object(solid)])
        .expect("make component");
    doc.add_node_tag(NodeId::Instance(instance), vec!["Furniture".to_string()])
        .expect("tag the instance");

    doc.set_tag_hidden(vec!["Furniture".to_string()], true);
    let probe = try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0)
        .expect("a tag-hidden instance claims nothing");
    doc.delete_node(NodeId::Object(probe)).expect("clean up");

    doc.set_tag_hidden(vec!["Furniture".to_string()], false);
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.0, 0.0, 1.0, 1.0),
        NodeId::Instance(instance),
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

/// Z7, slice half: each slice piece claims exactly the ground its own
/// geometry stands on — deleting one piece frees its half while the other
/// still blocks (no whole-source claim is inherited).
#[test]
fn z7_slice_pieces_claim_exactly_where_they_stand() {
    let mut doc = Document::new();
    let (_s, solid) = extrude_ground_rect(&mut doc, 0.0, 0.0, 2.0, 1.0);
    let plane = Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .expect("cut plane");
    let ((pos, neg), _) = doc.slice_node(solid, &plane).expect("slice at x = 1");

    // The positive piece stands on x∈[1,2]; the negative on x∈[0,1].
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.1, 0.1, 0.9, 0.9),
        NodeId::Object(neg),
    );
    doc.delete_node(NodeId::Object(neg)).expect("delete left");
    try_extrude_ground_rect(&mut doc, 0.1, 0.1, 0.9, 0.9)
        .expect("the deleted piece's half frees while the other stands");
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 1.1, 0.1, 1.9, 0.9),
        NodeId::Object(pos),
    );
}

/// Z7, push-through half: a through-hole opens the ground beneath it — the
/// result claims its actual (holed) base, not the source's full footprint.
#[test]
fn z7_push_through_results_claim_their_holed_base() {
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

    // Inside the hole the ground is free; on the ring it is claimed.
    try_extrude_ground_rect(&mut doc, 1.2, 1.2, 2.8, 2.8)
        .expect("the hole opened the ground beneath it");
    assert_blocked(
        try_extrude_ground_rect(&mut doc, 0.1, 0.1, 0.9, 0.9),
        NodeId::Object(results[0]),
    );
}
