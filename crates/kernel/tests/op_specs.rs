//! Executable specs for the stubbed kernel operations (DEVELOPMENT.md rule 3).
//!
//! Every test here is `#[ignore]`d because the op it exercises is a `todo!()`
//! stub. The contract for implementers (see docs/DEVELOPMENT.md):
//!
//! - Un-ignore a test in the SAME PR that implements its operation.
//! - Never weaken an assertion or delete a test to make it pass (rule 5);
//!   if a spec looks wrong, escalate — these assertions are the acceptance
//!   criteria, reviewed separately from the implementation.
//!
//! Equality between Objects is judged by `objects_equivalent`: same multiset
//! of faces as cyclically-matching position lists (winding preserved), within
//! `tol::POINT_MERGE`. Handles are allowed to differ — ops may rebuild
//! elements — but topology and geometry are not.

use kernel::{
    BooleanError, BooleanOp, FaceId, History, KernelOp, Object, Plane, Point3, Profile,
    PushPullError, Transform, Vec3, WatertightState, tol,
};
use proptest::prelude::*;

// ---------------------------------------------------------------- helpers

fn xy_plane() -> Plane {
    Plane::from_polygon(&[
        Point3::ORIGIN,
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

fn rect_profile(width: f64, height: f64) -> Profile {
    Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(width, 0.0, 0.0),
            Point3::new(width, height, 0.0),
            Point3::new(0.0, height, 0.0),
        ],
        vec![],
    )
    .unwrap()
}

/// 4x4 square with a centered 2x2 hole (hole wound CW seen from +z).
fn washer_profile() -> Profile {
    Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 4.0, 0.0),
            Point3::new(0.0, 4.0, 0.0),
        ],
        vec![vec![
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(3.0, 1.0, 0.0),
        ]],
    )
    .unwrap()
}

/// Axis-aligned box as a watertight Object.
fn box_object(min: Point3, max: Point3) -> Object {
    let (a, b) = (min, max);
    Object::from_polygons(
        &[
            Point3::new(a.x, a.y, a.z),
            Point3::new(b.x, a.y, a.z),
            Point3::new(b.x, b.y, a.z),
            Point3::new(a.x, b.y, a.z),
            Point3::new(a.x, a.y, b.z),
            Point3::new(b.x, a.y, b.z),
            Point3::new(b.x, b.y, b.z),
            Point3::new(a.x, b.y, b.z),
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

fn unit_cube() -> Object {
    box_object(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0))
}

/// The unique face whose plane normal matches `dir`.
fn face_with_normal(obj: &Object, dir: Vec3) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION))
        .map(|(id, _)| id)
        .expect("object has a face with the requested normal")
}

fn polygons_of(obj: &Object) -> Vec<Vec<Point3>> {
    let (points, faces) = obj.to_polygons();
    faces
        .into_iter()
        .map(|poly| poly.into_iter().map(|i| points[i]).collect())
        .collect()
}

/// Same cycle of positions, any starting corner, same winding.
fn cyclic_match(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && (0..a.len()).any(|shift| {
            a.iter()
                .enumerate()
                .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], tol::POINT_MERGE))
        })
}

/// Topology + geometry equality up to handle renaming (see module docs).
fn objects_equivalent(x: &Object, y: &Object) -> bool {
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

/// Euler–Poincaré left side: `V - E + F - H` (H = inner loops across faces);
/// must equal `2(S - G)`.
fn euler_poincare(obj: &Object) -> i64 {
    let holes: usize = obj.faces().values().map(|f| f.inner_loops.len()).sum();
    obj.vertices().len() as i64 - obj.edges().len() as i64 + obj.faces().len() as i64 - holes as i64
}

/// Divergence-theorem signed volume over fan-triangulated outer loops
/// (hole-free objects only). Positive iff faces wind outward — twin pairing
/// and the validator cannot detect a globally inside-out solid, this can.
fn signed_volume(obj: &Object) -> f64 {
    let mut six_v = 0.0;
    for poly in polygons_of(obj) {
        for i in 1..poly.len() - 1 {
            let (a, b, c) = (poly[0], poly[i], poly[i + 1]);
            six_v += a.to_vec().dot(b.to_vec().cross(c.to_vec()));
        }
    }
    six_v / 6.0
}

/// Test-only slack for volume comparisons (m^3); generous against f64
/// accumulation over fan sums, far tighter than any real volume.
const VOLUME_TOL: f64 = 1e-6;

// ------------------------------------------------------------- extrusion

proptest! {
    #[test]
    fn extruded_rectangle_is_a_watertight_box(
        width in 0.1..10.0f64,
        height in 0.1..10.0f64,
        distance in 0.1..10.0f64,
    ) {
        let solid = Object::from_extrusion(&rect_profile(width, height), distance).unwrap();
        solid.validate().unwrap();
        prop_assert_eq!(solid.watertight(), WatertightState::Watertight);
        prop_assert_eq!(solid.faces().len(), 6);
        prop_assert_eq!(euler_poincare(&solid), 2); // S=1, G=0
        // Outward orientation and exact size in one check.
        let volume = signed_volume(&solid);
        prop_assert!(
            (volume - width * height * distance).abs() <= VOLUME_TOL,
            "signed volume {volume}, expected {}", width * height * distance
        );
    }

    #[test]
    fn extrusion_direction_follows_sign(distance in 0.1..5.0f64) {
        let up = Object::from_extrusion(&rect_profile(1.0, 1.0), distance).unwrap();
        let down = Object::from_extrusion(&rect_profile(1.0, 1.0), -distance).unwrap();
        let max_z_up = up.vertices().values().map(|v| v.position.z).fold(f64::MIN, f64::max);
        let min_z_down = down.vertices().values().map(|v| v.position.z).fold(f64::MAX, f64::min);
        prop_assert!((max_z_up - distance).abs() <= tol::POINT_MERGE);
        prop_assert!((min_z_down + distance).abs() <= tol::POINT_MERGE);
        // BOTH directions must produce outward-wound (positive-volume)
        // solids; extruding downward must not yield an inside-out box.
        prop_assert!((signed_volume(&up) - distance).abs() <= VOLUME_TOL);
        prop_assert!((signed_volume(&down) - distance).abs() <= VOLUME_TOL);
    }
}

#[test]
fn extruded_washer_is_genus_one() {
    let solid = Object::from_extrusion(&washer_profile(), 2.0).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // 2 annulus caps + 4 outer walls + 4 hole walls.
    assert_eq!(solid.faces().len(), 10);
    let cap_holes: usize = solid.faces().values().map(|f| f.inner_loops.len()).sum();
    assert_eq!(cap_holes, 2);
    // Euler–Poincaré: V - E + F - H = 2(S - G) = 0 for one genus-1 shell.
    assert_eq!(euler_poincare(&solid), 0);
}

#[test]
fn extrusion_rejects_tiny_distance() {
    let err = Object::from_extrusion(&rect_profile(1.0, 1.0), tol::POINT_MERGE / 2.0).unwrap_err();
    assert_eq!(err, kernel::ExtrudeError::DistanceTooSmall);
}

// ------------------------------------------------------------- push/pull

proptest! {
    #[test]
    #[ignore = "spec for Object::push_pull: the inverse property (DEVELOPMENT.md rule 3)"]
    fn push_pull_then_inverse_is_identity(distance in 0.1..3.0f64) {
        let original = unit_cube();
        let mut cube = original.clone();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let report = cube.push_pull(top, distance).unwrap();
        cube.push_pull(report.face, -distance).unwrap();
        cube.validate().unwrap();
        prop_assert!(objects_equivalent(&cube, &original));
    }

    #[test]
    #[ignore = "spec for Object::push_pull: coplanar side walls merge"]
    fn pulling_a_box_face_keeps_six_faces(distance in 0.1..3.0f64) {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        cube.push_pull(top, distance).unwrap();
        cube.validate().unwrap();
        prop_assert_eq!(cube.watertight(), WatertightState::Watertight);
        // Side walls land coplanar with the existing sides and must merge:
        // a taller box, not a box with a box on top.
        prop_assert_eq!(cube.faces().len(), 6);
        let max_z = cube.vertices().values().map(|v| v.position.z).fold(f64::MIN, f64::max);
        prop_assert!((max_z - (1.0 + distance)).abs() <= tol::POINT_MERGE);
    }
}

#[test]
#[ignore = "spec for Object::push_pull: removing all material is refused"]
fn push_through_entire_solid_is_refused() {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let err = cube.push_pull(top, -1.0).unwrap_err();
    assert_eq!(err, PushPullError::WouldVanish);
    // Strong guarantee: the failed op left the cube untouched.
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &unit_cube()));
}

#[test]
#[ignore = "spec for Object::push_pull: open objects are rejected"]
fn push_pull_requires_watertight() {
    let mut open = Object::triangle();
    let face = open.faces().keys().next().unwrap();
    assert_eq!(
        open.push_pull(face, 1.0).unwrap_err(),
        PushPullError::ObjectNotSolid
    );
}

// ------------------------------------------------- sticky split / merge

#[test]
#[ignore = "spec for Object::split_face + merge_faces: split + re-merge is identity (DEVELOPMENT.md rule 3)"]
fn split_then_merge_is_identity() {
    let original = unit_cube();
    let mut cube = original.clone();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    // Cut the top face down the middle, edge midpoint to edge midpoint.
    let path = [Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
    let split = cube.split_face(top, &path).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.faces().len(), 7);
    assert_eq!(cube.watertight(), WatertightState::Watertight);

    cube.merge_faces(split.new_edges[0]).unwrap();
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &original));
}

#[test]
#[ignore = "spec for Object::split_face: both endpoints must reach the boundary (no dangling edges)"]
fn split_face_rejects_interior_endpoint() {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    // Second endpoint stops in the middle of the face: a dangling edge,
    // which the kernel refuses by design (see ops module docs).
    let path = [Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 0.5, 1.0)];
    let err = cube.split_face(top, &path).unwrap_err();
    assert_eq!(err, kernel::StickyError::EndpointNotOnBoundary { which: 1 });
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &unit_cube()));
}

#[test]
#[ignore = "spec for Object::merge_faces: non-coplanar faces never merge"]
fn merge_faces_rejects_non_coplanar() {
    let mut cube = unit_cube();
    // Every edge of a cube separates two perpendicular faces.
    let some_edge = cube.edges().keys().next().unwrap();
    assert_eq!(
        cube.merge_faces(some_edge).unwrap_err(),
        kernel::StickyError::FacesNotCoplanar
    );
}

// --------------------------------------------------------------- boolean

proptest! {
    #[test]
    #[ignore = "spec for Object::boolean: all ops on overlapping solids give watertight results"]
    fn booleans_of_overlapping_cubes_are_watertight(
        dx in 0.2..0.8f64,
        dy in 0.2..0.8f64,
        dz in 0.2..0.8f64,
    ) {
        let a = unit_cube();
        let b = unit_cube();
        let shift = Transform::translation(Vec3::new(dx, dy, dz));
        for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
            let result = Object::boolean(op, &a, &b, &shift).unwrap();
            result.validate().unwrap();
            prop_assert_eq!(result.watertight(), WatertightState::Watertight);
        }
    }
}

#[test]
#[ignore = "spec for Object::boolean: empty results are typed errors, not empty Objects"]
fn boolean_empty_results_are_errors() {
    let small = unit_cube();
    let big = box_object(Point3::new(-1.0, -1.0, -1.0), Point3::new(2.0, 2.0, 2.0));
    // Subtracting an enclosing solid removes everything.
    assert_eq!(
        Object::boolean(BooleanOp::Subtract, &small, &big, &Transform::IDENTITY).unwrap_err(),
        BooleanError::EmptyResult
    );
    // Intersecting disjoint solids yields nothing.
    let far = Transform::translation(Vec3::new(5.0, 5.0, 5.0));
    assert_eq!(
        Object::boolean(BooleanOp::Intersect, &small, &small, &far).unwrap_err(),
        BooleanError::EmptyResult
    );
}

#[test]
#[ignore = "spec for Object::boolean: open operands are rejected"]
fn boolean_requires_watertight_operands() {
    let solid = unit_cube();
    let open = Object::triangle();
    let err = Object::boolean(BooleanOp::Union, &solid, &open, &Transform::IDENTITY).unwrap_err();
    assert_eq!(
        err,
        BooleanError::OperandNotSolid {
            which: kernel::Operand::B
        }
    );
}

// --------------------------------------------------------------- history

#[test]
#[ignore = "spec for History: undo restores topology and geometry; redo restores the result"]
fn history_undo_redo_roundtrip() {
    let original = unit_cube();
    let mut cube = original.clone();
    let mut history = History::new();

    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    history
        .apply(
            &mut cube,
            KernelOp::PushPull {
                face: top,
                distance: 0.7,
            },
        )
        .unwrap();
    let after = cube.clone();
    assert!(!objects_equivalent(&cube, &original));
    assert!(history.can_undo());

    history.undo(&mut cube).unwrap();
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &original));
    assert!(history.can_redo());

    history.redo(&mut cube).unwrap();
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &after));
}

// ------------------------------------------------------------- serialize

#[test]
#[ignore = "spec for Object::encode/decode: roundtrip identity and deterministic bytes"]
fn serialize_roundtrip_is_identity_and_deterministic() {
    for original in [unit_cube(), Object::tetrahedron(), Object::triangle()] {
        let bytes = original.encode();
        assert_eq!(bytes, original.encode(), "encode must be deterministic");
        let decoded = Object::decode(&bytes).unwrap();
        decoded.validate().unwrap();
        assert!(objects_equivalent(&decoded, &original));
        assert_eq!(decoded.watertight(), original.watertight());
    }
}

#[test]
#[ignore = "spec for Object::decode: garbage is rejected with a typed error, never repaired"]
fn decode_rejects_garbage() {
    assert!(Object::decode(&[]).is_err());
    assert!(Object::decode(&[0x00, 0x01, 0x02, 0x03]).is_err());
}
