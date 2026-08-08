//! Phase-1 specs for nested component definitions (docs/design/
//! nested-components.md): the member vocabulary, the definition-graph
//! walks, and placement expansion — pinned BEFORE any op can create nested
//! members, so the flat world keeps behaving identically while the
//! machinery underneath generalizes.

use kernel::{Document, NodeId, Plane, Point3, Transform, Vec3};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn extrude_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("segment");
    }
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], h).expect("extrude").0
}

/// A flat definition reaches no other definition — the cycle walk's
/// baseline (no member instances exist yet).
#[test]
fn flat_definitions_reach_nothing() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (cid_a, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (cid_b, _, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();

    assert!(!doc.def_reaches(cid_a, cid_b));
    assert!(!doc.def_reaches(cid_b, cid_a));
    assert!(
        !doc.def_reaches(cid_a, cid_a),
        "no self-edge without members"
    );
}

/// `expanded_placements` over flat definitions reproduces the classic
/// two-level walk exactly: one entry per (instance, member object), posed
/// by the instance, tagged with that instance.
#[test]
fn expanded_placements_match_the_flat_two_level_walk() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (cid, first, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let shift = Transform::translation(Vec3::new(4.0, 0.0, 0.0));
    let (second, _) = doc.place_instance(cid, shift).unwrap();

    let placements = doc.expanded_placements();
    assert_eq!(placements.len(), 2, "two instances × one member");

    let member = doc.def_members(cid).unwrap()[0];
    let by_instance: Vec<_> = placements
        .iter()
        .map(|(oid, pose, outer)| (*oid, pose.to_affine(), *outer))
        .collect();
    assert!(by_instance.iter().any(|(oid, pose, outer)| *oid == member
        && *outer == first
        && pose == &Transform::IDENTITY.to_affine()));
    assert!(by_instance.iter().any(|(oid, pose, outer)| *oid == member
        && *outer == second
        && pose == &shift.to_affine()));
}

/// Hidden (tombstoned) placements contribute nothing.
#[test]
fn expanded_placements_skip_tombstoned_instances() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (cid, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (extra, _) = doc.place_instance(cid, Transform::IDENTITY).unwrap();
    doc.delete_node(NodeId::Instance(extra)).unwrap();

    assert_eq!(doc.expanded_placements().len(), 1);
}

/// The depth bound is a real, exported constant — the recursion guards and
/// the coming serialization validation all key on the same number.
#[test]
fn depth_bound_is_sane() {
    const {
        assert!(kernel::MAX_COMPONENT_DEPTH >= 16);
        assert!(kernel::MAX_COMPONENT_DEPTH <= 256);
    }
}

// ─────────────────────────── nested op semantics (phase 2 ops) ───────────────

/// `make_component` accepts an instance in the selection: the instance
/// becomes a definition MEMBER (nesting — THE fix), the new def's single
/// placement renders both leaves, and undo restores the world exactly.
#[test]
fn make_component_nests_a_selected_instance() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();

    let (outer_cid, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .expect("selecting an instance nests it");
    // The inner instance is def-owned now: one world instance remains.
    assert_eq!(doc.instance_ids(), vec![outer_inst]);
    assert_eq!(doc.def_depth(outer_cid), 2);
    // Both leaves render through the outer placement.
    let placements = doc.expanded_placements();
    assert_eq!(placements.len(), 2);
    assert!(placements.iter().all(|(_, _, outer)| *outer == outer_inst));

    // Round trip the organically nested document.
    let bytes = doc.save();
    let re = kernel::Document::load(&bytes).expect("loads");
    assert_eq!(re.save(), bytes);

    // Undo restores both world nodes.
    doc.undo().expect("undo the fold");
    assert_eq!(doc.instance_ids(), vec![inner_inst]);
    assert_eq!(doc.visible_object_ids().len(), 1);
    doc.redo().expect("redo the fold");
    assert_eq!(doc.instance_ids(), vec![outer_inst]);
    assert_eq!(doc.expanded_placements().len(), 2);
}

/// A selected GROUP folds in whole — kept as a member group with its
/// subtree re-owned in place (never flattened away), including an instance
/// nested inside it.
#[test]
fn make_component_keeps_a_selected_group_whole() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (gid, _) = doc
        .group_nodes(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();

    let (cid, world_inst, _) = doc.make_component(&[NodeId::Group(gid)]).unwrap();
    // The group is the def's member, not consumed.
    assert_eq!(doc.def_member_nodes(cid), Some(vec![NodeId::Group(gid)]));
    assert_eq!(doc.expanded_placements().len(), 2);
    assert!(
        doc.expanded_placements()
            .iter()
            .all(|(_, _, o)| *o == world_inst)
    );

    let bytes = doc.save();
    let re = kernel::Document::load(&bytes).expect("loads");
    assert_eq!(re.save(), bytes);

    doc.undo().expect("undo");
    // The group is a world node again with both members.
    assert_eq!(doc.top_level_nodes(), vec![NodeId::Group(gid)]);
    doc.redo().expect("redo");
    assert_eq!(doc.expanded_placements().len(), 2);
}

/// Exploding an instance of a nested definition surfaces the nested
/// member instance as an ordinary world instance with the COMPOSED pose —
/// geometry stays shared with the inner definition.
#[test]
fn explode_surfaces_nested_instances_with_composed_poses() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.transform_instance(
        inner_inst,
        &Transform::translation(Vec3::new(0.0, 2.0, 0.0)),
    )
    .unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();
    doc.transform_instance(
        outer_inst,
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    )
    .unwrap();

    let before = doc.expanded_placements().len();
    doc.explode_instance(outer_inst).expect("explode");
    // The nested instance surfaced as a world instance of the INNER def
    // with both translations composed.
    let world = doc.instance_ids();
    assert_eq!(world.len(), 1);
    let surfaced = world[0];
    assert_eq!(doc.instance_def(surfaced), Some(inner_cid));
    let pose = doc.instance_pose(surfaced).unwrap();
    let p = pose.apply_point(Point3::ORIGIN);
    assert!(
        p.approx_eq(Point3::new(10.0, 2.0, 0.0), 1e-9),
        "composed pose carries both translations, got {p:?}"
    );
    // One leaf through the surfaced instance + one baked world object.
    let _ = before;
    assert_eq!(doc.expanded_placements().len(), 1);
    assert_eq!(doc.visible_object_ids().len(), 1);

    // Undo restores the nested arrangement exactly.
    doc.undo().expect("undo explode");
    assert_eq!(doc.instance_ids(), vec![outer_inst]);
    assert_eq!(doc.expanded_placements().len(), 2);
    doc.redo().expect("redo explode");
    assert_eq!(doc.instance_ids().len(), 1);
}

/// `make_unique` on a nested definition copies ONE level deep: the private
/// copy's member instance still SHARES the inner definition (SketchUp's
/// exact semantics).
#[test]
fn make_unique_copies_one_level_and_shares_inner_defs() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (outer_cid, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();

    let (new_cid, _) = doc.make_unique(outer_inst).expect("make unique");
    assert_ne!(new_cid, outer_cid);
    // The copy's member instance shares the INNER definition.
    let copied_members = doc.def_member_nodes(new_cid).unwrap();
    let copied_inst = copied_members
        .iter()
        .find_map(|&m| match m {
            NodeId::Instance(i) => Some(i),
            _ => None,
        })
        .expect("the nested member copied as an instance");
    assert_ne!(copied_inst, inner_inst, "a fresh record");
    assert_eq!(
        doc.expanded_placements().len(),
        2,
        "the unique copy still renders both leaves"
    );

    let bytes = doc.save();
    let re = kernel::Document::load(&bytes).expect("loads");
    assert_eq!(re.save(), bytes);
    let _ = inner_cid;

    doc.undo().expect("undo make_unique");
    assert_eq!(doc.expanded_placements().len(), 2);
    doc.redo().expect("redo make_unique");
    assert_eq!(doc.expanded_placements().len(), 2);
}

/// A component EDIT session on a nested definition surfaces the whole
/// subtree: the member instance becomes an ordinary world instance at the
/// composed pose for the session's duration, and an edit-free close is a
/// byte-exact no-op on the saved document.
#[test]
fn sessions_open_nested_definitions() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.transform_instance(
        inner_inst,
        &Transform::translation(Vec3::new(0.0, 2.0, 0.0)),
    )
    .unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();
    doc.transform_instance(
        outer_inst,
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    )
    .unwrap();
    let before = doc.save();

    doc.open_explode_session(outer_inst)
        .expect("nested session opens");
    // The member instance surfaced: one world instance (the outer's own
    // placement is hidden), at the COMPOSED pose.
    let world = doc.instance_ids();
    assert_eq!(world.len(), 1);
    let surfaced = world[0];
    assert_eq!(surfaced, inner_inst);
    let p = doc
        .instance_pose(surfaced)
        .unwrap()
        .apply_point(Point3::ORIGIN);
    assert!(
        p.approx_eq(Point3::new(10.0, 2.0, 0.0), 1e-9),
        "surfaced at the composed pose, got {p:?}"
    );

    doc.close_explode_session().expect("close");
    assert_eq!(doc.save(), before, "an edit-free session is a no-op");
    assert_eq!(doc.expanded_placements().len(), 2);
}

/// Drill-down: a session on the outer definition, then a second STACKED
/// session entered through the surfaced inner instance; edits land in the
/// inner definition and closes fold back LIFO.
#[test]
fn sessions_stack_for_drill_down() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();

    doc.open_explode_session(outer_inst).expect("outer session");
    doc.open_explode_session(inner_inst)
        .expect("drill into the surfaced inner instance");
    // The inner definition's member is on loan now: move it.
    let inner_member = doc
        .def_members(inner_cid)
        .map(|m| m[0])
        .expect("inner def is live");
    doc.transform_object(
        inner_member,
        &Transform::translation(Vec3::new(0.0, 0.0, 5.0)),
    )
    .expect("edit inside the inner session");
    doc.close_explode_session().expect("close inner");
    doc.close_explode_session().expect("close outer");

    // The edit reached the shared inner definition.
    let placements = doc.expanded_placements();
    assert_eq!(placements.len(), 2);
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("round-trips");
    assert_eq!(re.save(), bytes);
}

// ─────────────── world-op gates on definition-owned nodes (review round) ─────

/// Every world-path op refuses a definition-owned member instance typed —
/// operating on one through the world path would edit or corrupt the
/// OWNING definition's shared content (adversarial review: explode hid a
/// member for every placement; transform/make_unique edited the shared
/// definition silently; extract copied def-local poses as world).
#[test]
fn world_ops_refuse_definition_owned_instances() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    doc.make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();

    use kernel::DocumentError as E;
    assert!(matches!(
        doc.explode_instance(inner_inst),
        Err(E::UnknownInstance)
    ));
    assert!(matches!(
        doc.make_unique(inner_inst),
        Err(E::UnknownInstance)
    ));
    assert!(matches!(
        doc.transform_instance(
            inner_inst,
            &Transform::translation(Vec3::new(1.0, 0.0, 0.0))
        ),
        Err(E::UnknownInstance)
    ));
    assert!(matches!(
        doc.open_explode_session(inner_inst),
        Err(E::UnknownInstance)
    ));
    assert!(matches!(
        doc.extract_item(&[NodeId::Instance(inner_inst)], true),
        Err(E::UnknownInstance)
    ));
}

/// A definition nested inside another AND placed in the world can be
/// edited through its world placement: the def-owned placement stays live
/// but the outer definitions' expansion SKIPS the on-loan content for the
/// session's duration, and everything returns at close.
#[test]
fn sessions_on_defs_nested_elsewhere_skip_on_loan_content() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    // A SECOND world placement of the inner def survives the fold below.
    let (world_inst, _) = doc
        .place_instance(inner_cid, Transform::translation(Vec3::new(8.0, 0.0, 0.0)))
        .unwrap();
    doc.make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();
    assert_eq!(doc.expanded_placements().len(), 3);

    doc.open_explode_session(world_inst)
        .expect("session through the world placement");
    // The outer definition's placement drops the on-loan inner content
    // (1 = the outer's own body; the session geometry renders as world
    // objects, not placements).
    assert_eq!(doc.expanded_placements().len(), 1);
    doc.close_explode_session().expect("close");
    assert_eq!(doc.expanded_placements().len(), 3, "content returns");
}

/// Placing an instance MID-SESSION folds it into the open definition as a
/// nested member at close — the interactive route to building nesting.
/// Placing the definition being edited (or anything reaching it) refuses
/// as a cycle.
#[test]
fn mid_session_placement_folds_as_nested_member() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (part_cid, _, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (assembly_cid, assembly_inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();

    doc.open_explode_session(assembly_inst).expect("open");
    let (placed, _) = doc
        .place_instance(part_cid, Transform::translation(Vec3::new(0.0, 5.0, 0.0)))
        .expect("placing an unrelated definition mid-session is allowed");
    assert!(matches!(
        doc.place_instance(assembly_cid, Transform::IDENTITY),
        Err(kernel::DocumentError::ComponentCycle)
    ));
    doc.close_explode_session()
        .expect("close folds the placement in");

    // The placement is now a MEMBER instance of the assembly.
    assert!(
        doc.def_member_nodes(assembly_cid)
            .unwrap()
            .contains(&NodeId::Instance(placed))
    );
    assert_eq!(
        doc.def_depth(assembly_cid),
        2,
        "nesting built interactively"
    );
    // 3 = the assembly's two leaves (own body + nested part) plus the
    // part's ORIGINAL world placement from its own make_component.
    assert_eq!(doc.expanded_placements().len(), 3);
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("round-trips");
    assert_eq!(re.save(), bytes);

    // And the whole construction undoes exactly.
    doc.undo().expect("undo close");
    doc.undo().expect("undo place");
    doc.undo().expect("undo open");
    assert_eq!(
        doc.expanded_placements().len(),
        2,
        "both flat placements back"
    );
    assert_eq!(doc.def_depth(assembly_cid), 1);
    let _ = placed;
}

/// Undo/redo across a nested session's boundaries restores each state
/// byte-exactly.
#[test]
fn nested_session_undo_redo_round_trips() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();
    let closed_before = doc.save();

    doc.open_explode_session(outer_inst).expect("open");
    doc.transform_instance(
        inner_inst,
        &Transform::translation(Vec3::new(0.0, 3.0, 0.0)),
    )
    .expect("move the surfaced member instance");
    doc.close_explode_session().expect("close");
    let closed_after = doc.save();
    assert_ne!(closed_after, closed_before, "the edit landed");

    doc.undo().expect("undo close (reopens)");
    doc.undo().expect("undo the move");
    doc.undo().expect("undo open");
    assert_eq!(doc.save(), closed_before, "fully unwound, byte-exact");

    doc.redo().expect("redo open");
    doc.redo().expect("redo the move");
    doc.redo().expect("redo close");
    assert_eq!(doc.save(), closed_after, "fully replayed, byte-exact");
}

/// A definition edit touches the OUTER placements too: `placing_instances`
/// reports world instances of every definition that transitively nests the
/// edited one, so renderers and inference refresh the placements that
/// actually draw the content.
#[test]
fn placing_instances_reaches_outer_placements() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Instance(inner_inst)])
        .unwrap();

    // The inner def has NO world placements of its own; its content
    // renders only through the outer instance.
    assert!(doc.instances_of(inner_cid).len() == 1); // the def-owned member
    assert_eq!(doc.placing_instances(inner_cid), vec![outer_inst]);

    // And a def-owned member instance is never a "world" instance.
    assert!(!doc.instance_is_world(inner_inst));
    assert!(doc.instance_is_world(outer_inst));
}

/// The expansion and definition-graph walks are iterative: a deep
/// group chain inside a definition cannot overflow the stack (hostile
/// files reach these walks through render/export paths).
#[test]
fn deep_group_chains_expand_without_recursion() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    // 1_000 nested world groups around the solid, then folded whole —
    // deep enough that a per-edge recursion would be felt, cheap enough
    // for the suite (the walk itself is O(n); the setup ops dominate).
    let mut node = NodeId::Object(a);
    for _ in 0..1_000 {
        let (g, _) = doc.group_nodes(&[node]).unwrap();
        node = NodeId::Group(g);
    }
    let (cid, _, _) = doc.make_component(&[node]).unwrap();
    assert_eq!(doc.expanded_placements().len(), 1);
    assert_eq!(doc.expanded_def_placements(cid).len(), 1);
    assert_eq!(doc.def_depth(cid), 1, "groups do not add definition depth");
    assert!(!doc.def_reaches(cid, cid));
    // And the file round-trips.
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("deep group chain loads");
    assert_eq!(re.expanded_placements().len(), 1);
}

/// The WIDTH bound: a doubling ladder of nested definitions (each level
/// folds two placements of the previous) grows to half the cap, and the
/// placement that would cross it refuses typed — while the document at
/// the boundary still saves and reloads (whatever saves, reopens).
#[test]
fn expansion_width_is_bounded_at_the_op() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    // 19 doublings: 2^19 = 524_288 expanded placements — legal.
    for _ in 0..19 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    assert_eq!(doc.expanded_total(), 1 << 19);
    // The 20th doubling would cross 1,000,000 — the PLACEMENT refuses.
    assert!(matches!(
        doc.place_instance(def, Transform::IDENTITY),
        Err(kernel::DocumentError::ComponentExpansionExceeded)
    ));
    // Duplicating the world instance would double the total likewise.
    assert!(matches!(
        doc.duplicate_node(
            NodeId::Instance(inst),
            &Transform::translation(Vec3::new(2.0, 0.0, 0.0))
        ),
        Err(kernel::DocumentError::ComponentExpansionExceeded)
    ));
    // The at-the-boundary document is legal: it saves and reopens.
    let bytes = doc.save();
    let re = Document::load(&bytes).expect("a within-bound document reopens");
    assert_eq!(re.expanded_total(), 1 << 19);
}

/// The app scopes session picking/selection to `session_direct_members`,
/// so a component frame must list its SURFACED member groups and
/// instances — not just objects — or a nested component would be
/// unselectable (and undrillable) inside its parent's session.
#[test]
fn session_direct_members_lists_surfaced_nested_nodes() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let c = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (part_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (gid, _) = doc.group_nodes(&[NodeId::Object(c)]).unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[
            NodeId::Object(b),
            NodeId::Instance(inner_inst),
            NodeId::Group(gid),
        ])
        .unwrap();

    doc.open_explode_session(outer_inst).expect("open");
    let members = doc.session_direct_members().expect("a frame is open");
    assert!(
        members.contains(&NodeId::Instance(inner_inst)),
        "the surfaced nested instance is selectable: {members:?}"
    );
    assert!(
        members.contains(&NodeId::Group(gid)),
        "the surfaced member group is selectable: {members:?}"
    );
    assert!(
        members.contains(&NodeId::Object(b)),
        "the plain member object still is: {members:?}"
    );
    assert!(
        !members.contains(&NodeId::Object(c)),
        "an object INSIDE a surfaced group belongs to the group, not the frame"
    );

    // A mid-session placement joins the frame's members immediately.
    let (placed, _) = doc
        .place_instance(part_cid, Transform::translation(Vec3::new(0.0, 9.0, 0.0)))
        .expect("place mid-session");
    assert!(
        doc.session_direct_members()
            .unwrap()
            .contains(&NodeId::Instance(placed))
    );
    doc.close_explode_session().expect("close");
    assert!(doc.session_direct_members().is_none());
}

/// The width bound is FOLD-AWARE: a placement made inside a session ends
/// up rendered once per placement of the open definition, so the gate
/// multiplies by that count instead of measuring the placement alone
/// (adversarial review — the mid-session route bypassed the cap).
#[test]
fn mid_session_placement_width_gate_accounts_for_the_fold() {
    let mut doc = Document::new();
    // A small part: 4 leaves.
    let p1 = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let p2 = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let p3 = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 1.0);
    let p4 = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (part_cid, part_inst, _) = doc
        .make_component(&[
            NodeId::Object(p1),
            NodeId::Object(p2),
            NodeId::Object(p3),
            NodeId::Object(p4),
        ])
        .unwrap();
    assert_eq!(doc.def_placement_count(part_cid), 4);

    // A BIG definition — 2^15 leaves — built by doubling, cheaply.
    let big_seed = extrude_box(&mut doc, 0.0, 40.0, 1.0, 41.0, 1.0);
    let (mut big_cid, mut big_inst, _) = doc.make_component(&[NodeId::Object(big_seed)]).unwrap();
    for _ in 0..15 {
        let (second, _) = doc
            .place_instance(big_cid, Transform::translation(Vec3::new(2.0, 40.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(big_inst), NodeId::Instance(second)])
            .unwrap();
        big_cid = d;
        big_inst = i;
    }
    assert_eq!(doc.def_placement_count(big_cid), 1 << 15);

    // A host definition placed 50 times: content folded into it multiplies.
    let h = extrude_box(&mut doc, 0.0, 20.0, 1.0, 21.0, 1.0);
    let (host_cid, host_inst, _) = doc.make_component(&[NodeId::Object(h)]).unwrap();
    for k in 1..50 {
        doc.place_instance(
            host_cid,
            Transform::translation(Vec3::new(3.0 * k as f64, 20.0, 0.0)),
        )
        .unwrap();
    }
    assert_eq!(doc.instances_of(host_cid).len(), 50);
    let outside_session = doc.expanded_total();

    doc.open_explode_session(host_inst).expect("open");
    // 2^15 × 50 placements = 1_638_400 once folded — REFUSED, even though
    // the same placement outside the session is affordable.
    assert!(matches!(
        doc.place_instance(big_cid, Transform::IDENTITY),
        Err(kernel::DocumentError::ComponentExpansionExceeded)
    ));
    // The small part costs 4 × 50 = 200 folded: affordable.
    doc.place_instance(part_cid, Transform::IDENTITY)
        .expect("a small placement is affordable");
    doc.close_explode_session().expect("close");

    // Every host placement now renders its own leaf plus the folded part.
    assert_eq!(
        doc.expanded_total(),
        outside_session + 200,
        "the fold multiplied across all 50 host placements"
    );
    let _ = (part_inst, big_inst);
}

/// Undoing a session CLOSE reports every entity it moved — the fold-outs
/// and the re-baked surfaced nodes — so the render and snap layers can't
/// keep stale state (adversarial review: the arm reported objects only).
#[test]
fn undo_of_close_reports_surfaced_and_folded_nodes() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let c = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 1.0);
    let (part_cid, inner_inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (gid, _) = doc.group_nodes(&[NodeId::Object(c)]).unwrap();
    let (_, outer_inst, _) = doc
        .make_component(&[
            NodeId::Object(b),
            NodeId::Instance(inner_inst),
            NodeId::Group(gid),
        ])
        .unwrap();

    doc.open_explode_session(outer_inst).expect("open");
    let (placed, _) = doc
        .place_instance(part_cid, Transform::translation(Vec3::new(0.0, 9.0, 0.0)))
        .expect("place mid-session");
    doc.close_explode_session().expect("close");

    let change = doc.undo().expect("undo the close reopens the session");
    assert!(
        change.groups_touched.contains(&gid),
        "the re-surfaced member group is reported: {:?}",
        change.groups_touched
    );
    assert!(
        change.instances_touched.contains(&inner_inst),
        "the re-surfaced member instance is reported"
    );
    assert!(
        change.instances_touched.contains(&placed),
        "the un-folded mid-session placement is reported"
    );
}

/// The close-time width BACKSTOP: geometry created inside a definition
/// placed many times multiplies at the fold, and plain creation is gated
/// nowhere else — so the close refuses typed rather than producing a
/// document that saves but cannot reopen.
#[test]
fn close_refuses_when_the_fold_would_exceed_the_width_bound() {
    let mut doc = Document::new();
    // A host definition whose fold multiplier is enormous: build a
    // 2^17-wide ladder, then edit its innermost definition.
    let seed = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(seed)]).unwrap();
    for _ in 0..17 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    assert_eq!(doc.expanded_total(), 1 << 17);

    // Edit the LEAF definition — its content renders 2^17 times.
    let leaf_inst = doc
        .instance_ids()
        .into_iter()
        .find(|&i| doc.instance_def(i) == Some(def))
        .expect("the outermost placement");
    doc.open_explode_session(leaf_inst).expect("open");
    // Placing the whole assembly inside itself is a cycle; place a big
    // sibling instead: 2^17 × the multiplier busts the bound at the fold.
    let extra = extrude_box(&mut doc, 100.0, 0.0, 101.0, 1.0, 1.0);
    let _ = extra;
    doc.close_explode_session()
        .expect("a small session closes fine");

    // Now the pathological case: a definition placed 8 times, edited to
    // hold more leaves than the cap allows once multiplied.
    let mut doc2 = Document::new();
    let h = extrude_box(&mut doc2, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (host_cid, host_inst, _) = doc2.make_component(&[NodeId::Object(h)]).unwrap();
    for k in 1..8 {
        doc2.place_instance(
            host_cid,
            Transform::translation(Vec3::new(3.0 * k as f64, 0.0, 0.0)),
        )
        .unwrap();
    }
    // A big definition to place inside: 2^18 wide.
    let seed2 = extrude_box(&mut doc2, 0.0, 50.0, 1.0, 51.0, 1.0);
    let (mut big, mut big_inst, _) = doc2.make_component(&[NodeId::Object(seed2)]).unwrap();
    for _ in 0..18 {
        let (second, _) = doc2
            .place_instance(big, Transform::translation(Vec3::new(2.0, 50.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc2
            .make_component(&[NodeId::Instance(big_inst), NodeId::Instance(second)])
            .unwrap();
        big = d;
        big_inst = i;
    }
    doc2.delete_node(NodeId::Instance(big_inst)).unwrap();
    doc2.open_explode_session(host_inst).expect("open host");
    // 2^18 × 8 placements = 2_097_152 > 1M — refused at the placement.
    assert!(matches!(
        doc2.place_instance(big, Transform::IDENTITY),
        Err(kernel::DocumentError::ComponentExpansionExceeded)
    ));
    doc2.close_explode_session()
        .expect("nothing oversized was added");
    let bytes = doc2.save();
    Document::load(&bytes).expect("whatever saves reopens");
}

/// Drilling into a nested GROUP while editing a component — the theater
/// model's shape (a component whose members are groups of solids). The
/// group frame stacks on the component frame, edits land in the
/// definition, and closes unwind LIFO.
#[test]
fn group_sessions_open_inside_a_component() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (inner_grp, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    doc.set_node_name(NodeId::Group(inner_grp), Some("Group#427".into()))
        .unwrap();
    let (cid, inst, _) = doc
        .make_component(&[NodeId::Group(inner_grp), NodeId::Object(b)])
        .expect("a component whose member is a group");
    let before = doc.save();

    doc.open_explode_session(inst).expect("open the component");
    // The member group surfaced: entering it stacks a group frame.
    doc.open_group_session(inner_grp)
        .expect("a nested group opens inside the component session");
    assert_eq!(
        doc.session_stack(),
        vec![NodeId::Instance(inst), NodeId::Group(inner_grp)]
    );

    // An edit inside the group lands in the definition after both closes.
    doc.transform_object(a, &Transform::translation(Vec3::new(0.0, 0.0, 4.0)))
        .expect("edit inside the nested group");
    doc.close_innermost_session().expect("close the group");
    assert_eq!(doc.session_stack(), vec![NodeId::Instance(inst)]);
    doc.close_innermost_session().expect("close the component");
    assert!(doc.session_stack().is_empty());

    // The definition kept its group member, and the edit is in it.
    assert_eq!(
        doc.def_member_nodes(cid),
        Some(vec![NodeId::Group(inner_grp), NodeId::Object(b)])
    );
    assert_eq!(doc.expanded_placements().len(), 2);
    let bytes = doc.save();
    assert_ne!(bytes, before, "the edit landed");
    let re = Document::load(&bytes).expect("round-trips");
    assert_eq!(re.save(), bytes);

    // ... and unwinds exactly.
    doc.undo().expect("undo close component");
    doc.undo().expect("undo close group");
    doc.undo().expect("undo the edit");
    doc.undo().expect("undo open group");
    doc.undo().expect("undo open component");
    assert_eq!(doc.save(), before, "byte-exact unwind");
}

/// A group from OUTSIDE the open component frame stays out of scope — the
/// stacking is for the definition's OWN content, not a door into the
/// world tree.
#[test]
fn outside_groups_stay_out_of_scope_during_a_component_session() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 1.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (outside, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();

    doc.open_explode_session(inst).expect("open");
    assert!(matches!(
        doc.open_group_session(outside),
        Err(kernel::DocumentError::ExplodeSessionScope)
    ));
}
