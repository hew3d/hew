//! Executable specs for `InferenceScene` (DEVELOPMENT.md rule 3). Same rules as
//! the kernel's `op_specs.rs`: `#[ignore]`d until implemented, un-ignored in
//! the implementing PR, never weakened.
//!
//! Geometry under test: the kernel's unit-cube Object placed at identity.
//! `ObjectId::default()` (the null key) is a legitimate tag here — the scene
//! treats ids as opaque labels.

use inference::{InferenceScene, PickRay, Snap, SnapKind, SnapLock, SnapQuery};
use kernel::{Object, ObjectId, Point3, Transform, Vec3, tol};

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
#[ignore = "spec for InferenceScene::resolve: endpoint snap lands exactly on the vertex"]
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
#[ignore = "spec for InferenceScene::resolve: priority — endpoint beats everything in the cone"]
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
#[ignore = "spec for InferenceScene::resolve: edge midpoints rank above on-edge"]
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
#[ignore = "spec for InferenceScene::resolve: a face interior yields an on-face snap on its plane"]
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
#[ignore = "spec for InferenceScene::resolve: empty cone over empty space returns None"]
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
#[ignore = "spec for InferenceScene::resolve: an axis lock projects the result onto the locked line"]
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
#[ignore = "spec for InferenceScene::add_object/remove_object: removal is complete and idempotent"]
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
#[ignore = "spec for InferenceScene::add_object: re-adding an id replaces, never duplicates"]
fn re_adding_an_object_replaces_its_candidates() {
    let mut scene = cube_scene();
    let first = scene.candidate_counts();
    scene.add_object(ObjectId::default(), &unit_cube(), &Transform::IDENTITY);
    assert_eq!(scene.candidate_counts(), first, "same object, same counts");
}

#[test]
#[ignore = "spec for InferenceScene::add_object: placement transforms candidates into world space"]
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
