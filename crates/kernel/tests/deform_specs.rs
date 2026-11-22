//! Executable specs for Deform (true-curves.md §5.3): Twist, Bend, Taper
//! and Bend to Path, and the Bend to Path checkpoint (00-README §5.1).

use kernel::trig;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_6, PI};

use kernel::deform::{DeformFrame, DeformSpec};
use kernel::math::{Plane, Point3, Vec3};
use kernel::shape_error::ShapeError;
use kernel::sketch::{CurveGeom, Profile, Sketch};
use kernel::topo::{Object, SurfaceRef, WatertightState};

fn xy_plane() -> Plane {
    Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

/// A box `[-hx, hx] × [-hy, hy] × [0, h]`.
fn bar(hx: f64, hy: f64, h: f64) -> Object {
    let p = Profile::new(
        xy_plane(),
        vec![
            Point3::new(-hx, -hy, 0.0),
            Point3::new(hx, -hy, 0.0),
            Point3::new(hx, hy, 0.0),
            Point3::new(-hx, hy, 0.0),
        ],
        vec![],
    )
    .unwrap();
    Object::from_extrusion(&p, h).unwrap()
}

fn z_frame(length: f64) -> DeformFrame {
    DeformFrame {
        origin: Point3::ORIGIN,
        axis: Vec3::new(0.0, 0.0, 1.0),
        u: Vec3::new(1.0, 0.0, 0.0),
        length,
    }
}

fn volume(o: &Object) -> f64 {
    let mut v = 0.0;
    for f in o.faces().values() {
        for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<Vec3> = o.loop_positions(lid).map(|q| q.to_vec()).collect();
            for k in 1..p.len() - 1 {
                v += p[0].dot(p[k].cross(p[k + 1]));
            }
        }
    }
    v / 6.0
}

fn assert_solid(o: &Object) {
    o.validate().unwrap();
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!(volume(o) > 0.0);
}

fn rot_z(p: Point3, a: f64) -> Point3 {
    let (s, c) = trig::sin_cos(a);
    Point3::new(p.x * c - p.y * s, p.x * s + p.y * c, p.z)
}

/// A blade plan: a rectangle `[0, l] × [0, w]` ending in a semicircular
/// tip of `n` facets beyond `x = l`, extruded `h`.
fn tipped_blade(l: f64, w: f64, h: f64, n: usize) -> Object {
    let r = w / 2.0;
    let mut pts = vec![Point3::new(0.0, 0.0, 0.0), Point3::new(l, 0.0, 0.0)];
    for i in 1..n {
        let t = -FRAC_PI_2 + PI * i as f64 / n as f64;
        pts.push(Point3::new(l + r * trig::cos(t), r + r * trig::sin(t), 0.0));
    }
    pts.push(Point3::new(l, w, 0.0));
    pts.push(Point3::new(0.0, w, 0.0));
    let p = Profile::new(xy_plane(), pts, vec![]).unwrap();
    Object::from_extrusion(&p, h).unwrap()
}

#[test]
fn twist_at_default_stations_keeps_the_volume_of_an_arc_tipped_blade() {
    // Playtest run 4, F-56: an 18° twist took 17% of a fan blade's volume
    // because the tip's facet vertices all lay inside one of two bands.
    // Cuts through every piece vertex, and four bands at least, keep it.
    let blade = tipped_blade(0.48, 0.11, 0.008, 24);
    let before = volume(&blade);
    let frame = DeformFrame {
        origin: Point3::new(0.0, 0.055, 0.004),
        axis: Vec3::new(1.0, 0.0, 0.0),
        u: Vec3::new(0.0, 1.0, 0.0),
        length: 0.535,
    };
    let d = blade
        .deformed(&DeformSpec::Twist {
            frame,
            angle: 18.0_f64.to_radians(),
            stations: None,
        })
        .unwrap();
    assert_solid(&d);
    let loss = (before - volume(&d)) / before;
    assert!(loss.abs() < 0.005, "volume changed by {:.2}%", loss * 100.0);
    // The same for a plain bar, which was already fine.
    let o = bar(0.055, 0.004, 0.48);
    let d = o
        .deformed(&DeformSpec::Twist {
            frame: z_frame(0.48),
            angle: 18.0_f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let loss = (volume(&o) - volume(&d)) / volume(&o);
    assert!(
        loss.abs() < 0.005,
        "bar volume changed by {:.2}%",
        loss * 100.0
    );
}

#[test]
fn twist_box_90() {
    let o = bar(0.05, 0.05, 1.0);
    let d = o
        .deformed(&DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: FRAC_PI_2,
            stations: Some(6),
        })
        .unwrap();
    assert_solid(&d);
    let corners = [
        Point3::new(-0.05, -0.05, 0.0),
        Point3::new(0.05, -0.05, 0.0),
        Point3::new(0.05, 0.05, 0.0),
        Point3::new(-0.05, 0.05, 0.0),
    ];
    // The top is one quad, turned exactly 90°.
    let top: Vec<_> = d
        .faces()
        .values()
        .filter(|f| f.plane.normal().z > 1.0 - 1e-12)
        .collect();
    assert_eq!(top.len(), 1);
    let ring: Vec<Point3> = d.loop_positions(top[0].outer_loop).collect();
    assert_eq!(ring.len(), 4);
    for c in corners {
        let want = rot_z(c, FRAC_PI_2) + Vec3::new(0.0, 0.0, 1.0);
        assert!(
            ring.iter().any(|p| (*p - want).length() < 1e-12),
            "{want:?}"
        );
    }
    // Station rings at i/6, turned 15°·i.
    for i in 1..6 {
        let z = i as f64 / 6.0;
        for c in corners {
            let want = rot_z(c, FRAC_PI_2 * i as f64 / 6.0) + Vec3::new(0.0, 0.0, z);
            assert!(
                d.vertices()
                    .values()
                    .any(|v| (v.position - want).length() < 1e-12),
                "station {i}: {want:?}"
            );
        }
    }
    // Sides are triangles; their diagonals are soft.
    let sides = d
        .faces()
        .values()
        .filter(|f| f.plane.normal().z.abs() < 1.0 - 1e-9)
        .count();
    assert_eq!(sides, 6 * 4 * 4, "each bent band fans to its center");
    assert!(d.edges().values().filter(|e| e.soft).count() >= 6 * 4);
    assert!(
        (volume(&d) - 0.01).abs() < 0.01 * 0.01,
        "volume {}",
        volume(&d)
    );
}

#[test]
fn taper_box_exact_frustum() {
    let o = bar(0.1, 0.1, 1.0);
    let d = o
        .deformed(&DeformSpec::Taper {
            frame: z_frame(1.0),
            ratio: 0.5,
            stations: None,
        })
        .unwrap();
    assert_solid(&d);
    assert_eq!(d.faces().len(), 6, "planar side quads");
    let (a0, a1) = (0.04_f64, 0.01_f64);
    let want = (a0 + a1 + (a0 * a1).sqrt()) / 3.0;
    assert!(((volume(&d) - want) / want).abs() < 1e-12);
}

#[test]
fn bend_beam_90() {
    let o = bar(0.025, 0.025, 1.0);
    let d = o
        .deformed(&DeformSpec::Bend {
            frame: z_frame(1.0),
            angle: FRAC_PI_2,
            stations: None,
        })
        .unwrap();
    assert_solid(&d);
    // The moving end now faces +u (+x).
    let end = d
        .faces()
        .values()
        .find(|f| f.plane.normal().x > 1.0 - 1e-12)
        .expect("an end face turned 90°");
    let n = end.plane.normal();
    assert!((n - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-12, "{n:?}");
}

#[test]
fn bend_too_far_refuses_with_limit() {
    let o = bar(0.1, 0.1, 0.2);
    let spec = DeformSpec::Bend {
        frame: z_frame(0.2),
        angle: 300f64.to_radians(),
        stations: None,
    };
    let err = o.deformed(&spec).unwrap_err();
    let ShapeError::SelfIntersecting { limit: Some(l) } = err else {
        panic!("{err:?}");
    };
    assert!(l > 0.0 && l < 300f64.to_radians());
    let ok = DeformSpec::Bend {
        frame: z_frame(0.2),
        angle: l * (1.0 - 1e-6),
        stations: None,
    };
    assert_solid(&o.deformed(&ok).unwrap());
    assert_eq!(o.deform_limit(&spec).unwrap(), l);
}

/// A 24-facet circle of radius 0.1 extruded 1 m: stamped cylinder walls.
fn cylinder() -> Object {
    let mut sk = Sketch::on_plane(xy_plane());
    sk.begin_curve_with(CurveGeom {
        center: Point3::ORIGIN,
        radius: 0.1,
    })
    .unwrap();
    let n = 24;
    let p = |k: usize| {
        let a = k as f64 * 2.0 * PI / n as f64;
        Point3::new(0.1 * trig::cos(a), 0.1 * trig::sin(a), 0.0)
    };
    for k in 0..n {
        sk.add_segment(p(k), p((k + 1) % n)).unwrap();
    }
    sk.end_curve();
    let region = sk.regions().keys().next().unwrap();
    Object::from_extrusion(&sk.profile(region).unwrap(), 1.0).unwrap()
}

#[test]
fn cylinder_twist_drops_claims_keeps_soft() {
    let o = cylinder();
    assert!(
        o.faces()
            .values()
            .any(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
    );
    let d = o
        .deformed(&DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: FRAC_PI_6,
            stations: None,
        })
        .unwrap();
    assert_solid(&d);
    assert!(
        d.faces().values().all(|f| f.surface.is_none()),
        "claims dropped"
    );
    assert!(d.edges().values().all(|e| e.curve.is_none()));
    // Every former facet seam (a vertical wall edge) is soft.
    let soft_seams = d
        .edges()
        .iter()
        .filter(|&(eid, e)| {
            let (a, b) = d.edge_endpoints(eid).unwrap();
            e.soft && (a.z - b.z).abs() > 1e-9
        })
        .count();
    assert!(soft_seams >= 24 * 2, "{soft_seams}");
}

/// A polyline: up 0.4, a quarter arc of radius 0.2 (24 facets), across 0.4.
fn l_with_arc() -> Vec<Point3> {
    let mut path = vec![Point3::ORIGIN, Point3::new(0.0, 0.0, 0.4)];
    for k in 1..=24 {
        let t = FRAC_PI_2 * k as f64 / 24.0;
        path.push(Point3::new(
            0.2 - 0.2 * trig::cos(t),
            0.0,
            0.4 + 0.2 * trig::sin(t),
        ));
    }
    path.push(Point3::new(0.6, 0.0, 0.6));
    path
}

#[test]
fn bend_to_path_l_with_arc() {
    let path = l_with_arc();
    let total: f64 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let o = bar(0.02, 0.02, total);
    let d = o
        .deformed(&DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::new(0.0, 0.0, total),
            path: path.clone(),
            closed: false,
            stations_per_segment: None,
        })
        .unwrap();
    assert_solid(&d);
    let end = d
        .faces()
        .values()
        .find(|f| f.plane.normal().x > 1.0 - 1e-9)
        .expect("the end face points along the last segment");
    let ring: Vec<Point3> = d.loop_positions(end.outer_loop).collect();
    let c = ring.iter().fold(Vec3::ZERO, |a, p| a + p.to_vec()) / ring.len() as f64;
    assert!((Point3::ORIGIN + c - *path.last().unwrap()).length() < 1e-9);
}

#[test]
fn strong_guarantee_on_refusal() {
    let o = bar(0.1, 0.1, 0.2);
    let before = format!("{o:?}");
    let _ = o.deformed(&DeformSpec::Bend {
        frame: z_frame(0.2),
        angle: 5.0,
        stations: None,
    });
    let _ = o.deformed(&DeformSpec::Twist {
        frame: z_frame(0.2),
        angle: f64::NAN,
        stations: None,
    });
    assert_eq!(format!("{o:?}"), before);
}

#[test]
fn bad_specs_refuse_amount_out_of_range() {
    let o = bar(0.1, 0.1, 1.0);
    let bad = [
        DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: 5.0 * PI,
            stations: None,
        },
        DeformSpec::Bend {
            frame: z_frame(1.0),
            angle: 7.0,
            stations: None,
        },
        DeformSpec::Taper {
            frame: z_frame(1.0),
            ratio: 0.0,
            stations: None,
        },
        DeformSpec::Taper {
            frame: z_frame(0.0),
            ratio: 0.5,
            stations: None,
        },
        DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: 0.5,
            stations: Some(0),
        },
        DeformSpec::Twist {
            frame: DeformFrame {
                u: Vec3::new(0.0, 0.0, 1.0),
                ..z_frame(1.0)
            },
            angle: 0.5,
            stations: None,
        },
        DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::ORIGIN,
            path: l_with_arc(),
            closed: false,
            stations_per_segment: None,
        },
        DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::new(0.0, 0.0, 1.0),
            path: vec![Point3::ORIGIN],
            closed: false,
            stations_per_segment: None,
        },
        // A typed station count past the cap refuses instead of building
        // billions of stations.
        DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: 0.5,
            stations: Some(4_000_000_000),
        },
        DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::new(0.0, 0.0, 1.0),
            path: l_with_arc(),
            closed: false,
            stations_per_segment: Some(4_000_000_000),
        },
    ];
    for spec in bad {
        assert_eq!(
            o.deformed(&spec).unwrap_err(),
            ShapeError::AmountOutOfRange,
            "{spec:?}"
        );
    }
}

#[test]
fn open_object_refuses_needs_solid() {
    // A box missing its top.
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let pos = [
        p(0., 0., 0.),
        p(1., 0., 0.),
        p(1., 1., 0.),
        p(0., 1., 0.),
        p(0., 0., 1.),
        p(1., 0., 1.),
        p(1., 1., 1.),
        p(0., 1., 1.),
    ];
    let o = Object::from_polygons(
        &pos,
        &[
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .unwrap();
    let err = o
        .deformed(&DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: 0.5,
            stations: None,
        })
        .unwrap_err();
    assert_eq!(err, ShapeError::ShapeNeedsSolid);
}

// ─────────────────────────────── checkpoint: Bend to Path (00-README §5.1)

/// Bends a 0.4 × 0.05 × 0.05 box along `path` and checks the checkpoint's
/// claims: valid, watertight, every station ring perpendicular to the path
/// (the chord bisector at a joint), volume within 2 % of the straight box
/// stretched to the path's length.
/// Returns (volume ratio, worst ring tilt) for the hand-off evidence.
fn checkpoint(path: Vec<Point3>) -> (f64, f64) {
    let total: f64 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let o = bar(0.025, 0.025, 0.4);
    let d = o
        .deformed(&DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::new(0.0, 0.0, 0.4),
            path: path.clone(),
            closed: false,
            stations_per_segment: Some(1),
        })
        .unwrap();
    assert_solid(&d);
    // The base line maps end to end onto the path, so the box stretches to
    // the path's length: compare with the straight box of that length.
    let ratio = volume(&d) / (0.05 * 0.05 * total);
    assert!((ratio - 1.0).abs() < 0.02, "volume ratio {ratio}");
    let half_diag = 0.025 * 2f64.sqrt();
    let mut worst = 0.0_f64;
    for j in 0..path.len() {
        let tangent = if j == 0 {
            (path[1] - path[0]).normalized().unwrap()
        } else if j + 1 == path.len() {
            (path[j] - path[j - 1]).normalized().unwrap()
        } else {
            ((path[j] - path[j - 1]).normalized().unwrap()
                + (path[j + 1] - path[j]).normalized().unwrap())
            .normalized()
            .unwrap()
        };
        // The ring at this station: the four corners at the half-diagonal
        // from the path point.
        let ring: Vec<Point3> = d
            .vertices()
            .values()
            .map(|v| v.position)
            .filter(|p| ((*p - path[j]).length() - half_diag).abs() < 1e-9)
            .collect();
        assert_eq!(ring.len(), 4, "station {j}");
        for p in ring {
            worst = worst.max(((p - path[j]).dot(tangent) / half_diag).abs());
        }
    }
    assert!(worst <= kernel::tol::NORMAL_DIRECTION, "worst {worst}");
    (ratio, worst)
}

fn quarter(n: usize) -> Vec<Point3> {
    (0..=n)
        .map(|k| {
            let t = FRAC_PI_2 * k as f64 / n as f64;
            Point3::new(1.0 - trig::cos(t), 0.0, trig::sin(t))
        })
        .collect()
}

#[test]
fn bend_to_path_keeps_material_beyond_the_base_line() {
    // The solid is twice as long as the base line; the excess continues
    // along the path's end tangent instead of collapsing onto the end.
    let r = 0.4 / FRAC_PI_2;
    let b = bar(0.025, 0.025, 0.8);
    let path: Vec<Point3> = quarter(24)
        .into_iter()
        .map(|q| Point3::new(q.x * r, 0.0, q.z * r))
        .collect();
    let out = b
        .deformed(&DeformSpec::BendToPath {
            base_from: Point3::ORIGIN,
            base_to: Point3::new(0.0, 0.0, 0.4),
            path,
            closed: false,
            stations_per_segment: None,
        })
        .unwrap();
    assert_solid(&out);
    let v = volume(&out);
    assert!(
        (v - 0.05 * 0.05 * 0.8).abs() / (0.05 * 0.05 * 0.8) < 0.05,
        "volume {v}"
    );
    // The far end continues one base-line length straight along the last
    // chord (the polyline's end tangent), a few degrees off +x.
    let tilt = FRAC_PI_2 / 24.0 / 2.0;
    let expected = r + 0.4 * trig::cos(tilt) + 0.025 * trig::sin(tilt);
    let far = out
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MIN, f64::max);
    assert!((far - expected).abs() < 1e-3, "far end at x = {far}");
}

#[test]
fn checkpoint_bend_to_path_quarter_circle() {
    let (ratio, worst) = checkpoint(quarter(24));
    println!("quarter circle: volume ratio {ratio:.6}, worst ring tilt {worst:.3e}");
}

#[test]
fn checkpoint_bend_to_path_s_curve() {
    // Two opposite quarter arcs of radius 1: (0,0,0) → (1,0,1) heading +x,
    // then around (1,0,2) up to (2,0,2) heading +z.
    let mut path = quarter(24);
    for k in 1..=24 {
        let t = FRAC_PI_2 * k as f64 / 24.0;
        path.push(Point3::new(1.0 + trig::sin(t), 0.0, 2.0 - trig::cos(t)));
    }
    let (ratio, worst) = checkpoint(path);
    println!("S-curve: volume ratio {ratio:.6}, worst ring tilt {worst:.3e}");
}

#[test]
fn document_deform_is_one_undo_step() {
    let mut doc = kernel::Document::new();
    let s = doc.add_sketch(xy_plane());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(-0.1, -0.1), (0.1, -0.1), (0.1, 0.1), (-0.1, 0.1)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let (oid, _) = doc.extrude_region(s, r, 1.0).unwrap();
    let before = doc.state_hash();
    let spec = DeformSpec::Twist {
        frame: z_frame(1.0),
        angle: FRAC_PI_2,
        stations: None,
    };
    let preview = doc.deform_preview(oid, &spec).unwrap();
    assert_eq!(doc.state_hash(), before, "a preview changes nothing");
    doc.deform(oid, &spec).unwrap();
    let after = doc.state_hash();
    assert_ne!(after, before);
    assert_eq!(
        format!("{:?}", doc.object(oid).unwrap()),
        format!("{preview:?}")
    );
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), after);
    assert_eq!(doc.deform_limit(oid, &spec).unwrap(), FRAC_PI_2);
}

/// `document_fuzz` seed 746900c5: a face split off a corner, a 3° bend, then
/// a push of a side face. The bend's planarized wall leaves a skew triangle
/// touching the pushed face at one corner only; its plane does not contain
/// the sweep, so the push must refuse typed instead of bending it. The bend
/// is the coarse one the fuzz ran (no cuts through piece vertices): the
/// default bend no longer leaves that fan, but the guard stays.
#[test]
fn push_pull_on_a_corner_shared_with_a_skew_face_refuses_typed() {
    use kernel::KernelOp;
    use kernel::ops::PushPullError;
    let mut doc = kernel::Document::new();
    let s = doc.add_sketch(xy_plane());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let (oid, _) = doc.extrude_region(s, r, 0.5).unwrap();
    let face_with_normal = |obj: &Object, n: Vec3| {
        obj.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().dot(n) > 0.999)
            .map(|(id, _)| id)
            .expect("a face with that normal")
    };
    // Cut a corner triangle off the +x wall: a path between points a quarter
    // of the way along two adjacent edges.
    let px = face_with_normal(doc.object(oid).unwrap(), Vec3::new(1.0, 0.0, 0.0));
    let ring: Vec<Point3> = {
        let obj = doc.object(oid).unwrap();
        obj.loop_positions(obj.faces()[px].outer_loop).collect()
    };
    let at = |i: usize, t: f64| ring[i] + (ring[(i + 1) % ring.len()] - ring[i]) * t;
    doc.apply_object_op(
        oid,
        KernelOp::SplitFace {
            face: px,
            path: vec![at(3, 0.25), at(0, 0.25)],
            restore: None,
            curves: Vec::new(),
        },
    )
    .unwrap();
    let mut bent = doc
        .object(oid)
        .unwrap()
        .deformed_coarse_for_test(&DeformSpec::Bend {
            frame: DeformFrame {
                origin: Point3::new(0.5, 0.5, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                u: Vec3::new(1.0, 0.0, 0.0),
                length: 0.5,
            },
            angle: 0.05 * PI / 3.0,
            // One band, as the fuzz run's default was for 3°.
            stations: Some(1),
        })
        .unwrap();
    let before = bent.clone();
    let minus_y = face_with_normal(&bent, Vec3::new(0.0, -1.0, 0.0));
    let err = bent
        .push_pull(minus_y, 2.497678704269216)
        .expect_err("a corner shared with a skew face cannot be pushed");
    assert_eq!(err, PushPullError::WallNeighborNonPlanar);
    assert!(kernel::same_geometry_for_test(&bent, &before, 0.0));
}

#[test]
fn fillet_then_twist_stays_smooth() {
    let o = bar(0.1, 0.1, 1.0);
    // Fillet one vertical edge of the bar.
    let edge = o
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            (a.x - 0.1).abs() < 1e-12
                && (a.y - 0.1).abs() < 1e-12
                && (b.x - 0.1).abs() < 1e-12
                && (b.y - 0.1).abs() < 1e-12
        })
        .expect("the (+x, +y) vertical edge");
    let spec = kernel::BlendSpec {
        edges: vec![edge],
        profile: kernel::BlendProfile::Round {
            radius: 0.03,
            segments: 6,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    };
    let (filleted, _) = o.blend_edges(&spec).unwrap();
    let smooth_before = filleted
        .edges()
        .keys()
        .filter(|&e| {
            matches!(
                filleted.edge_smoothness(e),
                kernel::surface::EdgeSmoothness::Seam | kernel::surface::EdgeSmoothness::Tangent
            )
        })
        .count();
    assert!(smooth_before > 0, "a fillet has rails and seams");
    let d = filleted
        .deformed(&DeformSpec::Twist {
            frame: z_frame(1.0),
            angle: FRAC_PI_6,
            stations: None,
        })
        .unwrap();
    assert_solid(&d);
    assert!(
        d.faces()
            .values()
            .all(|f| f.surface.is_none() && f.blend.is_none()),
        "claims and tags dropped"
    );
    // Every former rail and seam is still a soft edge (split at the
    // stations, so at least as many as before).
    let soft = d.edges().values().filter(|e| e.soft).count();
    assert!(
        soft >= smooth_before,
        "{soft} soft vs {smooth_before} smooth before"
    );
}

/// Rounds every edge of the face whose normal is `normal`.
fn fillet_face_loop(o: &Object, normal: Vec3, r: f64) -> Object {
    let (fid, f) = o
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().dot(normal) > 0.999)
        .unwrap();
    let near = o.loop_positions(f.outer_loop).next().unwrap();
    let spec = kernel::BlendSpec {
        edges: o.face_loop_edges(fid, near),
        profile: kernel::BlendProfile::Round {
            radius: r,
            segments: 6,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    };
    o.blend_edges(&spec).unwrap().0
}

#[test]
fn bend_after_twist_on_a_filleted_blade() {
    // A 1 m × 0.2 m × 6 mm blade with rounded rims, twisted 20° about its
    // length, then bent 5° across its width (the Deform tool's typed
    // bend): the twist leaves long facets fanned to centers along the
    // narrow rim strips, and the bend must not fold neighbors through each
    // other. Rounded on one side or both, the bend goes through whole.
    let blade = bar(0.5, 0.1, 0.006);
    let top = fillet_face_loop(&blade, Vec3::new(0.0, 0.0, 1.0), 0.002);
    let both = fillet_face_loop(&top, Vec3::new(0.0, 0.0, -1.0), 0.002);
    let frame = DeformFrame {
        origin: Point3::new(-0.5, 0.0, 0.003),
        axis: Vec3::new(1.0, 0.0, 0.0),
        u: Vec3::new(0.0, 1.0, 0.0),
        length: 1.0,
    };
    for o in [top, both] {
        let twisted = o
            .deformed(&DeformSpec::Twist {
                frame,
                angle: 20f64.to_radians(),
                stations: None,
            })
            .unwrap();
        let bent = twisted
            .deformed(&DeformSpec::Bend {
                frame,
                angle: 5f64.to_radians(),
                stations: None,
            })
            .expect("a 5° bend of a twisted blade does not pass through itself");
        assert_solid(&bent);
        assert!(kernel::patch::self_intersection_for_test(&bent).is_none());
    }
}

// ─────────────────────── Bend after Twist, and previews with a work cap

/// A 1 m × 0.2 m × 40 mm bar along x from the origin, its top rim rounded
/// 5 mm in 6 facets: the playtest's filleted bar.
fn filleted_bar() -> Object {
    let p = Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, -0.1, 0.0),
            Point3::new(1.0, -0.1, 0.0),
            Point3::new(1.0, 0.1, 0.0),
            Point3::new(0.0, 0.1, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let o = Object::from_extrusion(&p, 0.04).unwrap();
    fillet_face_loop(&o, Vec3::new(0.0, 0.0, 1.0), 0.005)
}

/// The deform box of `o` along +x from its −x end, bending toward `u`.
fn x_box(o: &Object, u: Vec3) -> DeformFrame {
    let (mut lo, mut hi) = (
        Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for v in o.vertices().values() {
        let p = v.position;
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    DeformFrame {
        origin: Point3::new(lo.x, 0.5 * (lo.y + hi.y), 0.5 * (lo.z + hi.z)),
        axis: Vec3::new(1.0, 0.0, 0.0),
        u,
        length: hi.x - lo.x,
    }
}

fn bend_x(o: &Object, u: Vec3, degrees: f64) -> DeformSpec {
    DeformSpec::Bend {
        frame: x_box(o, u),
        angle: degrees.to_radians(),
        stations: None,
    }
}

/// A budget the limit searches of these specs fit in: their retry budget
/// and as much again for first builds.
fn roomy_search() -> u64 {
    kernel::deform::LIMIT_RETRY_WORK + kernel::deform::LIMIT_WORK_SCALE
}

#[test]
fn bend_after_twist_commits_at_the_limit_it_names() {
    // F-58: a bend of a twisted solid named a limit that then committed
    // nothing. The limit a refusal names, and the hair under it the tool's
    // clamp asks for, both build; on either side of the bend.
    let p = Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, -0.25, 0.0),
            Point3::new(1.0, -0.25, 0.0),
            Point3::new(1.0, 0.25, 0.0),
            Point3::new(0.0, 0.25, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let wide = Object::from_extrusion(&p, 0.04).unwrap();
    let twisted = wide
        .deformed(&DeformSpec::Twist {
            frame: x_box(&wide, Vec3::new(0.0, 1.0, 0.0)),
            angle: 30f64.to_radians(),
            stations: None,
        })
        .unwrap();
    for degrees in [300.0, -300.0] {
        let spec = bend_x(&twisted, Vec3::new(0.0, 1.0, 0.0), degrees);
        let err = twisted.deformed(&spec).unwrap_err();
        let ShapeError::SelfIntersecting { limit: Some(l) } = err else {
            panic!("{err:?}");
        };
        assert!(l.signum() == degrees.signum() && l.abs() < 300f64.to_radians());
        // The drag's search under its own cap gives up on a solid this
        // dense (the drag skips the preview); given room, it names exactly
        // the commit's limit.
        assert_eq!(
            twisted.deform_preview_limit(&spec),
            Err(ShapeError::PreviewTooLarge)
        );
        assert_eq!(
            twisted
                .deform_preview_limit_with_cap_for_test(&spec, roomy_search())
                .unwrap(),
            l
        );
        for at in [l, l * (1.0 - 1e-6)] {
            let bent = twisted
                .deformed(&bend_x(&twisted, Vec3::new(0.0, 1.0, 0.0), at.to_degrees()))
                .unwrap_or_else(|e| panic!("{at} refused: {e:?}"));
            assert_solid(&bent);
        }
    }
}

#[test]
fn bend_after_twist_of_a_filleted_bar_is_not_refused_as_folding() {
    // A filleted bar twisted 90° leaves its rounded rim as strips of long
    // thin facets. Bent 65° (a radius of 0.88 m for 0.2 m of width) it
    // folds nowhere, but each thin facet mapped as one chord sagged by
    // about its own width and flipped against its neighbors: the bend was
    // refused as passing through itself while 80° went through. The retry
    // now bands such facets closely enough that their chords do not.
    let o = filleted_bar();
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let before = volume(&twisted);
    for degrees in [65.0, 70.0] {
        let bent = twisted
            .deformed(&bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), degrees))
            .unwrap_or_else(|e| panic!("{degrees}°: {e:?}"));
        assert_solid(&bent);
        assert!(kernel::patch::self_intersection_for_test(&bent).is_none());
        let change = (volume(&bent) - before) / before;
        assert!(change.abs() < 0.01, "{degrees}°: volume changed {change}");
    }
}

#[test]
fn a_preview_past_the_limit_refuses_without_searching_for_it() {
    // F-41: every pointer move past the limit ran the limit search inside
    // the preview (some 30 builds), and on a heavy object the app hung for
    // minutes. The preview refuses at the cost of the build and its retry;
    // the drag asks `deform_preview_limit` once; the commit names it.
    let o = bar(0.05, 0.05, 1.0);
    let spec = DeformSpec::Bend {
        frame: z_frame(1.0),
        angle: std::f64::consts::TAU,
        stations: None,
    };
    let (r, spent) = o.deform_preview_with_cap_for_test(&spec, u64::MAX);
    assert_eq!(r.unwrap_err(), ShapeError::SelfIntersecting { limit: None });
    assert!(spent > 0, "the full turn builds before it is refused");
    assert_eq!(
        o.deform_preview(&spec).unwrap_err(),
        ShapeError::SelfIntersecting { limit: None }
    );
    // The search needs many times what the preview spent.
    assert_eq!(
        o.deform_preview_limit_with_cap_for_test(&spec, 4 * spent),
        Err(ShapeError::PreviewTooLarge)
    );
    let l = o.deform_limit(&spec).unwrap();
    // The drag's own cap is one preview's work: too little for this
    // search, so the drag skips; given room, it names the commit's limit.
    assert_eq!(
        o.deform_preview_limit(&spec),
        Err(ShapeError::PreviewTooLarge)
    );
    assert_eq!(
        o.deform_preview_limit_with_cap_for_test(&spec, roomy_search())
            .unwrap(),
        l
    );
    assert_eq!(
        o.deformed(&spec).unwrap_err(),
        ShapeError::SelfIntersecting { limit: Some(l) }
    );
}

#[test]
fn a_preview_over_its_work_cap_gives_up_and_the_commit_still_builds() {
    let o = cylinder();
    let spec = DeformSpec::Twist {
        frame: z_frame(1.0),
        angle: PI,
        stations: None,
    };
    let (full, spent) = o.deform_preview_with_cap_for_test(&spec, u64::MAX);
    let full = full.unwrap();
    // A count, not a clock: the same spec spends the same every time.
    assert_eq!(o.deform_preview_with_cap_for_test(&spec, u64::MAX).1, spent);
    let (r, used) = o.deform_preview_with_cap_for_test(&spec, spent - 1);
    assert_eq!(r.unwrap_err(), ShapeError::PreviewTooLarge);
    assert_eq!(used, spent - 1);
    // At its cost exactly it builds, and builds what the commit does.
    let (at, _) = o.deform_preview_with_cap_for_test(&spec, spent);
    let committed = o.deformed(&spec).unwrap();
    assert!(kernel::same_geometry_for_test(
        &at.unwrap(),
        &committed,
        0.0
    ));
    assert!(kernel::same_geometry_for_test(&full, &committed, 0.0));
    assert!(kernel::same_geometry_for_test(
        &o.deform_preview(&spec).unwrap(),
        &committed,
        0.0
    ));
}

#[test]
fn a_heavy_preview_stops_at_the_cap() {
    // A dense object asked for many stations gives up as soon as the work
    // passes PREVIEW_WORK_CAP, not after building everything.
    let n = 400;
    let p = Profile::new(
        xy_plane(),
        (0..n)
            .map(|k| {
                let a = k as f64 * 2.0 * PI / n as f64;
                Point3::new(0.1 * trig::cos(a), 0.1 * trig::sin(a), 0.0)
            })
            .collect(),
        vec![],
    )
    .unwrap();
    let o = Object::from_extrusion(&p, 1.0).unwrap();
    let spec = DeformSpec::Twist {
        frame: z_frame(1.0),
        angle: 4.0 * PI,
        stations: Some(4096),
    };
    assert_eq!(
        o.deform_preview(&spec).unwrap_err(),
        ShapeError::PreviewTooLarge
    );
    let (r, used) = o.deform_preview_with_cap_for_test(&spec, kernel::deform::PREVIEW_WORK_CAP);
    assert_eq!(r.unwrap_err(), ShapeError::PreviewTooLarge);
    assert_eq!(used, kernel::deform::PREVIEW_WORK_CAP);
    assert_eq!(
        o.deform_preview_limit(&spec).unwrap_err(),
        ShapeError::PreviewTooLarge
    );
}

// ───────────────────────────── Round 9: bounded searches, shared crossings

/// A `w`-wide filleted bar like [`filleted_bar`].
fn filleted_bar_wide(w: f64) -> Object {
    let h = w / 2.0;
    let p = Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, -h, 0.0),
            Point3::new(1.0, -h, 0.0),
            Point3::new(1.0, h, 0.0),
            Point3::new(0.0, h, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let o = Object::from_extrusion(&p, 0.04).unwrap();
    fillet_face_loop(&o, Vec3::new(0.0, 0.0, 1.0), 0.005)
}

#[test]
fn a_genuine_fold_refusal_searches_its_limit_within_a_budget() {
    // Review round 9: a 1.0 × 0.6 m filleted bar twisted 30° and bent 300°
    // folds for real (the bend center reaches its far side). Each step of
    // the limit search ran the heavy thin-facet retry: refused after 207 s.
    // The steps' retries now share LIMIT_RETRY_WORK, so the whole search
    // fits a few times that, and the limit it names still commits.
    let o = filleted_bar_wide(0.6);
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 30f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let spec = bend_x(&twisted, Vec3::new(0.0, 1.0, 0.0), 300.0);
    let err = twisted.deformed(&spec).unwrap_err();
    let ShapeError::SelfIntersecting { limit: Some(l) } = err else {
        panic!("{err:?}");
    };
    assert!(l > 0.0 && l < 300f64.to_radians());
    assert_eq!(
        twisted.deform_preview_limit_with_cap_for_test(&spec, 4 * kernel::deform::LIMIT_RETRY_WORK),
        Ok(l)
    );
    let bent = twisted
        .deformed(&bend_x(&twisted, Vec3::new(0.0, 1.0, 0.0), l.to_degrees()))
        .unwrap();
    assert_solid(&bent);
}

#[test]
fn thin_facet_stations_are_charged_and_bounded() {
    // Review round 9: the retry's thin-facet stations were not charged to
    // a preview's work and kept a candidate per cut of every piece. They
    // are charged per piece, cut and bin, and never more than one per bin
    // (CONFORM_FRACTION of the box).
    let o = filleted_bar();
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let spec = bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), 65.0);
    let (r, spent) = twisted.deform_sliver_stations_for_test(&spec, u64::MAX);
    let ws = r.unwrap();
    assert!(!ws.is_empty(), "thin rim facets ask for stations");
    assert!(ws.len() <= 1000, "{} stations", ws.len());
    assert!(ws.windows(2).all(|p| p[1] - p[0] > 1e-3 * 0.999));
    assert!(spent >= twisted.faces().len() as u64);
    let (r, used) = twisted.deform_sliver_stations_for_test(&spec, spent / 2);
    assert_eq!(r.unwrap_err(), ShapeError::PreviewTooLarge);
    assert_eq!(used, spent / 2);
}

#[test]
fn the_self_intersection_check_charges_its_grid() {
    // Review round 9: the check charged a unit per face and per pair, not
    // the cells each face is registered in and visits. A twisted bar's
    // check costs at least a unit per face for registration and one per
    // face for matching, plus its pairs, and stops when the budget does.
    let o = filleted_bar();
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let faces = twisted.faces().len() as u64;
    let (total, r) = kernel::patch::self_intersection_cost_for_test(&twisted, u64::MAX);
    assert_eq!(r, Ok(false));
    assert!(total >= 4 * faces, "{total} for {faces} faces");
    let (_, r) = kernel::patch::self_intersection_cost_for_test(&twisted, 2 * faces);
    assert_eq!(r, Err(()));
}

#[test]
fn a_third_deform_after_a_bend_and_a_twist_builds() {
    // K-7: the filleted bar bent 3° and twisted 167° came out with edges of
    // 16 to 80 nm (a station through the tilted end cap crossed two edges
    // fanned nearly parallel from one corner), and every further Twist or
    // Bend refused ShapeDegenerate. Crossings of one cut that close are one
    // point now, shared by both pieces of each edge.
    k7_sequence(&filleted_bar());
}

/// K-7's sequence on `o`: bent 3°, twisted 167°, then a third Twist and
/// Bend, each building.
fn k7_sequence(o: &Object) {
    let bent = o
        .deformed(&bend_x(o, Vec3::new(0.0, 0.0, 1.0), 3.0))
        .unwrap();
    let twisted = bent
        .deformed(&DeformSpec::Twist {
            frame: x_box(&bent, Vec3::new(0.0, 1.0, 0.0)),
            angle: 167f64.to_radians(),
            stations: None,
        })
        .unwrap();
    assert_solid(&twisted);
    for spec in [
        DeformSpec::Twist {
            frame: x_box(&twisted, Vec3::new(0.0, 1.0, 0.0)),
            angle: 5f64.to_radians(),
            stations: None,
        },
        bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), 5.0),
    ] {
        let again = twisted
            .deformed(&spec)
            .unwrap_or_else(|e| panic!("{spec:?}: {e:?}"));
        assert_solid(&again);
    }
}

#[test]
fn k7_still_builds_with_a_tiny_chamfer_elsewhere() {
    // Review round 11: D-138's radius followed the solid's shortest edge,
    // so one tiny chamfer anywhere (5 µm here: a radius of 5 nm) shrank it
    // under the 16 to 80 nm K-7 needs merged. The radius follows the solid's section (twice its
    // volume over its area), which a tiny edge does not move.
    let o = filleted_bar();
    let edge = o
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            a.x.abs() < 1e-12 && b.x.abs() < 1e-12 && a.z.abs() < 1e-12 && b.z.abs() < 1e-12
        })
        .expect("the bottom edge of the fixed end");
    let spec = kernel::BlendSpec {
        edges: vec![edge],
        profile: kernel::BlendProfile::Chamfer {
            setback_a: 5e-6,
            setback_b: 5e-6,
        },
        primary: None,
        corners: kernel::CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    };
    let chamfered = o.blend_edges(&spec).unwrap().0;
    let shortest = chamfered
        .edges()
        .keys()
        .filter_map(|e| chamfered.edge_endpoints(e))
        .map(|(a, b)| (b - a).length())
        .fold(f64::INFINITY, f64::min);
    assert!(shortest < 1e-5, "a tiny edge: {shortest:e}");
    k7_sequence(&chamfered);
}

#[test]
fn a_limit_is_never_less_than_an_amount_shown_to_build() {
    // Review round 11: with first builds dear (a bar rounded top and
    // bottom, twisted 90°, bent with 60 stations), the search stopped at
    // its first dear refusal, restarted at half the amount and named the
    // identity, 0°, though 20°, 80° and 100° commit. It now seeds low and
    // keeps what it proved.
    let o = fillet_face_loop(&filleted_bar(), Vec3::new(0.0, 0.0, -1.0), 0.005);
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let spec = DeformSpec::Bend {
        frame: x_box(&twisted, Vec3::new(0.0, 0.0, 1.0)),
        angle: 360f64.to_radians(),
        stations: Some(60),
    };
    let l = twisted.deform_limit(&spec).unwrap();
    assert!(l.to_degrees() >= 20.0, "named {}°", l.to_degrees());
    let at = DeformSpec::Bend {
        frame: x_box(&twisted, Vec3::new(0.0, 0.0, 1.0)),
        angle: l,
        stations: Some(60),
    };
    assert_solid(&twisted.deformed(&at).unwrap());
}

#[test]
fn a_limit_names_the_amount_itself_when_the_commit_builds_it() {
    // Review round 11: the amount asked was checked with an eighth of the
    // retry budget, so 91° on the 90°-twisted filleted bar (it builds, by
    // the retry) was named as about 90°. The check has a budget of its own.
    let o = filleted_bar();
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let spec = bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), 91.0);
    assert!(twisted.deformed(&spec).is_ok());
    assert_eq!(twisted.deform_limit(&spec).unwrap(), 91f64.to_radians());
}

// ─────────────────────────── Round 10: thin sections, limits, edge records

/// A `w` × `t` bar `l` long standing on the ground.
fn thin_bar(w: f64, t: f64, l: f64) -> Object {
    bar(w / 2.0, t / 2.0, l)
}

#[test]
fn thin_sections_keep_their_volume_at_any_length() {
    // Review round 10: the shared-crossing radius was a millionth of the
    // box, so a 0.2 mm bar 1000 m long merged across its own section and
    // a 1.5 rad twist returned with 89% of its volume gone, and plates
    // thinner than a millionth of their length refused. The radius is
    // also bounded by the solid's shortest edge: every ratio behaves as a
    // stubby bar does.
    for (w, t, l) in [
        (2e-4, 2e-4, 1000.0),
        (2e-4, 2e-4, 100.0),
        (1e-3, 1e-3, 1.0),
        (1.0, 5e-7, 1.0),
        (1.0, 1e-6, 2.0),
        (0.1, 1e-5, 1.0),
    ] {
        let o = thin_bar(w, t, l);
        let before = volume(&o);
        for spec in [
            DeformSpec::Twist {
                frame: z_frame(l),
                angle: 1.5,
                stations: None,
            },
            DeformSpec::Bend {
                frame: z_frame(l),
                angle: 1.0,
                stations: None,
            },
        ] {
            let d = o
                .deformed(&spec)
                .unwrap_or_else(|e| panic!("{w} × {t} × {l} {spec:?}: {e:?}"));
            assert_eq!(d.watertight(), WatertightState::Watertight);
            let change = (volume(&d) - before) / before;
            assert!(
                change.abs() < 0.02,
                "{w} × {t} × {l}: volume changed {change}"
            );
        }
    }
}

#[test]
fn a_merge_that_would_drop_surface_refuses_instead() {
    // The backstop: forced to merge across the section (the old radius),
    // the cut would drop slivers that are real surface. It refuses
    // TooThinToDeform rather than return a smaller solid.
    let o = thin_bar(2e-4, 2e-4, 1000.0);
    let spec = DeformSpec::Twist {
        frame: z_frame(1000.0),
        angle: 1.5,
        stations: None,
    };
    assert_eq!(
        o.deformed_with_merge_for_test(&spec, 1e-3).unwrap_err(),
        ShapeError::TooThinToDeform
    );
    assert!(o.deformed_with_merge_for_test(&spec, 1e-7).is_ok());
}

#[test]
fn a_bend_after_twist_names_a_limit_near_the_true_one_quickly() {
    // Review round 10: the shared retry budget was spent on the first,
    // largest steps of the search, so the 90°-twisted filleted bar bent
    // toward its thickness named exactly 90° though 91°, 130° and 170°
    // build. The search now halves with first builds only, then climbs
    // past them a half at a time with the retry, near the boundary.
    let o = filleted_bar();
    let twisted = o
        .deformed(&DeformSpec::Twist {
            frame: x_box(&o, Vec3::new(0.0, 1.0, 0.0)),
            angle: 90f64.to_radians(),
            stations: None,
        })
        .unwrap();
    let spec = bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), 360.0);
    let l = twisted
        .deform_preview_limit_with_cap_for_test(&spec, roomy_search())
        .expect("the search fits its budgets");
    assert_eq!(twisted.deform_limit(&spec).unwrap(), l);
    assert!(l.to_degrees() >= 130.0, "named {}°", l.to_degrees());
    let bent = twisted
        .deformed(&bend_x(&twisted, Vec3::new(0.0, 0.0, 1.0), l.to_degrees()))
        .unwrap();
    assert_solid(&bent);
}

#[test]
fn a_plain_solid_names_the_limit_plain_halving_finds() {
    // The two-step search changes nothing where first builds decide: a
    // box and a cylinder name what 30 halvings of plain commits name, to
    // a millionth (the order of the steps decides the last bits), and the
    // amount named builds.
    let reference = |o: &Object, spec: &DeformSpec, target: f64| {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..30 {
            let mid = 0.5 * (lo + hi);
            let s = match spec {
                DeformSpec::Bend {
                    frame, stations, ..
                } => DeformSpec::Bend {
                    frame: *frame,
                    angle: target * mid,
                    stations: *stations,
                },
                _ => unreachable!(),
            };
            if o.deformed(&s).is_ok() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        target * lo
    };
    for (o, frame, target) in [
        (bar(0.1, 0.1, 0.2), z_frame(0.2), 300f64.to_radians()),
        (cylinder(), z_frame(1.0), 359f64.to_radians()),
    ] {
        let spec = DeformSpec::Bend {
            frame,
            angle: target,
            stations: None,
        };
        let l = o.deform_limit(&spec).unwrap();
        let r = reference(&o, &spec, target);
        assert!((l - r).abs() <= 1e-6 * target, "{l} against {r}");
        let at = DeformSpec::Bend {
            frame,
            angle: l,
            stations: None,
        };
        assert_solid(&o.deformed(&at).unwrap());
    }
}

#[test]
fn shared_crossings_record_their_edges_the_same_in_any_order() {
    // Review round 10: two edges from one corner whose crossings share a
    // point both name the segment from the corner to it, and the later
    // edge's record overwrote the earlier one's. The record is merged:
    // soft only when both are, a crease when either is, whatever the order.
    let lender = bar(0.5, 0.5, 1.0);
    let points = vec![
        Point3::new(0.0, 0.0, 0.0),  // 0 V
        Point3::new(1.0, 0.0, 0.0),  // 1 A
        Point3::new(1.0, 1e-8, 0.0), // 2 B: V-B nearly along V-A
        Point3::new(0.0, 1.0, 0.0),  // 3 C
        Point3::new(0.0, -1.0, 0.0), // 4 D
    ];
    let p_sliver = vec![0, 1, 2];
    let p_above = vec![0, 2, 3];
    let p_below = vec![0, 4, 1];
    let edges = vec![((0, 1), true, false), ((0, 2), false, true)];
    let mut seen = Vec::new();
    for order in [
        vec![p_below.clone(), p_sliver.clone(), p_above.clone()],
        vec![p_above.clone(), p_sliver.clone(), p_below.clone()],
    ] {
        let (pieces, recorded, dropped) =
            lender.deform_cut_for_test(points.clone(), order, edges.clone(), 0.5, 1e-6);
        // One crossing point for both edges (indices follow the order), the
        // sliver between them gone.
        let has = |pair: (u32, u32)| recorded.iter().any(|(k, ..)| *k == pair);
        let x = (5..10u32)
            .find(|&k| has((0, k)) && has((1, k)))
            .expect("V-A's crossing");
        assert!(has((2, x)), "V-B shares V-A's crossing");
        assert!(dropped > 0.0 && dropped < 1e-8);
        assert!(pieces.iter().all(|p| p.len() >= 3));
        let rec = |pair: (u32, u32)| {
            recorded
                .iter()
                .find(|(k, ..)| *k == pair)
                .map(|r| (r.1, r.2))
        };
        assert_eq!(rec((0, x)), Some((false, true)), "corner segment merges");
        assert_eq!(rec((1, x)), Some((true, false)), "A's own half keeps A's");
        assert_eq!(rec((2, x)), Some((false, true)), "B's own half keeps B's");
        seen.push((rec((0, x)), rec((1, x)), rec((2, x))));
    }
    assert_eq!(seen[0], seen[1]);
}

// ─────────────────────────── Round 12: previews that keep up with the drag

/// An `n`-facet circle of radius `r` extruded 1 m, its walls stamped.
fn dense_cylinder(n: usize, r: f64) -> Object {
    let mut sk = Sketch::on_plane(xy_plane());
    sk.begin_curve_with(CurveGeom {
        center: Point3::ORIGIN,
        radius: r,
    })
    .unwrap();
    let p = |k: usize| {
        let a = k as f64 * 2.0 * PI / n as f64;
        Point3::new(r * trig::cos(a), r * trig::sin(a), 0.0)
    };
    for k in 0..n {
        sk.add_segment(p(k), p((k + 1) % n)).unwrap();
    }
    sk.end_curve();
    let region = sk.regions().keys().next().unwrap();
    Object::from_extrusion(&sk.profile(region).unwrap(), 1.0).unwrap()
}

#[test]
fn a_dense_twist_previews_what_it_costs() {
    // A 300-facet cylinder twisted 60° leaves thousands of long fanned
    // facets; most pairs whose boxes overlap lie clear of each other's
    // planes, and the self-intersection check passes over them for a
    // fraction of a full test. The preview is charged that fraction, so it
    // previews under the drag's cap and builds exactly what the commit does,
    // while a twist four times as far still gives up.
    let o = dense_cylinder(300, 0.3);
    let twist = |deg: f64| DeformSpec::Twist {
        frame: z_frame(1.0),
        angle: deg.to_radians(),
        stations: None,
    };
    let preview = o.deform_preview(&twist(60.0)).unwrap();
    let committed = o.deformed(&twist(60.0)).unwrap();
    assert!(kernel::same_geometry_for_test(&preview, &committed, 0.0));
    let (passed, wrong) = kernel::patch::plane_shortcut_census_for_test(&committed);
    assert!(passed > 10 * committed.faces().len(), "{passed} passed");
    assert_eq!(wrong, 0);
    assert_eq!(
        o.deform_preview(&twist(240.0)).unwrap_err(),
        ShapeError::PreviewTooLarge
    );
    assert_solid(&o.deformed(&twist(240.0)).unwrap());
}
