//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{
    ElementRef, InferenceScene, PickRay, Snap, SnapKind, SnapLock, SnapQuery, SnapWeights,
};
use kernel::{
    Guide, GuideId, InstanceId, Object, ObjectId, Plane, Point3, SketchEdgeId, SketchId, Transform,
    Vec3, tol,
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
        constraint_plane: None,
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
        constraint_plane: None,
    };
    let snap = scene.resolve(&q).expect("lock with anchor always resolves");
    // On the X axis through the anchor:
    assert!(snap.position.y.abs() <= tol::POINT_MERGE);
    assert!(snap.position.z.abs() <= tol::POINT_MERGE);
    assert_eq!(snap.direction, Some(inference::Axis::X.unit()));
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
            constraint_plane: None,
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
    let segments: Vec<_> = s
        .edges()
        .iter()
        .map(|(eid, e)| {
            (
                eid,
                s.vertices()[e.from].position,
                s.vertices()[e.to].position,
            )
        })
        .collect();
    scene.add_sketch(id, &segments);
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

    // The exact center resolves as Center with no provenance (a sketch
    // curve is not a SnapSource element — like a guide, source is None).
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
    scene.add_sketch_polygon_centers(SketchId::default(), &[center]);

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
