//! Repro: subtracting a rotated N-gon prism whose facet geometry lands
//! *near* (but not exactly on) the target's face planes fails with
//! `DegenerateContact`, even though the contact is an ordinary transversal
//! cut a hair away from coincidence.
//!
//! A 90° Rodrigues rotation carries `cos(π/2) ≈ 6.1e-17` into every vertex,
//! so a facet edge that would sit exactly on the box's top plane lands
//! within ~1e-16 of it instead. Exactly-coincident contact is handled (see
//! `faceted_carrier_booleans.rs`); the near-coincident band must be too.
//! The failing offsets are scale-dependent (an absolute tolerance meets
//! representation noise), so both the meter-scale and true-centimeter-scale
//! variants are pinned here — this is the desk-organizer tutorial's bin
//! scoop at its real dimensions.

use std::f64::consts::FRAC_PI_2;

use kernel::{BooleanOp, Object, Plane, Point3, Profile, Transform, Vec3, WatertightState};
use proptest::prelude::*;

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

fn boxed(w: f64, d: f64, h: f64) -> Object {
    let plane = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let pts = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(w, 0.0, 0.0),
        Point3::new(w, d, 0.0),
        Point3::new(0.0, d, 0.0),
    ];
    let profile = Profile::new(plane, pts, vec![]).unwrap();
    Object::from_extrusion(&profile, h).unwrap()
}

/// A horizontal 24-gon cylinder (axis +X after a 90° rotation about Y),
/// translated so its axis sits on the box's front face plane at height `zc`.
fn lying_cylinder_transform(s: f64, zc: f64) -> Transform {
    Transform::rotation(Vec3::new(0.0, 1.0, 0.0), FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(-s, 5.0 * s, zc * s)))
}

/// The same lying cylinder, but positioned the way the app does it: the
/// rotation and the translation are *baked into the vertices* as two
/// successive `transform_object` commits, not composed into one matrix at
/// boolean time. The two paths round differently, and each has produced
/// `DegenerateContact` at offsets where the other is fine.
fn baked_lying_cylinder(s: f64, zc: f64) -> Object {
    let mut cyl = ngon_prism(Point3::ORIGIN, 2.0 * s, 24, 9.0 * s);
    cyl.apply_transform(&Transform::rotation(Vec3::new(0.0, 1.0, 0.0), FRAC_PI_2).unwrap())
        .unwrap();
    cyl.apply_transform(&Transform::translation(Vec3::new(-s, 5.0 * s, zc * s)))
        .unwrap();
    cyl
}

/// Every axis height in the sweep must subtract cleanly; before the fix,
/// meter scale failed at zc ∈ {6.0, 6.3}.
#[test]
fn rotated_prism_subtract_near_rim_meter_scale() {
    for zc in [5.8, 5.9, 6.0, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 6.7] {
        let a = boxed(7.0, 5.0, 6.0);
        let b = ngon_prism(Point3::ORIGIN, 2.0, 24, 9.0);
        let t = lying_cylinder_transform(1.0, zc);
        let r = Object::boolean(BooleanOp::Subtract, &a, &b, &t)
            .unwrap_or_else(|e| panic!("subtract failed at zc={zc}: {e:?}"));
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight, "zc={zc}");
    }
}

/// The same sweep at true centimeter scale with app-style baked transforms
/// (the getting-started tutorial's bin-scoop dimensions); before the fix
/// this path failed at zc ∈ {5.9, 6.5, 6.6, 6.7} — different offsets than
/// meter scale, betraying the absolute gate.
#[test]
fn rotated_prism_subtract_near_rim_centimeter_scale() {
    for zc in [5.8, 5.9, 6.0, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 6.7] {
        let s = 0.01;
        let a = boxed(7.0 * s, 5.0 * s, 6.0 * s);
        let b = baked_lying_cylinder(s, zc);
        let r = Object::boolean(BooleanOp::Subtract, &a, &b, &Transform::IDENTITY)
            .unwrap_or_else(|e| panic!("subtract failed at zc={zc} (cm): {e:?}"));
        r.validate().unwrap();
        assert_eq!(r.watertight(), WatertightState::Watertight, "zc={zc} (cm)");
    }
}

proptest! {
    /// Property: a lying 24-gon cylinder overlapping the box near its top rim
    /// subtracts cleanly at *every* axis height in the tangency band, at any
    /// scale and either transform path. The overlap always carries real
    /// volume (the axis stays below rim + radius), so no position in this
    /// family is a legitimate degenerate contact.
    #[test]
    fn rotated_prism_subtract_succeeds_across_tangency_band(
        zc in 4.5..7.5f64,
        scale_exp in -2i32..3,
        baked in proptest::bool::ANY,
    ) {
        let s = 10.0f64.powi(scale_exp);
        let a = boxed(7.0 * s, 5.0 * s, 6.0 * s);
        let r = if baked {
            let b = baked_lying_cylinder(s, zc);
            Object::boolean(BooleanOp::Subtract, &a, &b, &Transform::IDENTITY)
        } else {
            let b = ngon_prism(Point3::ORIGIN, 2.0 * s, 24, 9.0 * s);
            Object::boolean(BooleanOp::Subtract, &a, &b, &lying_cylinder_transform(s, zc))
        };
        let r = r.unwrap_or_else(|e| panic!("subtract failed at zc={zc} s={s} baked={baked}: {e:?}"));
        r.validate().unwrap();
        prop_assert_eq!(r.watertight(), WatertightState::Watertight);
    }
}

/// The tutorial's exact flow at real size: hollow the 7×5×6 cm bin first
/// (7 mm walls, 1 cm floor), then subtract the lying cylinder 5 mm above
/// the rim, both positioned by baked transforms as the app does. Failed
/// with `DegenerateContact` before the fix.
#[test]
fn tutorial_bin_scoop_after_hollow_centimeter_scale() {
    let s = 0.01;
    let block = boxed(7.0 * s, 5.0 * s, 6.0 * s);
    let cutter = boxed(5.6 * s, 3.6 * s, 6.5 * s);
    let hollow_t = Transform::translation(Vec3::new(0.7 * s, 0.7 * s, 1.0 * s));
    let bin = Object::boolean(BooleanOp::Subtract, &block, &cutter, &hollow_t).unwrap();

    let scoop = baked_lying_cylinder(s, 6.5);
    let r = Object::boolean(BooleanOp::Subtract, &bin, &scoop, &Transform::IDENTITY)
        .unwrap_or_else(|e| panic!("tutorial scoop failed: {e:?}"));
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
}

/// The boundary of the fix: an in-plane tangency whose result would be
/// *genuinely* non-manifold must still refuse. Subtracting a triangular
/// ridge whose apex edge lies exactly in the box's top face pinches the
/// result along that line — the groove's two walls and the top face's two
/// halves all meet at one edge, and the cross-section boundary visits the
/// apex twice. No manifold result exists, so `DegenerateContact` is the
/// contract (DEVELOPMENT.md rule 4: refuse, never repair), not a bug.
#[test]
fn pinched_subtract_still_refused() {
    let b = boxed(10.0, 10.0, 4.0);
    let apex = Point3::new(0.0, 5.0, 4.0);
    let right = Point3::new(0.0, 7.0, 1.0);
    let left = Point3::new(0.0, 3.0, 1.0);
    let plane = Plane::from_point_normal(apex, Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let profile = Profile::new(plane, vec![apex, right, left], vec![])
        .or_else(|_| Profile::new(plane, vec![apex, left, right], vec![]))
        .unwrap();
    let ridge = Object::from_extrusion(&profile, 10.0).unwrap();
    let r = Object::boolean(BooleanOp::Subtract, &b, &ridge, &Transform::IDENTITY);
    assert!(
        r.is_err(),
        "a pinched (non-manifold) result must be refused"
    );
}
