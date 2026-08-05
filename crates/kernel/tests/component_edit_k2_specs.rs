// Test-only: HashSet is used for order-independent id-set comparisons, not
// kernel output — suppress the workspace clippy.toml ban for this
// integration-test crate (mirrors `component_edit_k1_specs.rs`).
#![allow(clippy::disallowed_types)]

//! Executable specs for component-edit-parity.md **phase K2**: the
//! object-op family (Follow Me, boolean, slice, per-member transform) inside
//! a component definition.
//!
//! The contract under test, per surface:
//! - `follow_me_in_instance` / `follow_me_merged_in_instance` /
//!   `follow_me_face_in_instance` mirror the world Follow Me family entirely
//!   in DEFINITION-local space: profile and path must both belong to the
//!   SAME definition as the calling instance, or the call refuses typed
//!   (world↔definition mixing, foreign-definition mixing, and the
//!   world-only `InstanceFaceLoop` path variant are all refused); the
//!   result is born as a new member, seen by every instance at once; an
//!   optional partial-sweep `stop_len` is a WORLD-space arc length mapped
//!   through the instance's pose, refusing `AmbiguousInstanceScale` under a
//!   non-uniform scale exactly like `extrude_region_in_instance`'s
//!   `distance`;
//! - `boolean_in_component` combines two members of the SAME definition,
//!   replacing them with a new member; cross-ownership operands (world, or
//!   a different definition) refuse typed;
//! - `slice_def_member` cuts a member by a WORLD-space plane mapped through
//!   the instance's pose⁻¹ — unlike a scalar distance, a plane maps
//!   unambiguously under ANY invertible pose, including non-uniform scale;
//! - `transform_def_member` bakes a WORLD-space gesture into a member by
//!   conjugating it through the instance's pose; a full affine conjugation
//!   is never ambiguous (unlike a scalar), so this never refuses on account
//!   of the instance's own scale — only on a singular/reflecting gesture,
//!   exactly like `transform_object`;
//! - every surface's `DocChange` (forward, undo, redo) names the component
//!   and every sibling instance, not just the one edited through.

use kernel::{
    BooleanOp, ComponentId, Document, DocumentError, FaceId, InstanceId, KernelOp, KernelOpReport,
    NodeId, Object, ObjectId, Plane, Point3, SketchEdgeId, SketchId, Transform, Vec3,
    WatertightState,
};
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

/// A vertical plane at `x`, spanning y/z — the follow-me profile plane
/// convention `document_specs.rs` uses.
fn profile_plane_x(x: f64) -> Plane {
    Plane::from_polygon(&[
        Point3::new(x, 0.0, 0.0),
        Point3::new(x, 1.0, 0.0),
        Point3::new(x, 0.0, 1.0),
    ])
    .expect("vertical plane is well-defined")
}

/// Draws an axis-aligned rectangle directly into `s` (bypassing gestures —
/// undo grouping is not what these specs are about), returning its sole
/// region.
fn draw_rect(doc: &mut Document, s: SketchId, corners: [Point3; 4]) -> kernel::SketchRegionId {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for i in 0..4 {
        sk.add_segment(corners[i], corners[(i + 1) % 4])
            .expect("rectangle segment");
    }
    doc.extrudable_regions(s).expect("sketch is live")[0]
}

/// Draws a square profile on the x = `x` plane spanning `[y0, y1] x [z0,
/// z1]` — `document_specs.rs`'s `draw_profile_rect`, duplicated (integration
/// test binaries share no code).
fn draw_profile_rect(doc: &mut Document, s: SketchId, x: f64, y0: f64, z0: f64, y1: f64, z1: f64) {
    draw_rect(
        doc,
        s,
        [
            Point3::new(x, y0, z0),
            Point3::new(x, y1, z0),
            Point3::new(x, y1, z1),
            Point3::new(x, y0, z1),
        ],
    );
}

/// The single region of a sketch (panics if there isn't exactly one).
fn only_region(doc: &Document, s: SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// Builds a 4×2×1 box (x:0..4, y:0..2, z:0..1 — `document_specs.rs`'s
/// `boxed_document` box) directly in world space and folds it into a
/// component via `make_component`. Returns `(component, identity-posed
/// instance, member)`.
fn boxed_component(doc: &mut Document) -> (ComponentId, InstanceId, ObjectId) {
    let gs = doc.add_sketch(ground());
    let region = draw_rect(
        doc,
        gs,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ],
    );
    let (id, _) = doc.extrude_region(gs, region, 1.0).expect("extrude box");
    let (comp, inst, _) = doc
        .make_component(&[NodeId::Object(id)])
        .expect("make_component");
    let member = doc.def_members(comp).expect("live component")[0];
    (comp, inst, member)
}

/// A classic circle/rectangle-on-face push past the far wall must use the
/// same subtracting through-cut semantics inside a component as it does on a
/// world object. The original component wasm route skipped overshoot
/// detection, sent the sub-face to wall-building extrusion, and refused with
/// `NonManifoldResult` when those walls reached the opposite face.
#[test]
fn component_subface_push_through_cuts_every_instance_and_replays() {
    let mut doc = Document::new();
    let (component, instance, member) = boxed_component(&mut doc);
    let sibling = doc
        .duplicate_node(NodeId::Instance(instance), &Transform::IDENTITY)
        .expect("duplicate instance")
        .0;
    let sibling = match sibling {
        NodeId::Instance(id) => id,
        other => panic!("expected instance, got {other:?}"),
    };
    let top = face_matching(&doc, member, Vec3::new(0.0, 0.0, 1.0));
    let sub_face = match doc
        .apply_def_op(
            component,
            member,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path: vec![
                    Point3::new(1.0, 0.5, 1.0),
                    Point3::new(3.0, 0.5, 1.0),
                    Point3::new(3.0, 1.5, 1.0),
                    Point3::new(1.0, 1.5, 1.0),
                ],
                restore: None,
                curve: None,
            },
        )
        .expect("imprint member")
        .0
    {
        KernelOpReport::FaceSplitInner(report) => report.sub_face,
        other => panic!("unexpected report: {other:?}"),
    };

    assert!(
        doc.object(member)
            .expect("member")
            .push_pull_overshoots(sub_face, -1.5),
        "fixture must exercise the through-cut route"
    );
    let (results, change) = doc
        .push_pull_through_in_component(component, member, sub_face, -1.5)
        .expect("cut through the component member");
    assert_eq!(results.len(), 1, "a hole leaves one connected ring");
    assert_eq!(change.components_touched, vec![component]);
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        HashSet::from([instance, sibling])
    );
    assert!(doc.object(member).is_none(), "source is consumed");
    assert!(doc.object(results[0]).is_some(), "ring result is live");

    let undo = doc.undo().expect("undo through-cut");
    assert_eq!(undo.components_touched, vec![component]);
    assert_eq!(
        undo.instances_touched.into_iter().collect::<HashSet<_>>(),
        HashSet::from([instance, sibling])
    );
    assert!(doc.object(member).is_some(), "undo restores source");
    assert!(doc.object(results[0]).is_none(), "undo hides result");

    let redo = doc.redo().expect("redo through-cut");
    assert_eq!(redo.components_touched, vec![component]);
    assert_eq!(
        redo.instances_touched.into_iter().collect::<HashSet<_>>(),
        HashSet::from([instance, sibling])
    );
    assert!(doc.object(member).is_none(), "redo consumes source");
    assert!(doc.object(results[0]).is_some(), "redo restores result");

    let (unique, _) = doc.make_unique(sibling).expect("make cut instance unique");
    assert_eq!(
        doc.def_members(unique).unwrap().len(),
        1,
        "Make Unique must not resurrect the hidden pre-cut source"
    );
    let (exploded, _) = doc
        .explode_instance(instance)
        .expect("explode cut instance");
    assert_eq!(
        exploded.len(),
        1,
        "Explode must copy only the live ring, not the hidden pre-cut source"
    );
}

#[test]
fn definition_sketch_transform_stays_owned_and_updates_every_instance() {
    let mut doc = Document::new();
    let (component, instance, _) = boxed_component(&mut doc);
    let sibling = match doc
        .duplicate_node(NodeId::Instance(instance), &Transform::IDENTITY)
        .expect("duplicate instance")
        .0
    {
        NodeId::Instance(id) => id,
        other => panic!("expected instance, got {other:?}"),
    };
    let (sketch, _) = doc
        .begin_sketch_on_plane_in_instance(instance, ground())
        .expect("definition sketch");
    draw_rect(
        &mut doc,
        sketch,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    let before: Vec<_> = doc
        .sketch(sketch)
        .expect("sketch")
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    let change = doc
        .transform_def_sketch(
            instance,
            sketch,
            &Transform::translation(Vec3::new(2.0, 0.0, 0.0)),
        )
        .expect("move definition sketch");
    assert_eq!(doc.sketch_owner_component(sketch), Some(component));
    assert_eq!(change.components_touched, vec![component]);
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        HashSet::from([instance, sibling])
    );
    let after: Vec<_> = doc
        .sketch(sketch)
        .expect("sketch")
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    assert!(
        before
            .iter()
            .zip(&after)
            .all(|(a, b)| b.approx_eq(*a + Vec3::new(2.0, 0.0, 0.0), 1e-9))
    );

    let undo = doc.undo().expect("undo sketch transform");
    assert_eq!(undo.components_touched, vec![component]);
    assert_eq!(
        undo.instances_touched.into_iter().collect::<HashSet<_>>(),
        HashSet::from([instance, sibling])
    );
    let restored: Vec<_> = doc
        .sketch(sketch)
        .expect("sketch")
        .vertices()
        .values()
        .map(|v| v.position)
        .collect();
    assert_eq!(restored, before);
}

#[test]
fn definition_selection_transform_is_atomic_and_one_undo_step() {
    let mut doc = Document::new();
    let s1 = doc.add_sketch(ground());
    let r1 = draw_rect(
        &mut doc,
        s1,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    let a = doc.extrude_region(s1, r1, 1.0).unwrap().0;
    let s2 = doc.add_sketch(ground());
    let r2 = draw_rect(
        &mut doc,
        s2,
        [
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
        ],
    );
    let b = doc.extrude_region(s2, r2, 1.0).unwrap().0;
    let (_, instance, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let members = match doc.instance_def(instance) {
        Some(component) => doc.def_members(component).unwrap(),
        None => panic!("live instance"),
    };
    let before = doc.state_hash();
    doc.transform_def_selection(
        instance,
        &members,
        &[],
        &[],
        &Transform::translation(Vec3::new(0.0, 0.0, 2.0)),
    )
    .expect("compound transform");
    let after = doc.state_hash();
    assert_ne!(after, before);
    doc.undo().expect("one undo restores the whole selection");
    assert_eq!(doc.state_hash(), before);
    doc.redo().expect("one redo reapplies the whole selection");
    assert_eq!(doc.state_hash(), after);

    let world_sketch = doc.add_sketch(ground());
    let world_region = draw_rect(
        &mut doc,
        world_sketch,
        [
            Point3::new(5.0, 0.0, 0.0),
            Point3::new(6.0, 0.0, 0.0),
            Point3::new(6.0, 1.0, 0.0),
            Point3::new(5.0, 1.0, 0.0),
        ],
    );
    let world = doc
        .extrude_region(world_sketch, world_region, 1.0)
        .unwrap()
        .0;
    let before_refusal = doc.state_hash();
    let err = doc
        .transform_def_selection(
            instance,
            &[members[0], world],
            &[],
            &[],
            &Transform::translation(Vec3::new(1.0, 0.0, 0.0)),
        )
        .expect_err("foreign target must refuse");
    assert_eq!(err, DocumentError::UnknownObject);
    assert_eq!(
        doc.state_hash(),
        before_refusal,
        "a later invalid target must roll back earlier targets"
    );
}

/// Extrudes a box member through `inst` at its CURRENT pose — the caller is
/// responsible for pre-mapping `x0/y0/x1/y1` through pose⁻¹ if `inst` is not
/// at the identity pose (every helper caller in this file uses an
/// identity-posed instance, so no mapping is needed in practice; the general
/// pose⁻¹ correctness is proven separately by the dedicated pose specs
/// below).
fn extrude_box_in_instance(
    doc: &mut Document,
    inst: InstanceId,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    height: f64,
) -> ObjectId {
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin sketch in instance");
    let region = draw_rect(
        doc,
        sid,
        [
            Point3::new(x0, y0, 0.0),
            Point3::new(x1, y0, 0.0),
            Point3::new(x1, y1, 0.0),
            Point3::new(x0, y1, 0.0),
        ],
    );
    doc.extrude_region_in_instance(inst, sid, region, height)
        .expect("extrude member in instance")
        .0
}

/// The face of a live object whose outward normal most nearly matches
/// `dir` (dot product closest to 1) — a robust "pick the top/side/bottom
/// face" selector, avoiding a brittle exact-component threshold when a
/// member's local frame has been transformed.
fn face_matching(doc: &Document, object: ObjectId, dir: Vec3) -> FaceId {
    let obj = doc.object(object).expect("object is live");
    obj.faces()
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.plane
                .normal()
                .dot(dir)
                .partial_cmp(&b.plane.normal().dot(dir))
                .expect("finite dot products")
        })
        .map(|(id, _)| id)
        .expect("object has faces")
}

/// Among the faces of `object` whose normal most nearly matches `dir` (tied
/// within `1e-6`), the one with the SMALLEST (`smallest: true`) or LARGEST
/// bounding-box extent — disambiguates two coplanar-normal faces at
/// different heights (e.g. a small boss cap vs. the big roof it pokes
/// through), which `face_matching` alone cannot tell apart (both score a
/// perfect dot product).
fn face_matching_by_extent(doc: &Document, object: ObjectId, dir: Vec3, smallest: bool) -> FaceId {
    let obj = doc.object(object).expect("object is live");
    let best_dot = obj
        .faces()
        .values()
        .map(|f| f.plane.normal().dot(dir))
        .fold(f64::MIN, f64::max);
    obj.faces()
        .iter()
        .filter(|(_, f)| (f.plane.normal().dot(dir) - best_dot).abs() < 1e-6)
        .map(|(id, f)| {
            let pts: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
            let (mut lo, mut hi) = (pts[0], pts[0]);
            for p in &pts {
                lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
            }
            let extent = (hi.x - lo.x).max(hi.y - lo.y).max(hi.z - lo.z);
            (id, extent)
        })
        .max_by(|(_, a), (_, b)| {
            let (a, b) = if smallest { (*b, *a) } else { (*a, *b) };
            a.partial_cmp(&b).expect("finite extents")
        })
        .map(|(id, _)| id)
        .expect("a face matching direction exists")
}

/// The unique vertex positions of `obj` (topology-order, not per-face
/// flattened) — `component_edit_k1_specs.rs`'s helper of the same name.
fn vertex_positions(obj: &Object) -> Vec<Point3> {
    obj.to_polygons().0
}

/// Whether mapping every point of `local` through `pose` reproduces exactly
/// the vertex set of `expected_world` (each point matched once, any order) —
/// winding-agnostic, so it tolerates a mirrored pose's reversed winding.
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
/// cyclic rotation of each face loop (but not reversal).
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

// =========================================================================
// follow_me_in_instance — pose⁻¹ mapping
// =========================================================================

/// Drawing a profile AND a path through a ROTATED + UNIFORMLY-SCALED
/// instance and sweeping reproduces, once mapped forward through the same
/// pose, exactly the solid a plain world `follow_me`/`follow_me_to` would
/// have built from the same WORLD rectangle, path, and (for the partial
/// sweep) stop length — the pose⁻¹ round trip for `follow_me_in_instance`,
/// covering both the full and the partial sweep.
#[test]
fn follow_me_in_instance_round_trips_through_a_rotated_scaled_pose() {
    let mut doc = Document::new();
    let (_comp, inst, _placeholder) = boxed_component(&mut doc);

    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::uniform_scale(2.0))
        .then(&Transform::translation(Vec3::new(10.0, -3.0, 5.0)));
    doc.transform_instance(inst, &pose).unwrap();
    let pose_inv = pose.inverse().unwrap();

    // World-space profile (a square on the x = 0 plane, offset far from the
    // origin) and an L-shaped world-space path on the ground.
    let profile_corners_world = [
        Point3::new(20.0, -0.3, -0.3),
        Point3::new(20.0, 0.3, -0.3),
        Point3::new(20.0, 0.3, 0.3),
        Point3::new(20.0, -0.3, 0.3),
    ];
    let path_world = [
        Point3::new(20.0, 0.0, 0.0),
        Point3::new(22.0, 0.0, 0.0),
        Point3::new(22.0, 2.0, 0.0),
    ];
    let world_stop = 2.5;

    // Expected: the plain world sweep, independent of instance machinery.
    let wp = Plane::from_polygon(&[
        Point3::new(20.0, 0.0, 0.0),
        Point3::new(20.0, 1.0, 0.0),
        Point3::new(20.0, 0.0, 1.0),
    ])
    .unwrap();
    let ws1 = doc.add_sketch(wp);
    let wregion1 = draw_rect(&mut doc, ws1, profile_corners_world);
    let wgs = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(wgs).unwrap();
        sk.add_segment(path_world[0], path_world[1]).unwrap();
        sk.add_segment(path_world[1], path_world[2]).unwrap();
    }
    let wedges: Vec<SketchEdgeId> = doc.sketch(wgs).unwrap().edges().keys().collect();
    let wpath = kernel::FollowMePath::SketchEdges {
        sketch: wgs,
        edges: wedges,
    };
    let (expected_id, _) = doc.follow_me(ws1, wregion1, &wpath).expect("world sweep");
    let expected = doc.object(expected_id).unwrap().clone();
    // A FRESH profile sketch for the partial sweep: the first `follow_me`
    // call fully consumed `ws1`'s scaffolding (Model D) — the sketch itself
    // ceased to exist, so it cannot be redrawn into.
    let ws2 = doc.add_sketch(wp);
    let wregion2 = draw_rect(&mut doc, ws2, profile_corners_world);
    let (expected_partial_id, _) = doc
        .follow_me_to(ws2, wregion2, &wpath, world_stop)
        .expect("world partial sweep");
    let expected_partial = doc.object(expected_partial_id).unwrap().clone();

    // The same shapes, through the instance: every drawn point is mapped
    // through pose⁻¹ before being added to the def-local sketch.
    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, wp)
        .expect("begin profile sketch in instance");
    let local_profile = profile_corners_world.map(|p| pose_inv.apply_point(p));
    let pregion = draw_rect(&mut doc, psid, local_profile);

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch in instance");
    let local_path = path_world.map(|p| pose_inv.apply_point(p));
    {
        let sk = doc.sketch_mut(gsid).unwrap();
        sk.add_segment(local_path[0], local_path[1]).unwrap();
        sk.add_segment(local_path[1], local_path[2]).unwrap();
    }
    let gedges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges: gedges,
    };

    let (member_id, change) = doc
        .follow_me_in_instance(inst, psid, pregion, &path, None)
        .expect("full sweep in instance");
    let member = doc.object(member_id).unwrap().clone();
    assert!(
        mapped_vertices_match(&member, &pose, &expected),
        "the def-local sweep, mapped through the instance pose, must reproduce \
         the world sweep of the same corners/path"
    );
    assert_eq!(change.components_touched, vec![_comp]);
    assert_eq!(change.instances_touched, vec![inst]);

    // Partial sweep: `stop_len` is world-space, mapped through the pose's
    // uniform scale to the local arc length. A FRESH profile sketch again —
    // `psid` was fully consumed by the full sweep above.
    let (psid2, _) = doc
        .begin_sketch_on_plane_in_instance(inst, wp)
        .expect("begin second profile sketch in instance");
    let pregion2 = draw_rect(&mut doc, psid2, local_profile);
    let (partial_id, _) = doc
        .follow_me_in_instance(inst, psid2, pregion2, &path, Some(world_stop))
        .expect("partial sweep in instance");
    let partial = doc.object(partial_id).unwrap().clone();
    assert!(
        mapped_vertices_match(&partial, &pose, &expected_partial),
        "the def-local PARTIAL sweep, mapped through the pose, must reproduce \
         the world partial sweep at the same world stop length"
    );
}

/// The same round trip through a MIRRORED instance (negative determinant,
/// still a uniform-scale similarity): the swept solid's vertices must
/// reproduce the world corners exactly when mapped back through the pose,
/// regardless of the mirror.
#[test]
fn follow_me_in_instance_round_trips_through_a_mirrored_pose() {
    let mut doc = Document::new();
    let (_comp, inst, _placeholder) = boxed_component(&mut doc);

    let pose = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(7.0, 4.0, 0.0)));
    assert!(pose.determinant() < 0.0, "sanity: this pose mirrors");
    doc.transform_instance(inst, &pose).unwrap();
    let pose_inv = pose.inverse().unwrap();

    let profile_corners_world = [
        Point3::new(0.0, -0.3, -0.3),
        Point3::new(0.0, 0.3, -0.3),
        Point3::new(0.0, 0.3, 0.3),
        Point3::new(0.0, -0.3, 0.3),
    ];
    let path_world = [Point3::new(0.0, 0.0, 0.0), Point3::new(3.0, 0.0, 0.0)];

    let wp = profile_plane_x(0.0);
    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, wp)
        .expect("begin profile sketch in instance");
    let local_profile = profile_corners_world.map(|p| pose_inv.apply_point(p));
    let pregion = draw_rect(&mut doc, psid, local_profile);

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch in instance");
    let local_path = path_world.map(|p| pose_inv.apply_point(p));
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(local_path[0], local_path[1])
        .unwrap();
    let gedges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges: gedges,
    };

    let (member_id, _) = doc
        .follow_me_in_instance(inst, psid, pregion, &path, None)
        .expect("sweep through a mirrored pose");
    let member = doc.object(member_id).unwrap().clone();

    let mapped: Vec<Point3> = vertex_positions(&member)
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    for &corner in &profile_corners_world {
        assert!(
            mapped.iter().any(|&p| p.approx_eq(corner, 1e-6)),
            "world corner {corner:?} must reappear among the mapped member's vertices"
        );
    }
}

/// A non-uniformly-scaled instance cannot map a typed WORLD-space `stop_len`
/// unambiguously, so `follow_me_in_instance` refuses typed for a PARTIAL
/// sweep — but the FULL sweep (no scalar length involved) succeeds under the
/// exact same pose, since the profile/path themselves are point mappings,
/// never ambiguous under non-uniform scale.
#[test]
fn follow_me_in_instance_refuses_a_typed_stop_len_under_non_uniform_scale() {
    let mut doc = Document::new();
    let (comp, inst, _placeholder) = boxed_component(&mut doc);
    doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 3.0, 0.5)))
        .unwrap();

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(0.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.0, -0.2, -0.2, 0.2, 0.2);
    let pregion = only_region(&doc, psid);

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    let members_before = doc.def_members(comp).unwrap();
    let refused = doc.follow_me_in_instance(inst, psid, pregion, &path, Some(1.0));
    assert_eq!(refused, Err(DocumentError::AmbiguousInstanceScale));
    assert_eq!(
        doc.def_members(comp).unwrap(),
        members_before,
        "a refused partial sweep touches nothing"
    );

    // The SAME profile/path succeed as a FULL sweep under the same pose.
    let ok = doc.follow_me_in_instance(inst, psid, pregion, &path, None);
    assert!(
        ok.is_ok(),
        "a full sweep has no scalar to be ambiguous about"
    );
}

// =========================================================================
// follow_me_in_instance — propagation, undo/redo
// =========================================================================

/// Sweeping through ONE instance is a shared-geometry birth: the new member
/// shows up for `def_members`, and the returned `DocChange` names EVERY
/// instance of the definition.
#[test]
fn follow_me_in_instance_propagates_to_every_sibling_instance() {
    let mut doc = Document::new();
    let (comp, i1, _placeholder) = boxed_component(&mut doc);
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let (i3, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(-20.0, 0.0, 0.0)))
        .unwrap();

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(i1, profile_plane_x(0.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.0, -0.2, -0.2, 0.2, 0.2);
    let pregion = only_region(&doc, psid);
    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(i1, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    let (member_id, change) = doc
        .follow_me_in_instance(i1, psid, pregion, &path, None)
        .expect("sweep in instance");
    assert!(doc.def_members(comp).unwrap().contains(&member_id));
    let touched: HashSet<_> = change.instances_touched.into_iter().collect();
    assert_eq!(touched, [i1, i2, i3].into_iter().collect());
}

/// Undo/redo of `follow_me_in_instance` restore/re-apply exactly, and both
/// directions' `DocChange` name every sibling instance.
#[test]
fn follow_me_in_instance_undo_redo_round_trips() {
    let mut doc = Document::new();
    let (comp, inst, _placeholder) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let members_before = doc.def_members(comp).unwrap();

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(0.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.0, -0.2, -0.2, 0.2, 0.2);
    let pregion = only_region(&doc, psid);
    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    let (member_id, _) = doc
        .follow_me_in_instance(inst, psid, pregion, &path, None)
        .expect("sweep in instance");
    let shape = doc.object(member_id).unwrap().clone();

    let undo_change = doc.undo().expect("undo the sweep");
    assert!(doc.object(member_id).is_none(), "the member is hidden");
    assert_eq!(
        doc.def_members(comp).unwrap(),
        {
            let mut m = members_before.clone();
            m.push(member_id);
            m
        },
        "the member stays listed (hidden)"
    );
    assert_eq!(undo_change.components_touched, vec![comp]);
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let redo_change = doc.redo().expect("redo the sweep");
    assert!(doc.object(member_id).is_some());
    assert!(objects_equivalent(&shape, doc.object(member_id).unwrap()));
    assert_eq!(redo_change.components_touched, vec![comp]);
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

// =========================================================================
// follow_me_in_instance — ownership refusals (world↔definition mixing)
// =========================================================================

/// The profile sketch must belong to the calling instance's OWN definition:
/// a WORLD sketch and a sketch owned by a DIFFERENT definition both refuse.
#[test]
fn follow_me_in_instance_refuses_a_world_or_foreign_profile_sketch() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _) = boxed_component(&mut doc);
    let (_comp_b, inst_b, _) = boxed_component(&mut doc);

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    // A WORLD profile sketch.
    let world_ps = doc.add_sketch(profile_plane_x(0.0));
    draw_profile_rect(&mut doc, world_ps, 0.0, -0.2, -0.2, 0.2, 0.2);
    let world_region = only_region(&doc, world_ps);
    assert_eq!(
        doc.follow_me_in_instance(inst_a, world_ps, world_region, &path, None),
        Err(DocumentError::UnknownSketch)
    );

    // A profile sketch owned by a DIFFERENT definition (B).
    let (foreign_ps, _) = doc
        .begin_sketch_on_plane_in_instance(inst_b, profile_plane_x(0.0))
        .expect("begin profile in b");
    draw_profile_rect(&mut doc, foreign_ps, 0.0, -0.2, -0.2, 0.2, 0.2);
    let foreign_region = only_region(&doc, foreign_ps);
    assert_eq!(
        doc.follow_me_in_instance(inst_a, foreign_ps, foreign_region, &path, None),
        Err(DocumentError::UnknownSketch)
    );
}

/// The path, independently of the profile, must also belong to the SAME
/// definition: a WORLD path sketch and one owned by a DIFFERENT definition
/// both refuse.
#[test]
fn follow_me_in_instance_refuses_a_world_or_foreign_path_sketch() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _) = boxed_component(&mut doc);
    let (_comp_b, inst_b, _) = boxed_component(&mut doc);

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, profile_plane_x(0.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.0, -0.2, -0.2, 0.2, 0.2);
    let pregion = only_region(&doc, psid);

    // A WORLD path sketch.
    let world_gs = doc.add_sketch(ground());
    doc.sketch_mut(world_gs)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let world_edges: Vec<SketchEdgeId> = doc.sketch(world_gs).unwrap().edges().keys().collect();
    let world_path = kernel::FollowMePath::SketchEdges {
        sketch: world_gs,
        edges: world_edges,
    };
    assert_eq!(
        doc.follow_me_in_instance(inst_a, psid, pregion, &world_path, None),
        Err(DocumentError::UnknownSketch)
    );

    // A path sketch owned by a DIFFERENT definition (B).
    let (foreign_gs, _) = doc
        .begin_sketch_on_plane_in_instance(inst_b, ground())
        .expect("begin path in b");
    doc.sketch_mut(foreign_gs)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let foreign_edges: Vec<SketchEdgeId> = doc.sketch(foreign_gs).unwrap().edges().keys().collect();
    let foreign_path = kernel::FollowMePath::SketchEdges {
        sketch: foreign_gs,
        edges: foreign_edges,
    };
    assert_eq!(
        doc.follow_me_in_instance(inst_a, psid, pregion, &foreign_path, None),
        Err(DocumentError::UnknownSketch)
    );
}

/// A `FaceLoop` path object must be a live member of the SAME definition: a
/// WORLD object and a member of a DIFFERENT definition both refuse.
#[test]
fn follow_me_in_instance_refuses_a_world_or_foreign_face_loop_path() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, profile_plane_x(2.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 2.0, -0.2, 0.8, 0.2, 1.2);
    let pregion = only_region(&doc, psid);

    // A WORLD object.
    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(11.0, 0.0, 0.0),
                Point3::new(11.0, 1.0, 0.0),
                Point3::new(10.0, 1.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let world_face = face_matching(&doc, world_box, Vec3::new(0.0, 0.0, 1.0));
    let world_path = kernel::FollowMePath::FaceLoop {
        object: world_box,
        face: world_face,
    };
    assert_eq!(
        doc.follow_me_in_instance(inst_a, psid, pregion, &world_path, None),
        Err(DocumentError::UnknownObject)
    );

    // A member of a DIFFERENT definition.
    let foreign_face = face_matching(&doc, member_b, Vec3::new(0.0, 0.0, 1.0));
    let foreign_path = kernel::FollowMePath::FaceLoop {
        object: member_b,
        face: foreign_face,
    };
    assert_eq!(
        doc.follow_me_in_instance(inst_a, psid, pregion, &foreign_path, None),
        Err(DocumentError::UnknownObject)
    );
}

/// `InstanceFaceLoop` reaches through ANOTHER instance's pose into WORLD
/// space by construction, so it can never resolve inside a definition —
/// refused typed unconditionally.
#[test]
fn follow_me_in_instance_refuses_an_instance_face_loop_path() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, inst_b, member_b) = boxed_component(&mut doc);

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, profile_plane_x(2.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 2.0, -0.2, 0.8, 0.2, 1.2);
    let pregion = only_region(&doc, psid);

    let face = face_matching(&doc, member_b, Vec3::new(0.0, 0.0, 1.0));
    let path = kernel::FollowMePath::InstanceFaceLoop {
        instance: inst_b,
        object: member_b,
        face,
    };
    assert_eq!(
        doc.follow_me_in_instance(inst_a, psid, pregion, &path, None),
        Err(DocumentError::UnknownInstance)
    );
}

/// Sweeping a profile region around a SIBLING member's face loop leaves that
/// member entirely untouched — a plain (non-merging) sweep, mirroring the
/// world `follow_me_around_a_face_loop_leaves_the_solid_untouched` contract
/// inside a definition.
#[test]
fn follow_me_around_a_member_face_loop_in_instance_leaves_the_member_untouched() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let top = face_matching(&doc, member, Vec3::new(0.0, 0.0, 1.0));

    // Profile straddling the top rim from outside, at x = 0.5 (crossing the
    // y = 0 boundary edge mid-span) — the exact `document_specs.rs` shape.
    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(0.5))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.5, -0.3, 0.9, -0.05, 1.15);
    let pregion = only_region(&doc, psid);
    let path = kernel::FollowMePath::FaceLoop {
        object: member,
        face: top,
    };

    let (ring_id, change) = doc
        .follow_me_in_instance(inst, psid, pregion, &path, None)
        .expect("sweep around a member face loop");
    let ring = doc.object(ring_id).unwrap();
    assert_eq!(ring.watertight(), WatertightState::Watertight);
    assert!(
        doc.object(member).unwrap().faces().len() == 6,
        "the path member is untouched"
    );
    assert_eq!(change.components_touched, vec![comp]);
}

// =========================================================================
// follow_me_merged_in_instance
// =========================================================================

/// A standing profile rim, half inside a member, sweeps around that
/// member's OWN face loop and MERGES with it in one gesture: the base
/// member is consumed, a new merged member exists, and undo restores both
/// in one step — mirroring `follow_me_merged_consumes_the_path_solid_in_
/// one_undo_step` inside a definition.
#[test]
fn follow_me_merged_in_instance_carves_the_path_member_in_one_step() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let top = face_matching(&doc, member, Vec3::new(0.0, 0.0, 1.0));

    // Mid-edge on the y = 0 rim, half inside the box (the world spec's
    // exact shape): plane x = 2 crosses the (0,0,1)->(4,0,1) top edge.
    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(2.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 2.0, -0.2, 0.8, 0.2, 1.2);
    let pregion = only_region(&doc, psid);
    let path = kernel::FollowMePath::FaceLoop {
        object: member,
        face: top,
    };

    let (merged, change) = doc
        .follow_me_merged_in_instance(inst, psid, pregion, &path, None)
        .expect("rim profile sweeps and merges");
    assert!(doc.object(member).is_none(), "base consumed");
    assert!(doc.object(merged).is_some());
    assert_eq!(
        doc.object(merged).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(doc.def_members(comp).unwrap().contains(&merged));
    assert_eq!(change.components_touched, vec![comp]);

    doc.undo().expect("undo the merged sweep");
    assert!(doc.object(member).is_some(), "base restored");
    assert!(doc.object(merged).is_none());
    assert_eq!(
        doc.sketch(psid)
            .expect("profile scaffolding restored")
            .edges()
            .len(),
        4
    );

    doc.redo().expect("redo the merged sweep");
    assert!(doc.object(member).is_none());
    assert!(doc.object(merged).is_some());
}

/// An edge path has no solid to merge with: `follow_me_merged_in_instance`
/// refuses typed rather than silently doing a plain (non-merging) sweep.
#[test]
fn follow_me_merged_in_instance_refuses_a_non_face_loop_path() {
    let mut doc = Document::new();
    let (_comp, inst, _member) = boxed_component(&mut doc);

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(0.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 0.0, -0.2, -0.2, 0.2, 0.2);
    let pregion = only_region(&doc, psid);
    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    assert_eq!(
        doc.follow_me_merged_in_instance(inst, psid, pregion, &path, None),
        Err(DocumentError::UnknownObject)
    );
}

/// The merge target must be a member of the SAME definition as the calling
/// instance — a `FaceLoop` on a DIFFERENT definition's member refuses.
#[test]
fn follow_me_merged_in_instance_refuses_a_foreign_definitions_member() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, profile_plane_x(2.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 2.0, -0.2, 0.8, 0.2, 1.2);
    let pregion = only_region(&doc, psid);
    let foreign_top = face_matching(&doc, member_b, Vec3::new(0.0, 0.0, 1.0));
    let path = kernel::FollowMePath::FaceLoop {
        object: member_b,
        face: foreign_top,
    };

    assert_eq!(
        doc.follow_me_merged_in_instance(inst_a, psid, pregion, &path, None),
        Err(DocumentError::UnknownObject)
    );
}

/// A merged sweep's `DocChange` — forward, undo, and redo — names every
/// sibling instance of the definition, not just the one merged through.
#[test]
fn follow_me_merged_in_instance_propagates_to_every_sibling_instance() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let top = face_matching(&doc, member, Vec3::new(0.0, 0.0, 1.0));

    let (psid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, profile_plane_x(2.0))
        .expect("begin profile sketch");
    draw_profile_rect(&mut doc, psid, 2.0, -0.2, 0.8, 0.2, 1.2);
    let pregion = only_region(&doc, psid);
    let path = kernel::FollowMePath::FaceLoop {
        object: member,
        face: top,
    };

    let (_merged, change) = doc
        .follow_me_merged_in_instance(inst, psid, pregion, &path, None)
        .expect("merge sweep");
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let undo_change = doc.undo().expect("undo");
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
    let redo_change = doc.redo().expect("redo");
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

// =========================================================================
// follow_me_face_in_instance
// =========================================================================

/// A solid FACE as the profile (a member's own side face) swept along a
/// def-owned sketch path births a NEW member; the source member is
/// untouched (design §3a, inside a definition).
#[test]
fn follow_me_face_in_instance_sweeps_a_separate_member() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let side = face_matching(&doc, member, Vec3::new(-1.0, 0.0, 0.0));

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(-1.0, 1.0, 0.0), Point3::new(-3.0, 1.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    let (swept, change) = doc
        .follow_me_face_in_instance(inst, member, side, &path, None)
        .expect("face profile sweeps");
    assert_ne!(swept, member);
    assert!(doc.object(swept).is_some());
    assert!(doc.object(member).is_some(), "source member untouched");
    assert_eq!(doc.object(member).unwrap().faces().len(), 6);
    assert_eq!(
        doc.object(swept).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(doc.def_members(comp).unwrap().contains(&swept));
    assert_eq!(change.components_touched, vec![comp]);

    doc.undo().expect("undo sweep");
    assert!(doc.object(swept).is_none());
    assert!(doc.object(member).is_some());
    doc.redo().expect("redo sweep");
    assert!(doc.object(swept).is_some());
}

/// When the path's `FaceLoop` names the SAME object the profile face came
/// from, the sweep MERGES with it in one gesture — the profile-object
/// self-merge (design §3b, mirrored inside a definition).
#[test]
fn follow_me_face_in_instance_merges_when_the_path_is_its_own_face_loop() {
    let mut doc = Document::new();
    let (comp, inst, _placeholder) = boxed_component(&mut doc);
    // A base box (the standard 4x2x1 `boxed_component` footprint) with a
    // small BOSS unioned onto its top, entirely INSIDE the top face's
    // footprint (never touching its rim): the union leaves the base's outer
    // top loop exactly the plain 4x2 rectangle (an interior hole opens where
    // the boss passes through it), while the boss contributes its own tiny,
    // fully DETACHED top cap — the "small floating profile, carried along a
    // big loop" shape every OTHER Follow Me merge spec in this file already
    // proves works, here built as one compound solid instead of a drawn
    // profile so it has its own FACE to use as the Follow Me profile. A
    // profile taken from a face ADJACENT to (sharing a full edge with) its
    // own path loop is a structurally different, much tighter case
    // (`FollowMeError::PathTooTight`) that this construction sidesteps
    // entirely.
    let base = extrude_box_in_instance(&mut doc, inst, 0.0, 0.0, 4.0, 2.0, 1.0);
    let boss = extrude_box_in_instance(&mut doc, inst, 1.5, 0.5, 1.9, 0.9, 1.3);
    let (member, _) = doc
        .boolean_in_component(comp, base, boss, BooleanOp::Union)
        .expect("union the boss onto the base");

    let cap = face_matching_by_extent(&doc, member, Vec3::new(0.0, 0.0, 1.0), true);
    let top = face_matching_by_extent(&doc, member, Vec3::new(0.0, 0.0, 1.0), false);
    let path = kernel::FollowMePath::FaceLoop {
        object: member,
        face: top,
    };

    let result = doc.follow_me_face_in_instance(inst, member, cap, &path, None);
    let (merged, change) = result.expect("self-referential face sweep merges");
    assert!(
        doc.object(member).is_none(),
        "the source member is consumed"
    );
    assert!(doc.object(merged).is_some());
    assert_eq!(
        doc.object(merged).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(doc.def_members(comp).unwrap().contains(&merged));
    assert_eq!(change.components_touched, vec![comp]);

    doc.undo().expect("undo the merged sweep");
    assert!(doc.object(member).is_some(), "base restored");
    assert!(doc.object(merged).is_none());
    doc.redo().expect("redo the merged sweep");
    assert!(doc.object(member).is_none());
    assert!(doc.object(merged).is_some());
}

/// The profile object must be a live member of the calling instance's OWN
/// definition — a world object or a foreign definition's member refuses.
#[test]
fn follow_me_face_in_instance_refuses_a_world_or_foreign_profile_object() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);

    let (gsid, _) = doc
        .begin_sketch_on_plane_in_instance(inst_a, ground())
        .expect("begin path sketch");
    doc.sketch_mut(gsid)
        .unwrap()
        .add_segment(Point3::new(-1.0, 1.0, 0.0), Point3::new(-3.0, 1.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(gsid).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: gsid,
        edges,
    };

    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(11.0, 0.0, 0.0),
                Point3::new(11.0, 1.0, 0.0),
                Point3::new(10.0, 1.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let world_face = face_matching(&doc, world_box, Vec3::new(-1.0, 0.0, 0.0));
    assert_eq!(
        doc.follow_me_face_in_instance(inst_a, world_box, world_face, &path, None),
        Err(DocumentError::UnknownObject)
    );

    let foreign_face = face_matching(&doc, member_b, Vec3::new(-1.0, 0.0, 0.0));
    assert_eq!(
        doc.follow_me_face_in_instance(inst_a, member_b, foreign_face, &path, None),
        Err(DocumentError::UnknownObject)
    );
}

/// The path, independently of the profile object, must resolve within the
/// same definition — a path sketch owned by a DIFFERENT definition refuses.
#[test]
fn follow_me_face_in_instance_refuses_a_foreign_definitions_path() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, member_a) = boxed_component(&mut doc);
    let (_comp_b, inst_b, _member_b) = boxed_component(&mut doc);
    let side = face_matching(&doc, member_a, Vec3::new(-1.0, 0.0, 0.0));

    let (foreign_gs, _) = doc
        .begin_sketch_on_plane_in_instance(inst_b, ground())
        .expect("begin path in b");
    doc.sketch_mut(foreign_gs)
        .unwrap()
        .add_segment(Point3::new(-1.0, 1.0, 0.0), Point3::new(-3.0, 1.0, 0.0))
        .unwrap();
    let edges: Vec<SketchEdgeId> = doc.sketch(foreign_gs).unwrap().edges().keys().collect();
    let path = kernel::FollowMePath::SketchEdges {
        sketch: foreign_gs,
        edges,
    };

    assert_eq!(
        doc.follow_me_face_in_instance(inst_a, member_a, side, &path, None),
        Err(DocumentError::UnknownSketch)
    );
}

// =========================================================================
// boolean_in_component
// =========================================================================

/// Adds a second overlapping box member to `comp` through `inst`, offset so
/// it overlaps the first (0..4, 0..2) box by a 2x2 footprint — general
/// position for a well-defined boolean.
fn second_overlapping_member(doc: &mut Document, inst: InstanceId) -> ObjectId {
    extrude_box_in_instance(doc, inst, 2.0, 0.0, 6.0, 2.0, 1.0)
}

/// Union of two members of the SAME definition replaces both with one new
/// member, and every instance of the definition sees it.
#[test]
fn boolean_in_component_unions_two_members_and_propagates() {
    let mut doc = Document::new();
    let (comp, inst, a) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let b = second_overlapping_member(&mut doc, inst);

    let (result, change) = doc
        .boolean_in_component(comp, a, b, BooleanOp::Union)
        .expect("union in component");
    assert!(doc.object(a).is_none(), "operand a consumed");
    assert!(doc.object(b).is_none(), "operand b consumed");
    assert!(doc.object(result).is_some());
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(doc.def_members(comp).unwrap().contains(&result));
    assert_eq!(change.components_touched, vec![comp]);
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

/// Subtract and Intersect both work identically to their world counterparts,
/// just scoped to the definition's own members.
#[test]
fn boolean_in_component_supports_subtract_and_intersect() {
    for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
        let mut doc = Document::new();
        let (comp, inst, a) = boxed_component(&mut doc);
        let b = second_overlapping_member(&mut doc, inst);
        let (result, _) = doc
            .boolean_in_component(comp, a, b, op)
            .unwrap_or_else(|e| panic!("{op:?} in component failed: {e}"));
        assert_eq!(
            doc.object(result).unwrap().watertight(),
            WatertightState::Watertight
        );
    }
}

/// An operand that is a WORLD object, or a member of a DIFFERENT
/// definition, refuses typed — matching the group-boolean instance-refusal
/// precedent (never an implicit re-homing).
#[test]
fn boolean_in_component_refuses_cross_ownership_operands() {
    let mut doc = Document::new();
    let (comp_a, inst_a, a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);
    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(11.0, 0.0, 0.0),
                Point3::new(11.0, 1.0, 0.0),
                Point3::new(10.0, 1.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };

    assert_eq!(
        doc.boolean_in_component(comp_a, a, world_box, BooleanOp::Union),
        Err(DocumentError::UnknownObject),
        "a world operand refuses"
    );
    assert_eq!(
        doc.boolean_in_component(comp_a, a, member_b, BooleanOp::Union),
        Err(DocumentError::UnknownObject),
        "an operand from a different definition refuses"
    );
    // Nothing was touched by either refusal.
    let b = second_overlapping_member(&mut doc, inst_a);
    assert!(doc.object(a).is_some());
    assert!(doc.object(b).is_some());
}

/// Combining an object with itself is a degenerate contact, refused before
/// any mutation.
#[test]
fn boolean_in_component_refuses_combining_an_object_with_itself() {
    let mut doc = Document::new();
    let (comp, _inst, a) = boxed_component(&mut doc);
    assert_eq!(
        doc.boolean_in_component(comp, a, a, BooleanOp::Union),
        Err(DocumentError::Boolean(
            kernel::BooleanError::DegenerateContact
        ))
    );
    assert!(doc.object(a).is_some(), "a refused boolean touches nothing");
}

/// A stale/unknown `component` handle refuses typed.
#[test]
fn boolean_in_component_refuses_unknown_component() {
    let mut doc = Document::new();
    let (comp, _inst, a) = boxed_component(&mut doc);
    let b = second_overlapping_member(&mut doc, _inst);
    doc.undo().expect("undo the birth that created b");
    doc.undo().expect("undo make_component, hiding comp");
    assert_eq!(
        doc.boolean_in_component(comp, a, b, BooleanOp::Union),
        Err(DocumentError::UnknownComponent)
    );
}

/// Undo/redo of `boolean_in_component` restore/re-apply exactly and both
/// directions report every sibling instance.
#[test]
fn boolean_in_component_undo_redo_round_trips() {
    let mut doc = Document::new();
    let (comp, inst, a) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let b = second_overlapping_member(&mut doc, inst);

    let (result, _) = doc
        .boolean_in_component(comp, a, b, BooleanOp::Union)
        .expect("union");
    let shape = doc.object(result).unwrap().clone();

    let undo_change = doc.undo().expect("undo");
    assert!(doc.object(a).is_some(), "operand a restored");
    assert!(doc.object(b).is_some(), "operand b restored");
    assert!(doc.object(result).is_none());
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let redo_change = doc.redo().expect("redo");
    assert!(doc.object(a).is_none());
    assert!(doc.object(b).is_none());
    assert!(objects_equivalent(&shape, doc.object(result).unwrap()));
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

// =========================================================================
// slice_def_member
// =========================================================================

/// Slicing a member through a ROTATED + UNIFORMLY-SCALED instance, once
/// mapped forward through the pose, reproduces exactly the two pieces a
/// plain world `slice_node` would have produced from the same WORLD plane —
/// the pose⁻¹ round trip for `slice_def_member`.
#[test]
fn slice_def_member_round_trips_through_a_rotated_scaled_pose() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);

    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.6)
        .unwrap()
        .then(&Transform::uniform_scale(1.5))
        .then(&Transform::translation(Vec3::new(-8.0, 6.0, 2.0)));
    doc.transform_instance(inst, &pose).unwrap();

    // Expected: an identical WORLD box (same raw coordinates as the
    // member's own LOCAL frame), sliced by the LOCAL plane directly. The
    // plane `slice_def_member` is actually given is that same local plane
    // mapped forward through the pose — the "world-space plane" a user
    // would have drawn through the posed instance — which the kernel maps
    // back through pose⁻¹ to (up to floating-point noise) recover the
    // original local plane.
    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let local_plane =
        Plane::from_point_normal(Point3::new(2.0, 1.0, 0.5), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let world_plane = pose.apply_plane(&local_plane).unwrap();
    let ((world_pos, world_neg), _) = doc
        .slice_node(world_box, &local_plane)
        .expect("world slice");
    let world_pos = doc.object(world_pos).unwrap().clone();
    let world_neg = doc.object(world_neg).unwrap().clone();

    // `slice_def_member` maps `world_plane` back to (up to floating-point
    // noise) exactly `local_plane` via pose⁻¹ and slices the member in ITS
    // OWN local frame — so the pieces come out in the SAME raw coordinates
    // as `world_pos`/`world_neg` directly, no further pose mapping needed
    // for the comparison (unlike `follow_me_in_instance`'s round-trip specs,
    // where the "expected" shape was independently built with genuinely
    // different WORLD-space coordinates).
    let ((pos, neg), _) = doc
        .slice_def_member(inst, member, &world_plane)
        .expect("slice in instance");
    assert!(objects_equivalent(doc.object(pos).unwrap(), &world_pos));
    assert!(objects_equivalent(doc.object(neg).unwrap(), &world_neg));
}

/// The same round trip through a MIRRORED instance.
#[test]
fn slice_def_member_round_trips_through_a_mirrored_pose() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);

    let pose = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(5.0, 5.0, 0.0)));
    assert!(pose.determinant() < 0.0);
    doc.transform_instance(inst, &pose).unwrap();

    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let local_plane =
        Plane::from_point_normal(Point3::new(2.0, 1.0, 0.5), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let world_plane = pose.apply_plane(&local_plane).unwrap();
    let ((world_pos, world_neg), _) = doc
        .slice_node(world_box, &local_plane)
        .expect("world slice");
    let world_pos = doc.object(world_pos).unwrap().clone();
    let world_neg = doc.object(world_neg).unwrap().clone();

    let ((pos, neg), _) = doc
        .slice_def_member(inst, member, &world_plane)
        .expect("slice through a mirrored pose");
    assert!(objects_equivalent(doc.object(pos).unwrap(), &world_pos));
    assert!(objects_equivalent(doc.object(neg).unwrap(), &world_neg));
}

/// A plane maps unambiguously through ANY invertible pose, so
/// `slice_def_member` never refuses on account of a non-uniformly-scaled
/// instance — unlike a typed scalar distance (`AmbiguousInstanceScale`).
#[test]
fn slice_def_member_succeeds_under_non_uniform_scale() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);
    doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 3.0, 0.5)))
        .unwrap();

    let plane =
        Plane::from_point_normal(Point3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let result = doc.slice_def_member(inst, member, &plane);
    assert!(
        result.is_ok(),
        "a plane maps unambiguously under non-uniform scale"
    );
}

/// The full geometric round trip (not just success) under a GENUINELY
/// non-uniform pose: an off-axis rotation composed with an anisotropic
/// scale, so the pose's linear part is neither diagonal nor a similarity —
/// the spec above only checks that this shape of pose doesn't refuse;
/// this one checks the pieces it produces are actually right, by the same
/// method as `slice_def_member_round_trips_through_a_rotated_scaled_pose`
/// and its mirrored sibling: an equivalent WORLD box sliced by the same
/// LOCAL plane directly is the ground truth, since `slice_def_member` maps
/// the WORLD-space plane it's given back through pose⁻¹ to (up to
/// floating-point noise) that same local plane before slicing the member in
/// its own local frame.
#[test]
fn slice_def_member_round_trips_through_a_non_uniform_pose() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);

    let pose = Transform::rotation(Vec3::new(0.3, 1.0, 0.2), 0.55)
        .unwrap()
        .then(&Transform::scale(Vec3::new(2.0, 0.5, 3.0)))
        .then(&Transform::translation(Vec3::new(-4.0, 7.0, 3.0)));
    doc.transform_instance(inst, &pose).unwrap();

    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let local_plane =
        Plane::from_point_normal(Point3::new(2.0, 1.0, 0.5), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let world_plane = pose.apply_plane(&local_plane).unwrap();
    let ((world_pos, world_neg), _) = doc
        .slice_node(world_box, &local_plane)
        .expect("world slice");
    let world_pos = doc.object(world_pos).unwrap().clone();
    let world_neg = doc.object(world_neg).unwrap().clone();

    let ((pos, neg), _) = doc
        .slice_def_member(inst, member, &world_plane)
        .expect("slice through a non-uniform pose");
    assert!(objects_equivalent(doc.object(pos).unwrap(), &world_pos));
    assert!(objects_equivalent(doc.object(neg).unwrap(), &world_neg));
}

/// A slice propagates to every sibling instance, forward and through
/// undo/redo.
#[test]
fn slice_def_member_propagates_and_undo_redo_round_trips() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let plane =
        Plane::from_point_normal(Point3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();

    let ((pos, neg), change) = doc
        .slice_def_member(inst, member, &plane)
        .expect("slice in instance");
    assert!(doc.object(member).is_none(), "source consumed");
    assert!(doc.def_members(comp).unwrap().contains(&pos));
    assert!(doc.def_members(comp).unwrap().contains(&neg));
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let (pos_shape, neg_shape) = (
        doc.object(pos).unwrap().clone(),
        doc.object(neg).unwrap().clone(),
    );

    let undo_change = doc.undo().expect("undo");
    assert!(doc.object(member).is_some(), "source restored");
    assert!(doc.object(pos).is_none());
    assert!(doc.object(neg).is_none());
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let redo_change = doc.redo().expect("redo");
    assert!(doc.object(member).is_none());
    assert!(objects_equivalent(&pos_shape, doc.object(pos).unwrap()));
    assert!(objects_equivalent(&neg_shape, doc.object(neg).unwrap()));
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

/// An `object` that is a WORLD object, or a member of a DIFFERENT
/// definition, refuses typed.
#[test]
fn slice_def_member_refuses_an_object_outside_the_definition() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);
    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(11.0, 0.0, 0.0),
                Point3::new(11.0, 1.0, 0.0),
                Point3::new(10.0, 1.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let plane =
        Plane::from_point_normal(Point3::new(0.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();

    assert_eq!(
        doc.slice_def_member(inst_a, world_box, &plane),
        Err(DocumentError::UnknownObject)
    );
    assert_eq!(
        doc.slice_def_member(inst_a, member_b, &plane),
        Err(DocumentError::UnknownObject)
    );
}

// =========================================================================
// transform_def_member
// =========================================================================

/// Baking a WORLD-space gesture through the instance's pose reproduces, for
/// EVERY vertex, exactly what applying the gesture directly to the vertex's
/// WORLD image would have produced — the semantic contract of
/// `transform_def_member`'s pose conjugation, checked directly rather than
/// by comparing against a separately-built "expected" object. Proven under
/// a rotated + uniformly-scaled pose here; the non-uniform and mirrored
/// cases are proven by the specs immediately below using the same check.
#[test]
fn transform_def_member_bakes_a_world_gesture_through_the_pose() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.4)
        .unwrap()
        .then(&Transform::uniform_scale(2.0))
        .then(&Transform::translation(Vec3::new(3.0, -1.0, 2.0)));
    doc.transform_instance(inst, &pose).unwrap();

    let before_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();

    let t = Transform::rotation(Vec3::new(0.3, 0.1, 1.0), 0.8)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.0, 2.0, -1.0)));
    let change = doc
        .transform_def_member(inst, member, &t)
        .expect("bake a world gesture into the member");

    let after_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    let mut expected: Vec<Point3> = before_world.iter().map(|&p| t.apply_point(p)).collect();
    assert_eq!(after_world.len(), expected.len());
    for p in after_world {
        let i = expected.iter().position(|&q| p.approx_eq(q, 1e-6)).expect(
            "every post-bake world vertex must equal `t` applied to its pre-bake world image",
        );
        expected.swap_remove(i);
    }
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

/// A NON-UNIFORMLY-SCALED instance never refuses `transform_def_member`: a
/// full affine conjugation is well-defined regardless of the pose's scale
/// (unlike a scalar distance) — the design's key K2 posture decision,
/// checked with the same forward-image contract as the spec above.
#[test]
fn transform_def_member_allows_non_uniform_scale_never_ambiguous() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);
    let pose = Transform::scale(Vec3::new(2.0, 3.0, 0.5))
        .then(&Transform::translation(Vec3::new(4.0, -2.0, 1.0)));
    doc.transform_instance(inst, &pose).unwrap();

    let before_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();

    let t = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.0, 1.0, 1.0)));
    let result = doc.transform_def_member(inst, member, &t);
    assert!(
        result.is_ok(),
        "non-uniform instance scale must never refuse a full affine bake"
    );

    let after_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    let mut expected: Vec<Point3> = before_world.iter().map(|&p| t.apply_point(p)).collect();
    for p in after_world {
        let i = expected
            .iter()
            .position(|&q| p.approx_eq(q, 1e-6))
            .expect("world image contract must hold under non-uniform scale too");
        expected.swap_remove(i);
    }
}

/// The same forward-image contract holds through a MIRRORED instance: a
/// proper (non-reflecting) world gesture conjugates to a proper local one
/// regardless of the pose's own mirroring (`det(local_t) = det(t)`,
/// independent of the pose).
///
/// **Why this exact fixture:** a mirrored pose whose linear part `L` is
/// INVOLUTORY (`L⁻¹ = L` — e.g. a bare per-axis mirror with no rotation,
/// `diag(-1,1,1)`) is algebraically blind to a conjugation-direction bug.
/// The correct bake is `local_t = pose · t · pose⁻¹`
/// (`pose.then(t).then(&pose_inv)`); a bug that swapped the direction would
/// compute `pose⁻¹ · t · pose` instead. When `L⁻¹ = L`, those two reduce to
/// the exact same `L · R · L` linear part, so both directions produce an
/// IDENTICAL result — no fixture built on such a pose can ever catch a
/// swapped conjugation. Worse, if the pose's translation also lies on `t`'s
/// own rotation axis, the translation terms of the two candidate bakes
/// coincide too, so even the world-position half of the check goes blind.
///
/// This pose instead composes the mirror with a non-90° rotation and a
/// non-uniform scale, so its linear part is genuinely non-involutory
/// (confirmed with a scratch check before landing this spec: `pose`'s and
/// `pose⁻¹`'s linear parts differ by several units component-wise, and the
/// two candidate conjugations' images of a sample point diverge by whole
/// WORLD units — nowhere near float noise). `t`'s rotation axis
/// `(0.3, 0.1, 1.0)` is also off the pose's translation direction
/// `(6, 2, 1)` (neither parallel nor coincidentally cancelling), so the
/// translation terms don't line up by chance either. A regression that
/// swapped `pose`/`pose_inv` inside `transform_def_member` would fail this
/// spec loudly.
#[test]
fn transform_def_member_bakes_correctly_through_a_mirrored_pose() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);
    let pose = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7).unwrap())
        .then(&Transform::scale(Vec3::new(2.0, 3.0, 0.5)))
        .then(&Transform::translation(Vec3::new(6.0, 2.0, 1.0)));
    assert!(pose.determinant() < 0.0, "pose must still be mirrored");
    doc.transform_instance(inst, &pose).unwrap();

    let before_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();

    let t = Transform::rotation(Vec3::new(0.3, 0.1, 1.0), 0.8)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.0, 2.0, -1.0)));
    let result = doc.transform_def_member(inst, member, &t);
    assert!(result.is_ok(), "a proper gesture bakes through a mirror");

    let after_world: Vec<Point3> = vertex_positions(doc.object(member).unwrap())
        .into_iter()
        .map(|p| pose.apply_point(p))
        .collect();
    let mut expected: Vec<Point3> = before_world.iter().map(|&p| t.apply_point(p)).collect();
    assert_eq!(after_world.len(), expected.len());
    for p in after_world {
        let i = expected
            .iter()
            .position(|&q| p.approx_eq(q, 1e-6))
            .expect("world image contract must hold through a mirrored pose too");
        expected.swap_remove(i);
    }
}

/// A REFLECTING world gesture refuses regardless of the instance's own
/// pose — mirrored or not — exactly like `transform_object` refuses a
/// reflecting `t` in world space.
#[test]
fn transform_def_member_refuses_a_reflecting_gesture_regardless_of_pose() {
    let reflect = Transform::scale(Vec3::new(1.0, 1.0, -1.0));
    assert!(reflect.determinant() < 0.0);

    for pose in [
        Transform::IDENTITY,
        Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.3)
            .unwrap()
            .then(&Transform::uniform_scale(2.0)),
        Transform::scale(Vec3::new(-1.0, 1.0, 1.0)),
    ] {
        let mut doc = Document::new();
        let (_comp, inst, member) = boxed_component(&mut doc);
        doc.transform_instance(inst, &pose).unwrap();
        assert_eq!(
            doc.transform_def_member(inst, member, &reflect),
            Err(DocumentError::Transform(kernel::TransformError::Reflection))
        );
        assert!(
            doc.object(member).is_some(),
            "a refused bake touches nothing"
        );
    }
}

/// A singular world gesture refuses typed.
#[test]
fn transform_def_member_refuses_a_singular_gesture() {
    let mut doc = Document::new();
    let (_comp, inst, member) = boxed_component(&mut doc);
    let singular = Transform::scale(Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(
        doc.transform_def_member(inst, member, &singular),
        Err(DocumentError::Transform(kernel::TransformError::Singular))
    );
}

/// An `object` that is a WORLD object, or a member of a DIFFERENT
/// definition, refuses typed.
#[test]
fn transform_def_member_refuses_an_object_outside_the_definition() {
    let mut doc = Document::new();
    let (_comp_a, inst_a, _member_a) = boxed_component(&mut doc);
    let (_comp_b, _inst_b, member_b) = boxed_component(&mut doc);
    let world_box = {
        let gs = doc.add_sketch(ground());
        let region = draw_rect(
            &mut doc,
            gs,
            [
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(11.0, 0.0, 0.0),
                Point3::new(11.0, 1.0, 0.0),
                Point3::new(10.0, 1.0, 0.0),
            ],
        );
        doc.extrude_region(gs, region, 1.0).unwrap().0
    };
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(
        doc.transform_def_member(inst_a, world_box, &t),
        Err(DocumentError::UnknownObject)
    );
    assert_eq!(
        doc.transform_def_member(inst_a, member_b, &t),
        Err(DocumentError::UnknownObject)
    );
}

/// Undo/redo of `transform_def_member` restore/re-apply the exact inverse
/// and both directions report every sibling instance.
#[test]
fn transform_def_member_undo_redo_round_trips() {
    let mut doc = Document::new();
    let (comp, inst, member) = boxed_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .unwrap();
    let before = doc.object(member).unwrap().clone();

    let t = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::translation(Vec3::new(2.0, 1.0, 0.0)));
    let change = doc
        .transform_def_member(inst, member, &t)
        .expect("bake gesture");
    assert_eq!(
        change.instances_touched.into_iter().collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
    let after = doc.object(member).unwrap().clone();
    assert!(!objects_equivalent(&before, &after), "sanity: it moved");

    let undo_change = doc.undo().expect("undo");
    assert!(objects_equivalent(&before, doc.object(member).unwrap()));
    assert_eq!(
        undo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );

    let redo_change = doc.redo().expect("redo");
    assert!(objects_equivalent(&after, doc.object(member).unwrap()));
    assert_eq!(
        redo_change
            .instances_touched
            .into_iter()
            .collect::<HashSet<_>>(),
        [inst, sibling].into_iter().collect()
    );
}

/// A tombstoned member (`delete_def_member` hides it while keeping it
/// listed in `ComponentDef.members` — hide-not-delete) is not a live
/// operand: `apply_def_op` must refuse it with `UnknownObject`, exactly
/// as the world path (`apply_object_op`) refuses a hidden object —
/// otherwise a stale handle silently mutates invisible shared geometry,
/// records an undo entry for an edit nobody can see, and clears the redo
/// stack that could have revived the member. Found by adversarial review.
#[test]
fn apply_def_op_refuses_a_tombstoned_member() {
    let mut doc = Document::new();
    // Two members, so tombstoning one is legal (LastDefinitionMember
    // forbids emptying a definition).
    let gs = doc.add_sketch(ground());
    let ra = draw_rect(
        &mut doc,
        gs,
        [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    );
    let (a, _) = doc.extrude_region(gs, ra, 1.0).expect("extrude a");
    let gs2 = doc.add_sketch(ground());
    let rb = draw_rect(
        &mut doc,
        gs2,
        [
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 1.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
        ],
    );
    let (b, _) = doc.extrude_region(gs2, rb, 1.0).expect("extrude b");
    let (component, _inst, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("make_component");
    let member = doc.def_members(component).expect("live component")[1];
    doc.delete_def_member(component, member)
        .expect("tombstone the second member");
    let before = doc.save();

    let op = KernelOp::PushPull {
        face: FaceId::default(),
        distance: 0.1,
    };
    let err = doc
        .apply_def_op(component, member, op)
        .expect_err("a tombstoned member is not a live operand");
    assert!(
        matches!(err, DocumentError::UnknownObject),
        "expected UnknownObject, got {err:?}"
    );
    assert_eq!(
        doc.save(),
        before,
        "a refused def op must leave the document untouched"
    );
}
