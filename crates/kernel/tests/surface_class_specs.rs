//! Executable specs for the Sphere, Torus and Cone surface classes
//! (true-curves.md §3.1; fillets-chamfers.md §6.1): validator invariants,
//! `same_surface`, the per-class normal and distance math, map-or-drop
//! under transforms, Push/Pull routing, and center derivation.

mod common;

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use common::curves::*;
use kernel::{FaceId, Point3, SurfaceRef, Transform, Vec3, tol};

// ------------------------------------------------------- 1. validator

#[test]
fn valid_facets_of_every_class_validate() {
    for (o, f, s) in [
        sphere_facet(),
        torus_facet(),
        cone_facet(),
        cylinder_facet(),
    ] {
        with_claim(&o, f, s)
            .validate()
            .unwrap_or_else(|e| panic!("{s:?}: {e:?}"));
    }
}

#[test]
fn sphere_invariants_are_enforced() {
    let (o, f, _) = sphere_facet();
    let c = p3(0.0, 0.0, 0.0);
    // Non-finite radius.
    assert_mismatch(
        &o,
        f,
        SurfaceRef::Sphere {
            center: c,
            radius: f64::NAN,
        },
    );
    // Radius at or below POINT_MERGE.
    assert_mismatch(
        &o,
        f,
        SurfaceRef::Sphere {
            center: c,
            radius: tol::POINT_MERGE,
        },
    );
    // The plane farther than the radius from the center.
    assert_mismatch(
        &o,
        f,
        SurfaceRef::Sphere {
            center: p3(-5.0, 0.0, 0.0),
            radius: 1.0,
        },
    );
    // A vertex outside the sphere.
    assert_mismatch(
        &o,
        f,
        SurfaceRef::Sphere {
            center: c,
            radius: 0.9,
        },
    );
}

#[test]
fn torus_invariants_are_enforced() {
    let (o, f, _) = torus_facet();
    let c = p3(0.0, 0.0, 0.0);
    let torus = |axis: Vec3, major: f64, minor: f64| SurfaceRef::Torus {
        center: c,
        axis,
        major_radius: major,
        minor_radius: minor,
    };
    // Non-unit axis.
    assert_mismatch(&o, f, torus(Z * 2.0, 2.0, 0.5));
    // Non-finite radii.
    assert_mismatch(&o, f, torus(Z, f64::INFINITY, 0.5));
    assert_mismatch(&o, f, torus(Z, 2.0, f64::NAN));
    // Minor at or below POINT_MERGE.
    assert_mismatch(&o, f, torus(Z, 2.0, 0.0));
    // A zero-major torus is invalid (it must be a Sphere).
    assert_mismatch(&o, f, torus(Z, 0.0, 0.5));
    // A vertex outside the tube.
    assert_mismatch(&o, f, torus(Z, 2.0, 0.4));
}

#[test]
fn cone_invariants_are_enforced() {
    let (o, f, _) = cone_facet();
    let a = 30f64.to_radians();
    let cone = |apex: Point3, axis: Vec3, half_angle: f64| SurfaceRef::Cone {
        apex,
        axis,
        half_angle,
    };
    let origin = p3(0.0, 0.0, 0.0);
    // Non-unit axis.
    assert_mismatch(&o, f, cone(origin, Z * 2.0, a));
    // Half-angle out of (0, π/2).
    assert_mismatch(&o, f, cone(origin, Z, 0.0));
    assert_mismatch(&o, f, cone(origin, Z, FRAC_PI_2));
    assert_mismatch(&o, f, cone(origin, Z, f64::NAN));
    // The plane does not pass through the apex.
    assert_mismatch(&o, f, cone(p3(0.0, 0.0, -0.1), Z, a));
    // A vertex behind the apex (wrong nappe).
    assert_mismatch(&o, f, cone(origin, -Z, a));
    // A vertex outside the cone.
    assert_mismatch(&o, f, cone(origin, Z, 20f64.to_radians()));
}

// ---------------------------------------------------- 2. same_surface

/// Every field of `s` shifted by `d` (points per component, radii and the
/// half-angle by `d`, axes per component).
fn perturbed(s: SurfaceRef, d: f64) -> SurfaceRef {
    let pp = |p: Point3| p3(p.x + d, p.y + d, p.z + d);
    let pv = |v: Vec3| Vec3::new(v.x + d, v.y + d, v.z + d);
    match s {
        SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        } => SurfaceRef::Cylinder {
            axis_point: pp(axis_point),
            axis: pv(axis),
            radius: radius + d,
        },
        SurfaceRef::Sphere { center, radius } => SurfaceRef::Sphere {
            center: pp(center),
            radius: radius + d,
        },
        SurfaceRef::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } => SurfaceRef::Torus {
            center: pp(center),
            axis: pv(axis),
            major_radius: major_radius + d,
            minor_radius: minor_radius + d,
        },
        SurfaceRef::Cone {
            apex,
            axis,
            half_angle,
        } => SurfaceRef::Cone {
            apex: pp(apex),
            axis: pv(axis),
            half_angle: half_angle + d,
        },
    }
}

#[test]
fn same_surface_is_tolerance_aware_symmetric_and_class_strict() {
    let all = [
        sphere_facet().2,
        torus_facet().2,
        cone_facet().2,
        cylinder_facet().2,
    ];
    for &s in &all {
        let near = perturbed(s, 0.5 * tol::POINT_MERGE);
        let far = perturbed(s, 2.0 * tol::POINT_MERGE);
        assert!(s.same_surface(&near), "{s:?} vs half-tolerance copy");
        assert!(near.same_surface(&s), "symmetry");
        assert!(!s.same_surface(&far), "{s:?} vs twice-tolerance copy");
        assert!(!far.same_surface(&s), "symmetry");
        for &o in &all {
            assert_eq!(s.same_surface(&o), o.same_surface(&s), "symmetric");
            if s.kind_name() != o.kind_name() {
                assert!(!s.same_surface(&o), "different classes never agree");
            }
        }
    }
    // A cone's axis direction selects the nappe: reversed is a different surface.
    let cone = cone_facet().2;
    let SurfaceRef::Cone {
        apex, half_angle, ..
    } = cone
    else {
        panic!("expected cone")
    };
    assert!(!cone.same_surface(&SurfaceRef::Cone {
        apex,
        axis: -Z,
        half_angle
    }));
    // A torus's axis is a line: reversed is the same surface.
    let torus = torus_facet().2;
    let SurfaceRef::Torus {
        center,
        major_radius,
        minor_radius,
        ..
    } = torus
    else {
        panic!("expected torus")
    };
    assert!(torus.same_surface(&SurfaceRef::Torus {
        center,
        axis: -Z,
        major_radius,
        minor_radius
    }));
}

// --------------------------------------------- 3. normals and distances

const MATH_EPS: f64 = 1e-12;

fn assert_vec(a: Vec3, b: Vec3) {
    assert!(a.approx_eq(b, MATH_EPS), "{a:?} != {b:?}");
}

fn assert_f(a: f64, b: f64) {
    assert!((a - b).abs() <= MATH_EPS, "{a} != {b}");
}

#[test]
fn sphere_normal_and_distance() {
    let s = SurfaceRef::Sphere {
        center: p3(1.0, 2.0, 3.0),
        radius: 2.0,
    };
    let cases = [
        (p3(3.0, 2.0, 3.0), Vec3::new(1.0, 0.0, 0.0), 0.0),
        (p3(1.0, 2.0, 0.0), Vec3::new(0.0, 0.0, -1.0), 1.0),
        (p3(1.0, 3.0, 3.0), Vec3::new(0.0, 1.0, 0.0), -1.0),
        (p3(4.0, 6.0, 3.0), Vec3::new(0.6, 0.8, 0.0), 3.0),
        (p3(1.0, 2.0, 7.0), Vec3::new(0.0, 0.0, 1.0), 2.0),
    ];
    for (p, n, d) in cases {
        assert_vec(s.normal_at(p).unwrap(), n);
        assert_f(s.distance(p), d);
    }
    assert!(
        s.normal_at(p3(1.0, 2.0, 3.0)).is_none(),
        "undefined at the center"
    );
}

#[test]
fn torus_normal_and_distance() {
    let s = SurfaceRef::Torus {
        center: p3(0.0, 0.0, 0.0),
        axis: Z,
        major_radius: 2.0,
        minor_radius: 0.5,
    };
    let r2 = 0.5f64.sqrt();
    let cases = [
        (p3(2.5, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 0.0),
        (p3(1.5, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), 0.0),
        (p3(0.0, 2.0, 0.5), Vec3::new(0.0, 0.0, 1.0), 0.0),
        (p3(0.0, -2.0, -1.0), Vec3::new(0.0, 0.0, -1.0), 0.5),
        (p3(3.0 * r2, 3.0 * r2, 0.0), Vec3::new(r2, r2, 0.0), 0.5),
    ];
    for (p, n, d) in cases {
        assert_vec(s.normal_at(p).unwrap(), n);
        assert_f(s.distance(p), d);
    }
    assert!(
        s.normal_at(p3(0.0, 0.0, 1.0)).is_none(),
        "undefined on the axis"
    );
    assert!(
        s.normal_at(p3(2.0, 0.0, 0.0)).is_none(),
        "undefined on the center circle"
    );
}

#[test]
fn cone_normal_and_distance() {
    // α = 45°: generators at 45° to +Z through the origin.
    let s = SurfaceRef::Cone {
        apex: p3(0.0, 0.0, 0.0),
        axis: Z,
        half_angle: FRAC_PI_4,
    };
    let h = 0.5f64.sqrt();
    let cases = [
        (p3(1.0, 0.0, 1.0), Vec3::new(h, 0.0, -h), 0.0),
        (p3(0.0, 2.0, 2.0), Vec3::new(0.0, h, -h), 0.0),
        (p3(2.0, 0.0, 1.0), Vec3::new(h, 0.0, -h), h),
        (p3(0.0, -1.0, 2.0), Vec3::new(0.0, -h, -h), -h),
        (p3(-3.0, 0.0, 3.0), Vec3::new(-h, 0.0, -h), 0.0),
    ];
    for (p, n, d) in cases {
        assert_vec(s.normal_at(p).unwrap(), n);
        assert_f(s.distance(p), d);
    }
    assert!(
        s.normal_at(p3(0.0, 0.0, 1.0)).is_none(),
        "undefined on the axis"
    );
}

#[test]
fn cylinder_normal_and_distance() {
    let s = SurfaceRef::Cylinder {
        axis_point: p3(0.0, 0.0, 5.0),
        axis: Z,
        radius: 1.0,
    };
    let cases = [
        (p3(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), 0.0),
        (p3(0.0, 3.0, 9.0), Vec3::new(0.0, 1.0, 0.0), 2.0),
        (p3(-0.5, 0.0, -2.0), Vec3::new(-1.0, 0.0, 0.0), -0.5),
        (p3(0.6, 0.8, 1.0), Vec3::new(0.6, 0.8, 0.0), 0.0),
        (p3(0.0, -0.25, 5.0), Vec3::new(0.0, -1.0, 0.0), -0.75),
    ];
    for (p, n, d) in cases {
        assert_vec(s.normal_at(p).unwrap(), n);
        assert_f(s.distance(p), d);
    }
}

// --------------------------------------------------- 4. torus_or_sphere

#[test]
fn torus_or_sphere_degenerates_and_refuses() {
    let c = p3(0.0, 0.0, 0.0);
    assert_eq!(
        SurfaceRef::torus_or_sphere(c, Z, 1e-10, 0.5),
        Some(SurfaceRef::Sphere {
            center: c,
            radius: 0.5
        })
    );
    assert_eq!(SurfaceRef::torus_or_sphere(c, Z, -1e-6, 0.5), None);
    assert_eq!(SurfaceRef::torus_or_sphere(c, Z, 1.0, 0.0), None);
    assert!(matches!(
        SurfaceRef::torus_or_sphere(c, Z, 1.0, 0.5),
        Some(SurfaceRef::Torus { .. })
    ));
}

// -------------------------------------------------------- 5. mapping

#[test]
fn each_class_maps_under_similarity_and_drops_under_non_uniform_scale() {
    let similarity = Transform::rotation(Vec3::new(1.0, 1.0, 0.0), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(0.3, -2.0, 1.5)))
        .then(&Transform::uniform_scale(2.0));
    for (o, f, s) in [
        sphere_facet(),
        torus_facet(),
        cone_facet(),
        cylinder_facet(),
    ] {
        let claimed = with_claim(&o, f, s);
        let expected = s.mapped(&similarity, 2.0).unwrap();
        let mut moved = claimed.clone();
        moved.apply_transform(&similarity).unwrap();
        let got = moved.faces()[f]
            .surface
            .expect("similarity keeps the claim");
        assert!(got.same_surface(&expected), "{got:?} vs {expected:?}");
        moved.validate().unwrap();
        // The mapped claim is the hand-computed one.
        match (s, got) {
            (
                SurfaceRef::Sphere { center, radius },
                SurfaceRef::Sphere {
                    center: c2,
                    radius: r2,
                },
            ) => {
                assert!(similarity.apply_point(center).approx_eq(c2, 1e-12));
                assert_f(r2, 2.0 * radius);
            }
            (
                SurfaceRef::Torus {
                    major_radius,
                    minor_radius,
                    ..
                },
                SurfaceRef::Torus {
                    major_radius: m2,
                    minor_radius: n2,
                    ..
                },
            ) => {
                assert_f(m2, 2.0 * major_radius);
                assert_f(n2, 2.0 * minor_radius);
            }
            (SurfaceRef::Cone { half_angle, .. }, SurfaceRef::Cone { half_angle: a2, .. }) => {
                assert_f(a2, half_angle);
            }
            (SurfaceRef::Cylinder { radius, .. }, SurfaceRef::Cylinder { radius: r2, .. }) => {
                assert_f(r2, 2.0 * radius);
            }
            other => panic!("class changed under a similarity: {other:?}"),
        }

        let mut stretched = claimed.clone();
        stretched
            .apply_transform(&Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
            .unwrap();
        assert_eq!(
            stretched.faces()[f].surface,
            None,
            "non-uniform scale drops {s:?}"
        );
        stretched.validate().unwrap();
    }
}

// --------------------------------------------------- 6. push/pull route

#[test]
fn push_pull_on_a_sphere_facet_takes_the_planar_path_and_drops_its_claim() {
    // A box whose top face claims a sphere centered under it, whose four
    // side faces claim a cylinder (planes parallel to +Z): the top's claim
    // is dropped by the planar path; the sides keep theirs.
    let mut obj = unit_box();
    let top = face_with_normal(&obj, Z);
    let sphere = SurfaceRef::Sphere {
        center: p3(0.5, 0.5, 0.0),
        radius: 2.0,
    };
    obj.set_face_surface_for_test(top, Some(sphere));
    let side = face_with_normal(&obj, Vec3::new(1.0, 0.0, 0.0));
    let side_claim = SurfaceRef::Cylinder {
        axis_point: p3(0.5, 0.5, 0.0),
        axis: Z,
        radius: 0.75,
    };
    obj.set_face_surface_for_test(side, Some(side_claim));
    obj.validate().unwrap();

    let before_faces = obj.faces().len();
    obj.push_pull(top, 0.25).unwrap();
    assert_valid_solid(&obj);
    assert_eq!(
        obj.faces().len(),
        before_faces,
        "a box top translates, no new faces"
    );
    assert_eq!(
        obj.faces()[top].surface,
        None,
        "the moved facet drops its claim"
    );
    assert_eq!(
        obj.faces()[side].surface,
        Some(side_claim),
        "a stretched neighbour keeps its claim"
    );
    let top_z: Vec<f64> = obj
        .loop_positions(obj.faces()[top].outer_loop)
        .map(|p| p.z)
        .collect();
    assert!(
        top_z.iter().all(|&z| (z - 1.25).abs() <= 1e-12),
        "translated, not offset"
    );
}

// ---------------------------------------------- 7. analytic centers

#[test]
fn analytic_centers_report_each_distinct_sphere_and_torus_once() {
    let mut obj = unit_box();
    let ids: Vec<FaceId> = obj.faces().keys().collect();
    let sphere = SurfaceRef::Sphere {
        center: p3(0.5, 0.5, 0.5),
        radius: 5.0,
    };
    let sphere_copy = perturbed(sphere, 0.25 * tol::POINT_MERGE);
    let torus = SurfaceRef::Torus {
        center: p3(0.5, 0.5, -1.0),
        axis: Z,
        major_radius: 3.0,
        minor_radius: 4.0,
    };
    let cone = SurfaceRef::Cone {
        apex: p3(0.5, 0.5, 0.5),
        axis: Z,
        half_angle: 0.3,
    };
    // Claims are set directly (the validator is not the subject here).
    obj.set_face_surface_for_test(ids[0], Some(sphere));
    obj.set_face_surface_for_test(ids[1], Some(torus));
    obj.set_face_surface_for_test(ids[2], Some(sphere_copy));
    obj.set_face_surface_for_test(ids[3], Some(cone));
    let centers = obj.analytic_centers();
    assert_eq!(
        centers,
        vec![(p3(0.5, 0.5, 0.5), ids[0]), (p3(0.5, 0.5, -1.0), ids[1])]
    );
    assert_eq!(obj.analytic_centers(), centers, "deterministic");
}
