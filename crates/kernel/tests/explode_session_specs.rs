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
//!   instances outright (SketchUp's posture), fully undoable — unless a
//!   live definition still nests it, which stays, empty.
//! - the cycle and depth gates on placing a definition mid-session read the
//!   whole frame stack, not just the innermost frame.

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

/// Maps a point through a placement's transform and returns `(x, y, z)`.
fn map(t: &Transform, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let p = t.apply_point(Point3::new(x, y, z));
    (p.x, p.y, p.z)
}

/// Max corner z of `object` seen through `t`.
fn top_z_through(doc: &Document, object: ObjectId, t: &Transform) -> f64 {
    doc.object(object)
        .expect("object is live")
        .vertices()
        .iter()
        .map(|(_, v)| t.apply_point(v.position).z)
        .fold(f64::MIN, f64::max)
}

/// **Live sibling placements** (ARCHITECTURE.md 2.11): two instances of
/// one definition — opening a session through one hides ONLY the entered
/// instance; the sibling stays live and its expanded placements are the
/// session's world objects seen through the session's inverse pose and
/// then the sibling's own pose, so an edit inside the session shows on the
/// sibling as it happens. Undoing the edit reverts the sibling; undoing
/// the open restores the plain definition render; closing folds the edit
/// into the definition every placement shares.
#[test]
fn opening_through_one_instance_keeps_every_sibling_live_and_showing_the_edit() {
    let mut doc = Document::new();
    let (component, instance_a, member_a, member_b) = two_member_component(&mut doc);
    let shift = Vec3::new(20.0, 0.0, 0.0);
    let (instance_b, _) = doc
        .place_instance(component, Transform::translation(shift))
        .expect("place a sibling instance");

    assert!(doc.instance_ids().contains(&instance_a));
    assert!(doc.instance_ids().contains(&instance_b));

    // Enter the SHIFTED placement, so the session's pose is not identity
    // and the sibling's inverse mapping is actually exercised.
    doc.open_explode_session(instance_b).expect("open");
    assert!(
        !doc.instance_ids().contains(&instance_b),
        "the entered instance hides"
    );
    assert!(
        doc.instance_ids().contains(&instance_a),
        "the sibling stays live"
    );
    assert_eq!(doc.instances_of(component), vec![instance_a]);
    assert_eq!(doc.session_placements(), vec![instance_a]);

    // The members are world objects at the entered pose (x in [20, 22]).
    let baked = doc.object(member_a).expect("live");
    let min_x = baked
        .vertices()
        .iter()
        .map(|(_, v)| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!(
        (min_x - 20.0).abs() < 1e-9,
        "member baked to the entered pose"
    );

    // The sibling's expanded placements are exactly the session's world
    // objects, each through the inverse session pose (identity sibling
    // pose composes to just that inverse).
    let placements = doc.expanded_def_placements(component);
    assert_eq!(
        placements.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        vec![member_a, member_b]
    );
    for (_, local) in &placements {
        let (x, y, z) = map(local, 20.0, 0.0, 0.0);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9 && z.abs() < 1e-9);
    }
    assert_eq!(
        doc.expanded_def_placements_shown(component),
        vec![true, true]
    );
    // Through the sibling's own pose the member sits where the sibling is.
    let world = doc.expanded_placements();
    assert_eq!(world.len(), 2);
    for (o, t, i) in &world {
        assert_eq!(*i, instance_a);
        assert!(*o == member_a || *o == member_b);
        assert!((top_z_through(&doc, *o, t) - 1.0).abs() < 1e-9);
    }

    // Edit inside the session: the sibling shows the raised top at once.
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    let (_, change) = doc
        .apply_object_op(
            member_a,
            KernelOp::PushPull {
                face: top,
                distance: 0.4,
            },
        )
        .expect("edit the shared member");
    let mut completed = change;
    doc.complete_change(&mut completed);
    assert!(
        completed.instances_touched.contains(&instance_a),
        "the sibling is reported touched by a session edit"
    );
    let (_, t, _) = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, _)| *o == member_a)
        .expect("the sibling places member a");
    assert!((top_z_through(&doc, member_a, &t) - 1.4).abs() < 1e-9);

    // Abort the edit: the sibling reverts with it.
    doc.undo().expect("undo the push/pull");
    let (_, t, _) = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, _)| *o == member_a)
        .expect("the sibling places member a");
    assert!((top_z_through(&doc, member_a, &t) - 1.0).abs() < 1e-9);

    // Abort the session: both instances live, the plain definition back.
    // The sibling rendered the session's world geometry a moment ago, so
    // the undo reports it touched (the render/inference layers re-pull it).
    let change = doc.undo().expect("undo the open");
    assert!(change.instances_touched.contains(&instance_a));
    assert_eq!(doc.explode_session_instance(), None);
    assert!(doc.instance_ids().contains(&instance_a));
    assert!(doc.instance_ids().contains(&instance_b));
    assert!(doc.session_placements().is_empty());
    let world = doc.expanded_placements();
    assert_eq!(world.len(), 4);
    for (o, t, _) in &world {
        assert!((top_z_through(&doc, *o, t) - 1.0).abs() < 1e-9);
    }

    // Redo the open and the edit, close: every placement shares the edit.
    doc.redo().expect("redo the open");
    doc.redo().expect("redo the push/pull");
    doc.close_explode_session().expect("close");
    assert!(doc.instance_ids().contains(&instance_a));
    assert!(doc.instance_ids().contains(&instance_b));
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

/// **An emptied close with a GROUPED sibling** (adversarial-review
/// finding): the siblings tombstoned with the emptied definition leave
/// their groups' member lists (a group never lists a hidden member), an
/// annotation anchored to one detaches, and undo restores the list, the
/// anchor's liveness, and the siblings exactly; redo re-empties; the saved
/// document reopens at every step.
#[test]
fn an_emptied_close_splices_a_grouped_sibling_out_of_its_group_and_undo_restores_it() {
    let mut doc = Document::new();
    let (component, instance, member_a, member_b) = two_member_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .expect("sibling");
    let other = build_box(&mut doc, 30.0, 0.0, 0.0, 31.0, 1.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Instance(sibling), NodeId::Object(other)])
        .expect("group the sibling with a plain box");
    let note = doc
        .add_leader_text(
            kernel::Anchor {
                node: Some(NodeId::Instance(sibling)),
                point: Point3::new(10.0, 0.0, 1.0),
            },
            Vec3::new(0.2, 0.2, 0.0),
            "sibling note".to_string(),
        )
        .expect("a note anchored to the sibling");
    assert_eq!(doc.annotation_detached(note), Some(false));
    let members_before = doc.group_members(group).expect("group is live");

    doc.open_explode_session(instance).expect("open");
    doc.delete_node(NodeId::Object(member_a)).expect("delete a");
    doc.delete_node(NodeId::Object(member_b)).expect("delete b");
    doc.close_explode_session()
        .expect("close empties the definition");

    assert!(doc.def_members(component).is_none(), "definition gone");
    assert!(
        !doc.instance_ids().contains(&sibling),
        "sibling gone with it"
    );
    assert_eq!(
        doc.group_members(group),
        Some(vec![NodeId::Object(other)]),
        "the group no longer lists the tombstoned sibling"
    );
    assert_eq!(
        doc.annotation_detached(note),
        Some(true),
        "the note detached"
    );
    Document::load(&doc.save()).expect("the saved document reopens");

    doc.undo().expect("undo the close");
    assert_eq!(doc.explode_session_instance(), Some(instance));
    assert!(doc.instance_ids().contains(&sibling));
    assert_eq!(doc.group_members(group), Some(members_before.clone()));
    assert_eq!(doc.annotation_detached(note), Some(false));
    Document::load(&doc.save_for_persistence()).expect("the autosave bytes reopen");

    doc.redo().expect("redo the close");
    assert!(!doc.instance_ids().contains(&sibling));
    assert_eq!(doc.group_members(group), Some(vec![NodeId::Object(other)]));
    assert_eq!(doc.annotation_detached(note), Some(true));
    Document::load(&doc.save()).expect("reopens after redo");

    doc.undo().expect("undo close");
    doc.undo().expect("undo delete b");
    doc.undo().expect("undo delete a");
    doc.undo().expect("undo open");
    assert_eq!(doc.group_members(group), Some(members_before));
    assert_eq!(
        doc.def_members(component).expect("live"),
        vec![member_a, member_b]
    );
    Document::load(&doc.save()).expect("reopens fully undone");
}

/// **Sibling placements carry the session's drawing** (ARCHITECTURE.md
/// 2.11): the definition's own sketch, baked to the world at open, and a
/// free sketch drawn mid-session both render on a sibling through the
/// inverse session pose (`def_rendered_sketches`); after close the
/// definition's sketches are its own again, at identity.
#[test]
fn a_sibling_renders_the_definition_sketch_and_mid_session_drawing() {
    let mut doc = Document::new();
    let (component, instance_a, _a, _b) = two_member_component(&mut doc);
    let (instance_b, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .expect("sibling");
    // A def-owned sketch: a square drawn on the ground inside instance A.
    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(instance_a, ground())
        .expect("def sketch");
    draw_rect(
        &mut doc,
        def_sketch,
        [
            Point3::new(8.0, 0.0, 0.0),
            Point3::new(9.0, 0.0, 0.0),
            Point3::new(9.0, 1.0, 0.0),
            Point3::new(8.0, 1.0, 0.0),
        ],
    );
    let (closed, frame) = doc.def_rendered_sketches(component).expect("live");
    assert_eq!(closed, vec![def_sketch]);
    assert_eq!(frame, Transform::IDENTITY);

    // Enter the SHIFTED placement: the def sketch bakes to x in [28, 29].
    doc.open_explode_session(instance_b).expect("open");
    let (rendered, frame) = doc.def_rendered_sketches(component).expect("live");
    assert_eq!(
        rendered,
        vec![def_sketch],
        "the baked def sketch still renders"
    );
    let p = frame.apply_point(Point3::new(28.0, 0.0, 0.0));
    assert!(
        (p.x - 8.0).abs() < 1e-9 && p.y.abs() < 1e-9,
        "the frame is the inverse session pose"
    );
    let baked_min_x = doc
        .sketch(def_sketch)
        .expect("live")
        .vertices()
        .iter()
        .map(|(_, v)| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!(
        (baked_min_x - 28.0).abs() < 1e-9,
        "baked to the entered pose"
    );

    // Draw a free sketch mid-session, as a recorded gesture (the app's
    // path): a sibling renders it too — while it is still a fresh,
    // unrecorded sketch, and once its gesture is on the history.
    let drawn = doc.add_sketch(ground());
    doc.begin_sketch_gesture(drawn).expect("gesture");
    draw_rect(
        &mut doc,
        drawn,
        [
            Point3::new(25.0, 3.0, 0.0),
            Point3::new(26.0, 3.0, 0.0),
            Point3::new(26.0, 4.0, 0.0),
            Point3::new(25.0, 4.0, 0.0),
        ],
    );
    let (rendered, _) = doc.def_rendered_sketches(component).expect("live");
    assert!(
        rendered.contains(&def_sketch) && rendered.contains(&drawn),
        "a fresh mid-session sketch renders on the sibling"
    );
    doc.end_sketch_gesture(drawn).expect("gesture recorded");
    let (rendered, _) = doc.def_rendered_sketches(component).expect("live");
    assert!(rendered.contains(&def_sketch) && rendered.contains(&drawn));
    assert!(doc.session_placements().contains(&instance_a));

    doc.close_explode_session().expect("close");
    let (after, frame) = doc.def_rendered_sketches(component).expect("live");
    assert_eq!(frame, Transform::IDENTITY);
    let mut after_sorted = after.clone();
    after_sorted.sort();
    let mut expected = vec![def_sketch, drawn];
    expected.sort();
    assert_eq!(
        after_sorted, expected,
        "both are the definition's own sketches now"
    );
    let _ = instance_a;
}

/// **The frame's own drawing, as one query** (`session_frame_sketches`):
/// while a component frame is innermost it is exactly the set a sibling
/// renders (`def_rendered_sketches`) — the baked definition sketch, a
/// free sketch drawn mid-session, an owned sketch of a member — and never
/// a free sketch drawn before the open. Empty with no frame open; a group
/// frame's own drawing is specified in `group_sketch_specs.rs`.
#[test]
fn the_session_frame_sketches_are_the_sibling_rendered_set() {
    let mut doc = Document::new();
    // Drawn as a recorded gesture (the app's path), so it is history
    // before the open, not a fresh sketch.
    let outside = doc.add_sketch(ground());
    doc.begin_sketch_gesture(outside).expect("gesture");
    draw_rect(
        &mut doc,
        outside,
        [
            Point3::new(-5.0, 0.0, 0.0),
            Point3::new(-4.0, 0.0, 0.0),
            Point3::new(-4.0, 1.0, 0.0),
            Point3::new(-5.0, 1.0, 0.0),
        ],
    );
    doc.end_sketch_gesture(outside).expect("gesture recorded");
    let (component, instance, member_a, _b) = two_member_component(&mut doc);
    let (def_sketch, _) = doc
        .begin_sketch_on_plane_in_instance(instance, ground())
        .expect("def sketch");
    draw_rect(
        &mut doc,
        def_sketch,
        [
            Point3::new(8.0, 0.0, 0.0),
            Point3::new(9.0, 0.0, 0.0),
            Point3::new(9.0, 1.0, 0.0),
            Point3::new(8.0, 1.0, 0.0),
        ],
    );
    assert!(doc.session_frame_sketches().is_empty(), "no frame open");

    doc.open_explode_session(instance).expect("open");
    // An owned sketch on member A's top face (z = 1).
    let top = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .expect("top plane");
    let owned = doc.add_sketch(top);
    doc.set_sketch_object_owner(owned, Some(member_a))
        .expect("owned by member A");
    let drawn = doc.add_sketch(ground());
    let frame = doc.session_frame_sketches();
    let (rendered, _) = doc.def_rendered_sketches(component).expect("live");
    assert_eq!(frame, rendered, "one set, shared with the sibling render");
    for s in [def_sketch, owned, drawn] {
        assert!(frame.contains(&s), "{s:?} is the frame's drawing");
    }
    assert!(!frame.contains(&outside), "pre-session drawing is context");

    doc.close_explode_session().expect("close");
    assert!(doc.session_frame_sketches().is_empty(), "closed");

    // A group frame's own drawing is its own (group_sketch_specs.rs): the
    // component frame's drawing and the pre-session line stay out of it.
    let c = build_box(&mut doc, 10.0, 0.0, 0.0, 11.0, 1.0, 1.0);
    let d = build_box(&mut doc, 12.0, 0.0, 0.0, 13.0, 1.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(c), NodeId::Object(d)])
        .expect("group");
    doc.open_group_session(group).expect("open group");
    let in_group = doc.add_sketch(ground());
    let frame = doc.session_frame_sketches();
    // (A fresh sketch minted before the open and never drawn into, like
    // `drawn` above, rides along by the fresh-sketch rule; it is empty,
    // so there is nothing in it to pick.)
    assert!(frame.contains(&in_group), "group frame: its own drawing");
    for s in [outside, def_sketch, owned] {
        assert!(!frame.contains(&s), "{s:?} is not the group frame's");
    }
}

/// **A sibling is not an operand while its definition is on loan**: the
/// definition-level surfaces addressed through a live sibling (or by
/// component id) refuse typed with `ExplodeSessionOpen`, document
/// untouched — its members are world objects right now, and editing them
/// as def-local content would corrupt them. Painting a face the sibling
/// shows is fine: it reaches the one shared object.
#[test]
fn a_live_sibling_refuses_definition_level_ops_mid_session() {
    let mut doc = Document::new();
    let (component, instance_a, member_a, _b) = two_member_component(&mut doc);
    let (instance_b, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .expect("place a sibling instance");
    doc.open_explode_session(instance_a).expect("open");
    let before = doc.save_for_persistence();

    assert_eq!(
        doc.transform_instance(
            instance_b,
            &Transform::translation(Vec3::new(0.0, 1.0, 0.0))
        )
        .unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    assert_eq!(
        doc.make_unique(instance_b).unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    assert_eq!(
        doc.transform_def_member(
            instance_b,
            member_a,
            &Transform::translation(Vec3::new(0.0, 1.0, 0.0))
        )
        .unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(
        doc.apply_def_op(
            component,
            member_a,
            KernelOp::PushPull {
                face: top,
                distance: 0.4
            }
        )
        .unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    assert_eq!(
        doc.delete_def_member(component, member_a).unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    assert_eq!(
        doc.begin_sketch_on_plane_in_instance(instance_b, ground())
            .unwrap_err(),
        DocumentError::ExplodeSessionOpen
    );
    assert_eq!(doc.save_for_persistence(), before, "nothing moved");

    doc.close_explode_session().expect("close");
    doc.transform_instance(
        instance_b,
        &Transform::translation(Vec3::new(0.0, 1.0, 0.0)),
    )
    .expect("a sibling is an ordinary instance again after close");
}

/// **Nested frames**: a definition B placed inside definition A, with two
/// world placements of A. Entering A's first placement and then drilling
/// into B's surfaced instance stacks two frames. A's live sibling renders
/// B's content THROUGH both frames — the inner frame's world geometry
/// through the inner inverse pose, then the outer inverse, then its own
/// pose — so an edit on a B member shows on the sibling of A at once.
#[test]
fn a_sibling_of_the_outer_frame_shows_an_edit_made_two_frames_deep() {
    let mut doc = Document::new();
    let inner = build_box(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (def_b, inst_b, _) = doc
        .make_component(&[NodeId::Object(inner)])
        .expect("inner component");
    let outer = build_box(&mut doc, 3.0, 0.0, 0.0, 4.0, 1.0, 1.0);
    let (def_a, inst_a1, _) = doc
        .make_component(&[NodeId::Instance(inst_b), NodeId::Object(outer)])
        .expect("outer component nesting the inner one");
    let shift = Vec3::new(0.0, 10.0, 0.0);
    let (inst_a2, _) = doc
        .place_instance(def_a, Transform::translation(shift))
        .expect("second placement of A");
    let _ = def_b;

    doc.open_explode_session(inst_a1).expect("enter A");
    // B's member instance is a world instance now; drill into it.
    let surfaced = doc
        .session_direct_members()
        .expect("session open")
        .into_iter()
        .find_map(|n| match n {
            NodeId::Instance(i) => Some(i),
            _ => None,
        })
        .expect("the surfaced B placement");
    doc.open_explode_session(surfaced).expect("enter B");
    assert_eq!(doc.session_stack().len(), 2);

    // The sibling of A renders both A's own member and B's member.
    let placements = doc.expanded_placements();
    let for_a2: Vec<_> = placements
        .iter()
        .filter(|(_, _, i)| *i == inst_a2)
        .collect();
    assert_eq!(for_a2.len(), 2, "outer member plus the inner one");
    let inner_t = for_a2
        .iter()
        .find(|(o, _, _)| *o == inner)
        .map(|(_, t, _)| *t)
        .expect("the inner member renders on A's sibling");
    assert!((top_z_through(&doc, inner, &inner_t) - 1.0).abs() < 1e-9);
    // And where the sibling is: the inner box's origin lands at the shift.
    let (x, y, z) = map(&inner_t, 0.0, 0.0, 0.0);
    assert!(x.abs() < 1e-9 && (y - 10.0).abs() < 1e-9 && z.abs() < 1e-9);

    // Edit the inner member two frames deep; the sibling of A shows it.
    let top = face_matching(&doc, inner, Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        inner,
        KernelOp::PushPull {
            face: top,
            distance: 0.5,
        },
    )
    .expect("edit the inner member");
    let inner_t = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, i)| *o == inner && *i == inst_a2)
        .map(|(_, t, _)| t)
        .expect("still rendered");
    assert!((top_z_through(&doc, inner, &inner_t) - 1.5).abs() < 1e-9);
    assert!(doc.session_placements().contains(&inst_a2));

    doc.close_innermost_session().expect("close B");
    let inner_t = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, i)| *o == inner && *i == inst_a2)
        .map(|(_, t, _)| t)
        .expect("still rendered after the inner close");
    assert!((top_z_through(&doc, inner, &inner_t) - 1.5).abs() < 1e-9);
    doc.close_innermost_session().expect("close A");
    assert_eq!(doc.session_stack().len(), 0);
    Document::load(&doc.save()).expect("the saved document reopens");
}

/// **Mid-session creations show on siblings**: a solid extruded from a
/// ground sketch while the session is open is session content (it folds
/// into the definition at close), so a sibling renders it as soon as it
/// exists, and stops when it is deleted.
#[test]
fn a_solid_created_mid_session_renders_on_every_sibling() {
    let mut doc = Document::new();
    let (component, instance_a, _a, _b) = two_member_component(&mut doc);
    let (instance_b, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
        .expect("sibling");
    doc.open_explode_session(instance_a).expect("open");

    let created = build_box(&mut doc, 8.0, 0.0, 0.0, 9.0, 1.0, 1.0);
    let on_sibling = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, i)| *o == created && *i == instance_b)
        .map(|(_, t, _)| t)
        .expect("the new solid renders on the sibling");
    let (x, _, _) = map(&on_sibling, 8.0, 0.0, 0.0);
    assert!((x - 28.0).abs() < 1e-9, "through the sibling's pose");

    doc.delete_node(NodeId::Object(created)).expect("delete it");
    assert!(
        !doc.expanded_placements()
            .iter()
            .any(|(o, _, _)| *o == created),
        "a deleted creation leaves the sibling"
    );
    doc.undo().expect("undo the delete");
    assert!(
        doc.expanded_placements()
            .iter()
            .any(|(o, _, _)| *o == created)
    );
    doc.close_explode_session().expect("close");
    assert!(
        doc.def_members(component).expect("live").contains(&created),
        "and it folded into the definition"
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
    // Placing the definition BEING EDITED refuses as the cycle it is —
    // the close would fold the new placement into its own definition.
    // (Placing an UNRELATED definition mid-session is now allowed and
    // folds in at close: nested_session_specs.)
    assert_eq!(
        doc.place_instance(component, Transform::IDENTITY)
            .unwrap_err(),
        DocumentError::ComponentCycle
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
/// typed, document untouched. A top-level SIBLING of the same definition
/// opens normally: siblings stay live (ARCHITECTURE.md 2.11), so the
/// grouped one keeps its place in its group, renders the session's
/// geometry there, and the tree stays valid through the round trip.
#[test]
fn a_group_nested_instance_refuses_a_session() {
    let mut doc = Document::new();
    let (component, instance, member_a, _b) = two_member_component(&mut doc);
    let (sibling, _) = doc
        .place_instance(component, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .expect("second placement");
    let (group, _) = doc
        .group_nodes(&[NodeId::Instance(instance)])
        .expect("group the first instance");

    assert_eq!(
        doc.open_explode_session(instance).unwrap_err(),
        DocumentError::ExplodeSessionGroupedInstance
    );
    assert_eq!(doc.explode_session_instance(), None);

    // The ungrouped sibling opens; the grouped placement stays a live
    // member of its group and shows the session's geometry.
    doc.open_explode_session(sibling)
        .expect("the sibling opens");
    assert!(doc.instance_ids().contains(&instance));
    assert_eq!(
        doc.group_members(group),
        Some(vec![NodeId::Instance(instance)])
    );
    let top = face_matching(&doc, member_a, Vec3::new(0.0, 0.0, 1.0));
    doc.apply_object_op(
        member_a,
        KernelOp::PushPull {
            face: top,
            distance: 0.4,
        },
    )
    .expect("edit");
    let t = doc
        .expanded_placements()
        .into_iter()
        .find(|(o, _, i)| *o == member_a && *i == instance)
        .map(|(_, t, _)| t)
        .expect("the grouped placement renders the member");
    assert!((top_z_through(&doc, member_a, &t) - 1.4).abs() < 1e-9);
    doc.close_explode_session().expect("and closes");
    assert_eq!(
        doc.group_members(group),
        Some(vec![NodeId::Instance(instance)])
    );
    Document::load(&doc.save()).expect("the saved document reopens");
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

// -------------------------------------------------------------- spec 14

/// **A definition a live definition still nests survives being emptied**:
/// deleting every member mid-session and closing would otherwise hide the
/// definition while the nested placement inside the outer definition kept
/// referencing it — a document that panics under the debug invariants and
/// saves a file that cannot be reopened. (`delete_definition` refuses the
/// same case with `DefinitionNestedInDefinition`; a close cannot refuse, so
/// the definition stays, empty.) The whole chain stays undoable.
#[test]
fn closing_an_emptied_definition_that_is_nested_elsewhere_keeps_it() {
    let mut doc = Document::new();
    let (inner, nested, member_a, member_b) = two_member_component(&mut doc);
    // Wrap the placement in an outer definition: `nested` is now a member
    // of it, and a second, world placement of the inner definition is the
    // one the session opens.
    let (_outer, _outer_instance, _) = doc
        .make_component(&[NodeId::Instance(nested)])
        .expect("wrap the placement");
    let (world, _) = doc
        .place_instance(inner, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .expect("world placement");

    doc.open_explode_session(world).expect("open");
    doc.delete_node(NodeId::Object(member_a)).expect("delete a");
    doc.delete_node(NodeId::Object(member_b)).expect("delete b");
    doc.close_explode_session().expect("close");

    assert!(
        doc.def_members(inner).is_some(),
        "the nested definition is kept, not tombstoned"
    );
    assert!(
        doc.instance_ids().contains(&world),
        "its world placement unhides like on any other close (empty, but live)"
    );
    Document::load(&doc.save()).expect("the saved document must reopen");
    Document::load(&doc.save_for_persistence()).expect("so must a mid-edit save");

    doc.undo().expect("undo close");
    assert_eq!(doc.explode_session_instance(), Some(world));
    doc.redo().expect("redo close");
    assert!(doc.def_members(inner).is_some());
    Document::load(&doc.save()).expect("the redone state reopens too");
}

// -------------------------------------------------------------- spec 15

/// **The cycle gate sees the whole frame stack**: with a group frame
/// opened on top of a component frame, placing the definition on loan must
/// still refuse as a cycle — the placement folds into the group, which
/// folds into the definition. Reading only the innermost frame let it
/// through, and the saved file then failed to reopen with a component
/// cycle.
#[test]
fn placing_the_definition_on_loan_refuses_under_a_group_frame_too() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
    let b = build_box(&mut doc, 4.0, 0.0, 0.0, 6.0, 2.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    let (component, instance, _) = doc
        .make_component(&[NodeId::Group(group)])
        .expect("wrap the group");

    doc.open_explode_session(instance).expect("open component");
    doc.open_group_session(group)
        .expect("open the member group");

    assert_eq!(
        doc.place_instance(component, Transform::IDENTITY)
            .unwrap_err(),
        DocumentError::ComponentCycle
    );

    doc.close_innermost_session().expect("close group");
    doc.close_innermost_session().expect("close component");
    Document::load(&doc.save()).expect("the saved document must reopen");
}

// -------------------------------------------------------------- spec 16

/// **The depth gate counts the open frames**: a chain of stacked sessions is
/// invisible to the committed definition graph (each entered instance is
/// hidden while its definition is on loan, and a definition placed
/// mid-session is not yet anyone's member), so reading only the committed
/// ancestry let a stack of sessions fold a chain deeper than the bound. The
/// close does not re-check depth, and the saved file then failed to reopen
/// with "definitions nest beyond 64 levels". The placement must refuse
/// typed instead, and everything that did open must still close into a
/// document that reopens.
#[test]
fn stacked_sessions_cannot_fold_a_chain_past_the_depth_bound() {
    const DEFS: usize = 70;
    let mut doc = Document::new();
    // Flat, unplaced definitions: delete every world placement but the
    // first, so the width bound never trips before the depth bound does.
    let mut defs = Vec::new();
    let mut firsts = Vec::new();
    for i in 0..DEFS {
        let b = build_box(
            &mut doc,
            3.0 * i as f64,
            0.0,
            0.0,
            3.0 * i as f64 + 1.0,
            1.0,
            1.0,
        );
        let (c, inst, _) = doc.make_component(&[NodeId::Object(b)]).expect("component");
        defs.push(c);
        firsts.push(inst);
    }
    for &inst in &firsts[1..] {
        doc.delete_node(NodeId::Instance(inst))
            .expect("drop the placement");
    }

    doc.open_explode_session(firsts[0])
        .expect("open the first frame");
    let mut refused = false;
    for &def in &defs[1..] {
        match doc.place_instance(def, Transform::IDENTITY) {
            Ok((inst, _)) => doc.open_explode_session(inst).map(|_| ()).expect("enter"),
            Err(e) => {
                assert_eq!(e, DocumentError::ComponentDepthExceeded);
                refused = true;
                break;
            }
        }
    }
    assert!(refused, "the 64-level bound must refuse a deeper stack");

    while doc.close_innermost_session().is_ok() {}
    Document::load(&doc.save()).expect("the saved document must reopen");
}

// -------------------------------------------------------------- spec 17

/// **Ungrouping a member group inside a component session keeps its
/// contents in the component**: dissolving the surfaced group drops it from
/// the definition's member list, and the released objects then sit at the
/// definition's top level. They must be listed there at close. Left
/// unlisted, the definition owned objects it could not reach, and the saved
/// file refused to reopen.
#[test]
fn ungrouping_a_member_group_in_a_component_session_keeps_the_objects() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = build_box(&mut doc, 3.0, 0.0, 0.0, 4.0, 1.0, 1.0);
    let (group, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group");
    let (component, instance, _) = doc
        .make_component(&[NodeId::Group(group)])
        .expect("wrap the group");

    doc.open_explode_session(instance).expect("open");
    doc.ungroup(group).expect("ungroup the member group");
    Document::load(&doc.save_for_persistence()).expect("a mid-session autosave must reopen");
    doc.close_explode_session().expect("close");

    let members = doc.def_members(component).expect("component survives");
    assert!(
        members.contains(&a) && members.contains(&b),
        "the released objects are direct members of the definition"
    );
    Document::load(&doc.save()).expect("the saved document must reopen");

    doc.undo().expect("undo close");
    Document::load(&doc.save_for_persistence()).expect("the reopened session autosaves");
    doc.redo().expect("redo close");
    Document::load(&doc.save()).expect("the redone state reopens too");
}

// -------------------------------------------------------------- spec 18

/// **The same, for a group of component instances** — the shape a user
/// reported: group four placed components, make that group a component,
/// open it, and ungroup. The released instances stay inside the component,
/// and neither the autosave nor the saved file panics or fails to reopen.
#[test]
fn ungrouping_a_member_group_of_instances_in_a_component_session_keeps_them() {
    let mut doc = Document::new();
    let mut placed = Vec::new();
    for i in 0..4 {
        let x = 3.0 * i as f64;
        let b = build_box(&mut doc, x, 0.0, 0.0, x + 1.0, 1.0, 1.0);
        let (_, inst, _) = doc.make_component(&[NodeId::Object(b)]).expect("component");
        placed.push(NodeId::Instance(inst));
    }
    let (group, _) = doc.group_nodes(&placed).expect("group the instances");
    let (component, wrapper, _) = doc
        .make_component(&[NodeId::Group(group)])
        .expect("wrap the group");

    doc.open_explode_session(wrapper).expect("open");
    doc.ungroup(group).expect("ungroup the member group");
    Document::load(&doc.save_for_persistence()).expect("a mid-session autosave must reopen");
    doc.close_explode_session().expect("close");
    Document::load(&doc.save()).expect("the saved document must reopen");
    let members = doc.def_member_nodes(component).expect("component survives");
    for &n in &placed {
        let NodeId::Instance(i) = n else {
            unreachable!()
        };
        assert!(
            !doc.instance_ids().contains(&i),
            "a released instance is not back in the world"
        );
        assert!(
            members.contains(&n),
            "a released instance is a direct member of the definition: {members:?}"
        );
    }

    doc.undo().expect("undo close");
    Document::load(&doc.save_for_persistence()).expect("the reopened session autosaves");
    doc.redo().expect("redo close");
    Document::load(&doc.save()).expect("the redone state reopens too");
}

// -------------------------------------------------------------- spec 19

/// **A nested member group released by an ungroup is listed too**: with
/// `outer = { inner, c }` wrapped in a component, ungrouping `outer` in the
/// session leaves `inner` (a group, still owned by the definition) at the
/// top level next to `c`. Both must be direct members at close; a group
/// the definition owns but does not list is as unreachable as an object.
#[test]
fn ungrouping_the_outer_of_nested_member_groups_lists_the_inner_group() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = build_box(&mut doc, 3.0, 0.0, 0.0, 4.0, 1.0, 1.0);
    let c = build_box(&mut doc, 6.0, 0.0, 0.0, 7.0, 1.0, 1.0);
    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("inner group");
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .expect("outer group");
    let (component, instance, _) = doc
        .make_component(&[NodeId::Group(outer)])
        .expect("wrap the outer group");

    doc.open_explode_session(instance).expect("open");
    doc.ungroup(outer).expect("ungroup the outer group");
    Document::load(&doc.save_for_persistence()).expect("a mid-session autosave must reopen");
    doc.close_explode_session().expect("close");

    let members = doc.def_member_nodes(component).expect("component survives");
    assert!(
        members.contains(&NodeId::Group(inner)) && members.contains(&NodeId::Object(c)),
        "the released inner group and the loose box are direct members: {members:?}"
    );
    Document::load(&doc.save()).expect("the saved document must reopen");

    doc.undo().expect("undo close");
    doc.redo().expect("redo close");
    Document::load(&doc.save()).expect("the redone state reopens too");
}

// -------------------------------------------------------------- spec 20

/// **Ungroup reads the whole frame stack**: with a component frame opened
/// on top of a group frame, ungrouping a world group from OUTSIDE the
/// session must still refuse. Looking only at the innermost frame let it
/// through, and the group close's fold-in walk then absorbed the released
/// members into the group.
#[test]
fn ungrouping_an_outside_group_refuses_under_a_component_frame_on_a_group_frame() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let x = build_box(&mut doc, 3.0, 0.0, 0.0, 4.0, 1.0, 1.0);
    let w1 = build_box(&mut doc, 6.0, 0.0, 0.0, 7.0, 1.0, 1.0);
    let w2 = build_box(&mut doc, 9.0, 0.0, 0.0, 10.0, 1.0, 1.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(a)]).expect("component");
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(inst), NodeId::Object(x)])
        .expect("group");
    let (outside, _) = doc
        .group_nodes(&[NodeId::Object(w1), NodeId::Object(w2)])
        .expect("outside group");

    doc.open_group_session(g).expect("open the group");
    doc.open_explode_session(inst)
        .expect("open the instance inside it");

    assert_eq!(
        doc.ungroup(outside).unwrap_err(),
        DocumentError::ExplodeSessionScope
    );

    doc.close_innermost_session().expect("close component");
    doc.close_innermost_session().expect("close group");
    let members = doc.group_members(g).expect("group survives");
    assert!(
        !members.contains(&NodeId::Object(w1)) && !members.contains(&NodeId::Object(w2)),
        "the outside group's boxes never joined the session group: {members:?}"
    );
}
