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
fn push_pull_requires_watertight() {
    let mut open = Object::triangle();
    let face = open.faces().keys().next().unwrap();
    assert_eq!(
        open.push_pull(face, 1.0).unwrap_err(),
        PushPullError::ObjectNotSolid
    );
}

// ------------------------------------------------------------- push/pull unit tests

/// Inward partial push on a unit cube: the cube shrinks, remains 6 faces,
/// and the maximum z is reduced by the push amount.
#[test]
fn inward_partial_push_keeps_six_faces_and_reduces_extent() {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));

    // Push the top face inward by 0.3 (less than the full 1.0 extent).
    let report = cube.push_pull(top, -0.3).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert_eq!(cube.faces().len(), 6, "inward push must keep 6 faces");

    // The moved face is still present (same handle — translate mode).
    assert!(cube.faces().contains_key(report.face));

    // The maximum z should now be 0.7 (1.0 - 0.3).
    let max_z = cube
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MIN, f64::max);
    assert!(
        (max_z - 0.7_f64).abs() <= tol::POINT_MERGE,
        "max z after -0.3 push should be 0.7, got {max_z}"
    );

    // The minimum z is unchanged at 0.0.
    let min_z = cube
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MAX, f64::min);
    assert!(
        min_z.abs() <= tol::POINT_MERGE,
        "min z must remain 0.0, got {min_z}"
    );
}

/// A right-triangle prism has a hypotenuse wall whose normal is not
/// perpendicular to the front wall's normal.  Pushing the front wall must
/// return NonManifoldResult because the eligibility dot-product check fires.
///
/// Construction: extrude a right-triangle profile in XY by 1.0 along +z.
/// The triangle outer vertices are (0,0,0), (1,0,0), (0,1,0).
/// The resulting prism has 5 faces:
///   - bottom cap (normal ≈ -z)
///   - top cap    (normal ≈ +z)
///   - front wall (normal ≈ -y, edge (0,0)-(1,0))
///   - right wall (normal ≈ +x, edge (1,0)-(0,1)) — this IS the hypotenuse wall
///   - left wall  (normal ≈ -x, edge (0,1)-(0,0))
///
/// The front wall (normal -y) shares edges with the bottom cap, top cap,
/// hypotenuse wall (normal ≈ (1/√2, 1/√2, 0)), and left wall.
/// Dot product of -y with hypotenuse normal ≈ -1/√2, |dot| ≈ 0.707 >> tol::NORMAL_DIRECTION.
/// Therefore eligibility check must fire and return NonManifoldResult.
#[test]
fn push_pull_slanted_neighbor_returns_non_manifold_result() {
    use kernel::Profile;

    let plane = xy_plane();
    let right_triangle_profile = Profile::new(
        plane,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
        vec![],
    )
    .unwrap();

    let prism = Object::from_extrusion(&right_triangle_profile, 1.0).unwrap();
    prism.validate().unwrap();
    assert_eq!(prism.watertight(), WatertightState::Watertight);
    assert_eq!(prism.faces().len(), 5, "right-triangle prism has 5 faces");

    // Find the front wall: normal ≈ (0, -1, 0).
    let front_wall = face_with_normal(&prism, Vec3::new(0.0, -1.0, 0.0));

    // Verify the hypotenuse-wall neighbor exists (normal ≈ (1/√2, 1/√2, 0)).
    let hyp_normal = Vec3::new(
        1.0_f64 / std::f64::consts::SQRT_2,
        1.0_f64 / std::f64::consts::SQRT_2,
        0.0,
    );
    let _hyp_wall = face_with_normal(&prism, hyp_normal);

    // Confirm the dot product between the front normal and hypotenuse normal
    // actually violates the perpendicularity check.
    let front_normal = Vec3::new(0.0, -1.0, 0.0);
    let dot = front_normal.dot(hyp_normal).abs();
    assert!(
        dot >= tol::NORMAL_DIRECTION,
        "hypotenuse wall must fail the perpendicularity check (dot={dot})"
    );

    // The push must be refused with NonManifoldResult.
    let mut prism_mut = prism;
    let err = prism_mut.push_pull(front_wall, 0.5).unwrap_err();
    assert_eq!(
        err,
        PushPullError::NonManifoldResult,
        "pushing front wall of a prism (slanted hypotenuse neighbor) must return NonManifoldResult"
    );
    // Strong guarantee: prism is unchanged.
    prism_mut.validate().unwrap();
    assert_eq!(prism_mut.faces().len(), 5);
}

#[test]
fn push_past_an_interior_step_is_refused() {
    // L-shaped prism: pushing the outer x=2 wall inward past the interior
    // step at x=1 would fold the step wall past its fixed vertices into a
    // self-intersecting shell — every face stays planar and manifold, so
    // only the obstruction guard can refuse it.
    let l_profile = Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let original = Object::from_extrusion(&l_profile, 1.0).unwrap();
    // Two walls face +x (outer at x=2, upper arm at x=1); pick the outer one.
    let wall = original
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(1.0, 0.0, 0.0), tol::NORMAL_DIRECTION)
                && original
                    .loop_positions(f.outer_loop)
                    .any(|p| (p.x - 2.0).abs() <= tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .expect("outer +x wall exists");

    // Shrinking short of the step is a legitimate push.
    let mut shrunk = original.clone();
    shrunk.push_pull(wall, -0.5).unwrap();
    shrunk.validate().unwrap();
    assert_eq!(shrunk.faces().len(), original.faces().len());

    // At the step or past it: refused, object untouched (strong guarantee).
    for depth in [-1.0, -1.5] {
        let mut obj = original.clone();
        assert_eq!(
            obj.push_pull(wall, depth).unwrap_err(),
            PushPullError::NonManifoldResult,
            "depth {depth}"
        );
        assert!(objects_equivalent(&obj, &original));
    }
}

// ------------------------------------------------- sticky split / merge

#[test]
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
fn merge_faces_rejects_non_coplanar() {
    let mut cube = unit_cube();
    // Every edge of a cube separates two perpendicular faces.
    let some_edge = cube.edges().keys().next().unwrap();
    assert_eq!(
        cube.merge_faces(some_edge).unwrap_err(),
        kernel::StickyError::FacesNotCoplanar
    );
}

proptest! {
    /// The identity must hold wherever the cut lands, not just at the
    /// midline: off-center cuts stress the boundary-healing tolerance
    /// logic with unequal fragment lengths.
    #[test]
    fn split_then_merge_is_identity_anywhere(cut_at in 0.05..0.95f64) {
        let original = unit_cube();
        let mut cube = original.clone();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let path = [Point3::new(cut_at, 0.0, 1.0), Point3::new(cut_at, 1.0, 1.0)];
        let split = cube.split_face(top, &path).unwrap();
        cube.merge_faces(split.new_edges[0]).unwrap();
        cube.validate().unwrap();
        prop_assert!(objects_equivalent(&cube, &original));
    }
}

// --------------------------------------------------------------- boolean

proptest! {
    #[test]
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

proptest! {
    /// Boolean volumes obey set algebra: |A∪B| + |A∩B| = |A| + |B|, and
    /// |A−B| = |A| − |A∩B|. This catches a watertight-but-wrong result that the
    /// watertightness property alone would pass.
    #[test]
    fn boolean_volumes_obey_set_algebra(
        dx in 0.2..0.8f64,
        dy in 0.2..0.8f64,
        dz in 0.2..0.8f64,
    ) {
        let a = unit_cube();
        let b = unit_cube();
        let shift = Transform::translation(Vec3::new(dx, dy, dz));
        let vol = |op| signed_volume(&Object::boolean(op, &a, &b, &shift).unwrap());
        let (vu, vi, vs) = (vol(BooleanOp::Union), vol(BooleanOp::Intersect), vol(BooleanOp::Subtract));

        // The overlap is the box where the two unit cubes meet.
        let overlap = (1.0 - dx) * (1.0 - dy) * (1.0 - dz);
        prop_assert!((vi - overlap).abs() < VOLUME_TOL, "intersect {vi} vs {overlap}");
        prop_assert!((vu + vi - 2.0).abs() < VOLUME_TOL, "union {vu} + intersect {vi} != 2");
        prop_assert!((vs - (1.0 - vi)).abs() < VOLUME_TOL, "subtract {vs} != 1 - {vi}");
    }
}

#[test]
fn subtract_can_split_a_solid_into_multiple_shells() {
    // A bar (volume 3) with a slab bitten out of its middle third becomes two
    // disconnected unit cubes: one watertight Object, two shells.
    let bar = box_object(Point3::ORIGIN, Point3::new(3.0, 1.0, 1.0));
    let cutter = box_object(Point3::new(1.0, -0.5, -0.5), Point3::new(2.0, 1.5, 1.5));
    let result = Object::boolean(BooleanOp::Subtract, &bar, &cutter, &Transform::IDENTITY).unwrap();
    result.validate().unwrap();
    assert_eq!(result.watertight(), WatertightState::Watertight);
    assert_eq!(result.shells().len(), 2);
    assert!((signed_volume(&result) - 2.0).abs() < VOLUME_TOL);
}

#[test]
fn identical_operands_resolve_by_set_algebra() {
    // A ∪ A ≡ A, A ∩ A ≡ A, A − A is empty — every face pair is coincident, now
    // resolved coplanar contact (ARCHITECTURE.md #19, extends #15).
    let a = unit_cube();
    let u = Object::boolean(BooleanOp::Union, &a, &a, &Transform::IDENTITY).unwrap();
    u.validate().unwrap();
    assert!((signed_volume(&u) - 1.0).abs() < VOLUME_TOL);
    let i = Object::boolean(BooleanOp::Intersect, &a, &a, &Transform::IDENTITY).unwrap();
    i.validate().unwrap();
    assert!((signed_volume(&i) - 1.0).abs() < VOLUME_TOL);
    assert_eq!(
        Object::boolean(BooleanOp::Subtract, &a, &a, &Transform::IDENTITY).unwrap_err(),
        BooleanError::EmptyResult
    );
}

proptest! {
    /// Coplanar set algebra: force exactly one axis to share a plane (the other
    /// two offset), so every result has coplanar contact. Volumes must still obey
    /// |A∪B|+|A∩B|=|A|+|B| and |A−B|=|A|−|A∩B|, and every result stays watertight.
    #[test]
    fn coplanar_booleans_obey_set_algebra(
        d0 in 0.2..0.8f64,
        d1 in 0.2..0.8f64,
        shared_axis in 0..3usize,
    ) {
        let mut s = [0.0; 3];
        let offset_axes: Vec<usize> = (0..3).filter(|&i| i != shared_axis).collect();
        s[offset_axes[0]] = d0;
        s[offset_axes[1]] = d1;
        let shift = Transform::translation(Vec3::new(s[0], s[1], s[2]));
        let a = unit_cube();
        let b = unit_cube();

        let solve = |op| {
            let r = Object::boolean(op, &a, &b, &shift).unwrap();
            r.validate().unwrap();
            assert_eq!(r.watertight(), WatertightState::Watertight);
            signed_volume(&r)
        };
        let (vu, vi, vs) = (
            solve(BooleanOp::Union),
            solve(BooleanOp::Intersect),
            solve(BooleanOp::Subtract),
        );

        // Shared axis contributes (1 - 0) = 1 to the overlap.
        let overlap = (1.0 - s[0]) * (1.0 - s[1]) * (1.0 - s[2]);
        prop_assert!((vi - overlap).abs() < VOLUME_TOL, "intersect {vi} vs {overlap}");
        prop_assert!((vu + vi - 2.0).abs() < VOLUME_TOL, "union {vu} + intersect {vi} != 2");
        prop_assert!((vs - (1.0 - vi)).abs() < VOLUME_TOL, "subtract {vs} != 1 - {vi}");
    }
}

// --------------------------------------------------------------- history

#[test]
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
