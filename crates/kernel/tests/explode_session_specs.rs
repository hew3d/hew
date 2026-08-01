//! Executable specs for the **explode session** prototype
//! (docs/design/explode-session-prototype.md, branch `proto/explode-session`).
//! PROTOTYPE — nothing here implies these surfaces are staying; the design
//! doc explains the experiment and what it is trying to answer.
//!
//! The contract under test:
//! - `open_explode_session` bakes a component instance's live members and
//!   definition-owned sketches into WORLD-owned geometry at the instance's
//!   pose (SAME `ObjectId`/`SketchId`s — a move, not `explode_instance`'s
//!   copy), and hides every live instance of that definition; refuses typed
//!   on a non-similarity or mirrored pose, or a second session.
//! - the user edits the (now plain world) members with the ordinary tool
//!   set; `close_explode_session` folds them back — unbaking a touched
//!   member, restoring an untouched one from its pristine snapshot
//!   verbatim, folding in anything created mid-session, dropping anything
//!   deleted mid-session from membership, and unhiding the instances.
//! - session open/close are themselves ordinary, granular history actions.
//! - saving for persistence mid-session transparently serializes AS IF the
//!   session were closed, without touching the live document.
//! - ops that would restructure the tree, mutate instance poses, or derive
//!   new geometry from OUTSIDE the session refuse typed
//!   (`ExplodeSessionScope`) — the kernel backstop behind the app's own
//!   session modality.
//! - a close that finds the definition emptied deletes definition and
//!   instances outright (SketchUp's posture), fully undoable.

use kernel::{
    ComponentId, Document, DocumentError, InstanceId, KernelOp, NodeId, ObjectId, Plane, Point3,
    Transform, Vec3,
};

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

/// Builds an axis-aligned box `[x0,x1] x [y0,y1] x [z0,z1]` as a fresh world
/// object via a ground sketch + extrusion, returning its `ObjectId`.
fn build_box(doc: &mut Document, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let region = draw_rect(
        doc,
        s,
        [
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
        ],
    );
    let (id, _) = doc.extrude_region(s, region, z1 - z0).expect("extrude box");
    id
}

/// Two non-overlapping boxes folded into one component via an
/// identity-posed instance — the common fixture for every spec that does
/// not itself need a non-identity pose. Returns `(component, instance,
/// member_a, member_b)`.
fn two_member_component(doc: &mut Document) -> (ComponentId, InstanceId, ObjectId, ObjectId) {
    let a = build_box(doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let (component, instance, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("make_component");
    (component, instance, a, b)
}

/// The face of a live object whose outward normal most nearly matches
/// `dir` — `component_edit_k2_specs.rs`'s `face_matching`, duplicated
/// (integration test binaries share no code).
fn face_matching(doc: &Document, object: ObjectId, dir: Vec3) -> kernel::FaceId {
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

/// Max Euclidean deviation between `orig`'s vertex positions and
/// `current`'s, matched by `VertexId` (stable as long as topology — vertex
/// count — never changed, true for a plain translate-only push/pull).
fn max_vertex_deviation(orig: &kernel::Object, current: &kernel::Object) -> f64 {
    orig.vertices()
        .iter()
        .map(|(vid, v)| {
            let cur = current
                .vertices()
                .get(vid)
                .expect("vertex id stable across a translate-only push/pull");
            (cur.position - v.position).length()
        })
        .fold(0.0, f64::max)
}

// --------------------------------------------------------------- spec 1

/// **Id durability**: opening a session retargets ownership to World and
/// closing folds it back to Definition — same `ObjectId`s, same
/// `ComponentId`, identical membership, throughout.
#[test]
fn ids_and_ownership_survive_a_full_open_close_round_trip() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);

    let members_before = doc.def_members(component).expect("live component");
    assert_eq!(members_before, vec![member_a, member_b]);
    assert!(!doc.visible_object_ids().contains(&member_a));
    assert!(doc.instance_ids().contains(&instance));

    doc.open_explode_session(instance).expect("open session");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    // Members are now WORLD objects: visible directly, no longer listed as
    // this (or any) definition's members.
    assert!(doc.visible_object_ids().contains(&member_a));
    assert!(doc.visible_object_ids().contains(&member_b));
    // `ComponentDef.members` still lists them (the validator's narrowly
    // scoped session exemption — see `debug_validate_tree`), but neither
    // resolves to a `Definition`-owned object while the session is open.
    assert_eq!(
        doc.def_members(component).expect("still live"),
        vec![member_a, member_b]
    );
    // The entered instance (and every sibling of `component`) is hidden.
    assert!(!doc.instance_ids().contains(&instance));

    doc.close_explode_session().expect("close session");
    assert_eq!(doc.explode_session_instance(), None);

    let members_after = doc.def_members(component).expect("live component");
    assert_eq!(
        members_after,
        vec![member_a, member_b],
        "same ids, same order, back on the definition"
    );
    assert!(!doc.visible_object_ids().contains(&member_a));
    assert!(!doc.visible_object_ids().contains(&member_b));
    assert!(doc.instance_ids().contains(&instance));
}

// --------------------------------------------------------------- spec 2

/// **Undo granularity**: with session open/close recorded as ordinary
/// history actions, undo after a closed session steps back through the
/// close boundary, through each intra-session op, through the open — with
/// correct geometry/ownership at every step — and redo replays forward to
/// the same final state.
#[test]
fn undo_steps_granularly_through_the_session_boundary() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);
    // Identity pose (fresh `make_component` instance): bake/unbake are exact
    // no-ops, so this spec can assert bit-exact geometry at every step —
    // drift under a non-trivial pose is spec 3's concern, not this one's.

    doc.open_explode_session(instance).expect("open");
    assert_eq!(doc.explode_session_instance(), Some(instance));

    // Touch member_a: push one of its faces out by 0.25.
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    let pushed_object = doc
        .apply_object_op(
            member_a,
            KernelOp::PushPull {
                face: top,
                distance: 0.25,
            },
        )
        .expect("push/pull member_a")
        .0;
    let _ = pushed_object;
    let member_a_pushed = doc.object(member_a).expect("live").clone();

    // Draw + extrude a brand-new solid mid-session — must fold in at close.
    let new_obj = build_box(&mut doc, 10.0, 10.0, 0.0, 11.0, 11.0, 1.0);

    doc.close_explode_session().expect("close");
    assert_eq!(doc.explode_session_instance(), None);

    let members_after_close = doc.def_members(component).expect("live");
    assert_eq!(members_after_close.len(), 3, "the new solid folded in");
    assert!(members_after_close.contains(&new_obj));
    assert!(!doc.visible_object_ids().contains(&member_a));
    assert!(!doc.visible_object_ids().contains(&new_obj));

    // undo #1: undo the close — re-opens the session.
    doc.undo().expect("undo close");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    assert!(
        doc.visible_object_ids().contains(&new_obj),
        "the folded-in solid is back to being a plain world object"
    );
    assert!(doc.visible_object_ids().contains(&member_a));
    assert_eq!(
        doc.object(member_a).expect("live").faces().len(),
        member_a_pushed.faces().len()
    );

    // undo #2: undo the extrude — the new solid disappears again.
    doc.undo().expect("undo extrude");
    assert!(!doc.visible_object_ids().contains(&new_obj));
    assert!(
        doc.visible_object_ids().contains(&member_a),
        "still mid-session"
    );

    // undo #3: undo the push/pull — member_a's geometry returns to its
    // freshly-baked (pre-edit) state, still a world object mid-session.
    doc.undo().expect("undo push/pull");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    let top_after_undo = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    let z = doc.object(member_a).expect("live").faces()[top_after_undo]
        .plane
        .signed_distance(Point3::new(0.0, 0.0, 0.0));
    assert!(
        (z.abs() - 1.0).abs() < 1e-9,
        "top face back at its pre-push height, z-offset was {z}"
    );

    // undo #4: undo the open — the session is gone, everything back to the
    // definition, the instance visible again.
    doc.undo().expect("undo open");
    assert_eq!(doc.explode_session_instance(), None);
    assert!(doc.instance_ids().contains(&instance));
    let members_final = doc.def_members(component).expect("live");
    assert_eq!(members_final, vec![member_a, member_b]);
    assert!(!doc.visible_object_ids().contains(&member_a));

    // redo x4: forward to the exact same final state the forward pass built.
    doc.redo().expect("redo open");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    doc.redo().expect("redo push/pull");
    doc.redo().expect("redo extrude");
    doc.redo().expect("redo close");
    assert_eq!(doc.explode_session_instance(), None);
    let members_redone = doc.def_members(component).expect("live");
    assert_eq!(members_redone.len(), 3);
    assert!(members_redone.contains(&new_obj));
    assert!(members_redone.contains(&member_a));
    assert!(members_redone.contains(&member_b));
}

// --------------------------------------------------------------- spec 3

/// **Drift measurement**: 100 open/close cycles under an oblique
/// rotation + uniform-scale + translation pose, touching ONE member each
/// cycle (forcing the pose/pose⁻¹ round trip) and never touching the
/// other (the pristine-snapshot path). Prints the measured max vertex
/// deviation of each — these numbers go verbatim into the findings — and
/// asserts the untouched member is bit-exact while the touched member
/// stays under a generous bound.
#[test]
fn drift_measurement_over_100_open_close_cycles() {
    let mut doc = Document::new();
    let (_component, instance, member_a, member_b) = two_member_component(&mut doc);

    let pose = Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.9)
        .expect("well-defined rotation axis")
        .then(&Transform::uniform_scale(2.37))
        .then(&Transform::translation(Vec3::new(5.0, -3.0, 2.0)));
    assert!(pose.similarity_scale().is_some());
    assert!(pose.determinant() > 0.0);
    doc.transform_instance(instance, &pose)
        .expect("compose the oblique pose onto the identity instance");

    let orig_a = doc.object(member_a).expect("live").clone();
    let orig_b = doc.object(member_b).expect("live").clone();

    for _ in 0..100 {
        doc.open_explode_session(instance).expect("open");
        let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
        doc.apply_object_op(
            member_a,
            KernelOp::PushPull {
                face: top,
                distance: 0.05,
            },
        )
        .expect("push member_a out");
        let top2 = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
        doc.apply_object_op(
            member_a,
            KernelOp::PushPull {
                face: top2,
                distance: -0.05,
            },
        )
        .expect("pull member_a back");
        // member_b is never touched this cycle — the pristine-snapshot path.
        doc.close_explode_session().expect("close");
    }

    let final_a = doc.object(member_a).expect("live");
    let final_b = doc.object(member_b).expect("live");
    let drift_a = max_vertex_deviation(&orig_a, final_a);
    let drift_b = max_vertex_deviation(&orig_b, final_b);
    eprintln!(
        "explode-session drift over 100 cycles — touched member: {drift_a:e} m, \
         untouched member: {drift_b:e} m"
    );

    assert_eq!(
        drift_b, 0.0,
        "the untouched member is restored from its pristine snapshot verbatim every close — must be bit-exact"
    );
    assert!(
        drift_a < 1e-6,
        "touched member's pose round-trip drift {drift_a:e} exceeded the generous 1e-6 bound"
    );
}

// --------------------------------------------------------------- spec 4

/// **Pose gate**: a non-uniformly-scaled pose refuses typed; a mirrored
/// pose refuses typed; a uniform-scaled + rotated pose (a similarity with
/// positive determinant) opens.
#[test]
fn pose_gate_refuses_non_similarity_and_mirror_but_opens_a_similarity() {
    let mut doc = Document::new();
    let (_component, instance, _a, _b) = two_member_component(&mut doc);

    // Non-uniform scale: refused typed.
    doc.transform_instance(instance, &Transform::scale(Vec3::new(1.0, 2.0, 1.0)))
        .expect("compose a non-uniform scale");
    assert_eq!(
        doc.open_explode_session(instance).unwrap_err(),
        DocumentError::ExplodeSessionPoseUnsupported
    );
    // Undo the non-uniform scale back to identity (its own exact inverse).
    doc.transform_instance(instance, &Transform::scale(Vec3::new(1.0, 0.5, 1.0)))
        .expect("undo the non-uniform scale");

    // Mirror: refused typed.
    doc.transform_instance(instance, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .expect("compose a mirror");
    assert_eq!(
        doc.open_explode_session(instance).unwrap_err(),
        DocumentError::ExplodeSessionPoseUnsupported
    );
    // A pure axis mirror is its own inverse.
    doc.transform_instance(instance, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .expect("undo the mirror");

    // Uniform scale + rotation: a similarity with positive determinant — opens.
    doc.transform_instance(
        instance,
        &Transform::rotation(Vec3::new(1.0, 1.0, 1.0), 0.4).unwrap(),
    )
    .expect("compose a rotation");
    doc.transform_instance(instance, &Transform::uniform_scale(1.7))
        .expect("compose a uniform scale");
    doc.open_explode_session(instance)
        .expect("a similarity pose with positive determinant opens");
    doc.close_explode_session().expect("close");
}

// --------------------------------------------------------------- spec 5

/// **Metadata survival**: a member carrying analytic circle/cylinder
/// metadata round-trips a session (bake at open scales the radius by the
/// pose's uniform factor; the fold at close restores it) with the
/// metadata intact and the radius correctly restored.
#[test]
fn analytic_cylinder_metadata_survives_a_session_round_trip() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let center = Point3::new(1.0, 1.0, 0.0);
    let radius = 0.5;
    {
        let sk = doc.sketch_mut(s).expect("live");
        sk.begin_curve_with(kernel::CurveGeom { center, radius })
            .expect("analytic circle");
        let n = 16;
        let pt = |i: usize| {
            let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
            Point3::new(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
                0.0,
            )
        };
        for i in 0..n {
            sk.add_segment(pt(i), pt(i + 1)).expect("chord");
        }
        sk.end_curve();
    }
    let region = doc.extrudable_regions(s).expect("live")[0];
    let (member, _) = doc
        .extrude_region(s, region, 1.0)
        .expect("extrude cylinder");

    let wall_radius = |doc: &Document, obj: ObjectId| -> f64 {
        doc.object(obj)
            .expect("live")
            .faces()
            .values()
            .find_map(|f| match f.surface {
                Some(kernel::SurfaceRef::Cylinder { radius, .. }) => Some(radius),
                _ => None,
            })
            .expect("a wall face carries the cylinder")
    };
    assert!((wall_radius(&doc, member) - radius).abs() < 1e-12);

    let (component, instance, _) = doc
        .make_component(&[NodeId::Object(member)])
        .expect("make_component");

    let scale = 1.6;
    let pose = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.3)
        .expect("well-defined")
        .then(&Transform::uniform_scale(scale));
    doc.transform_instance(instance, &pose)
        .expect("compose a similarity pose");

    doc.open_explode_session(instance).expect("open");
    // Mid-session: the member is a baked world object; its cylinder radius
    // is scaled by the pose's uniform factor — the bake's own metadata
    // math, not merely a snapshot restore.
    let baked_radius = wall_radius(&doc, member);
    assert!(
        (baked_radius - radius * scale).abs() < 1e-9,
        "baked radius {baked_radius}, expected {}",
        radius * scale
    );

    doc.close_explode_session().expect("close");
    let restored_radius = wall_radius(&doc, member);
    assert!(
        (restored_radius - radius).abs() < 1e-9,
        "restored radius {restored_radius}, expected {radius}"
    );
    let members = doc.def_members(component).expect("live");
    assert_eq!(members, vec![member]);
}

// --------------------------------------------------------------- spec 6

/// **Deletion during session**: deleting one member mid-session drops it
/// from the definition's membership at close; undo of the whole chain
/// restores it.
#[test]
fn deleting_a_member_mid_session_drops_it_from_membership_at_close() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);

    doc.open_explode_session(instance).expect("open");
    doc.delete_node(NodeId::Object(member_b))
        .expect("delete member_b as a plain world object");
    doc.close_explode_session().expect("close");

    let members = doc.def_members(component).expect("live");
    assert_eq!(members, vec![member_a], "member_b dropped from membership");

    // Undo the whole chain: close, delete, open.
    doc.undo().expect("undo close");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    doc.undo().expect("undo delete");
    assert!(
        doc.visible_object_ids().contains(&member_b),
        "member_b restored as a world object mid-session"
    );
    doc.undo().expect("undo open");
    assert_eq!(doc.explode_session_instance(), None);
    let members_restored = doc.def_members(component).expect("live");
    assert_eq!(members_restored, vec![member_a, member_b]);
}

// --------------------------------------------------------------- spec 7

/// **Sibling handling**: two instances of the same definition — opening a
/// session through one hides BOTH; closing makes both visible again, both
/// reflecting the session's edit (shared geometry, one definition).
#[test]
fn opening_through_one_instance_hides_every_sibling_and_the_edit_is_shared() {
    let mut doc = Document::new();
    let (component, instance_a, member_a, _member_b) = two_member_component(&mut doc);
    let (instance_b, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .expect("place a sibling instance");

    assert!(doc.instance_ids().contains(&instance_a));
    assert!(doc.instance_ids().contains(&instance_b));

    doc.open_explode_session(instance_a).expect("open");
    assert!(
        !doc.instance_ids().contains(&instance_a),
        "entered instance hidden"
    );
    assert!(
        !doc.instance_ids().contains(&instance_b),
        "sibling hidden too"
    );

    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        member_a,
        KernelOp::PushPull {
            face: top,
            distance: 0.4,
        },
    )
    .expect("edit the shared member");

    doc.close_explode_session().expect("close");
    assert!(doc.instance_ids().contains(&instance_a));
    assert!(
        doc.instance_ids().contains(&instance_b),
        "sibling visible again"
    );
    // Both instances place the SAME definition, so both already reflect the
    // edit — there is only one copy of the geometry to look at.
    let members = doc.def_members(component).expect("live");
    let edited_top = face_matching(&doc, members[0], Vec3::new(0.0, 0.0, 1.0));
    let z = doc.object(members[0]).expect("live").faces()[edited_top]
        .plane
        .signed_distance(Point3::new(0.0, 0.0, 0.0));
    assert!(
        (z.abs() - 1.4).abs() < 1e-9,
        "the push/pull is visible on the shared definition"
    );
}

// --------------------------------------------------------------- spec 8

/// **Save for persistence**: saving mid-session yields exactly the bytes a
/// closed document would, WITHOUT closing the user's live session or
/// recording anything — the app never has to yank the user out of an edit
/// to save (autosave included).
#[test]
fn save_for_persistence_mid_session_matches_a_closed_save() {
    let mut doc = Document::new();
    let (component, instance, member_a, _b) = two_member_component(&mut doc);

    doc.open_explode_session(instance).expect("open");
    // Touch a member so the close being simulated is not a trivial one.
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        member_a,
        KernelOp::PushPull {
            face: top,
            distance: 0.25,
        },
    )
    .expect("push/pull member_a");

    let bytes_mid_session = doc.save_for_persistence();

    // The live document is untouched: session still open, members still
    // world-owned, the edit still in place.
    assert_eq!(doc.explode_session_instance(), Some(instance));
    assert!(doc.visible_object_ids().contains(&member_a));

    // Reference: a clone actually closed the ordinary way.
    let mut closed = doc.clone();
    closed.close_explode_session().expect("close the clone");
    assert_eq!(
        bytes_mid_session,
        closed.save(),
        "persistence bytes are exactly the closed document's bytes"
    );

    // And the real close still works afterwards.
    doc.close_explode_session().expect("close");
    assert_eq!(doc.explode_session_instance(), None);
    assert!(
        doc.def_members(component)
            .expect("live")
            .contains(&member_a)
    );
}

// --------------------------------------------------------------- spec 9

/// **Un-delete across a reopened boundary**: a member deleted mid-session,
/// closed over, then revived by undoing PAST the close and THEN past the
/// deletion, still belongs to the definition when the session is closed
/// again. (Guards the `SessionClosed` recorded-lists reconstruction: a
/// re-derivation by live-filtering would lose the member and leave it as
/// loose world geometry.)
#[test]
fn undeleting_across_a_reopened_boundary_keeps_membership() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);

    doc.open_explode_session(instance).expect("open");
    doc.delete_node(NodeId::Object(member_b)).expect("delete b");
    doc.close_explode_session().expect("close");
    assert_eq!(
        doc.def_members(component).expect("live"),
        vec![member_a],
        "the deleted member left the definition at close"
    );

    // Walk back INTO the session, then past the deletion.
    doc.undo().expect("undo close — reopens the session");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    doc.undo().expect("undo the deletion — revives member_b");
    assert!(doc.visible_object_ids().contains(&member_b));

    // Close again: the revived member must fold back in.
    doc.close_explode_session().expect("close again");
    let members = doc.def_members(component).expect("live");
    assert_eq!(
        members,
        vec![member_a, member_b],
        "the revived member is back in the definition, in order"
    );
    assert!(
        !doc.visible_object_ids().contains(&member_b),
        "member_b renders through instances again, not as loose world geometry"
    );
}

// -------------------------------------------------------------- spec 10

/// **Pre-existing tombstones survive**: a member deleted via
/// `delete_def_member` BEFORE a session opens keeps its membership-list
/// tombstone through an open/close round trip, so that older deletion's
/// undo still restores it.
#[test]
fn a_pre_session_tombstone_survives_the_round_trip() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);

    doc.delete_def_member(component, member_b)
        .expect("tombstone member_b before any session");

    doc.open_explode_session(instance).expect("open");
    doc.close_explode_session().expect("close");

    // The old deletion is still undoable: member_b comes back.
    // (Two undos first: the close and the open.)
    doc.undo().expect("undo close");
    doc.undo().expect("undo open");
    doc.undo().expect("undo the pre-session delete_def_member");
    let members = doc.def_members(component).expect("live");
    assert_eq!(
        members,
        vec![member_a, member_b],
        "the pre-session tombstone entry survived the session"
    );
}

// -------------------------------------------------------------- spec 11

/// **Scope and structure guards**: mid-session, ops that would restructure
/// the tree, mutate instance poses, or derive new geometry from outside
/// the session refuse `ExplodeSessionScope` typed — while the same ops on
/// session geometry keep working (a mid-session duplicate folds in).
#[test]
fn structural_and_outside_scope_ops_refuse_mid_session() {
    let mut doc = Document::new();
    let outside = build_box(&mut doc, 20.0, 20.0, 0.0, 21.0, 21.0, 1.0);
    // A genuine pre-session world sketch: drawn through the RECORDED
    // gesture path (as every real tool draws), so its creation is recorded
    // BEFORE the session opens — a bare `add_sketch` + direct `sketch_mut`
    // drawing would leave it "fresh" (never gestured), a state no app flow
    // produces and one the session scope deliberately treats as its own.
    let outside_sketch = doc.add_sketch(ground());
    doc.begin_sketch_gesture(outside_sketch)
        .expect("open the drawing gesture");
    let outside_region = draw_rect(
        &mut doc,
        outside_sketch,
        [
            Point3::new(30.0, 30.0, 0.0),
            Point3::new(31.0, 30.0, 0.0),
            Point3::new(31.0, 31.0, 0.0),
            Point3::new(30.0, 31.0, 0.0),
        ],
    );
    doc.end_sketch_gesture(outside_sketch)
        .expect("record the drawing gesture");
    let (component, instance, member_a, _b) = two_member_component(&mut doc);

    doc.open_explode_session(instance).expect("open");

    // Structure and pose mutations refuse outright.
    assert_eq!(
        doc.rescale_document(2.0).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.group_nodes(&[NodeId::Object(member_a)]).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.make_component(&[NodeId::Object(member_a)]).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.place_instance(component, Transform::IDENTITY)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );

    // Outside-fed creations refuse; the outside geometry is untouchable.
    assert_eq!(
        doc.boolean(kernel::BooleanOp::Union, member_a, outside)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.extrude_region(outside_sketch, outside_region, 1.0)
            .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );
    assert_eq!(
        doc.duplicate_node(
            NodeId::Object(outside),
            &Transform::translation(Vec3::new(1.0, 0.0, 0.0))
        )
        .unwrap_err(),
        DocumentError::ExplodeSessionScope
    );

    // The same shape of op on SESSION geometry works — and folds in.
    let (dup, _) = doc
        .duplicate_node(
            NodeId::Object(member_a),
            &Transform::translation(Vec3::new(0.0, 8.0, 0.0)),
        )
        .expect("duplicating a session member is allowed");
    let NodeId::Object(dup) = dup else {
        panic!("an object duplicate is an object");
    };
    doc.close_explode_session().expect("close");
    assert!(
        doc.def_members(component).expect("live").contains(&dup),
        "the mid-session duplicate folded into the definition"
    );
    assert!(
        doc.visible_object_ids().contains(&outside),
        "outside geometry untouched throughout"
    );
}

// -------------------------------------------------------------- spec 12

/// **Emptied definition**: deleting every member mid-session and closing
/// deletes the definition and its instances outright (SketchUp's posture
/// for a now-empty component) — and the whole chain is undoable back to
/// the intact original.
#[test]
fn closing_an_emptied_definition_deletes_it_and_its_instances() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .expect("second placement");

    doc.open_explode_session(instance).expect("open");
    doc.delete_node(NodeId::Object(member_a)).expect("delete a");
    doc.delete_node(NodeId::Object(member_b)).expect("delete b");
    doc.close_explode_session().expect("close");

    assert!(
        doc.def_members(component).is_none(),
        "the emptied definition is gone"
    );
    assert!(
        !doc.instance_ids().contains(&instance) && !doc.instance_ids().contains(&sibling),
        "both instances are gone with it"
    );

    // Undo the whole chain: close, delete b, delete a, open.
    doc.undo().expect("undo close");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    doc.undo().expect("undo delete b");
    doc.undo().expect("undo delete a");
    doc.undo().expect("undo open");
    assert_eq!(doc.explode_session_instance(), None);
    assert_eq!(
        doc.def_members(component).expect("live"),
        vec![member_a, member_b],
        "definition intact again"
    );
    assert!(doc.instance_ids().contains(&instance));
    assert!(doc.instance_ids().contains(&sibling));

    // Redo the chain forward to the deleted end state.
    doc.redo().expect("redo open");
    doc.redo().expect("redo delete a");
    doc.redo().expect("redo delete b");
    doc.redo().expect("redo close");
    assert!(doc.def_members(component).is_none());
    assert!(!doc.instance_ids().contains(&sibling));
}

// -------------------------------------------------------------- spec 13

/// **Redo pushes exactly one close**: redoing a session close leaves ONE
/// `SessionClosed` on the undo stack, so the subsequent undos walk
/// close -> edit -> open with no phantom step. (Guards the
/// exit_explode_session caller-owns-the-push contract: when the core
/// pushed internally, redo's shared tail pushed the original action again
/// — a duplicate whose second undo re-entered the session from a corrupt
/// base.)
#[test]
fn redo_of_a_close_leaves_exactly_one_undo_step() {
    let mut doc = Document::new();
    let (component, instance, member_a, _b) = two_member_component(&mut doc);

    doc.open_explode_session(instance).expect("open");
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        member_a,
        KernelOp::PushPull {
            face: top,
            distance: 0.25,
        },
    )
    .expect("push/pull member_a");
    doc.close_explode_session().expect("close");

    // Walk back, then fully forward again via redo.
    doc.undo().expect("undo close");
    doc.undo().expect("undo push/pull");
    doc.redo().expect("redo push/pull");
    doc.redo().expect("redo close");
    assert_eq!(doc.explode_session_instance(), None);

    // Undo #1 after the redo must RE-OPEN the session (retract the close)...
    doc.undo().expect("undo the redone close");
    assert_eq!(
        doc.explode_session_instance(),
        Some(instance),
        "one undo retracts the close"
    );
    // ...and undo #2 must retract the EDIT — not a phantom second close.
    doc.undo().expect("undo the push/pull");
    assert_eq!(
        doc.explode_session_instance(),
        Some(instance),
        "still in the session: the second undo retracted the edit, not a phantom close"
    );
    let top_now = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    let z = doc.object(member_a).expect("live").faces()[top_now]
        .plane
        .signed_distance(Point3::new(0.0, 0.0, 0.0));
    assert!(
        (z.abs() - 1.0).abs() < 1e-9,
        "push/pull retracted, top back at z=1 (offset {z})"
    );
    // And undo #3 retracts the open itself.
    doc.undo().expect("undo the open");
    assert_eq!(doc.explode_session_instance(), None);
    assert_eq!(
        doc.def_members(component).expect("live").len(),
        2,
        "definition intact"
    );
}

// -------------------------------------------------------------- spec 14

/// **Grouped instances refuse**: an instance nested inside a Group cannot
/// open an explode session (the bake would orphan members from the
/// containing group; the app falls back to in-context editing) — refused
/// typed, document untouched, and a top-level instance of the SAME
/// definition still opens.
#[test]
fn a_group_nested_instance_refuses_a_session() {
    let mut doc = Document::new();
    let (component, instance, _a, _b) = two_member_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .expect("second placement");
    doc.group_nodes(&[NodeId::Instance(instance)])
        .expect("group the first instance");

    assert_eq!(
        doc.open_explode_session(instance).unwrap_err(),
        DocumentError::ExplodeSessionGroupedInstance
    );
    assert_eq!(doc.explode_session_instance(), None);

    // The SIBLING refuses too: a session must hide every placement, and a
    // grouped one cannot be hidden without breaking the group-membership
    // invariant — so any grouped placement anywhere gates the whole
    // definition into the in-context fallback.
    assert_eq!(
        doc.open_explode_session(sibling).unwrap_err(),
        DocumentError::ExplodeSessionGroupedInstance
    );

    // Ungroup, and the definition opens normally again.
    doc.undo().expect("undo the grouping");
    doc.open_explode_session(sibling)
        .expect("sibling opens once ungrouped");
    doc.close_explode_session().expect("and closes");
}

// -------------------------------------------------------------- spec 15

/// **An undone edit stays undone across a re-close** (playtest finding):
/// open, rotate a member, close (both instances rotated), undo back INTO
/// the session, undo the rotate, close again — the definition must be
/// back at its ORIGINAL geometry. The reconstructed session must inherit
/// the ORIGINAL open's pristine snapshots (still recorded on the
/// SessionOpened action below the boundary): re-snapshotting the current,
/// already-edited geometry made the second close "restore" the edit the
/// user had just undone.
#[test]
fn an_undone_edit_stays_undone_across_a_reclose() {
    let mut doc = Document::new();
    let (component, instance, member_a, _b) = two_member_component(&mut doc);
    let original = doc.object(member_a).expect("live").clone();

    doc.open_explode_session(instance).expect("open");
    // Rotate the member -45 degrees about Z through its own corner.
    let rot = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), -45.0_f64.to_radians())
        .expect("z rotation is well-formed");
    doc.transform_object(member_a, &rot).expect("rotate member");
    doc.close_explode_session().expect("close");

    doc.undo().expect("undo close — back into the session");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    doc.undo().expect("undo the rotate");

    // Fresh close (NOT a redo): the definition must be back to original.
    doc.close_explode_session().expect("close again");
    assert_eq!(doc.explode_session_instance(), None);
    let after = doc.object(member_a).expect("live");
    let dev = max_vertex_deviation(&original, after);
    assert!(
        dev < 1e-9,
        "undone rotate leaked into the definition (max vertex deviation {dev})"
    );
    let _ = component;
}
