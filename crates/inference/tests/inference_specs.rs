//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{ElementRef, InferenceScene, PickRay, Snap, SnapKind, SnapLock, SnapQuery};
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
    let source = scene.pick_face(&ray).expect("ray crosses the cube faces");
    match source.element {
        ElementRef::Face(_) => {}
        other => panic!("expected a face, got {other:?}"),
    }
    // It is a top-face pick: re-querying from below must instead pick the
    // bottom face (different element), proving "nearest" is honored.
    let from_below = ray_at(Point3::new(0.5, 0.5, -3.0), Point3::new(0.5, 0.5, 0.0));
    let below = scene
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
    // free no-op that spares the instanced candidates.
    let mut scene = InferenceScene::new();
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &unit_cube(),
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

    // Instances mirror the same bookkeeping: additive registration never
    // scans; removal scans once; the idempotent repeat is free.
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &cube,
        &Transform::IDENTITY,
    );
    assert_eq!(scene.removal_candidates_visited(), scan * 2);
    scene.remove_instance(InstanceId::default());
    assert_eq!(scene.removal_candidates_visited(), scan * 3);
    scene.remove_instance(InstanceId::default());
    assert_eq!(scene.removal_candidates_visited(), scan * 3);
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
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &cube,
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );
    assert_eq!(
        scene.candidate_counts(),
        (world.0 * 2, world.1 * 2, world.2 * 2),
        "instance candidates coexist with the world object's"
    );

    // Removing the instance leaves the world object untouched...
    scene.remove_instance(InstanceId::default());
    assert_eq!(scene.candidate_counts(), world);

    // ...and removing the world object leaves a re-added instance's candidates.
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &cube,
        &Transform::IDENTITY,
    );
    scene.remove_object(ObjectId::default());
    assert_eq!(
        scene.candidate_counts(),
        world,
        "remove_object spares instanced candidates sharing the label"
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
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &cube,
        &Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
    );
    let rotated = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.0, 10.0, 0.0)));
    scene.add_instance(InstanceId::default(), ObjectId::default(), &cube, &rotated);
    let squashed = Transform::scale(Vec3::new(0.5, 2.0, 1.5))
        .then(&Transform::translation(Vec3::new(10.0, 10.0, 0.0)));
    scene.add_instance(InstanceId::default(), ObjectId::default(), &cube, &squashed);
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

    // add_instance / remove_instance.
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &cube,
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
    let plain = unit_cube();
    for gx in 0..5 {
        for gy in 0..5 {
            scene.add_instance(
                InstanceId::default(),
                ObjectId::default(),
                &plain,
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
    for gx in 0..41 {
        for gy in 0..41 {
            scene.add_instance(
                InstanceId::default(),
                ObjectId::default(),
                &cube,
                &Transform::translation(Vec3::new(3.0 * gx as f64, 3.0 * gy as f64, 0.0)),
            );
        }
    }
    let total_faces = scene.candidate_counts().2 as u64;
    assert_eq!(total_faces, 41 * 41 * 6, "the grid registered 10k+ faces");

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
    scene.add_instance(
        InstanceId::default(),
        ObjectId::default(),
        &unit_cube(),
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
