//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{
    ApertureMode, CrossedEdge, ElementRef, FacePolicy, InferenceScene, POINT_SNAP_REACH, PickRay,
    PiercedFace, SOFT_AXIS_SCREEN_REACH, Snap, SnapKind, SnapLock, SnapQuery, SnapWeights,
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
        see_through_points: false,
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

// ── Ranking ties: nearest depth wins among screen-coincident points ──────

/// Two endpoint-class points stacked along the view ray — a wall-top corner
/// directly above a slab corner, seen from a Top view — differ laterally
/// only by float noise (a placed instance's corner composed through its
/// pose vs. a world vertex read exactly). That noise must not decide the
/// pick: the NEARER one wins, in both aperture modes. Guide points are the
/// simplest endpoint-class candidate to stage this with.
fn stacked_scene(near_lateral_noise: f64) -> InferenceScene {
    let mut scene = InferenceScene::new();
    // Nearer, with a hair of lateral noise (the "placed" corner).
    scene.add_guide(
        GuideId::default(),
        &Guide::Point {
            position: Point3::new(near_lateral_noise, 0.0, 1.0),
        },
    );
    // Farther, laterally exact (the "world" corner). A distinct id: `add_guide`
    // replaces a same-id guide.
    scene.add_guide(
        GuideId::from(slotmap::KeyData::from_ffi(2)),
        &Guide::Point {
            position: Point3::new(0.0, 0.0, 5.0),
        },
    );
    scene
}

#[test]
fn screen_coincident_points_rank_by_depth_not_by_float_noise() {
    let noise = 1e-12;
    for (name, query) in [
        ("cone", cone_query(CYL_APERTURE_RAD)),
        ("cylinder", cylinder_query(0.1)),
    ] {
        let hit = stacked_scene(noise)
            .resolve(&query)
            .unwrap_or_else(|| panic!("{name}: a stacked pair is inside the aperture"));
        assert!(
            (hit.position.z - 1.0).abs() < 1e-9,
            "{name}: the nearer point wins the pick, not the laterally exact far one (got z = {})",
            hit.position.z
        );
    }
}

#[test]
fn a_dense_chain_of_near_tied_candidates_ranks_without_panicking() {
    // Lateral offsets stepping by 0.7 of the tie window: each neighbour is
    // "equal within the tie" but the ends of the chain are not — the
    // non-transitive case that made an epsilon comparator panic inside
    // `sort_by` on a real floor plan. The pick must still resolve, to the
    // nearest-depth candidate of the nearest bucket.
    let radius = 0.1;
    let tie = radius * inference::RANK_TIE_FRACTION;
    let mut scene = InferenceScene::new();
    for i in 0..80u64 {
        let lateral = 0.7 * tie * (i % 7) as f64;
        scene.add_guide(
            GuideId::from(slotmap::KeyData::from_ffi(i + 1)),
            &Guide::Point {
                position: Point3::new(lateral, 0.0, 1.0 + i as f64 * 0.05),
            },
        );
    }
    let hit = scene
        .resolve(&cylinder_query(radius))
        .expect("a dense stack is inside the aperture");
    assert!(
        (hit.position.z - 1.0).abs() < 1e-9,
        "the nearest of the laterally tied points wins (got z = {})",
        hit.position.z
    );
}

#[test]
fn rank_buckets_are_a_total_order() {
    use inference::rank_distance_bucket;
    let tie = 1e-7;
    let xs: Vec<f64> = (0..200).map(|i| i as f64 * 0.7 * tie).collect();
    let mut keys: Vec<i64> = xs.iter().map(|&x| rank_distance_bucket(x, tie)).collect();
    keys.sort();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(rank_distance_bucket(f64::NAN, tie), i64::MAX);
    assert_eq!(
        rank_distance_bucket(1.0, 0.0),
        rank_distance_bucket(1.0, 0.0)
    );
    assert!(rank_distance_bucket(1.0, 0.0) < rank_distance_bucket(2.0, 0.0));
}

#[test]
fn a_genuinely_closer_lateral_candidate_still_beats_a_nearer_depth() {
    // The tie window is a millionth of the aperture; a candidate a tenth of
    // the aperture closer laterally is a real preference, depth or not.
    let lateral = 0.1 * 0.1; // a tenth of the cylinder radius
    let scene = stacked_scene(lateral);
    let hit = scene.resolve(&cylinder_query(0.1)).expect("inside");
    assert!(
        (hit.position.z - 5.0).abs() < 1e-9,
        "the laterally closer far point wins over the noisy near one"
    );
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
        see_through_points: false,
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
        see_through_points: false,
    }
}

/// At the query's own reference depth, `radius = depth * tan(aperture)`
/// makes the cylinder and the cone agree exactly (design camera.md §1): a
/// candidate just inside one tolerance is just inside the other, and a
/// candidate just outside one is just outside the other. The candidate is a
/// point, so both tolerances reach `POINT_SNAP_REACH` times as far.
#[test]
fn cylinder_cone_equivalence_at_target_distance() {
    let radius = CYL_REF_DEPTH * CYL_APERTURE_RAD.tan();
    let reach = radius * POINT_SNAP_REACH;

    let inside = single_point_scene(0.9 * reach, CYL_REF_DEPTH);
    assert!(
        inside.resolve(&cone_query(CYL_APERTURE_RAD)).is_some(),
        "cone admits a point safely inside its aperture at the reference depth"
    );
    assert!(
        inside.resolve(&cylinder_query(radius)).is_some(),
        "cylinder admits the same point at the equivalent radius"
    );

    let outside = single_point_scene(1.1 * reach, CYL_REF_DEPTH);
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
        see_through_points: false,
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
        see_through_points: false,
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
            span: kernel::GuideSpan::INFINITE,
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
            see_through_points: false,
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

/// A weighted kind reaches BEYOND its base reach: the quadrant snaps from
/// 2.0 apertures away (past the 1.75x `POINT_SNAP_REACH`, inside its 2.5x
/// gravity) where the unweighted query has nothing at all to offer.
#[test]
fn gravity_reaches_past_the_plain_aperture() {
    let (scene, quadrant, _vertex) = circle_gravity_fixture();
    // Radially outward from the circle, away from every facet: at 0.12 m the
    // aim is 0.04 rad off-axis — past the 1.75 x 0.02 point reach, inside
    // 2.5 x 0.02.
    let outward = (quadrant - GRAVITY_CIRCLE_CENTER).normalized().unwrap();
    let aim = quadrant + outward * 0.12;

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

    // 0.12 m off at 3 m overhead is 0.04 rad — past the 1.75 x 0.02 point
    // reach, inside the 2.5x analytic-point gravity.
    let aim = center + Vec3::new(0.12, 0.0, 0.0);
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

/// Reach never steals. A quadrant admitted ONLY by its gravity — past its
/// base `POINT_SNAP_REACH` — must lose to anything inside the plain aperture,
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
    let aim = quadrant + outward * 0.12; // 0.04 rad: past 1.75x 0.02, inside 2.5x
    // Deliberately lopsided: a symmetric segment would put its Midpoint — a
    // stronger kind — exactly under the cursor and prove less. Both ends and
    // the midpoint sit past `POINT_SNAP_REACH` of the aim.
    scene.add_transient_segment(
        aim + Vec3::new(0.0, -0.2, 0.0),
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
    // 5 units along +X from the anchor, drifted 0.04 in Y: atan(0.04/5) ≈
    // 0.46°, inside the ~5° soft-axis tolerance, and 0.008 rad on screen
    // from 5 m up, inside the on-screen cap (SOFT_AXIS_SCREEN_REACH x NARROW).
    let target = Point3::new(7.0, 3.04, 0.0);
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
        see_through_points: false,
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

/// The drag-direction angle alone does not capture: 0.3 off the axis at
/// the end of a 5-unit drag is inside the ~5° tolerance (≈ 3.43°) but six
/// pick radii away on screen, past `SOFT_AXIS_SCREEN_REACH`. Without the
/// on-screen cap the axis pulled the cursor in from across the screen on
/// every long drag and held it off any point near the axis's far end.
#[test]
fn soft_axis_does_not_capture_from_across_the_screen() {
    let scene = InferenceScene::new();
    let anchor = Point3::new(2.0, 3.0, 0.0);
    let target = Point3::new(7.0, 3.3, 0.0);
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
        see_through_points: false,
    };
    assert_eq!(scene.resolve(&q), None);

    // Widening the query to put the axis inside the cap brings it back:
    // the cap, not the angle, is what refused it.
    let wide = SnapQuery {
        aperture: 0.06 / SOFT_AXIS_SCREEN_REACH * 1.05,
        ..q
    };
    assert_eq!(scene.resolve(&wide).map(|s| s.kind), Some(SnapKind::OnAxis));
}

/// Axis × edge crossings: dragging along a soft axis from an anchor on one
/// vertical edge of a face and hovering the OPPOSITE vertical edge resolves
/// to the exact point where the axis line through the anchor crosses that
/// edge — as an `Intersection` carrying the edge's provenance and the
/// axis direction. Before this candidate existed, `OnAxis` (a rank group
/// above `OnEdge`) won here and the edge was unreachable: the Line tool
/// "visibly following the red axis" with "no snapping to the edge at all".
#[test]
fn soft_axis_crossing_an_edge_snaps_the_crossing_as_an_intersection() {
    let scene = cube_scene();
    // Anchor part-way up the cube's (0,0,z) edge; the +X soft axis through
    // it crosses the (1,0,z) edge at (1, 0, 0.3).
    let anchor = Point3::new(0.0, 0.0, 0.3);
    let crossing = Point3::new(1.0, 0.0, 0.3);
    // Aimed a hair off the crossing along the edge (still inside the pick
    // cone), from a 3/4 view in front of the y=0 face.
    let aim = Point3::new(1.0, 0.0, 0.31);
    let ray = ray_at(Point3::new(2.0, -3.0, 1.5), aim);
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
        see_through_points: false,
    };
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(
        snap.kind,
        SnapKind::Intersection,
        "the crossing beats both OnAxis and OnEdge"
    );
    assert!(
        snap.position.approx_eq(crossing, tol::POINT_MERGE),
        "{:?}",
        snap.position
    );
    assert_eq!(
        snap.direction,
        Some(Vec3::new(1.0, 0.0, 0.0)),
        "carries the axis for the cue"
    );
    let source = snap.source.expect("carries the crossed edge's provenance");
    assert!(matches!(source.element, ElementRef::Edge(_)));
}

/// The same crossing without an anchor: a DRAWING axis through the origin
/// crossing a drawn sketch edge snaps the crossing (sketch-edge provenance),
/// not the axis line's ray-nearest point beside it.
#[test]
fn drawing_axis_crossing_a_sketch_edge_snaps_the_crossing() {
    let mut scene = InferenceScene::new();
    // A sketch segment crossing the +X axis at (3, 0, 0) — deliberately
    // NOT at its own midpoint (3, 0.5, 0), which would outrank the crossing
    // as the stronger kind and is the right answer when they coincide.
    scene.add_sketch(
        SketchId::default(),
        &[(
            SketchEdgeId::default(),
            Point3::new(3.0, -1.0, 0.0),
            Point3::new(3.0, 2.0, 0.0),
        )],
    );
    let aim = Point3::new(3.0, 0.02, 0.0);
    let ray = ray_at(Point3::new(3.5, -2.0, 3.0), aim);
    let snap = resolve(&scene, query(ray, NARROW)).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection);
    assert!(
        snap.position
            .approx_eq(Point3::new(3.0, 0.0, 0.0), tol::POINT_MERGE),
        "{:?}",
        snap.position
    );
    assert_eq!(snap.direction, Some(Vec3::new(1.0, 0.0, 0.0)));
    assert!(
        snap.sketch_source.is_some(),
        "carries the crossed sketch edge"
    );
}

/// A real vertex AT the crossing still wins (Endpoint outranks
/// Intersection), so the new candidate never masks exact geometry.
#[test]
fn a_vertex_at_an_axis_crossing_still_wins() {
    let scene = cube_scene();
    // The +X soft axis through (0,0,1) crosses the (1,0,z) edge exactly at
    // the (1,0,1) corner.
    let anchor = Point3::new(0.0, 0.0, 1.0);
    let corner = Point3::new(1.0, 0.0, 1.0);
    let ray = ray_at(Point3::new(2.0, -3.0, 2.0), corner);
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
        see_through_points: false,
    };
    let snap = scene.resolve(&q).expect("the corner is dead-on");
    assert_eq!(snap.kind, SnapKind::Endpoint);
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
    // drifted 0.04 along world -X (the frame's green axis): same geometry
    // as the world-identity fixture, just reoriented.
    let target = anchor + frame.x * 5.0 + frame.y * 0.04;
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
        see_through_points: false,
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
                    see_through_points: false,
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
                    see_through_points: false,
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
                    see_through_points: false,
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
            see_through_points: false,
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
            span: kernel::GuideSpan::INFINITE,
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
            see_through_points: false,
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
                see_through_points: false,
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
            span: kernel::GuideSpan::INFINITE,
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
                see_through_points: false,
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
            span: kernel::GuideSpan::INFINITE,
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
            see_through_points: false,
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
                see_through_points: false,
            })
            .expect("a lock with an anchor always resolves");
        // The blue drawing axis through the origin IS the lock line here and
        // crosses the edge at exactly this station, so the axis × edge
        // crossing (an `Intersection`, a point kind) may name the same point
        // — still the hovered edge, at its true station, never the anchor.
        assert!(
            matches!(snap.kind, SnapKind::OnEdge | SnapKind::Intersection),
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
            see_through_points: false,
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
            see_through_points: false,
        })
        .expect("the visible top face keeps a candidate in the cone");
    assert!(
        snap.position.z > 0.5,
        "occlusion must still hide the bottom corner from an off-plane-points query: {:?}",
        snap.position
    );
}

// ── Item 7: a locked Line chain hovering the cube's edges ──────────────────

/// The ISO-view eye used by the item-7 playtest repro: a unit cube, the
/// Line tool anchored on the top face's west-edge midpoint and locked to
/// red, the cursor hovering points of the top face's SOUTH edge.
fn item7_query(target: Point3) -> SnapQuery {
    let eye = Point3::new(-2.2, -2.6, 2.9);
    SnapQuery {
        weights: SnapWeights::default(),
        ray: ray_at(eye, target),
        anchor: Some(Point3::new(0.0, 0.5, 1.0)),
        lock: Some(SnapLock::Axis(inference::Axis::X)),
        aperture: 0.01,
        aperture_mode: ApertureMode::Cone,
        constraint_plane: Some(
            Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap(),
        ),
        soft_axis_aperture_scale: None,
        off_plane_points: false,
        see_through_points: false,
    }
}

#[test]
fn item7_locked_line_hovering_the_south_edge_projects_each_edge_point_onto_the_lock() {
    let scene = cube_scene();
    // 0.1 and 0.9 sit within `POINT_SNAP_REACH` of the edge's corners in
    // this view; the corner case is pinned below.
    for i in 2..9 {
        let x = i as f64 / 10.0;
        let snap = scene
            .resolve(&item7_query(Point3::new(x, 0.0, 1.0)))
            .expect("locked resolve always answers");
        assert!(
            snap.kind != SnapKind::OnAxis,
            "x={x}: the south edge under the cursor must snap (got the bare lock fallback at {:?})",
            snap.position
        );
        assert!(
            (snap.position.x - x).abs() < 1e-6 && (snap.position.y - 0.5).abs() < 1e-9,
            "x={x}: the edge point projects perpendicularly onto the lock line (got {:?})",
            snap.position
        );
    }
}

/// Hovering the south edge 0.1 short of its far corner — about 1.5 pick
/// radii away on screen, past the plain aperture but within
/// `POINT_SNAP_REACH` — captures the corner, projected onto the lock.
#[test]
fn item7_locked_line_near_the_south_corner_captures_the_corner() {
    let scene = cube_scene();
    let snap = scene
        .resolve(&item7_query(Point3::new(0.9, 0.0, 1.0)))
        .expect("locked resolve always answers");
    assert_eq!(snap.kind, SnapKind::Endpoint, "got {snap:?}");
    assert!((snap.position.x - 1.0).abs() < 1e-9 && (snap.position.y - 0.5).abs() < 1e-9);
}

#[test]
fn item7_locked_line_hovering_the_north_midpoint_reports_the_midpoint() {
    let scene = cube_scene();
    let snap = scene
        .resolve(&item7_query(Point3::new(0.5, 1.0, 1.0)))
        .expect("locked resolve always answers");
    assert_eq!(snap.kind, SnapKind::Midpoint, "got {:?}", snap);
    assert!((snap.position.x - 0.5).abs() < 1e-9 && (snap.position.y - 0.5).abs() < 1e-9);
    // The tie the UI draws: from the real midpoint to its projection.
    let from = snap
        .projected_from
        .expect("a projected snap remembers where it came from");
    assert!(
        (from.x - 0.5).abs() < 1e-9 && (from.y - 1.0).abs() < 1e-9 && (from.z - 1.0).abs() < 1e-9
    );
}

#[test]
fn an_unprojected_snap_has_no_projected_from() {
    let scene = cube_scene();
    let snap = scene
        .resolve(&query(
            ray_at(Point3::new(0.5, 1.0, 3.0), Point3::new(0.5, 1.0, 1.0)),
            NARROW,
        ))
        .expect("the north midpoint is under the ray");
    assert_eq!(snap.projected_from, None);
}

fn boxed(min: Point3, max: Point3) -> Object {
    Object::from_polygons(
        &[
            Point3::new(min.x, min.y, min.z),
            Point3::new(max.x, min.y, min.z),
            Point3::new(max.x, max.y, min.z),
            Point3::new(min.x, max.y, min.z),
            Point3::new(min.x, min.y, max.z),
            Point3::new(max.x, min.y, max.z),
            Point3::new(max.x, max.y, max.z),
            Point3::new(min.x, max.y, max.z),
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

/// A framed wall's corner stack seen from a "Top" view that is — as every
/// orbit-camera top view is — a hair off vertical: the wall-top corner, the
/// slab's corners below it, all within a pixel of one another on screen
/// but millimetres apart in the tilted ray's frame. The one in FRONT wins
/// (`RANK_TIE_FRACTION` is a pixel wide, not a millionth of one), whichever
/// way the tilt leans. Ray, aperture and geometry are the app's own from
/// the plan-view Dimension repro (parallel projection, cylinder aperture).
#[test]
fn stacked_corner_in_a_slightly_tilted_top_view_prefers_the_nearest() {
    let mut scene = InferenceScene::new();
    // The slab (its corner a hair inside the wall's, as the model has it).
    scene.add_object(
        ObjectId::default(),
        &boxed(
            Point3::new(3.0e-11, -13.3985, -0.1016),
            Point3::new(8.92175, -0.00635, 0.0),
        ),
        &Transform::IDENTITY,
    );
    let wall = ObjectId::from(slotmap::KeyData::from_ffi(2));
    scene.add_object(
        wall,
        &boxed(
            Point3::new(0.0, -13.3985, 0.0),
            Point3::new(0.1905, 0.0, 2.4384),
        ),
        &Transform::IDENTITY,
    );
    // The app's ray: a pixel outside the corner, tilted 0.001 rad toward
    // +y — so the deeper a stacked point, the nearer the ray passes it.
    let ray = PickRay {
        origin: Point3::new(-0.045, -13.481, 39.98),
        direction: Vec3::new(0.0, 0.0009999995, -0.9999995),
    };
    let q = SnapQuery {
        aperture_mode: ApertureMode::Cylinder,
        weights: SnapWeights::default(),
        ray,
        anchor: None,
        lock: None,
        aperture: 0.36, // 8 px at 45 mm/px
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
        see_through_points: false,
    };
    let snap = scene.resolve(&q).expect("a corner is within reach");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(
        (snap.position.z - 2.4384).abs() < 1e-9,
        "the wall-top corner, not the slab's: {:?}",
        snap.position
    );
}

/// The rank tie is a pixel of the ACQUIRE aperture even under the app's
/// widened hysteresis-release query (`soft_axis_aperture_scale` set to the
/// widening ratio), so re-querying at a wider reach never reorders two
/// candidates the normal query already admitted: a held snap the cursor is
/// still nearest to keeps winning, and the release query answers only the
/// question it was asked — is that target still within reach.
#[test]
fn a_widened_release_query_ranks_with_the_acquire_tie_band() {
    let mut scene = InferenceScene::new();
    // The ray runs straight down the blue axis; hide the axes so the frame
    // origin's own Endpoint candidate is not the (trivial) winner.
    scene.set_axes_enabled(false);
    // Two endpoint-class points: P_near (0.021 off the ray, deep) and P_far
    // (0.026 off the ray, nearer the eye). At an 0.08 cylinder aperture the
    // one the cursor is nearest to wins — one-pixel buckets 2 vs 3.
    scene.add_guide(
        GuideId::default(),
        &Guide::Point {
            position: Point3::new(0.021, 0.0, 8.0),
        },
    );
    scene.add_guide(
        GuideId::from(slotmap::KeyData::from_ffi(2)),
        &Guide::Point {
            position: Point3::new(0.026, 0.0, 3.0),
        },
    );
    let ray = PickRay {
        origin: Point3::new(0.0, 0.0, 10.0),
        direction: Vec3::new(0.0, 0.0, -1.0),
    };
    let base = SnapQuery {
        aperture_mode: ApertureMode::Cylinder,
        weights: SnapWeights::default(),
        ray,
        anchor: None,
        lock: None,
        aperture: 0.08,
        constraint_plane: None,
        soft_axis_aperture_scale: None,
        off_plane_points: false,
        see_through_points: false,
    };
    let acquire = scene.resolve(&base).expect("both inside the aperture");
    assert!(
        (acquire.position.x - 0.021).abs() < 1e-12,
        "{:?}",
        acquire.position
    );
    // The release query: twice the aperture AND the widening ratio declared.
    let release = scene
        .resolve(&SnapQuery {
            aperture: 0.16,
            soft_axis_aperture_scale: Some(2.0),
            ..base
        })
        .expect("both still inside");
    assert!(
        (release.position.x - 0.021).abs() < 1e-12,
        "the widened query must not reorder already-admitted candidates: {:?}",
        release.position
    );
}

/// A sketch's registration is bookkept per kind of candidate: registering
/// it again replaces exactly its own entries, removing it drops them and
/// nothing else, and a sketch never registered costs nothing to remove (a
/// document of tens of thousands of small sketches registers each once at
/// open, which used to walk every vector for every one of them).
#[test]
fn sketch_registration_replaces_and_removes_only_its_own_entries() {
    let sid = |n: u64| SketchId::from(slotmap::KeyData::from_ffi(n));
    let seg = |x: f64| {
        (
            SketchEdgeId::default(),
            Point3::new(x, 50.0, 0.0),
            Point3::new(x + 1.0, 50.0, 0.0),
        )
    };
    let at = |x: f64| Point3::new(x, 50.0, 0.0);
    let probe = |scene: &InferenceScene, target: Point3| -> Option<Snap> {
        scene.resolve(&SnapQuery {
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
            see_through_points: false,
        })
    };
    let mut scene = InferenceScene::new();
    scene.remove_sketch(sid(1)); // never registered: nothing to do
    scene.add_sketch(sid(1), &[seg(10.0)]);
    scene.add_sketch(sid(2), &[seg(20.0)]);
    assert_eq!(
        probe(&scene, at(10.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    assert_eq!(
        probe(&scene, at(20.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );

    // Registered again elsewhere: the old entries are gone, the other
    // sketch's stay.
    scene.add_sketch(sid(1), &[seg(30.0)]);
    assert!(probe(&scene, at(10.0)).is_none());
    assert_eq!(
        probe(&scene, at(30.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    assert_eq!(
        probe(&scene, at(20.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );

    // Faces are their own kind: added, replaced, and dropped with the
    // sketch.
    let square = |x: f64| inference::SketchRegionFace {
        region: kernel::SketchRegionId::default(),
        plane: Plane::from_polygon(&[at(x), at(x + 1.0), Point3::new(x, 51.0, 0.0)]).unwrap(),
        boundary: vec![
            at(x),
            at(x + 1.0),
            Point3::new(x + 1.0, 51.0, 0.0),
            Point3::new(x, 51.0, 0.0),
        ],
        holes: vec![],
    };
    let mid = |x: f64| Point3::new(x + 0.5, 50.5, 0.0);
    scene.add_sketch_faces(sid(1), &[square(40.0)]);
    assert_eq!(
        probe(&scene, mid(40.0)).map(|s| s.kind),
        Some(SnapKind::OnFace)
    );
    scene.add_sketch_faces(sid(1), &[square(42.0)]);
    assert!(probe(&scene, mid(40.0)).is_none());
    assert_eq!(
        probe(&scene, mid(42.0)).map(|s| s.kind),
        Some(SnapKind::OnFace)
    );

    scene.remove_sketch(sid(1));
    assert!(probe(&scene, at(30.0)).is_none());
    assert!(probe(&scene, mid(42.0)).is_none());
    assert_eq!(
        probe(&scene, at(20.0)).map(|s| s.kind),
        Some(SnapKind::Endpoint)
    );
    scene.remove_sketch(sid(1)); // twice is fine
    scene.remove_sketch(sid(2));
    assert!(probe(&scene, at(20.0)).is_none());
}

/// A guide segment or ray snaps only where it exists: hovering its line
/// past an end finds no guide there, hovering inside the stretch does,
/// and two guides cross only where both exist.
#[test]
fn a_guide_snaps_and_crosses_only_over_its_stretch() {
    let x = Vec3::new(1.0, 0.0, 0.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let mut scene = InferenceScene::new();
    scene.set_axes_enabled(false);
    // A segment along x from x = 2 to x = 4, at y = 5.
    let gid = |n: u64| GuideId::from(slotmap::KeyData::from_ffi(n));
    scene.add_guide(
        gid(1),
        &Guide::Line {
            origin: Point3::new(0.0, 5.0, 0.0),
            direction: x,
            span: kernel::GuideSpan {
                from: Some(2.0),
                to: Some(4.0),
            },
        },
    );
    let look_at = |p: Point3| {
        resolve(
            &scene,
            query(ray_at(p + Vec3::new(0.0, 0.0, 10.0), p), NARROW),
        )
    };
    let inside = look_at(Point3::new(3.0, 5.0, 0.0)).expect("on the segment");
    assert_eq!(inside.kind, SnapKind::OnGuide);
    assert!(
        look_at(Point3::new(7.0, 5.0, 0.0)).is_none(),
        "past the segment's end there is no guide"
    );
    assert!(
        look_at(Point3::new(-3.0, 5.0, 0.0)).is_none(),
        "before its start neither"
    );

    // A ray along y from y = 0 crossing the segment's line at (3, 5) and at
    // (6, 5): the first crossing is on both, the second is past the
    // segment.
    let mut crossing = scene.clone();
    crossing.add_guide(
        gid(2),
        &Guide::Line {
            origin: Point3::new(3.0, 0.0, 0.0),
            direction: y,
            span: kernel::GuideSpan {
                from: Some(0.0),
                to: None,
            },
        },
    );
    crossing.add_guide(
        gid(3),
        &Guide::Line {
            origin: Point3::new(6.0, 0.0, 0.0),
            direction: y,
            span: kernel::GuideSpan {
                from: Some(0.0),
                to: None,
            },
        },
    );
    let at = |p: Point3| {
        resolve(
            &crossing,
            query(ray_at(p + Vec3::new(0.0, 0.0, 10.0), p), NARROW),
        )
    };
    let hit = at(Point3::new(3.0, 5.0, 0.0)).expect("the crossing on both");
    assert_eq!(hit.kind, SnapKind::Intersection);
    let past = at(Point3::new(6.0, 5.0, 0.0)).expect("the ray is there");
    assert_eq!(
        past.kind,
        SnapKind::OnGuide,
        "no crossing past the segment's end, only the ray itself"
    );
    assert!(
        at(Point3::new(3.0, -2.0, 0.0)).is_none(),
        "the ray does not run back behind its start"
    );
}

/// A definition member registered deferred
/// ([`InferenceScene::set_def_member_deferred`]) is extracted by the first
/// query that reaches one of its placements, once, through the query's
/// source, and then snaps exactly as an eagerly registered member does. A
/// query aimed elsewhere extracts nothing, and one given no source offers
/// nothing of the member.
#[test]
fn a_deferred_member_is_extracted_by_the_first_query_that_reaches_it() {
    let cube = unit_cube();
    let mut eager = InferenceScene::new();
    eager.set_def_member(ObjectId::default(), &cube);
    eager.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::IDENTITY,
    );
    let mut deferred = InferenceScene::new();
    deferred.set_def_member_deferred(ObjectId::default(), &cube);
    deferred.add_placement(
        InstanceId::default(),
        ObjectId::default(),
        &Transform::IDENTITY,
    );
    assert_eq!(
        deferred.def_extractions(),
        0,
        "registration extracts nothing"
    );

    let eye = Point3::new(3.0, 3.0, 3.0);
    let at_corner = || query(ray_at(eye, Point3::new(0.98, 1.0, 1.01)), WIDE);
    // Aimed along -x from the eye: never within the cube's placement box.
    let away = || query(ray_at(eye, Point3::new(-40.0, 3.0, 3.0)), 0.05);
    let source = |_: ObjectId| Some(&cube);

    deferred.resolve_from(&away(), &source);
    assert_eq!(
        deferred.def_extractions(),
        0,
        "a query that reaches no placement of the member extracts nothing"
    );
    assert_eq!(
        deferred.resolve(&at_corner()),
        InferenceScene::new().resolve(&at_corner()),
        "without a source the member offers nothing"
    );
    assert_eq!(deferred.def_extractions(), 0);

    let snap = deferred.resolve_from(&at_corner(), &source);
    assert_eq!(snap.as_ref().map(|s| s.kind), Some(SnapKind::Endpoint));
    assert_eq!(
        snap,
        eager.resolve(&at_corner()),
        "a deferred member snaps as an eager one"
    );
    assert_eq!(
        deferred.def_extractions(),
        1,
        "extracted by the first query that reaches it"
    );
    deferred.resolve_from(&at_corner(), &source);
    assert_eq!(deferred.def_extractions(), 1, "extracted once");
    assert_eq!(deferred.candidate_counts(), eager.candidate_counts());

    let face_ray = || ray_at(eye, Point3::new(0.5, 0.5, 1.0));
    assert_eq!(
        deferred.pick_face_from(&face_ray(), &source),
        eager.pick_face(&face_ray())
    );
    assert_eq!(
        deferred.resolve_from(&at_corner(), &source),
        deferred.resolve_linear(&at_corner()),
        "the linear reference agrees once the member is extracted"
    );
}

/// A deferred member that only stands in the way is extracted too: the
/// occlusion test reaches it through the same source, so a snap behind a
/// deferred component resolves exactly as it does behind an eager one.
#[test]
fn a_deferred_member_in_the_way_is_extracted_for_occlusion() {
    use slotmap::KeyData;
    let cube = unit_cube();
    let a = ObjectId::from(KeyData::from_ffi(1 << 32 | 1));
    let b = ObjectId::from(KeyData::from_ffi(1 << 32 | 2));
    let ia = InstanceId::from(KeyData::from_ffi(1 << 32 | 1));
    let ib = InstanceId::from(KeyData::from_ffi(1 << 32 | 2));
    // B sits between the eye and A's corner, its own corners off the
    // narrow cone, so only its faces (the occlusion test) can reach it.
    let in_the_way = Transform::translation(Vec3::new(1.6, 1.4, 1.5));
    let eye = Point3::new(3.0, 3.0, 3.0);
    let q = || query(ray_at(eye, Point3::new(1.0, 1.0, 1.0)), 0.01);

    let mut alone = InferenceScene::new();
    alone.set_def_member(a, &cube);
    alone.add_placement(ia, a, &Transform::IDENTITY);
    let mut eager = InferenceScene::new();
    eager.set_def_member(a, &cube);
    eager.add_placement(ia, a, &Transform::IDENTITY);
    eager.set_def_member(b, &cube);
    eager.add_placement(ib, b, &in_the_way);
    let mut deferred = InferenceScene::new();
    deferred.set_def_member_deferred(a, &cube);
    deferred.add_placement(ia, a, &Transform::IDENTITY);
    deferred.set_def_member_deferred(b, &cube);
    deferred.add_placement(ib, b, &in_the_way);

    assert_ne!(
        eager.resolve(&q()),
        alone.resolve(&q()),
        "B changes the answer, so the spec exercises it"
    );
    let source = |_: ObjectId| Some(&cube);
    assert_eq!(deferred.resolve_from(&q(), &source), eager.resolve(&q()));
    assert_eq!(
        deferred.def_extractions(),
        2,
        "A for its candidates and B for standing in the way"
    );
    assert_eq!(
        deferred.resolve_from(&q(), &source),
        deferred.resolve_linear(&q()),
        "the linear reference agrees once both are extracted"
    );
}

// ---------------------------------------------------------------------------
// Edge × edge crossings between different owners (#51b).
//
// Separate Objects never merge, so where an edge of one passes through an
// edge of another there is no vertex to snap to. The crossing is offered as
// `Intersection` — only a true 3D crossing (within `tol::POINT_MERGE`),
// only between different owners, never at an endpoint of either edge — with
// both edges as provenance: the first in `source`/`sketch_source`, the other
// in `crossed_edge`.
// ---------------------------------------------------------------------------

/// An axis-aligned box Object spanning `lo`..`hi`.
fn box_object(lo: Point3, hi: Point3) -> Object {
    let (x0, y0, z0, x1, y1, z1) = (lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
    Object::from_polygons(
        &[
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
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

fn oid(n: u64) -> ObjectId {
    ObjectId::from(slotmap::KeyData::from_ffi(1 << 32 | n))
}

fn sid(n: u64) -> SketchId {
    SketchId::from(slotmap::KeyData::from_ffi(1 << 32 | n))
}

fn eid(n: u64) -> SketchEdgeId {
    SketchEdgeId::from(slotmap::KeyData::from_ffi(1 << 32 | n))
}

/// Box A: its top-front edge runs (0,0,1)–(2,0,1).
fn crossing_box_a() -> Object {
    box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0))
}

/// Box B, in front of A: its back-left vertical edge (0.7,0,0)–(0.7,0,2.6)
/// passes through A's top-front edge at [`CROSSING`] — 0.3 from either
/// edge's midpoint, and far from every corner.
fn crossing_box_b() -> Object {
    box_object(Point3::new(0.7, -1.0, 0.0), Point3::new(1.5, 0.0, 2.6))
}

const CROSSING: Point3 = Point3::new(0.7, 0.0, 1.0);

/// Looking at [`CROSSING`] from the left, in front and above: nothing
/// stands between the eye and the crossing.
fn crossing_query(aim: Point3) -> SnapQuery {
    query(ray_at(Point3::new(-2.0, -0.5, 3.0), aim), NARROW)
}

fn assert_at(snap: &Snap, p: Point3) {
    assert!(snap.position.approx_eq(p, tol::POINT_MERGE), "{snap:?}");
}

#[test]
fn edges_of_two_objects_crossing_snap_as_an_intersection() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &crossing_box_a(), &Transform::IDENTITY);
    scene.add_object(oid(2), &crossing_box_b(), &Transform::IDENTITY);
    // A hair off the crossing, still well inside the pick cone.
    let q = crossing_query(Point3::new(0.71, 0.0, 1.01));
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
    let first = snap.source.expect("the first edge's provenance");
    assert!(matches!(first.element, ElementRef::Edge(_)));
    let Some(CrossedEdge::Object(second)) = snap.crossed_edge else {
        panic!("the second edge's provenance: {snap:?}");
    };
    assert!(matches!(second.element, ElementRef::Edge(_)));
    let mut owners = [first.object, second.object];
    owners.sort();
    assert_eq!(owners, [oid(1), oid(2)], "one edge from each Object");
    assert_eq!(snap.direction, None, "an edge crossing names no direction");
    assert_eq!(
        scene.resolve(&q),
        scene.resolve_linear(&q),
        "the indexed and linear walks agree"
    );
}

/// The reporter's case (#51): the top-back edge of a low box crosses the
/// front-left vertical edge of a taller box behind it at
/// (0.5001, 0.0714, 0.1826). The coordinates are exact here, so it is a
/// true crossing. The low box's edge midpoint sits only 0.1 mm away at
/// x = 0.5, well inside a normal-zoom pick cone, and the crossing still
/// wins: a crossing outranks a midpoint (midpoints are on every edge,
/// crossings are rare).
#[test]
fn the_reported_two_box_crossing_is_a_true_crossing() {
    let mut scene = InferenceScene::new();
    let low = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0714, 0.1826));
    let tall = box_object(Point3::new(0.5001, 0.0714, 0.0), Point3::new(1.2, 0.8, 0.5));
    scene.add_object(oid(9), &low, &Transform::IDENTITY);
    scene.add_object(oid(7), &tall, &Transform::IDENTITY);
    let crossing = Point3::new(0.5001, 0.0714, 0.1826);
    let eye = Point3::new(0.2, -1.0, 0.8);
    // Zoomed far in: only the crossing is in the cone.
    let snap = resolve(&scene, query(ray_at(eye, crossing), 2e-5)).expect("dead on the crossing");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, crossing);
    assert!(snap.crossed_edge.is_some());
    // Normal zoom: an 8 px radius in a 45° view about 1.2 m away is some
    // 6 mrad, and the midpoint 0.1 mm off is ~0.08 mrad from the ray.
    let normal = query(ray_at(eye, crossing), 6e-3);
    let mut only_low = InferenceScene::new();
    only_low.add_object(oid(9), &low, &Transform::IDENTITY);
    let alone = resolve(&only_low, normal).expect("the midpoint is in the cone");
    assert_eq!(
        alone.kind,
        SnapKind::Midpoint,
        "the midpoint is in reach: {alone:?}"
    );
    let snap = resolve(&scene, normal).expect("the crossing is under the cursor");
    assert_eq!(
        snap.kind,
        SnapKind::Intersection,
        "the crossing beats the midpoint: {snap:?}"
    );
    assert_at(&snap, crossing);
    // A few pixels off the crossing, still nearer the midpoint than the
    // crossing is, the crossing holds.
    let off = query(ray_at(eye, Point3::new(0.4999, 0.0714, 0.1828)), 6e-3);
    let snap = resolve(&scene, off).expect("near the crossing");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    // A crossing is a named point: it reaches as far as a midpoint does
    // (`POINT_SNAP_REACH` apertures), so a cursor 1.4 apertures off it
    // (about 11 px of an 8 px radius) still takes the crossing, not the
    // midpoint beside it.
    let to = crossing - eye;
    let dist = to.length();
    let side = to.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().unwrap();
    let aim = crossing + side * (dist * (1.4 * 6e-3_f64).tan());
    let past_aperture = query(ray_at(eye, aim), 6e-3);
    let snap = resolve(&scene, past_aperture).expect("within a point's reach");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, crossing);
}

#[test]
fn edges_that_nearly_miss_do_not_snap_as_an_intersection() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &crossing_box_a(), &Transform::IDENTITY);
    // B pulled forward by a micrometre: its edge passes 1e-6 in front of
    // A's, a thousand times the scene tolerance.
    let b = box_object(Point3::new(0.7, -1.0, 0.0), Point3::new(1.5, -1e-6, 2.6));
    scene.add_object(oid(2), &b, &Transform::IDENTITY);
    let snap = scene
        .resolve(&crossing_query(Point3::new(0.71, 0.0, 1.01)))
        .expect("both edges are under the cursor");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_eq!(snap.crossed_edge, None);
}

/// An edge that ends ON another Object's edge (a T) already has its own
/// Endpoint snap there; the meeting is not offered again as a crossing.
#[test]
fn edges_meeting_at_an_endpoint_keep_the_endpoint_snap() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &crossing_box_a(), &Transform::IDENTITY);
    // C stands on A's top-front edge: its back-left vertical edge starts at
    // (0.7, 0, 1), inside A's edge.
    let c = box_object(Point3::new(0.7, -1.0, 1.0), Point3::new(1.5, 0.0, 2.6));
    scene.add_object(oid(2), &c, &Transform::IDENTITY);
    let snap = scene
        .resolve(&crossing_query(Point3::new(0.71, 0.0, 1.01)))
        .expect("the corner is under the cursor");
    assert_eq!(snap.kind, SnapKind::Endpoint, "{snap:?}");
    assert_at(&snap, CROSSING);
    assert!(matches!(
        snap.source.map(|s| s.element),
        Some(ElementRef::Vertex(_))
    ));
    assert_eq!(snap.crossed_edge, None);
}

/// Two crossing edges of ONE Object are not offered: the kernel already
/// splits edges within an Object, so a crossing there is either a vertex
/// or geometry the Object's owner must resolve, not a snap. Built from two
/// open quads whose edges cross; the same two quads as separate Objects
/// do snap, so the spec exercises the owner rule and nothing else.
#[test]
fn edges_of_one_object_do_not_snap_as_a_crossing() {
    let p = [
        // Quad 1 in the plane y = 0: top edge (0,0,1)–(2,0,1).
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 1.0),
        Point3::new(0.0, 0.0, 1.0),
        // Quad 2 in the plane x = 0.7: edge (0.7,0,-0.9)–(0.7,0,2.6).
        Point3::new(0.7, -1.0, -0.9),
        Point3::new(0.7, 0.0, -0.9),
        Point3::new(0.7, 0.0, 2.6),
        Point3::new(0.7, -1.0, 2.6),
    ];
    let quad1 = vec![0, 1, 2, 3];
    let quad2 = vec![4, 5, 6, 7];
    let one = Object::from_polygons(&p, &[quad1.clone(), quad2.clone()]).unwrap();
    let only1 = Object::from_polygons(&p[..4], &[quad1]).unwrap();
    let only2 = Object::from_polygons(&p[4..], &[vec![0, 1, 2, 3]]).unwrap();
    let q = crossing_query(Point3::new(0.71, 0.0, 1.01));

    let mut together = InferenceScene::new();
    together.add_object(oid(1), &one, &Transform::IDENTITY);
    let snap = together
        .resolve(&q)
        .expect("both edges are under the cursor");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");

    let mut apart = InferenceScene::new();
    apart.add_object(oid(1), &only1, &Transform::IDENTITY);
    apart.add_object(oid(2), &only2, &Transform::IDENTITY);
    let snap = apart.resolve(&q).expect("both edges are under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
}

#[test]
fn an_object_edge_crossing_a_placed_instance_edge_snaps() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &crossing_box_a(), &Transform::IDENTITY);
    // B as a component member placed by an instance, offset so the
    // placement transform is exercised.
    let b_at_origin = box_object(Point3::new(0.0, -1.0, 0.0), Point3::new(0.8, 0.0, 2.6));
    scene.set_def_member(oid(2), &b_at_origin);
    let instance = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 5));
    scene.add_placement(
        instance,
        oid(2),
        &Transform::translation(Vec3::new(0.7, 0.0, 0.0)),
    );
    let q = crossing_query(Point3::new(0.71, 0.0, 1.01));
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
    let sources = [
        snap.source,
        snap.crossed_edge.and_then(|c| match c {
            CrossedEdge::Object(s) => Some(s),
            CrossedEdge::Sketch(..) => None,
        }),
    ];
    assert!(
        sources
            .iter()
            .any(|s| s.and_then(|s| s.instance) == Some(instance)),
        "one edge is the instance's: {snap:?}"
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

#[test]
fn a_sketch_edge_crossing_an_object_edge_snaps() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &crossing_box_a(), &Transform::IDENTITY);
    // A drawn line in the plane y = 0 crossing A's top-front edge.
    scene.add_sketch(
        sid(1),
        &[(
            eid(1),
            Point3::new(0.7, 0.0, 0.2),
            Point3::new(0.7, 0.0, 2.6),
        )],
    );
    let snap = scene
        .resolve(&crossing_query(Point3::new(0.71, 0.0, 1.01)))
        .expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
    // Either edge may come first; together they are the two edges.
    let first = match (snap.source, snap.sketch_source) {
        (Some(s), None) => CrossedEdge::Object(s),
        (None, Some((s, e))) => CrossedEdge::Sketch(s, e),
        other => panic!("exactly one first edge: {other:?}"),
    };
    let second = snap.crossed_edge.expect("the second edge");
    let sketch = CrossedEdge::Sketch(sid(1), eid(1));
    assert!(
        (first == sketch && matches!(second, CrossedEdge::Object(o) if o.object == oid(1)))
            || (second == sketch && matches!(first, CrossedEdge::Object(o) if o.object == oid(1))),
        "one Object edge and the sketch edge: {snap:?}"
    );
}

#[test]
fn edges_of_two_sketches_cross_but_one_sketch_does_not() {
    // In the plane z = 0.5, clear of the drawing axes (an axis crossing
    // would snap there too).
    let a = (Point3::new(-1.0, 0.3, 0.5), Point3::new(3.0, 0.3, 0.5));
    let b = (Point3::new(0.7, -2.0, 0.5), Point3::new(0.7, 1.0, 0.5));
    let crossing = Point3::new(0.7, 0.3, 0.5);
    let q = query(
        ray_at(Point3::new(0.2, -1.0, 3.0), Point3::new(0.71, 0.31, 0.5)),
        NARROW,
    );

    let mut two = InferenceScene::new();
    two.add_sketch(sid(1), &[(eid(1), a.0, a.1)]);
    two.add_sketch(sid(2), &[(eid(2), b.0, b.1)]);
    let snap = two.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, crossing);
    let mut edges = [
        snap.sketch_source.expect("first sketch edge"),
        match snap.crossed_edge {
            Some(CrossedEdge::Sketch(s, e)) => (s, e),
            other => panic!("second sketch edge: {other:?}"),
        },
    ];
    edges.sort();
    assert_eq!(edges, [(sid(1), eid(1)), (sid(2), eid(2))]);

    let mut one = InferenceScene::new();
    one.add_sketch(sid(1), &[(eid(1), a.0, a.1), (eid(2), b.0, b.1)]);
    let snap = one.resolve(&q).expect("both edges are under the cursor");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");
}

/// A dense spot cannot stall a pointer move: a query tests at most
/// `CROSSING_PAIR_CAP` cross-owner pairs, nearest edges first, and counts
/// the pairs it left untested in `crossing_stats` rather than dropping them
/// silently.
#[test]
fn crossing_pairs_past_the_cap_are_skipped_and_counted() {
    let cap = inference::CROSSING_PAIR_CAP as u64;
    let pairs = |n: u64| n * (n - 1) / 2;
    let center = Point3::new(0.3, 0.2, 0.0);
    // `n` lines in the ground plane, each its own sketch, fanned through
    // `center` (off every line's midpoint and far from its endpoints).
    let fan = |n: u64| {
        let mut scene = InferenceScene::new();
        for i in 0..n {
            let t = std::f64::consts::PI * i as f64 / n as f64;
            let d = Vec3::new(t.cos(), t.sin(), 0.0);
            scene.add_sketch(sid(i + 1), &[(eid(i + 1), center - d * 0.3, center + d)]);
        }
        scene
    };
    let q = query(ray_at(Point3::new(0.4, 0.4, 3.0), center), NARROW);

    // Under the cap: every pair is tested.
    let n = 20;
    let scene = fan(n);
    let snap = scene
        .resolve(&q)
        .expect("the fan's center is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, center);
    let stats = scene.crossing_stats();
    assert_eq!(stats.pairs_tested, pairs(n));
    assert_eq!((stats.queries_capped, stats.pairs_skipped), (0, 0));

    // Past it: exactly `cap` pairs are tested, the rest counted.
    let n = 80;
    assert!(pairs(n) > cap);
    let scene = fan(n);
    let snap = scene
        .resolve(&q)
        .expect("the fan's center is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    let stats = scene.crossing_stats();
    assert_eq!(stats.pairs_tested, cap);
    assert_eq!(stats.queries_capped, 1);
    assert_eq!(stats.pairs_skipped, pairs(n) - cap);
}

/// Under the cap, the edges nearest the cursor pair first: two crossing
/// edges right under it still snap with dozens of other edges in the cone.
/// The distractors are registered BEFORE the crossing pair, so the pair
/// comes last in emission order and only the nearest-first order can reach
/// it inside the cap.
#[test]
fn the_nearest_edges_pair_first_past_the_cap() {
    let center = Point3::new(0.3, 0.2, 0.0);
    let mut scene = InferenceScene::new();
    // Seventy parallel distractors just below the ground plane, each its
    // own sketch: inside the pick cone, farther from the ray than the two
    // below, crossing nothing.
    let n = 70;
    let d = Vec3::new(1.0, -1.0, 0.0) * std::f64::consts::FRAC_1_SQRT_2;
    for i in 0..n {
        let p = Point3::new(0.304, 0.204, -0.002 * (i + 1) as f64);
        scene.add_sketch(sid(i + 1), &[(eid(i + 1), p - d * 0.3, p + d)]);
    }
    // The two edges crossing at `center`, off their midpoints.
    scene.add_sketch(
        sid(n + 1),
        &[(
            eid(n + 1),
            Point3::new(0.0, 0.2, 0.0),
            Point3::new(1.3, 0.2, 0.0),
        )],
    );
    scene.add_sketch(
        sid(n + 2),
        &[(
            eid(n + 2),
            Point3::new(0.3, -0.1, 0.0),
            Point3::new(0.3, 1.2, 0.0),
        )],
    );
    let q = query(ray_at(Point3::new(0.35, 0.3, 3.0), center), NARROW);
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, center);
    let stats = scene.crossing_stats();
    assert_eq!(stats.queries_capped, 1, "the spec must exercise the cap");
    let total = (n + 2) * (n + 1) / 2;
    assert!(
        total - (2 * n + 1) > inference::CROSSING_PAIR_CAP as u64,
        "in emission order the cap ends before the crossing pair"
    );
    assert_eq!(stats.pairs_tested, inference::CROSSING_PAIR_CAP as u64);
    assert_eq!(stats.pairs_skipped, total - stats.pairs_tested);
}

/// A cone full of ONE owner's edges (a dense import seen from afar) costs
/// the pair walk nothing for those edges: they never pair with each other,
/// and the walk steps over them rather than through every pair of them.
/// With two foreign edges among them, only the cross-owner pairs are
/// tested, in a number of steps bounded by them, not by the 45,451 pairs
/// of all 302 edges.
#[test]
fn edges_of_one_owner_cost_the_crossing_walk_nothing() {
    let center = Point3::new(0.3, 0.2, 0.5);
    let n = 300u64;
    // One sketch of `n` lines fanned through `center`, off every line's
    // midpoint and far from its endpoints.
    let fan: Vec<_> = (0..n)
        .map(|i| {
            let t = std::f64::consts::PI * i as f64 / n as f64;
            let d = Vec3::new(t.cos(), t.sin(), 0.0);
            (eid(i + 1), center - d * 0.3, center + d)
        })
        .collect();
    let q = query(ray_at(Point3::new(0.4, 0.4, 3.0), center), NARROW);

    let mut alone = InferenceScene::new();
    alone.add_sketch(sid(1), &fan);
    alone.resolve(&q);
    assert_eq!(
        alone.crossing_stats(),
        inference::CrossingStats::default(),
        "one owner: no pair, no step"
    );

    let mut scene = InferenceScene::new();
    scene.add_sketch(sid(1), &fan);
    for k in 0..2u64 {
        let t = 0.37 + 0.9 * k as f64;
        let d = Vec3::new(t.cos(), t.sin(), 0.0);
        scene.add_sketch(
            sid(10 + k),
            &[(eid(1000 + k), center - d * 0.4, center + d * 0.9)],
        );
    }
    let snap = scene
        .resolve(&q)
        .expect("the fan's center is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, center);
    let stats = scene.crossing_stats();
    let cross_pairs = 2 * n + 1;
    assert_eq!(stats.pairs_tested, cross_pairs);
    assert_eq!((stats.queries_capped, stats.pairs_skipped), (0, 0));
    assert!(
        stats.pair_steps <= 2 * cross_pairs + n + 2,
        "{} steps for {cross_pairs} pairs",
        stats.pair_steps
    );
}

/// Two instances of one component are separate geometry, as they are to
/// the kernel: where an edge of one crosses an edge of the other, that
/// crossing snaps, carrying both instances.
#[test]
fn edges_of_two_instances_of_one_component_cross() {
    // One member: an open quad in the plane y = 0, x 0..2, z 0..1.
    let quad = Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(0.0, 0.0, 1.0),
        ],
        &[vec![0, 1, 2, 3]],
    )
    .unwrap();
    let mut scene = InferenceScene::new();
    scene.set_def_member(oid(2), &quad);
    let first = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 5));
    let second = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 6));
    // The first as drawn: its top edge runs (0,0,1)–(2,0,1).
    scene.add_placement(first, oid(2), &Transform::IDENTITY);
    // The second turned a quarter about Y and moved: its bottom edge
    // becomes the vertical (0.7,0,2.6)–(0.7,0,0.6), crossing the first's
    // top edge at `CROSSING`.
    let turned = Transform::from_affine(&[
        0.0, 0.0, 1.0, 0.7, //
        0.0, 1.0, 0.0, 0.0, //
        -1.0, 0.0, 0.0, 2.6,
    ]);
    scene.add_placement(second, oid(2), &turned);
    let q = crossing_query(Point3::new(0.71, 0.0, 1.01));
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
    let Some(CrossedEdge::Object(other)) = snap.crossed_edge else {
        panic!("the second edge's provenance: {snap:?}");
    };
    let mut instances = [snap.source.and_then(|s| s.instance), other.instance];
    instances.sort();
    let mut want = [Some(first), Some(second)];
    want.sort();
    assert_eq!(instances, want, "one edge from each instance: {snap:?}");
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// Two members of ONE instance are separate Objects inside it: their edges
/// crossing snap, both carrying that instance.
#[test]
fn edges_of_two_members_of_one_instance_cross() {
    let p = [
        // Member 1 in the plane y = 0: top edge (0,0,1)–(2,0,1).
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 1.0),
        Point3::new(0.0, 0.0, 1.0),
        // Member 2 in the plane x = 0.7: edge (0.7,0,-0.9)–(0.7,0,2.6).
        Point3::new(0.7, -1.0, -0.9),
        Point3::new(0.7, 0.0, -0.9),
        Point3::new(0.7, 0.0, 2.6),
        Point3::new(0.7, -1.0, 2.6),
    ];
    let one = Object::from_polygons(&p[..4], &[vec![0, 1, 2, 3]]).unwrap();
    let two = Object::from_polygons(&p[4..], &[vec![0, 1, 2, 3]]).unwrap();
    let mut scene = InferenceScene::new();
    scene.set_def_member(oid(2), &one);
    scene.set_def_member(oid(3), &two);
    let instance = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 5));
    scene.add_placement(instance, oid(2), &Transform::IDENTITY);
    scene.add_placement(instance, oid(3), &Transform::IDENTITY);
    let q = crossing_query(Point3::new(0.71, 0.0, 1.01));
    let snap = scene.resolve(&q).expect("the crossing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, CROSSING);
    let first = snap.source.expect("the first edge");
    let Some(CrossedEdge::Object(second)) = snap.crossed_edge else {
        panic!("the second edge's provenance: {snap:?}");
    };
    assert_eq!(
        (first.instance, second.instance),
        (Some(instance), Some(instance))
    );
    let mut members = [first.object, second.object];
    members.sort();
    assert_eq!(members, [oid(2), oid(3)], "one edge from each member");
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// When a caller weights `Intersection` past `OnEdge`, the crossing pool
/// takes edges at `Intersection`'s reach, not at `OnEdge`'s: two edges
/// passing beyond the OnEdge reach but inside the boosted one still offer
/// their crossing. Unboosted, the same aim snaps nothing.
#[test]
fn a_boosted_intersection_reach_pools_edges_past_the_on_edge_reach() {
    // In the plane z = 0.5, clear of the drawing axes.
    let mut scene = InferenceScene::new();
    scene.add_sketch(
        sid(1),
        &[(
            eid(1),
            Point3::new(-1.0, 0.3, 0.5),
            Point3::new(3.0, 0.3, 0.5),
        )],
    );
    scene.add_sketch(
        sid(2),
        &[(
            eid(2),
            Point3::new(0.7, -2.0, 0.5),
            Point3::new(0.7, 1.0, 0.5),
        )],
    );
    let crossing = Point3::new(0.7, 0.3, 0.5);
    // Straight down, 0.015 m off each edge (0.021 m off the crossing),
    // with a 0.01 m pick radius: both edges lie past the OnEdge reach.
    let aim = Point3::new(0.715, 0.315, 0.5);
    let at = |weights: SnapWeights| SnapQuery {
        weights,
        aperture_mode: ApertureMode::Cylinder,
        ..query(
            PickRay {
                origin: aim + Vec3::new(0.0, 0.0, 3.0),
                direction: Vec3::new(0.0, 0.0, -1.0),
            },
            0.01,
        )
    };
    let plain = at(SnapWeights::default());
    assert_eq!(
        scene.resolve(&plain),
        None,
        "nothing within the plain reach"
    );
    let boosted = at(SnapWeights::default().with(SnapKind::Intersection, 3.0));
    let snap = scene
        .resolve(&boosted)
        .expect("the crossing is within the boosted reach");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, crossing);
    assert!(snap.crossed_edge.is_some());
    assert_eq!(scene.resolve(&boosted), scene.resolve_linear(&boosted));
}

// ---------------------------------------------------------------------------
// Face policy: the picking rule of a whole-model view mode. Shaded faces
// occlude and are targets (`Opaque`); X-ray faces occlude nothing but stay
// targets (`SeeThrough`); wireframe faces neither occlude nor exist for
// input (`Absent`).
// ---------------------------------------------------------------------------

const POLICIES: [FacePolicy; 3] = [
    FacePolicy::Opaque,
    FacePolicy::SeeThrough,
    FacePolicy::Absent,
];

fn cube_scene_with(policy: FacePolicy) -> InferenceScene {
    let mut scene = cube_scene();
    scene.set_face_policy(policy);
    scene
}

#[test]
fn the_face_policy_defaults_to_opaque() {
    assert_eq!(InferenceScene::new().face_policy(), FacePolicy::Opaque);
    assert!(FacePolicy::Opaque.occludes() && FacePolicy::Opaque.faces_are_targets());
    assert!(!FacePolicy::SeeThrough.occludes() && FacePolicy::SeeThrough.faces_are_targets());
    assert!(!FacePolicy::Absent.occludes() && !FacePolicy::Absent.faces_are_targets());
}

/// The back-bottom edge's midpoint (0.5, 1, 0) seen through the front face:
/// hidden in shaded, reachable once nothing occludes.
#[test]
fn a_hidden_midpoint_is_unreachable_when_faces_occlude_and_reachable_otherwise() {
    let eye = Point3::new(0.5, -3.0, 3.0);
    let hidden_mid = Point3::new(0.5, 1.0, 0.0);
    for policy in POLICIES {
        let scene = cube_scene_with(policy);
        let q = query(ray_at(eye, hidden_mid), NARROW);
        let snap = resolve(&scene, q);
        assert_eq!(
            snap,
            scene.resolve_linear(&q),
            "{policy:?}: indexed and linear agree"
        );
        match policy {
            FacePolicy::Opaque => {
                let snap = snap.expect("the front face is under the cursor");
                assert_eq!(
                    snap.kind,
                    SnapKind::OnFace,
                    "shaded: the front face, not the hidden midpoint"
                );
                assert!(
                    (snap.position.y).abs() <= tol::PLANE_DIST,
                    "on the front face: {snap:?}"
                );
            }
            FacePolicy::SeeThrough | FacePolicy::Absent => {
                let snap = snap.expect("the hidden midpoint is in the cone");
                assert_eq!(snap.kind, SnapKind::Midpoint, "{policy:?}: {snap:?}");
                assert!(
                    snap.position.approx_eq(hidden_mid, tol::POINT_MERGE),
                    "{snap:?}"
                );
                assert!(snap.source.is_some(), "the hidden edge's provenance");
            }
        }
    }
}

/// Looking straight down through the top face (z = 1) and the bottom face
/// (z = 0) behind it: the bottom face is never the pick, by snap or by
/// `pick_face`. X-ray picks the front face exactly as shaded does; wireframe
/// picks no face at all.
#[test]
fn a_face_behind_another_is_never_the_pick() {
    let ray = ray_at(Point3::new(0.4, 0.6, 5.0), Point3::new(0.4, 0.6, 0.0));
    for policy in POLICIES {
        let scene = cube_scene_with(policy);
        let q = query(ray, NARROW);
        let snap = resolve(&scene, q);
        assert_eq!(
            snap,
            scene.resolve_linear(&q),
            "{policy:?}: indexed and linear agree"
        );
        let pick = scene.pick_face(&ray);
        assert_eq!(
            pick,
            scene.pick_face_linear(&ray),
            "{policy:?}: indexed and linear agree"
        );
        if policy.faces_are_targets() {
            let snap = snap.expect("the top face is under the cursor");
            assert_eq!(snap.kind, SnapKind::OnFace, "{policy:?}");
            assert!(
                (snap.position.z - 1.0).abs() <= tol::PLANE_DIST,
                "{policy:?}: the top face: {snap:?}"
            );
            let (_, depth) = pick.expect("a face is under the cursor");
            assert!(
                (depth - 4.0).abs() <= 1e-9,
                "{policy:?}: the top face is 4 m away, got {depth}"
            );
        } else {
            assert_eq!(
                snap, None,
                "wireframe: no face to land on, nothing else in the cone"
            );
            assert_eq!(pick, None, "wireframe: faces are not pick targets");
        }
    }
}

/// Wireframe has no `OnFace` at all, a sketch-region fill included, and
/// a sketch region does not occlude either.
#[test]
fn no_face_snaps_at_all_when_faces_are_absent() {
    let mut scene = cube_scene_with(FacePolicy::Absent);
    let corner = |x: f64, y: f64| Point3::new(x, y, 2.0);
    scene.add_sketch_faces(
        sid(1),
        &[inference::SketchRegionFace {
            region: kernel::SketchRegionId::default(),
            plane: Plane::from_polygon(&[corner(-1.0, -1.0), corner(2.0, -1.0), corner(2.0, 2.0)])
                .unwrap(),
            boundary: vec![
                corner(-1.0, -1.0),
                corner(2.0, -1.0),
                corner(2.0, 2.0),
                corner(-1.0, 2.0),
            ],
            holes: vec![],
        }],
    );
    let eye = Point3::new(0.3, -2.0, 4.0);
    for i in 0..=10 {
        for j in 0..=10 {
            let aim = Point3::new(-0.5 + 0.2 * f64::from(i), -0.5 + 0.2 * f64::from(j), 0.5);
            for aperture in [NARROW, WIDE] {
                let q = query(ray_at(eye, aim), aperture);
                let snap = resolve(&scene, q);
                assert_eq!(snap, scene.resolve_linear(&q));
                assert!(
                    snap.is_none_or(|s| s.kind != SnapKind::OnFace),
                    "aim {aim:?} at {aperture}: {snap:?}"
                );
            }
        }
    }
    // The fill does not hide the cube's top corner below it.
    let top = Point3::new(1.0, 1.0, 1.0);
    let snap = resolve(&scene, query(ray_at(eye, top), NARROW)).expect("the corner is in the cone");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!(snap.position.approx_eq(top, tol::POINT_MERGE));
    // X-ray keeps the fill as the target (it is the nearest face), and it
    // still does not hide the corner.
    scene.set_face_policy(FacePolicy::SeeThrough);
    let over_fill = resolve(
        &scene,
        query(ray_at(eye, Point3::new(0.4, 0.4, 2.0)), NARROW),
    )
    .expect("the fill");
    assert_eq!(over_fill.kind, SnapKind::OnFace);
    assert!(over_fill.sketch_region_source.is_some(), "{over_fill:?}");
    let snap = resolve(&scene, query(ray_at(eye, top), NARROW)).expect("the corner is in the cone");
    assert_eq!(snap.kind, SnapKind::Endpoint);
    // Shaded: the fill hides it.
    scene.set_face_policy(FacePolicy::Opaque);
    let snap = resolve(&scene, query(ray_at(eye, top), NARROW)).expect("the fill");
    assert_eq!(snap.kind, SnapKind::OnFace, "{snap:?}");
}

/// The drawing tools hold a hover over a face to that face's plane
/// (`constraint_plane`). Under a non-occluding policy, a query that opts in
/// (`see_through_points`, the tools that drop such a snap onto their plane
/// and say so) is not vetoed from hidden geometry: in X-ray the Line tool
/// snaps to an endpoint inside a solid through its face. In shaded the face
/// plane still filters, and a query that does not opt in gets the plain
/// filter in every mode.
#[test]
fn hidden_geometry_behind_the_constraining_face_is_reachable_only_when_opted_in_and_faces_do_not_occlude()
 {
    let eye = Point3::new(0.3, 0.2, 4.0);
    let top =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let on_top = |aim: Point3, see_through_points: bool| SnapQuery {
        constraint_plane: Some(top),
        see_through_points,
        ..query(ray_at(eye, aim), NARROW)
    };
    let hidden_corner = Point3::new(1.0, 1.0, 0.0);
    let hidden_edge = Point3::new(0.3, 1.0, 0.0);
    for policy in POLICIES {
        let scene = cube_scene_with(policy);
        for opted_in in [false, true] {
            let corner = resolve(&scene, on_top(hidden_corner, opted_in));
            let edge = resolve(&scene, on_top(hidden_edge, opted_in));
            assert_eq!(
                corner,
                scene.resolve_linear(&on_top(hidden_corner, opted_in))
            );
            if !policy.faces_are_targets() && !opted_in {
                // Wireframe, plain filter: no face to land on, and the
                // hidden geometry is off the plane.
                assert_eq!(corner, None, "{corner:?}");
                assert_eq!(edge, None, "{edge:?}");
                continue;
            }
            let (corner, edge) = (corner.expect("a snap"), edge.expect("a snap"));
            if policy.occludes() || !opted_in {
                assert_eq!(
                    corner.kind,
                    SnapKind::OnFace,
                    "{policy:?}, opted in {opted_in}: held to the face: {corner:?}"
                );
                assert_eq!(
                    edge.kind,
                    SnapKind::OnFace,
                    "{policy:?}, opted in {opted_in}: held to the face: {edge:?}"
                );
            } else {
                assert_eq!(corner.kind, SnapKind::Endpoint, "{policy:?}: {corner:?}");
                assert!(corner.position.approx_eq(hidden_corner, tol::POINT_MERGE));
                assert_eq!(edge.kind, SnapKind::OnEdge, "{policy:?}: {edge:?}");
                assert!(
                    (edge.position.y - 1.0).abs() <= 1e-9 && edge.position.z.abs() <= 1e-9,
                    "{edge:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edge × face piercings between different owners.
//
// A panel set into a wall with nothing imprinted on the wall: the panel's
// edge passes through the wall's face, and since separate Objects never
// merge there is no vertex, no edge, nothing at the point where it does.
// Hovering it offered only On Face or On Edge, flickering between the two.
// The piercing is offered as `Intersection` — the edge strictly through the
// face's plane, the crossing point inside the face's outer loop and outside
// its holes, only between different owners, never for an edge that ends on
// the face (an Endpoint) or lies in its plane — with the edge in
// `source`/`sketch_source` and the face in `pierced_face`.
// ---------------------------------------------------------------------------

/// The wall: a thick slab standing in the plane x = 1, its near face x = 1
/// spanning y 0..2, z 0..1.
fn wall() -> Object {
    box_object(Point3::new(1.0, 0.0, 0.0), Point3::new(1.5, 2.0, 1.0))
}

/// The panel: 10 cm thick, 60 cm tall, set 30 cm into the wall. Its
/// top-front edge (0,0.9,0.6)–(1.3,0.9,0.6) passes through the wall's near
/// face at [`PIERCE`], well inside the face and far from every vertex (its
/// own end, 30 cm inside the wall, is outside the pick cone even with the
/// wall gone or see-through).
fn panel() -> Object {
    box_object(Point3::new(0.0, 0.9, 0.0), Point3::new(1.3, 1.0, 0.6))
}

const PIERCE: Point3 = Point3::new(1.0, 0.9, 0.6);

/// In front of the wall, left of the panel and above it: the eye sees the
/// piercing with nothing in between.
const PIERCE_EYE: Point3 = Point3::new(-1.0, 0.3, 1.5);

fn pierce_query(aim: Point3) -> SnapQuery {
    query(ray_at(PIERCE_EYE, aim), NARROW)
}

/// A hair off the piercing, out in the air in front of the wall.
fn near_pierce() -> SnapQuery {
    pierce_query(Point3::new(1.0, 0.895, 0.605))
}

fn wall_and_panel() -> InferenceScene {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    scene.add_object(oid(2), &panel(), &Transform::IDENTITY);
    scene
}

#[test]
fn an_edge_through_another_objects_face_snaps_the_piercing_as_an_intersection() {
    let scene = wall_and_panel();
    let q = near_pierce();
    let snap = scene.resolve(&q).expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
    // The edge is the panel's; the face is the wall's.
    let edge = snap.source.expect("the edge's provenance");
    assert_eq!(edge.object, oid(2));
    assert!(matches!(edge.element, ElementRef::Edge(_)), "{snap:?}");
    let Some(PiercedFace::Object(face)) = snap.pierced_face else {
        panic!("the pierced face's provenance: {snap:?}");
    };
    assert_eq!(face.object, oid(1));
    assert!(matches!(face.element, ElementRef::Face(_)), "{snap:?}");
    assert_eq!(
        snap.crossed_edge, None,
        "an edge × face piercing, not edge × edge"
    );
    assert_eq!(snap.direction, None, "a piercing names no direction");
    assert_eq!(
        scene.resolve(&q),
        scene.resolve_linear(&q),
        "the indexed and linear walks agree"
    );
    let stats = scene.crossing_stats();
    assert!(
        stats.pierce_faces_pooled > 0 && stats.pierce_pairs_tested > 0,
        "{stats:?}"
    );
}

/// The reported flicker: with only the panel the cursor near the point is
/// On Edge, with only the wall it is On Face, and together it used to be
/// one or the other depending on the pixel. Now every hover around the
/// point lands on the piercing, exactly.
#[test]
fn hovering_around_the_piercing_lands_on_it_instead_of_flickering() {
    let mut only_panel = InferenceScene::new();
    only_panel.add_object(oid(2), &panel(), &Transform::IDENTITY);
    let alone = resolve(&only_panel, near_pierce()).expect("the edge is in the cone");
    assert_eq!(alone.kind, SnapKind::OnEdge, "{alone:?}");
    let mut only_wall = InferenceScene::new();
    only_wall.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let alone = resolve(&only_wall, near_pierce()).expect("the face is under the cursor");
    assert_eq!(alone.kind, SnapKind::OnFace, "{alone:?}");

    let scene = wall_and_panel();
    // A ring of cursor positions 5 mm around the point in the wall's plane,
    // plus the point itself: each is a pixel or two off on screen.
    let mut aims = vec![PIERCE];
    for k in 0..8 {
        let t = std::f64::consts::TAU * k as f64 / 8.0;
        aims.push(PIERCE + Vec3::new(0.0, 0.005 * t.cos(), 0.005 * t.sin()));
    }
    for aim in aims {
        let snap = scene
            .resolve(&pierce_query(aim))
            .expect("something is under the cursor");
        assert_eq!(
            snap.kind,
            SnapKind::Intersection,
            "aimed at {aim:?}: the piercing beats On Face and On Edge: {snap:?}"
        );
        assert_at(&snap, PIERCE);
        assert!(snap.pierced_face.is_some(), "{snap:?}");
    }
    // A named point: the piercing reaches as far as an endpoint or midpoint
    // does (`POINT_SNAP_REACH` apertures), so a cursor 1.4 apertures off
    // still takes it over the face it is sitting on.
    let to = PIERCE - PIERCE_EYE;
    let side = to.cross(Vec3::new(0.0, 0.0, 1.0)).normalized().unwrap();
    let aim = PIERCE + side * (to.length() * (1.4 * NARROW).tan());
    let snap = resolve(&scene, pierce_query(aim)).expect("within a point's reach");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
}

/// A vertex at the piercing still wins: Endpoint is the stronger kind.
#[test]
fn a_vertex_at_the_piercing_still_wins() {
    let mut scene = wall_and_panel();
    // A block in front of the wall whose corner is exactly the piercing.
    let block = box_object(Point3::new(0.9, 0.9, 0.6), Point3::new(1.0, 1.0, 0.7));
    scene.add_object(oid(3), &block, &Transform::IDENTITY);
    let snap = scene
        .resolve(&near_pierce())
        .expect("the corner is under the cursor");
    assert_eq!(snap.kind, SnapKind::Endpoint, "{snap:?}");
    assert_at(&snap, PIERCE);
    assert_eq!(snap.source.map(|s| s.object), Some(oid(3)));
    assert_eq!(snap.pierced_face, None);
}

/// An edge that ENDS on the face (a butt joint) is a T meeting: its own
/// Endpoint snap names the point, and no piercing is offered — nor for the
/// panel's end-face edges lying in the wall's plane.
#[test]
fn an_edge_ending_on_the_face_is_an_endpoint_not_a_piercing() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let butted = box_object(Point3::new(0.0, 0.9, 0.0), Point3::new(1.0, 1.0, 0.6));
    scene.add_object(oid(2), &butted, &Transform::IDENTITY);
    let snap = scene
        .resolve(&near_pierce())
        .expect("the corner is under the cursor");
    assert_eq!(snap.kind, SnapKind::Endpoint, "{snap:?}");
    assert_at(&snap, PIERCE);
    assert_eq!(snap.pierced_face, None);
    // Aimed at the butted panel's vertical end edge (x = 1, y = 0.9,
    // z 0..0.6), which lies IN the wall's plane — a third of the way up,
    // clear of its corners and its midpoint: On Edge, since an edge lying
    // in the plane pierces nothing, and the wall's face under it loses to
    // the edge on it.
    let snap = scene
        .resolve(&pierce_query(Point3::new(1.0, 0.9, 0.2)))
        .expect("the end edge is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnEdge, "{snap:?}");
    assert_eq!(snap.source.map(|e| e.object), Some(oid(2)));
    assert_eq!(snap.pierced_face, None);
}

/// A panel stopping short of the wall pierces nothing: its edges are
/// parallel to the ones that would, but end in the air.
#[test]
fn an_edge_stopping_short_of_the_face_pierces_nothing() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let short = box_object(Point3::new(0.0, 0.9, 0.0), Point3::new(0.99, 1.0, 0.6));
    scene.add_object(oid(2), &short, &Transform::IDENTITY);
    let snap = scene
        .resolve(&near_pierce())
        .expect("the wall is under the cursor");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_eq!(snap.pierced_face, None);
}

/// An edge through the face's PLANE beside the face is no piercing: the
/// crossing must land inside the face's outer loop.
#[test]
fn an_edge_through_the_plane_beside_the_face_pierces_nothing() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    // The panel lifted above the wall (which is 1 m tall): its edges cross
    // x = 1 at z = 1.2 and 1.8, past the wall's top.
    let high = box_object(Point3::new(0.0, 0.9, 1.2), Point3::new(1.3, 1.0, 1.8));
    scene.add_object(oid(2), &high, &Transform::IDENTITY);
    let at = Point3::new(1.0, 0.9, 1.8);
    let snap = scene
        .resolve(&pierce_query(at + Vec3::new(0.0, -0.005, 0.005)))
        .expect("the edge is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnEdge, "{snap:?}");
    assert_eq!(snap.pierced_face, None);
}

/// Holes are respected: an edge through a window in a drawn region misses
/// the region; through its material it pierces, carrying the region.
#[test]
fn an_edge_through_a_hole_in_a_face_pierces_nothing() {
    let rid = kernel::SketchRegionId::default();
    // A 2 × 2 drawn region standing in the plane x = 3, with a 0.4 × 0.4
    // window in its middle.
    let plate = inference::SketchRegionFace {
        region: rid,
        plane: Plane::from_point_normal(Point3::new(3.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .unwrap(),
        boundary: vec![
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 2.0, 0.0),
            Point3::new(3.0, 2.0, 2.0),
            Point3::new(3.0, 0.0, 2.0),
        ],
        holes: vec![vec![
            Point3::new(3.0, 0.8, 0.8),
            Point3::new(3.0, 1.2, 0.8),
            Point3::new(3.0, 1.2, 1.2),
            Point3::new(3.0, 0.8, 1.2),
        ]],
    };
    let eye = Point3::new(0.0, -1.0, 3.0);
    let probe = |scene: &InferenceScene, at: Point3| {
        scene.resolve(&query(
            ray_at(eye, at + Vec3::new(0.0, -0.005, 0.005)),
            NARROW,
        ))
    };

    // A rod through the window: its edges cross x = 3 inside the hole.
    let mut through_hole = InferenceScene::new();
    through_hole.add_sketch_faces(sid(1), std::slice::from_ref(&plate));
    let rod = box_object(Point3::new(2.5, 0.95, 0.95), Point3::new(3.5, 1.05, 1.05));
    through_hole.add_object(oid(1), &rod, &Transform::IDENTITY);
    let snap = probe(&through_hole, Point3::new(3.0, 0.95, 1.05)).expect("the rod's edge");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_eq!(snap.pierced_face, None);

    // A rod through the material: a piercing, naming the region.
    let mut through_plate = InferenceScene::new();
    through_plate.add_sketch_faces(sid(1), &[plate]);
    let rod = box_object(Point3::new(2.5, 1.5, 1.5), Point3::new(3.5, 1.6, 1.6));
    through_plate.add_object(oid(1), &rod, &Transform::IDENTITY);
    let at = Point3::new(3.0, 1.5, 1.6);
    let snap = probe(&through_plate, at).expect("the piercing");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, at);
    assert_eq!(
        snap.pierced_face,
        Some(PiercedFace::SketchRegion(sid(1), rid))
    );
    assert_eq!(snap.source.map(|s| s.object), Some(oid(1)));
}

/// An imprinted face: the parent carries the imprint as a hole, the
/// coplanar sub-face fills it. An edge through the imprint pierces the
/// SUB-face, not the parent — the hole test and the face test together.
#[test]
fn an_edge_through_an_imprint_pierces_the_sub_face_not_the_parent() {
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
    let report = cube
        .split_face_inner(
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
    scene.add_object(oid(1), &cube, &Transform::IDENTITY);
    // A post standing through the imprint: its vertical edges cross z = 1
    // at (0.45, 0.45), (0.55, 0.45), … — inside the sub-face.
    let post = box_object(Point3::new(0.45, 0.45, 0.5), Point3::new(0.55, 0.55, 1.5));
    scene.add_object(oid(2), &post, &Transform::IDENTITY);
    let at = Point3::new(0.45, 0.45, 1.0);
    let q = query(
        ray_at(
            Point3::new(0.3, 0.3, 3.0),
            at + Vec3::new(-0.003, -0.003, 0.0),
        ),
        NARROW,
    );
    let snap = scene.resolve(&q).expect("the piercing");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, at);
    let Some(PiercedFace::Object(face)) = snap.pierced_face else {
        panic!("{snap:?}");
    };
    assert_eq!(face.object, oid(1));
    assert_eq!(
        face.element,
        ElementRef::Face(report.sub_face),
        "the sub-face, not the parent {:?}: {snap:?}",
        report.parent
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// A placed component's face is pierced like a world face, and the
/// provenance names the placement.
#[test]
fn an_edge_through_a_placed_instances_face_snaps() {
    let mut scene = InferenceScene::new();
    // The wall as a definition member at the origin, placed 1 m along x.
    let member = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(0.2, 2.0, 1.0));
    scene.set_def_member(oid(1), &member);
    let instance = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 5));
    scene.add_placement(
        instance,
        oid(1),
        &Transform::translation(Vec3::new(1.0, 0.0, 0.0)),
    );
    scene.add_object(oid(2), &panel(), &Transform::IDENTITY);
    let q = near_pierce();
    let snap = scene.resolve(&q).expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
    let Some(PiercedFace::Object(face)) = snap.pierced_face else {
        panic!("{snap:?}");
    };
    assert_eq!((face.object, face.instance), (oid(1), Some(instance)));
    assert_eq!(snap.source.map(|s| s.object), Some(oid(2)));
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    // And the other way round: a placed panel through a world wall.
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let member = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.1, 0.1, 0.6));
    scene.set_def_member(oid(2), &member);
    scene.add_placement(
        instance,
        oid(2),
        &Transform::translation(Vec3::new(0.0, 0.9, 0.0)),
    );
    let snap = scene.resolve(&q).expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
    assert_eq!(
        snap.source.map(|s| (s.object, s.instance)),
        Some((oid(2), Some(instance)))
    );
    assert!(
        matches!(snap.pierced_face, Some(PiercedFace::Object(f)) if f.object == oid(1)),
        "{snap:?}"
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// A drawn line through a solid's face pierces it, with sketch-edge
/// provenance.
#[test]
fn a_sketch_edge_through_an_objects_face_snaps() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    scene.add_sketch(
        sid(1),
        &[(
            eid(1),
            Point3::new(0.5, 0.9, 0.6),
            Point3::new(1.1, 0.9, 0.6),
        )],
    );
    let snap = scene
        .resolve(&near_pierce())
        .expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
    assert_eq!(snap.sketch_source, Some((sid(1), eid(1))));
    assert_eq!(snap.source, None);
    assert!(
        matches!(snap.pierced_face, Some(PiercedFace::Object(f)) if f.object == oid(1)),
        "{snap:?}"
    );
}

/// Faces exist for input only under a policy that has them: in Wireframe
/// nothing pierces; in X-ray the piercing is offered like in Shaded.
#[test]
fn piercings_follow_the_face_policy() {
    let mut scene = wall_and_panel();
    scene.set_face_policy(FacePolicy::Absent);
    let snap = scene
        .resolve(&near_pierce())
        .expect("the edge is still there");
    assert_ne!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_eq!(snap.pierced_face, None);
    scene.set_face_policy(FacePolicy::SeeThrough);
    let snap = scene.resolve(&near_pierce()).expect("the piercing");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, PIERCE);
}

/// A dense spot cannot stall a pointer move: a query tests at most
/// `PIERCE_PAIR_CAP` cross-owner edge × face pairs, nearest first, and
/// counts the pairs it left untested in `crossing_stats` rather than
/// dropping them silently. A scene of one owner pools no face and tests
/// nothing.
#[test]
fn pierce_pairs_past_the_cap_are_skipped_and_counted() {
    let cap = inference::PIERCE_PAIR_CAP as u64;
    // One drawn line slanting down through a stack of `n` drawn squares
    // (each its own sketch) a millimetre apart, seen from straight above
    // with the line's upper end behind the eye and its lower end out of
    // the cone: the edge is in the pool, no endpoint competes.
    let stack = |n: u64| {
        let mut scene = InferenceScene::new();
        for i in 0..n {
            let z = -0.001 * (i + 1) as f64;
            let at = |x: f64, y: f64| Point3::new(x, y, z);
            scene.add_sketch_faces(
                sid(i + 1),
                &[inference::SketchRegionFace {
                    region: kernel::SketchRegionId::default(),
                    plane: Plane::from_polygon(&[at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0)])
                        .unwrap(),
                    boundary: vec![at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
                    holes: vec![],
                }],
            );
        }
        scene.add_sketch(
            sid(10_000),
            &[(
                eid(1),
                Point3::new(0.3, 0.5, 4.0),
                Point3::new(0.65, 0.5, -3.0),
            )],
        );
        scene
    };
    let q = query(
        PickRay {
            origin: Point3::new(0.5, 0.5, 3.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        NARROW,
    );
    // The line crosses z = 0 at x = 0.5 and the first square 1 mm lower.
    let first = Point3::new(0.5 + 0.35 * 0.001 / 7.0, 0.5, -0.001);

    // Under the cap: every pair is tested, the nearest piercing wins.
    let n = 20;
    let scene = stack(n);
    let snap = scene.resolve(&q).expect("the stack is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, first);
    let stats = scene.crossing_stats();
    assert_eq!(stats.pierce_pairs_tested, n);
    assert_eq!(stats.pierce_faces_pooled, n, "every square is in the cone");
    assert_eq!(
        (stats.pierce_queries_capped, stats.pierce_pairs_skipped),
        (0, 0)
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    // Past it: exactly `cap` pairs are tested, the rest counted.
    let n = cap + 100;
    let scene = stack(n);
    let snap = scene.resolve(&q).expect("the stack is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, first);
    let stats = scene.crossing_stats();
    assert_eq!(stats.pierce_pairs_tested, cap);
    assert_eq!(stats.pierce_queries_capped, 1);
    assert_eq!(stats.pierce_pairs_skipped, n - cap);

    // One owner: no pair, no step.
    let scene = cube_scene();
    scene.resolve(&query(
        ray_at(Point3::new(2.0, -2.0, 3.0), Point3::new(0.5, 0.5, 1.0)),
        WIDE,
    ));
    assert_eq!(scene.crossing_stats(), inference::CrossingStats::default());
}

mod pierce_props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any panel set into any wall: the piercing of the panel's
        /// top-front edge through the wall's near face snaps exactly, from
        /// any eye in front of and above it, identically on the indexed
        /// and linear paths.
        #[test]
        fn a_panel_set_into_a_wall_snaps_its_piercing_exactly(
            xw in 0.5f64..3.0,
            thickness in 0.05f64..1.0,
            length in 1.0f64..4.0,
            height in 1.0f64..3.0,
            fy in 0.1f64..0.9,
            fz in 0.1f64..0.9,
            depth in 0.02f64..0.4,
            eye in (0.5f64..2.0, 0.2f64..1.5, 0.2f64..1.5),
        ) {
            // The wall's near face x = xw spans y 0..length, z 0..height;
            // the panel's top-front edge crosses it at least 10 cm inside
            // every boundary (the panel is 10 cm thick and 60 cm tall, so
            // its other piercings are at least 10 cm away).
            let py = 0.1 + fy * (length - 0.2);
            let pz = 0.1 + fz * (height - 0.2);
            prop_assume!(pz > 0.6);
            let pierce = Point3::new(xw, py, pz);
            let wall = box_object(
                Point3::new(xw, 0.0, 0.0),
                Point3::new(xw + thickness, length, height),
            );
            let panel = box_object(
                Point3::new(xw - 2.0, py, pz - 0.6),
                Point3::new(xw + depth, py + 0.1, pz),
            );
            let mut scene = InferenceScene::new();
            scene.add_object(oid(1), &wall, &Transform::IDENTITY);
            scene.add_object(oid(2), &panel, &Transform::IDENTITY);
            let eye = Point3::new(xw - 1.0 - eye.0, py - eye.1, pz + eye.2);
            let q = query(ray_at(eye, pierce), NARROW);
            let snap = scene.resolve(&q);
            prop_assert!(
                snap.is_some_and(|s| s.kind == SnapKind::Intersection
                    && s.position.approx_eq(pierce, tol::POINT_MERGE)
                    && matches!(s.pierced_face, Some(PiercedFace::Object(f)) if f.object == oid(1))
                    && s.source.is_some_and(|e| e.object == oid(2))),
                "{:?}",
                snap
            );
            prop_assert_eq!(snap, scene.resolve_linear(&q));
        }
    }
}

/// The pool holds only faces the pick cone reaches, drawn regions included,
/// so geometry far from the cursor — however much of it — never spends the
/// cap, and the linear reference path, which sees every face, pools the
/// same ones and agrees with the indexed path.
#[test]
fn faces_far_from_the_cursor_never_reach_the_pool_and_both_paths_agree() {
    let mut scene = InferenceScene::new();
    // A drawn line slanting down through a drawn square 0.5 m below the
    // ground: the piercing at (0.525, 0.5, -0.5).
    let plate = |z: f64, x0: f64| inference::SketchRegionFace {
        region: kernel::SketchRegionId::default(),
        plane: Plane::from_polygon(&[
            Point3::new(x0, 0.0, z),
            Point3::new(x0 + 1.0, 0.0, z),
            Point3::new(x0 + 1.0, 1.0, z),
        ])
        .unwrap(),
        boundary: vec![
            Point3::new(x0, 0.0, z),
            Point3::new(x0 + 1.0, 0.0, z),
            Point3::new(x0 + 1.0, 1.0, z),
            Point3::new(x0, 1.0, z),
        ],
        holes: vec![],
    };
    // Two thousand drawn squares and 340 boxes (2,040 faces) a kilometre
    // away, all registered before the target so they come first in every
    // order but distance.
    for i in 0..2000u64 {
        scene.add_sketch_faces(sid(i + 1), &[plate(-0.001 * (i + 1) as f64, 1000.0)]);
    }
    for i in 0..340u64 {
        let x = 1000.0 + 3.0 * i as f64;
        scene.add_object(
            oid(i + 1),
            &box_object(Point3::new(x, 0.0, 0.0), Point3::new(x + 1.0, 1.0, 1.0)),
            &Transform::IDENTITY,
        );
    }
    scene.add_sketch_faces(sid(9000), &[plate(-0.5, 0.0)]);
    scene.add_sketch(
        sid(10_000),
        &[(
            eid(1),
            Point3::new(0.3, 0.5, 4.0),
            Point3::new(0.65, 0.5, -3.0),
        )],
    );
    let q = query(
        PickRay {
            origin: Point3::new(0.5, 0.5, 3.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        NARROW,
    );
    let snap = scene.resolve(&q).expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, Point3::new(0.525, 0.5, -0.5));
    let stats = scene.crossing_stats();
    assert_eq!(
        stats.pierce_faces_pooled, 1,
        "only the target square: {stats:?}"
    );
    assert_eq!(stats.pierce_pairs_tested, 1);
    assert_eq!(stats.pierce_queries_capped, 0);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// Faces the cone reaches but the cursor is not on cannot starve the face
/// it IS on: pairs are tested in order of their distance from the ray, so
/// the pierced face under the cursor is tested first however many nearby
/// decoys the cap then cuts off — and the indexed and linear paths agree
/// about which pairs the cap reached.
#[test]
fn decoys_near_the_cursor_do_not_starve_the_piercing_under_it() {
    let cap = inference::PIERCE_PAIR_CAP as u64;
    let mut scene = InferenceScene::new();
    // Decoys: 2 cm drawn squares a centimetre beside the ray, stacked a
    // millimetre apart, each its own sketch, registered first. The line
    // below passes at x ≥ 0.5 and never pierces one.
    let n = cap + 100;
    for i in 0..n {
        let z = -0.001 * (i + 1) as f64;
        let at = |x: f64, y: f64| Point3::new(x, y, z);
        scene.add_sketch_faces(
            sid(i + 1),
            &[inference::SketchRegionFace {
                region: kernel::SketchRegionId::default(),
                plane: Plane::from_polygon(&[at(0.47, 0.49), at(0.49, 0.49), at(0.49, 0.51)])
                    .unwrap(),
                boundary: vec![
                    at(0.47, 0.49),
                    at(0.49, 0.49),
                    at(0.49, 0.51),
                    at(0.47, 0.51),
                ],
                holes: vec![],
            }],
        );
    }
    // The target: a unit square 0.5 m down, the ray through it.
    let z = -0.5;
    let at = |x: f64, y: f64| Point3::new(x, y, z);
    scene.add_sketch_faces(
        sid(9000),
        &[inference::SketchRegionFace {
            region: kernel::SketchRegionId::default(),
            plane: Plane::from_polygon(&[at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0)]).unwrap(),
            boundary: vec![at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
            holes: vec![],
        }],
    );
    scene.add_sketch(
        sid(10_000),
        &[(
            eid(1),
            Point3::new(0.3, 0.5, 4.0),
            Point3::new(0.65, 0.5, -3.0),
        )],
    );
    let q = query(
        PickRay {
            origin: Point3::new(0.5, 0.5, 3.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        },
        NARROW,
    );
    let snap = scene.resolve(&q).expect("the piercing is under the cursor");
    assert_eq!(snap.kind, SnapKind::Intersection, "{snap:?}");
    assert_at(&snap, Point3::new(0.525, 0.5, -0.5));
    let stats = scene.crossing_stats();
    assert_eq!(
        stats.pierce_faces_pooled,
        n + 1,
        "every decoy is in the cone"
    );
    assert_eq!(stats.pierce_pairs_tested, cap, "the cap was reached");
    assert_eq!(
        (stats.pierce_queries_capped, stats.pierce_pairs_skipped),
        (1, n + 1 - cap)
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// Placements under a rotation, a tilt, a non-uniform scale and a mirror:
/// the face of a placed wall is pierced where the posed geometry says, and
/// so is a world face by a placed panel's edge, identically on the indexed
/// and linear paths.
#[test]
fn piercings_hold_under_rotated_scaled_and_mirrored_placements() {
    let poses: Vec<(&str, Transform)> = vec![
        (
            "rotated",
            Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.3)
                .unwrap()
                .then(&Transform::translation(Vec3::new(1.0, -0.2, 0.1))),
        ),
        (
            "tilted",
            Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -0.4)
                .unwrap()
                .then(&Transform::translation(Vec3::new(1.0, 0.3, 0.2))),
        ),
        (
            "scaled",
            Transform::scale(Vec3::new(2.0, 0.5, 1.5))
                .then(&Transform::translation(Vec3::new(1.0, 0.0, 0.0))),
        ),
        (
            "mirrored",
            Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
                .then(&Transform::translation(Vec3::new(1.0, 0.0, 0.0))),
        ),
    ];
    let instance = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 5));
    // A unit vector perpendicular to `v`, leaning the way `lean` maps to.
    let aside = |pose: &Transform, v: Vec3, lean: Vec3| {
        let w = pose.apply_vector(lean);
        let v = v.normalized().unwrap();
        (w - v * w.dot(v)).normalized().unwrap()
    };
    for (name, pose) in &poses {
        // The wall placed: its near face is the member's x = 0, pierced at
        // the image of (0, 0.9, 0.6) by a drawn line along the image of x.
        let member = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(0.2, 2.0, 1.0));
        let mut scene = InferenceScene::new();
        scene.set_def_member(oid(1), &member);
        scene.add_placement(instance, oid(1), pose);
        let q0 = pose.apply_point(Point3::new(0.0, 0.9, 0.6));
        let v = pose.apply_vector(Vec3::new(1.0, 0.0, 0.0));
        scene.add_sketch(sid(1), &[(eid(1), q0 - v * 0.5, q0 + v * 0.1)]);
        let side = aside(pose, v, Vec3::new(0.0, -1.0, 1.0));
        let eye = q0 - v.normalized().unwrap() * 1.5 + side * 0.8;
        let q = query(ray_at(eye, q0), NARROW);
        let snap = scene
            .resolve(&q)
            .unwrap_or_else(|| panic!("{name}: the piercing is under the cursor"));
        assert_eq!(snap.kind, SnapKind::Intersection, "{name}: {snap:?}");
        assert!(
            snap.position.approx_eq(q0, tol::POINT_MERGE),
            "{name}: {snap:?}"
        );
        assert_eq!(snap.sketch_source, Some((sid(1), eid(1))), "{name}");
        assert!(
            matches!(
                snap.pierced_face,
                Some(PiercedFace::Object(f)) if f.object == oid(1) && f.instance == Some(instance)
            ),
            "{name}: {snap:?}"
        );
        assert_eq!(scene.resolve(&q), scene.resolve_linear(&q), "{name}");

        // The panel placed: its top-front edge, the image of
        // (0,0.9,0.6)–(1.3,0.9,0.6), through a drawn square standing
        // across it at the image of (1.0, 0.9, 0.6).
        let member = box_object(Point3::new(0.0, 0.9, 0.0), Point3::new(1.3, 1.0, 0.6));
        let mut scene = InferenceScene::new();
        scene.set_def_member(oid(2), &member);
        scene.add_placement(instance, oid(2), pose);
        let q1 = pose.apply_point(Point3::new(1.0, 0.9, 0.6));
        let d = (pose.apply_point(Point3::new(1.3, 0.9, 0.6))
            - pose.apply_point(Point3::new(0.0, 0.9, 0.6)))
        .normalized()
        .unwrap();
        let u = aside(pose, d, Vec3::new(0.0, 1.0, 0.0));
        let w = d.cross(u);
        let rid = kernel::SketchRegionId::default();
        scene.add_sketch_faces(
            sid(2),
            &[inference::SketchRegionFace {
                region: rid,
                plane: Plane::from_point_normal(q1, d).unwrap(),
                boundary: vec![
                    q1 - u * 0.5 - w * 0.5,
                    q1 + u * 0.5 - w * 0.5,
                    q1 + u * 0.5 + w * 0.5,
                    q1 - u * 0.5 + w * 0.5,
                ],
                holes: vec![],
            }],
        );
        // In front of the square, off to the side the panel is not on.
        let out = aside(pose, d, Vec3::new(0.0, -1.0, 1.0));
        let eye = q1 - d * 1.2 + out * 1.0;
        let q = query(ray_at(eye, q1), NARROW);
        let snap = scene
            .resolve(&q)
            .unwrap_or_else(|| panic!("{name}: the piercing is under the cursor"));
        assert_eq!(snap.kind, SnapKind::Intersection, "{name}: {snap:?}");
        assert!(
            snap.position.approx_eq(q1, tol::POINT_MERGE),
            "{name}: {snap:?}"
        );
        assert_eq!(
            snap.source.map(|e| (e.object, e.instance)),
            Some((oid(2), Some(instance))),
            "{name}: {snap:?}"
        );
        assert_eq!(
            snap.pierced_face,
            Some(PiercedFace::SketchRegion(sid(2), rid)),
            "{name}"
        );
        assert_eq!(scene.resolve(&q), scene.resolve_linear(&q), "{name}");
    }
}

// ---------------------------------------------------------------------------
// Face × face meeting lines between different owners.
//
// A slab passing through a wall draws four lines on the wall's face where
// the slab's faces meet it. Separate Objects never merge, so none of them is
// an edge — there is nothing on the wall but its face — yet each is a line a
// person sees and aims at, and hovering it offered only On Face until the
// cursor reached one of the slab's real edges. The stretch of the two
// planes' intersection line inside both faces (outer loops, outside their
// holes) is offered as a virtual edge: `OnIntersectionLine` along it, a
// `Midpoint` at its middle, with the first face in `source` (or
// `sketch_region_source`) and the second face and the stretch's ends in
// `meeting_line`. Its ends are the piercings and crossings the Intersection
// walks already name. Only between different owners; parallel faces make no
// line; never for a selection pick.
// ---------------------------------------------------------------------------

/// A slab 10 cm thick passing through the wall's near face (x = 1): x
/// 0.5..1.2, y 0.6..1.4, z 0.4..0.5. Its top, bottom and two side faces
/// meet the wall's near face along four lines: z = 0.5 and z = 0.4 over y
/// 0.6..1.4, and y = 0.6 and y = 1.4 over z 0.4..0.5. Its end face inside
/// the wall meets nothing.
fn slab() -> Object {
    box_object(Point3::new(0.5, 0.6, 0.4), Point3::new(1.2, 1.4, 0.5))
}

fn wall_and_slab() -> InferenceScene {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    scene.add_object(oid(2), &slab(), &Transform::IDENTITY);
    scene
}

/// The four meeting lines as `(name, one end, other end, eye)`: an eye in
/// front of the wall that sees the whole line with nothing in between. The
/// side lines are 10 cm long, so their eyes are close: from 2 m out the
/// ends' and middle's point reach (`POINT_SNAP_REACH` apertures each)
/// covers the whole line, and the line itself is rightly never offered.
fn meeting_lines() -> [(&'static str, Point3, Point3, Point3); 4] {
    [
        (
            "top",
            Point3::new(1.0, 0.6, 0.5),
            Point3::new(1.0, 1.4, 0.5),
            Point3::new(-1.0, 1.0, 1.4),
        ),
        (
            "bottom",
            Point3::new(1.0, 0.6, 0.4),
            Point3::new(1.0, 1.4, 0.4),
            Point3::new(-1.0, 1.0, 0.05),
        ),
        (
            "near side",
            Point3::new(1.0, 0.6, 0.4),
            Point3::new(1.0, 0.6, 0.5),
            Point3::new(0.6, 0.3, 0.45),
        ),
        (
            "far side",
            Point3::new(1.0, 1.4, 0.4),
            Point3::new(1.0, 1.4, 0.5),
            Point3::new(0.6, 1.7, 0.45),
        ),
    ]
}

/// A query from `eye` aimed a hair in front of the wall at `at`.
fn meeting_query(eye: Point3, at: Point3) -> SnapQuery {
    query(ray_at(eye, at + Vec3::new(-0.001, 0.0, 0.0)), NARROW)
}

fn is_segment(line: &inference::MeetingLine, p: Point3, q: Point3) -> bool {
    (line.a.approx_eq(p, tol::POINT_MERGE) && line.b.approx_eq(q, tol::POINT_MERGE))
        || (line.a.approx_eq(q, tol::POINT_MERGE) && line.b.approx_eq(p, tol::POINT_MERGE))
}

/// Which face a provenance names: `(object, instance)`.
fn face_owner(face: PiercedFace) -> (ObjectId, Option<InstanceId>) {
    match face {
        PiercedFace::Object(s) => {
            assert!(matches!(s.element, ElementRef::Face(_)), "{s:?}");
            (s.object, s.instance)
        }
        PiercedFace::SketchRegion(..) => panic!("an Object face: {face:?}"),
    }
}

#[test]
fn the_line_where_two_objects_faces_meet_snaps_as_a_virtual_edge() {
    let scene = wall_and_slab();
    for (name, a, b, eye) in meeting_lines() {
        // A quarter of the way along the line: clear of the point reach of
        // its middle and of its ends.
        let at = a + (b - a) * 0.25;
        let q = meeting_query(eye, at);
        let snap = scene.resolve(&q).expect("the line is under the cursor");
        assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{name}: {snap:?}");
        // On the line: on the wall's plane, on the slab's face's plane, and
        // near the aimed station.
        assert!(
            (snap.position.x - 1.0).abs() <= tol::POINT_MERGE,
            "{name}: {snap:?}"
        );
        assert!(
            snap.position.approx_eq(at, 0.01),
            "{name}: landed {:?}, aimed {at:?}",
            snap.position
        );
        let along = (snap.position - a).dot((b - a).normalized().unwrap());
        let off = (snap.position - a) - (b - a).normalized().unwrap() * along;
        assert!(off.length() <= 1e-9, "{name}: off the line by {off:?}");
        // Both faces are the provenance: the wall's and the slab's, the
        // stretch exactly the line.
        let line = snap.meeting_line.expect("the meeting line");
        assert!(is_segment(&line, a, b), "{name}: {line:?}");
        let first = face_owner(PiercedFace::Object(snap.source.expect("the first face")));
        let second = face_owner(line.face);
        let mut owners = [first.0, second.0];
        owners.sort();
        assert_eq!(owners, [oid(1), oid(2)], "{name}: {snap:?}");
        assert_eq!(snap.pierced_face, None, "{name}");
        assert_eq!(snap.crossed_edge, None, "{name}");
        assert_eq!(
            snap.direction, None,
            "{name}: a line names no direction cue"
        );
        assert_eq!(
            scene.resolve(&q),
            scene.resolve_linear(&q),
            "{name}: the indexed and linear walks agree"
        );
    }
    let stats = scene.crossing_stats();
    assert!(
        stats.meeting_faces_pooled > 0 && stats.meeting_pairs_tested > 0,
        "{stats:?}"
    );
}

/// With only the wall the cursor there is On Face on the wall; with only
/// the slab it is On Face on the slab's top (the ray, aimed a hair in front
/// of the wall, comes down onto the top just short of where the wall would
/// be); together it is the line.
#[test]
fn the_meeting_line_exists_only_with_both_objects() {
    let (_, a, b, eye) = meeting_lines()[0];
    let at = a + (b - a) * 0.25;
    let face_of = |snap: &Snap| match snap.source {
        Some(s) if matches!(s.element, ElementRef::Face(_)) => s.object,
        _ => panic!("a face: {snap:?}"),
    };
    let mut only_wall = InferenceScene::new();
    only_wall.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let alone = resolve(&only_wall, meeting_query(eye, at)).expect("the wall's face");
    assert_eq!(alone.kind, SnapKind::OnFace, "{alone:?}");
    assert_eq!(face_of(&alone), oid(1));
    assert!((alone.position.x - 1.0).abs() <= 1e-9, "{alone:?}");
    assert_eq!(alone.meeting_line, None);
    let mut only_slab = InferenceScene::new();
    only_slab.add_object(oid(2), &slab(), &Transform::IDENTITY);
    let alone = resolve(&only_slab, meeting_query(eye, at)).expect("the slab's top");
    assert_eq!(alone.kind, SnapKind::OnFace, "{alone:?}");
    assert_eq!(face_of(&alone), oid(2));
    assert!(
        (alone.position.z - 0.5).abs() <= 1e-9,
        "on the top: {alone:?}"
    );
    assert!(alone.position.x < 1.0, "short of the wall: {alone:?}");
    assert_eq!(alone.meeting_line, None);
    let stats = only_slab.crossing_stats();
    assert_eq!(
        (stats.meeting_faces_pooled, stats.meeting_pairs_tested),
        (0, 0),
        "one owner pools and tests nothing: {stats:?}"
    );
}

/// The middle of the line is a Midpoint, as on a real edge, carrying the
/// same two faces.
#[test]
fn the_middle_of_the_meeting_line_is_a_midpoint() {
    let scene = wall_and_slab();
    for (name, a, b, eye) in meeting_lines() {
        let mid = Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0);
        let q = meeting_query(eye, mid);
        let snap = scene.resolve(&q).expect("the middle is under the cursor");
        assert_eq!(snap.kind, SnapKind::Midpoint, "{name}: {snap:?}");
        assert_at(&snap, mid);
        let line = snap.meeting_line.expect("the meeting line");
        assert!(is_segment(&line, a, b), "{name}: {line:?}");
        assert_eq!(scene.resolve(&q), scene.resolve_linear(&q), "{name}");
    }
}

/// The line's middle reaches as far as a real edge's: a named point, out
/// to `POINT_SNAP_REACH` apertures, even where the line itself is past a
/// plain aperture. A slab set only a centimetre into the wall, aimed at the
/// wall above its top line's middle, between one and `POINT_SNAP_REACH`
/// apertures off it: the ray passes over the slab's top, which ends just
/// inside the wall, at about that distance, and it is still the Midpoint.
#[test]
fn the_meeting_lines_middle_reaches_as_far_as_a_real_edges() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let shallow = box_object(Point3::new(0.5, 0.6, 0.4), Point3::new(1.01, 1.4, 0.5));
    scene.add_object(oid(2), &shallow, &Transform::IDENTITY);
    let (_, a, b, eye) = meeting_lines()[0];
    let mid = Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0);
    let angle = |at: Point3| {
        let r = meeting_query(eye, at).ray.direction.normalized().unwrap();
        let m = (mid - eye).normalized().unwrap();
        r.cross(m).length().atan2(r.dot(m))
    };
    let target = 1.6 * NARROW;
    assert!(target < POINT_SNAP_REACH * NARROW);
    let mut dz = 0.0;
    while angle(mid + Vec3::new(0.0, 0.0, dz)) < target {
        dz += 1e-4;
    }
    let q = meeting_query(eye, mid + Vec3::new(0.0, 0.0, dz));
    let snap = scene.resolve(&q).expect("the middle is in reach");
    assert_eq!(snap.kind, SnapKind::Midpoint, "{dz}: {snap:?}");
    assert_at(&snap, mid);
    assert!(snap.meeting_line.is_some(), "{snap:?}");
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// The pair's order is the scene's, not the cursor's: wherever along and
/// across the line the cursor sits, every snap on it names the same first
/// face and hands back the same ends in the same order, so a held snap
/// does not change provenance as the cursor moves.
#[test]
fn the_meeting_lines_provenance_does_not_follow_the_cursor() {
    let scene = wall_and_slab();
    for (name, a, b, eye) in meeting_lines() {
        let across = (b - a)
            .cross(Vec3::new(1.0, 0.0, 0.0))
            .normalized()
            .unwrap();
        let mut seen: Option<(Option<inference::SnapSource>, inference::MeetingLine)> = None;
        for t in [0.2, 0.3, 0.5, 0.7, 0.8] {
            for off in [-0.002, -0.0005, 0.0, 0.0005, 0.002] {
                let at = a + (b - a) * t + across * off;
                let Some(snap) = scene.resolve(&meeting_query(eye, at)) else {
                    continue;
                };
                let Some(line) = snap.meeting_line else {
                    continue;
                };
                match seen {
                    None => seen = Some((snap.source, line)),
                    Some(first) => {
                        assert_eq!(first, (snap.source, line), "{name}: at {t}, {off} off")
                    }
                }
            }
        }
        assert!(seen.is_some(), "{name}: the line was under the cursor");
    }
}

/// Past either end of the stretch the wall is just a face: the planes'
/// line runs on, the slab's face does not. Above the slab likewise.
#[test]
fn beyond_the_stretch_there_is_no_line() {
    let scene = wall_and_slab();
    let (_, _, _, eye) = meeting_lines()[0];
    for at in [
        Point3::new(1.0, 1.6, 0.5), // on the top line's extension, past the slab
        Point3::new(1.0, 0.4, 0.5), // and before it
        Point3::new(1.0, 1.0, 0.7), // above the slab, mid-wall
    ] {
        let snap = scene
            .resolve(&meeting_query(eye, at))
            .expect("the wall is under the cursor");
        assert_eq!(snap.kind, SnapKind::OnFace, "aimed at {at:?}: {snap:?}");
        assert_eq!(snap.meeting_line, None);
    }
}

/// The line's ends are where the slab's edges pierce the wall's face: the
/// Intersection snap, which outranks the line and reaches as far as any
/// named point, so near an end the piercing wins.
#[test]
fn the_meeting_lines_ends_are_piercings_and_still_win() {
    let scene = wall_and_slab();
    let (_, a, b, eye) = meeting_lines()[0];
    for end in [a, b] {
        // Dead on, and a few millimetres along the line from it.
        let toward = (b - a).normalized().unwrap() * if end == a { 1.0 } else { -1.0 };
        for at in [end, end + toward * 0.004] {
            let snap = scene
                .resolve(&meeting_query(eye, at))
                .expect("the end is under the cursor");
            assert_eq!(
                snap.kind,
                SnapKind::Intersection,
                "aimed at {at:?}: {snap:?}"
            );
            assert_at(&snap, end);
            assert!(snap.pierced_face.is_some(), "{snap:?}");
            assert_eq!(snap.meeting_line, None, "a piercing, not the line");
        }
    }
}

/// A real edge running along the meeting line wins: a box flush against
/// the wall, its top face meeting the wall's face along its own back edge.
#[test]
fn a_real_edge_along_the_meeting_line_wins() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    let flush = box_object(Point3::new(0.5, 0.6, 0.4), Point3::new(1.0, 1.4, 0.5));
    scene.add_object(oid(2), &flush, &Transform::IDENTITY);
    let (_, a, b, eye) = meeting_lines()[0];
    let at = a + (b - a) * 0.3;
    let q = meeting_query(eye, at);
    let snap = scene.resolve(&q).expect("the edge is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnEdge, "{snap:?}");
    let edge = snap.source.expect("the edge's provenance");
    assert_eq!(edge.object, oid(2));
    assert!(matches!(edge.element, ElementRef::Edge(_)), "{snap:?}");
    assert_eq!(snap.meeting_line, None);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// Parallel faces make no line: a second wall behind the first, and the
/// wall's own faces, meet along nothing.
#[test]
fn parallel_faces_and_one_owners_faces_make_no_meeting_line() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    scene.add_object(
        oid(2),
        &box_object(Point3::new(1.6, 0.0, 0.0), Point3::new(2.0, 2.0, 1.0)),
        &Transform::IDENTITY,
    );
    let eye = Point3::new(-1.0, 1.0, 1.4);
    let snap = scene
        .resolve(&meeting_query(eye, Point3::new(1.0, 1.0, 0.5)))
        .expect("the wall's face");
    assert_eq!(snap.kind, SnapKind::OnFace, "{snap:?}");
    assert_eq!(snap.meeting_line, None);
    // Hovering the wall's own top edge: On Edge, and no face pair is ever
    // tested — one owner's faces meet at its own edges.
    let scene = cube_scene();
    let q = query(
        ray_at(Point3::new(0.4, -2.0, 3.0), Point3::new(0.4, 0.0, 1.0)),
        NARROW,
    );
    let snap = scene.resolve(&q).expect("the cube's edge");
    assert_eq!(snap.kind, SnapKind::OnEdge, "{snap:?}");
    assert_eq!(snap.meeting_line, None);
    let stats = scene.crossing_stats();
    assert_eq!(
        (stats.meeting_faces_pooled, stats.meeting_pairs_tested),
        (0, 0),
        "{stats:?}"
    );
}

/// A hole in a face cuts the line: a slab through a window in a drawn
/// region leaves a stretch on each side of the window and none across it,
/// and the stretches carry the region as a face.
#[test]
fn a_hole_in_a_face_cuts_the_meeting_line() {
    let rid = kernel::SketchRegionId::default();
    // A 2 × 2 drawn region standing in the plane x = 3, with a 0.4 × 0.4
    // window in its middle (y 0.8..1.2, z 0.8..1.2).
    let plate = inference::SketchRegionFace {
        region: rid,
        plane: Plane::from_point_normal(Point3::new(3.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
            .unwrap(),
        boundary: vec![
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 2.0, 0.0),
            Point3::new(3.0, 2.0, 2.0),
            Point3::new(3.0, 0.0, 2.0),
        ],
        holes: vec![vec![
            Point3::new(3.0, 0.8, 0.8),
            Point3::new(3.0, 1.2, 0.8),
            Point3::new(3.0, 1.2, 1.2),
            Point3::new(3.0, 0.8, 1.2),
        ]],
    };
    let mut scene = InferenceScene::new();
    scene.add_sketch_faces(sid(1), &[plate]);
    // A slab through the plate whose top face (z = 1) crosses the window:
    // y 0.5..1.5, so the top line is cut into y 0.5..0.8 and 1.2..1.5.
    let slab = box_object(Point3::new(2.5, 0.5, 0.9), Point3::new(3.5, 1.5, 1.0));
    scene.add_object(oid(1), &slab, &Transform::IDENTITY);
    let eye = Point3::new(0.0, 1.0, 3.0);
    let probe = |at: Point3| query(ray_at(eye, at + Vec3::new(-0.001, 0.0, 0.0)), NARROW);

    // Across the window: no line — the plate has no material there for the
    // slab's top to meet. (The ray, aimed a hair in front of the plate,
    // lands on the slab's top just short of it.)
    let q = probe(Point3::new(3.0, 1.0, 1.0));
    let snap = scene.resolve(&q);
    assert!(
        snap.is_none_or(|s| s.kind != SnapKind::OnIntersectionLine && s.meeting_line.is_none()),
        "{snap:?}"
    );
    // Either side of it, off each stretch's middle: the stretch, with its
    // ends at the window's edge.
    for (at, a, b) in [
        (
            Point3::new(3.0, 0.75, 1.0),
            Point3::new(3.0, 0.5, 1.0),
            Point3::new(3.0, 0.8, 1.0),
        ),
        (
            Point3::new(3.0, 1.25, 1.0),
            Point3::new(3.0, 1.2, 1.0),
            Point3::new(3.0, 1.5, 1.0),
        ),
    ] {
        let q = probe(at);
        let snap = scene.resolve(&q).expect("the stretch is under the cursor");
        assert_eq!(
            snap.kind,
            SnapKind::OnIntersectionLine,
            "aimed at {at:?}: {snap:?}"
        );
        let line = snap.meeting_line.expect("the meeting line");
        assert!(is_segment(&line, a, b), "{line:?}");
        // The region is one face and the slab's top the other, in pool
        // order (world faces before drawn regions).
        assert_eq!(snap.source.map(|s| s.object), Some(oid(1)), "{snap:?}");
        assert_eq!(line.face, PiercedFace::SketchRegion(sid(1), rid));
        assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
    }
}

/// A placed component's faces meet a world face like a world object's —
/// scaled, mirrored and rotated placements included — and the provenance
/// names the placement.
#[test]
fn a_placed_instances_faces_meet_a_world_face() {
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    // The member: a unit cube centered on the origin. Scaled to 0.7 × 0.8
    // × 0.1 with y mirrored, turned a quarter turn about z (x → y, y → -x),
    // and moved to (0.85, 1.0, 0.45): x 0.45..1.25, y 0.65..1.35, z
    // 0.4..0.5 — a slab through the wall's near face, its top line z = 0.5
    // over y 0.65..1.35.
    let member = box_object(Point3::new(-0.5, -0.5, -0.5), Point3::new(0.5, 0.5, 0.5));
    scene.set_def_member(oid(2), &member);
    let instance = InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | 7));
    let pose = Transform::scale(Vec3::new(0.7, -0.8, 0.1))
        .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2).unwrap())
        .then(&Transform::translation(Vec3::new(0.85, 1.0, 0.45)));
    scene.add_placement(instance, oid(2), &pose);
    let (a, b) = (Point3::new(1.0, 0.65, 0.5), Point3::new(1.0, 1.35, 0.5));
    let eye = Point3::new(-1.0, 1.0, 1.4);
    let at = a + (b - a) * 0.3;
    let q = meeting_query(eye, at);
    let snap = scene.resolve(&q).expect("the line is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    assert!(snap.position.approx_eq(at, 0.01), "{snap:?}");
    assert!((snap.position.z - 0.5).abs() <= 1e-9 && (snap.position.x - 1.0).abs() <= 1e-9);
    let line = snap.meeting_line.expect("the meeting line");
    assert!(is_segment(&line, a, b), "{line:?}");
    let first = face_owner(PiercedFace::Object(snap.source.expect("the first face")));
    let second = face_owner(line.face);
    // World faces come first in the pool: the wall, then the placement.
    assert_eq!(first, (oid(1), None), "{snap:?}");
    assert_eq!(second, (oid(2), Some(instance)), "{snap:?}");
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
    // The middle, a Midpoint on the placed line.
    let mid = Point3::new(1.0, 1.0, 0.5);
    let snap = scene.resolve(&meeting_query(eye, mid)).expect("the middle");
    assert_eq!(snap.kind, SnapKind::Midpoint, "{snap:?}");
    assert_at(&snap, mid);
}

/// Faces exist for input only under a policy that has them: in Wireframe
/// no line; in X-ray the line is offered like in Shaded.
#[test]
fn meeting_lines_follow_the_face_policy() {
    let mut scene = wall_and_slab();
    let (_, a, b, eye) = meeting_lines()[0];
    let q = meeting_query(eye, a + (b - a) * 0.3);
    scene.set_face_policy(FacePolicy::Absent);
    let snap = scene.resolve(&q);
    assert!(
        snap.is_none_or(|s| s.kind != SnapKind::OnIntersectionLine && s.meeting_line.is_none()),
        "{snap:?}"
    );
    scene.set_face_policy(FacePolicy::SeeThrough);
    let snap = scene.resolve(&q).expect("the line");
    assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    assert!(snap.meeting_line.is_some());
}

/// A selection pick asks what is under the cursor; a meeting line is two
/// owners' faces at once and nothing to select, so the pick lands on the
/// face the ray hits (here the slab's top: the ray, aimed a hair in front of
/// the wall, comes down onto it just short of the wall).
#[test]
fn a_selection_pick_is_never_offered_a_meeting_line() {
    let scene = wall_and_slab();
    let (_, a, b, eye) = meeting_lines()[0];
    let q = SnapQuery {
        weights: SnapWeights::default().for_picking(),
        ..meeting_query(eye, a + (b - a) * 0.3)
    };
    let snap = scene.resolve(&q).expect("a face");
    assert_eq!(snap.kind, SnapKind::OnFace, "{snap:?}");
    assert_eq!(snap.source.map(|s| s.object), Some(oid(2)), "{snap:?}");
    assert!((snap.position.z - 0.5).abs() <= 1e-9, "{snap:?}");
    assert_eq!(snap.meeting_line, None);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// The line is a sliding kind for the drawing-plane filter, like On Edge:
/// kept on a plane it lies in (either face's), dropped off one.
#[test]
fn the_meeting_line_honors_the_drawing_plane() {
    let scene = wall_and_slab();
    let (_, a, b, eye) = meeting_lines()[0];
    let at = a + (b - a) * 0.3;
    let on = |plane: Plane| SnapQuery {
        constraint_plane: Some(plane),
        ..meeting_query(eye, at)
    };
    let wall_plane =
        Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let top_plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let ground = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    for plane in [wall_plane, top_plane] {
        let snap = scene.resolve(&on(plane)).expect("the line");
        assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    }
    let snap = scene.resolve(&on(ground));
    assert!(
        snap.is_none_or(|s| s.kind != SnapKind::OnIntersectionLine),
        "{snap:?}"
    );
}

/// Under an axis lock the line's point is projected onto the locked line
/// like any candidate, keeping its kind and both faces, with the unprojected
/// point in `projected_from`.
#[test]
fn the_meeting_line_projects_under_an_axis_lock() {
    let scene = wall_and_slab();
    let (_, a, b, eye) = meeting_lines()[0];
    let at = a + (b - a) * 0.25; // (1, 0.8, 0.5)
    let anchor = Point3::new(1.0, 0.7, 0.2);
    let q = SnapQuery {
        anchor: Some(anchor),
        lock: Some(SnapLock::Axis(inference::Axis::Z)),
        ..meeting_query(eye, at)
    };
    let snap = scene.resolve(&q).expect("a locked snap");
    assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    assert!((snap.position.x - 1.0).abs() <= 1e-9 && (snap.position.y - 0.7).abs() <= 1e-9);
    assert!((snap.position.z - 0.5).abs() <= 1e-6, "{snap:?}");
    let from = snap.projected_from.expect("the line's own point");
    assert!(
        from.approx_eq(at, 0.01) && (from.z - 0.5).abs() <= 1e-9,
        "{from:?}"
    );
    assert!(snap.meeting_line.is_some());
    assert_eq!(snap.direction, Some(Vec3::new(0.0, 0.0, 1.0)));
}

/// A dense spot cannot stall a pointer move: a query tests at most
/// `MEETING_PAIR_CAP` cross-owner face pairs, nearest first, and counts the
/// pairs it left untested in `crossing_stats` rather than dropping them
/// silently; the indexed walk agrees with the linear one either way.
#[test]
fn meeting_pairs_past_the_cap_are_skipped_and_counted() {
    let cap = inference::MEETING_PAIR_CAP as u64;
    // One drawn plate standing in the plane y = 0.5 through a stack of `n`
    // drawn squares (each its own sketch) a millimetre apart, seen from
    // above and a little to the side: every square meets the plate along
    // a line, every pair of squares is parallel, and all n + 1 faces are
    // different owners, so there are (n + 1) n / 2 pairs to test.
    let stack = |n: u64| {
        let mut scene = InferenceScene::new();
        let at = |x: f64, z: f64| Point3::new(x, 0.5, z);
        scene.add_sketch_faces(
            sid(10_000),
            &[inference::SketchRegionFace {
                region: kernel::SketchRegionId::default(),
                plane: Plane::from_polygon(&[at(0.0, -1.0), at(1.0, -1.0), at(1.0, 1.0)]).unwrap(),
                boundary: vec![at(0.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(0.0, 1.0)],
                holes: vec![],
            }],
        );
        for i in 0..n {
            let z = -0.001 * (i + 1) as f64;
            let at = |x: f64, y: f64| Point3::new(x, y, z);
            scene.add_sketch_faces(
                sid(i + 1),
                &[inference::SketchRegionFace {
                    region: kernel::SketchRegionId::default(),
                    plane: Plane::from_polygon(&[at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0)])
                        .unwrap(),
                    boundary: vec![at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
                    holes: vec![],
                }],
            );
        }
        scene
    };
    // Aimed at the top square's line, 30 cm along it, from above and 2 cm
    // to the side so the plate's top edge is clear of the ray.
    let first = Point3::new(0.3, 0.5, -0.001);
    let q = query(ray_at(Point3::new(0.3, 0.48, 3.0), first), NARROW);

    // Under the cap: every pair is tested, the top square's line wins.
    let n = 20;
    let scene = stack(n);
    let snap = scene.resolve(&q).expect("the stack is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    assert_at(&snap, first);
    let stats = scene.crossing_stats();
    assert_eq!(
        stats.meeting_faces_pooled,
        n + 1,
        "every face is in the cone"
    );
    assert_eq!(stats.meeting_pairs_tested, (n + 1) * n / 2);
    assert_eq!(
        (stats.meeting_queries_capped, stats.meeting_pairs_skipped),
        (0, 0)
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    // Past it: exactly `cap` pairs are tested, the rest counted.
    let n = 70;
    assert!((n + 1) * n / 2 > cap);
    let scene = stack(n);
    let snap = scene.resolve(&q).expect("the stack is under the cursor");
    assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
    assert_at(&snap, first);
    let stats = scene.crossing_stats();
    assert_eq!(stats.meeting_pairs_tested, cap);
    assert_eq!(stats.meeting_queries_capped, 1);
    assert_eq!(stats.meeting_pairs_skipped, (n + 1) * n / 2 - cap);
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));
}

/// The cap cuts off the farthest pairs, not the line under the cursor: a
/// stack of 70 thin plates standing behind the wall, every one of them its
/// own owner and every one crossed by the pick ray, pools far more pairs
/// than `MEETING_PAIR_CAP`. The faces the ray passes through all come equally
/// close to it, so closeness alone orders them by when they were added, and
/// plates added before the wall and the slab once filled the cap with pairs
/// hidden behind the wall while the line in plain view went untested. The
/// walk takes the pool front to back, so the wall's face and the slab's top,
/// the first faces along the ray, pair first, whatever the order the scene
/// was built in — for world Objects and for placed instances alike, and on
/// the indexed path exactly as on the linear one.
#[test]
fn the_meeting_cap_tests_the_line_in_view_before_decoys_behind_it() {
    let n = 70;
    let plate = |i: u64| {
        let x = 1.6 + 0.1 * i as f64;
        box_object(
            Point3::new(x, -10.0, -10.0),
            Point3::new(x + 0.05, 10.0, 10.0),
        )
    };
    let (_, a, b, eye) = meeting_lines()[0];
    let at = a + (b - a) * 0.25;
    let q = meeting_query(eye, at);
    let check = |name: &str, scene: &InferenceScene| {
        let snap = scene.resolve(&q).expect("the line is under the cursor");
        let stats = scene.crossing_stats();
        assert_eq!(
            stats.meeting_queries_capped, 1,
            "{name}: the cap is reached: {stats:?}"
        );
        assert!(stats.meeting_pairs_skipped > 0, "{name}: {stats:?}");
        assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{name}: {snap:?}");
        let line = snap.meeting_line.expect("the meeting line");
        assert!(is_segment(&line, a, b), "{name}: {line:?}");
        assert_eq!(
            scene.resolve(&q),
            scene.resolve_linear(&q),
            "{name}: the indexed and linear walks agree under the cap"
        );
    };

    // World Objects: the plates first, then the wall and the slab.
    let mut scene = InferenceScene::new();
    for i in 0..n {
        scene.add_object(oid(100 + i), &plate(i), &Transform::IDENTITY);
    }
    scene.add_object(oid(1), &wall(), &Transform::IDENTITY);
    scene.add_object(oid(2), &slab(), &Transform::IDENTITY);
    check("world", &scene);

    // Placed instances, in the same order: one plate member placed 70
    // times, then the wall and the slab each placed once.
    let mut scene = InferenceScene::new();
    scene.set_def_member(oid(10), &plate(0));
    scene.set_def_member(oid(11), &wall());
    scene.set_def_member(oid(12), &slab());
    let instance = |n: u64| InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | n));
    for i in 0..n {
        let pose = Transform::translation(Vec3::new(0.1 * i as f64, 0.0, 0.0));
        scene.add_placement(instance(100 + i), oid(10), &pose);
    }
    scene.add_placement(instance(1), oid(11), &Transform::IDENTITY);
    scene.add_placement(instance(2), oid(12), &Transform::IDENTITY);
    check("placed", &scene);
}

/// A drawn region elsewhere in the scene does not make every hover pay for
/// the meeting walk: hovering one solid with a region far off pools no
/// face, exactly as with no region at all — the region's box is nowhere
/// near the cone. Draw the region where the cone reaches it and the walk
/// runs. The snap is the same on the indexed and the linear path either way.
#[test]
fn a_drawn_region_elsewhere_does_not_start_the_meeting_walk() {
    let region = |x: f64| {
        let at = |y: f64, z: f64| Point3::new(x, y, z);
        inference::SketchRegionFace {
            region: kernel::SketchRegionId::default(),
            plane: Plane::from_point_normal(at(0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap(),
            boundary: vec![at(0.0, 0.0), at(2.0, 0.0), at(2.0, 2.0), at(0.0, 2.0)],
            holes: vec![],
        }
    };
    // From above, onto the middle of the unit cube's top.
    let q = query(
        ray_at(Point3::new(0.4, 0.45, 3.0), Point3::new(0.4, 0.45, 1.0)),
        NARROW,
    );
    let mut scene = cube_scene();
    scene.add_sketch_faces(sid(1), &[region(100.0)]);
    let snap = scene.resolve(&q).expect("the cube's top");
    assert_eq!(snap.kind, SnapKind::OnFace, "{snap:?}");
    let stats = scene.crossing_stats();
    assert_eq!(
        (stats.meeting_faces_pooled, stats.pierce_faces_pooled),
        (0, 0),
        "a region 100 m away pools nothing: {stats:?}"
    );
    assert_eq!(scene.resolve(&q), scene.resolve_linear(&q));

    // A region standing in x = 0.4, through the cube under the cursor: the
    // walk pools the faces of both.
    let mut scene = cube_scene();
    scene.add_sketch_faces(sid(1), &[region(0.4)]);
    let snap = scene.resolve(&q);
    let stats = scene.crossing_stats();
    assert!(stats.meeting_faces_pooled > 0, "{stats:?}");
    assert_eq!(snap, scene.resolve_linear(&q));
}

/// A box flush against a wall makes no meeting line, at any rotation: its
/// top face meets the wall's face only along its own back edge, which is a
/// real edge and snaps as one, with its own middle. The contact used to be
/// decided by which side rounding put each vertex of that edge, so a turned
/// pair offered a truncated stretch of it — and, ahead of the real edge, a
/// Midpoint at the stretch's middle, a point in the middle of nothing.
#[test]
fn a_flush_box_makes_no_meeting_line_at_any_rotation() {
    let flush = box_object(Point3::new(0.5, 0.6, 0.4), Point3::new(1.0, 1.4, 0.5));
    for i in 0..40 {
        let k = i as f64;
        let axis = Vec3::new((k * 1.7).sin(), (k * 2.3).cos(), 1.0 + (k * 0.9).sin());
        let pose =
            Transform::rotation(axis, 0.37 + k * 0.29)
                .unwrap()
                .then(&Transform::translation(Vec3::new(
                    k * 3.1 - 60.0,
                    7.0 - k,
                    k * 0.5,
                )));
        let mut scene = InferenceScene::new();
        scene.set_def_member(oid(1), &wall());
        scene.set_def_member(oid(2), &flush);
        let instance = |n: u64| InstanceId::from(slotmap::KeyData::from_ffi(1 << 32 | n));
        scene.add_placement(instance(1), oid(1), &pose);
        scene.add_placement(instance(2), oid(2), &pose);
        let w = |x: f64, y: f64, z: f64| pose.apply_point(Point3::new(x, y, z));
        let eye = w(-1.0, 1.0, 1.4);
        // Along the box's back edge, and at its middle, from in front.
        for (y, kind) in [(0.84, SnapKind::OnEdge), (1.0, SnapKind::Midpoint)] {
            let q = query(ray_at(eye, w(0.999, y, 0.5)), NARROW);
            let snap = scene.resolve(&q).expect("the box's back edge");
            assert_eq!(snap.kind, kind, "turn {i}, y {y}: {snap:?}");
            assert_eq!(snap.meeting_line, None, "turn {i}, y {y}: {snap:?}");
            let source = snap.source.expect("the edge's provenance");
            assert_eq!(source.object, oid(2), "turn {i}, y {y}: {snap:?}");
            assert!(
                matches!(source.element, ElementRef::Edge(_)),
                "turn {i}, y {y}: {snap:?}"
            );
            if kind == SnapKind::Midpoint {
                assert!(
                    snap.position.approx_eq(w(1.0, 1.0, 0.5), 1e-9),
                    "turn {i}: {snap:?}"
                );
            }
            assert_eq!(scene.resolve(&q), scene.resolve_linear(&q), "turn {i}");
        }
    }
}

/// A slab through a cylinder meets its wall along a curve, cut into one
/// stretch per facet. Each stretch is the line, so the cursor slides along
/// the curve, but none offers its middle: the middle of one facet's piece
/// is no point of the curve, and the curve would otherwise scatter a
/// Midpoint over every facet. A prism of the same facets drawn as plain
/// faces — no surface behind them, as an imported mesh — keeps the
/// middles, as its own facet edges do.
#[test]
fn a_meeting_with_a_curved_surface_offers_no_middle_per_facet() {
    // A plate 5 cm thick whose top, z = 0.5, cuts the wall of a 24-facet
    // cylinder of radius 0.5 about the z axis.
    let plate = box_object(Point3::new(-1.5, -1.5, 0.45), Point3::new(1.5, 1.5, 0.5));
    let n = 24;
    let r = 0.5;
    // The facet between the vertices at 165° and 180°, its piece of the
    // line between them and that piece's middle.
    let vertex = |deg: f64| {
        let a = deg.to_radians();
        Point3::new(r * a.cos(), r * a.sin(), 0.5)
    };
    let (a, b) = (vertex(165.0), vertex(180.0));
    let mid = Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, 0.5);
    let outward = Vec3::new(mid.x, mid.y, 0.0).normalized().unwrap();
    let eye = mid + outward * 2.0 + Vec3::new(0.0, 0.0, 1.0);
    let q = query(ray_at(eye, mid + outward * 0.0005), NARROW);
    let along = query(ray_at(eye, a + (b - a) * 0.3 + outward * 0.0005), NARROW);

    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &plate, &Transform::IDENTITY);
    scene.add_object(
        oid(2),
        &analytic_cylinder(Point3::ORIGIN, r, n, 1.0),
        &Transform::IDENTITY,
    );
    for q in [&q, &along] {
        let snap = scene.resolve(q).expect("the facet's piece of the curve");
        assert_eq!(snap.kind, SnapKind::OnIntersectionLine, "{snap:?}");
        let line = snap.meeting_line.expect("the piece");
        assert!(is_segment(&line, a, b), "{line:?}");
        assert_eq!(scene.resolve(q), scene.resolve_linear(q));
    }

    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let mut s = kernel::Sketch::on_plane(plane);
    let p = |i: usize| {
        let t = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(r * t.cos(), r * t.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    let region = s.regions().keys().next().unwrap();
    let prism = Object::from_extrusion(&s.profile(region).unwrap(), 1.0).unwrap();
    let mut scene = InferenceScene::new();
    scene.add_object(oid(1), &plate, &Transform::IDENTITY);
    scene.add_object(oid(2), &prism, &Transform::IDENTITY);
    let snap = scene.resolve(&q).expect("the facet's middle");
    assert_eq!(snap.kind, SnapKind::Midpoint, "{snap:?}");
    assert_at(&snap, mid);
    assert!(snap.meeting_line.is_some(), "{snap:?}");
}
