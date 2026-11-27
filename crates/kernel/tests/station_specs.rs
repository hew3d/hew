//! Executable specs for the skin builder (true-curves.md §5.2, §10): the
//! one assembly path generated sweeps share (Follow Me twist and end
//! scale, Deform, Loft).

use kernel::math::{Point3, Vec3};
use kernel::shape_error::ShapeError;
use kernel::station::{SkinSpecForTest, build_skin_for_test};
use kernel::topo::{Object, WatertightState};
use kernel::trig;

fn square(z: f64, half: f64, angle: f64) -> Vec<Point3> {
    (0..4)
        .map(|k| {
            let a = angle + std::f64::consts::FRAC_PI_4 + k as f64 * std::f64::consts::FRAC_PI_2;
            let r = half * std::f64::consts::SQRT_2;
            Point3::new(r * trig::cos(a), r * trig::sin(a), z)
        })
        .collect()
}

fn spec(stations: Vec<Vec<Vec<Point3>>>) -> SkinSpecForTest {
    let column_smooth = stations[0].iter().map(|l| vec![false; l.len()]).collect();
    let n = stations.len();
    SkinSpecForTest {
        stations,
        column_smooth,
        station_smooth: vec![false; n],
        cap_start: true,
        cap_end: true,
    }
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

fn soft_count(o: &Object) -> usize {
    o.edges().values().filter(|e| e.soft).count()
}

fn assert_solid(o: &Object) {
    o.validate().unwrap();
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!(six_volume(o) > 0.0, "oriented outward");
}

#[test]
fn translated_squares_make_a_box() {
    let o = build_skin_for_test(&spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![square(1.0, 0.5, 0.0)],
    ]))
    .unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 6, "planar quads, two caps");
    assert!((six_volume(&o) / 6.0 - 1.0).abs() < 1e-12);
    assert_eq!(soft_count(&o), 0);
}

#[test]
fn reversed_rings_still_orient_outward() {
    let mut s = spec(vec![
        vec![square(1.0, 0.5, 0.0)],
        vec![square(0.0, 0.5, 0.0)],
    ]);
    s.stations[0][0].reverse();
    s.stations[1][0].reverse();
    let o = build_skin_for_test(&s).unwrap();
    assert_solid(&o);
    assert!((six_volume(&o) / 6.0 - 1.0).abs() < 1e-12);
}

#[test]
fn rotated_square_triangulates_with_soft_diagonals() {
    let o = build_skin_for_test(&spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![square(1.0, 0.5, std::f64::consts::FRAC_PI_4)],
    ]))
    .unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 8 + 2);
    assert_eq!(soft_count(&o), 4, "one soft diagonal per split quad");
}

#[test]
fn square_to_apex_is_a_pyramid() {
    let mut s = spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![vec![Point3::new(0.0, 0.0, 1.0)]],
    ]);
    s.cap_end = false;
    let o = build_skin_for_test(&s).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 4 + 1);
    assert!((six_volume(&o) / 6.0 - 1.0 / 3.0).abs() < 1e-12);
}

#[test]
fn annular_rings_make_a_tube_with_holed_caps() {
    let ring = |z: f64| {
        let mut hole = square(z, 0.25, 0.0);
        hole.reverse();
        vec![square(z, 0.5, 0.0), hole]
    };
    let o = build_skin_for_test(&spec(vec![ring(0.0), ring(1.0)])).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 4 + 4 + 2);
    assert!((six_volume(&o) / 6.0 - 0.75).abs() < 1e-12);
    let holed = o
        .faces()
        .values()
        .filter(|f| !f.inner_loops.is_empty())
        .count();
    assert_eq!(holed, 2);
}

#[test]
fn smooth_columns_and_stations_are_soft() {
    let mut s = spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![square(1.0, 0.5, 0.0)],
        vec![square(2.0, 0.5, 0.0)],
    ]);
    s.column_smooth = vec![vec![true, false, false, false]];
    s.station_smooth = vec![false, true, false];
    let o = build_skin_for_test(&s).unwrap();
    assert_solid(&o);
    // Column 0 across two bands, and the middle ring's four edges.
    assert_eq!(soft_count(&o), 2 + 4);
}

#[test]
fn a_folded_ring_sequence_refuses_self_intersecting() {
    // Up a unit, then back down and sideways: the third band runs back
    // through the first one's walls.
    let shift = |ring: Vec<Point3>, dx: f64| {
        ring.into_iter()
            .map(|p| p + Vec3::new(dx, 0.0, 0.0))
            .collect()
    };
    let err = build_skin_for_test(&spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![square(1.0, 0.5, 0.0)],
        vec![shift(square(0.5, 0.5, 0.0), 0.3)],
    ]))
    .unwrap_err();
    assert_eq!(err, ShapeError::SelfIntersecting { limit: None });
}

#[test]
fn mismatched_rings_refuse_degenerate() {
    let err = build_skin_for_test(&spec(vec![
        vec![square(0.0, 0.5, 0.0)],
        vec![square(1.0, 0.5, 0.0)[..3].to_vec()],
    ]))
    .unwrap_err();
    assert_eq!(err, ShapeError::ShapeDegenerate);
}
