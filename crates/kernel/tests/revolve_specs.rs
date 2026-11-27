//! Executable specs for Revolve and for Follow Me stamping the surfaces of
//! revolution it sweeps (true-curves.md §5.1; WP12).

use crate::common;
use kernel::trig;

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use common::curves::*;
use kernel::{
    BooleanOp, CurveGeom, Document, DocumentError, FollowMeError, Object, ObjectId, Plane, Point3,
    ShapeError, SketchId, SurfaceRef, Transform, Vec3,
    document::{RevolveProfile, RevolveSpec},
};

/// The XZ plane (normal −Y, so a profile drawn at x ≥ 0 sits beside the
/// +Z axis).
fn xz() -> Plane {
    Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap()
}

/// A Document holding one XZ sketch drawn by `draw`; returns it and the
/// sketch's only region.
fn sketch_doc(
    draw: impl FnOnce(&mut kernel::Sketch),
) -> (Document, SketchId, kernel::SketchRegionId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(xz());
    doc.begin_sketch_gesture(s).unwrap();
    draw(doc.sketch_mut(s).unwrap());
    doc.end_sketch_gesture(s).unwrap();
    let regions: Vec<_> = doc.sketch(s).unwrap().regions().keys().collect();
    assert_eq!(regions.len(), 1, "the profile closes one region");
    (doc, s, regions[0])
}

fn polygon(sk: &mut kernel::Sketch, pts: &[(f64, f64)]) {
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        sk.add_segment(p3(a.0, 0.0, a.1), p3(b.0, 0.0, b.1))
            .unwrap();
    }
}

/// An arc chain in the XZ plane about `(cx, cz)` from angle `a0` sweeping
/// `sweep` (angles measured from +X toward +Z), `n` facets.
fn arc(sk: &mut kernel::Sketch, cx: f64, cz: f64, r: f64, a0: f64, sweep: f64, n: usize) {
    sk.begin_curve_with(CurveGeom {
        center: p3(cx, 0.0, cz),
        radius: r,
    })
    .unwrap();
    let at = |k: usize| {
        let a = a0 + sweep * k as f64 / n as f64;
        p3(cx + r * trig::cos(a), 0.0, cz + r * trig::sin(a))
    };
    for k in 0..n {
        sk.add_segment(at(k), at(k + 1)).unwrap();
    }
    sk.end_curve();
}

fn z_axis(angle: f64, segments: u32) -> RevolveSpec {
    RevolveSpec {
        axis_point: p3(0.0, 0.0, 0.0),
        axis: Vec3::new(0.0, 0.0, 1.0),
        angle,
        segments,
    }
}

fn revolve_region(
    draw: impl FnOnce(&mut kernel::Sketch),
    spec: RevolveSpec,
) -> Result<(Document, ObjectId), DocumentError> {
    let (mut doc, s, r) = sketch_doc(draw);
    let (id, _) = doc.revolve(
        RevolveProfile::Region {
            sketch: s,
            region: r,
        },
        &spec,
    )?;
    Ok((doc, id))
}

fn claims(o: &Object) -> Vec<SurfaceRef> {
    o.faces().values().filter_map(|f| f.surface).collect()
}

#[test]
fn rectangle_on_axis_is_cylinder() {
    let (doc, id) = revolve_region(
        |sk| polygon(sk, &[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)]),
        z_axis(TAU, 48),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    let census = claim_census(o);
    assert_eq!(census.get("cylinder"), Some(&48), "{census:?}");
    for c in claims(o) {
        let SurfaceRef::Cylinder { radius, .. } = c else {
            panic!("{c:?}")
        };
        assert!((radius - 0.5).abs() <= 1e-9);
    }
    // Every unclaimed face is a cap facet, planar and horizontal.
    for f in o.faces().values().filter(|f| f.surface.is_none()) {
        assert!(f.plane.normal().z.abs() > 1.0 - 1e-9);
    }
    let v = signed_volume(o);
    let expected = PI * 0.25 * (48.0 / TAU * trig::sin(TAU / 48.0)); // inscribed 48-gon area × h
    assert!((v - expected).abs() <= 1e-9, "{v} vs {expected}");
}

#[test]
fn offset_rectangle_is_tube() {
    let (doc, id) = revolve_region(
        |sk| polygon(sk, &[(0.3, 0.0), (0.5, 0.0), (0.5, 1.0), (0.3, 1.0)]),
        z_axis(TAU, 48),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    let mut radii: Vec<f64> = claims(o)
        .iter()
        .map(|c| match *c {
            SurfaceRef::Cylinder { radius, .. } => radius,
            other => panic!("{other:?}"),
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    radii.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    assert_eq!(radii.len(), 2);
    assert!((radii[0] - 0.3).abs() <= 1e-9 && (radii[1] - 0.5).abs() <= 1e-9);
    // The annular top and bottom rings are planar and unclaimed.
    assert!(
        o.faces()
            .values()
            .filter(|f| f.surface.is_none())
            .all(|f| f.plane.normal().z.abs() > 1.0 - 1e-9)
    );
}

#[test]
fn triangle_is_cone() {
    let (doc, id) = revolve_region(
        |sk| polygon(sk, &[(0.0, 0.0), (0.5, 0.0), (0.0, 1.0)]),
        z_axis(TAU, 48),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    let cones: Vec<SurfaceRef> = claims(o);
    assert_eq!(cones.len(), 48);
    for c in &cones {
        let SurfaceRef::Cone {
            apex,
            axis,
            half_angle,
        } = *c
        else {
            panic!("{c:?}")
        };
        assert!(apex.approx_eq(p3(0.0, 0.0, 1.0), 1e-9), "apex on the axis");
        assert!(
            axis.approx_eq(Vec3::new(0.0, 0.0, -1.0), 1e-9),
            "opening downward"
        );
        assert!((half_angle - trig::atan(0.5f64)).abs() <= 1e-9);
        assert!(c.same_surface(&cones[0]));
    }
}

#[test]
fn semicircle_on_axis_is_sphere() {
    let (doc, id) = revolve_region(
        |sk| {
            arc(sk, 0.0, 0.0, 0.5, -FRAC_PI_2, PI, 12);
            sk.add_segment(p3(0.0, 0.0, 0.5), p3(0.0, 0.0, -0.5))
                .unwrap();
        },
        z_axis(TAU, 24),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    assert_eq!(
        claims(o).len(),
        o.faces().len(),
        "every facet claims the sphere, poles too"
    );
    let first = claims(o)[0];
    for c in claims(o) {
        assert!(matches!(c, SurfaceRef::Sphere { radius, .. } if (radius - 0.5).abs() <= 1e-9));
        assert!(c.same_surface(&first));
    }
    assert!(
        o.analytic_centers()
            .iter()
            .any(|(c, _)| c.approx_eq(p3(0.0, 0.0, 0.0), 1e-9))
    );
}

#[test]
fn circle_off_axis_is_torus() {
    let (doc, id) =
        revolve_region(|sk| arc(sk, 2.0, 0.0, 0.5, 0.0, TAU, 24), z_axis(TAU, 48)).unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    let all = claims(o);
    assert_eq!(all.len(), o.faces().len());
    for c in &all {
        let SurfaceRef::Torus {
            center,
            major_radius,
            minor_radius,
            ..
        } = *c
        else {
            panic!("{c:?}")
        };
        assert!(center.approx_eq(p3(0.0, 0.0, 0.0), 1e-9));
        assert!((major_radius - 2.0).abs() <= 1e-9 && (minor_radius - 0.5).abs() <= 1e-9);
    }
}

#[test]
fn goblet_mixed_profile() {
    // A vertical foot edge (cylinder), a slanted shoulder (cone), and an
    // arc bowl (torus), closed down the axis.
    let (doc, id) = revolve_region(
        |sk| {
            sk.add_segment(p3(0.0, 0.0, 0.0), p3(0.5, 0.0, 0.0))
                .unwrap();
            sk.add_segment(p3(0.5, 0.0, 0.0), p3(0.5, 0.0, 0.2))
                .unwrap();
            sk.add_segment(p3(0.5, 0.0, 0.2), p3(0.2, 0.0, 0.5))
                .unwrap();
            arc(sk, 0.2, 0.7, 0.2, -FRAC_PI_2, 1.5 * PI, 18);
            sk.add_segment(p3(0.0, 0.0, 0.7), p3(0.0, 0.0, 0.0))
                .unwrap();
        },
        z_axis(TAU, 48),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    let census = claim_census(o);
    assert_eq!(census.get("cylinder"), Some(&48), "{census:?}");
    assert_eq!(census.get("cone"), Some(&48), "{census:?}");
    assert_eq!(census.get("torus"), Some(&(48 * 18)), "{census:?}");
}

#[test]
fn partial_90_has_planar_end_caps() {
    let (doc, id) = revolve_region(
        |sk| polygon(sk, &[(0.3, 0.0), (0.5, 0.0), (0.5, 1.0), (0.3, 1.0)]),
        z_axis(FRAC_PI_2, 48),
    )
    .unwrap();
    let o = doc.object(id).unwrap();
    assert_valid_solid(o);
    // 12 segments × 4 walls + 2 end caps.
    assert_eq!(o.faces().len(), 12 * 4 + 2);
    let caps: Vec<_> = o
        .faces()
        .values()
        .filter(|f| f.surface.is_none() && f.plane.normal().z.abs() < 1e-9)
        .collect();
    assert_eq!(
        claim_census(o).get("cylinder"),
        Some(&24),
        "every wall band stays on its cylinder"
    );
    assert_eq!(caps.len(), 2, "two radial end caps");
    let mut normals: Vec<Vec3> = caps.iter().map(|f| f.plane.normal()).collect();
    normals.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert!(
        normals[0].dot(normals[1]).abs() < 1e-9,
        "the caps are a quarter turn apart"
    );
}

#[test]
fn axis_off_plane_refused() {
    let (mut doc, s, r) =
        sketch_doc(|sk| polygon(sk, &[(0.3, 0.0), (0.5, 0.0), (0.5, 1.0), (0.3, 1.0)]));
    let before = doc.state_hash();
    for spec in [
        RevolveSpec {
            axis: Vec3::new(1.0, 1.0, 0.0),
            ..z_axis(TAU, 48)
        },
        RevolveSpec {
            axis_point: p3(0.0, 0.1, 0.0),
            ..z_axis(TAU, 48)
        },
    ] {
        assert_eq!(
            doc.revolve(
                RevolveProfile::Region {
                    sketch: s,
                    region: r
                },
                &spec
            )
            .unwrap_err(),
            DocumentError::Shape(ShapeError::AxisNotInProfilePlane)
        );
    }
    for spec in [z_axis(0.0, 48), z_axis(7.0, 48), z_axis(TAU, 2)] {
        assert_eq!(
            doc.revolve(
                RevolveProfile::Region {
                    sketch: s,
                    region: r
                },
                &spec
            )
            .unwrap_err(),
            DocumentError::Shape(ShapeError::AmountOutOfRange)
        );
    }
    assert_eq!(doc.state_hash(), before);
}

#[test]
fn crossing_refused() {
    let err = revolve_region(
        |sk| polygon(sk, &[(-0.2, 0.0), (0.5, 0.0), (0.5, 1.0), (-0.2, 1.0)]),
        z_axis(TAU, 48),
    )
    .unwrap_err();
    assert_eq!(
        err,
        DocumentError::FollowMe(FollowMeError::ProfileCrossesAxis)
    );
}

#[test]
fn partial_turn_touching_the_axis_refused_as_partial_pole() {
    let err = revolve_region(
        |sk| polygon(sk, &[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)]),
        z_axis(FRAC_PI_2, 48),
    )
    .unwrap_err();
    assert_eq!(
        err,
        DocumentError::FollowMe(FollowMeError::PartialSweepOnPole)
    );
}

#[test]
fn follow_me_around_drawn_circle_stamps_torus() {
    // The existing Follow Me API with a drawn-circle path: a circle profile
    // in the XZ plane swept around a 24-facet ground circle.
    let profile = circle_profile(p3(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 0.25, 24);
    let n = 24;
    let path: Vec<Point3> = (0..n)
        .map(|i| {
            let a = TAU * i as f64 / n as f64;
            p3(trig::cos(a), trig::sin(a), 0.0)
        })
        .collect();
    let curves = vec![
        Some(CurveGeom {
            center: p3(0.0, 0.0, 0.0),
            radius: 1.0
        });
        n
    ];
    let ring = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    assert_valid_solid(&ring);
    let census = claim_census(&ring);
    assert_eq!(census.get("torus"), Some(&(24 * 24)), "{census:?}");
}

#[test]
fn revolve_then_boolean_keeps_claims_and_seams() {
    let (doc, id) = revolve_region(
        |sk| polygon(sk, &[(0.3, 0.0), (0.5, 0.0), (0.5, 1.0), (0.3, 1.0)]),
        z_axis(TAU, 48),
    )
    .unwrap();
    let tube = doc.object(id).unwrap().clone();
    let soft_before = tube.edges().values().filter(|e| e.soft).count();
    assert!(soft_before > 0, "the revolve's transverse joints are soft");
    let mut b = box_xyz(0.4, 0.4, 0.4);
    b.apply_transform(&Transform::translation(Vec3::new(0.3, 0.3, 0.8)))
        .unwrap();
    let r = Object::boolean(BooleanOp::Union, &tube, &b, &Transform::IDENTITY).unwrap();
    assert_valid_solid(&r);
    let census: BTreeMap<&str, usize> = claim_census(&r);
    assert!(
        census.get("cylinder").copied().unwrap_or(0) >= 48,
        "claims survive: {census:?}"
    );
    let soft_after = r.edges().values().filter(|e| e.soft).count();
    assert!(
        soft_after >= soft_before - 8,
        "seams survive ({soft_after} of {soft_before})"
    );
}
