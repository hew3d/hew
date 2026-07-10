//! Regression (found by adversarial review of the boundary-graze checks): a
//! straight chord across a concave face can exit through an off-center notch
//! and re-enter. Its midpoint sits in the wide solid part, no boundary vertex
//! lies on it, and a two-point path has no interior points — so point
//! sampling alone accepted it, committing two faces that cover the notch
//! region where the solid has no material. `split_face` must refuse it: a cut
//! segment may not cross the outer boundary anywhere but its anchors.

use kernel::{Object, Plane, Point3, Profile, StickyError, Vec3};

#[test]
fn chord_across_concave_notch_is_refused() {
    // U-shaped profile: a 10 x 5 rectangle with a 1-wide notch cut down from
    // the top edge between x=8 and x=9, extruded to a solid.
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 0.0, 0.0),
            Point3::new(10.0, 5.0, 0.0),
            Point3::new(9.0, 5.0, 0.0),
            Point3::new(9.0, 1.0, 0.0),
            Point3::new(8.0, 1.0, 0.0),
            Point3::new(8.0, 5.0, 0.0),
            Point3::new(0.0, 5.0, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let mut obj = Object::from_extrusion(&profile, 1.0).unwrap();

    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().dot(Vec3::new(0.0, 0.0, 1.0)) > 0.9)
        .map(|(id, _)| id)
        .expect("top face");

    // The chord spans the U's mouth at y=3: outside the face between x=8 and
    // x=9, crossing two notch side edges mid-edge.
    let err = obj
        .split_face(
            top,
            &[Point3::new(0.0, 3.0, 1.0), Point3::new(10.0, 3.0, 1.0)],
        )
        .unwrap_err();
    assert_eq!(err, StickyError::PathNotSimple);
    obj.validate().expect("refused cut leaves the object valid");

    // A chord that stays within the solid part of the U is still accepted.
    obj.split_face(
        top,
        &[Point3::new(0.0, 0.5, 1.0), Point3::new(10.0, 0.5, 1.0)],
    )
    .expect("chord clear of the notch splits normally");
    obj.validate().expect("valid after legitimate split");
}
