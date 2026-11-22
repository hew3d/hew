//! Executable specs for Loft (true-curves.md §5.4; decisions I-17, I-23).

use kernel::trig;
use std::f64::consts::{PI, TAU};

use kernel::loft::{LoftProfile, LoftSpec};
use kernel::math::{Plane, Point3, Vec3};
use kernel::shape_error::ShapeError;
use kernel::sketch::CurveGeom;
use kernel::topo::{Object, SurfaceRef, WatertightState};

fn profile(points: Vec<Point3>) -> LoftProfile {
    LoftProfile {
        points,
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A square of half-width `h` at height `z`, starting at (+h, −h).
fn square(h: f64, z: f64) -> LoftProfile {
    profile(vec![
        Point3::new(h, -h, z),
        Point3::new(h, h, z),
        Point3::new(-h, h, z),
        Point3::new(-h, -h, z),
    ])
}

/// An `n`-gon circle of radius `r` at height `z` (center on the z axis),
/// every edge attributed to the circle, starting at angle `phase`.
fn circle(r: f64, z: f64, n: usize, phase: f64) -> LoftProfile {
    let g = CurveGeom {
        center: Point3::new(0.0, 0.0, z),
        radius: r,
    };
    LoftProfile {
        points: (0..n)
            .map(|k| {
                let a = phase + TAU * k as f64 / n as f64;
                Point3::new(r * trig::cos(a), r * trig::sin(a), z)
            })
            .collect(),
        curves: vec![Some(g); n],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

fn spec(profiles: Vec<LoftProfile>) -> LoftSpec {
    LoftSpec {
        profiles,
        apex: None,
        rails: Vec::new(),
        stations: None,
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

/// The vertices on the plane z = `z`.
fn ring_at(o: &Object, z: f64) -> Vec<Point3> {
    o.vertices()
        .values()
        .map(|v| v.position)
        .filter(|p| (p.z - z).abs() < 1e-12)
        .collect()
}

/// Longitudinal edges (spanning z0 → z1), split (soft, hard).
fn columns(o: &Object, z0: f64, z1: f64) -> (usize, usize) {
    let (mut soft, mut hard) = (0, 0);
    for (eid, e) in o.edges() {
        let (a, b) = o.edge_endpoints(eid).unwrap();
        let spans = ((a.z - z0).abs() < 1e-12 && (b.z - z1).abs() < 1e-12)
            || ((a.z - z1).abs() < 1e-12 && (b.z - z0).abs() < 1e-12);
        if spans {
            if e.soft {
                soft += 1;
            } else {
                hard += 1;
            }
        }
    }
    (soft, hard)
}

#[test]
fn two_squares_ruled_box() {
    let o = Object::from_loft(&spec(vec![square(0.5, 0.0), square(0.5, 1.0)])).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 6);
    assert!((volume(&o) - 1.0).abs() < 1e-12);
}

#[test]
fn reversed_and_rotated_inputs_orient_and_align() {
    // The second square wound the other way and starting elsewhere: the
    // loft orients both CCW about the loft direction and starts the second
    // at its vertex nearest the first's start, so it is still a box.
    let mut top = square(0.5, 1.0);
    top.points.reverse();
    top.points.rotate_left(1);
    let o = Object::from_loft(&spec(vec![square(0.5, 0.0), top])).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 6);
}

/// The fold angle (degrees) across edge `e` between its two faces.
fn fold_deg(o: &Object, e: kernel::EdgeId) -> f64 {
    let (f, g) = o.edge_faces_for_test(e).unwrap();
    let d = (o.faces()[f].plane.normal() - o.faces()[g].plane.normal()).length();
    2.0 * trig::asin(d / 2.0).to_degrees()
}

/// A square to a circle as large as it, square-on: no wall edge is hard,
/// columns and diagonals alike. The square's corner columns are a corner
/// in one profile only; the band fans each corner out (D-126 faceting),
/// and there the corner column folds 7.34° and the fan diagonals 41.13°,
/// both under twice the 33.68° of the plain quads beside them (the larger
/// of that and the circle's 7.5° facet floor, D-128), so the surface reads
/// smooth (D-126 R4). Before, the four corner columns were hard all the
/// way up.
#[test]
fn square_to_circle() {
    let o = Object::from_loft(&spec(vec![
        square(0.5, 0.0),
        circle(0.5, 1.0, 48, -PI / 4.0),
    ]))
    .unwrap();
    assert_solid(&o);
    // M = 48 samples per ring.
    assert_eq!(ring_at(&o, 0.0).len(), 48);
    assert_eq!(ring_at(&o, 1.0).len(), 48);
    let (soft, hard) = columns(&o, 0.0, 1.0);
    assert_eq!(hard, 0, "no hard wall edge, columns and diagonals alike");
    assert!(soft >= 48, "{soft}");
}

/// A circle to a square meets each corner square-on (the corner column
/// ends at 45° on the circle, not a corner's step round), whichever is
/// drawn first, and reads smooth: no hard wall edge (D-126 R3, R4).
#[test]
fn a_circle_to_a_square_lofts_square_on() {
    let circle_first =
        Object::from_loft(&spec(vec![circle(0.5, 1.0, 48, 0.0), square(0.5, 0.0)])).unwrap();
    let square_first =
        Object::from_loft(&spec(vec![square(0.5, 0.0), circle(0.5, 1.0, 48, 0.0)])).unwrap();
    for o in [&circle_first, &square_first] {
        assert_solid(o);
        let corner = Point3::new(0.5, 0.5, 0.0);
        let at45 = Point3::new(0.5 * trig::cos(PI / 4.0), 0.5 * trig::sin(PI / 4.0), 1.0);
        let joined = o.edges().keys().any(|e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            ((p - corner).length() < 1e-12 && (q - at45).length() < 1e-12)
                || ((q - corner).length() < 1e-12 && (p - at45).length() < 1e-12)
        });
        assert!(joined, "the corner's column ends at 45° on the circle");
        assert_eq!(hard_longitudinal(o).len(), 0, "no hard wall edge");
    }
    assert!(same_points(
        &vertex_set(&circle_first),
        &vertex_set(&square_first)
    ));
    assert!((volume(&circle_first) - volume(&square_first)).abs() < 1e-12);
}

#[test]
fn mixed_counts_stay_on_circles() {
    let o = Object::from_loft(&spec(vec![
        circle(0.1, 0.0, 24, 0.0),
        circle(0.15, 1.0, 32, 0.0),
    ]))
    .unwrap();
    assert_solid(&o);
    // U = the union of 24 and 32 equal fractions: 24 + 32 − 8 shared.
    let m = 24 + 32 - 8;
    for (z, r) in [(0.0, 0.1), (1.0, 0.15)] {
        let ring = ring_at(&o, z);
        assert_eq!(ring.len(), m);
        for p in ring {
            assert!(
                ((p.x * p.x + p.y * p.y).sqrt() - r).abs() < 1e-12,
                "{p:?} off its circle"
            );
        }
    }
    let walls: Vec<_> = o
        .faces()
        .values()
        .filter(|f| f.plane.normal().z.abs() < 1.0 - 1e-9)
        .collect();
    assert_eq!(walls.len(), m);
    assert!(
        walls
            .iter()
            .all(|f| matches!(f.surface, Some(SurfaceRef::Cone { .. }))),
        "every band verifies on the cone"
    );
}

/// A triangle to a square keeps every corner of both, and creases exactly
/// where the faces fold: no quad there is plain (every one has a corner
/// column), so every reference is 0 and a wall edge is hard where it folds
/// more than a degree. Among them the square's corner at (−0.5, −0.5),
/// which falls on a diagonal (90°), and every fan diagonal (35.3°, 65.9°);
/// the one-sided columns lie in flat faces (0°) and are soft (D-126).
#[test]
fn triangle_to_square_keeps_all_corners() {
    let tri = profile(vec![
        Point3::new(0.5, -0.5, 0.0),
        Point3::new(0.0, 0.5, 0.0),
        Point3::new(-0.5, -0.5, 0.0),
    ]);
    let o = Object::from_loft(&spec(vec![tri.clone(), square(0.5, 1.0)])).unwrap();
    assert_solid(&o);
    let bottom = ring_at(&o, 0.0);
    let top = ring_at(&o, 1.0);
    assert_eq!(bottom.len(), top.len());
    for c in &tri.points {
        assert!(bottom.iter().any(|p| (*p - *c).length() < 1e-12));
    }
    for c in &square(0.5, 1.0).points {
        assert!(top.iter().any(|p| (*p - *c).length() < 1e-12));
    }
    let crease = 2.0 * trig::asin(kernel::tol::LOFT_CREASE_FOLD / 2.0).to_degrees();
    let mut named = false;
    for (eid, e) in o.edges() {
        let (a, b) = o.edge_endpoints(eid).unwrap();
        if (a.z - b.z).abs() < 1e-9 {
            continue;
        }
        let fold = fold_deg(&o, eid);
        assert_eq!(!e.soft, fold > crease, "{a:?}–{b:?} folds {fold}°");
        if (a.x + 0.5).abs() < 1e-12
            && (a.y + 0.5).abs() < 1e-12
            && (b.x + 0.5).abs() < 1e-12
            && (b.y + 0.5).abs() < 1e-12
        {
            named = true;
            assert!(!e.soft && (fold - 90.0).abs() < 1e-6, "{fold}");
        }
    }
    assert!(named, "the diagonal at the square's corner (−0.5, −0.5)");
}

/// A rectangle of half-sizes `w` × `h` at height `z`, starting at (w, −h).
fn rect(w: f64, h: f64, z: f64) -> LoftProfile {
    profile(vec![
        Point3::new(w, -h, z),
        Point3::new(w, h, z),
        Point3::new(-w, h, z),
        Point3::new(-w, -h, z),
    ])
}

/// The sorted vertex positions of `o`.
fn vertex_set(o: &Object) -> Vec<Point3> {
    let mut v: Vec<Point3> = o.vertices().values().map(|v| v.position).collect();
    v.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    v
}

/// The same points, each within 1e-12 of one of the other's.
fn same_points(a: &[Point3], b: &[Point3]) -> bool {
    let near = |x: &[Point3], y: &[Point3]| {
        x.iter()
            .all(|p| y.iter().any(|q| (*p - *q).length() < 1e-12))
    };
    a.len() == b.len() && near(a, b) && near(b, a)
}

/// Hard longitudinal edges (not within one plane z = const).
fn hard_longitudinal(o: &Object) -> Vec<(Point3, Point3)> {
    o.edges()
        .iter()
        .filter_map(|(eid, e)| {
            let (a, b) = o.edge_endpoints(eid).unwrap();
            ((a.z - b.z).abs() > 1e-9 && !e.soft).then_some((a, b))
        })
        .collect()
}

/// Rectangles of different proportions (the real-app S2 deck: 1.0 × 0.6,
/// 1.2 × 0.8, 1.1 × 0.7) loft corner to corner, whichever vertex each was
/// drawn from: every ring is the four corners, the four corner columns are
/// the only creases, every wall face is a quad and the deck's rim is its
/// four sides. Before (perimeter mode alone), the corners at different
/// perimeter fractions became clusters of columns with sliver faces and
/// short deck pieces (D-126 R1).
#[test]
fn rectangles_of_different_aspect_loft_corner_to_corner() {
    let base = [(0.5, 0.3, 0.0), (0.6, 0.4, 0.5), (0.55, 0.35, 1.0)];
    let build = |turn: [usize; 3]| {
        let profiles = base
            .iter()
            .zip(turn)
            .map(|(&(w, h, z), t)| {
                let mut p = rect(w, h, z);
                p.points.rotate_left(t);
                p
            })
            .collect();
        Object::from_loft(&spec(profiles)).unwrap()
    };
    let o = build([0, 0, 0]);
    assert_solid(&o);
    for z in [0.0, 0.5, 1.0] {
        assert_eq!(ring_at(&o, z).len(), 4, "ring at {z}");
    }
    let hard = hard_longitudinal(&o);
    let columns: Vec<Point3> = hard
        .iter()
        .filter_map(|&(a, b)| {
            if a.z.abs() < 1e-12 {
                Some(a)
            } else if b.z.abs() < 1e-12 {
                Some(b)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(columns.len(), 4, "four corner columns");
    for c in &columns {
        assert!(
            (c.x.abs() - 0.5).abs() < 1e-12 && (c.y.abs() - 0.3).abs() < 1e-12,
            "{c:?}"
        );
    }
    let walls = o
        .faces()
        .values()
        .filter(|f| f.plane.normal().z.abs() < 1.0 - 1e-9);
    for f in walls {
        assert_eq!(
            o.loop_positions(f.outer_loop).count(),
            4,
            "a quad wall face"
        );
    }
    let rim = o
        .sharp_edges()
        .into_iter()
        .filter(|&e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            (a.z - 1.0).abs() < 1e-12 && (b.z - 1.0).abs() < 1e-12
        })
        .count();
    assert_eq!(rim, 4, "the deck rim is four sides");
    let want = vertex_set(&o);
    for i in 0..3 {
        for t in 1..4 {
            let mut turn = [0; 3];
            turn[i] = t;
            assert!(
                same_points(&vertex_set(&build(turn)), &want),
                "profile {i} turned {t}"
            );
        }
    }
}

/// A square much larger than the circle it meets fans each corner out to
/// the circle too: measured, the corner column folds 3.33° and its fan
/// diagonals 22.58°, beside plain quads whose soft edges fold up to 19.05°
/// (a limit of 38.1°), so no wall edge is hard; nowhere does the band
/// fold the 30°-plus of a ridge (D-126 R4).
#[test]
fn a_large_square_to_a_small_circle_fans_its_corners_out() {
    let o = Object::from_loft(&spec(vec![square(0.5, 0.0), circle(0.1, 0.3, 48, 0.0)])).unwrap();
    assert_solid(&o);
    assert_eq!(hard_longitudinal(&o).len(), 0);
    let corner = Point3::new(0.5, -0.5, 0.0);
    let mut folds: Vec<f64> = o
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p - corner).length() < 1e-12 || (q - corner).length() < 1e-12
        })
        .filter(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p.z - q.z).abs() > 1e-9
        })
        .map(|e| fold_deg(&o, e))
        .collect();
    folds.sort_by(f64::total_cmp);
    // The corner column, then its two fan diagonals.
    assert_eq!(folds.len(), 3, "{folds:?}");
    assert!((folds[0] - 3.33).abs() < 0.01, "{folds:?}");
    assert!(
        (folds[1] - 22.58).abs() < 0.01 && (folds[2] - 22.58).abs() < 0.01,
        "{folds:?}"
    );
}

/// Parallel profiles offset sideways (a sheared box) start at the corner
/// that matches, not at the vertex nearest in space: the loft is the
/// sheared box, six planar faces. Before, the second square started at
/// the vertex nearest the first's start, a corner over, and twisted
/// (KB-32).
#[test]
fn sheared_parallel_profiles_loft_without_twist() {
    let mut top = square(0.5, 1.0);
    for p in &mut top.points {
        p.x += 0.8;
    }
    let o = Object::from_loft(&spec(vec![square(0.5, 0.0), top])).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 6);
    assert!((volume(&o) - 1.0).abs() < 1e-12);
}

/// The start grips are the corners where every profile has the same
/// number of corners: a rectangle drawn with a point in the middle of one
/// side offers four starts, not five, and an offset of one turns the
/// correspondence by one corner (D-126 R2).
#[test]
fn start_grips_count_corners() {
    let five = profile(vec![
        Point3::new(0.5, -0.3, 0.0),
        Point3::new(0.5, 0.3, 0.0),
        Point3::new(0.0, 0.3, 0.0),
        Point3::new(-0.5, 0.3, 0.0),
        Point3::new(-0.5, -0.3, 0.0),
    ]);
    let starts =
        kernel::loft::start_points(&spec(vec![five.clone(), rect(0.5, 0.3, 1.0)])).unwrap();
    assert_eq!(starts.iter().map(Vec::len).collect::<Vec<_>>(), vec![4, 4]);
    let mut top = rect(0.5, 0.3, 1.0);
    top.start_offset = 1;
    let o = Object::from_loft(&spec(vec![five, top])).unwrap();
    assert_solid(&o);
    // The bottom's start corner now meets the top's next corner.
    let joined = |a: Point3, b: Point3| {
        o.edges().keys().any(|e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            ((p - a).length() < 1e-12 && (q - b).length() < 1e-12)
                || ((p - b).length() < 1e-12 && (q - a).length() < 1e-12)
        })
    };
    assert!(joined(
        Point3::new(0.5, -0.3, 0.0),
        Point3::new(0.5, 0.3, 1.0)
    ));
    // Twisted, so the sides are no longer the box's planar quads.
    assert!(o.faces().len() > 7, "{}", o.faces().len());
}

/// A rounded rectangle of half-sizes `w` × `h` with corner radius `r` at
/// height `z`: four lines and four quarter arcs, each arc one edge
/// carrying its circle, every junction tangent.
fn rounded_rect(w: f64, h: f64, r: f64, z: f64) -> LoftProfile {
    let p = |x: f64, y: f64| Point3::new(x, y, z);
    let arc = |cx: f64, cy: f64| {
        Some(CurveGeom {
            center: Point3::new(cx, cy, z),
            radius: r,
        })
    };
    LoftProfile {
        points: vec![
            p(w, -h + r),
            p(w, h - r),
            p(w - r, h),
            p(-w + r, h),
            p(-w, h - r),
            p(-w, -h + r),
            p(-w + r, -h),
            p(w - r, -h),
        ],
        curves: vec![
            None,
            arc(w - r, h - r),
            None,
            arc(-w + r, h - r),
            None,
            arc(-w + r, -h + r),
            None,
            arc(w - r, -h + r),
        ],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A line meeting an arc tangentially is no corner: rounded rectangles
/// have none, so they loft by perimeter (every vertex a start candidate)
/// and smooth all round, as does a circle meeting one. Before, the turn
/// was read from the arc's chord, each of the eight tangent junctions
/// counted as a corner, corner mode paired them and creased every one.
#[test]
fn rounded_rectangles_have_no_corners_and_loft_smooth() {
    let pair = spec(vec![
        rounded_rect(1.0, 0.6, 0.25, 0.0),
        rounded_rect(0.6, 0.4, 0.15, 1.0),
    ]);
    let starts = kernel::loft::start_points(&pair).unwrap();
    assert_eq!(starts.iter().map(Vec::len).collect::<Vec<_>>(), vec![8, 8]);
    let o = Object::from_loft(&pair).unwrap();
    assert_solid(&o);
    assert_eq!(
        hard_longitudinal(&o).len(),
        0,
        "no crease at a tangent seam"
    );
    let o = Object::from_loft(&spec(vec![
        circle(0.5, 0.0, 48, 0.0),
        rounded_rect(0.6, 0.4, 0.15, 1.0),
    ]))
    .unwrap();
    assert_solid(&o);
    assert_eq!(
        hard_longitudinal(&o).len(),
        0,
        "a circle to a rounded rectangle"
    );
    // A half disc's two junctions are corners still.
    let g = Some(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 0.5,
    });
    let half = LoftProfile {
        points: vec![
            Point3::new(0.5, 0.0, 0.0),
            Point3::new(0.0, 0.5, 0.0),
            Point3::new(-0.5, 0.0, 0.0),
        ],
        curves: vec![g, g, None],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let mut top = half.clone();
    for q in &mut top.points {
        q.z = 1.0;
    }
    top.curves = vec![
        Some(CurveGeom {
            center: Point3::new(0.0, 0.0, 1.0),
            radius: 0.5,
        });
        2
    ];
    top.curves.push(None);
    let starts = kernel::loft::start_points(&spec(vec![half, top])).unwrap();
    assert_eq!(
        starts.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![2, 2],
        "two corners each"
    );
}

/// A rounded rectangle whose corners are runs of six edges: on a quarter
/// circle as arcs (`Arc`), on it as a smooth-keyed polyline (`Spline`), or
/// on a cubic Bézier as one (`Bezier`). 28 vertices at height `z`.
#[derive(Clone, Copy)]
enum Rounding {
    Arc,
    Spline,
    Bezier,
}

fn rounded_runs(w: f64, h: f64, r: f64, z: f64, how: Rounding) -> LoftProfile {
    let mut points = Vec::new();
    let mut curves = Vec::new();
    let mut smooth = Vec::new();
    // Corner centers, counterclockwise from (+x, +y), each turning a
    // quarter from angle `a0`.
    let corners = [
        (w - r, h - r, 0.0),
        (-w + r, h - r, PI / 2.0),
        (-w + r, -h + r, PI),
        (w - r, -h + r, 1.5 * PI),
    ];
    for (key, &(cx, cy, a0)) in corners.iter().enumerate() {
        let on_circle = |t: f64| {
            let a = a0 + t * PI / 2.0;
            (cx + r * trig::cos(a), cy + r * trig::sin(a))
        };
        let (p0, p3) = (on_circle(0.0), on_circle(1.0));
        // Bézier controls along the end tangents (the circle's 0.5523).
        let k = 0.552_284_75 * r;
        let (t0, t1) = (
            (-trig::sin(a0), trig::cos(a0)),
            (-trig::sin(a0 + PI / 2.0), trig::cos(a0 + PI / 2.0)),
        );
        let c1 = (p0.0 + k * t0.0, p0.1 + k * t0.1);
        let c2 = (p3.0 - k * t1.0, p3.1 - k * t1.1);
        let bezier = |t: f64| {
            let u = 1.0 - t;
            let f = |a: f64, b: f64, c: f64, d: f64| {
                u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
            };
            (f(p0.0, c1.0, c2.0, p3.0), f(p0.1, c1.1, c2.1, p3.1))
        };
        for i in 0..=6 {
            let t = i as f64 / 6.0;
            let (x, y) = match how {
                Rounding::Bezier => bezier(t),
                _ => on_circle(t),
            };
            points.push(Point3::new(x, y, z));
            if i < 6 {
                match how {
                    Rounding::Arc => {
                        curves.push(Some(CurveGeom {
                            center: Point3::new(cx, cy, z),
                            radius: r,
                        }));
                        smooth.push(None);
                    }
                    _ => {
                        curves.push(None);
                        smooth.push(Some(key as u32));
                    }
                }
            } else {
                // The straight side to the next corner.
                curves.push(None);
                smooth.push(None);
            }
        }
    }
    LoftProfile {
        points,
        curves,
        smooth,
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A rounded rectangle whose corners are smooth-keyed polylines (a spline
/// or a Bézier, no analytic curve) has no corners either: where its
/// straight side meets a corner run, it turns less than the run's own end
/// facet (D-129 C). So it lofts by perimeter, offers every vertex as a
/// start, and meets an arc-rounded rectangle of the same count with no
/// hard wall edge. A run that meets a side with a real kink (30°, sharper
/// than its own 20° end facet) still makes a corner there: a square with
/// one corner so replaced counts four. Before, every junction of a keyed
/// run counted as a corner.
#[test]
fn smooth_keyed_rounded_corners_are_no_corners() {
    for how in [Rounding::Spline, Rounding::Bezier] {
        let pair = spec(vec![
            rounded_runs(1.0, 0.6, 0.25, 0.0, how),
            rounded_runs(0.8, 0.5, 0.2, 1.0, Rounding::Arc),
        ]);
        let starts = kernel::loft::start_points(&pair).unwrap();
        assert_eq!(
            starts.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![28, 28]
        );
        let o = Object::from_loft(&pair).unwrap();
        assert_solid(&o);
        assert_eq!(hard_longitudinal(&o).len(), 0);
        // Against its own kind too: no corners either side, so every
        // vertex is a start (eight junction corners each would offer 8).
        let same = spec(vec![
            rounded_runs(1.0, 0.6, 0.25, 0.0, how),
            rounded_runs(0.8, 0.5, 0.2, 1.0, how),
        ]);
        let starts = kernel::loft::start_points(&same).unwrap();
        assert_eq!(
            starts.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![28, 28]
        );
    }
    // A square whose (0.5, 0.5) corner is a keyed run of four chords
    // heading 120°, 140°, 160°, 175°: a 30° kink off the right side, 5° onto
    // the top (under the run's 15° end facet).
    let l = 0.05;
    let mut pts = vec![Point3::new(-0.5, -0.5, 0.0), Point3::new(0.5, -0.5, 0.0)];
    let mut at = Point3::new(0.5, 0.5 - 1.938_037 * l, 0.0);
    pts.push(at);
    for deg in [120.0_f64, 140.0, 160.0] {
        let a = deg.to_radians();
        at = Point3::new(at.x + l * trig::cos(a), at.y + l * trig::sin(a), 0.0);
        pts.push(at);
    }
    let end = Point3::new(at.x + l * trig::cos(175f64.to_radians()), 0.5, 0.0);
    pts.push(end);
    pts.push(Point3::new(-0.5, 0.5, 0.0));
    let n = pts.len();
    let mut smooth = vec![None; n];
    for key in &mut smooth[2..6] {
        *key = Some(7);
    }
    let kinked = LoftProfile {
        points: pts,
        curves: Vec::new(),
        smooth,
        through: Vec::new(),
        start_offset: 0,
    };
    let starts = kernel::loft::start_points(&spec(vec![kinked, square(0.5, 1.0)])).unwrap();
    assert_eq!(
        starts.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![4, 4],
        "four corners each"
    );
}

/// A smooth-keyed edge running straight on from a plain one is no corner
/// in any direction, as two plain collinear edges are not: a square turned
/// to any of 199 angles, one side split in two (0.37 of the way along)
/// with either part keyed, offers the same four starts as with neither
/// keyed. Before, the keyed junction compared angles from `acos` against
/// an absolute 1e-9, so rounding in the chords' directions read as a
/// corner at many angles.
#[test]
fn a_keyed_edge_straight_on_from_a_plain_one_is_no_corner() {
    for i in 0..199 {
        let turn = TAU * i as f64 / 199.0 + 0.123;
        let at = |x: f64, y: f64| {
            let (s, c) = trig::sin_cos(turn);
            Point3::new(0.5 * (c * x - s * y), 0.5 * (s * x + c * y), 0.0)
        };
        let points = vec![
            at(-1.0, -1.0),
            at(1.0, -1.0),
            at(1.0, -0.26),
            at(1.0, 1.0),
            at(-1.0, 1.0),
        ];
        let count = |smooth: Vec<Option<u32>>| {
            let p = LoftProfile {
                points: points.clone(),
                curves: Vec::new(),
                smooth,
                through: Vec::new(),
                start_offset: 0,
            };
            kernel::loft::start_points(&spec(vec![p, square(0.5, 1.0)])).unwrap()[0].len()
        };
        let plain = count(vec![None; 5]);
        assert_eq!(plain, 4, "{i}");
        for keyed in [1, 2] {
            let mut smooth = vec![None; 5];
            smooth[keyed] = Some(3);
            assert_eq!(count(smooth), plain, "{i}: edge {keyed} keyed");
        }
    }
}

/// A whole circle whose start falls between its vertices (a square's
/// corner at −45° meets a 48-gon drawn from 3°) shows its start markers
/// where the loft puts column 0: the first on that direction, the k-th
/// turned by k sides; a start offset of k lands on the k-th marker, and a
/// rail from the square's corner to that marker builds. Before, the
/// markers were the circle's vertices, an offset landed short of its
/// marker, and such a rail refused (`RailMissesProfile`).
#[test]
fn a_circle_started_between_vertices_puts_its_markers_on_its_columns() {
    let phase = 3.0_f64.to_radians();
    let base = spec(vec![square(0.5, 0.0), circle(0.4, 1.0, 48, phase)]);
    let markers = kernel::loft::start_points(&base).unwrap()[1].clone();
    assert_eq!(markers.len(), 48);
    let at = |deg: f64| {
        Point3::new(
            0.4 * trig::cos(deg.to_radians()),
            0.4 * trig::sin(deg.to_radians()),
            1.0,
        )
    };
    assert!(
        (markers[0] - at(-45.0)).length() < 1e-12,
        "{:?}",
        markers[0]
    );
    let corner = Point3::new(0.5, -0.5, 0.0);
    for k in [0usize, 1, 5] {
        assert!(
            (markers[k] - at(-45.0 + 7.5 * k as f64)).length() < 1e-12,
            "{k}"
        );
        let mut s = base.clone();
        s.profiles[1].start_offset = k as i32;
        let o = Object::from_loft(&s).unwrap();
        assert_solid(&o);
        let joined = o.edges().keys().any(|e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            ((p - corner).length() < 1e-12 && (q - markers[k]).length() < 1e-12)
                || ((q - corner).length() < 1e-12 && (p - markers[k]).length() < 1e-12)
        });
        assert!(joined, "offset {k}: the corner's column ends at marker {k}");
        // A rail through the corner and that marker.
        s.rails = vec![vec![corner, markers[k]]];
        let railed = Object::from_loft(&s);
        assert!(railed.is_ok(), "offset {k}: {:?}", railed.err());
    }
}

/// One profile and no apex is no loft: the start markers refuse as the
/// loft does. Before, they panicked.
#[test]
fn start_points_of_one_profile_are_refused() {
    assert_eq!(
        kernel::loft::start_points(&spec(vec![square(0.5, 0.0)])).unwrap_err(),
        ShapeError::LoftNeedsTwoProfiles
    );
}

/// A dart whose notch vertex sits exactly on its area centroid, started
/// there: its start direction falls back to the mean of its points (up),
/// so the circle it meets starts up too, not at whatever vertex comes
/// first. Before, the direction was a zero vector and the circle took its
/// first vertex.
#[test]
fn a_start_on_the_centroid_still_has_a_direction() {
    // Notch (0, 0.5) is the centroid of the dart (−1, 0), (0, 1), (1, 0),
    // (0, 0.5): (t + r) / 3 with t = 1, r = 0.5.
    let dart = profile(vec![
        Point3::new(0.0, 0.5, 0.0),
        Point3::new(-1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
    ]);
    let markers = kernel::loft::start_points(&spec(vec![dart, circle(0.5, 1.0, 48, 0.0)])).unwrap();
    let m = markers[1][0];
    assert!(m.y > 0.49 && m.x.abs() < 1e-9, "{m:?}");
}

/// A `n`-sided polygon drawn with straight sides (every vertex a corner),
/// radius `r` at height `z`, first vertex at angle `phase`.
fn polygon(n: usize, r: f64, z: f64, phase: f64) -> LoftProfile {
    profile(
        (0..n)
            .map(|k| {
                let a = phase + TAU * k as f64 / n as f64;
                Point3::new(r * trig::cos(a), r * trig::sin(a), z)
            })
            .collect(),
    )
}

/// A polygon of many straight sides meets a circle as smooth as the
/// circle's own facets: its 5.6° corners and the fan diagonals beside them
/// stand against the circle's 7.5° facet turn (the span's floor, a limit
/// of 15°), not against 0. An 8-gon's 45° corners spread over the bands to
/// the circle as well. Before (no plain quad, so a reference of 0), the
/// 64-gon gave 96 hard wall edges folding 1.9° to 5.6° (D-128).
#[test]
fn a_many_sided_polygon_meets_a_circle_as_smooth_as_its_facets() {
    for sides in [64, 8] {
        let o = Object::from_loft(&spec(vec![
            polygon(sides, 0.5, 0.0, 0.0),
            circle(0.5, 1.0, 48, 0.0),
        ]))
        .unwrap();
        assert_solid(&o);
        assert_eq!(hard_longitudinal(&o).len(), 0, "{sides}-gon");
    }
}

/// A square at height `z` (corners (0.5, −0.5), (0.5, 0.5), (−0.5, 0.5),
/// (−0.5, −0.5), listed from the first) whose corner `k` is rounded to
/// radius `r` by an arc of `chords` edges on one circle.
fn square_one_round(k: usize, r: f64, chords: usize, z: f64) -> LoftProfile {
    let corner = [(0.5, -0.5), (0.5, 0.5), (-0.5, 0.5), (-0.5, -0.5)];
    let mut points = Vec::new();
    let mut curves = Vec::new();
    for (i, &(x, y)) in corner.iter().enumerate() {
        if i != k {
            points.push(Point3::new(x, y, z));
            curves.push(None);
            continue;
        }
        let (px, py) = corner[(i + 3) % 4];
        let (nx, ny) = corner[(i + 1) % 4];
        let d_in = ((x - px), (y - py));
        let d_out = ((nx - x), (ny - y));
        let c = Point3::new(
            x - d_in.0 * r + d_out.0 * r,
            y - d_in.1 * r + d_out.1 * r,
            z,
        );
        let a0 = trig::atan2(y - d_in.1 * r - c.y, x - d_in.0 * r - c.x);
        let g = Some(CurveGeom {
            center: c,
            radius: r,
        });
        for j in 0..=chords {
            let a = a0 + (j as f64 / chords as f64) * PI / 2.0;
            points.push(Point3::new(
                c.x + r * trig::cos(a),
                c.y + r * trig::sin(a),
                z,
            ));
            curves.push(if j < chords { g } else { None });
        }
    }
    LoftProfile {
        points,
        curves,
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A square with one corner rounded, lofted to a plain square (either way
/// up), keeps its three untouched corners: exactly one hard wall edge
/// meets each of them at each end, whichever corner is rounded, whatever
/// the round's radius (2 cm, 20 cm) or chord count (1, 2, 3, 4, 8, 24).
/// Before, with the start corner rounded, a near-sharp round's steep skirt
/// set the reference and softened all three, and a coarse round's chord
/// turn (through the floor) the fourth: the guard keeps a corner that
/// folds most of its own turn, and the floor is capped at 15° (D-129 A).
#[test]
fn a_square_with_one_rounded_corner_keeps_its_other_three() {
    for k in 0..4 {
        for r in [0.02, 0.2] {
            for chords in [1, 2, 3, 4, 8, 24] {
                for round_first in [true, false] {
                    let (a, b) = if round_first {
                        (square_one_round(k, r, chords, 0.0), square(0.5, 1.0))
                    } else {
                        (square(0.5, 0.0), square_one_round(k, r, chords, 1.0))
                    };
                    let o = Object::from_loft(&spec(vec![a, b])).unwrap();
                    assert_solid(&o);
                    let hard = hard_longitudinal(&o);
                    let corners = [(0.5, -0.5), (0.5, 0.5), (-0.5, 0.5), (-0.5, -0.5)];
                    for (i, &(x, y)) in corners.iter().enumerate() {
                        if i == k {
                            continue;
                        }
                        for z in [0.0, 1.0] {
                            let c = Point3::new(x, y, z);
                            let n = hard
                                .iter()
                                .filter(|(p, q)| {
                                    (*p - c).length() < 1e-12 || (*q - c).length() < 1e-12
                                })
                                .count();
                            assert_eq!(
                                n, 1,
                                "corner {k} rounded r {r}, {chords} chords, round first {round_first}: {c:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A 64-gon with one vertex pushed out keeps it as a crease against a
/// circle (D-129 A, expected by the design owner). Open: measured against
/// a 48-gon circle the counts are 0, 4, 2, 2, 58 hard wall edges for a
/// push of 1.0, 1.05, 1.1, 1.2, 1.5 (64-gon circle: 0, 2, 2, 13, 76): at
/// 1.1 the spike turns 93.8°, its column folds 3.7° and its fan
/// diagonals 41.4° and 48.8°, each under two thirds of the turn (62.5°), so
/// only the skirt's two 38.5° corners crease. The fan splits the turn the
/// guard compares against.
#[test]
#[ignore = "open (D-129 A): the guard compares one fan diagonal with the corner's whole turn, which the fan splits; measured counts in the doc comment"]
fn a_spike_on_a_many_sided_polygon_creases_against_a_circle() {
    let mut last = 0;
    for s in [1.0, 1.05, 1.1, 1.2, 1.5] {
        let mut poly = polygon(64, 0.5, 0.0, 0.0);
        let p = poly.points[5];
        poly.points[5] = Point3::new(p.x * s, p.y * s, 0.0);
        let o = Object::from_loft(&spec(vec![poly, circle(0.5, 1.0, 48, 0.0)])).unwrap();
        assert_solid(&o);
        let n = hard_longitudinal(&o).len();
        if s == 1.0 {
            assert_eq!(n, 0);
        } else {
            assert!(n >= 3, "push {s}: {n}");
            assert!(n >= last, "push {s}: {n} after {last}");
        }
        last = n;
    }
}

/// A 5-tip star (0.5 / 0.25) keeps its tips against a circle (D-129 A,
/// expected by the design owner). Open: measured 0 hard wall edges: each
/// tip's column folds 7.5° and its two fan diagonals 60.0°, under two
/// thirds of the tip's turn.
#[test]
#[ignore = "open (D-129 A): a tip's turn is split across its two fan diagonals (60° each), under the guard's two thirds of the turn; measured 0"]
fn a_coarse_star_keeps_its_tips_against_a_circle() {
    let star = profile(
        (0..10)
            .map(|k| {
                let a = TAU * k as f64 / 10.0;
                let r = if k % 2 == 0 { 0.5 } else { 0.25 };
                Point3::new(r * trig::cos(a), r * trig::sin(a), 0.0)
            })
            .collect(),
    );
    let o = Object::from_loft(&spec(vec![star, circle(0.5, 1.0, 48, 0.0)])).unwrap();
    assert_solid(&o);
    let n = hard_longitudinal(&o).len();
    assert!(n >= 5, "{n}");
}

/// A star's points fold far beyond a circle's facets: every star column
/// still creases where it meets one (the facet floor raises the reference
/// only to the circle's 7.5°, D-128).
#[test]
fn a_star_keeps_its_points_against_a_circle() {
    let star = profile(
        (0..64)
            .map(|k| {
                let a = TAU * k as f64 / 64.0;
                let r = if k % 2 == 0 { 0.5 } else { 0.35 };
                Point3::new(r * trig::cos(a), r * trig::sin(a), 0.0)
            })
            .collect(),
    );
    let o = Object::from_loft(&spec(vec![star, circle(0.5, 1.0, 48, 0.0)])).unwrap();
    assert_solid(&o);
    let hard = hard_longitudinal(&o).len();
    assert!(hard >= 64, "{hard}");
}

/// A 64-gon to the same 64-gon moved and turned a few degrees: every corner
/// is a corner of both, so all 64 columns crease, as on a prism (corner
/// mode; the floor plays no part).
#[test]
fn a_many_sided_prism_keeps_every_corner() {
    let mut top = polygon(64, 0.5, 1.0, 3.0_f64.to_radians());
    for p in &mut top.points {
        p.x += 0.05;
    }
    let o = Object::from_loft(&spec(vec![polygon(64, 0.5, 0.0, 0.0), top])).unwrap();
    assert_solid(&o);
    let columns = hard_longitudinal(&o)
        .iter()
        .filter(|(a, b)| {
            (a.z.abs() < 1e-12 && (b.z - 1.0).abs() < 1e-12)
                || (b.z.abs() < 1e-12 && (a.z - 1.0).abs() < 1e-12)
        })
        .count();
    assert!(columns >= 64, "{columns}");
    // The 64 corner columns: every bottom corner has a hard edge up.
    for k in 0..64 {
        let a = TAU * k as f64 / 64.0;
        let c = Point3::new(0.5 * trig::cos(a), 0.5 * trig::sin(a), 0.0);
        assert!(
            hard_longitudinal(&o)
                .iter()
                .any(|(p, q)| (*p - c).length() < 1e-12 || (*q - c).length() < 1e-12),
            "corner {k}"
        );
    }
}

#[test]
fn apex_on_profile_plane_refused() {
    let mut s = spec(vec![square(0.5, 0.0)]);
    s.apex = Some(Point3::new(0.2, 0.1, 0.0));
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::ShapeDegenerate
    );
}

#[test]
fn coaxial_circles_ruled_cone_and_cylinder() {
    let cone = Object::from_loft(&spec(vec![
        circle(0.2, 0.0, 24, 0.0),
        circle(0.1, 0.5, 24, 0.0),
    ]))
    .unwrap();
    assert_solid(&cone);
    let mut apexes = cone.faces().values().filter_map(|f| match f.surface {
        Some(SurfaceRef::Cone { apex, .. }) => Some(apex),
        _ => None,
    });
    let apex = apexes.next().expect("cone claims");
    assert!(
        (apex - Point3::new(0.0, 0.0, 1.0)).length() < 1e-9,
        "{apex:?}"
    );
    let cyl = Object::from_loft(&spec(vec![
        circle(0.2, 0.0, 24, 0.0),
        circle(0.2, 0.5, 24, 0.0),
    ]))
    .unwrap();
    assert_eq!(
        cyl.faces()
            .values()
            .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
            .count(),
        24
    );
}

/// A U-shaped hull section of half-width `w` and depth `d` at height `z`.
fn u_section(w: f64, d: f64, z: f64) -> LoftProfile {
    let mut pts = vec![Point3::new(w, 0.0, z)];
    for k in 1..8 {
        let a = PI * k as f64 / 8.0;
        pts.push(Point3::new(w * trig::cos(a), -d * trig::sin(a), z));
    }
    pts.push(Point3::new(-w, 0.0, z));
    profile(pts)
}

#[test]
fn five_station_hull() {
    let s = spec(vec![
        u_section(0.2, 0.1, 0.0),
        u_section(0.4, 0.3, 1.0),
        u_section(0.5, 0.4, 2.0),
        u_section(0.4, 0.3, 3.0),
        u_section(0.2, 0.1, 4.0),
    ]);
    let o = Object::from_loft(&s).unwrap();
    assert_solid(&o);
    // Every ring carries the common sampling M (the union of the five
    // profiles' perimeter fractions).
    let m = ring_at(&o, 0.0).len();
    assert!(m >= 9);
    // 8 stations per span, the profiles included exactly.
    assert_eq!(o.vertices().len(), m * (4 * 8 + 1));
    for z in [1.0, 2.0, 3.0, 4.0] {
        assert_eq!(ring_at(&o, z).len(), m, "profile at z = {z}");
    }
    // The interior profile rings are soft (Catmull–Rom is smooth there).
    let mid_ring_soft = o
        .edges()
        .iter()
        .filter(|&(eid, e)| {
            let (a, b) = o.edge_endpoints(eid).unwrap();
            (a.z - 2.0).abs() < 1e-9 && (b.z - 2.0).abs() < 1e-9 && e.soft
        })
        .count();
    assert_eq!(mid_ring_soft, m);
}

/// The largest distance of `o`'s vertices from the z axis, and their
/// height range.
fn radial_extent(o: &Object) -> (f64, f64, f64) {
    let mut out = (0.0_f64, f64::INFINITY, f64::NEG_INFINITY);
    for v in o.vertices().values() {
        let p = v.position;
        out.0 = out.0.max(trig::hypot(p.x, p.y));
        out.1 = out.1.min(p.z);
        out.2 = out.2.max(p.z);
    }
    out
}

/// Stations between profiles stay within the profiles: a hull through
/// coaxial circles of radii 350, 580 and 600 (the playtest's discs) never
/// passes 600, and one whose middle circle is the largest peaks there.
/// Before, the Catmull–Rom path through the three radii bulged to 602.2
/// between the last two.
#[test]
fn the_hull_stays_within_its_profiles() {
    for radii in [[0.35, 0.58, 0.6], [0.3, 0.6, 0.35], [0.6, 0.58, 0.35]] {
        for n in [24, 48] {
            let o = Object::from_loft(&spec(vec![
                circle(radii[0], 0.008, n, 0.0),
                circle(radii[1], 0.508, n, 0.0),
                circle(radii[2], 1.008, n, 0.0),
            ]))
            .unwrap();
            assert_solid(&o);
            let (r, z0, z1) = radial_extent(&o);
            let largest = radii.iter().copied().fold(0.0_f64, f64::max);
            assert!(r <= largest + 1e-12, "{radii:?}, {n} sides: {r}");
            assert!(
                (r - largest).abs() < 1e-12,
                "the largest circle's ring is reached"
            );
            assert!(z0 >= 0.008 - 1e-12 && z1 <= 1.008 + 1e-12, "{z0}..{z1}");
            // Every station is a circle; through monotone radii the radius
            // grows from station to station.
            let rings = station_radii(&o);
            if radii[0] <= radii[1] && radii[1] <= radii[2] {
                for w in rings.windows(2) {
                    assert!(w[1].1 >= w[0].1 - 1e-12, "{radii:?}: {w:?}");
                }
            }
        }
    }
}

/// Each station's height and radius (every vertex at one height at one
/// distance from the z axis), bottom to top.
fn station_radii(o: &Object) -> Vec<(f64, f64)> {
    let mut rings: Vec<(f64, f64)> = Vec::new();
    for v in o.vertices().values() {
        let p = v.position;
        let r = trig::hypot(p.x, p.y);
        match rings.iter().find(|(z, _)| (z - p.z).abs() < 1e-12) {
            Some(&(_, r0)) => assert!((r - r0).abs() < 1e-12, "a station is not a circle at {p:?}"),
            None => rings.push((p.z, r)),
        }
    }
    rings.sort_by(|a, b| a.0.total_cmp(&b.0));
    rings
}

/// A circle of radius `r` at height `z` (center on the z axis) whose
/// vertices sit at `angles` (radians, increasing), every edge attributed to
/// the circle.
fn circle_at(r: f64, z: f64, angles: &[f64]) -> LoftProfile {
    LoftProfile {
        points: angles
            .iter()
            .map(|a| Point3::new(r * trig::cos(*a), r * trig::sin(*a), z))
            .collect(),
        curves: vec![
            Some(CurveGeom {
                center: Point3::new(0.0, 0.0, z),
                radius: r,
            });
            angles.len()
        ],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// An arc edge is sampled along the way it runs about its center: half a
/// turn (endpoints opposite each other) and more than half a turn alike.
/// Before, an edge between opposite points fell back to its chord, so the
/// stations collapsed toward the axis (ring radii down to 0.04 between
/// circles of 0.35 and 0.6), and one over half a turn was sampled the short
/// way.
#[test]
fn circle_profiles_sample_their_arcs_the_way_they_run() {
    let deg = |d: f64| d.to_radians();
    for angles in [
        vec![deg(0.0), deg(30.0), deg(180.0)],
        vec![deg(0.0), deg(30.0), deg(150.0)],
    ] {
        let mut s = spec(vec![
            circle_at(0.35, 0.0, &angles),
            circle_at(0.6, 1.0, &[0.0, deg(90.0), deg(180.0), deg(270.0)]),
        ]);
        s.stations = Some(4);
        let o = Object::from_loft(&s).unwrap();
        assert_solid(&o);
        let rings = station_radii(&o);
        assert_eq!(rings.len(), 5);
        for (z, r) in rings {
            let want = 0.35 + (0.6 - 0.35) * z;
            assert!(
                (r - want).abs() < 1e-12,
                "{angles:?}: ring at {z} has radius {r}"
            );
        }
    }
}

/// Small squares wandering sideways: limiting one span of the path
/// shrinks the tangent it shares with the span before, which leaves that
/// span outside the monotone region; the limits sweep until every span is
/// in, so every station stays in the box of the two squares either side.
/// Before, one pass left the span outside and its stations up to 0.96 mm
/// past the box.
#[test]
fn stations_stay_in_the_box_when_a_shared_tangent_shrinks() {
    let centers = [
        (-0.3192, 0.0, -0.18798),
        (0.3488, 0.0, 0.34484),
        (0.43005, 0.0, 0.968),
        (0.4316, 0.0, 1.46756),
    ];
    let h = 0.05;
    let square = |(x, y, z): (f64, f64, f64)| {
        profile(vec![
            Point3::new(x + h, y - h, z),
            Point3::new(x + h, y + h, z),
            Point3::new(x - h, y + h, z),
            Point3::new(x - h, y - h, z),
        ])
    };
    let o = Object::from_loft(&spec(centers.iter().map(|&c| square(c)).collect())).unwrap();
    assert_solid(&o);
    for v in o.vertices().values() {
        let p = v.position;
        let span = (0..centers.len() - 1)
            .find(|&i| p.z <= centers[i + 1].2 + 1e-12)
            .expect("a span");
        let (a, b) = (centers[span], centers[span + 1]);
        assert!(
            p.x >= a.0.min(b.0) - h - 1e-12 && p.x <= a.0.max(b.0) + h + 1e-12,
            "{p:?} outside span {span}'s box"
        );
    }
}

/// A unit circle profile at height 0 with vertices at `angles` (degrees),
/// edge `i` an arc of the circle where `arc[i]`, else a straight line.
fn arcs_and_lines(angles: &[f64], arc: &[bool]) -> LoftProfile {
    let g = CurveGeom {
        center: Point3::ORIGIN,
        radius: 1.0,
    };
    LoftProfile {
        points: angles
            .iter()
            .map(|a| Point3::new(trig::cos(a.to_radians()), trig::sin(a.to_radians()), 0.0))
            .collect(),
        curves: arc.iter().map(|&a| a.then_some(g)).collect(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A profile at height `z` from `(points, curves)` (curves per edge).
fn with_arcs(points: &[(f64, f64)], curves: &[Option<(f64, f64, f64)>], z: f64) -> LoftProfile {
    LoftProfile {
        points: points.iter().map(|&(x, y)| Point3::new(x, y, z)).collect(),
        curves: curves
            .iter()
            .map(|c| {
                c.map(|(cx, cy, r)| CurveGeom {
                    center: Point3::new(cx, cy, z),
                    radius: r,
                })
            })
            .collect(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// Longitudinal edges (spanning z0 → z1), and how many of them are not
/// vertical.
fn longitudinal(o: &Object) -> (usize, usize) {
    let mut all = 0;
    let mut slanted = 0;
    for e in o.edges().keys() {
        let (a, b) = o.edge_endpoints(e).unwrap();
        if (a.z - b.z).abs() > 1e-9 {
            all += 1;
            if (a.x - b.x).abs() > 1e-9 || (a.y - b.y).abs() > 1e-9 {
                slanted += 1;
            }
        }
    }
    (all, slanted)
}

/// A lone arc edge turns the way a straight neighbour tangent to its
/// circle continues it, else the short way (an exact half turn with no
/// such neighbour, counterclockwise with the loop); a run of two or more
/// arc edges on one circle turns as before. So a concave fillet drawn as
/// one edge lofts as the same fillet split in two, a stadium or a D of
/// lone semicircles builds, a shallow concave arc dips inward, and a lone
/// 250° arc sweeps 110° the other way while 250° + 50° still turns 300°.
/// Before, a lone arc always turned counterclockwise (the long way for a
/// concave fillet: the lone and split L lofted twisted, 26 faces) and a
/// lone half turn refused (D-129 B).
#[test]
fn a_lone_arc_takes_the_short_way_unless_a_tangent_neighbor_decides() {
    // The L [0,2]×[0,1] ∪ [0,1]×[0,2], its inner corner (1,1) filleted
    // r 0.5 (concave, center (1.5,1.5)), as one arc edge or two.
    let h = 0.5 * std::f64::consts::FRAC_1_SQRT_2;
    let fillet = Some((1.5, 1.5, 0.5));
    let l_shape = |split: bool, z: f64| {
        let mut pts = vec![(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.5, 1.0)];
        let mut cs = vec![None, None, None, fillet];
        if split {
            pts.push((1.5 - h, 1.5 - h));
            cs.push(fillet);
        }
        pts.extend([(1.0, 1.5), (1.0, 2.0), (0.0, 2.0)]);
        cs.extend([None, None, None]);
        with_arcs(&pts, &cs, z)
    };
    for (bottom, top, faces, long) in [
        (false, true, 10, 8),
        (true, true, 10, 8),
        (false, false, 9, 7),
    ] {
        let o = Object::from_loft(&spec(vec![l_shape(bottom, 0.0), l_shape(top, 1.0)]))
            .unwrap_or_else(|e| panic!("{bottom} {top}: {e:?}"));
        assert_solid(&o);
        assert_eq!(o.faces().len(), faces, "{bottom} {top}");
        assert_eq!(longitudinal(&o), (long, 0), "{bottom} {top}");
    }
    // A stadium of two lone semicircles, the straight sides tangent.
    let stadium = |z: f64| {
        with_arcs(
            &[(1.0, -0.5), (1.0, 0.5), (-1.0, 0.5), (-1.0, -0.5)],
            &[Some((1.0, 0.0, 0.5)), None, Some((-1.0, 0.0, 0.5)), None],
            z,
        )
    };
    let o = Object::from_loft(&spec(vec![stadium(0.0), stadium(1.0)])).unwrap();
    assert_solid(&o);
    let stadium_faces = o.faces().len();
    // A D: a lone semicircle on its diameter, closed by two lines through
    // (−0.3, 0) (a diameter alone has no plane), neither tangent: no vote,
    // so the half turn runs with the loop and its middle lies on the far
    // side of the diameter.
    let d = |split: bool, z: f64| {
        let g = Some((0.0, 0.0, 0.5));
        if split {
            with_arcs(
                &[(0.0, -0.5), (0.5, 0.0), (0.0, 0.5), (-0.3, 0.0)],
                &[g, g, None, None],
                z,
            )
        } else {
            with_arcs(&[(0.0, -0.5), (0.0, 0.5), (-0.3, 0.0)], &[g, None, None], z)
        }
    };
    let o = Object::from_loft(&spec(vec![d(false, 0.0), d(true, 1.0)])).unwrap();
    assert_solid(&o);
    assert!(
        ring_at(&o, 0.0)
            .iter()
            .any(|p| (p.x - 0.5).abs() < 1e-9 && p.y.abs() < 1e-9),
        "the D's middle at (0.5, 0)"
    );
    // A shallow lone arc replacing the top side of the square [−1,1]², r 10:
    // concave (center above) it dips to 0.95, convex to 1.05.
    let q = 99f64.sqrt();
    for (cy, mid) in [(1.0 + q, 10.0 - q), (1.0 - q, -(10.0 - q))] {
        let side = |split: bool, z: f64| {
            let g = Some((0.0, cy, 10.0));
            if split {
                let y = 1.0 - mid;
                with_arcs(
                    &[(1.0, -1.0), (1.0, 1.0), (0.0, y), (-1.0, 1.0), (-1.0, -1.0)],
                    &[None, g, g, None, None],
                    z,
                )
            } else {
                with_arcs(
                    &[(1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)],
                    &[None, g, None, None],
                    z,
                )
            }
        };
        let o = Object::from_loft(&spec(vec![side(false, 0.0), side(true, 1.0)])).unwrap();
        assert_solid(&o);
        let y = 1.0 - mid;
        assert!(
            ring_at(&o, 0.0)
                .iter()
                .any(|p| p.x.abs() < 1e-9 && (p.y - y).abs() < 1e-9),
            "center y {cy}: the arc's middle at {y}"
        );
    }
    // A lone arc from 0° to 250° (a wedge closed through the center) turns
    // the short way: 110° clockwise; 250° + 50° on one circle still turns
    // 300° with the loop.
    let wedge = arcs_and_lines(&[0.0, 250.0], &[true, false]);
    let mut wedge = wedge;
    wedge.points.push(Point3::ORIGIN);
    wedge.curves.push(None);
    for (bottom, lo, hi) in [
        (wedge, 250.0, 360.0),
        (
            arcs_and_lines(&[0.0, 250.0, 300.0], &[true, true, false]),
            0.0,
            300.0,
        ),
    ] {
        let mut s = spec(vec![
            bottom,
            circle_at(1.0, 1.0, &[0.0, 120f64.to_radians(), 240f64.to_radians()]),
        ]);
        s.stations = Some(4);
        let o = Object::from_loft(&s).unwrap_or_else(|e| panic!("{lo}–{hi}: {e:?}"));
        assert_solid(&o);
        for p in ring_at(&o, 0.0) {
            if (trig::hypot(p.x, p.y) - 1.0).abs() < 1e-9 {
                let a = trig::atan2(p.y, p.x).to_degrees().rem_euclid(360.0);
                let a = if a < 1e-9 && hi > 300.0 { 360.0 } else { a };
                assert!(a >= lo - 1e-9 && a <= hi + 1e-9, "{lo}–{hi}: {a}");
            }
        }
    }
    // Four sides (each lone semicircle one band between its two copies)
    // and two caps.
    assert_eq!(stadium_faces, 6, "the stadium");
}

/// A caller that knows which way an arc runs says so with a point on it
/// (D-130): a lone 250° arc edge given its middle (125°) turns the long way
/// it was drawn (a wedge of 250° closed through the center), where
/// inference takes the short way; a point off the circle, or on an
/// endpoint, refuses; and on a run (which fits one way only) a point
/// confirms its direction or, contradicting it, refuses.
#[test]
fn a_given_arc_point_decides_its_direction() {
    let on = |deg: f64| {
        Point3::new(
            trig::cos(deg.to_radians()),
            trig::sin(deg.to_radians()),
            0.0,
        )
    };
    let wedge = |through: Option<Point3>| {
        let mut p = arcs_and_lines(&[0.0, 250.0], &[true, false]);
        p.points.push(Point3::ORIGIN);
        p.curves.push(None);
        p.through = vec![through, None, None];
        p
    };
    let loft = |bottom: LoftProfile| {
        let mut s = spec(vec![
            bottom,
            circle_at(1.0, 1.0, &[0.0, 120f64.to_radians(), 240f64.to_radians()]),
        ]);
        s.stations = Some(4);
        Object::from_loft(&s)
    };
    let ring_angles = |bottom: LoftProfile| -> Vec<f64> {
        let o = loft(bottom).unwrap();
        assert_solid(&o);
        ring_at(&o, 0.0)
            .into_iter()
            .filter(|p| (trig::hypot(p.x, p.y) - 1.0).abs() < 1e-9)
            .map(|p| trig::atan2(p.y, p.x).to_degrees().rem_euclid(360.0))
            .collect()
    };
    // Through 125°: every arc sample within 0°..250°, counterclockwise.
    let long = ring_angles(wedge(Some(on(125.0))));
    assert!(long.iter().all(|&a| a <= 250.0 + 1e-9), "{long:?}");
    assert!(long.iter().any(|&a| a > 1.0 && a < 249.0), "{long:?}");
    // Through 300° (or inferred): the short way, 110° clockwise.
    for through in [Some(on(300.0)), None] {
        let short = ring_angles(wedge(through));
        assert!(
            short.iter().all(|&a| !(1e-9..250.0 - 1e-9).contains(&a)),
            "{through:?}: {short:?}"
        );
    }
    // Off the circle, or on an endpoint: refused, on either side of the
    // endpoint within tolerance (a point a hair short of the start reads
    // as nearly a full turn from it, and is still the start).
    let mut bads = vec![Point3::new(0.0, 0.5, 0.0), on(0.0)];
    for end in [0.0, 250.0] {
        for noise in [1e-12_f64, 5e-10] {
            for sign in [-1.0, 1.0] {
                bads.push(on(end + sign * noise.to_degrees()));
            }
        }
    }
    for bad in bads {
        assert_eq!(
            loft(wedge(Some(bad))).map(|_| ()),
            Err(ShapeError::ShapeDegenerate),
            "{bad:?}"
        );
    }
    // A run 0° → 250° → 300° fits one way only (300° counterclockwise;
    // clockwise it would sweep −420°): a point at 125° confirms it, one at
    // 330° contradicts it and refuses.
    let run = |through: Point3| {
        let mut p = arcs_and_lines(&[0.0, 250.0, 300.0], &[true, true, false]);
        p.through = vec![Some(through), None, None];
        p
    };
    let ccw = ring_angles(run(on(125.0)));
    assert!(ccw.iter().all(|&a| a <= 300.0 + 1e-9), "{ccw:?}");
    assert_eq!(
        loft(run(on(330.0))).unwrap_err(),
        ShapeError::ShapeDegenerate
    );
    // A small arc in the right side of the square [−1,1]² (center
    // (0.95, 0), ends (1, ±0.1)), whose direction leaves the loop's
    // winding alone, lofted to a fine circle (so the arc gets columns of
    // its own): through its outer middle it bulges out; a hair either side
    // of either end refuses (before, one just short of the start read as a
    // point on the long way round, and the arc dented the side instead).
    let r = trig::hypot(0.05_f64, 0.1);
    let notch = |through: Option<Point3>| {
        let g = Some((0.95, 0.0, r));
        let mut p = with_arcs(
            &[
                (-1.0, -1.0),
                (1.0, -1.0),
                (1.0, -0.1),
                (1.0, 0.1),
                (1.0, 1.0),
                (-1.0, 1.0),
            ],
            &[None, None, g, None, None, None],
            0.0,
        );
        p.through = vec![None, None, through, None, None, None];
        Object::from_loft(&spec(vec![p, circle(1.2, 1.0, 256, 0.0)]))
    };
    let at = |a: f64| Point3::new(0.95 + r * trig::cos(a), r * trig::sin(a), 0.0);
    let o = notch(Some(at(0.0))).unwrap();
    assert_solid(&o);
    let side: Vec<f64> = ring_at(&o, 0.0)
        .iter()
        .filter(|p| p.y.abs() < 0.1 - 1e-9 && p.x > 0.5)
        .map(|p| p.x)
        .collect();
    assert!(
        !side.is_empty() && side.iter().all(|&x| x > 1.0 + 1e-6),
        "bulges out: {side:?}"
    );
    for end in [-trig::atan(2f64), trig::atan(2f64)] {
        for noise in [-1e-12, 1e-12, -5e-10, 5e-10] {
            assert_eq!(
                notch(Some(at(end + noise))).map(|_| ()),
                Err(ShapeError::ShapeDegenerate),
                "{end} {noise:e}"
            );
        }
    }
}

/// Rectangles whose proportions change sharply: the stations stay inside
/// the box of the two profiles either side of them. Before, limiting in a
/// frame turned about the loft axis let a column leave that box (by 0.025
/// to 0.05 here).
#[test]
fn stations_stay_in_their_profiles_box() {
    let rect = |w: f64, h: f64, z: f64| {
        profile(vec![
            Point3::new(w, -h, z),
            Point3::new(w, h, z),
            Point3::new(-w, h, z),
            Point3::new(-w, -h, z),
        ])
    };
    let mut cases: Vec<Vec<(f64, f64)>> =
        vec![vec![(0.7287, 0.2623), (0.7324, 0.8632), (0.2099, 0.8610)]];
    // A fixed pseudo-random spread of proportions.
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        0.1 + 0.9 * (x % 10_000) as f64 / 10_000.0
    };
    for rows in [3, 4, 5] {
        for _ in 0..60 {
            cases.push((0..rows).map(|_| (next(), next())).collect());
        }
    }
    for dims in cases {
        let profiles: Vec<LoftProfile> = dims
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| rect(w, h, 0.5 * i as f64))
            .collect();
        let Ok(o) = Object::from_loft(&spec(profiles)) else {
            continue;
        };
        for v in o.vertices().values() {
            let p = v.position;
            let span = ((p.z / 0.5).floor() as usize).min(dims.len() - 2);
            let (w, h) = (
                dims[span].0.max(dims[span + 1].0),
                dims[span].1.max(dims[span + 1].1),
            );
            assert!(
                p.x.abs() <= w + 1e-12 && p.y.abs() <= h + 1e-12,
                "{dims:?}: {p:?} outside {w} × {h}"
            );
        }
    }
}

/// Coaxial circles of different side counts, drawn from different
/// starting angles (as the app draws them: 60, 76 and 80 sides for the
/// playtest's discs), loft on meridians: every column stays at one angle
/// about the axis, so every station is a circle and every band quad is
/// planar (no soft diagonals). Before, each profile started at its own
/// first vertex and the columns twisted by up to half a side.
#[test]
fn coaxial_circles_of_any_count_and_phase_loft_on_meridians() {
    let deg = |d: f64| d.to_radians();
    let uniform = |n: usize| {
        (0..n)
            .map(|k| TAU * k as f64 / n as f64)
            .collect::<Vec<_>>()
    };
    // Uneven spacing too: arcs of 100°, 100° and 160°, and vertices at
    // angles 0, 0.5, 2, 2.2, 4 and 5.9 against a uniform 16-gon (a circle
    // edited by moving its vertices): the correspondence is by angle, as
    // the sampling is.
    for profiles in [
        vec![
            circle(0.35, 0.0, 60, 0.3),
            circle(0.58, 0.5, 76, 1.1),
            circle(0.6, 1.0, 80, 2.0),
        ],
        vec![
            circle_at(0.35, 0.0, &uniform(24)),
            circle_at(0.58, 0.5, &uniform(16)),
            circle_at(0.6, 1.0, &[deg(0.0), deg(100.0), deg(200.0)]),
        ],
        vec![
            circle_at(0.35, 0.0, &[0.0, 0.5, 2.0, 2.2, 4.0, 5.9]),
            circle_at(0.6, 1.0, &uniform(16)),
        ],
    ] {
        let o = Object::from_loft(&spec(profiles)).unwrap();
        assert_solid(&o);
        station_radii(&o);
        let quads = o
            .faces()
            .values()
            .filter(|f| o.loop_positions(f.outer_loop).count() == 4)
            .count();
        let triangles = o
            .faces()
            .values()
            .filter(|f| o.loop_positions(f.outer_loop).count() == 3)
            .count();
        assert!(quads > 0);
        assert_eq!(triangles, 0, "no band was split by a diagonal");
    }
}

/// A square's corner is a crease where the hull leaves the square; past
/// the first circle, between circles (smooth at every column), the hull is
/// smooth all the way round although the circles carry vertices on the
/// square's corner columns. Before, those four columns stayed hard up the
/// whole faceted hull.
#[test]
fn a_corner_column_is_smooth_between_smooth_profiles() {
    let r = 0.5 * std::f64::consts::SQRT_2;
    let o = Object::from_loft(&spec(vec![
        square(0.5, 0.0),
        circle(r, 1.0, 48, -PI / 4.0),
        circle(r * 1.2, 2.0, 48, -PI / 4.0),
        circle(r, 3.0, 48, -PI / 4.0),
    ]))
    .unwrap();
    assert_solid(&o);
    let hard_between = |z0: f64, z1: f64| {
        o.edges()
            .keys()
            .filter(|&e| {
                let (p, q) = o.edge_endpoints(e).unwrap();
                (p.z - q.z).abs() > 1e-9
                    && p.z.min(q.z) >= z0 - 1e-9
                    && p.z.max(q.z) <= z1 + 1e-9
                    && o.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
            })
            .count()
    };
    assert!(hard_between(0.0, 1.0) >= 4, "the square's corners crease");
    assert_eq!(hard_between(1.0, 3.0), 0, "smooth between the circles");
}

/// Between profiles on parallel planes every station is a plane slice:
/// all its points at one height, so a band between two stations whose
/// sides are parallel is one planar quad. Before, each column's path took its
/// own knots from its own chord lengths, so a station's points sat at
/// slightly different heights and its bands split into triangles a hair out
/// of plane.
#[test]
fn stations_between_parallel_profiles_are_planar() {
    let rect = |w: f64, h: f64, z: f64| {
        profile(vec![
            Point3::new(w, -h, z),
            Point3::new(w, h, z),
            Point3::new(-w, h, z),
            Point3::new(-w, -h, z),
        ])
    };
    for profiles in [
        vec![
            rect(0.5, 0.3, 0.0),
            rect(0.6, 0.4, 0.5),
            rect(0.55, 0.35, 1.0),
        ],
        vec![
            rect(0.8, 0.28, 0.0),
            rect(1.0, 0.44, 0.25),
            rect(1.1, 0.52, 0.5),
            rect(1.1, 0.56, 0.75),
            rect(1.0, 0.56, 1.0),
        ],
        vec![
            u_section(0.2, 0.1, 0.0),
            u_section(0.4, 0.3, 1.0),
            u_section(0.5, 0.4, 2.0),
        ],
    ] {
        let o = Object::from_loft(&spec(profiles)).unwrap();
        assert_solid(&o);
        let mut heights: Vec<f64> = o.vertices().values().map(|v| v.position.z).collect();
        heights.sort_by(f64::total_cmp);
        heights.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let m = ring_at(&o, 0.0).len();
        assert_eq!(
            heights.len() * m,
            o.vertices().len(),
            "every station at one height"
        );
    }
}

#[test]
fn apex_nose() {
    let mut s = spec(vec![square(0.5, 0.0)]);
    s.apex = Some(Point3::new(0.0, 0.0, 1.0));
    let o = Object::from_loft(&s).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 4 + 1, "a triangle fan and one cap");
    assert!((volume(&o) - 1.0 / 3.0).abs() < 1e-12);
}

#[test]
fn one_rail() {
    // Two squares, and a rail through their (+h, −h) corners that bows
    // out in x halfway.
    let mut s = spec(vec![square(0.5, 0.0), square(0.5, 1.0)]);
    s.rails = vec![vec![
        Point3::new(0.5, -0.5, 0.0),
        Point3::new(0.8, -0.5, 0.5),
        Point3::new(0.5, -0.5, 1.0),
    ]];
    s.stations = Some(4);
    let o = Object::from_loft(&s).unwrap();
    assert_solid(&o);
    // The ring at z = 0.5 has its rail column on the rail's middle vertex.
    assert!(
        o.vertices()
            .values()
            .any(|v| (v.position - Point3::new(0.8, -0.5, 0.5)).length() < 1e-9)
    );
}

#[test]
fn two_rails() {
    let mut s = spec(vec![square(0.5, 0.0), square(0.5, 1.0)]);
    s.rails = vec![
        vec![
            Point3::new(0.5, -0.5, 0.0),
            Point3::new(0.7, -0.7, 0.5),
            Point3::new(0.5, -0.5, 1.0),
        ],
        vec![
            Point3::new(-0.5, 0.5, 0.0),
            Point3::new(-0.7, 0.7, 0.5),
            Point3::new(-0.5, 0.5, 1.0),
        ],
    ];
    s.stations = Some(4);
    let o = Object::from_loft(&s).unwrap();
    assert_solid(&o);
    for x in [Point3::new(0.7, -0.7, 0.5), Point3::new(-0.7, 0.7, 0.5)] {
        assert!(
            o.vertices()
                .values()
                .any(|v| (v.position - x).length() < 1e-9),
            "{x:?} on the rail"
        );
    }
}

/// A rail from `a` to `b` through their midpoint moved by `bow` (its part
/// across `b − a`, so the middle point is halfway along the rail).
fn bowed_rail(a: Point3, b: Point3, bow: Vec3) -> Vec<Point3> {
    let d = b - a;
    let bow = bow - d * (bow.dot(d) / d.dot(d));
    let mid = a + d * 0.5 + bow;
    vec![a, mid, b]
}

#[test]
fn a_rail_between_any_two_corners_sets_the_correspondence() {
    // Playtest 3.3.2 (D-232): a square and the same square turned 45°, a
    // rail from the bottom's (+h, −h) corner to EACH corner of the top in
    // turn. The automatic start pairs only one of them; a rail to any
    // other corner used to refuse `RailMissesProfile` although it touches
    // both shapes at a corner. The rail now decides which corners pair.
    let r = 0.5;
    let top_corners = [
        Point3::new(r, 0.0, 1.0),
        Point3::new(0.0, r, 1.0),
        Point3::new(-r, 0.0, 1.0),
        Point3::new(0.0, -r, 1.0),
    ];
    for &to in &top_corners {
        let top = profile(top_corners.to_vec());
        let mut s = spec(vec![square(0.5, 0.0), top]);
        let from = Point3::new(0.5, -0.5, 0.0);
        let rail = bowed_rail(from, to, Vec3::new(0.2, -0.2, 0.0));
        let mid = rail[1];
        s.rails = vec![rail];
        s.stations = Some(4);
        let o = Object::from_loft(&s).unwrap_or_else(|e| panic!("rail to {to:?}: {e:?}"));
        assert_solid(&o);
        assert!(
            o.vertices()
                .values()
                .any(|v| (v.position - mid).length() < 1e-9),
            "rail to {to:?}: the middle station's rail column on the rail"
        );
    }
}

#[test]
fn a_rail_between_unlike_shapes_sets_the_start() {
    // A square and a triangle (perimeter matching, every vertex a start):
    // a rail from a square corner to any triangle vertex builds, the
    // middle station's rail column on the rail.
    let tri = [
        Point3::new(0.6, 0.0, 1.0),
        Point3::new(-0.3, 0.5, 1.0),
        Point3::new(-0.3, -0.5, 1.0),
    ];
    // Each vertex from the square corner nearest it in angle (a rail
    // twisting the loft past half a side would make it pass through
    // itself, a separate refusal).
    let from = [
        Point3::new(0.5, 0.5, 0.0),
        Point3::new(-0.5, 0.5, 0.0),
        Point3::new(-0.5, -0.5, 0.0),
    ];
    for (&to, &from) in tri.iter().zip(&from) {
        let mut s = spec(vec![square(0.5, 0.0), profile(tri.to_vec())]);
        let rail = bowed_rail(from, to, Vec3::new(from.x, from.y, 0.0) * 0.2);
        let mid = rail[1];
        s.rails = vec![rail];
        s.stations = Some(4);
        let o = Object::from_loft(&s).unwrap_or_else(|e| panic!("rail to {to:?}: {e:?}"));
        assert_solid(&o);
        assert!(
            o.vertices()
                .values()
                .any(|v| (v.position - mid).length() < 1e-9),
            "rail to {to:?}"
        );
    }
}

#[test]
fn rails_that_disagree_on_the_pairing_refuse() {
    // Review F2: the first rail forces a turn the automatic start does not
    // make (the bottom's (+h, −h) corner to the top's (−h, −h) corner, a
    // quarter turn), and on its own it builds. A second rail straight up
    // from the bottom's (−h, +h) corner asks for no turn: it contradicts
    // the first, which decides, so the pair refuses.
    let first = bowed_rail(
        Point3::new(0.5, -0.5, 0.0),
        Point3::new(-0.5, -0.5, 1.0),
        Vec3::new(0.0, -0.2, 0.0),
    );
    let mut s = spec(vec![square(0.5, 0.0), square(0.5, 1.0)]);
    s.rails = vec![first.clone()];
    s.stations = Some(4);
    let o = Object::from_loft(&s).unwrap_or_else(|e| panic!("the first rail alone: {e:?}"));
    assert_solid(&o);
    assert!(
        o.vertices()
            .values()
            .any(|v| (v.position - first[1]).length() < 1e-9),
        "the first rail's column runs through its middle"
    );
    s.rails.push(bowed_rail(
        Point3::new(-0.5, 0.5, 0.0),
        Point3::new(-0.5, 0.5, 1.0),
        Vec3::new(-0.1, 0.1, 0.0),
    ));
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::RailMissesProfile
    );
    // The second rail agreeing with the first (its corner a quarter turn
    // on too) builds.
    s.rails[1] = bowed_rail(
        Point3::new(-0.5, 0.5, 0.0),
        Point3::new(0.5, 0.5, 1.0),
        Vec3::new(0.0, 0.2, 0.0),
    );
    assert_solid(&Object::from_loft(&s).unwrap());
}

#[test]
fn a_rail_between_turned_circles_sets_their_pairing() {
    // Review F4: two 16-gon circles, the upper drawn from 0.7 of a half
    // step past the lower (so no vertex of it sits on a meridian of the
    // lower's), and a bowed rail from the lower's vertex 0 to the upper's
    // vertex k. Coaxial circles pair by angle, which puts the upper's
    // vertices between columns, and no start offset helps: every k used to
    // refuse `RailMissesProfile`. The rail decides the pairing: the upper
    // circle turns until vertex k sits on the lower's vertex 0 column.
    let n = 16;
    let phase = 0.7 * (PI / n as f64);
    let lower = circle(0.5, 0.0, n, 0.0);
    let upper = circle(0.5, 1.0, n, phase);
    for k in [0usize, 1, 2, 15] {
        let from = lower.points[0];
        let to = upper.points[k];
        let mut s = spec(vec![lower.clone(), upper.clone()]);
        let out = Vec3::new(from.x + to.x, from.y + to.y, 0.0) * 0.15;
        let rail = bowed_rail(from, to, out);
        let mid = rail[1];
        s.rails = vec![rail];
        s.stations = Some(4);
        let o = Object::from_loft(&s).unwrap_or_else(|e| panic!("rail to vertex {k}: {e:?}"));
        assert_solid(&o);
        assert!(
            o.vertices()
                .values()
                .any(|v| (v.position - mid).length() < 1e-9),
            "rail to vertex {k}: the middle station's rail column on the rail"
        );
        let joined_rows = [from, to].iter().all(|&p| {
            o.vertices()
                .values()
                .any(|v| (v.position - p).length() < 1e-12)
        });
        assert!(joined_rows, "rail to vertex {k}: both ends are vertices");
    }
}

#[test]
fn rail_missing_profile_refused() {
    let mut s = spec(vec![square(0.5, 0.0), square(0.5, 1.0)]);
    s.rails = vec![vec![
        Point3::new(0.5, -0.5, 0.0),
        Point3::new(3.0, 3.0, 1.0),
    ]];
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::RailMissesProfile
    );
    let mut s = spec(vec![square(0.5, 0.0), square(0.5, 1.0)]);
    s.rails = vec![vec![]; 3];
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::LoftTooManyRails
    );
}

#[test]
fn start_offset_twists() {
    let mut top = square(0.5, 1.0);
    top.start_offset = 1;
    let o = Object::from_loft(&spec(vec![square(0.5, 0.0), top])).unwrap();
    assert_solid(&o);
    assert_eq!(o.faces().len(), 4 * 2 + 2, "triangulated sides");
}

#[test]
fn one_profile_refused() {
    assert_eq!(
        Object::from_loft(&spec(vec![square(0.5, 0.0)])).unwrap_err(),
        ShapeError::LoftNeedsTwoProfiles
    );
}

#[test]
fn self_intersecting_refused() {
    // Two close squares matched half a turn apart: every ruled band passes
    // through the axis.
    let mut top = square(0.5, 0.05);
    top.start_offset = 2;
    let err = Object::from_loft(&spec(vec![square(0.5, 0.0), top])).unwrap_err();
    assert!(
        matches!(
            err,
            ShapeError::SelfIntersecting { .. } | ShapeError::ShapeDegenerate
        ),
        "{err:?}"
    );
}

#[test]
fn half_turn_start_offset_is_refused() {
    // A straight-line half turn passes every side through the axis.
    let mut top = square(0.5, 3.0);
    top.start_offset = 2;
    let err = Object::from_loft(&spec(vec![square(0.5, 0.0), top])).unwrap_err();
    assert!(
        matches!(err, ShapeError::SelfIntersecting { .. }),
        "{err:?}"
    );
}

#[test]
fn crossing_profiles_are_refused() {
    // The second profile is a bowtie: its sides cross.
    let z = 0.05;
    let bowtie = profile(vec![
        Point3::new(0.5, 0.5, z),
        Point3::new(-0.5, -0.5, z),
        Point3::new(0.5, -0.5, z),
        Point3::new(-0.5, 0.5, z),
    ]);
    let err = Object::from_loft(&spec(vec![square(0.5, 0.0), bowtie])).unwrap_err();
    assert!(matches!(err, ShapeError::ShapeDegenerate), "{err:?}");
}

#[test]
fn non_planar_profile_refused() {
    let mut bent = square(0.5, 0.0);
    bent.points[1].z = 0.1;
    let err = Object::from_loft(&spec(vec![bent, square(0.5, 1.0)])).unwrap_err();
    assert_eq!(err, ShapeError::ShapeDegenerate);
    let _ = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0));
}

// ─────────────────────────────────────────────────────────── document

fn ground_square_doc(
    doc: &mut kernel::Document,
    h: f64,
    z: f64,
    hole: bool,
) -> kernel::document::LoftSource {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let mut ring = |r: f64| {
            let c = [(-r, -r), (r, -r), (r, r), (-r, r)];
            for i in 0..4 {
                let (a, b) = (c[i], c[(i + 1) % 4]);
                sk.add_segment(Point3::new(a.0, a.1, z), Point3::new(b.0, b.1, z))
                    .unwrap();
            }
        };
        ring(h);
        if hole {
            ring(h / 2.0);
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    // The outer region: the one whose profile has the larger outer loop.
    let region = doc
        .extrudable_regions(s)
        .unwrap()
        .into_iter()
        .max_by(|&a, &b| {
            let pa = doc.sketch(s).unwrap().profile(a).unwrap();
            let pb = doc.sketch(s).unwrap().profile(b).unwrap();
            let ext = |p: &kernel::sketch::Profile| {
                p.outer().iter().map(|q| q.x).fold(f64::MIN, f64::max)
            };
            ext(&pa).total_cmp(&ext(&pb))
        })
        .unwrap();
    kernel::document::LoftSource::Region { sketch: s, region }
}

#[test]
fn holes_refused() {
    let mut doc = kernel::Document::new();
    let a = ground_square_doc(&mut doc, 0.5, 0.0, true);
    let b = ground_square_doc(&mut doc, 0.5, 1.0, false);
    let err = doc.loft(&[a, b], &[], &[], None).unwrap_err();
    assert_eq!(
        err,
        kernel::DocumentError::Shape(ShapeError::LoftProfileHasHole)
    );
}

#[test]
fn undo_hides() {
    let mut doc = kernel::Document::new();
    let a = ground_square_doc(&mut doc, 0.5, 0.0, false);
    let b = ground_square_doc(&mut doc, 0.3, 1.0, false);
    let before = doc.state_hash();
    let preview = doc.loft_preview(&[a, b], &[], &[], None).unwrap();
    assert_eq!(doc.state_hash(), before);
    let (id, _) = doc.loft(&[a, b], &[], &[], None).unwrap();
    assert_eq!(
        format!("{:?}", doc.object(id).unwrap()),
        format!("{preview:?}")
    );
    assert_solid(doc.object(id).unwrap());
    let after = doc.state_hash();
    doc.undo().unwrap();
    assert!(
        !doc.visible_object_ids().contains(&id),
        "undo hides the loft"
    );
    assert_eq!(doc.state_hash(), before, "profiles were not consumed");
    doc.redo().unwrap();
    assert!(doc.visible_object_ids().contains(&id));
    assert_eq!(doc.state_hash(), after);
    let starts = doc.loft_start_points(&[a, b], &[0, 1]).unwrap();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[1].len(), 4);
}

#[test]
fn huge_station_count_refuses() {
    // A typed count past the cap refuses instead of building billions of
    // points; the cap itself still lofts.
    let mut s = spec(vec![square(1.0, 0.0), square(1.0, 1.0)]);
    s.stations = Some(100_000_000);
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::AmountOutOfRange
    );
    s.stations = Some(64);
    assert_solid(&Object::from_loft(&s).unwrap());
}

#[test]
fn dense_rings_times_many_stations_refuse() {
    // Each count alone is under its cap, but 512 samples × 4000 stations
    // is two million quads: refuse rather than allocate them.
    let ring = |z: f64| -> Vec<Point3> {
        (0..512)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 512.0;
                Point3::new(trig::cos(a), trig::sin(a), z)
            })
            .collect()
    };
    let mut s = spec(vec![profile(ring(0.0)), profile(ring(1.0))]);
    s.stations = Some(4000);
    assert_eq!(
        Object::from_loft(&s).unwrap_err(),
        ShapeError::AmountOutOfRange
    );
    s.stations = Some(8);
    assert_solid(&Object::from_loft(&s).unwrap());
}

/// A drawn circle (the Circle tool's chain on its curve) of `n` sides.
fn circle_doc(
    doc: &mut kernel::Document,
    r: f64,
    z: f64,
    n: usize,
) -> kernel::document::LoftSource {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(0.0, 0.0, z),
            radius: r,
        })
        .unwrap();
        let at = |k: usize| {
            let a = TAU * k as f64 / n as f64;
            Point3::new(r * trig::cos(a), r * trig::sin(a), z)
        };
        for k in 0..n {
            sk.add_segment(at(k), at(k + 1)).unwrap();
        }
        sk.end_curve();
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.extrudable_regions(s).unwrap()[0];
    kernel::document::LoftSource::Region { sketch: s, region }
}

#[test]
fn drawn_circles_loft_with_soft_meridians() {
    // Three drawn circles of different sizes and side counts: no corner in
    // any profile, so no meridian draws as a hard line.
    let mut doc = kernel::Document::new();
    let a = circle_doc(&mut doc, 0.35, 0.0, 24);
    let b = circle_doc(&mut doc, 0.58, 0.5, 36);
    let c = circle_doc(&mut doc, 0.6, 1.0, 36);
    let (id, _) = doc.loft(&[a, b, c], &[], &[], None).unwrap();
    let o = doc.object(id).unwrap();
    assert_solid(o);
    let hard_side = o
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p.z - q.z).abs() > 1e-9 && o.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
        })
        .count();
    assert_eq!(hard_side, 0);
}

#[test]
fn moved_circle_slab_faces_loft_with_soft_meridians() {
    // The playtest's stations: circles pushed into thin slabs, moved into
    // place, and lofted through their top faces.
    let mut doc = kernel::Document::new();
    let mut tops = Vec::new();
    for (r, z, n) in [(0.35, 0.0, 24), (0.58, 0.5, 36), (0.6, 1.0, 36)] {
        let kernel::document::LoftSource::Region { sketch, region } =
            circle_doc(&mut doc, r, 0.0, n)
        else {
            unreachable!()
        };
        let (id, _) = doc.extrude_region(sketch, region, 0.01).unwrap();
        doc.transform_object(id, &kernel::Transform::translation(Vec3::new(1.5, 0.25, z)))
            .unwrap();
        let o = doc.object(id).unwrap();
        let top = o
            .faces()
            .iter()
            .find(|(_, f)| {
                f.plane.normal().z > 0.99
                    && f.plane
                        .signed_distance(Point3::new(0.0, 0.0, z + 0.01))
                        .abs()
                        < 1e-9
            })
            .map(|(f, _)| f)
            .unwrap();
        tops.push(kernel::document::LoftSource::Face {
            object: id,
            face: top,
        });
    }
    let (id, _) = doc.loft(&tops, &[], &[], None).unwrap();
    let o = doc.object(id).unwrap();
    assert_solid(o);
    let hard_side = o
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (p.z - q.z).abs() > 1e-9 && o.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
        })
        .count();
    assert_eq!(hard_side, 0);
}
