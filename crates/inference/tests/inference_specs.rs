//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{
    ApertureMode, ElementRef, InferenceScene, PickRay, Snap, SnapKind, SnapLock, SnapQuery,
    SnapWeights,
};
use kernel::{
    AxesFrame, Guide, GuideId, InstanceId, Object, ObjectId, Plane, Point3, SketchEdgeId, SketchId,
    Transform, Vec3, tol,
};

const WIDE: f64 = 0.3; // generous pick-cone half-angle (radians)
const NARROW: f64 = 0.01;

fn unit_cube() -> Object {
    Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
            Point3::new(0.0, 1.0, 1.0),
        ],
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .unwrap()
}

fn cube_scene() -> InferenceScene {
    let mut scene = InferenceScene::new();
    scene.add_object(ObjectId::default(), &unit_cube(), &Transform::IDENTITY);
    scene
}

/// A ray from `eye` toward `target`.
fn ray_at(eye: Point3, target: Point3) -> PickRay {
    PickRay {
        origin: eye,
        direction: target - eye,
    }
}

fn query(ray: PickRay, aperture: f64) -> SnapQuery {
    SnapQuery {
        weights: SnapWeights::default(),
        ray,
        anchor: None,
        lock: None,
        aperture,
        aperture_mode: ApertureMode::Cone,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    }
}

fn resolve(scene: &InferenceScene, q: SnapQuery) -> Option<Snap> {
    scene.resolve(&q)
}

#[test]
fn endpoint_snap_is_exact() {
    let scene = cube_scene();
    let eye = Point3::new(3.0, 3.0, 3.0);
    // Aim slightly off the (1,1,1) corner; the cone still covers it.
    let snap = resolve(
        &scene,
        query(ray_at(eye, Point3::new(0.98, 1.0, 1.01)), WIDE),
    )
    .expect("corner is inside the cone");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(
        snap.position
            .approx_eq(Point3::new(1.0, 1.0, 1.0), tol::POINT_MERGE)
    );
    assert!(snap.source.is_some());
}

#[test]
fn endpoint_outranks_weaker_snaps() {
    let scene = cube_scene();
    // A wide cone aimed at the top face sees vertices, edges, midpoints,
    // and the face itself; the endpoint must win regardless of distance.
    let snap = resolve(
        &scene,
        query(
            ray_at(Point3::new(0.8, 0.8, 4.0), Point3::new(0.8, 0.8, 1.0)),
            0.6,
        ),
    )
    .expect("plenty of candidates in the cone");
    assert_eq!(snap.kind, SnapKind::Endpoint);
}

#[test]
fn midpoint_beats_on_edge() {
    let scene = cube_scene();
    // Aim at the midpoint of the top-front edge (0.5, 0, 1) with a cone
    // tight enough to exclude the corners (nearest corner is 0.5 away;
    // at distance ~3 that's ~0.16 rad off-axis).
    let eye = Point3::new(0.5, -3.0, 1.0);
    let snap = resolve(&scene, query(ray_at(eye, Point3::new(0.5, 0.0, 1.0)), 0.05))
        .expect("edge is under the cursor");
    assert_eq!(snap.kind, SnapKind::Midpoint);
    assert!(
        snap.position
            .approx_eq(Point3::new(0.5, 0.0, 1.0), tol::POINT_MERGE)
    );
}

#[test]
fn face_interior_snaps_on_face() {
    let scene = cube_scene();
    // Straight down at the middle of the top face, cone too tight for any
    // edge or vertex (nearest edge 0.5 away at distance 2 => ~0.24 rad).
    let eye = Point3::new(0.5, 0.5, 3.0);
    let snap = resolve(
        &scene,
        query(ray_at(eye, Point3::new(0.5, 0.5, 1.0)), NARROW),
    )
    .expect("face is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnFace);
    assert!(
        snap.position
            .approx_eq(Point3::new(0.5, 0.5, 1.0), tol::POINT_MERGE)
    );
}

#[test]
fn nothing_in_the_cone_returns_none() {
    let scene = cube_scene();
    let eye = Point3::new(10.0, 10.0, 10.0);
    let away = resolve(
        &scene,
        query(ray_at(eye, Point3::new(20.0, 20.0, 20.0)), NARROW),
    );
    assert!(away.is_none());
}

// ── ApertureMode::Cylinder (docs/design/camera.md §1) ──────────────────────
//
// Parallel (orthographic) projection's pick tolerance is a constant
// world-radius CYLINDER around the ray rather than perspective's
// angular CONE (`ApertureMode` doc comment). A ray straight along +Z from
// the origin, with one guide point as the sole candidate, isolates the
// tolerance-shape math: everything below chooses the point's perpendicular
// offset from the axis and its depth along it, then compares a Cone query at
// `aperture` (radians) against a Cylinder query at the EQUIVALENT radius
// `depth_ref * tan(aperture)` — the exact conversion `CameraRig` uses when
// switching projections (design camera.md §1).

const CYL_APERTURE_RAD: f64 = 0.1; // ~5.7°, arbitrary but not tiny/huge
const CYL_REF_DEPTH: f64 = 10.0; // the "target distance" the two modes agree at

/// A scene with exactly one guide point candidate, `offset` meters off the
/// +Z ray axis at `depth` meters along it.
fn single_point_scene(offset: f64, depth: f64) -> InferenceScene {
    let mut scene = InferenceScene::new();
    scene.add_guide(
        GuideId::default(),
        &Guide::Point {
            position: Point3::new(offset, 0.0, depth),
        },
    );
    scene
}

fn straight_z_ray() -> PickRay {
    PickRay {
        origin: Point3::ORIGIN,
        direction: Vec3::new(0.0, 0.0, 1.0),
    }
}

fn cone_query(aperture: f64) -> SnapQuery {
    SnapQuery {
        weights: SnapWeights::default(),
        ray: straight_z_ray(),
        anchor: None,
        lock: None,
        aperture,
        aperture_mode: ApertureMode::Cone,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    }
}

fn cylinder_query(radius: f64) -> SnapQuery {
    SnapQuery {
        weights: SnapWeights::default(),
        ray: straight_z_ray(),
        anchor: None,
        lock: None,
        aperture: radius,
        aperture_mode: ApertureMode::Cylinder,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    }
}

/// At the query's own reference depth, `radius = depth * tan(aperture)`
/// makes the cylinder and the cone agree exactly (design camera.md §1): a
/// candidate just inside one tolerance is just inside the other, and a
/// candidate just outside one is just outside the other.
#[test]
fn cylinder_cone_equivalence_at_target_distance() {
    let radius = CYL_REF_DEPTH * CYL_APERTURE_RAD.tan();

    let inside = single_point_scene(0.9 * radius, CYL_REF_DEPTH);
    assert!(
        inside.resolve(&cone_query(CYL_APERTURE_RAD)).is_some(),
        "cone admits a point safely inside its aperture at the reference depth"
    );
    assert!(
        inside.resolve(&cylinder_query(radius)).is_some(),
        "cylinder admits the same point at the equivalent radius"
    );

    let outside = single_point_scene(1.1 * radius, CYL_REF_DEPTH);
    assert!(
        outside.resolve(&cone_query(CYL_APERTURE_RAD)).is_none(),
        "cone rejects a point safely outside its aperture at the reference depth"
    );
    assert!(
        outside.resolve(&cylinder_query(radius)).is_none(),
        "cylinder rejects the same point at the equivalent radius"
    );
}

/// At a depth FARTHER than the reference, the cone's radius has grown past
/// the cylinder's constant one: a point on the cone's surface (so the cone
/// still admits it, at ANY depth, by construction) can sit outside the
/// fixed-radius cylinder.
#[test]
fn cylinder_cone_diverge_when_the_candidate_is_farther_than_the_target_distance() {
    let radius = CYL_REF_DEPTH * CYL_APERTURE_RAD.tan();
    let far_depth = 100.0 * CYL_REF_DEPTH;
    // Comfortably inside the cone at `far_depth` (its radius there is
    // `far_depth * tan(aperture)`, far larger than `offset`), but well
    // outside the cylinder's constant `radius`.
    let offset = 5.0 * radius;
    assert!(
        offset < far_depth * CYL_APERTURE_RAD.tan(),
        "still inside the cone"
    );
    assert!(offset > radius, "already outside the cylinder");

    let scene = single_point_scene(offset, far_depth);
    assert!(
        scene.resolve(&cone_query(CYL_APERTURE_RAD)).is_some(),
        "the cone's tolerance grows with depth, so a far candidate near its \
         surface is still admitted"
    );
    assert!(
        scene.resolve(&cylinder_query(radius)).is_none(),
        "the cylinder's tolerance is constant, so the same far candidate \
         falls outside it"
    );
}

/// At a depth NEARER than the reference, the opposite divergence: the cone
/// has shrunk (its radius there is smaller than the cylinder's constant
/// one), so a candidate comfortably inside the cylinder can fall outside
/// the cone.
#[test]
fn cylinder_cone_diverge_when_the_candidate_is_nearer_than_the_target_distance() {
    let radius = CYL_REF_DEPTH * CYL_APERTURE_RAD.tan();
    let near_depth = CYL_REF_DEPTH / 100.0;
    // Well inside the cylinder's constant radius, but well outside the
    // cone's shrunk radius at this depth (`near_depth * tan(aperture)`).
    let offset = 0.5 * radius;
    assert!(
        offset > near_depth * CYL_APERTURE_RAD.tan(),
        "already outside the cone"
    );
    assert!(offset < radius, "still inside the cylinder");

    let scene = single_point_scene(offset, near_depth);
    assert!(
        scene.resolve(&cone_query(CYL_APERTURE_RAD)).is_none(),
        "the cone's tolerance shrinks with depth, so a near candidate this \
         far off-axis falls outside it"
    );
    assert!(
        scene.resolve(&cylinder_query(radius)).is_some(),
        "the cylinder's tolerance is constant, so the same near candidate \
         is still admitted"
    );
}

/// Both modes reject a candidate behind the ray origin, regardless of how
/// wide the tolerance is — `ApertureMode` only changes the tolerance SHAPE
/// in front of the ray, never the in-front-of-the-origin gate.
#[test]
fn cylinder_mode_still_rejects_points_behind_the_ray() {
    let scene = single_point_scene(0.0, -5.0);
    assert!(scene.resolve(&cone_query(3.0)).is_none());
    assert!(scene.resolve(&cylinder_query(1e6)).is_none());
}

// The indexed spatial path's `Aabb::maybe_in_cylinder` prune agreeing with
// the linear reference scan (`resolve` ≡ `resolve_linear`) for a `Cylinder`
// query, over real solids/placements/guides/sketches, is covered by
// `indexed_queries_equal_the_linear_reference` in `index_props.rs`, which
// now randomizes `ApertureMode` alongside every other query dimension —
// guide-point candidates (used above for their exact, single-candidate
// geometry) bypass the index entirely, so they cannot stand in for that
// contract themselves.

#[test]
fn pick_face_returns_the_nearest_face_through_the_ray() {
    let scene = cube_scene();
    // Straight down through the cube: the ray crosses the top face (z=1, t=2)
    // and the bottom face (z=0, t=3). pick_face must return the nearer (top),
    // regardless of the snap-priority model (which resolve would apply).
    let ray = ray_at(Point3::new(0.5, 0.5, 3.0), Point3::new(0.5, 0.5, 1.0));
    let (source, depth) = scene.pick_face(&ray).expect("ray crosses the cube faces");
    match source.element {
        ElementRef::Face(_) => {}
        other => panic!("expected a face, got {other:?}"),
    }
    // The reported depth is the distance to the NEARER (top, z=1) face, ~2.
    assert!(
        (depth - 2.0).abs() < 1e-9,
        "depth is the nearest-hit distance"
    );
    // It is a top-face pick: re-querying from below must instead pick the
    // bottom face (different element), proving "nearest" is honored.
    let from_below = ray_at(Point3::new(0.5, 0.5, -3.0), Point3::new(0.5, 0.5, 0.0));
    let (below, _below_depth) = scene
        .pick_face(&from_below)
        .expect("ray crosses from below");
    assert_ne!(source.element, below.element);
}

#[test]
fn pick_face_misses_return_none() {
    let scene = cube_scene();
    // Aimed well clear of the unit cube.
    let ray = ray_at(Point3::new(10.0, 10.0, 10.0), Point3::new(20.0, 20.0, 20.0));
    assert!(scene.pick_face(&ray).is_none());
    // Degenerate direction is None, not a panic.
    let degenerate = PickRay {
        origin: Point3::ORIGIN,
        direction: Vec3::ZERO,
    };
    assert!(scene.pick_face(&degenerate).is_none());
}

#[test]
fn axis_lock_projects_onto_the_locked_line() {
    let scene = cube_scene();
    let anchor = Point3::new(0.0, 0.0, 0.0);
    // Cursor drifts off-axis toward the cube; lock to X.
    let q = SnapQuery {
        weights: SnapWeights::default(),
        ray: ray_at(Point3::new(0.7, 0.4, 3.0), Point3::new(0.7, 0.4, 0.0)),
        anchor: Some(anchor),
        lock: Some(SnapLock::Axis(inference::Axis::X)),
        aperture: WIDE,
        aperture_mode: ApertureMode::Cone,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene.resolve(&q).expect("lock with anchor always resolves");
    // On the X axis through the anchor:
    assert!(snap.position.y.abs() <= tol::POINT_MERGE);
    assert!(snap.position.z.abs() <= tol::POINT_MERGE);
    assert_eq!(snap.direction, Some(inference::Axis::X.unit()));
}

/// A moved drawing-axes frame (tool-parity design §4) reorients `SnapLock::Axis`:
/// "lock to X" resolves along the FRAME's red axis, not the literal world X,
/// once `set_axes_frame` has pushed a non-identity frame in.
#[test]
fn moved_axes_frame_reorients_the_axis_lock() {
    let mut scene = cube_scene();
    // Swap red/green: red now points along world +Y.
    let frame = AxesFrame::new(
        Point3::new(5.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
    )
    .expect("orthonormal frame");
    scene.set_axes_frame(frame);

    let anchor = Point3::new(5.0, 0.0, 0.0);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray: ray_at(Point3::new(5.4, 0.7, 3.0), Point3::new(5.4, 0.7, 0.0)),
        anchor: Some(anchor),
        lock: Some(SnapLock::Axis(inference::Axis::X)),
        aperture: WIDE,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene.resolve(&q).expect("lock with anchor always resolves");
    // On the frame's red axis (world +Y through the anchor): x and z pinned
    // to the anchor, y free.
    assert!((snap.position.x - anchor.x).abs() <= tol::POINT_MERGE);
    assert!((snap.position.z - anchor.z).abs() <= tol::POINT_MERGE);
    assert_eq!(snap.direction, Some(frame.x));
}

/// A moved drawing-axes frame also reorients the ambient origin/axis snap
/// candidates: the origin snaps at the frame's origin (not world origin),
/// and an on-axis snap reports the frame's own directions.
#[test]
fn moved_axes_frame_reorients_the_axis_snap_candidates() {
    let mut scene = InferenceScene::new(); // no cube: isolate the axes candidates
    let frame = AxesFrame::new(
        Point3::new(2.0, 3.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
    )
    .expect("orthonormal frame");
    scene.set_axes_frame(frame);

    // Aim squarely at the frame's origin: should snap Endpoint there, not at
    // the world origin.
    let q = query(ray_at(Point3::new(2.0, 3.0, 5.0), frame.origin), NARROW);
    let snap = scene.resolve(&q).expect("frame origin is an Endpoint snap");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(frame.origin, tol::POINT_MERGE));

    // Aim along the frame's red axis (world +Y) away from its origin: should
    // resolve OnAxis with the frame's own x direction, not world +X.
    let along_red = frame.origin + frame.x * 4.0;
    let q2 = query(
        ray_at(Point3::new(along_red.x, along_red.y, 5.0), along_red),
        NARROW,
    );
    let snap2 = scene.resolve(&q2).expect("on the frame's red axis");
    assert_eq!(snap2.kind, SnapKind::OnAxis);
    assert_eq!(snap2.direction, Some(frame.x));
}

#[test]
fn remove_object_clears_candidates_idempotently() {
    let mut scene = cube_scene();
    let (p, s, f) = scene.candidate_counts();
    assert!(p > 0 && s > 0 && f > 0, "cube produced candidates");
    scene.remove_object(ObjectId::default());
    assert_eq!(scene.candidate_counts(), (0, 0, 0));
    // Idempotent: removing again is a no-op, not a panic.
    scene.remove_object(ObjectId::default());
    assert_eq!(scene.candidate_counts(), (0, 0, 0));
}

#[test]
fn re_adding_an_object_replaces_its_candidates() {
    let mut scene = cube_scene();
    let first = scene.candidate_counts();
    scene.add_object(ObjectId::default(), &unit_cube(), &Transform::IDENTITY);
    assert_eq!(scene.candidate_counts(), first, "same object, same counts");
}

/// Removing an id that was never registered is a *free* no-op: it visits no
/// candidates (the owner-set fast path — `removal_candidates_visited` counts
/// the retain-pass work, like `occlusion_face_tests` for occlusion) and
/// perturbs nothing a later query can observe, including the lazily built
/// spatial index (nothing was removed, so no candidate index shifted and the
/// index must not be invalidated — the indexed/linear equality here would
/// catch a wrongly kept stale index).
#[test]
fn unknown_id_removal_is_free_and_unobservable() {
    let corner = Point3::new(1.0, 1.0, 1.0);
    let q = query(ray_at(Point3::new(3.0, 3.0, 3.0), corner), WIDE);

    // World-only scene: removing a never-registered *instance* id.
    let mut scene = cube_scene();
    let counts = scene.candidate_counts();
    let before = scene.resolve(&q); // also warms the index
    assert_eq!(before.map(|s| s.kind), Some(SnapKind::Endpoint));
    let visits = scene.removal_candidates_visited();
    scene.remove_instance(InstanceId::default());
    assert_eq!(
        scene.removal_candidates_visited(),
        visits,
        "a no-op removal must not scan candidates"
    );
    assert_eq!(scene.candidate_counts(), counts);
    assert_eq!(scene.resolve(&q), before);
    assert_eq!(
        scene.resolve(&q),
        scene.resolve_linear(&q),
        "index went stale"
    );

    // Instance-only scene: the shared member *label* is not a world-object
    // registration, so `remove_object` must key on the exact predicate it
    // retains by (`object == id && instance == None`) — and therefore stay a
    // free no-op that spares the placed candidates.
    let mut scene = InferenceScene::new();
    scene.set_def_member(ObjectId::default(), &unit_cube());
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::IDENTITY,
    );
    let counts = scene.candidate_counts();
    let before = scene.resolve(&q);
    assert_eq!(before.map(|s| s.kind), Some(SnapKind::Endpoint));
    let visits = scene.removal_candidates_visited();
    scene.remove_object(ObjectId::default());
    assert_eq!(
        scene.removal_candidates_visited(),
        visits,
        "the instance's member label is not a world-object registration"
    );
    assert_eq!(scene.candidate_counts(), counts);
    assert_eq!(scene.resolve(&q), before);
    assert_eq!(
        scene.resolve(&q),
        scene.resolve_linear(&q),
        "index went stale"
    );
}

/// Perf sanity for removal bookkeeping (deterministic, no wall-clock),
/// pinning the contract that keeps bulk registration linear: document load
/// and undo/redo re-registration call the replace-semantics `add_*` once per
/// object on a scene where the id is not yet present, so the implicit
/// removal inside each add must not scan the candidate Vecs. A removal only
/// scans the candidate Vecs when the id is actually registered — and once
/// removed, the id is unknown again, so the idempotent second removal is
/// free too.
#[test]
fn removal_scans_candidates_once_then_never_again() {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();

    // Fresh-scene registration: the implicit removal finds nothing to do.
    scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
    assert_eq!(
        scene.removal_candidates_visited(),
        0,
        "registering into a fresh scene must not scan for stale candidates"
    );

    // Replacing a *registered* id does scan — the old candidates must go.
    scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
    let (p, s, f) = scene.candidate_counts();
    let scan = (p + s + f) as u64;
    assert_eq!(scene.removal_candidates_visited(), scan);

    // A real removal walks the Vecs once...
    scene.remove_object(ObjectId::default());
    assert_eq!(scene.removal_candidates_visited(), scan * 2);
    assert_eq!(scene.candidate_counts(), (0, 0, 0));
    // ...and removing the now-unknown id again is free.
    scene.remove_object(ObjectId::default());
    assert_eq!(scene.removal_candidates_visited(), scan * 2);

    // Instances go further: registration AND removal never scan candidates
    // at all — placements are lightweight records over shared definition
    // geometry, not candidate spans, so there is nothing to walk.
    scene.set_def_member(ObjectId::default(), &cube);
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::IDENTITY,
    );
    assert_eq!(scene.removal_candidates_visited(), scan * 2);
    scene.remove_instance(InstanceId::default());
    assert_eq!(
        scene.removal_candidates_visited(),
        scan * 2,
        "instance removal drops placement records, never candidate spans"
    );
    scene.remove_instance(InstanceId::default());
    assert_eq!(scene.removal_candidates_visited(), scan * 2);
}

#[test]
fn placement_transform_is_applied() {
    let mut scene = InferenceScene::new();
    let shift = Transform::translation(Vec3::new(10.0, 0.0, 0.0));
    scene.add_object(ObjectId::default(), &unit_cube(), &shift);
    let eye = Point3::new(13.0, 3.0, 3.0);
    let snap = resolve(
        &scene,
        query(ray_at(eye, Point3::new(11.0, 1.0, 1.0)), WIDE),
    )
    .expect("translated corner is in the cone");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(
        snap.position
            .approx_eq(Point3::new(11.0, 1.0, 1.0), tol::POINT_MERGE)
    );
}

/// Instanced candidates are keyed by their placing instance, *separately*
/// from world objects — so a definition's geometry placed by an instance does
/// not collide with, or get cleared by, world-object bookkeeping (and two
/// instances of one definition would likewise stay distinct). Here a world
/// object and an instance deliberately share the same `ObjectId` label; the
/// instance tag keeps them apart.
#[test]
fn instanced_candidates_are_keyed_separately_from_world_objects() {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();
    scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
    let world = scene.candidate_counts();
    assert!(world.0 > 0 && world.1 > 0 && world.2 > 0);

    // An instance of the same geometry adds to — never replaces — the world set.
    scene.set_def_member(ObjectId::default(), &cube);
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );
    assert_eq!(
        scene.candidate_counts(),
        (world.0 * 2, world.1 * 2, world.2 * 2),
        "placed candidates coexist with the world object's"
    );

    // Removing the instance leaves the world object untouched (the shared
    // definition geometry stays registered but placementless, contributing
    // nothing a query — or the counts — can see)...
    scene.remove_instance(InstanceId::default());
    assert_eq!(scene.candidate_counts(), world);

    // ...and removing the world object leaves a re-added instance's candidates.
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::IDENTITY,
    );
    scene.remove_object(ObjectId::default());
    assert_eq!(
        scene.candidate_counts(),
        world,
        "remove_object spares placed candidates sharing the label"
    );
}

// ---------------------------------------------------------------------------
// RR16: spatial index behind `InferenceScene`
//
// The indexed hot paths (`resolve`, `pick_face`, and the occlusion walk
// inside `resolve`) must return byte-for-byte the same results as a full
// linear scan; the scan survives as the #[doc(hidden)] linear reference
// (`resolve_linear` / `pick_face_linear`) so these specs stay honest. The
// index is invalidated per committed mutation and rebuilt lazily on the
// next query.
// ---------------------------------------------------------------------------

/// A scene exercising every candidate family the index covers (and the ones
/// it deliberately leaves linear): a world object, instances under
/// translated / rotated / non-uniformly scaled poses, guides, a sketch, and
/// a transient segment.
fn mixed_scene() -> InferenceScene {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();
    scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
    scene.set_def_member(ObjectId::default(), &cube);
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );
    let rotated = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.0, 10.0, 0.0)));
    scene.add_placement(InstanceId::default(), ObjectId::default(), &rotated);
    let squashed = Transform::scale(Vec3::new(0.5, 2.0, 1.5))
        .then(&Transform::translation(Vec3::new(10.0, 10.0, 0.0)));
    scene.add_placement(InstanceId::default(), ObjectId::default(), &squashed);
    scene.add_guide(
        GuideId::default(),
        &Guide::Line {
            origin: Point3::new(5.0, 5.0, 0.0),
            direction: Vec3::new(0.0, 0.0, 1.0),
        },
    );
    scene.add_guide(
        GuideId::default(),
        &Guide::Point {
            position: Point3::new(6.0, 2.0, 1.0),
        },
    );
    scene.add_sketch(
        SketchId::default(),
        &[
            (
                SketchEdgeId::default(),
                Point3::new(3.0, 3.0, 0.0),
                Point3::new(4.0, 3.0, 0.0),
            ),
            (
                SketchEdgeId::default(),
                Point3::new(4.0, 3.0, 0.0),
                Point3::new(4.0, 4.0, 0.0),
            ),
        ],
    );
    scene.add_transient_segment(Point3::new(2.0, 6.0, 0.0), Point3::new(2.0, 6.0, 2.0));
    scene
}

/// The indexed `resolve`/`pick_face` return exactly what the linear
/// reference returns — same winner, same position bits, same tie-breaks —
/// across a sweep of eyes, targets, apertures, locks, and constraint
/// planes over a mixed scene.
#[test]
fn indexed_queries_match_the_linear_reference() {
    let scene = mixed_scene();
    let eyes = [
        Point3::new(5.0, 5.0, 20.0),
        Point3::new(-8.0, 3.0, 2.0),
        Point3::new(15.0, 15.0, 6.0),
        Point3::new(0.5, 0.5, 4.0),
    ];
    let coords = [-1.0, 0.5, 5.0, 10.5, 12.0];
    let heights = [0.0, 0.5, 1.0];
    let apertures = [NARROW, 0.05, WIDE, 2.0];
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    for eye in eyes {
        for x in coords {
            for y in coords {
                for z in heights {
                    let ray = ray_at(eye, Point3::new(x, y, z));
                    assert_eq!(
                        scene.pick_face(&ray),
                        scene.pick_face_linear(&ray),
                        "pick_face diverged for eye {eye:?} target ({x}, {y}, {z})"
                    );
                    for aperture in apertures {
                        let mut q = query(ray, aperture);
                        assert_eq!(
                            scene.resolve(&q),
                            scene.resolve_linear(&q),
                            "resolve diverged for eye {eye:?} target ({x}, {y}, {z}) aperture {aperture}"
                        );
                        q.constraint_plane = Some(ground);
                        assert_eq!(
                            scene.resolve(&q),
                            scene.resolve_linear(&q),
                            "constrained resolve diverged for eye {eye:?} target ({x}, {y}, {z})"
                        );
                        q.constraint_plane = None;
                        q.anchor = Some(Point3::new(1.0, 1.0, 1.0));
                        q.lock = Some(SnapLock::Axis(inference::Axis::X));
                        assert_eq!(
                            scene.resolve(&q),
                            scene.resolve_linear(&q),
                            "locked resolve diverged for eye {eye:?} target ({x}, {y}, {z})"
                        );
                    }
                }
            }
        }
    }
}

/// Every mutator invalidates the lazily built index: a query, then a
/// mutation, then another query must see the new scene — a stale index
/// would keep answering from the old candidate Vecs. Each step also
/// re-checks equality with the linear reference, which reads the Vecs
/// directly and therefore cannot go stale.
#[test]
fn index_invalidation_tracks_every_mutation() {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();
    // Probes a corner with a tight cone; also asserts indexed == linear.
    fn probe(scene: &InferenceScene, target: Point3) -> Option<Snap> {
        let q = SnapQuery {
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(target.x, target.y, target.z + 5.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: NARROW,
            aperture_mode: ApertureMode::Cone,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        };
        let indexed = scene.resolve(&q);
        assert_eq!(indexed, scene.resolve_linear(&q), "index went stale");
        indexed
    }
    let corner = Point3::new(21.0, 20.0, 1.0);
    let far_corner = Point3::new(41.0, 41.0, 1.0);

    // Warm the (empty) index, then add: the object must appear.
    assert!(probe(&scene, corner).is_none());
    scene.add_object(
        ObjectId::default(),
        &cube,
        &Transform::translation(Vec3::new(20.0, 19.0, 0.0)),
    );
    assert_eq!(
        probe(&scene, corner).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );

    // Re-add under the same id (replace semantics): old placement gone.
    scene.add_object(
        ObjectId::default(),
        &cube,
        &Transform::translation(Vec3::new(40.0, 40.0, 0.0)),
    );
    assert!(probe(&scene, corner).is_none());
    assert_eq!(
        probe(&scene, far_corner).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );

    // remove_object: everything gone.
    scene.remove_object(ObjectId::default());
    assert!(probe(&scene, far_corner).is_none());

    // set_def_member + add_placement / remove_instance.
    scene.set_def_member(ObjectId::default(), &cube);
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::translation(Vec3::new(20.0, 19.0, 0.0)),
    );
    assert_eq!(
        probe(&scene, corner).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    scene.remove_instance(InstanceId::default());
    assert!(probe(&scene, corner).is_none());

    // Sketch and transient candidates stay on the linear path by design,
    // but their mutators must coexist with the index without staleness.
    scene.add_sketch(
        SketchId::default(),
        &[(
            SketchEdgeId::default(),
            Point3::new(60.0, 60.0, 0.0),
            Point3::new(62.0, 60.0, 0.0),
        )],
    );
    assert_eq!(
        probe(&scene, Point3::new(60.0, 60.0, 0.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    scene.remove_sketch(SketchId::default());
    assert!(probe(&scene, Point3::new(60.0, 60.0, 0.0)).is_none());
    scene.add_transient_segment(Point3::new(70.0, 70.0, 0.0), Point3::new(72.0, 70.0, 0.0));
    assert_eq!(
        probe(&scene, Point3::new(70.0, 70.0, 0.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    scene.clear_transient();
    assert!(probe(&scene, Point3::new(70.0, 70.0, 0.0)).is_none());
}

/// The occlusion early-out must not change occlusion *semantics*: a ray
/// down an imprinted hole still reaches the coplanar sub-face (holes punch
/// through occlusion), a wide cone over a solid still refuses to dive to
/// hidden back geometry, and both answers equal the linear reference — in
/// a scene crowded enough that the index genuinely prunes.
#[test]
fn occlusion_early_out_respects_holes_and_matches_linear() {
    let mut cube = unit_cube();
    let top = cube
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        })
        .map(|(id, _)| id)
        .unwrap();
    cube.split_face_inner(
        top,
        &[
            Point3::new(0.25, 0.25, 1.0),
            Point3::new(0.75, 0.25, 1.0),
            Point3::new(0.75, 0.75, 1.0),
            Point3::new(0.25, 0.75, 1.0),
        ],
    )
    .unwrap();
    let mut scene = InferenceScene::new();
    scene.add_object(ObjectId::default(), &cube, &Transform::IDENTITY);
    // A crowd of plain cubes so the occlusion walk has subtrees to skip.
    // NOTE: the member label deliberately collides with the world object's
    // id — shared definition storage must not bleed into world candidates.
    let plain = unit_cube();
    scene.set_def_member(ObjectId::default(), &plain);
    for gx in 0..5 {
        for gy in 0..5 {
            scene.add_placement(
                InstanceId::default(),
                ObjectId::default(),
                &Transform::translation(Vec3::new(
                    10.0 + 3.0 * gx as f64,
                    10.0 + 3.0 * gy as f64,
                    0.0,
                )),
            );
        }
    }

    // Down the hole centre: the sub-face at z = 1 stays visible.
    let through_hole = query(
        PickRay {
            origin: Point3::new(0.5, 0.5, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        NARROW,
    );
    let snap = scene
        .resolve(&through_hole)
        .expect("the sub-face seen through the hole is visible");
    assert!(
        (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
        "hole must punch through occlusion, got {:?}",
        snap.position
    );
    assert_eq!(
        scene.resolve(&through_hole),
        scene.resolve_linear(&through_hole)
    );

    // A wide cone into the top-face interior also catches the hidden bottom
    // corner (an Endpoint, which outranks OnFace): occlusion must cull it.
    let wide_over_solid = query(
        PickRay {
            origin: Point3::new(0.3, 0.3, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        0.6,
    );
    let snap = scene
        .resolve(&wide_over_solid)
        .expect("something visible in the wide cone");
    assert!(
        snap.position.z > 0.5,
        "occluded back geometry must not win: {:?}",
        snap.position
    );
    assert_eq!(
        scene.resolve(&wide_over_solid),
        scene.resolve_linear(&wide_over_solid)
    );
}

/// Perf sanity (deterministic, no wall-clock): on a ~10k-face scene, one
/// occlusion-culled `resolve` must run exact ray-vs-face tests on far
/// fewer faces than the linear reference, which by construction tests all
/// of them. `occlusion_face_tests` counts exact tests on both paths.
#[test]
fn occlusion_on_a_large_scene_tests_a_small_fraction_of_faces() {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();
    scene.set_def_member(ObjectId::default(), &cube);
    for gx in 0..41 {
        for gy in 0..41 {
            scene.add_placement(
                InstanceId::default(),
                ObjectId::default(),
                &Transform::translation(Vec3::new(3.0 * gx as f64, 3.0 * gy as f64, 0.0)),
            );
        }
    }
    let total_faces = scene.candidate_counts().2 as u64;
    assert_eq!(total_faces, 41 * 41 * 6, "the grid registered 10k+ faces");
    assert_eq!(
        scene.def_extractions(),
        1,
        "1681 placements of one member cost exactly one extraction"
    );

    // Case 1 — visible winner: straight down at the centre of one cube's
    // top face. The winner's own face enters the ray only at ~its own
    // depth, beyond the `near_threshold` prune, so the walk may need zero
    // exact tests at all (the strongest possible pruning); the linear
    // reference still tests every face and rejects each one.
    let q = query(
        PickRay {
            origin: Point3::new(0.5, 0.5, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        NARROW,
    );

    let before = scene.occlusion_face_tests();
    let indexed = scene.resolve(&q).expect("the top face is under the cursor");
    let indexed_tests = scene.occlusion_face_tests() - before;
    assert_eq!(indexed.kind, SnapKind::OnFace);

    let before = scene.occlusion_face_tests();
    let linear = scene.resolve_linear(&q);
    let linear_tests = scene.occlusion_face_tests() - before;
    assert_eq!(Some(indexed), linear, "indexed and linear answers agree");

    assert!(
        linear_tests >= total_faces,
        "the linear reference scans every face ({linear_tests} < {total_faces})"
    );
    assert!(
        indexed_tests * 20 <= linear_tests,
        "the index must prune at least 20x: {indexed_tests} exact tests vs {linear_tests} linear"
    );
    assert!(
        indexed_tests < 100,
        "an occlusion query on 10k faces stays local: {indexed_tests} exact tests"
    );

    // Case 2 — occluded candidates: a wide cone into the top-face interior
    // also catches hidden bottom corners (Endpoints, which outrank OnFace),
    // so the winner search must actually *find* occluders. The early-out
    // stops at the first occluding face; the linear reference still tests
    // every face once per is_occluded call.
    let wide = query(
        PickRay {
            origin: Point3::new(0.3, 0.3, 4.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        0.6,
    );

    let before = scene.occlusion_face_tests();
    let indexed = scene.resolve(&wide).expect("plenty in the wide cone");
    let indexed_tests = scene.occlusion_face_tests() - before;
    assert!(indexed.position.z > 0.5, "hidden bottom geometry culled");

    let before = scene.occlusion_face_tests();
    let linear = scene.resolve_linear(&wide);
    let linear_tests = scene.occlusion_face_tests() - before;
    assert_eq!(Some(indexed), linear, "indexed and linear answers agree");

    assert!(
        indexed_tests >= 1,
        "occluded candidates force real exact tests"
    );
    assert!(
        linear_tests >= total_faces,
        "every is_occluded call in the linear reference scans all faces"
    );
    assert!(
        indexed_tests * 20 <= linear_tests,
        "the index must prune at least 20x: {indexed_tests} exact tests vs {linear_tests} linear"
    );
    assert!(
        indexed_tests < 200,
        "occlusion stays local even with several candidates: {indexed_tests} exact tests"
    );
}

/// Regression: at pick-cone apertures small enough that `cone_test`'s
/// `acos(depth / dist)` saturates (the independently rounded dot product
/// and length collapse to the same f64, so cos == 1.0 and the computed
/// angle is exactly 0), the exact test admits candidates with a true
/// angular offset of up to ~2e-8 rad *outside* the cone. The spatial
/// index's node test is built from exact geometry, so without a guard
/// band it pruned such candidates and `resolve` diverged from
/// `resolve_linear` — here by losing the vertex Endpoint and falling back
/// to the coincident world-origin candidate, stripping the snap of its
/// element provenance (in other scenes the snap is lost outright).
///
/// Geometry pinned to the found divergence: a thin plate whose points-BVH
/// root box has its bounding-sphere silhouette almost tangent to the pick
/// ray, a corner endpoint at depth 13 m, and a ray offset ~1.5e-7 m
/// laterally — inside `cone_test`'s saturation window, but ~1.15e-8 rad
/// off axis, outside the 1e-8 rad aperture.
#[test]
fn tiny_aperture_saturation_still_matches_the_linear_reference() {
    let plate = Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1e-4, 0.0),
            Point3::new(0.0, 1e-4, 0.0),
            Point3::new(0.0, 0.0, 2.0),
            Point3::new(2.0, 0.0, 2.0),
            Point3::new(2.0, 1e-4, 2.0),
            Point3::new(0.0, 1e-4, 2.0),
        ],
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .unwrap();
    let mut scene = InferenceScene::new();
    scene.add_object(ObjectId::default(), &plate, &Transform::IDENTITY);

    let q = query(
        PickRay {
            origin: Point3::new(-1.0606601717798211e-7, -13.0, -1.0606601717798211e-7),
            direction: Vec3::new(0.0, 1.0, 0.0),
        },
        1e-8,
    );

    let linear = scene.resolve_linear(&q);
    let snap = linear.expect("the saturated exact test accepts the corner");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(Point3::ORIGIN, tol::POINT_MERGE));
    assert!(
        snap.source.is_some(),
        "the winner is the plate's vertex, not the world origin"
    );
    assert_eq!(scene.resolve(&q), linear, "index must never prune a snap");
}

/// `clear_solids` drops every object- and instance-sourced candidate in one
/// scan-free call — the bulk visibility-rebuild primitive — while guides,
/// sketches, and transient segments stay registered and the index never goes
/// stale. Fresh registrations afterwards keep the scan-free replace-semantics
/// fast path (their owners were cleared too).
#[test]
fn clear_solids_drops_solids_and_keeps_guides_and_sketches() {
    let corner = Point3::new(1.0, 1.0, 1.0);
    let q = query(ray_at(Point3::new(3.0, 3.0, 3.0), corner), WIDE);

    let mut scene = cube_scene();
    scene.set_def_member(ObjectId::default(), &unit_cube());
    scene.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );
    let guide_target = Point3::new(6.0, 2.0, 1.0);
    scene.add_guide(
        GuideId::default(),
        &Guide::Point {
            position: guide_target,
        },
    );
    scene.add_sketch(
        SketchId::default(),
        &[(
            SketchEdgeId::default(),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(4.0, 3.0, 0.0),
        )],
    );

    // Warm the index, then clear: no candidate scan, solids gone, index fresh.
    assert_eq!(scene.resolve(&q).map(|s| s.kind), Some(SnapKind::Endpoint));
    let visits = scene.removal_candidates_visited();
    scene.clear_solids();
    assert_eq!(
        scene.removal_candidates_visited(),
        visits,
        "clear_solids must not scan candidates"
    );
    assert_eq!(scene.candidate_counts(), (0, 0, 0));
    assert_eq!(scene.guide_count(), 1);
    assert_eq!(
        scene.resolve(&q),
        scene.resolve_linear(&q),
        "index went stale across clear_solids"
    );

    // Guides and sketches survive: both still resolve as snap targets.
    let on_guide = scene.resolve(&query(
        ray_at(Point3::new(8.0, 4.0, 3.0), guide_target),
        WIDE,
    ));
    assert!(
        on_guide.is_some(),
        "guide candidates must survive clear_solids"
    );
    let sketch_end = Point3::new(4.0, 3.0, 0.0);
    let on_sketch = scene.resolve(&query(ray_at(Point3::new(5.0, 4.0, 2.0), sketch_end), WIDE));
    assert!(
        on_sketch.is_some(),
        "sketch candidates must survive clear_solids"
    );

    // Re-registration after the clear stays scan-free and fully functional.
    scene.add_object(ObjectId::default(), &unit_cube(), &Transform::IDENTITY);
    assert_eq!(
        scene.removal_candidates_visited(),
        visits,
        "re-registration after clear_solids must hit the empty-owner fast path"
    );
    let snap = scene
        .resolve(&q)
        .expect("cube snaps again after re-registration");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

// ---------------------------------------------------------- center snapping

/// A faceted cylinder whose walls carry their analytic surface reference:
/// built the way the app builds one — a circle chain with geometry, closed
/// into a region, extruded.
fn analytic_cylinder(center: Point3, radius: f64, n: usize, height: f64) -> Object {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut s = kernel::Sketch::on_plane(plane);
    s.begin_curve_with(kernel::CurveGeom { center, radius })
        .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
            0.0,
        )
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let region = s.regions().keys().next().unwrap();
    Object::from_extrusion(&s.profile(region).unwrap(), height).unwrap()
}

/// The true center of an extruded circle's cap snaps as `SnapKind::Center` —
/// the exact drawn center, which is NOT any facet vertex or midpoint.
#[test]
fn cap_center_snaps_as_center() {
    let mut scene = InferenceScene::new();
    let cyl = analytic_cylinder(Point3::new(1.0, 2.0, 0.0), 0.5, 24, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);

    // Aim straight down at the TOP cap's center from above.
    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(1.0, 2.0, 5.0), Point3::new(1.0, 2.0, 1.0)),
            NARROW,
        ))
        .expect("center candidate resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(
        snap.position
            .approx_eq(Point3::new(1.0, 2.0, 1.0), tol::POINT_MERGE)
    );
    // Provenance points at a claiming wall face of the owning object.
    let source = snap.source.expect("centers carry object provenance");
    assert!(matches!(source.element, ElementRef::Face(_)));
}

/// The bottom cap's center is occluded by the solid when viewed from above:
/// only the visible center snaps (the same only-what-you-see rule as every
/// other candidate).
#[test]
fn occluded_center_does_not_snap_through_the_solid() {
    let mut scene = InferenceScene::new();
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 1.0, 24, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);

    // From above, aiming at the BOTTOM center (z=0): the top cap hides it,
    // so the resolved snap must not be the bottom center.
    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(0.0, 0.0, 5.0), Point3::new(0.0, 0.0, 0.0)),
            NARROW,
        ))
        .expect("something under the cursor resolves");
    if snap.kind == SnapKind::Center {
        assert!(
            snap.position
                .approx_eq(Point3::new(0.0, 0.0, 1.0), tol::POINT_MERGE),
            "only the visible (top) center may snap, got {:?}",
            snap.position
        );
    }
}

/// A plain faceted prism (no analytic reference) produces no Center
/// candidates — centers derive from metadata, never from facet geometry.
#[test]
fn unattributed_prism_has_no_center_candidates() {
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut s = kernel::Sketch::on_plane(plane);
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / 24.0;
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..24 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    let region = s.regions().keys().next().unwrap();
    let prism = Object::from_extrusion(&s.profile(region).unwrap(), 1.0).unwrap();
    scene.add_object(ObjectId::default(), &prism, &Transform::IDENTITY);

    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(0.0, 0.0, 5.0), Point3::new(0.0, 0.0, 1.0)),
            NARROW,
        ))
        .expect("the cap face still resolves");
    assert_ne!(snap.kind, SnapKind::Center, "no metadata, no center snap");
}

/// A rim with ZERO surviving arc offers no Center: slant-cut the entire top
/// off a cylinder and the top "rim" is a circle of which nothing remains —
/// its center is a fabricated point floating in the air above the slanted
/// face. Only the intact bottom rim's center may snap
/// (the true-curves design, review follow-up: per-rim coverage gates
/// Center exactly like Quadrant/Tangent).
#[test]
fn slant_cut_rim_offers_no_phantom_center() {
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 1.0, 24, 1.0);
    let cutter = {
        // Quad on the plane z = 0.6 - 0.2x, extruded along its normal: the
        // prism's slanted bottom face slices the whole top off.
        let corners = [
            Point3::new(-3.0, -3.0, 1.2),
            Point3::new(3.0, -3.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(-3.0, 3.0, 1.2),
        ];
        let plane = Plane::from_polygon(&corners).unwrap();
        let mut cs = kernel::Sketch::on_plane(plane);
        for i in 0..4 {
            cs.add_segment(corners[i], corners[(i + 1) % 4]).unwrap();
        }
        let region = cs.regions().keys().next().unwrap();
        Object::from_extrusion(&cs.profile(region).unwrap(), 2.0).unwrap()
    };
    let obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();

    let mut scene = InferenceScene::new();
    scene.set_axes_enabled(false); // the ambient Z axis is under these rays
    scene.add_object(ObjectId::default(), &obj, &Transform::IDENTITY);

    // The phantom top center would sit at (0,0,0.8) — the highest surviving
    // vertex's station — 0.2 above the slant face, occluded by nothing.
    // Aiming straight at it must NOT resolve a Center there.
    if let Some(snap) = scene.resolve(&query(
        ray_at(Point3::new(0.0, 0.0, 5.0), Point3::new(0.0, 0.0, 0.8)),
        NARROW,
    )) {
        assert!(
            !(snap.kind == SnapKind::Center
                && snap
                    .position
                    .approx_eq(Point3::new(0.0, 0.0, 0.8), tol::POINT_MERGE)),
            "a Center snapped at the fabricated point of a rim with zero \
             surviving arc: {snap:?}"
        );
    }

    // The intact bottom rim still offers its center (aimed from below,
    // where nothing occludes it).
    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(0.0, 0.0, -5.0), Point3::new(0.0, 0.0, 0.0)),
            NARROW,
        ))
        .expect("bottom center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(
        snap.position
            .approx_eq(Point3::new(0.0, 0.0, 0.0), tol::POINT_MERGE)
    );
}

/// Removing the object removes its center candidates (replace semantics and
/// idempotent removal, like every other candidate kind).
#[test]
fn center_candidates_are_removed_with_their_object() {
    let mut scene = InferenceScene::new();
    scene.set_axes_enabled(false); // the ambient Z axis is under this ray
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 1.0, 12, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);
    scene.remove_object(ObjectId::default());
    assert!(
        scene
            .resolve(&query(
                ray_at(Point3::new(0.0, 0.0, 5.0), Point3::new(0.0, 0.0, 1.0)),
                NARROW,
            ))
            .is_none(),
        "no candidates survive removal"
    );
}

/// A Center beats derived candidates (midpoints/edges) but loses to a real
/// endpoint at the same screen position — the documented priority order.
#[test]
fn center_priority_sits_between_endpoint_and_midpoint() {
    assert!(SnapKind::Endpoint < SnapKind::Center);
    assert!(SnapKind::Center < SnapKind::Midpoint);
}

/// An instanced placement's centers follow the instance pose and are keyed
/// to the instance for removal.
#[test]
fn instanced_centers_follow_the_pose_and_the_instance_key() {
    let mut scene = InferenceScene::new();
    scene.set_axes_enabled(false); // keep the post-removal probe unambiguous
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 0.5, 12, 1.0);
    let inst = InstanceId::default();
    let pose = Transform::translation(Vec3::new(10.0, 0.0, 0.0));
    scene.set_def_member(ObjectId::default(), &cyl);
    scene.add_placement(inst, ObjectId::default(), &pose);

    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(10.0, 0.0, 5.0), Point3::new(10.0, 0.0, 1.0)),
            NARROW,
        ))
        .expect("instanced center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(
        snap.position
            .approx_eq(Point3::new(10.0, 0.0, 1.0), tol::POINT_MERGE)
    );
    assert_eq!(snap.source.unwrap().instance, Some(inst));

    scene.remove_instance(inst);
    assert!(
        scene
            .resolve(&query(
                ray_at(Point3::new(10.0, 0.0, 5.0), Point3::new(10.0, 0.0, 1.0)),
                NARROW,
            ))
            .is_none()
    );
}

// ------------------------------------------------ quadrant + tangent snaps

/// Tighter than [`NARROW`]: quadrant/tangent points on a fine polygon sit a
/// few centimeters from real vertices, whose Endpoint candidates outrank
/// them; the pin cone isolates the exact analytic point under test.
const PIN: f64 = 0.003;

/// The upper half-disc (semicircular analytic arc closed by a chord),
/// extruded — a partial cylinder band whose rims cover only y >= 0.
fn analytic_half_cylinder(radius: f64, n: usize, height: f64) -> Object {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut s = kernel::Sketch::on_plane(plane);
    s.begin_curve_with(kernel::CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius,
    })
    .unwrap();
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s.add_segment(
        Point3::new(-radius, 0.0, 0.0),
        Point3::new(radius, 0.0, 0.0),
    )
    .unwrap();
    let region = s.regions().keys().next().unwrap();
    Object::from_extrusion(&s.profile(region).unwrap(), height).unwrap()
}

/// A rim quadrant point of a drawn circle snaps as `SnapKind::Quadrant` —
/// the TRUE circle's cardinal point. With a facet vertex coinciding there
/// (multiple-of-4 counts put vertices at the quadrants), Endpoint wins on
/// priority; so probe a 26-gon, whose vertices straddle the cardinal.
#[test]
fn rim_quadrant_snaps_as_quadrant() {
    let mut scene = InferenceScene::new();
    let cyl = analytic_cylinder(Point3::new(1.0, 2.0, 0.0), 0.5, 26, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);

    // +Y cardinal of the top rim: (1, 2.5, 1). A 26-gon has no vertex at
    // 90° (vertices sit at k·360/26 ≈ 13.85°·k, and the drawn polygon
    // starts at +X), so the exact-cardinal point is between two vertices,
    // slightly outside their chord, on the exact circle. The aperture is
    // tight enough to exclude the neighboring vertices (~0.06 m away).
    let target = Point3::new(1.0, 2.5, 1.0);
    let snap = scene
        .resolve(&query(ray_at(Point3::new(1.0, 5.0, 4.0), target), PIN))
        .expect("quadrant candidate resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(snap.position.approx_eq(target, tol::POINT_MERGE));
    let source = snap.source.expect("quadrants carry object provenance");
    assert!(matches!(source.element, ElementRef::Face(_)));
    // Indexed and reference paths agree (quadrants stay off the index).
    let q = query(ray_at(Point3::new(1.0, 5.0, 4.0), target), PIN);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// A partial arc offers quadrant points only over its covered range: the
/// half cylinder's -Y cardinal does not exist anywhere in the scene.
#[test]
fn uncovered_quadrant_of_a_partial_arc_never_snaps() {
    let mut scene = InferenceScene::new();
    let half = analytic_half_cylinder(1.0, 13, 0.5);
    scene.add_object(ObjectId::default(), &half, &Transform::IDENTITY);

    // Covered apex quadrant (+Y) snaps…
    let apex = Point3::new(0.0, 1.0, 0.5);
    let snap = scene
        .resolve(&query(ray_at(Point3::new(0.0, 4.0, 3.0), apex), NARROW))
        .expect("apex quadrant resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(snap.position.approx_eq(apex, tol::POINT_MERGE));

    // …but aiming at where the -Y quadrant WOULD be finds nothing at all
    // (there is no geometry there — the solid's flat chord wall is at y=0).
    let phantom = Point3::new(0.0, -1.0, 0.25);
    let miss = scene.resolve(&query(
        ray_at(Point3::new(0.0, -4.0, 0.25), phantom),
        NARROW,
    ));
    assert!(
        miss.is_none()
            || !miss
                .expect("checked")
                .position
                .approx_eq(phantom, tol::POINT_MERGE),
        "the uncovered quadrant must not be offered"
    );
}

/// With an anchor set, the rim point where the segment from the anchor
/// touches the exact circle snaps as `SnapKind::Tangent`; the returned
/// point satisfies tangency exactly (radius vector perpendicular to the
/// anchor segment).
#[test]
fn tangent_from_anchor_snaps_on_the_true_circle() {
    let mut scene = InferenceScene::new();
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 1.0, 26, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);

    let anchor = Point3::new(3.0, 0.0, 1.0);
    // Expected tangent point in the top rim plane: alpha = acos(r/d).
    let alpha = (1.0f64 / 3.0).acos();
    // The tangent point at +Y side: (cos a, sin a) relative to the +X
    // direction of the anchor.
    let expected = Point3::new(alpha.cos(), alpha.sin(), 1.0);

    let mut q = query(ray_at(Point3::new(2.0, 3.0, 4.0), expected), PIN);
    q.anchor = Some(anchor);
    let snap = scene.resolve(&q).expect("tangent candidate resolves");
    assert_eq!(snap.kind, SnapKind::Tangent);
    assert!(snap.position.approx_eq(expected, 1e-9));
    // Exact tangency: (p - c) ⟂ (p - anchor).
    let radial = snap.position - Point3::new(0.0, 0.0, 1.0);
    let along = snap.position - anchor;
    assert!(radial.dot(along).abs() <= 1e-9);

    // No anchor, no tangent: the same ray without an anchor resolves to
    // something else (or nothing).
    let bare = scene.resolve(&query(ray_at(Point3::new(2.0, 3.0, 4.0), expected), PIN));
    assert!(bare.is_none_or(|s| s.kind != SnapKind::Tangent));
}

/// Tangent points obey coverage: the half cylinder never offers a tangent
/// on its missing (y < 0) side.
#[test]
fn tangent_respects_the_covered_angular_range() {
    let mut scene = InferenceScene::new();
    let half = analytic_half_cylinder(1.0, 13, 0.5);
    scene.add_object(ObjectId::default(), &half, &Transform::IDENTITY);

    // Anchor on +X far side: tangent points sit at ±alpha off the +X
    // direction — one on the covered +Y side, one on the uncovered -Y side.
    let anchor = Point3::new(3.0, 0.0, 0.5);
    let alpha = (1.0f64 / 3.0).acos();
    let covered = Point3::new(alpha.cos(), alpha.sin(), 0.5);
    let uncovered = Point3::new(alpha.cos(), -alpha.sin(), 0.5);

    let mut q = query(ray_at(Point3::new(2.0, 3.0, 3.0), covered), PIN);
    q.anchor = Some(anchor);
    let snap = scene.resolve(&q).expect("covered tangent resolves");
    assert_eq!(snap.kind, SnapKind::Tangent);
    assert!(snap.position.approx_eq(covered, 1e-9));

    let mut q2 = query(ray_at(Point3::new(2.0, -3.0, 3.0), uncovered), NARROW);
    q2.anchor = Some(anchor);
    let miss = scene.resolve(&q2);
    assert!(
        miss.is_none_or(|s| s.kind != SnapKind::Tangent),
        "no tangent on the uncovered side"
    );
}

/// An anchor inside the circle has no tangent lines; nothing is offered.
#[test]
fn anchor_inside_the_circle_offers_no_tangent() {
    let mut scene = InferenceScene::new();
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 1.0, 26, 1.0);
    scene.add_object(ObjectId::default(), &cyl, &Transform::IDENTITY);

    let mut q = query(
        ray_at(Point3::new(2.0, 3.0, 4.0), Point3::new(0.9, 0.43, 1.0)),
        WIDE,
    );
    q.anchor = Some(Point3::new(0.2, 0.1, 1.0));
    let snap = scene.resolve(&q);
    assert!(snap.is_none_or(|s| s.kind != SnapKind::Tangent));
}

/// Quadrant and tangent candidates are removed with their object, exactly
/// like centers.
#[test]
fn quadrant_and_tangent_candidates_die_with_their_object() {
    let mut scene = InferenceScene::new();
    let id = ObjectId::default();
    let cyl = analytic_cylinder(Point3::new(1.0, 2.0, 0.0), 0.5, 26, 1.0);
    scene.add_object(id, &cyl, &Transform::IDENTITY);
    scene.remove_object(id);

    let target = Point3::new(1.5, 2.0, 1.0);
    assert!(
        scene
            .resolve(&query(ray_at(Point3::new(4.0, 2.0, 4.0), target), NARROW))
            .is_none(),
        "no candidates survive removal"
    );
}

/// Analytic candidates materialized through a component PLACEMENT (not a
/// world object) obey the same index/linear-parity contract as world
/// objects, for Center, Quadrant, and Tangent alike — the placed-rim
/// path (`DefMember::rims_at`) must be a faithful mirror of the world path.
/// The property fuzz only generates boxes/tetras, so this is the sole guard
/// against a placement/mirror regression in curved-instance snapping.
#[test]
fn placed_analytic_candidates_match_the_linear_reference() {
    let mut scene = InferenceScene::new();
    let member = ObjectId::default();
    let cyl = analytic_cylinder(Point3::new(0.0, 0.0, 0.0), 0.5, 26, 1.0);
    scene.set_def_member(member, &cyl);

    // A plain translated placement: its top-rim center, a +Y quadrant, and
    // an anchored tangent must each snap AND agree between the two paths.
    scene.add_placement(
        InstanceId::default(),
        member,
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );

    // Center at (10, 0, 1).
    let center = Point3::new(10.0, 0.0, 1.0);
    let cq = query(ray_at(Point3::new(10.0, 0.0, 4.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("placed center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));
    assert_eq!(
        snap.source
            .expect("placed center carries provenance")
            .instance,
        Some(InstanceId::default())
    );
    assert_eq!(scene.resolve(&cq), scene.resolve_linear(&cq));

    // +Y quadrant at (10, 0.5, 1) (26-gon: no vertex at 90°, so Quadrant
    // wins over the straddling Endpoints under the pin cone).
    let quad = Point3::new(10.0, 0.5, 1.0);
    let qq = query(ray_at(Point3::new(10.0, 3.0, 4.0), quad), PIN);
    let snap = scene.resolve(&qq).expect("placed quadrant resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(snap.position.approx_eq(quad, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&qq), scene.resolve_linear(&qq));

    // Anchored tangent: anchor at (13, 0, 1), tangent at +alpha off +X.
    let alpha = (0.5f64 / 3.0).acos();
    let tan = Point3::new(10.0 + 0.5 * alpha.cos(), 0.5 * alpha.sin(), 1.0);
    let mut tq = query(ray_at(Point3::new(10.0, 3.0, 4.0), tan), PIN);
    tq.anchor = Some(Point3::new(13.0, 0.0, 1.0));
    let snap = scene.resolve(&tq).expect("placed tangent resolves");
    assert_eq!(snap.kind, SnapKind::Tangent);
    assert_eq!(scene.resolve(&tq), scene.resolve_linear(&tq));

    // A MIRRORED placement (reflection, negative determinant): a similarity,
    // so its tangent rim survives the map-or-drop gate. Its analytic
    // candidates must still agree between the two paths. `scale(-1,1,1)`
    // then translate clear to (0, 20, 0): (x,y,z) -> (-x, y+20, z).
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(0.0, 20.0, 0.0)));
    scene.add_placement(InstanceId::default(), member, &mirror);

    // Mirrored top-rim center at (0, 20, 1).
    let mcenter = Point3::new(0.0, 20.0, 1.0);
    let mcq = query(ray_at(Point3::new(0.0, 20.0, 4.0), mcenter), NARROW);
    let snap = scene
        .resolve(&mcq)
        .expect("mirrored placed center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(snap.position.approx_eq(mcenter, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&mcq), scene.resolve_linear(&mcq));

    // A mirrored quadrant at (0, 20.5, 1), and a mirrored anchored tangent —
    // the reflected basis path must stay index/linear consistent.
    let mquad = Point3::new(0.0, 20.5, 1.0);
    let mqq = query(ray_at(Point3::new(0.0, 23.0, 4.0), mquad), PIN);
    let snap = scene
        .resolve(&mqq)
        .expect("mirrored placed quadrant resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert_eq!(scene.resolve(&mqq), scene.resolve_linear(&mqq));

    let mtan_anchor = Point3::new(0.0, 23.0, 1.0);
    let malpha = (0.5f64 / 3.0).acos();
    // Anchor is +Y of the mirrored center; tangent at +alpha off +Y.
    let mtan = Point3::new(-0.5 * malpha.sin(), 20.0 + 0.5 * malpha.cos(), 1.0);
    let mut mtq = query(ray_at(Point3::new(-2.0, 21.0, 4.0), mtan), PIN);
    mtq.anchor = Some(mtan_anchor);
    // Whatever it resolves to, the two paths must agree (the load-bearing
    // parity claim — never vacuous, since the center/quadrant above proved
    // the mirrored placement emits analytic candidates).
    assert_eq!(scene.resolve(&mtq), scene.resolve_linear(&mtq));
}

/// An axis-aligned cube of edge length `s` at the origin (the unit cube's
/// shape, scaled) — a second definition "revision" for the edit spec below.
fn scaled_cube(s: f64) -> Object {
    Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(s, 0.0, 0.0),
            Point3::new(s, s, 0.0),
            Point3::new(0.0, s, 0.0),
            Point3::new(0.0, 0.0, s),
            Point3::new(s, 0.0, s),
            Point3::new(s, s, s),
            Point3::new(0.0, s, s),
        ],
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .unwrap()
}

/// The shared-storage contract end to end: N placements of one member cost
/// exactly one extraction, and a definition edit — invalidate, re-extract
/// once, re-register placements, the order wasm-api's reconcile uses —
/// propagates to every placement, with the indexed path never going stale
/// against the linear reference.
#[test]
fn definition_geometry_extracts_once_and_edits_propagate_to_all_placements() {
    let mut scene = InferenceScene::new();
    let place_all = |scene: &mut InferenceScene| {
        for i in 0..8 {
            scene.add_placement(
                InstanceId::default(),
                ObjectId::default(),
                &Transform::translation(Vec3::new(4.0 * i as f64, 0.0, 0.0)),
            );
        }
    };
    scene.set_def_member(ObjectId::default(), &unit_cube());
    place_all(&mut scene);
    assert_eq!(
        scene.def_extractions(),
        1,
        "eight placements share one extraction"
    );

    // The second placement's far corner snaps where its pose puts it.
    let corner = Point3::new(5.0, 1.0, 1.0);
    let q = query(ray_at(Point3::new(8.0, 4.0, 4.0), corner), NARROW);
    let snap = scene.resolve(&q).expect("posed corner snaps");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(corner, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    // Definition edit: drop the member (which drops its placements), extract
    // the new revision once, re-register the placements.
    scene.remove_def_member(ObjectId::default());
    assert_eq!(
        scene.candidate_counts(),
        (0, 0, 0),
        "placements can't outlive their member's geometry"
    );
    scene.set_def_member(ObjectId::default(), &scaled_cube(2.0));
    place_all(&mut scene);
    assert_eq!(scene.def_extractions(), 2, "the edit re-extracts once");

    // Every placement now resolves against the new geometry: the second
    // placement's far corner moved from (5,1,1) to (6,2,2)...
    let grown = Point3::new(6.0, 2.0, 2.0);
    let q_grown = query(ray_at(Point3::new(9.0, 5.0, 5.0), grown), NARROW);
    let snap = scene.resolve(&q_grown).expect("grown corner snaps");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(grown, tol::POINT_MERGE));
    // ...and the old corner position is no longer a vertex (an edge point at
    // most), so stale geometry would be caught here.
    let stale = scene.resolve(&q);
    assert_ne!(
        stale.map(|s| (s.kind, s.position)),
        Some((SnapKind::Endpoint, corner)),
        "old revision's corner must not snap as a vertex"
    );
    assert_eq!(scene.resolve(&q_grown), scene.resolve_linear(&q_grown));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

// ------------------------------------------------------- sketch curve rims
//
// A drawn (unextruded) circle or arc must snap at its exact center, covered
// quadrants, and anchored tangents in EVERY context — ground sketch or
// detached standing sketch — before any extrusion exists. These candidates
// come from `Sketch::curve_rims` registered via `add_sketch_curves`;
// historically Center/Quadrant existed only for solids' analytic rims, so a
// bare drawn circle had no center point at all.

/// Builds a sketch on `plane` with one `n`-gon circle chain (center on the
/// plane) and returns it.
fn sketch_with_circle(plane: Plane, center: Point3, radius: f64, n: usize) -> kernel::Sketch {
    let mut s = kernel::Sketch::on_plane(plane);
    s.begin_curve_with(kernel::CurveGeom { center, radius })
        .unwrap();
    let (u, v) = {
        // Any in-plane frame works for authoring the facets; reuse the
        // rim's own basis so the vertices land off the cardinals is not
        // needed — n is chosen so no vertex sits on a quadrant point.
        let normal = plane.normal();
        let reference = if normal.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let u = normal.cross(reference).normalized().unwrap();
        (u, normal.cross(u).normalized().unwrap())
    };
    for i in 0..n {
        // Half-facet phase: no vertex lands on a cardinal, so Quadrant
        // candidates aren't shadowed by an Endpoint at the same spot.
        let a0 = 2.0 * std::f64::consts::PI * (i as f64 + 0.5) / (n as f64);
        let a1 = 2.0 * std::f64::consts::PI * (i as f64 + 1.5) / (n as f64);
        let p0 = center + u * (radius * a0.cos()) + v * (radius * a0.sin());
        let p1 = center + u * (radius * a1.cos()) + v * (radius * a1.sin());
        s.add_segment(p0, p1).unwrap();
    }
    s.end_curve();
    s
}

fn register_sketch_full(scene: &mut InferenceScene, id: SketchId, s: &kernel::Sketch) {
    // Mirror the production path (`Scene::register_sketch`): segments carry
    // their owning curve so a facet vertex's Endpoint snap names the curve.
    let segments: Vec<_> = s
        .edges()
        .iter()
        .map(|(eid, e)| {
            (
                eid,
                e.curve,
                s.vertices()[e.from].position,
                s.vertices()[e.to].position,
            )
        })
        .collect();
    scene.add_sketch_edges(id, &segments);
    scene.add_sketch_curves(id, &s.curve_rims());
}

#[test]
fn sketch_circle_center_snaps_before_any_extrusion() {
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = Point3::new(1.0, 2.0, 0.0);
    // 24-gon (a multiple of 4): with the half-facet phase no vertex lands
    // on ANY cardinal, so Quadrant candidates aren't shadowed by Endpoints.
    let s = sketch_with_circle(plane, center, 0.5, 24);
    register_sketch_full(&mut scene, SketchId::default(), &s);

    // The exact center resolves as Center. No Object provenance (a sketch
    // curve is not a SnapSource element — like a guide, source is None) and
    // no EDGE provenance (the center lies on no edge), but it does name the
    // curve chain it belongs to; see
    // `drawn_curve_analytic_points_name_their_curve`.
    let cq = query(ray_at(Point3::new(1.0, 2.0, 3.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("drawn circle center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));
    assert_eq!(snap.source, None);
    assert_eq!(snap.sketch_source, None);
    assert_eq!(scene.resolve(&cq), scene.resolve_linear(&cq));

    // A quadrant point resolves as Quadrant (exactly on the true circle).
    let rims = s.curve_rims();
    let q0 = rims[0].quadrant_points()[0];
    let qq = query(ray_at(q0 + Vec3::new(0.0, 0.0, 3.0), q0), PIN);
    let snap = scene.resolve(&qq).expect("drawn circle quadrant resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(snap.position.approx_eq(q0, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&qq), scene.resolve_linear(&qq));

    // An anchored tangent resolves on the exact circle.
    let rim = &rims[0];
    let anchor = center + rim.basis_u * 1.5;
    let alpha = (0.5f64 / 1.5).acos();
    let tan = center + rim.basis_u * (0.5 * alpha.cos()) + rim.basis_v * (0.5 * alpha.sin());
    let mut tq = query(ray_at(tan + Vec3::new(0.0, 0.0, 3.0), tan), PIN);
    tq.anchor = Some(anchor);
    let snap = scene.resolve(&tq).expect("drawn circle tangent resolves");
    assert_eq!(snap.kind, SnapKind::Tangent);
    assert!(snap.position.approx_eq(tan, tol::POINT_MERGE));

    // Unregistering the sketch drops every curve candidate with it.
    scene.remove_sketch(SketchId::default());
    let gone = scene.resolve(&cq);
    assert_ne!(
        gone.map(|s| s.kind),
        Some(SnapKind::Center),
        "removed sketch must not keep offering its center"
    );
}

// --------------------------------------------------- sketch-curve provenance
//
// The analytic points a drawn curve publishes about itself — a circle's or
// arc's exact center, its covered quadrants, an anchored tangent, a regular
// polygon's drawn center — plus the endpoints of the facets it is built from,
// must name the CURVE CHAIN they belong to. Without it the app's selection
// resolver has nothing to select and falls through to a ray re-probe, which
// lands on whatever region happens to be under the cursor: clicking a drawn
// circle at (or near) a quadrant or centre selected the sketch's island
// instead of the circle. It is deliberately its own field, not a
// `sketch_source` with some stand-in facet edge: a center lies on no edge at
// all, and `sketch_source` is read as a DIRECTION reference by Tape Measure's
// parallel guides, so a stand-in there would mislead a real consumer.

/// The sketch handle every curve fixture below registers under. A distinct
/// non-default id, so a test cannot pass by comparing two null keys.
fn curve_sketch_id() -> SketchId {
    let mut sm: slotmap::SlotMap<SketchId, ()> = slotmap::SlotMap::with_key();
    sm.insert(());
    sm.insert(()) // the second key: never `SketchId::default()`
}

#[test]
fn drawn_curve_analytic_points_name_their_curve() {
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = Point3::new(1.0, 2.0, 0.0);
    let s = sketch_with_circle(plane, center, 0.5, 24);
    let sid = curve_sketch_id();
    register_sketch_full(&mut scene, sid, &s);

    let rims = s.curve_rims();
    let rim = &rims[0];
    let want = Some((sid, rim.curve));

    // Center.
    let cq = query(ray_at(Point3::new(1.0, 2.0, 3.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("drawn circle center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert_eq!(snap.sketch_curve_source, want);
    // Mutually exclusive with every other provenance field.
    assert_eq!(snap.source, None);
    assert_eq!(snap.sketch_source, None);
    assert_eq!(snap.sketch_region_source, None);
    assert_eq!(scene.resolve(&cq), scene.resolve_linear(&cq));

    // Every covered quadrant, not just the first — the reported bug was
    // angle-dependent, so no quadrant may be the odd one out.
    for q in rim.quadrant_points() {
        let qq = query(ray_at(q + Vec3::new(0.0, 0.0, 3.0), q), PIN);
        let snap = scene.resolve(&qq).expect("drawn circle quadrant resolves");
        assert_eq!(snap.kind, SnapKind::Quadrant);
        assert_eq!(snap.sketch_curve_source, want);
        assert_eq!(snap.sketch_source, None);
        assert_eq!(scene.resolve(&qq), scene.resolve_linear(&qq));
    }

    // An anchored tangent point lies exactly ON the curve, so it names it too.
    let anchor = center + rim.basis_u * 1.5;
    let alpha = (0.5f64 / 1.5).acos();
    let tan = center + rim.basis_u * (0.5 * alpha.cos()) + rim.basis_v * (0.5 * alpha.sin());
    let mut tq = query(ray_at(tan + Vec3::new(0.0, 0.0, 3.0), tan), PIN);
    tq.anchor = Some(anchor);
    let snap = scene.resolve(&tq).expect("drawn circle tangent resolves");
    assert_eq!(snap.kind, SnapKind::Tangent);
    assert_eq!(snap.sketch_curve_source, want);

    // The curve's own EDGE snaps are unchanged: still sketch-edge provenance,
    // and never curve provenance — the two never collide.
    let (eid, e) = s.edges().iter().next().expect("the circle has facets");
    let mid = Point3::new(
        (s.vertices()[e.from].position.x + s.vertices()[e.to].position.x) * 0.5,
        (s.vertices()[e.from].position.y + s.vertices()[e.to].position.y) * 0.5,
        (s.vertices()[e.from].position.z + s.vertices()[e.to].position.z) * 0.5,
    );
    let mq = query(ray_at(mid + Vec3::new(0.0, 0.0, 3.0), mid), PIN);
    let snap = scene.resolve(&mq).expect("a facet midpoint resolves");
    assert_eq!(snap.kind, SnapKind::Midpoint);
    assert_eq!(snap.sketch_source, Some((sid, eid)));
    assert_eq!(
        snap.sketch_curve_source, None,
        "an edge snap keeps naming its edge — the new field never displaces it"
    );

    // The facet VERTEX (Endpoint), by contrast, names the CURVE, not the
    // edge: a vertex is not a direction reference, and this is what makes a
    // click landing exactly on a facet vertex select the curve rather than
    // fall through to the region. It carries NO `sketch_source`, so Tape
    // Measure never treats it as a parallel reference.
    let v = s.vertices()[e.from].position;
    let vq = query(ray_at(v + Vec3::new(0.0, 0.0, 3.0), v), PIN);
    let snap = scene.resolve(&vq).expect("a facet vertex resolves");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert_eq!(snap.sketch_curve_source, want);
    assert_eq!(
        snap.sketch_source, None,
        "a facet vertex is not a direction reference — no sketch_source"
    );
    assert_eq!(scene.resolve(&vq), scene.resolve_linear(&vq));
}

#[test]
fn a_drawn_arc_names_its_curve_like_a_circle() {
    // An arc is the same analytic record with partial coverage, so it gets
    // the same treatment by construction: its center is equally the point the
    // user placed, and its chain is equally the thing a click selects.
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = Point3::new(-2.0, 1.0, 0.0);
    let radius = 0.4;
    let mut s = kernel::Sketch::on_plane(plane);
    let cid = s
        .begin_curve_with(kernel::CurveGeom { center, radius })
        .unwrap();
    // Six facets of a 24-gon, on the same half-facet phase
    // `sketch_with_circle` uses, so no vertex lands on a cardinal and the
    // covered range is a proper subset of the circle: angles
    // [0.5, 6.5] * 2pi/24, which contains the pi/2 cardinal and excludes 0.
    let (u, v) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let step = 2.0 * std::f64::consts::PI / 24.0;
    for i in 0..6 {
        let a0 = (i as f64 + 0.5) * step;
        let a1 = (i as f64 + 1.5) * step;
        let p0 = center + u * (radius * a0.cos()) + v * (radius * a0.sin());
        let p1 = center + u * (radius * a1.cos()) + v * (radius * a1.sin());
        s.add_segment(p0, p1).unwrap();
    }
    s.end_curve();

    let sid = curve_sketch_id();
    register_sketch_full(&mut scene, sid, &s);

    let cq = query(ray_at(center + Vec3::new(0.0, 0.0, 3.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("drawn arc center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert_eq!(snap.sketch_curve_source, Some((sid, cid)));

    // Exactly one cardinal falls on this arc's covered span; the other three
    // are not offered at all — arc coverage still gates the candidates.
    let rims = s.curve_rims();
    let rim = &rims[0];
    let qs = rim.quadrant_points();
    assert_eq!(qs.len(), 1, "this quarter arc covers exactly one cardinal");
    for q in &qs {
        let qq = query(ray_at(*q + Vec3::new(0.0, 0.0, 3.0), *q), PIN);
        let snap = scene.resolve(&qq).expect("arc quadrant resolves");
        assert_eq!(snap.sketch_curve_source, Some((sid, cid)));
    }

    // A partial arc DOES offer tangents — but only where the tangent point
    // falls on its covered span (the inference tangent walk gates each on
    // `rim.covers`). Find one and confirm its Tangent snap names the curve
    // too. Self-calibrating on the rim's OWN basis (for +Z, `plane_axes` gives
    // basis_u = +Y, basis_v = -X, so the covered span is not aligned with the
    // authoring frame). An anchor along basis_u places the two tangents at
    // ±acos(r/D); scan for a covered one clear of every cardinal (so a
    // Quadrant can't shadow it) and every facet vertex (so an Endpoint can't).
    let pi = std::f64::consts::PI;
    let ang_dist = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(2.0 * pi);
        d.min(2.0 * pi - d)
    };
    // Every competing positional snap the dense facets offer, in the rim's
    // basis: each facet VERTEX (an Endpoint) and each facet CHORD-MIDPOINT (a
    // Midpoint). The chosen tangent target must sit clear of all of them, or a
    // stronger-kinded candidate at the same spot would win the pin.
    let basis_angle = |p: Point3| -> f64 {
        let d = p - center;
        d.dot(rim.basis_v).atan2(d.dot(rim.basis_u))
    };
    let mut competitors: Vec<f64> = Vec::new();
    for (_, e) in s.edges().iter() {
        let a = s.vertices()[e.from].position;
        let b = s.vertices()[e.to].position;
        competitors.push(basis_angle(a));
        competitors.push(basis_angle(b));
        competitors.push(basis_angle(Point3::new(
            (a.x + b.x) * 0.5,
            (a.y + b.y) * 0.5,
            (a.z + b.z) * 0.5,
        )));
    }
    // A 0.5 m eye height keeps the pin cone (aperture PIN = 0.003 rad) tiny at
    // the target — ~0.0015 m radius — so a competitor 1.7° of arc away
    // (r·0.03 ≈ 0.012 m) is well outside it; the clearance only picks a clean
    // target, the eye height guarantees nothing else contends for the pin.
    let clear = |theta: f64| {
        [0.0, 0.5 * pi, pi, -0.5 * pi]
            .iter()
            .all(|&c| ang_dist(theta, c) > 0.2)
            && competitors.iter().all(|&ca| ang_dist(theta, ca) > 0.03)
    };
    let mut chosen: Option<(f64, f64)> = None; // (alpha, theta)
    let mut a_deg: f64 = 25.0;
    while a_deg <= 80.0 && chosen.is_none() {
        let alpha = a_deg.to_radians();
        for theta in [alpha, -alpha] {
            if rim.covers(theta) && clear(theta) {
                chosen = Some((alpha, theta));
                break;
            }
        }
        a_deg += 0.5;
    }
    let (alpha, theta) =
        chosen.expect("the arc must offer a covered tangent clear of its facet snaps");
    let d = radius / alpha.cos();
    let anchor = center + rim.basis_u * d;
    let tan = center + rim.basis_u * (radius * theta.cos()) + rim.basis_v * (radius * theta.sin());
    let mut tq = query(ray_at(tan + Vec3::new(0.0, 0.0, 0.5), tan), PIN);
    tq.anchor = Some(anchor);
    let snap = scene.resolve(&tq).expect("arc tangent resolves");
    assert_eq!(
        snap.kind,
        SnapKind::Tangent,
        "the covered tangent point snaps as Tangent"
    );
    assert_eq!(
        snap.sketch_curve_source,
        Some((sid, cid)),
        "an arc's tangent names its curve chain, just like a circle's"
    );
}

#[test]
fn a_polygon_center_names_its_curve_too() {
    // Same defect, different hat: a polygon's center rides its own scene
    // channel but is the same kind of analytic point, so it names its chain
    // the same way — a polygon's center selects the polygon.
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = Point3::new(3.0, -1.0, 0.0);
    let radius = 0.6;
    let mut s = kernel::Sketch::on_plane(plane);
    let cid = s
        .begin_curve_with_kind(
            kernel::CurveGeom { center, radius },
            kernel::SketchCurveKind::Polygon,
        )
        .unwrap();
    let (u, v) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    for i in 0..6 {
        let a0 = 2.0 * std::f64::consts::PI * (i as f64) / 6.0;
        let a1 = 2.0 * std::f64::consts::PI * (i as f64 + 1.0) / 6.0;
        let p0 = center + u * (radius * a0.cos()) + v * (radius * a0.sin());
        let p1 = center + u * (radius * a1.cos()) + v * (radius * a1.sin());
        s.add_segment(p0, p1).unwrap();
    }
    s.end_curve();

    let sid = curve_sketch_id();
    register_sketch_full(&mut scene, sid, &s);
    let centers = s.polygon_centers();
    assert_eq!(centers, vec![(cid, center)]);
    scene.add_sketch_polygon_centers(sid, &centers);
    assert!(
        s.curve_rims().is_empty(),
        "a polygon publishes no rim — only its center"
    );

    let cq = query(ray_at(center + Vec3::new(0.0, 0.0, 3.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("drawn polygon center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert_eq!(snap.sketch_curve_source, Some((sid, cid)));
    assert_eq!(snap.source, None);
    assert_eq!(snap.sketch_source, None);
    assert_eq!(scene.resolve(&cq), scene.resolve_linear(&cq));

    // Removal still clears the channel (the id now rides alongside the point).
    scene.remove_sketch(sid);
    assert_ne!(
        scene.resolve(&cq).map(|s| s.kind),
        Some(SnapKind::Center),
        "removed sketch must not keep offering its polygon center"
    );
}

#[test]
fn standing_sketch_circle_center_snaps_too() {
    // The detached-island case: a circle on an upright plane (normal +Y)
    // offers its center exactly like a ground one.
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 2.0, 0.0),
        Point3::new(1.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 1.0),
    ])
    .unwrap();
    let center = Point3::new(1.0, 2.0, 1.0);
    let s = sketch_with_circle(plane, center, 0.05, 24);
    register_sketch_full(&mut scene, SketchId::default(), &s);

    let cq = query(ray_at(Point3::new(1.0, 5.0, 1.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("standing circle center resolves");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&cq), scene.resolve_linear(&cq));
}

#[test]
fn sketch_arc_offers_center_but_only_covered_quadrants() {
    // Half of the circle deleted: the center stays, the deleted side's
    // quadrant does not, and re-registration (replace semantics) reflects
    // the trim — the same notch behavior a solid rim has.
    let mut scene = InferenceScene::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    // Off the world origin so the origin/axis candidates cannot shadow it.
    let center = Point3::new(2.0, 1.0, 0.0);
    let mut s = sketch_with_circle(plane, center, 1.0, 24);
    let rim_before = &s.curve_rims()[0];
    let bv = rim_before.basis_v;

    // Remove every facet on the -v side (frame angles in (-pi, 0)).
    let curve = s
        .edges()
        .values()
        .find_map(|e| e.curve)
        .expect("circle chain");
    for eid in s.curve_edges(curve) {
        let e = s.edges()[eid];
        let mid = kernel::Point3::new(
            (s.vertices()[e.from].position.x + s.vertices()[e.to].position.x) * 0.5,
            (s.vertices()[e.from].position.y + s.vertices()[e.to].position.y) * 0.5,
            0.0,
        );
        let d = mid - center;
        if d.dot(bv) < -1e-9 {
            s.remove_edge(eid).unwrap();
        }
    }
    register_sketch_full(&mut scene, SketchId::default(), &s);

    // Center still snaps.
    let cq = query(ray_at(Point3::new(0.0, 0.0, 3.0), center), NARROW);
    let snap = scene.resolve(&cq).expect("arc center resolves");
    assert_eq!(snap.kind, SnapKind::Center);

    // The +v quadrant survives; the -v one is gone (nothing at that spot —
    // its facets were deleted, so no Endpoint/OnEdge rescue either).
    let qplus = center + bv * 1.0;
    let qminus = center - bv * 1.0;
    let qp = query(ray_at(qplus + Vec3::new(0.0, 0.0, 3.0), qplus), PIN);
    let snap = scene.resolve(&qp).expect("covered quadrant resolves");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    let qm = query(ray_at(qminus + Vec3::new(0.0, 0.0, 3.0), qminus), PIN);
    assert_eq!(
        scene.resolve(&qm).map(|s| s.kind),
        None,
        "uncovered cardinal offers nothing"
    );
}

// ---------------------------------------------------------------------------
// Snap gravity (per-kind weighting) and precision mode
// ---------------------------------------------------------------------------
//
// A drawn circle's exact center and quadrant points are what a user aims at;
// the endpoints and midpoints of the many facets approximating that circle
// are noise crowded around them. `SnapWeights` gives Center/Quadrant a larger
// effective aperture and divides their angular distance before ranking, so
// they out-pull a facet point the cursor happens to be slightly nearer.
// `SnapWeights::uniform()` — precision mode — turns that off, restoring
// nearest-wins so a facet point stays reachable.

/// The center of the gravity fixture's circle. Deliberately off every world
/// axis and away from the origin: an ambient `OnAxis` candidate through a
/// quadrant point would otherwise stand in for the "nothing is in the cone"
/// case these specs rely on.
const GRAVITY_CIRCLE_CENTER: Point3 = Point3 {
    x: 2.0,
    y: 3.0,
    z: 0.0,
};

/// A ground-plane sketch holding one 24-gon circle chain, plus the rim's
/// first quadrant point and the facet vertex nearest that quadrant. The
/// half-facet phase in `sketch_with_circle` guarantees no vertex lands ON a
/// cardinal, so the quadrant and the facet endpoint are genuinely distinct
/// competing candidates.
fn circle_gravity_fixture() -> (InferenceScene, Point3, Point3) {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = GRAVITY_CIRCLE_CENTER;
    let s = sketch_with_circle(plane, center, 0.5, 24);
    let mut scene = InferenceScene::new();
    register_sketch_full(&mut scene, SketchId::default(), &s);

    let quadrant = s.curve_rims()[0].quadrant_points()[0];
    // The facet vertex nearest that quadrant — the candidate gravity has to
    // out-pull. Taken from the sketch itself rather than recomputed, so the
    // spec cannot drift from the geometry actually registered.
    let vertex = s
        .vertices()
        .iter()
        .map(|(_, v)| v.position)
        .min_by(|a, b| {
            (*a - quadrant)
                .length()
                .partial_cmp(&(*b - quadrant).length())
                .unwrap()
        })
        .expect("the circle chain has vertices");
    (scene, quadrant, vertex)
}

/// An eye 3 m above `target`, looking straight down at it. Every candidate in
/// these fixtures lies in the z = 0 plane, so angular distance from the ray
/// axis is (in-plane distance) / 3 to well under a percent.
fn overhead(target: Point3) -> PickRay {
    ray_at(target + Vec3::new(0.0, 0.0, 3.0), target)
}

/// Gravity's whole point: a quadrant wins from the neighborhood of a facet
/// endpoint the cursor is *nearer* to. Aim 60% of the way from the quadrant
/// to the adjacent facet vertex — the vertex is 1.5x closer, and still loses,
/// because the quadrant's weighted distance (÷ 2.5) is smaller.
#[test]
fn a_quadrant_out_pulls_the_facet_endpoint_beside_it() {
    let (scene, quadrant, vertex) = circle_gravity_fixture();
    let aim = quadrant + (vertex - quadrant) * 0.6;
    assert!(
        (aim - vertex).length() < (aim - quadrant).length(),
        "the fixture must aim NEARER the facet vertex, or it proves nothing"
    );

    let q = query(overhead(aim), 0.02);
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(
        snap.kind,
        SnapKind::Quadrant,
        "the quadrant's gravity beats the nearer facet endpoint"
    );
    assert!(snap.position.approx_eq(quadrant, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// The same aim in precision mode resolves to the facet endpoint: uniform
/// weights restore nearest-wins, which is exactly what the modifier is for.
#[test]
fn precision_mode_gives_the_nearer_facet_endpoint_back() {
    let (scene, quadrant, vertex) = circle_gravity_fixture();
    let aim = quadrant + (vertex - quadrant) * 0.6;

    let mut q = query(overhead(aim), 0.02);
    q.weights = SnapWeights::uniform();
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(vertex, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// Gravity is a preference, not a capture: aiming squarely AT a facet
/// endpoint still returns that endpoint. Its normalized distance is zero, and
/// nothing divided by a weight beats zero.
#[test]
fn aiming_squarely_at_a_facet_endpoint_still_gets_the_endpoint() {
    let (scene, _quadrant, vertex) = circle_gravity_fixture();
    let q = query(overhead(vertex), 0.02);
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(vertex, tol::POINT_MERGE));
}

/// A weighted kind reaches BEYOND the plain pick cone: the quadrant snaps
/// from 1.25 apertures away (inside its 2.5x reach) where the unweighted
/// query has nothing at all to offer.
#[test]
fn gravity_reaches_past_the_plain_aperture() {
    let (scene, quadrant, _vertex) = circle_gravity_fixture();
    // Radially outward from the circle, away from every facet: at 0.075 m the
    // aim is 0.025 rad off-axis — past the 0.02 aperture, inside 2.5 x 0.02.
    let outward = (quadrant - GRAVITY_CIRCLE_CENTER).normalized().unwrap();
    let aim = quadrant + outward * 0.075;

    let q = query(overhead(aim), 0.02);
    let snap = scene
        .resolve(&q)
        .expect("the quadrant is within its gravity");
    assert_eq!(snap.kind, SnapKind::Quadrant);
    assert!(snap.position.approx_eq(quadrant, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    let mut precise = q;
    precise.weights = SnapWeights::uniform();
    assert_eq!(
        scene.resolve(&precise),
        None,
        "unweighted, nothing is inside the cone at all"
    );
}

/// A polygon center is a bare `Center` — no rim, no quadrants — registered
/// through its own scene channel, and it carries the same analytic-point
/// gravity a circle's center does: it reaches past the plain aperture. Pins
/// the polygon-center walk on the weighted cone; an unweighted cone here
/// offers nothing at this distance.
#[test]
fn a_polygon_center_reaches_past_the_plain_aperture() {
    let center = Point3::new(2.0, 3.0, 0.0);
    let mut scene = InferenceScene::new();
    scene.add_sketch_polygon_centers(
        SketchId::default(),
        &[(kernel::SketchCurveId::default(), center)],
    );

    // 0.075 m off at 3 m overhead is 0.025 rad — past the 0.02 aperture,
    // inside the 2.5x analytic-point reach.
    let aim = center + Vec3::new(0.075, 0.0, 0.0);
    let q = query(overhead(aim), 0.02);
    let snap = scene.resolve(&q).expect("the center is within its gravity");
    assert_eq!(snap.kind, SnapKind::Center);
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    let mut precise = q;
    precise.weights = SnapWeights::uniform();
    assert_eq!(
        scene.resolve(&precise),
        None,
        "unweighted, nothing is inside the cone at all"
    );
}

/// Reach never steals. A quadrant admitted ONLY by its gravity — past the
/// query's own aperture — must lose to anything inside the plain aperture,
/// however weak its kind: otherwise hovering a face near a circle would yank
/// the cursor a couple of apertures away onto the rim, which is exactly the
/// regression `standing_sketch_region_is_a_hoverable_face` (wasm-api, over
/// the maintainer's real file) caught.
#[test]
fn gravity_reach_never_steals_from_a_candidate_inside_the_aperture() {
    let (mut scene, quadrant, _vertex) = circle_gravity_fixture();
    // A transient segment straight through the aim point gives an `OnEdge` —
    // the weakest positional kind that can sit right under the cursor.
    let outward = (quadrant - GRAVITY_CIRCLE_CENTER).normalized().unwrap();
    let aim = quadrant + outward * 0.075; // 0.025 rad: past 0.02, inside 2.5x
    // Deliberately lopsided: a symmetric segment would put its Midpoint — a
    // stronger kind — exactly under the cursor and prove less.
    scene.add_transient_segment(
        aim + Vec3::new(0.0, -0.1, 0.0),
        aim + Vec3::new(0.0, 0.5, 0.0),
    );

    let q = query(overhead(aim), 0.02);
    let snap = scene.resolve(&q).expect("the segment is under the cursor");
    assert_eq!(
        snap.kind,
        SnapKind::OnEdge,
        "a gravity-extended quadrant must not outrank what the cursor is on"
    );
    assert!(snap.position.approx_eq(aim, tol::POINT_MERGE));

    // Take the segment away and the quadrant's reach is uncontested again —
    // proving the guard is about *competition*, not about suppressing reach.
    scene.clear_transient();
    assert_eq!(scene.resolve(&q).map(|s| s.kind), Some(SnapKind::Quadrant));
}

/// The invariant `SnapKind::Center`'s docs promise, kept through weighting:
/// a real vertex sitting exactly on a circle's center still wins. Equal
/// normalized distance (both zero) breaks toward the stronger kind.
#[test]
fn a_vertex_exactly_on_the_center_still_wins() {
    let (mut scene, _quadrant, _vertex) = circle_gravity_fixture();
    let center = GRAVITY_CIRCLE_CENTER;
    // A transient segment starting exactly at the center contributes an
    // Endpoint candidate there, with no provenance — the cheapest way to put
    // a real vertex on top of a center.
    scene.add_transient_segment(center, center + Vec3::new(0.0, 0.2, 0.0));

    let q = query(overhead(center), 0.02);
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));
}

/// Weights trade places only inside a rank group. Boosting `Midpoint` and
/// `OnFace` to the maximum cannot lift either over an `Endpoint`, however
/// much nearer the cursor is to them — otherwise a face (angular distance
/// zero by construction) would beat every vertex in the model.
#[test]
fn gravity_never_lifts_a_weaker_rank_group_over_a_stronger_one() {
    let scene = cube_scene();
    // Straight down onto the top face, 60% of the way from the (1,1,1) corner
    // toward the (0.5,1,1) midpoint of the +y top edge, nudged just inside
    // the face (a ray lying exactly in the y = 1 side face's plane is a
    // degenerate occlusion test, not the thing under test). Overhead, so
    // angular distance is horizontal distance / 3: the midpoint sits at
    // 0.067 rad, the corner at 0.101, nothing else inside the 0.12 aperture.
    let corner = Point3::new(1.0, 1.0, 1.0);
    let mid = Point3::new(0.5, 1.0, 1.0);
    let aim = Point3::new(0.7, 0.97, 1.0);
    assert!(
        (aim - mid).length() < (aim - corner).length(),
        "the fixture must aim NEARER the midpoint, or it proves nothing"
    );

    let mut q = query(overhead(aim), 0.12);
    q.weights = SnapWeights::default()
        .with(SnapKind::Midpoint, inference::GRAVITY_MAX)
        .with(SnapKind::OnFace, inference::GRAVITY_MAX);
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(
        snap.kind,
        SnapKind::Endpoint,
        "rank group beats gravity: a boosted midpoint/face never outranks a vertex"
    );
    assert!(snap.position.approx_eq(corner, tol::POINT_MERGE));
}

/// The boundary of the whole feature, stated as a test: gravity and precision
/// mode change outcomes ONLY inside rank group 0 (Endpoint vs Center vs
/// Quadrant). A `Midpoint` sits in a weaker group, so a center already beat it
/// at any distance before weighting existed — and precision mode, which only
/// flattens weights, cannot hand it back. Worth pinning: the obvious reading
/// of "precision mode lets you pick the other thing" is wrong here, and a
/// future change that made it true would be a real (and unrequested) change in
/// the priority model.
#[test]
fn a_nearer_midpoint_loses_to_a_center_with_or_without_gravity() {
    let (mut scene, _quadrant, _vertex) = circle_gravity_fixture();
    let center = GRAVITY_CIRCLE_CENTER;
    // A segment whose MIDPOINT lands 0.06 m from the center, laid along the
    // aim direction so its own OnEdge cannot sit nearer than the midpoint.
    let dir = Vec3::new(0.8, 0.6, 0.0).normalized().unwrap();
    let mid = center + dir * 0.06;
    scene.add_transient_segment(mid - dir * 0.25, mid + dir * 0.25);

    // 65% of the way from the center to that midpoint: NEARER the midpoint.
    let aim = center + dir * 0.039;
    let q = query(overhead(aim), 0.02);
    assert_eq!(
        scene.resolve(&q).map(|s| s.kind),
        Some(SnapKind::Center),
        "the center outranks a nearer midpoint by rank group"
    );

    let mut precise = q;
    precise.weights = SnapWeights::uniform();
    assert_eq!(
        scene.resolve(&precise).map(|s| s.kind),
        Some(SnapKind::Center),
        "precision mode flattens weights; it does not re-order rank groups"
    );
}

/// An `OnFace` weight is inert by construction: `face_cone_hit` is a
/// ray-vs-face intersection that never reads the aperture and always reports
/// angular distance zero, so there is nothing for a multiplier to scale and
/// nothing for it to divide. Pinned because `SnapWeights` accepts a weight for
/// every kind, and a caller who sets one here deserves to find it documented
/// as a no-op rather than to discover it silently does nothing.
#[test]
fn an_on_face_weight_is_inert() {
    let scene = cube_scene();
    // Straight down at the middle of the top face: OnFace is the winner, and
    // no vertex or edge is anywhere near the cone.
    let q = query(overhead(Point3::new(0.5, 0.5, 1.0)), 0.02);
    let baseline = scene.resolve(&q).expect("the top face is under the ray");
    assert_eq!(baseline.kind, SnapKind::OnFace);

    for w in [inference::GRAVITY_NEUTRAL, 2.5, inference::GRAVITY_MAX] {
        let mut boosted = q;
        boosted.weights = SnapWeights::default().with(SnapKind::OnFace, w);
        assert_eq!(
            scene.resolve(&boosted),
            Some(baseline),
            "an OnFace weight of {w} changed the answer"
        );
    }
}

/// Gravity keys on `SnapKind` ALONE, never on `(kind, provenance)`: a
/// `Center` with no quadrant or tangent companions gets exactly the same pull
/// as a circle's. That matters beyond arcs — a polygon's center registers as
/// an ordinary `Center` `ScenePoint` with no rim beside it at all, and must
/// not quietly lose its gravity for want of one.
#[test]
fn a_center_with_no_quadrant_companions_still_has_full_gravity() {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let center = GRAVITY_CIRCLE_CENTER;
    let mut s = sketch_with_circle(plane, center, 0.5, 24);
    let rim_before = &s.curve_rims()[0];
    let (bu, bv) = (rim_before.basis_u, rim_before.basis_v);

    // Keep only the wedge between ~10 and ~80 degrees, so NO cardinal is
    // covered: the rim offers a center and nothing else.
    let curve = s
        .edges()
        .values()
        .find_map(|e| e.curve)
        .expect("circle chain");
    for eid in s.curve_edges(curve) {
        let e = s.edges()[eid];
        let a = s.vertices()[e.from].position;
        let b = s.vertices()[e.to].position;
        let mid = Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5);
        let d = mid - center;
        let angle = d.dot(bv).atan2(d.dot(bu));
        if !(angle > 0.17 && angle < 1.40) {
            s.remove_edge(eid).unwrap();
        }
    }
    let mut scene = InferenceScene::new();
    register_sketch_full(&mut scene, SketchId::default(), &s);

    // No quadrant survives anywhere on this rim.
    for q in s.curve_rims()[0].quadrant_points() {
        let qq = query(overhead(q), 0.02);
        assert_ne!(
            scene.resolve(&qq).map(|snap| snap.kind),
            Some(SnapKind::Quadrant),
            "the fixture must offer no quadrant, or it proves nothing"
        );
    }

    // A bare vertex 0.06 m from the center — nearer the cursor than the
    // center is, and it still loses to the center's gravity.
    scene.add_transient_segment(
        center + Vec3::new(0.06, 0.0, 0.0),
        center + Vec3::new(0.5, 0.0, 0.0),
    );
    let aim = center + Vec3::new(0.039, 0.0, 0.0); // 65% of the way across
    let q = query(overhead(aim), 0.02);
    let snap = scene.resolve(&q).expect("something snaps");
    assert_eq!(
        snap.kind,
        SnapKind::Center,
        "a companion-less center must pull exactly like a circle's"
    );
    assert!(snap.position.approx_eq(center, tol::POINT_MERGE));

    // ...and precision mode gives the vertex back, same as anywhere else.
    let mut precise = q;
    precise.weights = SnapWeights::uniform();
    assert_eq!(
        scene.resolve(&precise).map(|snap| snap.kind),
        Some(SnapKind::Endpoint)
    );
}

/// Weighting an INDEXED kind must widen the index's prune cone too
/// (`SnapWeights::max_indexed`): a boosted kind is admitted further off-axis
/// than `aperture`, so pruning at the bare `aperture` throws candidates away
/// before the exact test can see them, and `resolve` stops agreeing with
/// `resolve_linear`.
///
/// Pinning that needs a query where the boosted candidate would BOTH win and
/// be pruned. A BVH node box is only rejected when the whole box clears the
/// cone, so the ray has to pass wide of the object — about 1.5 m from a unit
/// cube's centre at 10 m, with a 0.02 rad aperture: the cube's node box is
/// 0.63 m outside the plain cone (0.22 m at that depth) and well inside the
/// 8x one (1.74 m), while the nearest vertex sits at ~0.11 rad — outside the
/// plain aperture, inside 8x it, and with nothing nearer to beat it. Reverting
/// `prune_aperture` to a bare `aperture` fails this test.
#[test]
fn boosting_an_indexed_kind_still_matches_the_linear_reference() {
    let cube = unit_cube();
    let mut scene = InferenceScene::new();
    scene.set_def_member(ObjectId::default(), &cube);
    // A 6x6 grid spaced far enough apart that only the cube at the origin is
    // anywhere near the ray, but with enough placements for the index to have
    // real structure rather than collapsing to a single leaf.
    for gx in 0..6 {
        for gy in 0..6 {
            scene.add_placement(
                InstanceId::default(),
                ObjectId::default(),
                &Transform::translation(Vec3::new(12.0 * gx as f64, 12.0 * gy as f64, 0.0)),
            );
        }
    }

    // Straight down at (2, 0.5), 1.5 m clear of the origin cube in +x.
    let ray = PickRay {
        origin: Point3::new(2.0, 0.5, 10.5),
        direction: Vec3::new(0.0, 0.0, -1.0),
    };
    let plain = query(ray, 0.02);
    assert_eq!(
        scene.resolve(&plain).map(|s| s.kind),
        None,
        "unweighted, nothing is inside the cone"
    );

    let mut boosted = plain;
    boosted.weights = SnapWeights::default().with(SnapKind::Endpoint, inference::GRAVITY_MAX);
    let snap = scene
        .resolve(&boosted)
        .expect("a +x corner is inside 8x the aperture");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    // All four +x corners are the same in-plane distance from this vertical
    // ray, so the deeper (bottom) pair are marginally nearer the axis in
    // ANGLE and win on that; which of them is a depth question the test does
    // not care about. What it pins is that a vertex on that face is reachable
    // at all, and that the indexed answer is the reference answer.
    assert!(
        snap.position.x == 1.0 && (snap.position.y == 0.0 || snap.position.y == 1.0),
        "expected a +x corner of the origin cube, got {:?}",
        snap.position
    );
    assert_eq!(
        scene.resolve(&boosted),
        scene.resolve_linear(&boosted),
        "the prune cone must widen with the weight"
    );
}

// -----------------------------------------------------------------------
// Soft axis inference (tool-parity playtest2 §2c): "if you're dragging
// roughly along an axis from the anchor, snap to it" — extends the OnAxis
// producer to also emit the three axis lines through `query.anchor`, not
// just through the frame origin. Every fixture here uses an anchor AWAY
// from the world/frame origin, specifically so the pre-existing
// origin-relative OnAxis candidate (a genuinely different line, `aperture`-
// gated) never coincidentally interferes with what's being tested.
// -----------------------------------------------------------------------

/// A bird's-eye ray whose target is within `SOFT_AXIS_APERTURE_DEG` (~5°) of
/// the X axis through a non-origin anchor resolves OnAxis, through that
/// anchor, with the anchor's own axis direction.
#[test]
fn soft_axis_snaps_through_the_anchor_within_tolerance() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 0.0);
    // 5 units along +X from the anchor, drifted 0.3 in Y: atan(0.3/5) ≈
    // 3.43°, inside the ~5° soft-axis tolerance.
    let target = Point3::new(7.0, 3.3, 0.0);
    let ray = ray_at(Point3::new(target.x, target.y, 5.0), target);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW, // tight — isolates the soft-axis candidate specifically
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("within the soft-axis tolerance of the anchor's X axis");
    assert_eq!(snap.kind, SnapKind::OnAxis);
    assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
    // On the line y=3, z=0 through the anchor.
    assert!((snap.position.y - anchor.y).abs() <= tol::POINT_MERGE);
    assert!((snap.position.z - anchor.z).abs() <= tol::POINT_MERGE);
}

/// Past the soft-axis tolerance, dragging "roughly" along an axis no longer
/// snaps — with nothing else in the scene, resolve finds nothing at all.
#[test]
fn soft_axis_does_not_snap_outside_its_tolerance() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 0.0);
    // 5 units along +X, drifted 1.0 in Y: atan(1/5) ≈ 11.3°, past ~5°.
    let target = Point3::new(7.0, 4.0, 0.0);
    let ray = ray_at(Point3::new(target.x, target.y, 5.0), target);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    assert!(
        scene.resolve(&q).is_none(),
        "11.3 degrees off axis is well past the ~5 degree soft-axis tolerance"
    );
}

/// No `anchor` on the query — the plain (pre-existing) origin-relative
/// OnAxis behavior is untouched, and no anchor-relative candidate is ever
/// offered (there is no anchor to offer one through).
#[test]
fn soft_axis_requires_an_anchor() {
    let scene = InferenceScene::new();
    // Same near-axis geometry as the "within tolerance" fixture above, but
    // relative to (2,3,0) with NO anchor on the query — nothing at the
    // world origin is anywhere near this ray, so nothing should snap.
    let target = Point3::new(7.0, 3.3, 0.0);
    let ray = ray_at(Point3::new(target.x, target.y, 5.0), target);
    let q = query(ray, NARROW); // query()'s anchor is None
    assert!(
        scene.resolve(&q).is_none(),
        "no anchor means no anchor-relative axis candidate, and this ray is nowhere near the origin's own axes"
    );
}

/// Ranking (design's §2c requirement): the soft-axis candidate through the
/// anchor beats a coincident OnFace hit, even though both tie at angular
/// distance zero — `OnAxis`'s rank_group now sits above `OnEdge`/`OnFace`.
#[test]
fn soft_axis_beats_a_coincident_on_face_hit() {
    let scene = cube_scene(); // unit cube, faces at x/y/z in [0,1]
    // The X axis through this anchor passes EXACTLY through the top face's
    // center (0.5, 0.5, 1.0) at t=3.5.
    let anchor = Point3::new(-3.0, 0.5, 1.0);
    let face_center = Point3::new(0.5, 0.5, 1.0);
    let ray = ray_at(Point3::new(face_center.x, face_center.y, 4.0), face_center);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        // Tight enough to exclude the cube's vertices/edges (comfortably
        // farther from this ray than the face center), wide enough to be
        // irrelevant to the face hit (exact regardless of aperture) and to
        // the soft-axis hit (governed by its own SOFT_AXIS_APERTURE, not
        // this value).
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("both the face and the axis are hit dead-on");
    assert_eq!(
        snap.kind,
        SnapKind::OnAxis,
        "the soft-axis candidate must outrank the coincident OnFace hit"
    );
    assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
}

/// Ranking (design's §2c requirement): an exact point snap (Endpoint) still
/// beats soft-axis inference, even when the axis line passes close by.
#[test]
fn point_snaps_still_beat_soft_axis() {
    let scene = cube_scene();
    let anchor = Point3::new(-4.0, 0.05, 1.0);
    // Aimed squarely at the (1,0,1) corner; the X axis through the anchor
    // passes within ~1 degree of it (well inside the soft-axis tolerance
    // too), so both are legitimate candidates.
    let corner = Point3::new(1.0, 0.0, 1.0);
    let ray = ray_at(Point3::new(corner.x, corner.y, 4.0), corner);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: WIDE,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene.resolve(&q).expect("the corner is dead-on");
    assert_eq!(snap.kind, SnapKind::Endpoint);
}

/// Hard lock (an explicit `query.lock`) always wins — the same anchor and
/// direction that would otherwise resolve as soft-axis inference instead
/// takes the dedicated lock branch, which short-circuits ranking entirely
/// (this is architectural: `resolve` never reaches the candidate list at
/// all once `query.lock` is `Some`, so a lock cannot lose to anything the
/// candidate/ranking system might have offered, soft-axis included).
#[test]
fn hard_lock_wins_over_what_would_otherwise_be_soft_axis_inference() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 0.0);
    let target = Point3::new(7.0, 3.3, 0.0); // within soft-axis tolerance of +X
    let ray = ray_at(Point3::new(target.x, target.y, 5.0), target);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: Some(SnapLock::Axis(inference::Axis::Y)), // locked to a DIFFERENT axis
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("a lock with an anchor always resolves");
    assert_eq!(snap.direction, Some(Vec3::new(0.0, 1.0, 0.0)));
}

/// A moved drawing-axes frame (tool-parity design §4) reorients soft-axis
/// candidates exactly like the origin-relative ones: the anchor-relative
/// axis lines follow the frame's own directions, not world X/Y/Z.
#[test]
fn moved_axes_frame_reorients_soft_axis_candidates() {
    let mut scene = InferenceScene::new();
    let frame = AxesFrame::new(
        Point3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),  // red = world +Y
        Vec3::new(-1.0, 0.0, 0.0), // green = world -X
    )
    .expect("orthonormal frame");
    scene.set_axes_frame(frame);

    let anchor = Point3::new(2.0, 3.0, 0.0);
    // 5 units along the frame's red axis (world +Y) from the anchor,
    // drifted 0.3 along world -X (the frame's green axis): same ~3.43°
    // geometry as the world-identity fixture, just reoriented.
    let target = anchor + frame.x * 5.0 + frame.y * 0.3;
    let ray = ray_at(Point3::new(target.x, target.y, 5.0), target);
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("within the soft-axis tolerance of the anchor's (frame) red axis");
    assert_eq!(snap.kind, SnapKind::OnAxis);
    assert_eq!(snap.direction, Some(frame.x));
}

/// The edge-on guard (design's §2c "test that case explicitly"): a ray
/// sighting nearly straight down a candidate axis produces NO soft-axis
/// candidate at all — not a wildly-swung one — even though, absent the
/// guard, `closest_point_on_line_to_ray`'s near-parallel ill-conditioning
/// would otherwise resolve SOME point. The anchor sits far from the world
/// origin/frame origin so the pre-existing origin-relative producer (which
/// this guard deliberately does not touch — see its doc) cannot supply an
/// unrelated candidate that would mask what this test is actually about.
#[test]
fn edge_on_axis_produces_no_soft_candidate() {
    let scene = InferenceScene::new();
    let anchor = Point3::ORIGIN;
    // A ray sighting almost exactly along +X (2 degrees off) — the classic
    // "drawing a very long line nearly end-on to the camera" configuration
    // — offset far off in BOTH y and z so it is nowhere near the anchor's
    // OTHER two axis lines either (each ends up ~45 degrees away, comfortably
    // outside the soft-axis tolerance): this test is specifically about the
    // X candidate's own edge-on suppression, not an accidental near-miss on
    // Y or Z substituting a different (valid) result.
    let two_deg = 2.0_f64.to_radians();
    let ray = PickRay {
        origin: Point3::new(-500.0, 500.0, 500.0),
        direction: Vec3::new(two_deg.cos(), 0.0, two_deg.sin()),
    };
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    assert!(
        scene.resolve(&q).is_none(),
        "an edge-on axis must be suppressed, not resolved to a wildly-swung point"
    );
}

// -----------------------------------------------------------------------
// Playtest-2 defects: soft-axis admission must be a genuine, camera-
// position-independent ~5° tolerance, and a hard direction lock must never
// let an UNRELATED soft-axis candidate hijack its projection line. Every
// fixture above this point uses a near-straight-overhead ray (the classic
// `ray_at(eye_directly_above, target)` shape) precisely because that keeps
// the eye-relative and anchor-relative angle measurements coincident — it
// never exercised an ordinary oblique 3/4 orbit camera, which is where both
// of these defects actually showed up in the app.
// -----------------------------------------------------------------------

/// From an ordinary oblique (non-overhead) camera, aiming 25° off the
/// anchor-relative X axis toward Y must NOT resolve as `OnAxis` — 25° is
/// five times `SOFT_AXIS_APERTURE_DEG` (~5°). Pre-fix, `resolve` measured
/// this candidate's admission from the RAY ORIGIN (the camera eye) to
/// wherever along the (unbounded) X line the ray happened to pass closest —
/// a point that can lie far from the anchor, subtending a small eye-angle
/// even for a genuinely wide miss. For this exact camera/anchor pair,
/// pre-fix `resolve` returned `Some(OnAxis)` at `(6.99, 3.0, 0.0)`, direction
/// +X — a candidate admitted at five times its documented tolerance. That
/// silent over-admission is the direct cause of the reported "aim roughly
/// midway between two axes — the chip stays stuck, the click commits
/// nothing" defect: whichever axis wins this mis-measured contest doesn't
/// track the cursor the way the user actually moved it, and on the ground
/// plane a Z-axis win in particular collapses the point straight back onto
/// the anchor (`sibling test` below covers the direction-lock half of the
/// same root cause).
#[test]
fn soft_axis_admission_does_not_widen_with_an_oblique_camera() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 0.0);
    // An ordinary 3/4 orbit camera — NOT straight overhead, NOT edge-on to
    // any axis. Elevation ~24° off the ground plane, azimuth off both X
    // and Y — representative of how a user actually looks at their model.
    let eye = Point3::new(10.0, -8.0, 6.0);
    // Target 5 units out from the anchor, 25° off +X toward +Y, in the
    // anchor's ground plane (z unchanged) — an ordinary ground-plane drag.
    let deg25 = 25.0_f64.to_radians();
    let target = Point3::new(
        anchor.x + 5.0 * deg25.cos(),
        anchor.y + 5.0 * deg25.sin(),
        anchor.z,
    );
    let ray = PickRay {
        origin: eye,
        direction: target - eye,
    };
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    assert!(
        scene.resolve(&q).is_none(),
        "25 degrees off axis (5x SOFT_AXIS_APERTURE_DEG) must not resolve as OnAxis, \
         regardless of how oblique the camera is"
    );
}

/// Tool-parity delta-review finding 1: a mathematically dead-on aim (the ray
/// is aimed exactly at a point ON the anchor-relative axis) must resolve as
/// `OnAxis` at a genuine multi-metre drag length from an ordinary oblique
/// camera — not just from the near-straight-overhead shape every fixture
/// above this point happens to use. Pre-fix, `soft_axis_deviation` measured
/// admission from a point fixed at the anchor's own depth ALONG THE RAY
/// (an eye-relative quantity), which only coincides with the true deviation
/// when the ray is perpendicular to the axis; for this ordinary 3/4-orbit
/// camera aiming dead-on at 20 units out, it reported very roughly 27° off
/// axis (comfortably outside `SOFT_AXIS_APERTURE_DEG`) and this resolved to
/// `None` — the soft axis silently let go mid-drag while the user kept
/// aiming exactly along it the whole time. Same anchor, axis, and camera as
/// `soft_axis_admission_does_not_widen_with_an_oblique_camera` above, so the
/// two together prove neither of that fix's two failure modes regressed the
/// other: a genuine miss is still rejected, and a genuine hit is admitted,
/// from the same non-overhead camera.
#[test]
fn soft_axis_admits_a_dead_on_aim_at_a_multi_metre_drag_from_an_oblique_camera() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 9.0);
    let eye = Point3::new(-8.0, -7.0, 17.0);
    let target = Point3::new(anchor.x + 20.0, anchor.y, anchor.z); // dead-on, 20m along +X
    let ray = PickRay {
        origin: eye,
        direction: target - eye,
    };
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: None,
        aperture: NARROW,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("a dead-on aim along the anchor's own axis must resolve regardless of camera");
    assert_eq!(snap.kind, SnapKind::OnAxis);
    assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
    assert!(
        (snap.position.y - anchor.y).abs() <= tol::POINT_MERGE
            && (snap.position.z - anchor.z).abs() <= tol::POINT_MERGE,
        "expected the resolved point on the anchor's own X line, got {:?}",
        snap.position
    );
}

/// A hard direction lock (`query.lock`) must resolve along the LOCKED line,
/// never collapse back onto `anchor` because an unrelated soft-axis
/// candidate won the ranking and got projected onto it. This is the exact
/// "genuinely non-ground plane" LineTool repro: anchored on the X-Z wall a
/// re-home from an X-then-Z chain produces (normal +Y), locked to Z
/// (arrow-up), cursor aimed mostly along +X with only a slight upward
/// drift — a very plausible mid-drag mouse position, not a contrived one.
///
/// Pre-fix, `resolve` still built the anchor-relative soft-axis candidates
/// for ALL three axes even though `query.lock` was set (its own doc
/// comment's claim that "a lock short-circuits `resolve` before candidates
/// are even ranked" was simply false), so the in-plane X candidate — never
/// edge-on, always live on this same wall — won the ranking outright. The
/// lock branch then projected that WINNER onto the locked Z line via
/// `project_onto_line(anchor, Z, winner_pos)`; since the X candidate's
/// position never varies in Z, that projection landed EXACTLY on `anchor`
/// (verified pre-fix: `Some(OnAxis)` at `(2.0, 3.0, 9.0)` itself — zero
/// displacement in every axis). `_planeCursor`/`_commitPlaneSegment` then
/// silently drop that click (a degenerate zero-length segment) — matching
/// the reported "mid-chain Z lock never tracks the cursor or commits
/// anything" defect exactly, including the "Value 0 m" length readout.
#[test]
fn hard_lock_is_never_hijacked_by_an_unrelated_soft_axis_candidate() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 9.0);
    let plane = Plane::from_point_normal(anchor, Vec3::new(0.0, 1.0, 0.0)).unwrap();
    let eye = Point3::new(10.0, -8.0, 6.0);
    // Mostly +X, barely upward — an ordinary "roughly along the locked
    // line, but not pixel-perfect" mouse position.
    let target = Point3::new(anchor.x + 5.0, anchor.y, anchor.z + 0.3);
    let ray = PickRay {
        origin: eye,
        direction: target - eye,
    };
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray,
        anchor: Some(anchor),
        lock: Some(SnapLock::Axis(inference::Axis::Z)),
        aperture: NARROW,
        constraint_plane: Some(plane),
        soft_axis_aperture_scale: None,
        off_plane_points: false,
    };
    let snap = scene
        .resolve(&q)
        .expect("a lock with an anchor always resolves (the fallback line, if nothing else)");
    assert_eq!(snap.direction, Some(Vec3::new(0.0, 0.0, 1.0)));
    assert!(
        (snap.position.z - anchor.z).abs() > tol::POINT_MERGE,
        "the Z-locked snap must track the cursor's upward drift, not collapse onto the anchor \
         (got {:?}, anchor {:?})",
        snap.position,
        anchor
    );
}

// ---------------------------------------------------------------------------
// Cross-sketch point snapping under a constraint plane (`off_plane_points`,
// the 3d-line staircase defect). A Line chain that re-homed across sketch
// planes (green on the ground, blue-locked up, red-locked across) must still
// snap back to its own origin vertex — which lives on the FIRST sketch's
// plane, not the current frozen one. The scene below is exactly the saved
// 3d-line.hew repro: three one-segment sketches, one per plane the chain
// re-homed onto; the camera is OBLIQUE (the suite's near-overhead fixtures
// were this bug's documented blind spot) and the aperture is the app's real
// pick cone (8 px at a 900 px / 50° viewport), not a test-friendly wide one.
// ---------------------------------------------------------------------------

/// Mints a distinct `SketchId` per call site — candidate registration is
/// keyed by id with replace semantics, so the three staircase sketches must
/// not share `SketchId::default()`.
fn staircase_sketch_id(n: u64) -> SketchId {
    use slotmap::KeyData;
    SketchId::from(KeyData::from_ffi(n))
}

/// The 3d-line.hew staircase: (0,0,0)→(0,1,0) on the ground, →(0,1,1) on
/// x = 0, →(1,1,1) on y = 1 — three sketches, exactly as the re-homing
/// Line chain commits them.
fn staircase_scene() -> InferenceScene {
    let mut scene = InferenceScene::new();
    scene.add_sketch(
        staircase_sketch_id(1),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        )],
    );
    scene.add_sketch(
        staircase_sketch_id(2),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 1.0),
        )],
    );
    scene.add_sketch(
        staircase_sketch_id(3),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 1.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
        )],
    );
    scene
}

/// An oblique modelling camera (nothing near-overhead about it) and the
/// app's real point-snap aperture: 8 px on a 900 px, 50° viewport.
const STAIRCASE_EYE: Point3 = Point3 {
    x: 5.0,
    y: -4.0,
    z: 3.5,
};
const APP_APERTURE: f64 = 0.00829;

/// The current frozen plane of the staircase chain (y = 1, minted by the
/// red-locked third segment's re-home) with the anchor at (1,1,1) — the
/// exact query `LineTool` issues while the user aims the fourth segment.
fn staircase_query(target: Point3, off_plane_points: bool) -> SnapQuery {
    SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray: PickRay {
            origin: STAIRCASE_EYE,
            direction: target - STAIRCASE_EYE,
        },
        anchor: Some(Point3::new(1.0, 1.0, 1.0)),
        lock: None,
        aperture: APP_APERTURE,
        constraint_plane: Some(
            Plane::from_point_normal(Point3::new(0.0, 1.0, 1.0), Vec3::new(0.0, 1.0, 0.0)).unwrap(),
        ),
        soft_axis_aperture_scale: None,
        off_plane_points,
    }
}

/// With `off_plane_points`, aiming at the chain's own origin resolves the
/// true origin VERTEX — a precise point candidate off the frozen plane must
/// survive the plane filter for the one tool contract (a re-homing line
/// chain) that can honour it. Without the flag this query returned `None`
/// and the app's ray∩plane fallback then invented a point on y = 1 metres
/// behind the visible target — the committed stray vertex in 3d-line.hew.
#[test]
fn off_plane_points_snaps_the_cross_sketch_origin_vertex() {
    let scene = staircase_scene();
    let snap = scene
        .resolve(&staircase_query(Point3::new(0.0, 0.0, 0.0), true))
        .expect("the origin vertex is visible and under the cursor");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(
        snap.position
            .approx_eq(Point3::new(0.0, 0.0, 0.0), tol::POINT_MERGE),
        "must resolve the TRUE origin vertex, got {:?}",
        snap.position
    );
}

/// Without the opt-in (every tool that commits into one frozen plane), the
/// same query still refuses: those tools cannot honour an off-plane point,
/// and a snap that cannot be honoured must not be offered.
#[test]
fn constraint_plane_still_drops_off_plane_points_by_default() {
    let scene = staircase_scene();
    let snap = scene.resolve(&staircase_query(Point3::new(0.0, 0.0, 0.0), false));
    assert!(
        snap.is_none(),
        "no on-plane candidate is anywhere near this ray, got {snap:?}"
    );
}

/// The staircase's THIRD segment, click for click: anchored at (0,1,1) on
/// the frozen x = 0 plane (minted by the blue-locked riser's re-home) with
/// the X axis locked — the lock direction is NORMAL to the frozen plane, so
/// every candidate the constraint plane keeps projects onto the locked line
/// exactly at the anchor. The query is precisely what `LineTool`'s
/// `snapConstraint` issues in that state (anchor + lockAxis +
/// constraintPlane + offPlanePoints).
fn staircase_third_segment_query(target: Point3) -> SnapQuery {
    SnapQuery {
        aperture_mode: ApertureMode::Cone,
        weights: SnapWeights::default(),
        ray: PickRay {
            origin: STAIRCASE_EYE,
            direction: target - STAIRCASE_EYE,
        },
        anchor: Some(Point3::new(0.0, 1.0, 1.0)),
        lock: Some(SnapLock::Axis(inference::Axis::X)),
        aperture: APP_APERTURE,
        constraint_plane: Some(
            Plane::from_point_normal(Point3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap(),
        ),
        soft_axis_aperture_scale: None,
        off_plane_points: true,
    }
}

/// An axis-locked CLICK must land on the locked line at the aimed distance —
/// at EVERY ordinary distance from the anchor, not just wherever no stray
/// candidate happens to sit near the pick ray. Swept, not spot-checked: the
/// original defect only fired inside camera-dependent distance WINDOWS
/// (here the anchor's own Endpoint at very close range, and the world green
/// axis — which lies ON the frozen x = 0 plane — where the extended pick
/// ray grazes it around d ≈ 1.35–1.5; in the in-app drive that found it,
/// 0.85–1.0 m), so any single-distance probe can silently sit outside the
/// window and stay green. Inside a window, a candidate that survives the
/// constraint-plane filter wins on angular rank, its lock projection
/// collapses onto the anchor (the lock is normal to the plane every such
/// candidate lies in), and the honoured snap degenerates into the
/// "same as the last point" refusal — a locked click at a perfectly
/// ordinary distance simply refuses to commit. The winner search must
/// treat a degenerate lock projection as disqualifying (the candidate
/// contributes nothing a locked gesture can commit) and fall through to
/// the next candidate or the directional lock fallback.
#[test]
fn locked_click_on_rehomed_plane_lands_at_every_distance() {
    let mut scene = InferenceScene::new();
    // The chain as committed BEFORE the third click: the green ground
    // segment and the blue-locked riser (both lie on the x = 0 plane).
    scene.add_sketch(
        staircase_sketch_id(1),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        )],
    );
    scene.add_sketch(
        staircase_sketch_id(2),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 1.0),
        )],
    );

    let anchor = Point3::new(0.0, 1.0, 1.0);
    for step in 1..=50 {
        let d = f64::from(step) * 0.05; // 0.05 m … 2.5 m, spanning every window
        let target = Point3::new(d, 1.0, 1.0);
        let snap = scene
            .resolve(&staircase_third_segment_query(target))
            .expect("an axis lock always produces a point");
        assert!(
            (snap.position - anchor).length() > tol::POINT_MERGE,
            "locked click at d = {d} collapsed onto the anchor (kind {:?})",
            snap.kind
        );
        assert!(
            snap.position.approx_eq(target, tol::POINT_MERGE),
            "locked click at d = {d} must land on the locked line at the aimed \
             distance, got {:?} (kind {:?})",
            snap.position,
            snap.kind
        );
        assert_eq!(snap.direction, Some(inference::Axis::X.unit()));
    }
}

/// The lock-projection disqualification line is `tol::POINT_MERGE` — the
/// kernel's own `DegenerateSegment` threshold — and deliberately NOT some
/// looser UI-scale epsilon: a real vertex a few nanometres along the lock
/// from the anchor is a committable segment per the kernel's contract, so
/// the engine must return it at its true projected position rather than
/// discard it. This pins the whole-system agreement: engine
/// disqualification, tool commit gate (`LineTool`'s
/// `DEGENERATE_SEGMENT_EPS`), and kernel refusal all sit on `POINT_MERGE`,
/// so no band can exist where one layer returns a winner another refuses.
#[test]
fn locked_resolve_returns_a_committable_band_scale_vertex() {
    let mut scene = InferenceScene::new();
    scene.add_sketch(
        staircase_sketch_id(1),
        &[(
            SketchEdgeId::default(),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 1.0),
        )],
    );
    // A real vertex 5 nm along +X from the anchor (0,1,1) — off the frozen
    // x = 0 plane, kept by `off_plane_points` (point kinds have no distance
    // gate). Its lock projection is 5e-9: above `POINT_MERGE`, so it is a
    // genuine, committable target — NOT a degenerate one to disqualify.
    scene.add_sketch(
        staircase_sketch_id(2),
        &[(
            SketchEdgeId::default(),
            Point3::new(5e-9, 1.0, 1.0),
            Point3::new(5e-9, 1.0, 0.0),
        )],
    );

    let anchor = Point3::new(0.0, 1.0, 1.0);
    let snap = scene
        .resolve(&staircase_third_segment_query(anchor))
        .expect("an axis lock always produces a point");
    assert_eq!(snap.kind, SnapKind::Endpoint, "the 5 nm vertex must win");
    assert!(
        snap.position.approx_eq(Point3::new(5e-9, 1.0, 1.0), 1e-12),
        "must project to the vertex's true locked position, got {:?}",
        snap.position
    );
    assert!(
        (snap.position - anchor).length() > tol::POINT_MERGE,
        "the returned winner must be committable per the kernel's own line"
    );
}

/// A locked resolve must land on the SIDE of the anchor the gesture
/// indicates — for both signs, on all three axes. The lock-projection
/// disqualification skips candidates that collapse onto the anchor, but
/// avoiding degeneracy is not sufficient: the lock line is infinite in both
/// directions, so a candidate on the WRONG side of the anchor has a
/// perfectly "real" (non-collapsed) projection and would otherwise pass.
/// Here a snappable vertex sits exactly ON the extended pick ray but on the
/// opposite side of the anchor from the aimed station — dead-on angularly,
/// so it wins the ranking outright — and pre-fix the resolve projected it
/// to the opposite side: a Z-locked Move+Alt copy "up" landed BELOW the
/// ground (the donut-copy e2e regression's defect class). The wrong-side
/// candidate must be disqualified like a collapsed one, falling through to
/// the directional fallback at the aimed station.
#[test]
fn locked_resolve_lands_on_the_gesture_side_for_both_signs_on_all_axes() {
    for axis in [inference::Axis::X, inference::Axis::Y, inference::Axis::Z] {
        let u = axis.unit();
        // Two directions perpendicular to the locked axis (cyclic pick).
        let e1 = Vec3::new(u.z, u.x, u.y);
        let e2 = Vec3::new(u.y, u.z, u.x);
        for sign in [1.0_f64, -1.0] {
            let anchor = Point3::ORIGIN;
            // Eye well off the lock line, biased +5 along the axis so the
            // extended ray crosses to the anchor's other side.
            let eye = anchor + e1 * 10.0 + u * 5.0;
            // Aim exactly at the lock line, half a metre to the gesture side.
            let target = anchor + u * (0.5 * sign);
            // A real vertex exactly ON the pick ray (angular distance zero:
            // it outranks everything) whose station along the lock lies on
            // the OPPOSITE side of the anchor from the aimed one.
            let s = if sign > 0.0 { 1.2 } else { 0.8 };
            let stray = eye + (target - eye) * s;
            debug_assert!((stray - anchor).dot(u) * sign < 0.0);
            let mut scene = InferenceScene::new();
            scene.add_sketch(
                staircase_sketch_id(1),
                &[(SketchEdgeId::default(), stray, stray + e2 * 3.0)],
            );

            let snap = scene
                .resolve(&SnapQuery {
                    aperture_mode: ApertureMode::Cone,
                    weights: SnapWeights::default(),
                    ray: PickRay {
                        origin: eye,
                        direction: target - eye,
                    },
                    anchor: Some(anchor),
                    lock: Some(SnapLock::Axis(axis)),
                    aperture: APP_APERTURE,
                    constraint_plane: None,
                    soft_axis_aperture_scale: None,
                    off_plane_points: false,
                })
                .expect("a lock with an anchor always resolves");
            let station = (snap.position - anchor).dot(u);
            assert!(
                station * sign > 0.0,
                "{axis:?} lock, sign {sign}: locked resolve landed on the wrong side of \
                 the anchor (station {station}, kind {:?})",
                snap.kind
            );
            assert!(
                snap.position.approx_eq(target, tol::POINT_MERGE),
                "{axis:?} lock, sign {sign}: must land at the aimed station, got {:?} \
                 (kind {:?})",
                snap.position,
                snap.kind
            );
            assert_eq!(snap.direction, Some(u));
        }
    }
}

/// The wrong-side gate above judges the SIDE the cursor indicates from
/// `t_fallback` — the station where the pick ray comes closest to the lock
/// line. That reading only holds while the cursor is aiming AT the lock
/// line. Off it, `t_fallback` is set by where the EYE is: the ray's closest
/// approach to a vertical lock line passes above the anchor for geometry on
/// the far side of it and below for geometry on the near side, for the very
/// same vertex, purely from orbiting. Applying the gate there threw away
/// legitimately-hovered geometry, which is the whole mechanism a projected
/// snap works by — and made WHICH points survived change with every camera
/// move (the reported defect: a floating rectangle corner Z-locked back down
/// to the ground could reach one plate corner from one camera, none from the
/// next).
///
/// The property is camera invariance: hovering a real vertex under a lock
/// must resolve to that vertex's own station from EVERY camera that can see
/// it. Swept around the full azimuth, on all three axes, with the vertex on
/// the NEGATIVE side of the anchor (the direction the defect suppressed).
#[test]
fn a_locked_hover_on_geometry_resolves_its_station_from_every_camera() {
    for axis in [inference::Axis::X, inference::Axis::Y, inference::Axis::Z] {
        let u = axis.unit();
        // Two directions perpendicular to the locked axis (cyclic pick).
        let e1 = Vec3::new(u.z, u.x, u.y);
        let e2 = Vec3::new(u.y, u.z, u.x);

        // The grabbed point, 1 m up the lock from the origin.
        let anchor = Point3::ORIGIN + u;
        // A vertex to drop onto: at station 0 (a metre BELOW the anchor
        // along the lock) and well off the lock line, exactly as a ground
        // vertex sits under a floating sketch. Carried by a short edge so
        // the scene has a real Endpoint candidate.
        let vertex = Point3::ORIGIN + e1 * 2.0 + e2 * 2.0;
        let mut scene = InferenceScene::new();
        scene.add_sketch(
            staircase_sketch_id(1),
            &[(SketchEdgeId::default(), vertex, vertex + e1 * 0.5)],
        );

        // 16 eyes on a ring around the scene, all above it, every one with
        // the vertex in plain sight. Nothing in the scene moves.
        let mut tested = 0;
        for i in 0..16 {
            let theta = std::f64::consts::TAU * f64::from(i) / 16.0;
            let eye =
                Point3::ORIGIN + e1 * (8.0 * theta.cos()) + e2 * (8.0 * theta.sin()) + u * 6.0;

            // Skip the azimuths whose ray to the vertex ALSO passes within
            // the pick aperture of the lock line itself. There the cursor is
            // on the drawn guide line as well as on the vertex, and which of
            // the two the gesture means is the genuinely ambiguous case that
            // `locked_resolve_lands_on_the_gesture_side_for_both_signs_on_all_axes`
            // above already adjudicates in the guide line's favour — a
            // different question from this one. The count assertion after
            // the loop keeps the exclusion honest: it must stay a handful of
            // degenerate viewpoints and never quietly grow to swallow the
            // property under test.
            //
            // The aperture is scaled by the distance to the VERTEX, where
            // the resolve scales by the distance to `fall` — deliberately
            // approximate, so this stays a plain skew-line distance instead
            // of a second copy of the production clamp. It is only ever
            // asked to separate "the ray crosses the lock line" (lateral
            // exactly 0 here) from "it misses by metres" (> 0.8 here), with
            // either threshold landing around 0.06–0.1 in between.
            let to_vertex = vertex - eye;
            let miss = anchor - eye;
            let lateral = match to_vertex.cross(u).normalized() {
                // Distance from the lock line to the ray, along their common
                // perpendicular — zero exactly when the two lines cross.
                Ok(n) => miss.dot(n).abs(),
                // Ray parallel to the lock: no crossing to be ambiguous with.
                Err(_) => f64::INFINITY,
            };
            if lateral < APP_APERTURE * to_vertex.length() {
                continue;
            }
            tested += 1;

            let snap = scene
                .resolve(&SnapQuery {
                    aperture_mode: ApertureMode::Cone,
                    weights: SnapWeights::default(),
                    ray: PickRay {
                        origin: eye,
                        direction: vertex - eye,
                    },
                    anchor: Some(anchor),
                    lock: Some(SnapLock::Axis(axis)),
                    aperture: APP_APERTURE,
                    constraint_plane: None,
                    soft_axis_aperture_scale: None,
                    off_plane_points: false,
                })
                .expect("a lock with an anchor always resolves");
            assert_eq!(
                snap.kind,
                SnapKind::Endpoint,
                "{axis:?} lock, azimuth {i}/16: the hovered vertex must be the winner, \
                 not the directional fallback"
            );
            // Its station is the vertex's own: a metre back down the lock.
            assert!(
                snap.position.approx_eq(Point3::ORIGIN, tol::POINT_MERGE),
                "{axis:?} lock, azimuth {i}/16: must project the hovered vertex onto the \
                 lock line at its own station, got {:?}",
                snap.position
            );
            assert_eq!(snap.direction, Some(u));
        }
        assert!(
            tested >= 12,
            "{axis:?} lock: only {tested}/16 azimuths were unambiguous — the \
             on-the-guide-line exclusion has grown past the degenerate cases \
             and this spec is no longer testing camera invariance"
        );
    }
}

/// Hovering the anchor ITSELF under a lock must resolve to the anchor
/// exactly — zero displacement — not to a noise-signed station. The
/// reconstructed cursor ray never re-crosses the anchor exactly (pixel
/// rounding leaves a sub-pixel miss), and every station the ray itself
/// produces (the directional fallback's, or a ray-sliding
/// `OnAxis`/`OnEdge`/`OnFace`/`OnGuide` candidate's) is that signed miss
/// verbatim. Pre-fix, the disqualification of the anchor's own collapsed
/// Endpoint let exactly those noise stations win, and a tool deriving a
/// typed-entry direction from the resolved station amplified the noise
/// SIGN into a full-magnitude gesture the wrong way: the Move+Alt donut
/// e2e's Z-locked copy of `0.5` landed at `-0.5`. Swept over both miss
/// signs on all three axes, at a sub-pixel miss scale.
#[test]
fn locked_hover_on_the_anchor_resolves_the_anchor_exactly() {
    for axis in [inference::Axis::X, inference::Axis::Y, inference::Axis::Z] {
        let u = axis.unit();
        let e1 = Vec3::new(u.z, u.x, u.y);
        let e2 = Vec3::new(u.y, u.z, u.x);
        for miss in [1e-4_f64, -1e-4] {
            let anchor = Point3::ORIGIN;
            // The donut-copy camera shape, axis-generic: looking straight
            // down the locked axis from 6 m, slightly off to one side.
            let eye = anchor + u * 6.0 + e1 * 0.8 + e2 * 0.8;
            // Aimed at the anchor, missing by a sub-pixel hair along the
            // lock — the reconstruction-noise band, in both signs.
            let target = anchor + u * miss;
            // The anchor is a real sketch corner (its Endpoint candidate
            // collapses onto the anchor and is rightly disqualified — it
            // must not drag a noise station in behind it).
            let mut scene = InferenceScene::new();
            scene.add_sketch(
                staircase_sketch_id(1),
                &[
                    (SketchEdgeId::default(), anchor, anchor + e1 * 1.6),
                    (SketchEdgeId::default(), anchor, anchor + e2 * 1.6),
                ],
            );

            let snap = scene
                .resolve(&SnapQuery {
                    aperture_mode: ApertureMode::Cone,
                    weights: SnapWeights::default(),
                    ray: PickRay {
                        origin: eye,
                        direction: target - eye,
                    },
                    anchor: Some(anchor),
                    lock: Some(SnapLock::Axis(axis)),
                    aperture: APP_APERTURE,
                    constraint_plane: None,
                    soft_axis_aperture_scale: None,
                    off_plane_points: false,
                })
                .expect("a lock with an anchor always resolves");
            assert!(
                snap.position.approx_eq(anchor, tol::POINT_MERGE),
                "{axis:?} lock, miss {miss:+e}: hovering the anchor must resolve the \
                 anchor exactly, got {:?} (kind {:?}) — a noise-signed station here \
                 inverts typed Move/Line commits",
                snap.position,
                snap.kind
            );
            assert_eq!(snap.direction, Some(u));
        }
    }
}

/// The noise cull's test is EQUALITY with the directional fallback point —
/// never a proximity band. Here the anchor sits near the camera, well OFF
/// the ray's own path (the on-anchor zone does not apply), while the user
/// aims squarely at a real, visually distinguishable edge far along the
/// same ray, one full metre along the lock. Nothing on screen is
/// ambiguous, and a one-metre displacement is a thing a user actually
/// wants — yet a depth-scaled station ENVELOPE (`depth · tan(fraction ·
/// aperture)`) is ~1.5 m wide at this edge's depth and would swallow the
/// genuine station, resolving instead to the fallback's own near-zero
/// noise station at the anchor. Bounded is not the same as small enough:
/// the only station that is truly "the ray's own miss" is the fallback
/// point itself, so the cull must compare the candidate against that one
/// point and nothing wider.
#[test]
fn locked_resolve_off_the_anchor_keeps_a_far_real_edge_at_its_true_station() {
    let mut scene = InferenceScene::new();
    // A horizontal edge 1 m up, crossing the line of sight 300 m out; both
    // ends and the (asymmetric) midpoint sit outside the cone, so the
    // ray-sliding OnEdge candidate is what must survive.
    scene.add_sketch(
        staircase_sketch_id(1),
        &[(
            SketchEdgeId::default(),
            Point3::new(-15.0, 300.0, 1.0),
            Point3::new(40.0, 300.0, 1.0),
        )],
    );
    let anchor = Point3::new(0.02, 0.0, 0.0);
    let snap = scene
        .resolve(&SnapQuery {
            aperture_mode: ApertureMode::Cone,
            weights: SnapWeights::default(),
            // The ray runs past the anchor (2 cm off its path at 2 m depth,
            // ~0.01 rad — twice the on-anchor zone's half-angle) toward the
            // far edge (1 m off at ~300 m, ~0.003 rad — well inside the
            // cone).
            ray: ray_at(Point3::new(0.0, -2.0, 0.0), Point3::new(0.0, 300.0, 0.0)),
            anchor: Some(anchor),
            lock: Some(SnapLock::Axis(inference::Axis::Z)),
            aperture: 0.02,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        })
        .expect("a lock with an anchor always resolves");
    assert_eq!(
        snap.kind,
        SnapKind::OnEdge,
        "the aimed-at far edge must win, got {:?} at {:?}",
        snap.kind,
        snap.position
    );
    assert!(
        snap.position
            .approx_eq(Point3::new(0.02, 0.0, 1.0), tol::POINT_MERGE),
        "the edge's full one-metre station must project onto the locked line, got {:?}",
        snap.position
    );
    assert_eq!(snap.direction, Some(inference::Axis::Z.unit()));
}

/// A construction guide COLLINEAR with the locked line must resolve
/// exactly as the lock's own fallback does, however the guide happens to
/// be parameterized. `closest_point_on_line_to_ray`'s reach clamp used to
/// derive its bound from the line's OWN origin, so a collinear guide whose
/// origin sits near the camera got a far tighter clamp than the fallback
/// (parameterized from the anchor) under near-parallel viewing — the same
/// geometric construction, computed twice, metres apart. The winner cull's
/// noise-equality test then could not see they were the same thing, the
/// on-anchor state had suspended the wrong-side gate, and the guide WON
/// with the corrupted station: cursor dead on the anchor, resolve tens of
/// metres up the lock line. The clamp now re-origins every anchored call
/// at the anchor's own foot on the line (the function's parameterization
/// invariance), making the guide's point identical to the fallback's, so
/// the equality cull removes it and the resolve returns the honest
/// zero-displacement answer.
#[test]
fn collinear_guide_parameterized_near_the_camera_cannot_corrupt_a_locked_hover() {
    let mut scene = InferenceScene::new();
    // A vertical guide on the lock line itself, with its origin 2 m from
    // the camera — the parameterization that used to clamp differently.
    scene.add_guide(
        GuideId::default(),
        &Guide::Line {
            origin: Point3::new(0.0, 0.0, 38.0),
            direction: Vec3::new(0.0, 0.0, 1.0),
        },
    );
    let anchor = Point3::ORIGIN;
    let snap = scene
        .resolve(&SnapQuery {
            aperture_mode: ApertureMode::Cone,
            weights: SnapWeights::default(),
            // Near-parallel viewing (~0.14 degrees off the lock line), aimed
            // exactly at the anchor: the ill-conditioned band where the old
            // origin-relative clamp diverged by tens of metres.
            ray: ray_at(Point3::new(0.1, 0.0, 40.0), anchor),
            anchor: Some(anchor),
            lock: Some(SnapLock::Axis(inference::Axis::Z)),
            aperture: APP_APERTURE,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        })
        .expect("a lock with an anchor always resolves");
    assert!(
        snap.position.approx_eq(anchor, tol::POINT_MERGE),
        "hovering the anchor must resolve the anchor exactly — the collinear guide's \
         differently-clamped point must not win (got {:?}, kind {:?})",
        snap.position,
        snap.kind
    );
    assert_eq!(snap.direction, Some(inference::Axis::Z.unit()));
}

/// The clamp's anchor reach-reference is scoped to the ONE case the
/// winner cull depends on it — an active lock collinear with the
/// candidate's line — and an ordinary anchored-but-UNLOCKED query gets
/// the candidate's own natural origin and bound, bit for bit. An anchor
/// exists from the first click of nearly every drawing tool, and with no
/// lock the winner's raw position is returned unprojected, so an
/// unconditional anchor reference reached the user directly: with the
/// anchor sitting far along the hovered axis, the near-edge-on clamp
/// bound inflated from the camera-to-origin scale to the
/// camera-to-anchor's-foot scale, and the identical ray resolved ~100 m
/// farther along the axis than the same hover without an anchor. The
/// two resolves here must agree EXACTLY (`==`, not a tolerance): with no
/// lock the reach reference must be None, making both calls
/// argument-identical.
#[test]
fn unlocked_anchored_hover_matches_the_anchorless_answer_exactly() {
    let scene = InferenceScene::new();
    // Near-edge-on down the world X axis (~0.37 degrees), where the reach
    // clamp engages: the natural bound is camera-to-origin scaled.
    let eye = Point3::new(-20.0, 1.0, 1.0);
    let target = Point3::new(200.0, 0.0, 0.0);
    let resolve = |anchor: Option<Point3>| {
        scene
            .resolve(&SnapQuery {
                aperture_mode: ApertureMode::Cone,
                weights: SnapWeights::default(),
                ray: ray_at(eye, target),
                anchor,
                lock: None,
                aperture: 0.02,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
            })
            .expect("the world X axis is under this ray")
    };
    let with_anchor = resolve(Some(Point3::new(40.0, 0.0, 0.0)));
    let without = resolve(None);
    assert_eq!(with_anchor.kind, SnapKind::OnAxis);
    assert_eq!(without.kind, SnapKind::OnAxis);
    assert_eq!(
        with_anchor.position, without.position,
        "an unlocked anchored hover must return the candidate's own natural answer, \
         identical to the anchorless one"
    );
}

/// Under an active lock, a candidate line NOT collinear with the lock
/// keeps its own natural clamp too — the anchor reference applies only
/// to the collinear family the fallback-equality cull compares. This is
/// the variant an unconditional reference corrupts most quietly: for the
/// frame's own axes the inflation slides the candidate along its own
/// line, which a non-parallel lock projection never reads, but a SKEW
/// guide's direction has a component along the lock, so the inflation
/// leaks straight into the projected station — here a Z-locked hover
/// down a diagonal guide (anchored 40 units along it, the ordinary
/// "first click landed on the guide" state) resolved a station twice as
/// far up the lock line as the guide's natural point. The locked
/// station must equal the natural (anchorless, unlocked) guide point's
/// station.
#[test]
fn locked_resolve_keeps_a_skew_guides_natural_clamp() {
    let u = Vec3::new(1.0, 0.0, 1.0).normalized().unwrap();
    let mut scene = InferenceScene::new();
    scene.set_axes_enabled(false); // isolate the guide candidate
    scene.add_guide(
        GuideId::default(),
        &Guide::Line {
            origin: Point3::ORIGIN,
            direction: u,
        },
    );
    let anchor = Point3::ORIGIN + u * 40.0;
    let eye = Point3::ORIGIN + u * -20.0 + Vec3::new(0.0, 1.0, 0.0);
    let target = Point3::ORIGIN + u * 200.0;
    let resolve = |anchor: Option<Point3>, lock: Option<SnapLock>| {
        scene
            .resolve(&SnapQuery {
                aperture_mode: ApertureMode::Cone,
                weights: SnapWeights::default(),
                ray: ray_at(eye, target),
                anchor,
                lock,
                aperture: 0.02,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
            })
            .expect("the guide is under this ray")
    };
    let natural = resolve(None, None);
    assert_eq!(natural.kind, SnapKind::OnGuide);
    let locked = resolve(Some(anchor), Some(SnapLock::Axis(inference::Axis::Z)));
    assert_eq!(locked.kind, SnapKind::OnGuide);
    assert!(
        (locked.position.z - natural.position.z).abs() <= tol::POINT_MERGE,
        "the Z-locked station must come from the guide's natural point \
         (z = {}), not an anchor-inflated one (got z = {})",
        natural.position.z,
        locked.position.z
    );
}

/// The collinear case itself — the reason the reach reference exists —
/// still holds away from the anchor: a guide ON the lock line, however
/// parameterized, resolves to the same point as the lock's own fallback,
/// so the equality cull removes it and the resolve lands at the aimed
/// station via the fallback. The camera here is ~10 degrees off the lock
/// (stations are screen-distinguishable, so the on-anchor clamp does not
/// apply) and the aim is 2 m up the line.
#[test]
fn collinear_guide_matches_the_fallback_at_an_aimed_station() {
    let mut scene = InferenceScene::new();
    scene.add_guide(
        GuideId::default(),
        &Guide::Line {
            origin: Point3::new(0.0, 0.0, 38.0),
            direction: Vec3::new(0.0, 0.0, 1.0),
        },
    );
    let anchor = Point3::ORIGIN;
    let snap = scene
        .resolve(&SnapQuery {
            aperture_mode: ApertureMode::Cone,
            weights: SnapWeights::default(),
            ray: ray_at(Point3::new(1.8, 0.0, 12.0), Point3::new(0.0, 0.0, 2.0)),
            anchor: Some(anchor),
            lock: Some(SnapLock::Axis(inference::Axis::Z)),
            aperture: APP_APERTURE,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        })
        .expect("a lock with an anchor always resolves");
    assert!(
        snap.position
            .approx_eq(Point3::new(0.0, 0.0, 2.0), tol::POINT_MERGE),
        "the locked resolve must land at the aimed station via the fallback, got {:?} \
         (kind {:?})",
        snap.position,
        snap.kind
    );
    assert_eq!(snap.direction, Some(inference::Axis::Z.unit()));
}

/// The noise cull judges each CANDIDATE by its own station — never by
/// where the ray points. With the pick ray aimed EXACTLY at the anchor
/// (dead inside the on-anchor zone), a real edge crossing the pick cone
/// metres along the lock must still win at its true station, on either
/// side of the anchor: its station is geometry, far outside the
/// pixel-derived noise radius. A ray-based cull here would discard it,
/// clamp the resolve to the anchor, and turn a legitimate locked
/// click-to-place into a silent no-op (Move's degenerate guard) or a
/// refused click (Line) — and a typed Move commit would then default to
/// +lock regardless of which side the hovered edge was on: the same
/// wrong-side commit the noise cull exists to prevent, triggered by real
/// geometry instead of noise.
#[test]
fn locked_hover_on_the_anchor_still_takes_real_geometry_at_its_true_station() {
    for side in [2.0_f64, -2.0] {
        let mut scene = InferenceScene::new();
        // A horizontal edge crossing the line of sight `side` metres along
        // the lock from the anchor. Both ends and the (asymmetric) midpoint
        // sit far outside even this WIDE cone, so the ray-sliding OnEdge
        // candidate — not a point-kind Endpoint/Midpoint — is what must
        // survive.
        scene.add_sketch(
            staircase_sketch_id(1),
            &[(
                SketchEdgeId::default(),
                Point3::new(-15.0, 0.0, side),
                Point3::new(40.0, 0.0, side),
            )],
        );
        let snap = scene
            .resolve(&SnapQuery {
                aperture_mode: ApertureMode::Cone,
                weights: SnapWeights::default(),
                // Aimed EXACTLY at the anchor: the on-anchor zone's own
                // centre, the case a ray-based cull gets wrong.
                ray: ray_at(Point3::new(0.0, -20.0, 0.0), Point3::ORIGIN),
                anchor: Some(Point3::ORIGIN),
                lock: Some(SnapLock::Axis(inference::Axis::Z)),
                aperture: WIDE,
                constraint_plane: None,
                soft_axis_aperture_scale: None,
                off_plane_points: false,
            })
            .expect("a lock with an anchor always resolves");
        assert_eq!(
            snap.kind,
            SnapKind::OnEdge,
            "side {side}: the hovered edge must win, got {:?} at {:?}",
            snap.kind,
            snap.position
        );
        assert!(
            snap.position
                .approx_eq(Point3::new(0.0, 0.0, side), tol::POINT_MERGE),
            "side {side}: the edge's real station must project onto the locked line, \
             got {:?}",
            snap.position
        );
        assert_eq!(snap.direction, Some(inference::Axis::Z.unit()));
    }
}

/// The guard for the fix above: a ray-sliding candidate whose station is
/// REAL geometry (not on-anchor ray noise) must keep winning under a lock.
/// Z-locked, cursor hovering a horizontal edge 3 m up and well away from
/// the anchor: the OnEdge candidate's projection carries the edge's height
/// onto the locked line — "lift to that edge's height" — and must not be
/// culled by either of the new disqualifications (it is on the gesture's
/// side, and the cursor is nowhere near the anchor).
#[test]
fn locked_resolve_still_takes_a_sliding_candidate_at_its_real_station() {
    let mut scene = InferenceScene::new();
    scene.add_sketch(
        staircase_sketch_id(1),
        &[(
            SketchEdgeId::default(),
            Point3::new(-5.0, 2.0, 3.0),
            Point3::new(5.0, 2.0, 3.0),
        )],
    );
    let snap = scene
        .resolve(&SnapQuery {
            aperture_mode: ApertureMode::Cone,
            weights: SnapWeights::default(),
            ray: ray_at(Point3::new(0.0, -10.0, 4.0), Point3::new(1.0, 2.0, 3.0)),
            anchor: Some(Point3::ORIGIN),
            lock: Some(SnapLock::Axis(inference::Axis::Z)),
            aperture: APP_APERTURE,
            constraint_plane: None,
            soft_axis_aperture_scale: None,
            off_plane_points: false,
        })
        .expect("a lock with an anchor always resolves");
    assert_eq!(snap.kind, SnapKind::OnEdge);
    assert!(
        snap.position
            .approx_eq(Point3::new(0.0, 0.0, 3.0), tol::POINT_MERGE),
        "the hovered edge's height must project onto the locked line, got {:?}",
        snap.position
    );
}

/// `off_plane_points` loosens only the PLANE filter, never the occlusion
/// cull: the hidden bottom corner of a solid stays unsnappable while
/// drawing on its top face, so the see-through protection the plane filter
/// was introduced for (the rectangle-on-face abort bug) is fully preserved.
#[test]
fn off_plane_points_never_defeats_occlusion() {
    let scene = cube_scene();
    // Straight-down ray entering the top face interior at (0.3, 0.3); the
    // wide cone also catches the hidden bottom corner (0,0,0) — an
    // Endpoint, which `off_plane_points` now lets PAST the plane filter.
    let top =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let snap = scene
        .resolve(&SnapQuery {
            aperture_mode: ApertureMode::Cone,
            weights: SnapWeights::default(),
            ray: PickRay {
                origin: Point3::new(0.3, 0.3, 4.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            anchor: None,
            lock: None,
            aperture: 0.6,
            constraint_plane: Some(top),
            soft_axis_aperture_scale: None,
            off_plane_points: true,
        })
        .expect("the visible top face keeps a candidate in the cone");
    assert!(
        snap.position.z > 0.5,
        "occlusion must still hide the bottom corner from an off-plane-points query: {:?}",
        snap.position
    );
}
