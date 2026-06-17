//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{ElementRef, InferenceScene, PickRay, Snap, SnapKind, SnapLock, SnapQuery};
use kernel::{InstanceId, Object, ObjectId, Point3, Transform, Vec3, tol};

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
