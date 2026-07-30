// Test-only: the set comparison below is an order-independent equality
// assertion on an id collection, not kernel output, so HashSet is fine here.
// Suppress the workspace clippy.toml ban for this integration-test crate
// (mirrors `document_specs.rs`).
#![allow(clippy::disallowed_types)]

//! Executable specs for component-edit-parity.md **phase K1**: sketches and
//! extrusion owned by a component definition instead of the world tree.
//!
//! The contract under test:
//! - a def-owned sketch created through an instance ([`begin_sketch_on_plane_in_instance`])
//!   is stored in DEFINITION-local space — the kernel maps the caller's
//!   WORLD-space plane through the instance's pose⁻¹ itself, exactly once,
//!   regardless of the pose's rotation, mirror, or (for the plane) non-uniform
//!   scale;
//! - extruding such a sketch ([`extrude_region_in_instance`]) births the solid
//!   as a definition member — visible, and edited, through **every** instance
//!   of that definition, not just the one it was drawn through — and maps its
//!   WORLD-space `distance` through the pose's uniform scale, refusing typed
//!   under a non-uniformly-scaled instance where a single scalar is ambiguous;
//! - `delete_def_member` refuses to delete a definition's last live member;
//! - the pre-existing WORLD-only surfaces (`extrude_region`, `follow_me*`)
//!   refuse a def-owned sketch exactly like `apply_object_op` refuses a
//!   definition-member object — a def-owned sketch must never leak a
//!   WORLD object built from definition-local coordinates;
//! - undo/redo are exact through the new surfaces, and the manifest
//!   round-trips a sketch's `SketchOwner`.
//!
//! [`begin_sketch_on_plane_in_instance`]: kernel::Document::begin_sketch_on_plane_in_instance
//! [`extrude_region_in_instance`]: kernel::Document::extrude_region_in_instance

use kernel::{Document, DocumentError, NodeId, Object, Plane, Point3, Transform, Vec3};
use std::collections::HashSet;

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

/// A throwaway placeholder box, extruded directly in world space — just
/// something to fold into a component via `make_component` so a definition
/// (and its first instance) exist to attach def-owned sketches/members to.
fn placeholder_box(doc: &mut Document) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)),
        (Point3::new(1.0, 0.0, 0.0), Point3::new(1.0, 1.0, 0.0)),
        (Point3::new(1.0, 1.0, 0.0), Point3::new(0.0, 1.0, 0.0)),
        (Point3::new(0.0, 1.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    let (id, _) = doc.extrude_region(s, region, 1.0).expect("extrude");
    id
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

/// The unique vertex positions of `obj` (topology-order, not per-face
/// flattened) — a box's 8 corners. Order-independent identity is exactly
/// what a `pose` mapping test needs: a mirrored pose reverses face winding,
/// which a cyclic polygon-loop comparison (as in `document_specs.rs`'s
/// `objects_equivalent`) would wrongly reject.
fn vertex_positions(obj: &Object) -> Vec<Point3> {
    obj.to_polygons().0
}

/// Whether mapping every point of `local` through `pose` reproduces exactly
/// the vertex set of `expected_world` (each point matched once, any order) —
/// the pose⁻¹-mapping correctness check, robust to a mirrored pose's
/// reversed winding (see `vertex_positions`).
fn mapped_vertices_match(local: &Object, pose: &Transform, expected_world: &Object) -> bool {
    let mapped: Vec<Point3> = vertex_positions(local)
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    let mut remaining = vertex_positions(expected_world);
    if mapped.len() != remaining.len() {
        return false;
    }
    for p in mapped {
        match remaining.iter().position(|&q| p.approx_eq(q, 1e-6)) {
            Some(i) => {
                remaining.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// Whether `x` and `y` are the same polygon soup up to index relabeling and
/// cyclic rotation of each face loop (but NOT reversal — see
/// `mapped_vertices_match` for the winding-agnostic comparison a mirrored
/// pose needs). Used to confirm redo reproduces byte-for-byte-equivalent
/// geometry after undo, mirroring `document_specs.rs`'s helper of the same
/// name and contract (duplicated locally: integration test binaries do not
/// share code across files).
fn objects_equivalent(x: &Object, y: &Object) -> bool {
    fn polygons_of(obj: &Object) -> Vec<Vec<Point3>> {
        let (points, faces) = obj.to_polygons();
        faces
            .into_iter()
            .map(|poly| poly.into_iter().map(|i| points[i]).collect())
            .collect()
    }
    fn cyclic_match(a: &[Point3], b: &[Point3]) -> bool {
        a.len() == b.len()
            && (0..a.len()).any(|shift| {
                a.iter()
                    .enumerate()
                    .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], 1e-9))
            })
    }
    let xs = polygons_of(x);
    let mut ys = polygons_of(y);
    if xs.len() != ys.len() {
        return false;
    }
    for poly in xs {
        match ys.iter().position(|cand| cyclic_match(&poly, cand)) {
            Some(i) => {
                ys.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// The unsigned volume of `obj` (six-times-volume divided down), independent
/// of winding sign — used to cross-check `mapped_vertices_match` against a
/// scale-conversion bug that happens to preserve the vertex set (impossible
/// for a box, but cheap insurance).
fn abs_volume(obj: &Object) -> f64 {
    let (pts, faces) = obj.to_polygons();
    let mut six_v = 0.0;
    for face in faces {
        for i in 1..face.len() - 1 {
            let (a, b, c) = (pts[face[0]], pts[face[i]], pts[face[i + 1]]);
            six_v += a.to_vec().dot(b.to_vec().cross(c.to_vec()));
        }
    }
    (six_v / 6.0).abs()
}

/// Builds a component (with a throwaway placeholder member) and returns
/// `(doc, component, first_instance)`.
fn component_with_instance(doc: &mut Document) -> (kernel::ComponentId, kernel::InstanceId) {
    let o = placeholder_box(doc);
    let (comp, inst, _) = doc
        .make_component(&[NodeId::Object(o)])
        .expect("make_component");
    (comp, inst)
}

/// A 4×6 rectangle's WORLD corners on the ground plane, translated to
/// `origin` — the fixed "world shape" every pose⁻¹ test draws and expects
/// back out, regardless of which instance (and pose) it went through.
fn world_rect_corners(origin: Point3) -> [Point3; 4] {
    [
        origin,
        origin + Vec3::new(4.0, 0.0, 0.0),
        origin + Vec3::new(4.0, 6.0, 0.0),
        origin + Vec3::new(0.0, 6.0, 0.0),
    ]
}

// --------------------------------------------------- pose⁻¹ mapping specs

/// Drawing and extruding through a ROTATED + UNIFORMLY-SCALED instance
/// reproduces, once mapped forward through the same pose, exactly the box a
/// plain world `extrude_region` would have built from the same WORLD
/// rectangle and distance — the pose⁻¹ round trip for both the sketch plane
/// and the extrusion distance (divided by the pose's similarity scale).
#[test]
fn extrude_region_in_instance_round_trips_through_a_rotated_scaled_pose() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);

    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7)
        .unwrap()
        .then(&Transform::uniform_scale(2.0))
        .then(&Transform::translation(Vec3::new(10.0, -3.0, 5.0)));
    doc.transform_instance(inst, &pose).unwrap();

    let world_distance = 8.0;
    let corners = world_rect_corners(Point3::new(2.0, 2.0, 0.0));

    // The expected shape: extrude the SAME world rectangle in plain world
    // space, independent of the instance/component machinery entirely.
    let ws = doc.add_sketch(ground());
    let world_region = draw_rect(&mut doc, ws, corners);
    let (expected_id, _) = doc
        .extrude_region(ws, world_region, world_distance)
        .expect("world extrude");
    let expected = doc.object(expected_id).unwrap().clone();

    // The same shape, drawn and extruded THROUGH the instance: the plane
    // comes back in definition-local space, so the corners must be mapped
    // through pose⁻¹ before being drawn (the per-segment sketch API is not
    // itself instance-aware — only sketch *creation* and *extrusion* map
    // through the pose in this phase).
    let (sid, _change) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let pose_inv = pose.inverse().unwrap();
    let local_corners = corners.map(|p| pose_inv.apply_point(p));
    let region = draw_rect(&mut doc, sid, local_corners);
    let (member_id, change) = doc
        .extrude_region_in_instance(inst, sid, region, world_distance)
        .expect("extrude in instance");
    let member = doc.object(member_id).unwrap().clone();

    assert!(
        mapped_vertices_match(&member, &pose, &expected),
        "the def-local member, mapped through the instance pose, must reproduce \
         the same box a plain world extrude_region would have built"
    );
    assert!(
        (abs_volume(&member) * pose.determinant().abs() - abs_volume(&expected)).abs() < 1e-6,
        "volume scales by the pose's determinant"
    );

    // Birth touches the component and every one of its instances — the
    // shared-geometry propagation contract `apply_def_op` already has.
    assert!(doc.def_members(comp).unwrap().contains(&member_id));
    assert_eq!(change.components_touched, vec![comp]);
    assert_eq!(change.instances_touched, vec![inst]);
}

/// The same shape, drawn and extruded through a MIRRORED instance (negative
/// determinant, still a uniform-scale similarity): `Transform::apply_plane`
/// and the distance mapping must both handle the reflection.
///
/// Unlike the rotated/scaled spec above, this one does NOT compare against
/// an independently-built "same positive world distance" solid: a mirror
/// genuinely flips WHICH world side a positive local distance sweeps
/// toward (the same real quirk every 3D app's mirrored-component push/pull
/// has — not a bug to paper over). What must hold regardless of that sign
/// convention is the actual pose⁻¹ CONTRACT: the profile's own corners —
/// mapped through pose⁻¹ to draw, then back through pose after extruding —
/// round-trip EXACTLY to the original world corners, and the swept volume's
/// MAGNITUDE matches the typed world distance exactly (scaled by the pose's
/// determinant, here 1).
#[test]
fn extrude_region_in_instance_round_trips_through_a_mirrored_pose() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);

    // A pure mirror about the world YZ plane, plus a translation: uniform
    // |scale| = 1, determinant < 0.
    let pose = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(7.0, 4.0, 0.0)));
    assert!(pose.determinant() < 0.0, "sanity: this pose mirrors");
    doc.transform_instance(inst, &pose).unwrap();

    let world_distance = 3.0;
    let corners = world_rect_corners(Point3::new(1.0, 1.0, 0.0));
    let area = (corners[1] - corners[0])
        .cross(corners[3] - corners[0])
        .length();

    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let pose_inv = pose.inverse().unwrap();
    let local_corners = corners.map(|p| pose_inv.apply_point(p));
    let region = draw_rect(&mut doc, sid, local_corners);
    let (member_id, _) = doc
        .extrude_region_in_instance(inst, sid, region, world_distance)
        .expect("extrude in instance through a mirrored pose");
    let member = doc.object(member_id).unwrap().clone();

    // The base profile's corners round-trip EXACTLY through pose⁻¹-then-pose
    // (this is the actual pose⁻¹ contract; extrusion direction is not part
    // of it).
    let mapped: Vec<Point3> = vertex_positions(&member)
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    for &corner in &corners {
        assert!(
            mapped.iter().any(|&p| p.approx_eq(corner, 1e-6)),
            "world corner {corner:?} must reappear among the mapped member's vertices"
        );
    }

    // The swept volume's magnitude matches the typed world distance, scaled
    // by the pose's determinant (1 here) — the same invariant the
    // rotated/scaled spec checks, independent of extrusion side.
    assert!(
        (abs_volume(&member) * pose.determinant().abs() - area * world_distance).abs() < 1e-6,
        "volume magnitude matches the typed world distance under a mirror"
    );
}

/// A non-uniformly-scaled instance cannot map a typed WORLD-space distance
/// unambiguously (which axis's scale would it be?), so
/// `extrude_region_in_instance` refuses typed — the document untouched —
/// even though the SKETCH PLANE itself maps through `apply_plane` just fine
/// under non-uniform scale (no ambiguity for a plane/point mapping, only for
/// a bare scalar length).
#[test]
fn extrude_region_in_instance_refuses_a_typed_distance_under_non_uniform_scale() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);

    let pose = Transform::scale(Vec3::new(2.0, 3.0, 0.5));
    doc.transform_instance(inst, &pose).unwrap();

    // The plane itself maps fine under non-uniform scale.
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance under non-uniform scale");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );

    let members_before = doc.def_members(comp).unwrap();
    let result = doc.extrude_region_in_instance(inst, sid, region, 5.0);
    assert_eq!(result, Err(DocumentError::AmbiguousInstanceScale));
    assert_eq!(
        doc.def_members(comp).unwrap(),
        members_before,
        "a refused extrude touches nothing"
    );
}

// ------------------------------------------- instance-propagation spec

/// Extruding a def-owned sketch through ONE instance is a shared-geometry
/// birth: the new member shows up for `def_members`, and the returned
/// `DocChange` names EVERY instance of the definition (not just the one
/// drawn through) — the exact contract `apply_def_op` already guarantees for
/// edits to an EXISTING member, now proven for a BIRTH.
#[test]
fn extrude_region_in_instance_propagates_to_every_sibling_instance() {
    let mut doc = Document::new();
    let (comp, i1) = component_with_instance(&mut doc);
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let (i3, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(-20.0, 0.0, 0.0)))
        .unwrap();

    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(i1, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    let (member_id, change) = doc
        .extrude_region_in_instance(i1, sid, region, 1.0)
        .expect("extrude in instance");

    assert!(doc.def_members(comp).unwrap().contains(&member_id));
    let touched: HashSet<_> = change.instances_touched.into_iter().collect();
    assert_eq!(
        touched,
        [i1, i2, i3].into_iter().collect(),
        "a shared-geometry birth touches every instance, not just the one drawn through"
    );
}

// --------------------------------------------------------------- undo/redo

/// Undo/redo of `extrude_region_in_instance` are exact inverses through
/// history: undo hides the born member (handle stays valid) and restores
/// the sketch's consumed scaffolding; redo re-applies both — exactly like
/// `extrude_region`'s own undo/redo, just for a definition member.
///
/// A definition-owned birth's undo/redo must also report EVERY sibling
/// instance in `DocChange` on BOTH directions — not just the object/sketch —
/// exactly like `DeletedDefMember`'s own undo/redo already do. Without it,
/// undoing/redoing an in-component extrusion leaves stale inference/render
/// state on every instance but the one edited through (verified live).
#[test]
fn extrude_region_in_instance_undo_redo_round_trips() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);
    // A second sibling instance: the touched-list regression is invisible
    // with only one instance (it happens to equal `def_members`'s
    // single-instance case by coincidence).
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let members_before = doc.def_members(comp).unwrap();

    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    let (member_id, _) = doc
        .extrude_region_in_instance(inst, sid, region, 2.0)
        .expect("extrude in instance");
    let shape = doc.object(member_id).unwrap().clone();
    assert!(
        doc.sketch(sid).is_none(),
        "the sketch fully consumed (Model D)"
    );

    let undo_change = doc.undo().expect("undo the extrusion");
    assert!(doc.object(member_id).is_none(), "the member is hidden");
    assert_eq!(
        doc.def_members(comp).unwrap(),
        {
            let mut m = members_before.clone();
            m.push(member_id);
            m
        },
        "the member stays listed (hidden), like an undone world birth"
    );
    assert!(doc.sketch(sid).is_some(), "the scaffolding came back");
    assert_eq!(undo_change.components_touched, vec![comp]);
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect(),
        "undoing a def-owned birth must touch every sibling instance, not \
         just the one drawn through"
    );

    let redo_change = doc.redo().expect("redo the extrusion");
    assert!(
        doc.object(member_id).is_some(),
        "the member is visible again"
    );
    assert!(objects_equivalent(&shape, doc.object(member_id).unwrap()));
    assert!(doc.sketch(sid).is_none(), "re-consumed on redo");
    assert_eq!(redo_change.components_touched, vec![comp]);
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect(),
        "redoing a def-owned birth must touch every sibling instance too"
    );
}

// -------------------------------------------- delete_def_member specs

/// `delete_def_member` refuses to delete a definition's LAST live member —
/// SketchUp deletes the emptied component outright; v1 refuses instead — but
/// happily deletes any OTHER member, and undo restores it exactly.
#[test]
fn delete_def_member_refuses_the_last_member_but_allows_others() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);
    let only_member = doc.def_members(comp).unwrap()[0];

    assert_eq!(
        doc.delete_def_member(comp, only_member),
        Err(DocumentError::LastDefinitionMember)
    );
    assert!(
        doc.object(only_member).is_some(),
        "a refused delete touches nothing"
    );

    // Add a second member, then delete IS allowed (no longer the last).
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
        ],
    );
    let (second_member, _) = doc
        .extrude_region_in_instance(inst, sid, region, 1.0)
        .expect("second member");

    let change = doc
        .delete_def_member(comp, second_member)
        .expect("delete the (no longer last) member");
    assert!(
        doc.object(second_member).is_none(),
        "deleted member is hidden"
    );
    assert!(
        doc.object(only_member).is_some(),
        "sibling member untouched"
    );
    assert_eq!(change.instances_touched, vec![inst]);

    // Now only one live member remains — deleting IT is refused again.
    assert_eq!(
        doc.delete_def_member(comp, only_member),
        Err(DocumentError::LastDefinitionMember)
    );

    // Undo restores the deleted member exactly.
    doc.undo().expect("undo the delete");
    assert!(
        doc.object(second_member).is_some(),
        "undo restores the member"
    );

    doc.redo().expect("redo the delete");
    assert!(doc.object(second_member).is_none(), "redo re-hides it");
}

/// `delete_def_member` refuses a stale/unknown component or a non-member
/// object, and never touches the document on a refusal.
#[test]
fn delete_def_member_refuses_unknown_component_and_non_member() {
    let mut doc = Document::new();
    let (comp, _inst) = component_with_instance(&mut doc);
    let (other_comp, _other_inst) = component_with_instance(&mut doc);
    let other_member = doc.def_members(other_comp).unwrap()[0];

    // A live object that is a member of a DIFFERENT definition is not a
    // member of `comp`.
    assert_eq!(
        doc.delete_def_member(comp, other_member),
        Err(DocumentError::UnknownObject)
    );

    // Undoing the `make_component` that created `other_comp` hides the
    // definition itself (tombstone) — a genuinely stale/hidden component id.
    doc.undo()
        .expect("undo the make_component that created other_comp");
    assert_eq!(
        doc.delete_def_member(other_comp, other_member),
        Err(DocumentError::UnknownComponent)
    );
}

// ---------------------------------------------- world-op guard regression

/// A def-owned sketch must never leak a WORLD object built from
/// definition-local coordinates: the pre-existing WORLD-only
/// `extrude_region` refuses one exactly like `apply_object_op` refuses a
/// definition-member OBJECT (the `is_world()` guard). Without this guard, a
/// def-owned sketch sitting in the same `SketchId` space as world sketches
/// would silently produce a wrongly-placed "world" solid.
#[test]
fn world_extrude_region_refuses_a_def_owned_sketch() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );

    assert_eq!(
        doc.extrude_region(sid, region, 1.0),
        Err(DocumentError::UnknownSketch)
    );
}

/// `extrude_region_in_instance` itself refuses a WORLD sketch (never owned
/// by any definition) and a sketch owned by a DIFFERENT definition — the
/// symmetric half of the guard above.
#[test]
fn extrude_region_in_instance_refuses_a_world_sketch_and_a_foreign_definitions_sketch() {
    let mut doc = Document::new();
    let (_comp_a, inst_a) = component_with_instance(&mut doc);
    let (_comp_b, inst_b) = component_with_instance(&mut doc);

    let world_sketch = doc.add_sketch(ground());
    let world_region = draw_rect(
        &mut doc,
        world_sketch,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    assert_eq!(
        doc.extrude_region_in_instance(inst_a, world_sketch, world_region, 1.0),
        Err(DocumentError::UnknownSketch)
    );

    let (sid_b, _) = doc
        .begin_sketch_on_plane_in_instance(inst_b, ground())
        .expect("begin sketch in instance b");
    let region_b = draw_rect(
        &mut doc,
        sid_b,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    assert_eq!(
        doc.extrude_region_in_instance(inst_a, sid_b, region_b, 1.0),
        Err(DocumentError::UnknownSketch),
        "a sketch owned by a DIFFERENT definition is refused through instance A"
    );
}

/// Live-reproduced regression: `follow_me`'s PATH sketch is independent of
/// its PROFILE sketch/region, and only the profile was ownership-checked —
/// the path's `resolve_follow_me_path` chokepoint let a def-owned path
/// sketch's DEFINITION-LOCAL edges sweep straight into a WORLD solid. Pinned
/// with the exact live reproduction: an instance translated far from the
/// origin, so a coordinate-frame leak lands somewhere unmistakably wrong
/// rather than somewhere that could pass for a coincidence.
#[test]
fn world_follow_me_refuses_a_def_owned_path_sketch() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);
    doc.transform_instance(inst, &Transform::translation(Vec3::new(500.0, 500.0, 0.0)))
        .expect("place the instance far from the origin");

    // The def-owned PATH sketch: drawn through the instance, so its
    // coordinates are DEFINITION-local, not world.
    let (path_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    doc.sketch_mut(path_sketch)
        .expect("sketch is live")
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 2.0, 0.0))
        .expect("path segment");
    let edges: Vec<kernel::SketchEdgeId> = doc
        .sketch(path_sketch)
        .expect("sketch is live")
        .edges()
        .keys()
        .collect();

    // A perfectly ordinary WORLD profile, nowhere near the instance.
    let profile_sketch = doc.add_sketch(ground());
    let region = draw_rect(
        &mut doc,
        profile_sketch,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );

    let objects_before: HashSet<_> = doc.visible_object_ids().into_iter().collect();
    let result = doc.follow_me(
        profile_sketch,
        region,
        &kernel::FollowMePath::SketchEdges {
            sketch: path_sketch,
            edges,
        },
    );
    assert_eq!(
        result,
        Err(DocumentError::UnknownSketch),
        "a def-owned PATH sketch must refuse exactly like a def-owned PROFILE sketch"
    );
    let objects_after: HashSet<_> = doc.visible_object_ids().into_iter().collect();
    assert_eq!(
        objects_before, objects_after,
        "a refused sweep touches nothing (the document stays untouched)"
    );
}

// ------------------------------------------------- ownership bookkeeping

/// `sketch_ids` (the world-render list) excludes def-owned sketches, exactly
/// like `visible_object_ids` excludes definition-member objects;
/// `def_member_sketches` is the definition-scoped counterpart, mirroring
/// `def_members`. `sketch_owner_component` answers the ownership query
/// directly.
#[test]
fn def_owned_sketches_are_excluded_from_the_world_list_and_reachable_via_the_definition() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);
    let world_sketch = doc.add_sketch(ground());

    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");

    assert!(doc.sketch_ids().contains(&world_sketch));
    assert!(!doc.sketch_ids().contains(&def_sketch));
    assert_eq!(doc.sketch_owner_component(def_sketch), Some(comp));
    assert_eq!(doc.sketch_owner_component(world_sketch), None);
    assert_eq!(doc.def_member_sketches(comp), Some(vec![def_sketch]));
}

/// `Sketch::offset_region` (the Offset tool's kernel primitive) works
/// identically whether the sketch is world- or definition-owned — ownership
/// is document-level bookkeeping the `Sketch` value itself never sees, so
/// "widening" it is a non-event, proven directly rather than assumed.
#[test]
fn offset_region_works_identically_on_a_def_owned_sketch() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);

    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ],
    );
    let regions_before = doc.sketch(sid).unwrap().regions().len();
    let report = doc
        .sketch_mut(sid)
        .unwrap()
        .offset_region(region, 0.5)
        .expect("offset a def-owned sketch's region");
    assert!(
        !report.regions_created.is_empty(),
        "the offset produced new geometry"
    );
    assert_eq!(
        doc.sketch(sid).unwrap().regions().len(),
        regions_before + 1,
        "the offset region coexists with the original"
    );
}

// -------------------------------------------------- serialization round trip

/// A sketch's `SketchOwner` (manifest v13) round-trips through save/load:
/// the def-owned sketch stays owned by the (re-resolved) definition, and a
/// world sketch stays ownerless.
#[test]
fn sketch_owner_round_trips_through_save_load() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);
    let world_sketch = doc.add_sketch(ground());
    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    // Draw something so the sketch is not fresh-and-empty (a fresh sketch's
    // creation is untracked and would not survive save at all — irrelevant
    // to the ownership round trip, but keeps this spec realistic).
    draw_rect(
        &mut doc,
        def_sketch,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    draw_rect(
        &mut doc,
        world_sketch,
        [
            Point3::new(5.0, 0.0, 0.0),
            Point3::new(6.0, 0.0, 0.0),
            Point3::new(6.0, 1.0, 0.0),
            Point3::new(5.0, 1.0, 0.0),
        ],
    );

    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");

    // The world sketch is the only one in the world-render list.
    assert_eq!(
        loaded.sketch_ids().len(),
        1,
        "exactly the world sketch renders directly"
    );
    let loaded_world_sketch = loaded.sketch_ids()[0];
    assert_eq!(loaded.sketch_owner_component(loaded_world_sketch), None);
    assert_eq!(
        loaded.sketch(loaded_world_sketch).unwrap().regions().len(),
        1
    );

    // There is exactly one live definition, re-resolved through
    // `component_ids` (ids are dense-insertion-order stable across a
    // no-hidden-entries save/load, but re-resolving by identity rather than
    // by raw ordinal is the honest check).
    let comps = loaded.component_ids();
    assert_eq!(comps.len(), 1, "exactly one definition was saved");
    let loaded_comp = comps[0];
    let owned = loaded.def_member_sketches(loaded_comp).expect("live def");
    assert_eq!(owned.len(), 1, "the definition owns exactly one sketch");
    assert_eq!(
        loaded.sketch(owned[0]).unwrap().regions().len(),
        1,
        "the def-owned sketch's drawn region survived save/load"
    );
    assert_eq!(loaded.sketch_owner_component(owned[0]), Some(loaded_comp));
}

// ----------------------------------------- undo-of-fold orphan regressions
//
// Two fuzz-discovered regressions (document_fuzz.proptest-regressions), both
// `Document::save` panics from a def-owned sketch left pointing at a
// definition `Document::save`/`encode_document` no longer knows how to
// resolve. Pinned here directly, not just in the fuzz corpus: the bug class
// (an undo path that predates sketch ownership forgetting sketches exist)
// is exactly the kind of thing a future def-owning surface could
// reintroduce silently.

/// Undoing the very `make_component` that created a definition must also
/// hide any sketch drawn into it afterward (`begin_sketch_on_plane_in_instance`)
/// — otherwise the sketch stays "live" while its `SketchOwner` points at a
/// now-hidden definition, an unreachable/orphaned state `Document::save`
/// cannot serialize. Redoing the fold must bring the sketch back.
#[test]
fn undoing_make_component_hides_a_def_owned_sketch_drawn_afterward() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);

    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    assert!(doc.sketch(sid).is_some(), "sanity: the sketch is live");

    // Undo the extrude-worthy draw is unnecessary here (nothing was
    // extruded yet); undo the `make_component` itself directly.
    doc.undo().expect("undo make_component");
    assert!(
        doc.sketch(sid).is_none(),
        "the def-owned sketch must be hidden once its definition is dissolved"
    );
    assert!(
        doc.component_ids().is_empty(),
        "sanity: the definition is gone"
    );

    // The save path must not choke on the now-orphaned-but-hidden sketch.
    let bytes = doc.save();
    let reloaded = Document::load(&bytes).expect("save/load must not panic");
    assert!(reloaded.sketch_ids().is_empty());

    doc.redo().expect("redo make_component");
    assert!(
        doc.sketch(sid).is_some(),
        "redo brings the def-owned sketch back"
    );
    assert_eq!(doc.sketch_owner_component(sid), Some(comp));
}

/// The same orphan gap for `make_unique`: undoing it dissolves the PRIVATE
/// COPY's definition, which must also hide a sketch drawn into that copy.
#[test]
fn undoing_make_unique_hides_a_def_owned_sketch_drawn_into_the_copy() {
    let mut doc = Document::new();
    let (_comp, inst) = component_with_instance(&mut doc);

    let (new_def, _) = doc.make_unique(inst).expect("make_unique");
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in the unique copy");
    draw_rect(
        &mut doc,
        sid,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    assert_eq!(doc.sketch_owner_component(sid), Some(new_def));

    doc.undo().expect("undo make_unique");
    assert!(
        doc.sketch(sid).is_none(),
        "the def-owned sketch must be hidden once its private copy is dissolved"
    );

    let bytes = doc.save();
    Document::load(&bytes).expect("save/load must not panic");

    doc.redo().expect("redo make_unique");
    assert!(doc.sketch(sid).is_some(), "redo brings the sketch back");
    assert_eq!(doc.sketch_owner_component(sid), Some(new_def));
}

// --------------------------------------- make_unique / explode: sketch parity
//
// Closing the "KNOWN K1 SCOPE BOUNDARY" gap for real: `make_unique` and
// `explode_instance` used to only copy/bake a definition's extruded solid
// `members`, silently leaving a live, not-yet-extruded def-owned sketch
// behind on the ORIGINAL definition. That premise ("unreachable until app
// Phase A wires drawing into a component") no longer holds — the wasm and
// recording surfaces this effort's earlier specs exercise already reach it.

/// `make_unique` deep-copies the source definition's LIVE def-owned sketch
/// into the private copy — its own id, same def-local coordinates, and later
/// edits to one copy never leak into the other (the whole point of Make
/// Unique).
#[test]
fn make_unique_clones_the_source_definitions_live_def_owned_sketch() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);

    let (source_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let corners = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ];
    draw_rect(&mut doc, source_sketch, corners);
    let source_edges_before = doc.sketch(source_sketch).unwrap().edges().len();

    let (new_def, change) = doc.make_unique(inst).expect("make_unique");

    let cloned = *doc
        .def_member_sketches(new_def)
        .expect("live def")
        .iter()
        .find(|&&s| s != source_sketch)
        .expect("the copy owns its OWN cloned sketch, not the source's id");
    assert_eq!(doc.sketch_owner_component(cloned), Some(new_def));
    assert!(
        change.sketches_touched.contains(&cloned),
        "the forward DocChange must report the cloned sketch, or a private \
         copy's drafting surface stays snap-blind until an unrelated op \
         happens to touch it"
    );
    assert_eq!(
        doc.sketch_owner_component(source_sketch),
        Some(comp),
        "the source definition keeps its own sketch, untouched"
    );
    assert_eq!(
        doc.sketch(cloned).unwrap().edges().len(),
        source_edges_before,
        "def-local coordinates copy verbatim"
    );

    // Drawing further into the CLONE must never reach the source.
    draw_rect(
        &mut doc,
        cloned,
        [
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
        ],
    );
    assert_eq!(
        doc.sketch(source_sketch).unwrap().edges().len(),
        source_edges_before,
        "the source sketch is untouched by edits made to the clone"
    );

    // Undo is exact: the clone's definition dissolves and hides it; the
    // source is never touched. Redo brings it back live. Both DocChanges
    // must keep reporting the clone so inference un/re-registers it exactly
    // when it goes hidden/live, mirroring the forward assertion above.
    let undo_change = doc.undo().expect("undo make_unique");
    assert!(
        doc.sketch(cloned).is_none(),
        "the cloned sketch is hidden along with its dissolved private definition"
    );
    assert!(
        undo_change.sketches_touched.contains(&cloned),
        "undo's DocChange must report the clone so inference drops it"
    );
    assert_eq!(
        doc.sketch(source_sketch).unwrap().edges().len(),
        source_edges_before
    );

    let redo_change = doc.redo().expect("redo make_unique");
    assert!(doc.sketch(cloned).is_some(), "redo restores the clone live");
    assert_eq!(doc.sketch_owner_component(cloned), Some(new_def));
    assert!(
        redo_change.sketches_touched.contains(&cloned),
        "redo's DocChange must report the clone so inference re-registers it"
    );
}

/// `explode_instance` bakes a LIVE def-owned sketch into an independent WORLD
/// sketch through the instance's pose — the sketch analog of the member
/// bake — so a not-yet-extruded profile drawn into the component does not
/// silently disappear from the exploded result. The definition (and its
/// sketch) survive untouched for any sibling instance.
#[test]
fn explode_instance_bakes_a_live_def_owned_sketch_into_an_independent_world_sketch() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);
    // A sibling instance proves the definition's own sketch survives.
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();

    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.4)
        .unwrap()
        .then(&Transform::uniform_scale(2.0))
        .then(&Transform::translation(Vec3::new(10.0, -3.0, 5.0)));
    doc.transform_instance(inst, &pose).unwrap();

    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    // The def-owned sketch's plane is `ground()` mapped through pose⁻¹, so a
    // WORLD rectangle must itself be mapped through pose⁻¹ before drawing —
    // exactly like the pose⁻¹-mapping specs earlier in this file.
    let world_corners = world_rect_corners(Point3::new(2.0, 2.0, 0.0));
    let pose_inv = pose.inverse().unwrap();
    let local_corners = world_corners.map(|p| pose_inv.apply_point(p));
    draw_rect(&mut doc, def_sketch, local_corners);
    let expected_world: Vec<Point3> = doc
        .sketch(def_sketch)
        .unwrap()
        .vertices()
        .values()
        .map(|v| pose.apply_point(v.position))
        .collect();

    let world_sketches_before: HashSet<_> = doc.sketch_ids().into_iter().collect();
    let (created_objects, change) = doc.explode_instance(inst).expect("explode");
    assert_eq!(
        created_objects.len(),
        1,
        "the placeholder member baked as usual"
    );

    let new_sketches: Vec<_> = doc
        .sketch_ids()
        .into_iter()
        .filter(|s| !world_sketches_before.contains(s))
        .collect();
    assert_eq!(
        new_sketches.len(),
        1,
        "exactly one independent world sketch appeared"
    );
    let baked = new_sketches[0];
    assert_eq!(
        doc.sketch_owner_component(baked),
        None,
        "the baked sketch is genuinely world-owned, not left on the definition"
    );
    assert!(
        change.sketches_touched.contains(&baked),
        "the DocChange must report the newly baked sketch"
    );

    let mut got: Vec<Point3> = doc
        .sketch(baked)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    assert_eq!(got.len(), expected_world.len());
    for p in expected_world {
        let i = got
            .iter()
            .position(|&q| q.approx_eq(p, 1e-6))
            .expect("baked vertex must reproduce the pose-mapped world corner");
        got.swap_remove(i);
    }

    // The definition's OWN sketch is untouched — the sibling instance still
    // sees it.
    assert_eq!(doc.sketch_owner_component(def_sketch), Some(comp));
    assert!(doc.sketch(def_sketch).is_some());
    assert_eq!(doc.def_member_sketches(comp).unwrap(), vec![def_sketch]);

    // Undo is exact: the baked sketch hides, the definition/sibling are
    // unaffected either way.
    doc.undo().expect("undo explode");
    assert!(
        !doc.sketch_ids().contains(&baked),
        "undo hides the baked world sketch"
    );
    assert!(doc.instance_pose(sibling).is_some(), "sibling untouched");

    doc.redo().expect("redo explode");
    assert!(
        doc.sketch_ids().contains(&baked),
        "redo restores the baked world sketch"
    );
}

/// A non-uniformly-scaled instance cannot bake its definition's live
/// def-owned sketch (the map-or-drop contract would drop curve identity), so
/// `explode_instance` refuses typed BEFORE any mutation — not even the
/// ordinary member bake happens.
#[test]
fn explode_instance_refuses_a_non_uniform_pose_when_the_definition_holds_a_live_sketch() {
    let mut doc = Document::new();
    let (comp, inst) = component_with_instance(&mut doc);
    doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 3.0, 0.5)))
        .unwrap();

    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    draw_rect(
        &mut doc,
        def_sketch,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );

    let objects_before: HashSet<_> = doc.visible_object_ids().into_iter().collect();
    let sketches_before: HashSet<_> = doc.sketch_ids().into_iter().collect();
    assert_eq!(
        doc.explode_instance(inst),
        Err(DocumentError::CannotExplodeNonUniformScale)
    );
    assert_eq!(
        doc.visible_object_ids().into_iter().collect::<HashSet<_>>(),
        objects_before,
        "a refused explode bakes no member objects either (the strong exception guarantee)"
    );
    assert_eq!(
        doc.sketch_ids().into_iter().collect::<HashSet<_>>(),
        sketches_before,
        "a refused explode touches nothing"
    );
    assert!(doc.instance_pose(inst).is_some(), "the instance survives");
    assert_eq!(doc.sketch_owner_component(def_sketch), Some(comp));

    // The SAME non-uniform pose explodes fine once the def-owned sketch is
    // gone: the guard is specifically about baking a live sketch, not a
    // blanket non-uniform-scale refusal (member objects bake under
    // non-uniform scale in the pre-existing
    // `explode_bakes_pose_into_world_objects_and_refuses_mirror` contract
    // too — only a REFLECTED pose refuses there).
    doc.delete_sketch(def_sketch)
        .expect("delete the def-owned sketch");
    assert!(doc.explode_instance(inst).is_ok());
}
