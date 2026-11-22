//! Executable specs for Follow Me twist and end scale (true-curves.md
//! §5.2, decision I-18): stations inside Follow Me's own pipeline, rings
//! turned and scaled about the transported profile centroid, assembled by
//! the skin builder.

use kernel::trig;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, PI};

use kernel::SweepOptions;
use kernel::math::{Plane, Point3, Vec3};
use kernel::ops::FollowMeError;
use kernel::sketch::{CurveGeom, Profile, Sketch};
use kernel::topo::{Object, WatertightState};

fn xy_plane() -> Plane {
    Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    Profile::new(
        xy_plane(),
        vec![
            Point3::new(x0, y0, 0.0),
            Point3::new(x1, y0, 0.0),
            Point3::new(x1, y1, 0.0),
            Point3::new(x0, y1, 0.0),
        ],
        vec![],
    )
    .unwrap()
}

fn opts(twist: f64, end_scale: f64) -> SweepOptions {
    SweepOptions {
        twist,
        end_scale,
        stations: None,
    }
}

fn rotate(p: Point3, c: Point3, axis: Vec3, angle: f64) -> Point3 {
    let v = p - c;
    let (s, co) = trig::sin_cos(angle);
    c + v * co + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - co))
}

fn six_volume(o: &Object) -> f64 {
    let mut v = 0.0;
    for f in o.faces().values() {
        for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<Vec3> = o.loop_positions(lid).map(|q| q.to_vec()).collect();
            for k in 1..p.len() - 1 {
                v += p[0].dot(p[k].cross(p[k + 1]));
            }
        }
    }
    v
}

fn assert_solid(o: &Object) {
    o.validate().unwrap();
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!(six_volume(o) > 0.0);
}

/// The vertices of `o` lying on the plane `n·x = d`.
fn on_plane(o: &Object, n: Vec3, d: f64) -> Vec<Point3> {
    o.vertices()
        .values()
        .map(|v| v.position)
        .filter(|p| (p.to_vec().dot(n) - d).abs() < 1e-9)
        .collect()
}

fn contains(set: &[Point3], p: Point3, eps: f64) -> bool {
    set.iter().any(|q| (*q - p).length() <= eps)
}

/// Edges whose both endpoints satisfy `pred`, split (soft, hard).
fn edges_where(o: &Object, pred: impl Fn(Point3) -> bool) -> (usize, usize) {
    let (mut soft, mut hard) = (0, 0);
    for (eid, e) in o.edges() {
        let (a, b) = o.edge_endpoints(eid).unwrap();
        if pred(a) && pred(b) {
            if e.soft {
                soft += 1;
            } else {
                hard += 1;
            }
        }
    }
    (soft, hard)
}

/// A Follow Me fixture: profile, path, closed, path curves, stop.
type Case = (
    Profile,
    Vec<Point3>,
    bool,
    Vec<Option<CurveGeom>>,
    Option<f64>,
);

#[test]
fn defaults_are_byte_identical() {
    let straight = [Point3::ORIGIN, Point3::new(0.0, 0.0, 2.0)];
    let elbow = [
        Point3::ORIGIN,
        Point3::new(0.0, 0.0, 2.0),
        Point3::new(2.0, 0.0, 2.0),
    ];
    // A ring path around a yz rectangle profile.
    let yz = Profile::new(
        Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap(),
        vec![
            Point3::new(1.0, -0.2, -0.2),
            Point3::new(1.0, 0.2, -0.2),
            Point3::new(1.0, 0.2, 0.2),
            Point3::new(1.0, -0.2, 0.2),
        ],
        vec![],
    )
    .unwrap();
    let ring = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    // A circle path with attribution and a radial profile (a torus).
    let n = 24;
    let circle: Vec<Point3> = (0..n)
        .map(|k| {
            let a = k as f64 * 2.0 * PI / n as f64;
            Point3::new(2.0 * trig::cos(a), 2.0 * trig::sin(a), 0.0)
        })
        .collect();
    let g = CurveGeom {
        center: Point3::ORIGIN,
        radius: 2.0,
    };
    let curves = vec![Some(g); n];
    let radial = Profile::new(
        Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 1.0, 0.0)).unwrap(),
        vec![
            Point3::new(1.8, 0.0, -0.2),
            Point3::new(1.8, 0.0, 0.2),
            Point3::new(2.2, 0.0, 0.2),
            Point3::new(2.2, 0.0, -0.2),
        ],
        vec![],
    )
    .unwrap();
    let cases: Vec<Case> = vec![
        (
            rect(0.0, 0.0, 1.0, 1.0),
            straight.to_vec(),
            false,
            vec![],
            None,
        ),
        (
            rect(-0.1, -0.1, 0.1, 0.1),
            elbow.to_vec(),
            false,
            vec![],
            None,
        ),
        (yz, ring.to_vec(), true, vec![], None),
        (radial.clone(), circle.clone(), true, curves.clone(), None),
        (radial, circle, true, curves, Some(3.0)),
    ];
    for (profile, path, closed, curves, stop) in cases {
        let reference = match stop {
            None => Object::from_follow_me(&profile, &path, closed, &curves),
            Some(s) => Object::from_follow_me_to(&profile, &path, closed, &curves, s),
        }
        .unwrap();
        let with = Object::from_follow_me_with(
            &profile,
            &path,
            closed,
            &curves,
            stop,
            SweepOptions::default(),
        )
        .unwrap();
        assert_eq!(format!("{reference:?}"), format!("{with:?}"));
    }
}

#[test]
fn twisted_square_bar() {
    let profile = rect(-0.05, -0.05, 0.05, 0.05);
    let path = [Point3::ORIGIN, Point3::new(0.0, 0.0, 1.0)];
    let o = Object::from_follow_me_with(&profile, &path, false, &[], None, opts(FRAC_PI_2, 1.0))
        .unwrap();
    assert_solid(&o);
    let end = on_plane(&o, Vec3::new(0.0, 0.0, 1.0), 1.0);
    assert_eq!(end.len(), 4);
    for &p in profile.outer() {
        let want = rotate(p, Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0), FRAC_PI_2)
            + Vec3::new(0.0, 0.0, 1.0);
        assert!(contains(&end, want, 1e-12), "{want:?} not on the end cap");
    }
    // 90° in 15° steps: 6 bands. Corner columns (6 × 4) and the two cap
    // rings (8) are hard; the 5 inner rings (20) and every split quad's
    // diagonal (24) are soft.
    let (soft, hard) = edges_where(&o, |_| true);
    assert_eq!((soft, hard), (44, 32));
}

#[test]
fn scale_only_frustum() {
    let profile = rect(0.0, 0.0, 0.2, 0.2);
    let path = [Point3::new(0.1, 0.1, 0.0), Point3::new(0.1, 0.1, 1.0)];
    let o = Object::from_follow_me_with(&profile, &path, false, &[], None, opts(0.0, 0.5)).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 6, "planar side quads");
    let (a0, a1, h) = (0.04_f64, 0.01_f64, 1.0);
    let want = (a0 + a1 + (a0 * a1).sqrt()) * h / 3.0;
    let got = six_volume(&o) / 6.0;
    assert!(((got - want) / want).abs() < 1e-12, "{got} vs {want}");
}

/// A half-disc airfoil: an arc of radius 0.1 over a straight chord, built
/// the way the drawing tools build one (curve bracket with its circle).
fn airfoil() -> Profile {
    let mut sk = Sketch::on_plane(xy_plane());
    let n = 8;
    let p = |k: usize| {
        let a = k as f64 * PI / n as f64;
        Point3::new(0.1 * trig::cos(a), 0.1 * trig::sin(a), 0.0)
    };
    sk.begin_curve_with(CurveGeom {
        center: Point3::ORIGIN,
        radius: 0.1,
    })
    .unwrap();
    for k in 0..n {
        sk.add_segment(p(k), p(k + 1)).unwrap();
    }
    sk.end_curve();
    sk.add_segment(p(n), p(0)).unwrap();
    let region = sk.regions().keys().next().unwrap();
    sk.profile(region).unwrap()
}

#[test]
fn fan_blade() {
    let profile = airfoil();
    let path = [Point3::new(0.0, 0.04, 0.0), Point3::new(0.0, 0.04, 1.0)];
    let twist = 20f64.to_radians();
    let o =
        Object::from_follow_me_with(&profile, &path, false, &[], None, opts(twist, 0.4)).unwrap();
    assert_solid(&o);
    // 20° in 15° steps: 2 bands; station 1 at half the length.
    let outer = profile.outer();
    let c = {
        let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for k in 0..outer.len() {
            let (p, q) = (outer[k], outer[(k + 1) % outer.len()]);
            let cr = p.x * q.y - q.x * p.y;
            a += cr;
            cx += (p.x + q.x) * cr;
            cy += (p.y + q.y) * cr;
        }
        Point3::new(cx / (3.0 * a), cy / (3.0 * a), 0.0)
    };
    let mid = |p: Point3| {
        let r = rotate(p, c, Vec3::new(0.0, 0.0, 1.0), twist / 2.0);
        c + (r - c) * 0.7 + Vec3::new(0.0, 0.0, 0.5)
    };
    let soft_column = |p: Point3| -> bool {
        let q = mid(p);
        o.edges()
            .iter()
            .find(|&(eid, _)| {
                let (a, b) = o.edge_endpoints(eid).unwrap();
                ((a - p).length() < 1e-9 && (b - q).length() < 1e-9)
                    || ((b - p).length() < 1e-9 && (a - q).length() < 1e-9)
            })
            .map(|(_, e)| e.soft)
            .expect("a column edge")
    };
    for &p in outer {
        let corner = (p.y).abs() < 1e-12; // the chord's two ends
        assert_eq!(soft_column(p), !corner, "column at {p:?}");
    }
}

#[test]
fn closed_path_refused() {
    let yz = Profile::new(
        Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap(),
        vec![
            Point3::new(1.0, -0.2, -0.2),
            Point3::new(1.0, 0.2, -0.2),
            Point3::new(1.0, 0.2, 0.2),
            Point3::new(1.0, -0.2, 0.2),
        ],
        vec![],
    )
    .unwrap();
    let ring = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let err = Object::from_follow_me_with(&yz, &ring, true, &[], None, opts(0.3, 1.0)).unwrap_err();
    assert_eq!(err, FollowMeError::TwistNeedsOpenPath);
}

#[test]
fn bad_options_refused() {
    let profile = rect(-0.05, -0.05, 0.05, 0.05);
    let path = [Point3::ORIGIN, Point3::new(0.0, 0.0, 1.0)];
    for o in [
        opts(0.0, 0.0),
        opts(f64::NAN, 1.0),
        opts(0.0, f64::INFINITY),
        SweepOptions {
            stations: Some(0),
            ..opts(0.5, 1.0)
        },
    ] {
        let err = Object::from_follow_me_with(&profile, &path, false, &[], None, o).unwrap_err();
        assert_eq!(err, FollowMeError::SweepDegenerate);
    }
}

#[test]
fn huge_twist_or_station_count_refuses() {
    // A typed 1,000,000° or a station count in the billions refuses at once
    // instead of building millions of stations; 50 turns still sweeps.
    let profile = rect(-0.05, -0.05, 0.05, 0.05);
    let path = [Point3::ORIGIN, Point3::new(0.0, 0.0, 1.0)];
    for o in [
        opts(1.0e6_f64.to_radians(), 1.0),
        SweepOptions {
            stations: Some(u32::MAX),
            ..opts(0.5, 1.0)
        },
    ] {
        let err = Object::from_follow_me_with(&profile, &path, false, &[], None, o).unwrap_err();
        assert_eq!(err, FollowMeError::SweepDegenerate);
    }
    let fifty = opts(50.0 * std::f64::consts::TAU, 1.0);
    assert_solid(&Object::from_follow_me_with(&profile, &path, false, &[], None, fifty).unwrap());
}

#[test]
fn stop_with_twist() {
    let profile = rect(-0.05, -0.05, 0.05, 0.05);
    let path = [Point3::ORIGIN, Point3::new(0.0, 0.0, 2.0)];
    let o =
        Object::from_follow_me_with(&profile, &path, false, &[], Some(1.0), opts(FRAC_PI_2, 1.0))
            .unwrap();
    assert_solid(&o);
    let end = on_plane(&o, Vec3::new(0.0, 0.0, 1.0), 1.0);
    assert_eq!(end.len(), 4, "the sweep stops at 1 m");
    for &p in profile.outer() {
        let want = rotate(p, Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0), FRAC_PI_2)
            + Vec3::new(0.0, 0.0, 1.0);
        assert!(
            contains(&end, want, 1e-12),
            "the whole twist over the truncated length"
        );
    }
}

#[test]
fn auto_oriented_profile_twists() {
    // A flat ground rectangle and a ground path: folded upright onto the
    // plane y = 0.5 (as `follow_me_folds_a_parallel_profile_upright`).
    let profile = rect(0.0, 0.0, 1.0, 1.0);
    let path = [Point3::ORIGIN, Point3::new(0.0, 5.0, 0.0)];
    let o = Object::from_follow_me_with(&profile, &path, false, &[], None, opts(FRAC_PI_4, 1.0))
        .unwrap();
    assert_solid(&o);
    let y = Vec3::new(0.0, 1.0, 0.0);
    let start = on_plane(&o, y, 0.5);
    let end = on_plane(&o, y, 5.5);
    assert_eq!((start.len(), end.len()), (4, 4));
    let c = start.iter().fold(Vec3::ZERO, |acc, p| acc + p.to_vec()) / 4.0;
    let c = Point3::ORIGIN + c;
    for &p in &start {
        let want = rotate(p, c, y, FRAC_PI_4) + y * 5.0;
        assert!(contains(&end, want, 1e-9), "{want:?}");
    }
}

#[test]
fn tapered_blade_does_not_lean() {
    let profile = rect(0.0, 0.0, 1.0, 0.5);
    let path = [Point3::ORIGIN, Point3::new(0.0, 0.0, 1.0)];
    let o = Object::from_follow_me_with(&profile, &path, false, &[], None, opts(0.0, 0.5)).unwrap();
    assert_solid(&o);
    let end = on_plane(&o, Vec3::new(0.0, 0.0, 1.0), 1.0);
    assert_eq!(end.len(), 4);
    let c = end.iter().fold(Vec3::ZERO, |acc, p| acc + p.to_vec()) / 4.0;
    assert!((c - Vec3::new(0.5, 0.25, 1.0)).length() < 1e-12, "{c:?}");
}

#[test]
fn sharp_path_corner_stays_hard() {
    let profile = rect(-0.05, -0.05, 0.05, 0.05);
    let path = [
        Point3::ORIGIN,
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
    ];
    // 60° over 2 m, two stations per leg.
    let o = Object::from_follow_me_with(
        &profile,
        &path,
        false,
        &[],
        None,
        SweepOptions {
            stations: Some(2),
            ..opts(FRAC_PI_3, 1.0)
        },
    )
    .unwrap();
    assert_solid(&o);
    // The corner's miter plane x + z = 1: its ring stays hard.
    let (soft, hard) = edges_where(&o, |p| (p.x + p.z - 1.0).abs() < 1e-9);
    assert_eq!((soft, hard), (0, 4));
    // The inserted station on the first leg (z = 0.5): soft.
    let (soft, hard) = edges_where(&o, |p| (p.z - 0.5).abs() < 1e-9);
    assert_eq!((soft, hard), (4, 0));
}
