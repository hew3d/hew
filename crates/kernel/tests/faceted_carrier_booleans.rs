//! Evidence tests for the true-curves design: the boolean engine's
//! faceted carrier already covers curved-solid workloads — N-gon prism
//! ("faceted cylinder") booleans, mismatched facet counts, coplanar cap
//! contact, and through-hole drilling all produce validated watertight
//! results. The analytic-overlay path (path B in that document) rests on
//! this staying true; these tests keep the claim honest.

use kernel::{BooleanOp, Object, Plane, Point3, Profile, Transform, Vec3, WatertightState};

fn ngon_prism(center: Point3, radius: f64, n: usize, height: f64) -> Object {
    let plane = Plane::from_point_normal(center, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let pts: Vec<Point3> = (0..n)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            Point3::new(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
                center.z,
            )
        })
        .collect();
    let profile = Profile::new(plane, pts, vec![]).unwrap();
    Object::from_extrusion(&profile, height).unwrap()
}

#[test]
fn overlapping_prisms_boolean_all_ops() {
    let a = ngon_prism(Point3::ORIGIN, 1.0, 24, 1.0);
    let b = ngon_prism(Point3::ORIGIN, 1.0, 24, 1.0);
    let t = Transform::translation(Vec3::new(0.8, 0.0, 0.0));
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let r = Object::boolean(op, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight, "{op:?}");
    }
}

#[test]
fn mismatched_facet_counts_boolean() {
    let a = ngon_prism(Point3::ORIGIN, 1.0, 24, 1.0);
    let b = ngon_prism(Point3::ORIGIN, 0.7, 20, 2.0);
    let t = Transform::translation(Vec3::new(0.5, 0.3, -0.5));
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let r = Object::boolean(op, &a, &b, &t).unwrap();
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight, "{op:?}");
    }
}

#[test]
fn coaxial_stacked_prisms_union() {
    // Cylinder standing on a cylinder: coplanar cap contact, identical rims.
    let a = ngon_prism(Point3::ORIGIN, 1.0, 24, 1.0);
    let b = ngon_prism(Point3::ORIGIN, 1.0, 24, 1.0);
    let t = Transform::translation(Vec3::new(0.0, 0.0, 1.0));
    let r = Object::boolean(BooleanOp::Union, &a, &b, &t).unwrap();
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
}

#[test]
fn narrow_pin_through_wide_slab() {
    // Small-radius prism drilled through a big one: subtract makes a tunnel.
    let slab = ngon_prism(Point3::ORIGIN, 2.0, 24, 0.5);
    let pin = ngon_prism(Point3::new(0.3, 0.2, -1.0), 0.4, 16, 3.0);
    let r = Object::boolean(BooleanOp::Subtract, &slab, &pin, &Transform::IDENTITY).unwrap();
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
}
