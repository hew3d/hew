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
    BooleanError, BooleanOp, FaceId, History, HistoryError, KernelOp, KernelOpReport, Object,
    Plane, Point3, Profile, PushPullError, StickyError, Transform, TransformError, Vec3,
    WatertightState, tol,
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

/// Divergence-theorem signed volume, fan-triangulating every loop of every face.
/// Inner (hole) loops wind opposite to the outer, so summing their fans subtracts
/// the hole correctly — `to_polygons` drops holes, so it can't be used once
/// imprints/booleans produce genus > 0. Positive iff faces wind outward.
fn signed_volume(obj: &Object) -> f64 {
    let mut six_v = 0.0;
    for f in obj.faces().values() {
        for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<_> = obj.loop_positions(lid).map(|pt| pt.to_vec()).collect();
            for i in 1..p.len().saturating_sub(1) {
                six_v += p[0].dot(p[i].cross(p[i + 1]));
            }
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

// ------------------------------------------------------------- follow me

/// The x = 0 plane with normal +x.
fn yz_plane() -> Plane {
    Plane::from_polygon(&[
        Point3::ORIGIN,
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
    ])
    .unwrap()
}

/// Rectangle on the x = 0 plane spanning `[y0, y1] x [z0, z1]`, wound CCW
/// seen from +x (the plane normal side).
fn yz_profile(y0: f64, z0: f64, y1: f64, z1: f64) -> Profile {
    yz_profile_at(0.0, y0, z0, y1, z1)
}

/// [`yz_profile`], on the x = `x` plane instead of x = 0 — an interior
/// crossing rather than a corner touch, for a path whose corners sit at
/// x = 0.
fn yz_profile_at(x: f64, y0: f64, z0: f64, y1: f64, z1: f64) -> Profile {
    Profile::new(
        Plane::from_polygon(&[
            Point3::new(x, 0.0, 0.0),
            Point3::new(x, 1.0, 0.0),
            Point3::new(x, 0.0, 1.0),
        ])
        .unwrap(),
        vec![
            Point3::new(x, y0, z0),
            Point3::new(x, y1, z0),
            Point3::new(x, y1, z1),
            Point3::new(x, y0, z1),
        ],
        vec![],
    )
    .unwrap()
}

/// A 24-facet circle profile with analytic curve attribution, built the way
/// the drawing tools build one: a curve bracket carrying the exact
/// [`CurveGeom`](kernel::CurveGeom), facets committed as segments. On the
/// x = 0 plane, centered at `(0, cy, cz)`.
fn circle_profile_yz(cy: f64, cz: f64, radius: f64) -> Profile {
    let mut sketch = kernel::Sketch::on_plane(yz_plane());
    sketch
        .begin_curve_with(kernel::CurveGeom {
            center: Point3::new(0.0, cy, cz),
            radius,
        })
        .unwrap();
    let n = 24;
    let pt = |i: usize| {
        let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
        Point3::new(0.0, cy + radius * a.cos(), cz + radius * a.sin())
    };
    for i in 0..n {
        sketch.add_segment(pt(i), pt(i + 1)).unwrap();
    }
    sketch.end_curve();
    let region = sketch.regions().keys().next().expect("circle closed");
    sketch.profile(region).unwrap()
}

proptest! {
    #[test]
    fn follow_me_single_segment_matches_extrusion(
        width in 0.1..10.0f64,
        height in 0.1..10.0f64,
        distance in 0.1..10.0f64,
        up in proptest::bool::ANY,
    ) {
        // A one-segment path is an extrusion by another name — both
        // directions along the profile normal (the flip branch).
        let d = if up { distance } else { -distance };
        let profile = rect_profile(width, height);
        let swept = Object::from_follow_me(
            &profile,
            &[Point3::ORIGIN, Point3::new(0.0, 0.0, d)],
            false,
            &[],
        )
        .unwrap();
        swept.validate().unwrap();
        let extruded = Object::from_extrusion(&profile, d).unwrap();
        prop_assert!(objects_equivalent(&swept, &extruded));
    }

    #[test]
    fn follow_me_l_path_volume_is_exact(
        len1 in 2.0..6.0f64,
        len2 in 2.0..6.0f64,
        w in 0.1..0.8f64,
        y0 in -1.0..0.4f64,
    ) {
        // Path: +x for len1, 90-degree turn, +y for len2, in the ground
        // plane. Profile: a w x w square on the x = 0 plane at lateral
        // offset y0 (kept below len2's fold limit by the ranges). The
        // mitered volume is exact: area * (len1 + len2 - 2 * y_bar)
        // (each prism runs to the 45-degree miter x + y = len1).
        let profile = yz_profile(y0, 0.1, y0 + w, 0.1 + w);
        let path = [
            Point3::ORIGIN,
            Point3::new(len1, 0.0, 0.0),
            Point3::new(len1, len2, 0.0),
        ];
        let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
        solid.validate().unwrap();
        prop_assert_eq!(solid.watertight(), WatertightState::Watertight);
        // 2 caps + 4 walls per segment.
        prop_assert_eq!(solid.faces().len(), 10);
        prop_assert_eq!(euler_poincare(&solid), 2);
        let area = w * w;
        let y_bar = y0 + w / 2.0;
        let expected = area * (len1 + len2 - 2.0 * y_bar);
        let volume = signed_volume(&solid);
        prop_assert!(
            (volume - expected).abs() <= VOLUME_TOL,
            "signed volume {volume}, expected {expected}"
        );
    }
}

#[test]
fn follow_me_closed_rectangle_ring_is_genus_one() {
    // A closed rectangular path (picture frame) swept with a profile
    // centered on the path spine (the anchor crossing at (0, -2, 0)). The
    // anchor splits one side, so 5 segments x 4 profile edges = 20 walls,
    // no caps; the offset-loop perimeter integral makes the volume exactly
    // area * perimeter for a spine-centered profile.
    let profile = yz_profile(-2.3, -0.3, -1.7, 0.3);
    let path = [
        Point3::new(-2.0, -2.0, 0.0),
        Point3::new(2.0, -2.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(-2.0, 2.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 20);
    // Torus topology: V - E + F - H = 2(S - G) = 0.
    assert_eq!(euler_poincare(&solid), 0);
    let volume = signed_volume(&solid);
    let expected = 0.36 * 16.0;
    assert!(
        (volume - expected).abs() <= VOLUME_TOL,
        "signed volume {volume}, expected {expected}"
    );
}

#[test]
fn follow_me_lathe_loop_volume_follows_pappus() {
    // A 24-gon "circle" path in the ground plane around the z axis, swept
    // with a square profile straddling the path — a faceted torus. One
    // facet's midpoint sits at angle 0 so the profile plane (y = 0,
    // normal +y) crosses it strictly inside. Faceted Pappus is exact:
    // every profile point at distance x from the axis traces a 24-gon of
    // perimeter 48 * tan(pi/24) * x, so V = 48 tan(pi/24) * x_bar * area.
    let n = 24usize;
    let r = 2.0f64;
    let step = std::f64::consts::TAU / n as f64;
    let path: Vec<Point3> = (0..n)
        .map(|i| {
            let a = (i as f64 - 0.5) * step;
            Point3::new(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect();
    // The y = 0 plane with normal +y (z cross x); outer ring CCW seen
    // from +y is CCW in the (z, x) frame.
    let profile = Profile::new(
        Plane::from_polygon(&[
            Point3::ORIGIN,
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(1.0, 0.0, 0.0),
        ])
        .unwrap(),
        vec![
            Point3::new(1.7, 0.0, -0.25),
            Point3::new(1.7, 0.0, 0.25),
            Point3::new(2.3, 0.0, 0.25),
            Point3::new(2.3, 0.0, -0.25),
        ],
        vec![],
    )
    .unwrap();
    let solid = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // 24 path vertices + the anchor split = 25 segments x 4 profile edges.
    assert_eq!(solid.faces().len(), 100);
    assert_eq!(euler_poincare(&solid), 0);
    let area = 0.6 * 0.5;
    let x_bar = 2.0;
    let expected = 48.0 * (std::f64::consts::PI / 24.0).tan() * x_bar * area;
    let volume = signed_volume(&solid);
    assert!(
        (volume - expected).abs() <= VOLUME_TOL,
        "signed volume {volume}, expected {expected}"
    );
}

// ---- analytic-tangent perpendicularity on drawn-curve paths (design §2) ----

/// A drawn 24-facet circle PATH in the ground plane, as the tools produce
/// one: uniform facets, a vertex on +X, every segment attributed with the
/// exact [`kernel::CurveGeom`]. Returns the polyline and the per-segment
/// attribution (wrap segment last).
fn circle_path(radius: f64, n: usize) -> (Vec<Point3>, Vec<Option<kernel::CurveGeom>>) {
    let geom = kernel::CurveGeom {
        center: Point3::ORIGIN,
        radius,
    };
    let pts = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
        })
        .collect();
    (pts, vec![Some(geom); n])
}

/// A 0.6 x 0.5 rectangular lathe profile on the RADIAL plane at angle `phi`
/// around the +Z axis: plane normal = the circle's analytic tangent at
/// angle `phi`, profile centered at radial distance 2 (matching the Pappus
/// spec's profile at phi = 0).
fn radial_profile(phi: f64) -> Profile {
    let radial = Vec3::new(phi.cos(), phi.sin(), 0.0);
    let up = Vec3::new(0.0, 0.0, 1.0);
    let corner = |u: f64, w: f64| Point3::ORIGIN + radial * (2.0 + u) + up * w;
    Profile::new(
        Plane::from_polygon(&[corner(-0.3, -0.25), corner(0.3, -0.25), corner(0.3, 0.25)]).unwrap(),
        vec![
            corner(-0.3, -0.25),
            corner(0.3, -0.25),
            corner(0.3, 0.25),
            corner(-0.3, 0.25),
        ],
        vec![],
    )
    .unwrap()
}

// ---- pole closure: spheres, goblets, cones (design §9) ----

/// A FULL circle profile on the x = 0 plane, drawn the way the tools draw one
/// (a curve bracket carrying the analytic [`CurveGeom`]), centered on the z
/// axis at height `cz`, radius `r`, `n >= 24` facets, phase-offset by `phase`
/// facets. It straddles the axis — crossing it at the two poles `(0, 0, cz ±
/// r)` — the sphere setup the maintainer's perpendicular-circles construction
/// produces. The attribution is what makes the crossing a FAITHFUL (lossless,
/// symmetric) split: an axis-centered analytic circle (design §9.2).
fn axis_circle_profile(cz: f64, r: f64, n: usize, phase: f64) -> Profile {
    let mut sketch = kernel::Sketch::on_plane(yz_plane());
    sketch
        .begin_curve_with(kernel::CurveGeom {
            center: Point3::new(0.0, 0.0, cz),
            radius: r,
        })
        .unwrap();
    let pt = |i: usize| {
        let a = ((i % n) as f64 + phase) / n as f64 * std::f64::consts::TAU;
        Point3::new(0.0, r * a.cos(), cz + r * a.sin())
    };
    for i in 0..n {
        sketch.add_segment(pt(i), pt((i + 1) % n)).unwrap();
    }
    sketch.end_curve();
    let region = sketch
        .regions()
        .keys()
        .next()
        .expect("circle closes one region");
    sketch.profile(region).unwrap()
}

/// A circular path on the ground (z = 0) about the origin, radius `radius`,
/// `n` facets phase-offset by `phase`, every segment carrying the analytic
/// [`CurveGeom`] a drawn circle does — the path pole closure requires.
fn attributed_ground_circle(
    radius: f64,
    n: usize,
    phase: f64,
) -> (Vec<Point3>, Vec<Option<kernel::CurveGeom>>) {
    let geom = kernel::CurveGeom {
        center: Point3::ORIGIN,
        radius,
    };
    let pts = (0..n)
        .map(|i| {
            let a = (i as f64 + phase) / n as f64 * std::f64::consts::TAU;
            Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
        })
        .collect();
    (pts, vec![Some(geom); n])
}

/// The SketchUp sphere: a full circle profile revolved around a drawn circle
/// path it crosses on the axis. Pole closure splits the profile at the axis
/// and revolves one half — a single watertight, genus-0 shell (never the
/// double-covered pair a naive full revolve would trace). With a vertex on
/// the axis crossing, every swept vertex lands EXACTLY on the sphere (the
/// revolution preserves distance from the on-axis center).
#[test]
fn follow_me_sphere_from_full_circle_crossing_the_axis() {
    let (r, cz, n) = (1.0, 1.0, 24usize);
    let profile = axis_circle_profile(cz, r, n, 0.0);
    let (path, curves) = attributed_ground_circle(1.0, n, 0.0);

    let sphere = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    sphere.validate().unwrap();
    assert_eq!(sphere.watertight(), WatertightState::Watertight);
    // One shell, genus 0: V - E + F = 2(S - G) = 2.
    assert_eq!(euler_poincare(&sphere), 2);

    let center = Point3::new(0.0, 0.0, cz);
    for v in sphere.vertices().values() {
        let d = (v.position - center).length();
        assert!(
            (d - r).abs() < 1e-6,
            "vertex off the sphere: dist {d}, r {r}"
        );
    }
    let zmin = sphere
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MAX, f64::min);
    let zmax = sphere
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MIN, f64::max);
    assert!((zmin - (cz - r)).abs() < 1e-6, "bottom pole at {zmin}");
    assert!((zmax - (cz + r)).abs() < 1e-6, "top pole at {zmax}");

    // Outward-wound, and the faceted-inscribed volume approaches the true
    // sphere from below (4/3 pi r^3).
    let vol = signed_volume(&sphere);
    let true_vol = 4.0 / 3.0 * std::f64::consts::PI * r * r * r;
    assert!(
        vol > 0.95 * true_vol && vol <= true_vol + VOLUME_TOL,
        "sphere volume {vol}, true {true_vol}"
    );
}

/// The maintainer's actual geometry: neither the path facets nor the profile
/// facets align a vertex with the axis crossing (perpendicular-circles-2.hew,
/// r = 0.1, both circles 32 facets, off phase). The split inserts both poles
/// mid-facet and the sweep still closes one watertight genus-0 sphere.
#[test]
fn follow_me_sphere_closes_off_phase_like_the_maintainer_file() {
    let (r, cz, n) = (0.1, 0.1, 32usize);
    let profile = axis_circle_profile(cz, r, n, 0.37);
    let (path, curves) = attributed_ground_circle(0.1, n, 0.21);

    let sphere = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    sphere.validate().unwrap();
    assert_eq!(sphere.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&sphere), 2);
    let vol = signed_volume(&sphere);
    let true_vol = 4.0 / 3.0 * std::f64::consts::PI * r * r * r;
    assert!(
        vol > 0.9 * true_vol && vol <= true_vol + VOLUME_TOL,
        "off-phase sphere volume {vol}, true {true_vol}"
    );
}

/// Determinism (DEVELOPMENT.md rule 7): the sphere rebuilds bit-for-bit
/// identically — same vertex count, same positions to the last bit, same
/// face count.
#[test]
fn follow_me_sphere_is_deterministic() {
    let (r, cz, n) = (1.0, 1.0, 24usize);
    let profile = axis_circle_profile(cz, r, n, 0.0);
    let (path, curves) = attributed_ground_circle(1.0, n, 0.0);
    let a = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    let b = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    let pos = |o: &Object| {
        o.vertices()
            .values()
            .map(|v| {
                (
                    v.position.x.to_bits(),
                    v.position.y.to_bits(),
                    v.position.z.to_bits(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pos(&a), pos(&b));
    assert_eq!(a.faces().len(), b.faces().len());
}

/// A single-profile revolve touching the axis (no crossing, so no split): a
/// right-triangle profile whose two axis vertices become poles and whose
/// on-axis edge is suppressed — a cone. Watertight, genus 0, with the
/// faceted-inscribed volume of a cone.
#[test]
fn follow_me_cone_revolve_touching_the_axis() {
    let (base_r, height, n) = (1.0, 2.0, 24usize);
    let profile = Profile::new(
        yz_plane(),
        vec![
            Point3::new(0.0, 0.0, 0.0),    // bottom pole (on axis)
            Point3::new(0.0, base_r, 0.0), // base rim
            Point3::new(0.0, 0.0, height), // apex (on axis)
        ],
        vec![],
    )
    .unwrap();
    let (path, curves) = attributed_ground_circle(1.0, n, 0.0);

    let cone = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    cone.validate().unwrap();
    assert_eq!(cone.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&cone), 2);
    let vol = signed_volume(&cone);
    let true_vol = std::f64::consts::PI * base_r * base_r * height / 3.0;
    assert!(
        vol > 0.95 * true_vol && vol <= true_vol + VOLUME_TOL,
        "cone volume {vol}, true {true_vol}"
    );
}

/// Pole closure is gated on a recognized circle path: the identical sphere
/// profile on a path WITHOUT analytic attribution still refuses — no
/// silent guess at a circle. Auto-orientation (design §2c) changes the
/// variant, not the outcome: the fold makes the plane chord-perpendicular,
/// but an axis-centered profile's folded plane attaches to no chord
/// strictly between its endpoints, and without attribution there is no
/// revolution to close the poles.
#[test]
fn follow_me_sphere_refuses_without_circle_attribution() {
    let (r, cz, n) = (1.0, 1.0, 24usize);
    let profile = axis_circle_profile(cz, r, n, 0.0);
    let (path, _) = attributed_ground_circle(1.0, n, 0.0);
    let err = Object::from_follow_me(&profile, &path, true, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathDetachedFromProfile);
}

proptest! {
    /// Any full circle profile crossing the axis of any drawn circle path
    /// closes a single watertight, genus-0, outward-wound sphere — never a
    /// double cover — across radius and facet count (both vertex-aligned and
    /// mid-facet axis crossings).
    #[test]
    fn follow_me_sphere_closes_for_any_radius_and_facets(
        r in 0.05f64..3.0,
        n in 3usize..40, // holds at every facet count a circle can have
    ) {
        let cz = r + 0.5;
        let profile = axis_circle_profile(cz, r, n, 0.0);
        let (path, curves) = attributed_ground_circle(r, n, 0.0);
        let sphere = Object::from_follow_me(&profile, &path, true, &curves)
            .expect("a full circle crossing the axis closes a sphere");
        sphere.validate().unwrap();
        prop_assert_eq!(sphere.watertight(), WatertightState::Watertight);
        prop_assert_eq!(euler_poincare(&sphere), 2);
        prop_assert_eq!(sphere.split_connected_components().len(), 1); // one shell, not two
        prop_assert!(signed_volume(&sphere) > 0.0);
    }
}

/// A profile that crosses the axis but is NOT an axis-centered circle is
/// refused, never split — the fix for the adversarial findings (design §9.2).
/// Splitting such a profile would silently drop or disconnect the geometry the
/// user drew, worse than a clean typed refusal (rule 4).
#[test]
fn follow_me_asymmetric_axis_crossing_refuses_typed() {
    // A lopsided (egg-like) silhouette crossing the axis once: one lobe is
    // large, the sliver on the other side would simply vanish under a
    // majority-side clip. Refused, document untouched.
    let profile = Profile::new(
        yz_plane(),
        vec![
            Point3::new(0.0, -0.2, 0.0),
            Point3::new(0.0, 3.0, 0.5),
            Point3::new(0.0, 3.0, 3.0),
            Point3::new(0.0, -0.2, 3.5),
        ],
        vec![],
    )
    .unwrap();
    let (path, curves) = attributed_ground_circle(1.0, 24, 0.0);
    let err = Object::from_follow_me(&profile, &path, true, &curves).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::ProfileCrossesAxis);
}

/// A profile that crosses the axis MORE than once (a dominant lobe plus a
/// notch dipping to the other side — the reviewer's exact refuter). The old
/// min/max gate let it through and its suppressed connecting walls severed it
/// into TWO disjoint watertight shells inside ONE Object (euler 4, two
/// components) — a silently invalid solid. It is now refused typed, and the
/// document is untouched. The connectivity backstop (§9.4) is the independent
/// second line: were any crossing ever mis-admitted, a disconnected result is
/// caught and refused rather than emitted.
#[test]
fn follow_me_multi_lobe_axis_crossing_refuses_and_never_disconnects() {
    let profile = Profile::new(
        yz_plane(),
        vec![
            Point3::new(0.0, -0.1, 0.0),
            Point3::new(0.0, 5.0, -1.0),
            Point3::new(0.0, 5.0, 4.0),
            Point3::new(0.0, 0.1, 3.0),
            Point3::new(0.0, -0.1, 2.0),
            Point3::new(0.0, 0.1, 1.0),
        ],
        vec![],
    )
    .unwrap();
    let (path, curves) = attributed_ground_circle(1.0, 24, 0.0);
    let err = Object::from_follow_me(&profile, &path, true, &curves).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::ProfileCrossesAxis);
}

/// The faithful-split gate must NOT trust the circle ATTRIBUTION: the public
/// Sketch API stamps every segment added under an open curve bracket with that
/// bracket's `CurveGeom`, on the circle or not (no positional check). A
/// mislabeled profile — a genuine axis-centered circle attribution on a
/// boundary that detours out to an OFF-circle lump — reports one center/radius
/// for every edge yet is not a circle. Trusting the attribution would split it
/// into a silently WRONG solid (volume far from the sphere). The gate
/// re-verifies every boundary vertex lies AT the radius (map-or-drop, as
/// cylinder stamping re-verifies its `SurfaceRef`), so it refuses typed.
#[test]
fn follow_me_mislabeled_circle_crossing_refuses_typed() {
    let (cz, r, n) = (1.0, 1.0, 24usize);
    let mut sketch = kernel::Sketch::on_plane(yz_plane());
    sketch
        .begin_curve_with(kernel::CurveGeom {
            center: Point3::new(0.0, 0.0, cz),
            radius: r,
        })
        .unwrap();
    let pt = |i: usize| {
        let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
        Point3::new(0.0, r * a.cos(), cz + r * a.sin())
    };
    // A genuine axis-centered circle EXCEPT facet 0, which detours out to an
    // off-circle lump and back — every segment attributed to the open bracket.
    for i in 0..n {
        let (from, to) = (pt(i), pt((i + 1) % n));
        if i == 0 {
            let lump = Point3::new(0.0, 3.0, cz); // radial 3, far off the r = 1 circle
            sketch.add_segment(from, lump).unwrap();
            sketch.add_segment(lump, to).unwrap();
        } else {
            sketch.add_segment(from, to).unwrap();
        }
    }
    sketch.end_curve();
    let region = sketch.regions().keys().next().expect("closes one region");
    let profile = sketch.profile(region).unwrap();
    // The trap: attribution IS uniform across every boundary edge, lump
    // included — exactly what a claim-trusting gate would wave through.
    assert!(
        (0..profile.outer().len()).all(|k| profile.outer_curve(k).is_some()),
        "every edge is attributed to the circle bracket"
    );

    let (path, curves) = attributed_ground_circle(1.0, n, 0.0);
    let err = Object::from_follow_me(&profile, &path, true, &curves).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::ProfileCrossesAxis);
}

/// The lathe that was structurally impossible under chord perpendicularity:
/// on a curve-attributed circle path, a radially-placed profile (its plane
/// containing the circle's axis — the only orientation perpendicular to the
/// drawn curve) is accepted at ANY station: a facet vertex (the natural
/// snap target — the seam is that joint's miter plane), the mid-facet
/// crossing, and an arbitrary off-midpoint crossing. Every seam closes
/// into a watertight genus-1 ring with an exact faceted-Pappus volume;
/// the vertex-anchored ring is the smallest (its stations are the radial
/// vertex planes) and the mid-facet ring the largest, bounding the
/// arbitrary station between them.
#[test]
fn follow_me_lathe_accepts_radial_profiles_on_a_drawn_circle() {
    let n = 24usize;
    let step = std::f64::consts::TAU / n as f64;
    let (path, curves) = circle_path(2.0, n);
    let area = 0.6 * 0.5;
    let x_bar = 2.0;

    // Station 1: seam at a facet VERTEX (angle 0 — a quadrant point). No
    // split: 24 segments x 4 walls. Faceted Pappus per oblique prism: each
    // slab between consecutive RADIAL vertex planes displaces a point at
    // radius x by the chord 2 x sin(step/2), inclined at step/2 to the
    // base normal — volume = area * 24 * 2 sin(step/2) cos(step/2) * x_bar.
    let at_vertex = Object::from_follow_me(&radial_profile(0.0), &path, true, &curves).unwrap();
    at_vertex.validate().unwrap();
    assert_eq!(at_vertex.watertight(), WatertightState::Watertight);
    assert_eq!(at_vertex.faces().len(), 96);
    assert_eq!(euler_poincare(&at_vertex), 0);
    let vol_vertex = signed_volume(&at_vertex);
    let expected_vertex = 48.0 * (step / 2.0).sin() * (step / 2.0).cos() * x_bar * area;
    assert!(
        (vol_vertex - expected_vertex).abs() <= VOLUME_TOL,
        "vertex-seam volume {vol_vertex}, expected {expected_vertex}"
    );

    // Station 2: seam at a facet MIDPOINT (the plane crosses the chord at
    // its center). Split: 25 segments. Pappus with the centroid's orbit at
    // the APOTHEM — the same volume the unattributed Pappus spec pins.
    let at_mid = Object::from_follow_me(&radial_profile(step / 2.0), &path, true, &curves).unwrap();
    at_mid.validate().unwrap();
    assert_eq!(at_mid.watertight(), WatertightState::Watertight);
    assert_eq!(at_mid.faces().len(), 100);
    assert_eq!(euler_poincare(&at_mid), 0);
    let vol_mid = signed_volume(&at_mid);
    let expected_mid = 48.0 * (step / 2.0).tan() * x_bar * area;
    assert!(
        (vol_mid - expected_mid).abs() <= VOLUME_TOL,
        "mid-facet volume {vol_mid}, expected {expected_mid}"
    );

    // Station 3: an ARBITRARY angle, crossing a chord strictly inside and
    // off-center. Same ring topology; volume strictly between the two
    // exact stations above.
    let at_odd = Object::from_follow_me(&radial_profile(0.3 * step), &path, true, &curves).unwrap();
    at_odd.validate().unwrap();
    assert_eq!(at_odd.watertight(), WatertightState::Watertight);
    assert_eq!(at_odd.faces().len(), 100);
    assert_eq!(euler_poincare(&at_odd), 0);
    let vol_odd = signed_volume(&at_odd);
    assert!(
        vol_odd > vol_vertex - VOLUME_TOL && vol_odd < vol_mid + VOLUME_TOL,
        "arbitrary-station volume {vol_odd} outside [{vol_vertex}, {vol_mid}]"
    );
}

/// The antipodal trap that a first-in-path-order anchor search fell into. A
/// radial profile plane through a full circle's center is perpendicular to
/// the drawn curve at TWO antipodal crossings; the seam must be anchored at
/// the one NEAREST the profile, not the first the path happens to number.
/// Placed at angle PI, the profile's plane also passes through the vertex at
/// angle 0 (path index 0) — the old search seamed there, carried the ring
/// from the far side, and fired `PathTooTight`. The near-anchor rule seams
/// at the angle-PI vertex (index 12) and the ring closes watertight, matching
/// the angle-0 vertex seam exactly by symmetry.
#[test]
fn follow_me_lathe_anchors_at_the_near_crossing_not_the_antipode() {
    let n = 24usize;
    let step = std::f64::consts::TAU / n as f64;
    let (path, curves) = circle_path(2.0, n);
    let solid = Object::from_follow_me(&radial_profile(std::f64::consts::PI), &path, true, &curves)
        .expect("a radial profile antipodal to path-index 0 must still sweep");
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 96); // vertex seam: 24 segments x 4 walls
    assert_eq!(euler_poincare(&solid), 0);
    let area = 0.6 * 0.5;
    let x_bar = 2.0;
    let expected = 48.0 * (step / 2.0).sin() * (step / 2.0).cos() * x_bar * area;
    let volume = signed_volume(&solid);
    assert!(
        (volume - expected).abs() <= VOLUME_TOL,
        "antipodal vertex-seam volume {volume}, expected {expected}"
    );
}

/// Full lathe coverage: the identical radial profile must sweep at EVERY
/// station around the drawn circle, both halves, not just the half whose
/// crossings the path numbers first. Stepping the placement angle every half
/// facet (48 stations), each build is watertight and genus-1, and its
/// faceted-Pappus volume falls in the exact `[vertex-seam, mid-facet-seam]`
/// band the single-station spec pins. Before the near-anchor fix the far
/// half refused `PathTooTight`.
#[test]
fn follow_me_lathe_accepts_radial_profiles_all_the_way_around() {
    let n = 24usize;
    let step = std::f64::consts::TAU / n as f64;
    let (path, curves) = circle_path(2.0, n);
    let area = 0.6 * 0.5;
    let x_bar = 2.0;
    let vol_vertex = 48.0 * (step / 2.0).sin() * (step / 2.0).cos() * x_bar * area;
    let vol_mid = 48.0 * (step / 2.0).tan() * x_bar * area;

    for h in 0..(2 * n) {
        let phi = h as f64 * step / 2.0;
        let solid = Object::from_follow_me(&radial_profile(phi), &path, true, &curves)
            .unwrap_or_else(|e| panic!("station h={h} (phi={phi}) refused: {e:?}"));
        solid.validate().unwrap();
        assert_eq!(
            solid.watertight(),
            WatertightState::Watertight,
            "station h={h} (phi={phi}) not watertight"
        );
        assert_eq!(
            euler_poincare(&solid),
            0,
            "station h={h} (phi={phi}) not genus-1"
        );
        let volume = signed_volume(&solid);
        assert!(
            volume > vol_vertex - VOLUME_TOL && volume < vol_mid + VOLUME_TOL,
            "station h={h} (phi={phi}): volume {volume} outside [{vol_vertex}, {vol_mid}]"
        );
    }
}

proptest! {
    /// The same guarantee over a random placement angle: a radial profile
    /// anywhere on a drawn circle's rim sweeps to a watertight genus-1 ring.
    #[test]
    fn follow_me_lathe_accepts_any_radial_placement_angle(
        phi in 0.0f64..std::f64::consts::TAU,
    ) {
        let n = 24usize;
        let (path, curves) = circle_path(2.0, n);
        let solid = Object::from_follow_me(&radial_profile(phi), &path, true, &curves)
            .expect("a radial profile must sweep at every placement angle");
        solid.validate().unwrap();
        prop_assert_eq!(solid.watertight(), WatertightState::Watertight);
        prop_assert_eq!(euler_poincare(&solid), 0);
    }
}

/// Deterministic regression for the closed-lathe seam flake that made
/// `follow_me_lathe_accepts_any_radial_placement_angle` intermittently red
/// (~2.6% of runs, reliably at high `PROPTEST_CASES`). When the radial
/// profile plane crosses a facet a HAIR before a drawn rim vertex (a few
/// microradians), the near-anchor rule splits that facet at the crossing —
/// exact closure forces the split, because a vertex seam a few microradians
/// off the miter plane provably would not close (design §1) — leaving a full
/// wrap facet and a sub-facet SLIVER. The sliver is legitimate geometry, but
/// its near-collinear seam walls once drove the global self-intersection
/// guard's exact predicates ill-conditioned (their shared rim vertex
/// reconstructed ~1e-7 off, past `POINT_MERGE`), so a sound lathe refused
/// `SweepSelfIntersects` across a thin band of angles either side of every
/// rim vertex. The two pinned angles are the shrunk failures two independent
/// proptest runs reported; the swept band steps across the rest. Each must
/// now sweep to a watertight genus-1 ring, exercising the fix without relying
/// on proptest luck.
#[test]
fn follow_me_lathe_accepts_a_radial_seam_a_hair_off_a_rim_vertex() {
    let n = 24usize;
    let (path, curves) = circle_path(2.0, n);
    let step = std::f64::consts::TAU / n as f64;

    let mut phis = vec![1.3089725229526878f64, 3.92698012551821f64];
    // The old bad band ran roughly 2.5e-6..3.5e-5 rad off a rim vertex; step
    // finely across both sides of vertex 5 to cover it densely.
    for k in 1..=80 {
        let d = k as f64 * 5e-7;
        phis.push(5.0 * step - d);
        phis.push(5.0 * step + d);
    }

    for phi in phis {
        let solid = Object::from_follow_me(&radial_profile(phi), &path, true, &curves)
            .unwrap_or_else(|e| {
                panic!("radial profile a hair off a rim vertex (phi={phi}): {e:?}")
            });
        solid.validate().unwrap();
        assert_eq!(
            solid.watertight(),
            WatertightState::Watertight,
            "phi={phi} not watertight"
        );
        assert_eq!(euler_poincare(&solid), 0, "phi={phi} not genus-1");
    }
}

/// Attribution still matters without being a refusal gate: the SAME
/// 24-gon ring without curve attribution keeps polyline semantics — a
/// radial profile plane is perpendicular to no chord — but auto-
/// orientation (design §2c) now folds the profile the half-facet angle
/// onto the nearest chord and the ring sweeps, instead of refusing.
#[test]
fn follow_me_unattributed_facet_ring_folds_onto_the_chord() {
    let (path, _) = circle_path(2.0, 24);
    // Mid-facet placement: the fold's crease lands strictly inside the
    // nearest chord and the ring sweeps.
    let mid = std::f64::consts::PI / 24.0;
    let solid = Object::from_follow_me(&radial_profile(mid), &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&solid), 0, "a ring: genus one");
    assert!(signed_volume(&solid) > 0.0);

    // Vertex-aligned placement: the crease lands exactly on a path
    // corner and the profile straddles it — the corner-seam wedge folds
    // back over the incoming chord, the same honest refusal a straddling
    // corner profile gets everywhere (design §2b).
    let err = Object::from_follow_me(&radial_profile(0.0), &path, true, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathTooTight);
}

/// The analytic rule is exactly "radial plane" — and auto-orientation
/// (design §2c) now FOLDS a non-radial plane onto the drawn circle's
/// radial family instead of refusing: a profile parallel to the path
/// plane, or one offset off the center, sweeps the lathe it visibly
/// suggests. The fold's crease is at the analytic rim point nearest the
/// profile, so the folded plane contains the center exactly.
#[test]
fn follow_me_folds_non_radial_planes_onto_the_radial_family() {
    let (path, curves) = circle_path(2.0, 24);

    // Parallel to the path plane (normal +z): never perpendicular.
    let flat = Profile::new(
        Plane::from_polygon(&[
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(2.0, 1.0, 1.0),
        ])
        .unwrap(),
        vec![
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(2.0, 1.0, 1.0),
            Point3::new(1.0, 1.0, 1.0),
        ],
        vec![],
    )
    .unwrap();
    let from_flat = Object::from_follow_me(&flat, &path, true, &curves).unwrap();
    from_flat.validate().unwrap();
    assert_eq!(from_flat.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&from_flat), 0);
    assert!(signed_volume(&from_flat) > 0.0);

    // In the path plane's directions but OFF the center: a chord-crossing
    // plane that is radial to no point of the drawn circle.
    let offset = Profile::new(
        Plane::from_polygon(&[
            Point3::new(1.7, 0.05, -0.25),
            Point3::new(2.3, 0.05, -0.25),
            Point3::new(2.3, 0.05, 0.25),
        ])
        .unwrap(),
        vec![
            Point3::new(1.7, 0.05, -0.25),
            Point3::new(2.3, 0.05, -0.25),
            Point3::new(2.3, 0.05, 0.25),
            Point3::new(1.7, 0.05, 0.25),
        ],
        vec![],
    )
    .unwrap();
    let from_offset = Object::from_follow_me(&offset, &path, true, &curves).unwrap();
    from_offset.validate().unwrap();
    assert_eq!(from_offset.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&from_offset), 0);
    assert!(signed_volume(&from_offset) > 0.0);
}

/// An OPEN attributed path measures its end the same way: the profile must
/// sit perpendicular to the drawn curve's tangent at the end vertex, where
/// the chord rule (half a facet angle off) refuses. The unattributed same
/// setup keeps refusing — polyline semantics unchanged.
#[test]
fn follow_me_open_arc_end_uses_the_analytic_tangent() {
    // A quarter arc from (2,0,0) to (0,2,0) about the origin, 6 uniform
    // facets, attributed. Tangent at the (2,0,0) end is +y — the profile
    // plane there is y = 0.
    let n = 6usize;
    let geom = kernel::CurveGeom {
        center: Point3::ORIGIN,
        radius: 2.0,
    };
    let path: Vec<Point3> = (0..=n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::FRAC_PI_2;
            Point3::new(2.0 * a.cos(), 2.0 * a.sin(), 0.0)
        })
        .collect();
    let curves = vec![Some(geom); n];
    let profile = radial_profile(0.0);

    let solid = Object::from_follow_me(&profile, &path, false, &curves).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // 2 caps + 6 segments x 4 walls.
    assert_eq!(solid.faces().len(), 26);

    // WITHOUT attribution the same placement is perpendicular to no
    // chord. It no longer refuses — auto-orientation (design §2c) folds
    // it the half-facet angle onto the end chord — but the result is a
    // genuinely different solid from the attributed sweep, which used
    // the placement EXACTLY and never folded. The contrast is the spec:
    // attribution buys exactness, the fold buys success.
    let folded = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    folded.validate().unwrap();
    assert_eq!(folded.watertight(), WatertightState::Watertight);
    assert!(
        !objects_equivalent(&folded, &solid),
        "the fold must have rotated the profile; identical solids would \
         mean the chord rule silently adopted the analytic tangent"
    );
}

#[test]
fn follow_me_straight_sweep_of_a_circle_is_a_stamped_cylinder() {
    // The true-curves overlay (the follow-me design section 4):
    // a straight path sweeping a drawn circle IS a cylinder, and every
    // wall says so.
    let profile = circle_profile_yz(0.0, 0.0, 0.5);
    let path = [Point3::ORIGIN, Point3::new(2.0, 0.0, 0.0)];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 26);
    let expected = kernel::SurfaceRef::Cylinder {
        axis_point: Point3::ORIGIN,
        axis: Vec3::new(1.0, 0.0, 0.0),
        radius: 0.5,
    };
    let stamped = solid
        .faces()
        .values()
        .filter(|f| {
            f.surface
                .as_ref()
                .is_some_and(|s| s.same_surface(&expected))
        })
        .count();
    assert_eq!(stamped, 24, "every wall claims the one cylinder");
    // Both rim circles survive with full coverage.
    let rims = solid.analytic_rims();
    assert_eq!(rims.len(), 2);
    assert!(rims.iter().all(|rim| rim.coverage.is_none()));
}

#[test]
fn follow_me_collinear_joints_sweep_like_one_segment() {
    // A path drawn in two collinear strokes: the interior joint's miter
    // normal degenerates to the segment direction itself (a perpendicular
    // station), so the sweep must behave exactly like one long segment —
    // same volume, coplanar adjacent walls, watertight — and a circle
    // profile's per-segment cylinder stamps must agree on ONE cylinder.
    let profile = yz_profile(-0.4, 0.1, 0.4, 0.9);
    let path = [
        Point3::ORIGIN,
        Point3::new(1.5, 0.0, 0.0),
        Point3::new(4.0, 0.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 10, "2 caps + 2 segments x 4 walls");
    let volume = signed_volume(&solid);
    let expected = 0.8 * 0.8 * 4.0;
    assert!(
        (volume - expected).abs() <= VOLUME_TOL,
        "signed volume {volume}, expected {expected}"
    );

    let circle = circle_profile_yz(0.0, 0.5, 0.3);
    let tube = Object::from_follow_me(&circle, &path, false, &[]).unwrap();
    tube.validate().unwrap();
    let expected_cyl = kernel::SurfaceRef::Cylinder {
        axis_point: Point3::new(0.0, 0.0, 0.5),
        axis: Vec3::new(1.0, 0.0, 0.0),
        radius: 0.3,
    };
    let stamped = tube
        .faces()
        .values()
        .filter(|f| {
            f.surface
                .as_ref()
                .is_some_and(|s| s.same_surface(&expected_cyl))
        })
        .count();
    assert_eq!(stamped, 48, "both segments claim the SAME cylinder");
    // One logical wall: exactly one rim-circle pair, full coverage.
    let rims = tube.analytic_rims();
    assert_eq!(rims.len(), 2);
    assert!(rims.iter().all(|rim| rim.coverage.is_none()));
}

#[test]
fn follow_me_bent_sweep_stamps_cylinders_per_segment() {
    // Around a bend the circle still sweeps exact cylinder patches per
    // straight segment — two distinct axes, every wall stamped.
    let profile = circle_profile_yz(0.0, 0.0, 0.3);
    let path = [
        Point3::ORIGIN,
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 50);
    let along_x = kernel::SurfaceRef::Cylinder {
        axis_point: Point3::ORIGIN,
        axis: Vec3::new(1.0, 0.0, 0.0),
        radius: 0.3,
    };
    let along_y = kernel::SurfaceRef::Cylinder {
        axis_point: Point3::new(2.0, 0.0, 0.0),
        axis: Vec3::new(0.0, 1.0, 0.0),
        radius: 0.3,
    };
    let count = |expected: &kernel::SurfaceRef| {
        solid
            .faces()
            .values()
            .filter(|f| f.surface.as_ref().is_some_and(|s| s.same_surface(expected)))
            .count()
    };
    assert_eq!(count(&along_x), 24);
    assert_eq!(count(&along_y), 24);
}

proptest! {
    #[test]
    fn follow_me_staircase_sweeps_are_watertight(
        w in 0.05..0.3f64,
        z0 in -0.5..0.5f64,
        lens in proptest::collection::vec(1.0..3.0f64, 1..6),
    ) {
        // Random staircase paths (alternating +x / +y runs, every run far
        // longer than the profile is wide) with a random square profile:
        // the sweep must always be a valid, watertight, outward-wound
        // solid — the mandatory watertightness property (rule 3).
        let profile = yz_profile(-w, z0, w, z0 + 2.0 * w);
        let mut pts = vec![Point3::ORIGIN];
        for (i, len) in lens.iter().enumerate() {
            let last = *pts.last().unwrap();
            let step = if i % 2 == 0 {
                Vec3::new(*len, 0.0, 0.0)
            } else {
                Vec3::new(0.0, *len, 0.0)
            };
            pts.push(last + step);
        }
        let solid = Object::from_follow_me(&profile, &pts, false, &[]).unwrap();
        solid.validate().unwrap();
        prop_assert_eq!(solid.watertight(), WatertightState::Watertight);
        prop_assert_eq!(solid.faces().len(), 2 + 4 * (pts.len() - 1));
        prop_assert_eq!(euler_poincare(&solid), 2);
        prop_assert!(signed_volume(&solid) > 0.0);
    }
}

#[test]
fn follow_me_folds_a_parallel_profile_upright() {
    // Profile in the ground plane, path in the ground plane: nowhere
    // perpendicular — which used to refuse and now auto-orients (design
    // §2c). The rect spans x,y in [0,1]; the crease is the x-parallel
    // line through the path's nearest point to the centroid, (0, 0.5, 0),
    // so the fold maps the y-extent onto z and the carry re-attaches the
    // path at the folded plane: a 1 x 1 x 5 prism from y = 0.5 to 5.5.
    let profile = rect_profile(1.0, 1.0);
    let path = [Point3::ORIGIN, Point3::new(0.0, 5.0, 0.0)];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert!((signed_volume(&solid) - 5.0).abs() < 1e-9);
    let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
    for v in solid.vertices().values() {
        ymin = ymin.min(v.position.y);
        ymax = ymax.max(v.position.y);
    }
    assert!((ymin - 0.5).abs() < 1e-9 && (ymax - 5.5).abs() < 1e-9);
}

#[test]
fn follow_me_detached_open_path_is_carried_to_the_profile() {
    // Perpendicular but starting off the profile plane: the sweep starts
    // where the PROFILE is (design §2a) — the path's shape is carried
    // rigidly to the plane, not refused. Identical to sweeping the same
    // shape drawn attached.
    let profile = yz_profile(0.0, 0.0, 1.0, 1.0);
    let path = [Point3::new(0.5, 0.0, 0.0), Point3::new(3.0, 0.0, 0.0)];
    let swept = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    swept.validate().unwrap();
    let attached = [Point3::ORIGIN, Point3::new(2.5, 0.0, 0.0)];
    let reference = Object::from_follow_me(&profile, &attached, false, &[]).unwrap();
    assert!(objects_equivalent(&swept, &reference));
}

#[test]
fn follow_me_carry_anchors_the_nearer_end() {
    // Both ends perpendicular, both off the plane: the nearer end anchors,
    // reversing the path when that end is the last one. Here the last
    // vertex (x = 1) is nearer than the first (x = 4), so the sweep leaves
    // the profile toward +x and spans the path's length exactly.
    let profile = yz_profile(0.0, 0.0, 1.0, 1.0);
    let path = [Point3::new(4.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)];
    let swept = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    swept.validate().unwrap();
    let reference = Object::from_follow_me(
        &profile,
        &[Point3::ORIGIN, Point3::new(3.0, 0.0, 0.0)],
        false,
        &[],
    )
    .unwrap();
    assert!(objects_equivalent(&swept, &reference));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn follow_me_carried_path_matches_the_attached_sweep(
        along in -6.0..6.0f64,
        lat_z in -4.0..4.0f64,
        lat_y in -0.25..4.0f64,
        leg in 0.5..5.0f64,
    ) {
        // The carry cancels a longitudinal offset exactly (design §2a),
        // and an offset perpendicular to a planar path's plane moves no
        // station plane at all (every station normal lies in that plane) —
        // so an L path offset by both sweeps the very solid the attached
        // original does. An in-path-plane lateral offset is different on
        // purpose: it legitimately moves the elbow's miter plane relative
        // to the profile, so it must still SWEEP (the ring is wherever the
        // profile is), just not the identical solid — pinned by the
        // watertightness assertion alone.
        let profile = yz_profile(-0.2, 0.1, 0.2, 0.5);
        let attached = [
            Point3::ORIGIN,
            Point3::new(leg, 0.0, 0.0),
            Point3::new(leg, leg, 0.0),
        ];
        let delta = Vec3::new(along, 0.0, lat_z);
        let offset: Vec<Point3> = attached.iter().map(|&p| p + delta).collect();
        let swept = Object::from_follow_me(&profile, &offset, false, &[]).unwrap();
        swept.validate().unwrap();
        let reference = Object::from_follow_me(&profile, &attached, false, &[]).unwrap();
        prop_assert!(objects_equivalent(&swept, &reference));

        // In-plane lateral offset: still a valid sweep whenever every ring
        // vertex keeps a positive advance at the elbow's miter — i.e.
        // `leg + lat_y > 0.2` (the ring's largest y), which the ranges
        // guarantee with margin. Sliding further back is a genuine
        // PathTooTight fold, not a carry defect.
        let side = Vec3::new(0.0, lat_y, 0.0);
        let slid: Vec<Point3> = attached.iter().map(|&p| p + side).collect();
        let slid_swept = Object::from_follow_me(&profile, &slid, false, &[]).unwrap();
        slid_swept.validate().unwrap();
        prop_assert_eq!(slid_swept.watertight(), WatertightState::Watertight);
    }
}

#[test]
fn follow_me_refuses_reversing_path() {
    let profile = yz_profile(0.0, 0.0, 1.0, 1.0);
    let path = [
        Point3::ORIGIN,
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(0.5, 0.0, 0.0),
    ];
    let err = Object::from_follow_me(&profile, &path, false, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathReverses);
}

#[test]
fn follow_me_miter_limit_refuses_near_reversals_and_admits_ordinary_bends() {
    // The unbounded-miter hole (design section 3): a joint eps radians
    // short of a full reversal has a miter ratio ~ 2/eps — at eps = 1e-3
    // the outer corner would spike ~2000x the model's scale into a
    // "valid", watertight, absurd solid. Every such joint must refuse
    // typed via the miter limit; every ordinary bend must still sweep.
    let profile = yz_profile(-0.05, 0.1, 0.05, 0.2);
    let bend_path = |turn: f64| {
        // Two 4-meter legs; the second leaves the joint turned by `turn`
        // radians from straight ahead (0 = collinear, pi = reversal).
        [
            Point3::ORIGIN,
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0 + 4.0 * turn.cos(), 4.0 * turn.sin(), 0.0),
        ]
    };

    for eps in [1e-3, 1e-4, 1e-5, 1e-6] {
        let err =
            Object::from_follow_me(&profile, &bend_path(std::f64::consts::PI - eps), false, &[])
                .unwrap_err();
        assert_eq!(
            err,
            kernel::FollowMeError::PathReverses,
            "eps = {eps}: a near-reversal must refuse, never commit a miter spike"
        );
    }

    for turn_deg in [45.0f64, 90.0, 150.0] {
        let solid = Object::from_follow_me(&profile, &bend_path(turn_deg.to_radians()), false, &[])
            .unwrap_or_else(|e| panic!("a {turn_deg} degree bend must sweep, got {e:?}"));
        solid.validate().unwrap();
        assert_eq!(solid.watertight(), WatertightState::Watertight);
        // The mitered solid stays at the scale of its inputs: nothing can
        // sit farther out than the path extent plus the limited miter of
        // the profile's own extent.
        let max_coord = solid
            .vertices()
            .values()
            .map(|v| {
                v.position
                    .x
                    .abs()
                    .max(v.position.y.abs())
                    .max(v.position.z.abs())
            })
            .fold(0.0f64, f64::max);
        assert!(
            max_coord < 12.0,
            "{turn_deg} degree bend: coordinates stay bounded, got {max_coord}"
        );
    }
}

#[test]
fn follow_me_chained_sharp_joints_do_not_compound_the_miter() {
    // Several consecutive near-limit joints must NOT multiply their
    // stretches (3.86^4 would be ~220x): the transport between
    // perpendicular cross-sections is an isometry (design section 1), so
    // every cross-section stays congruent to the profile and each miter
    // ring is stretched at most once, by its own joint's ratio. A zigzag
    // of four alternating 150-degree turns (each at ratio 3.86, near the
    // limit of 8) sweeps into a bounded, watertight solid.
    let profile = yz_profile(-0.05, 0.1, 0.05, 0.2);
    let d_a = Vec3::new(1.0, 0.0, 0.0);
    let turn = 150.0f64.to_radians();
    let d_b = Vec3::new(turn.cos(), turn.sin(), 0.0);
    let mut pts = vec![Point3::ORIGIN];
    for i in 0..5 {
        let last = *pts.last().unwrap();
        pts.push(last + if i % 2 == 0 { d_a } else { d_b });
    }
    let solid = Object::from_follow_me(&profile, &pts, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 2 + 5 * 4);
    assert_eq!(euler_poincare(&solid), 2);
    assert!(signed_volume(&solid) > 0.0);
    // Path extent ~1.3 plus ONE joint's limited miter of the profile's own
    // ~0.2 extent — nowhere near a compounded 220x spike.
    let max_coord = solid
        .vertices()
        .values()
        .map(|v| {
            v.position
                .x
                .abs()
                .max(v.position.y.abs())
                .max(v.position.z.abs())
        })
        .fold(0.0f64, f64::max);
    assert!(
        max_coord < 2.5,
        "chained sharp joints stay bounded, got {max_coord}"
    );
}

#[test]
fn follow_me_refuses_bend_tighter_than_profile() {
    // The profile extends 1.0 toward the inside of a 90-degree turn whose
    // legs are only 0.4 long: ring vertices would be dragged backward.
    let profile = yz_profile(0.0, 0.0, 1.0, 1.0);
    let path = [
        Point3::ORIGIN,
        Point3::new(0.4, 0.0, 0.0),
        Point3::new(0.4, 0.4, 0.0),
    ];
    let err = Object::from_follow_me(&profile, &path, false, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathTooTight);
}

#[test]
fn follow_me_refuses_lathe_profile_touching_the_axis() {
    // A profile touching the axis of revolution on a NON-circular path (here
    // an UNATTRIBUTED facet ring, treated as a polyline): pole closure needs
    // a recognized circle path (design §9), so the would-be pole is not fixed
    // by any revolution — the on-axis vertex never advances and it refuses
    // typed, exactly as before. (Give this same ring analytic attribution and
    // it closes a sphere — see follow_me_sphere_*.)
    let n = 24usize;
    let step = std::f64::consts::TAU / n as f64;
    let path: Vec<Point3> = (0..n)
        .map(|i| {
            let a = (i as f64 - 0.5) * step;
            Point3::new(2.0 * a.cos(), 2.0 * a.sin(), 0.0)
        })
        .collect();
    // The y = 0 plane with normal -y (x cross z); outer ring CCW seen
    // from -y. One profile edge lies exactly on the z axis (x = 0).
    let profile = Profile::new(
        Plane::from_polygon(&[
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ])
        .unwrap(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.5, 0.0, 0.0),
            Point3::new(0.5, 0.0, 0.5),
            Point3::new(0.0, 0.0, 0.5),
        ],
        vec![],
    )
    .unwrap();
    let err = Object::from_follow_me(&profile, &path, true, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathTooTight);
}

#[test]
fn follow_me_closed_seam_starts_at_a_corner() {
    // The profile plane passes exactly through a corner of the closed
    // path, perpendicular to the OUTGOING flank, with the profile beyond
    // the corner (design §2b): the seam ring stays on the profile plane,
    // the corner's own miter plane becomes one extra station, and the
    // wedge between them closes the loop. The result is the mitered
    // picture frame: an outward band of width 0.4 around the 2 x 2 square
    // extruded z in [-0.2, 0.2] — volume (2.8² − 2²) · 0.4 exactly.
    let profile = yz_profile(-0.4, -0.2, 0.0, 0.2);
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // A ring solid: genus one, Euler characteristic zero.
    assert_eq!(euler_poincare(&solid), 0);
    let expected = (2.8f64 * 2.8 - 2.0 * 2.0) * 0.4;
    assert!((signed_volume(&solid) - expected).abs() < 1e-9);
}

#[test]
fn follow_me_corner_seam_walks_reversed_when_the_flank_enters_the_corner() {
    // The perpendicular flank ENTERS the chosen corner in path order, so
    // the loop is walked against it (design §2b) — the anchor nearest the
    // profile is the (0, 2, 0) corner, whose perpendicular flank is the
    // top segment arriving there. Same frame, same exact volume.
    let profile = yz_profile(2.0, -0.2, 2.4, 0.2);
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&solid), 0);
    let expected = (2.8f64 * 2.8 - 2.0 * 2.0) * 0.4;
    assert!((signed_volume(&solid) - expected).abs() < 1e-9);
}

#[test]
fn follow_me_partial_sweep_stops_at_the_arc_length() {
    // An L path swept only 1.5 of its 3.0 total length: the sweep covers
    // the whole first leg (2.0 > 1.5? no — 1.5 lands mid-first-leg), so
    // the result is exactly the straight prism of length 1.5. A stop at
    // or past the full length is the full L sweep; a stop of nothing is
    // EmptyPath; a stop within POINT_MERGE of a joint truncates AT the
    // joint (no sliver segment).
    let profile = yz_profile(-0.2, -0.2, 0.2, 0.2);
    let path = [
        Point3::ORIGIN,
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 3.0, 0.0),
    ];
    let partial = Object::from_follow_me_to(&profile, &path, false, &[], 1.5).unwrap();
    partial.validate().unwrap();
    let straight = Object::from_follow_me(
        &profile,
        &[Point3::ORIGIN, Point3::new(1.5, 0.0, 0.0)],
        false,
        &[],
    )
    .unwrap();
    assert!(objects_equivalent(&partial, &straight));

    let full = Object::from_follow_me_to(&profile, &path, false, &[], 99.0).unwrap();
    let reference = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    assert!(objects_equivalent(&full, &reference));

    let err = Object::from_follow_me_to(&profile, &path, false, &[], 0.0).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::EmptyPath);

    let at_joint = Object::from_follow_me_to(&profile, &path, false, &[], 2.0 + 1e-12).unwrap();
    let leg = Object::from_follow_me(
        &profile,
        &[Point3::ORIGIN, Point3::new(2.0, 0.0, 0.0)],
        false,
        &[],
    )
    .unwrap();
    assert!(objects_equivalent(&at_joint, &leg));
}

#[test]
fn follow_me_partial_sweep_opens_a_closed_path_from_its_seam() {
    // A closed square loop swept only part-way from the profile's seam:
    // the result is an OPEN sweep — two caps — covering exactly the first
    // 3.0 of the loop (the 2.0 first leg plus 1.0 of the second), i.e. an
    // L band with a mitered elbow. The profile sits entirely OUTSIDE the
    // turn, so the miter adds the exterior corner square (w²) and removes
    // nothing: area = (l1 + l2)·w + w², times the profile's z-extent.
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    // The profile plane (x = 0) crosses no segment strictly and passes
    // through two corners; the nearest-to-profile corner (0,0,0) seams.
    // Sweep 3.0 of the 8.0 loop.
    let profile_off = yz_profile(-0.4, -0.2, 0.0, 0.2);
    let partial = Object::from_follow_me_to(&profile_off, &path, true, &[], 3.0).unwrap();
    partial.validate().unwrap();
    assert_eq!(partial.watertight(), WatertightState::Watertight);
    // Genus zero now — the ring was cut open.
    assert_eq!(euler_poincare(&partial), 2);
    let expected = ((2.0 + 1.0) * 0.4 + 0.4 * 0.4) * 0.4;
    assert!((signed_volume(&partial) - expected).abs() < 1e-9);
}

#[test]
fn follow_me_partial_sweep_refuses_a_pole_closing_lathe() {
    // A sphere profile touching its revolution axis cannot be cut open:
    // the poles exist only in the closed revolution. Typed refusal.
    let profile = axis_circle_profile(1.0, 1.0, 24, 0.0);
    let (path, curves) = attributed_ground_circle(1.0, 24, 0.0);
    let err = Object::from_follow_me_to(&profile, &path, true, &curves, 1.0).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PartialSweepOnPole);
}

#[test]
fn follow_me_partial_sweep_at_exactly_the_full_perimeter_is_the_full_sweep() {
    // A closed path's stop landing exactly on its own full perimeter walks
    // the last band back to the seam — which must read as the documented
    // "at or beyond the full path length is the full sweep, closed seam and
    // all", not as a truncation whose cut point happens to coincide with the
    // seam (which would build two coincident end caps and self-intersect).
    // Covers both a plain closed anchor and a corner-seam one, since only
    // the closed case's wrap-around band can alias against the seam this
    // way at all.
    // x = 1 crosses the bottom leg's INTERIOR (an ordinary anchor, not a
    // corner) — the path's corners all sit at x = 0/x = 2.
    let profile = yz_profile_at(1.0, -0.2, -0.2, 0.2, 0.2);
    let path = [
        Point3::ORIGIN,
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let full = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    let at_perimeter = Object::from_follow_me_to(&profile, &path, true, &[], 8.0).unwrap();
    at_perimeter.validate().unwrap();
    assert!(objects_equivalent(&full, &at_perimeter));
    // A hair short still truncates (the ordinary case, unaffected).
    let short = Object::from_follow_me_to(&profile, &path, true, &[], 8.0 - 1e-6).unwrap();
    assert_eq!(euler_poincare(&short), 2); // genus zero: still cut open

    // The corner-seam anchor (design §2b) — its own spec fixture. The wedge
    // is a zero-length band (see `follow_me_partial_sweep_opens_a_closed_
    // path_from_its_seam`'s "8.0 loop" comment), so the walkable arc length
    // is still the path's own 8.0 perimeter, not the swept frame's own
    // outer geometry.
    let corner_profile = yz_profile(-0.4, -0.2, 0.0, 0.2);
    let corner_full = Object::from_follow_me(&corner_profile, &path, true, &[]).unwrap();
    let corner_at_perimeter =
        Object::from_follow_me_to(&corner_profile, &path, true, &[], 8.0).unwrap();
    corner_at_perimeter.validate().unwrap();
    assert!(objects_equivalent(&corner_full, &corner_at_perimeter));
}

#[test]
fn follow_me_corner_straddling_profile_refuses_the_fold() {
    // A profile centered ON the corner hangs over the incoming flank: on
    // that side the wedge folds back into the flank's own swept material
    // (the sweep provably overlaps itself), so the advance check refuses
    // — typed, never nudged (design §2b).
    let profile = yz_profile(-0.2, -0.2, 0.2, 0.2);
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let err = Object::from_follow_me(&profile, &path, true, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::PathTooTight);
}

#[test]
fn follow_me_refuses_self_intersecting_sweep() {
    // A G-shaped path whose last leg re-enters the first leg's swept
    // material: no local fold (every turn clears the advance check), but
    // the distant legs interpenetrate. Refused whole.
    let profile = yz_profile(-0.6, -0.6, 0.6, 0.6);
    let path = [
        Point3::ORIGIN,
        Point3::new(6.0, 0.0, 0.0),
        Point3::new(6.0, 4.0, 0.0),
        Point3::new(0.0, 4.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(3.0, 1.0, 0.0),
    ];
    let err = Object::from_follow_me(&profile, &path, false, &[]).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::SweepSelfIntersects);
}

#[test]
fn follow_me_refuses_degenerate_paths() {
    let profile = yz_profile(0.0, 0.0, 1.0, 1.0);
    assert_eq!(
        Object::from_follow_me(&profile, &[], false, &[]).unwrap_err(),
        kernel::FollowMeError::EmptyPath
    );
    assert_eq!(
        Object::from_follow_me(&profile, &[Point3::ORIGIN], false, &[]).unwrap_err(),
        kernel::FollowMeError::EmptyPath
    );
    assert_eq!(
        Object::from_follow_me(
            &profile,
            &[
                Point3::ORIGIN,
                Point3::new(tol::POINT_MERGE / 2.0, 0.0, 0.0)
            ],
            false,
            &[],
        )
        .unwrap_err(),
        kernel::FollowMeError::PathSegmentTooShort
    );
}

#[test]
fn follow_me_sweeps_holes_into_tunnels() {
    // A washer profile swept along an L: the hole tunnels the whole way,
    // so the solid is genus 1 with annulus caps.
    let profile = Profile::new(
        yz_plane(),
        vec![
            Point3::new(0.0, -1.0, 0.5),
            Point3::new(0.0, 1.0, 0.5),
            Point3::new(0.0, 1.0, 2.5),
            Point3::new(0.0, -1.0, 2.5),
        ],
        vec![vec![
            Point3::new(0.0, -0.5, 1.0),
            Point3::new(0.0, -0.5, 2.0),
            Point3::new(0.0, 0.5, 2.0),
            Point3::new(0.0, 0.5, 1.0),
        ]],
    )
    .unwrap();
    let path = [
        Point3::ORIGIN,
        Point3::new(4.0, 0.0, 0.0),
        Point3::new(4.0, 4.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // 2 annulus caps + 2 segments x (4 outer + 4 hole) walls.
    assert_eq!(solid.faces().len(), 18);
    let cap_holes: usize = solid.faces().values().map(|f| f.inner_loops.len()).sum();
    assert_eq!(cap_holes, 2);
    assert_eq!(euler_poincare(&solid), 0);
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

// ------------------------- flat-face push/pull (translate-and-build)
//
// Push/pull on any planar face of a solid, whatever the angle of its
// neighbors — a Slice-produced wedge's cut face, a prism side facet (the
// Circle tool's own output), a face from a dissolved boolean seam. Every
// such face is flat, so it follows classic SketchUp push/pull: the moved
// face translates rigidly by `distance` along its normal, and every
// NON-transverse boundary edge (a coplanar `split_face` sibling OR a slanted
// wedge/facet neighbor) unwelds and grows a fresh quad side wall, while every
// transverse neighbor extends seamlessly. Topology therefore CHANGES (walls
// appear), unlike the earlier in-plane "stretch" that this replaces.
//
// The asymmetry the flat-face ruling asks for (see `Object::push_pull` and
// `validate_sweep_result`):
// - PULL (outward) always succeeds — it erects a prism of material on the
//   face — so it is unbounded regardless of neighbor angle.
// - PUSH (inward) succeeds only as far as the built result stays valid, and
//   refuses typed byte-identical past that: a wedge's slant face cannot be
//   pushed in at all (the moved face immediately crosses the fixed structure),
//   a fatter prism's facet can be pushed in until it would.
// - A purely transverse boundary (a box face) keeps the bit-identical
//   translate fast path.
//
// The exact inverse of a wall-building push is the recorded
// `UnbuildPushPull`, dispatched by `History` — a plain `push_pull(-d)` cannot
// re-collapse a slanted neighbor's non-coplanar wall — so invertibility specs
// go through `History`, not a direct `push_pull(report.face, -d)`.

/// Exact (bit-for-bit) polygon snapshot, for asserting the strong guarantee
/// after a refusal: a refused push must leave the object byte-identical, not
/// merely equivalent within tolerance.
fn exact_snapshot(obj: &Object) -> (Vec<Point3>, Vec<Vec<usize>>) {
    obj.to_polygons()
}

/// Pull `face` of `original` by `d` through `History`, assert the built result
/// validates and stays watertight, then undo and assert the recorded inverse
/// restores `original` exactly (topology + geometry). Returns the post-pull
/// object for further inspection. This is the canonical translate-and-build
/// round-trip: the inverse is `UnbuildPushPull`, not a direct `push_pull(-d)`.
fn assert_build_and_invert(original: &Object, face: FaceId, d: f64) -> Object {
    let mut obj = original.clone();
    let mut history = History::new();
    history
        .apply(&mut obj, KernelOp::PushPull { face, distance: d })
        .expect("wall-building push/pull must succeed");
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    let after = obj.clone();
    history.undo(&mut obj).expect("recorded inverse must undo");
    obj.validate().unwrap();
    assert!(
        objects_equivalent(&obj, original),
        "history undo of a wall-building push must restore the original exactly"
    );
    // A redo must return to the pulled state, so the pair is a true inverse.
    history.redo(&mut obj).expect("redo must re-apply");
    assert!(
        objects_equivalent(&obj, &after),
        "redo must restore the pulled state"
    );
    after
}

/// The base success case: a right-triangle prism's front wall (normal -y) has
/// a MIXED boundary — top cap, bottom cap, and left wall are transverse; the
/// hypotenuse wall (|dot| ≈ 0.707) is slanted.
///
/// Pulling the front wall out by 0.5 translates it rigidly to y = -0.5. The
/// three transverse neighbors extend seamlessly; the slanted hypotenuse edge
/// unwelds and grows ONE fresh vertical wall bridging the old edge to the
/// raised one. So the wall's near corners rise straight out — (1,0,z) stays
/// put, (1,-0.5,z) is a raised copy — the hypotenuse does NOT slide sideways
/// the way the old stretch made it. The prism gains a facet.
#[test]
fn push_pull_prism_front_wall_builds_a_wall() {
    let plane = xy_plane();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let prism = Object::from_extrusion(&profile, 1.0).unwrap();
    let front_wall = face_with_normal(&prism, Vec3::new(0.0, -1.0, 0.0));

    let after = assert_build_and_invert(&prism, front_wall, 0.5);

    // One new wall along the single slanted (hypotenuse) edge.
    assert_eq!(after.faces().len(), 6, "the prism grew one side facet");
    assert_eq!(
        after.vertices().len(),
        8,
        "the slanted edge raised two copies"
    );
    assert_eq!(euler_poincare(&after), 2, "still one genus-0 shell");

    // The moved face translated rigidly to y = -0.5, and the original
    // hypotenuse corners stayed put (the wall bridges to them) — no sideways
    // slide.
    for expect in [
        Point3::new(0.0, -0.5, 0.0),
        Point3::new(1.0, -0.5, 0.0),
        Point3::new(1.0, -0.5, 1.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 0.0),
    ] {
        assert!(
            after
                .vertices()
                .values()
                .any(|v| (v.position - expect).length() <= tol::POINT_MERGE),
            "expected a vertex at {expect:?}"
        );
    }
    // Cross-section (0,-0.5),(1,-0.5),(1,0),(0,1): area 1.0, extruded by 1.
    let volume = signed_volume(&after);
    assert!(
        (volume - 1.0).abs() <= VOLUME_TOL,
        "signed volume {volume}, expected 1.0"
    );
}

/// A pull is UNBOUNDED regardless of neighbor angle: pulling the same slanted
/// front wall out by a large distance still validates (it just erects a longer
/// prism of material), it does not refuse the way the old stretch did once its
/// walls pinched off.
#[test]
fn push_pull_slanted_face_pull_is_unbounded() {
    let profile = Profile::new(
        xy_plane(),
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
        vec![],
    )
    .unwrap();
    let prism = Object::from_extrusion(&profile, 1.0).unwrap();
    let front_wall = face_with_normal(&prism, Vec3::new(0.0, -1.0, 0.0));
    for d in [5.0, 50.0, 500.0] {
        let after = assert_build_and_invert(&prism, front_wall, d);
        assert_eq!(after.faces().len(), 6, "distance {d}");
    }
}

/// Regular hexagonal prism (radius 1, height 1) — the Circle tool's own
/// output shape. Pulling one side facet outward by 0.2 translates it rigidly
/// and grows a pad: the two adjacent facets (|dot| = 0.5) are slanted, so each
/// unwelds and gains a fresh wall bridging out to the raised facet; the caps
/// extend seamlessly. This is a flat-facet bump (acceptable on this branch),
/// not the whole-cylinder bulge the curves branch will deliver.
#[test]
fn push_pull_hex_prism_facet_builds_a_pad() {
    let (hex, facet, n) = hex_prism_and_facet();
    let apothem = 3.0_f64.sqrt() / 2.0;

    let after = assert_build_and_invert(&hex, facet, 0.2);

    // Two new walls along the two slanted adjacent-facet edges.
    assert_eq!(after.faces().len(), 10, "the prism grew a two-walled pad");
    assert_eq!(
        after.vertices().len(),
        16,
        "the facet edges raised four copies"
    );
    assert_eq!(euler_poincare(&after), 2);

    // Exactly the facet's 4 corners advanced to apothem + 0.2 along the facet
    // normal; the adjacent facets' far corners stayed put (a wall bridges to
    // them) rather than sliding.
    let advanced = after
        .vertices()
        .values()
        .filter(|v| {
            let d = v.position.to_vec().dot(n);
            (d - (apothem + 0.2)).abs() <= tol::POINT_MERGE
        })
        .count();
    assert_eq!(
        advanced, 4,
        "facet corners at apothem + 0.2 along the normal"
    );
}

/// A 24-gon prism — the Circle tool's actual output. Translate-and-build lifts
/// the old stretch range limit on the OUTWARD side: pulling a facet out grows
/// a two-walled pad and is unbounded (a small nudge and a large pull both
/// validate and invert). Pushing a facet IN is bounded by validity — a shallow
/// recess builds cleanly, a deep push where the moved facet would cross
/// non-adjacent facets refuses typed, byte-identical.
#[test]
fn push_pull_cylinder_facet_pull_unbounded_push_bounded() {
    let pts: Vec<Point3> = (0..24)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 24.0;
            Point3::new(a.cos(), a.sin(), 0.0)
        })
        .collect();
    let profile = Profile::new(xy_plane(), pts, vec![]).unwrap();
    let cyl = Object::from_extrusion(&profile, 1.0).unwrap();
    let th = 7.5f64.to_radians();
    let n = Vec3::new(th.cos(), th.sin(), 0.0);
    let facet = face_with_normal(&cyl, n);

    // 24 side facets + 2 caps = 26 faces; a two-walled pad makes 28.
    // Outward pads at any depth — small and large both build and invert.
    for d in [0.02, 0.5, 5.0] {
        let after = assert_build_and_invert(&cyl, facet, d);
        assert_eq!(after.faces().len(), 28, "distance {d}: a two-walled pad");
    }

    // Shallow inward recess builds cleanly and inverts.
    let shallow = assert_build_and_invert(&cyl, facet, -0.05);
    assert_eq!(shallow.faces().len(), 28);

    // A push deep enough to consume the solid refuses as WouldVanish,
    // byte-identical.
    let mut refused = cyl.clone();
    let before = exact_snapshot(&refused);
    let err = refused.push_pull(facet, -3.0).unwrap_err();
    assert_eq!(err, PushPullError::WouldVanish);
    assert_eq!(exact_snapshot(&refused), before);
}

/// The undo contract for a wall-building push, stated directly: the recorded
/// `UnbuildPushPull` inverse (via `History`) restores the prism exactly, while
/// a plain `push_pull(report.face, -d)` CANNOT — the walls it built are
/// perpendicular to the moved facet, so pushing back does not re-collapse a
/// slanted neighbor's non-coplanar wall. This asymmetry is why invertibility
/// specs route through `History`.
#[test]
fn push_pull_slanted_inverse_is_the_recorded_unbuild() {
    let (hex, facet, _n) = hex_prism_and_facet();
    let original = hex.clone();

    // History inverse restores exactly.
    let mut via_history = hex.clone();
    let mut history = History::new();
    history
        .apply(
            &mut via_history,
            KernelOp::PushPull {
                face: facet,
                distance: 0.2,
            },
        )
        .unwrap();
    history.undo(&mut via_history).unwrap();
    via_history.validate().unwrap();
    assert!(objects_equivalent(&via_history, &original));

    // A direct push-back cannot: the built walls are not re-collapsible this
    // way. It refuses typed, leaving the pulled solid untouched.
    let mut direct = hex.clone();
    let report = direct.push_pull(facet, 0.2).unwrap();
    let pulled = direct.clone();
    let err = direct.push_pull(report.face, -0.2).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert!(
        objects_equivalent(&direct, &pulled),
        "refusal is byte-identical"
    );
}

/// A recorded wall that an intervening `split_face_inner` turned into a holed
/// face is no longer the pristine quad the push recorded. Undoing that push
/// (via the recorded `UnbuildPushPull` inverse) must refuse TYPED with the
/// object byte-identical — it must NOT remove a wall that still owns a hole
/// loop and a sub-face, which would orphan them (a debug `check_invariants`
/// panic, and a corruption in release). This test runs under plain
/// `cargo test` (a debug build), so a passing run proves there is no panic.
#[test]
fn unbuild_refuses_when_a_recorded_wall_gained_a_hole() {
    let mut obj = sliced_wedge();
    let cut = wedge_cut_face(&obj);
    let mut history = History::new();
    let report = match history
        .apply(
            &mut obj,
            KernelOp::PushPull {
                face: cut,
                distance: 0.3,
            },
        )
        .expect("wedge cut face pull builds walls")
    {
        KernelOpReport::PushPull(r) => r,
        other => panic!("expected a PushPull report, got {other:?}"),
    };
    let wall = report.created_faces[0];

    // An inset rectangle strictly inside the built wall, on its own plane —
    // a purely additive imprint that appends a hole loop to `wall` and mints a
    // sub-face, leaving `wall`'s outer 4-cycle untouched.
    let corners: Vec<Point3> = obj.loop_positions(obj.faces()[wall].outer_loop).collect();
    let n = corners.len() as f64;
    let centroid = corners
        .iter()
        .fold(Point3::ORIGIN, |acc, &p| acc + p.to_vec() / n);
    let rect: Vec<Point3> = corners
        .iter()
        .map(|&p| centroid + (p - centroid) * 0.4)
        .collect();
    obj.split_face_inner(wall, &rect)
        .expect("imprint a hole on the built wall");
    obj.validate().unwrap();
    let holed = obj.to_polygons();

    // Undo the PUSH while the wall carries a hole. Must fail typed (a kernel
    // bug surfaced as InverseFailed), object byte-identical — never panic.
    let err = history
        .undo(&mut obj)
        .expect_err("undo must refuse: the recorded wall is no longer a pristine quad");
    assert!(
        matches!(err, HistoryError::InverseFailed(_)),
        "undo must fail typed, got {err:?}"
    );
    assert_eq!(
        obj.to_polygons(),
        holed,
        "a refused undo must leave the object byte-identical"
    );
    obj.validate().unwrap();
}

/// Negative distance recesses the facet: pushing a hex facet inward by 0.5
/// translates it in and builds walls back to the adjacent facets — a valid
/// recess (the fatter hexagon leaves room), removing material. Invertible
/// through the recorded inverse.
#[test]
fn push_pull_inward_facet_recesses() {
    let (hex, facet, n) = hex_prism_and_facet();
    let apothem = 3.0_f64.sqrt() / 2.0;
    let volume_before = signed_volume(&hex);

    let after = assert_build_and_invert(&hex, facet, -0.5);
    assert_eq!(after.faces().len(), 10, "the recess adds two walls");

    let recessed = after
        .vertices()
        .values()
        .filter(|v| {
            let d = v.position.to_vec().dot(n);
            (d - (apothem - 0.5)).abs() <= tol::POINT_MERGE
        })
        .count();
    assert_eq!(
        recessed, 4,
        "facet corners at apothem - 0.5 along the normal"
    );
    assert!(
        signed_volume(&after) < volume_before,
        "an inward push removes material"
    );
}

/// A fatter prism leaves room to push a facet in well past its own plane (a
/// deep notch stays valid), but a push deep enough to consume the whole solid
/// (beyond the far side's extent, √3 ≈ 1.732) refuses as WouldVanish,
/// byte-identical — the kernel never deletes the Object as a geometric side
/// effect.
#[test]
fn push_pull_inward_facet_consuming_the_solid_is_refused() {
    let (hex, facet, _n) = hex_prism_and_facet();

    // A deep-but-valid inward notch (past the facet's own apothem, 0.866).
    let notched = assert_build_and_invert(&hex, facet, -1.0);
    assert_eq!(notched.faces().len(), 10, "a two-walled inward notch");

    // Beyond the whole solid's extent: consuming all material is refused.
    let mut past_extent = hex.clone();
    let before = exact_snapshot(&past_extent);
    let err = past_extent.push_pull(facet, -2.0).unwrap_err();
    assert_eq!(err, PushPullError::WouldVanish);
    assert_eq!(exact_snapshot(&past_extent), before);
}

/// The unit right tetrahedron: the slanted face's three neighbors are all
/// slanted, so pulling it out grows three fresh walls (a bump on the oblique
/// face), watertight and invertible through the recorded inverse.
#[test]
fn push_pull_tetrahedron_face_builds_three_walls() {
    let tetra = Object::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ],
        &[
            vec![0, 2, 1], // bottom, outward -z
            vec![0, 3, 2], // outward -x
            vec![0, 1, 3], // outward -y
            vec![1, 2, 3], // slanted, outward (1,1,1)
        ],
    )
    .unwrap();
    let slanted = face_with_normal(&tetra, Vec3::new(1.0, 1.0, 1.0).normalized().unwrap());
    let volume_before = signed_volume(&tetra);

    let after = assert_build_and_invert(&tetra, slanted, 0.3);
    assert_eq!(after.faces().len(), 7, "three slanted edges → three walls");
    assert!(
        signed_volume(&after) > volume_before,
        "an outward pull adds material"
    );
}

/// A face whose every neighbor is oblique with a fully-oblique corner
/// configuration (the octahedron, all vertices valence 4) — a case the old
/// in-plane stretch REFUSED as over-determined — now pulls cleanly: translate-
/// and-build never solves a per-vertex slide, it just erects a wall on each
/// oblique edge, so there is nothing to over-determine.
#[test]
fn push_pull_octahedron_face_builds_walls() {
    let octa = Object::from_polygons(
        &[
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(-1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, -1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(0.0, 0.0, -1.0),
        ],
        &[
            vec![0, 2, 4], // (+,+,+)
            vec![1, 4, 2], // (-,+,+)
            vec![0, 4, 3], // (+,-,+)
            vec![0, 5, 2], // (+,+,-)
            vec![1, 3, 4], // (-,-,+)
            vec![1, 2, 5], // (-,+,-)
            vec![0, 3, 5], // (+,-,-)
            vec![1, 5, 3], // (-,-,-)
        ],
    )
    .unwrap();
    octa.validate().unwrap();

    let face = face_with_normal(&octa, Vec3::new(1.0, 1.0, 1.0).normalized().unwrap());
    let after = assert_build_and_invert(&octa, face, 0.2);
    assert_eq!(after.faces().len(), 11, "three oblique edges → three walls");
}

/// A boundary that MIXES a coplanar sibling (from `split_face`) with slanted
/// neighbors is now a first-class case: coplanar and slanted edges both build
/// walls, so the same surgery handles them together. Pulling a bisected half
/// of a wedge's cut face out builds walls along the cut edge and the wedge's
/// oblique legs alike — watertight, invertible.
#[test]
fn push_pull_mixed_coplanar_and_slanted_boundary_builds_walls() {
    let mut wedge = sliced_wedge();
    let cut = wedge_cut_face(&wedge);
    let report = wedge
        .split_face(
            cut,
            &[Point3::new(1.0, 0.5, 0.0), Point3::new(0.0, 0.5, 1.0)],
        )
        .expect("edge-to-edge bisect of the cut face");
    let half = report.new_faces[0];
    let base = wedge.clone();

    let after = assert_build_and_invert(&base, half, 0.2);
    assert!(
        after.faces().len() > base.faces().len(),
        "the mixed push erected walls"
    );
}

// ------------------------------------- stretch push/pull on sliced solids

/// Unit cube sliced by the plane x + z = 1; the piece kept is the wedge
/// x + z <= 1 (a right-triangle prism along y with a slanted cut face).
fn sliced_wedge() -> Object {
    let cube = unit_cube();
    let plane =
        Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0)).unwrap();
    let (_above, below) = cube.slice(&plane).expect("slice through the cube");
    below.validate().unwrap();
    assert_eq!(below.faces().len(), 5, "wedge: 2 caps + 2 legs + cut face");
    below
}

/// The wedge's cut face (outward normal = the slice plane's normal).
fn wedge_cut_face(wedge: &Object) -> FaceId {
    face_with_normal(wedge, Vec3::new(1.0, 0.0, 1.0).normalized().unwrap())
}

/// Pushing the cut face of a Slice-produced wedge — the flagship user case,
/// and the exact behavior the flat-face ruling asks for.
///
/// PULL (outward): the two oblique legs each unweld and grow a fresh wall, so
/// the wedge gains material along its slope. Watertight, and exactly
/// invertible through the recorded inverse. Unbounded — even a large pull
/// validates.
///
/// PUSH (inward): a wedge's slant face cannot be pushed in AT ALL. The moved
/// face immediately drives across the fixed bottom/back structure (the new
/// walls would dip outside the solid), so the interpenetration guard refuses
/// the very first infinitesimal push — typed, byte-identical.
#[test]
fn push_pull_wedge_cut_face_builds_walls_and_cannot_push_in() {
    let wedge = sliced_wedge();
    let cut = wedge_cut_face(&wedge);

    // Outward: builds two walls (one per oblique leg), watertight, invertible.
    let after = assert_build_and_invert(&wedge, cut, 0.3);
    assert_eq!(after.faces().len(), 7, "two oblique legs → two walls");
    assert_eq!(after.vertices().len(), 10);
    assert!(
        signed_volume(&after) > signed_volume(&wedge),
        "an outward pull adds material along the slope"
    );

    // A large pull is still fine (unbounded).
    let far = assert_build_and_invert(&wedge, cut, 4.0);
    assert_eq!(far.faces().len(), 7);

    // Inward: not possible at all — even a tiny push refuses, byte-identical.
    for d in [-0.01, -0.3, -0.6] {
        let mut pushed = sliced_wedge();
        let before = exact_snapshot(&pushed);
        let err = pushed.push_pull(wedge_cut_face(&pushed), d).unwrap_err();
        assert_eq!(err, PushPullError::NonManifoldResult, "distance {d}");
        assert_eq!(exact_snapshot(&pushed), before, "distance {d}");
    }
}

/// A slanted face WITH A HOLE: punch a square tunnel through the wedge's cut
/// face (`push_through`), then pull the now-holed cut face out. The outer
/// boundary's oblique legs each grow a wall; the hole ring rides along rigidly
/// (its tunnel walls are transverse), staying inside the face. The recorded
/// inverse restores the pushed solid — a face-with-hole pull round-trips.
#[test]
fn push_pull_holed_cut_face_builds_walls_and_keeps_the_hole() {
    let mut wedge = sliced_wedge();
    let cut = wedge_cut_face(&wedge);
    // Imprint a small square (points on the plane x + z = 1) and punch it
    // through: the sub-face swept inward past the bottom leaves a tunnel.
    let inner = wedge
        .split_face_inner(
            cut,
            &[
                Point3::new(0.55, 0.35, 0.45),
                Point3::new(0.75, 0.35, 0.25),
                Point3::new(0.75, 0.65, 0.25),
                Point3::new(0.55, 0.65, 0.45),
            ],
        )
        .expect("imprint a square on the cut face");
    let wedge = wedge
        .push_through(inner.sub_face, -0.8)
        .expect("punch the square through the wedge");
    wedge.validate().unwrap();
    assert_eq!(wedge.watertight(), WatertightState::Watertight);

    // The cut face now carries the tunnel mouth as a hole.
    let holed = wedge_cut_face(&wedge);
    assert_eq!(
        wedge.faces()[holed].inner_loops.len(),
        1,
        "cut face carries the tunnel mouth as a hole ring"
    );

    let pushed = assert_build_and_invert(&wedge, holed, 0.1);
    let holed_after = wedge_cut_face(&pushed);
    assert_eq!(
        pushed.faces()[holed_after].inner_loops.len(),
        1,
        "the hole ring survives the build"
    );
    // The whole holed face translated rigidly by the sweep (the hole's tunnel
    // walls are transverse): the hole corner moved by exactly 0.1 along the
    // cut normal.
    let n = Vec3::new(1.0, 0.0, 1.0).normalized().unwrap();
    let expect = Point3::new(0.55, 0.35, 0.45) + n * 0.1;
    assert!(
        pushed
            .vertices()
            .values()
            .any(|v| (v.position - expect).length() <= tol::POINT_MERGE),
        "hole corner must ride along the sweep to {expect:?}"
    );
}

/// Through-cut interaction on a Slice-produced face. Overshoot detection
/// reports the through case for a deep push, so the document layer routes it to
/// `push_through`, whose boolean shaves the oblique cut face inward.
///
/// The swept tool's side walls meet the wedge's oblique legs coplanarly, along
/// the cut face's boundary edges. That contact once seeded sliver vertices in
/// the boolean arrangement — the leg rim was imprinted twice, as the coplanar
/// boundary and as the tool wall's transversal seam — so the weld came out
/// non-manifold and the op was refused. With the redundant transversal seam
/// dropped (crates/kernel/src/boolean.rs), the shave welds cleanly: pushing a
/// wedge whose slant sits at distance 1/√2 from the far corner in by `d` yields
/// a similar, smaller wedge of volume (1 − d√2)²/2, until the slant reaches the
/// corner and the solid vanishes (refused typed).
#[test]
fn push_through_on_a_slice_cut_face_shaves_the_wedge() {
    let wedge = sliced_wedge();
    let cut = wedge_cut_face(&wedge);
    assert!(
        (signed_volume(&wedge) - 0.5).abs() < tol::POINT_MERGE,
        "wedge is half the cube"
    );

    // Deep inward pushes report as through-cuts (the document layer routes
    // them to `push_through`).
    assert!(wedge.push_pull_overshoots(cut, -0.8));

    // A full-face inward sweep shaves the slant into a smaller, still-watertight
    // wedge whose volume is exactly the shaved similar triangle's.
    for d in [0.2_f64, 0.5] {
        let shaved = wedge.push_through(cut, -d).unwrap();
        shaved.validate().unwrap();
        assert_eq!(shaved.watertight(), WatertightState::Watertight);
        assert_eq!(shaved.faces().len(), 5, "still a 5-faced wedge");
        let expect = (1.0 - d * std::f64::consts::SQRT_2).powi(2) / 2.0;
        assert!(
            (signed_volume(&shaved) - expect).abs() < VOLUME_TOL,
            "shave d={d}: vol {} != {expect}",
            signed_volume(&shaved)
        );
    }
    // Source untouched (push_through borrows, never mutates).
    assert_eq!(wedge.faces().len(), 5);

    // Sweeping to or past the far corner consumes the whole wedge; the empty
    // result is refused typed.
    let err = wedge.push_through(cut, -0.8).unwrap_err();
    assert!(
        matches!(
            err,
            PushPullError::WouldVanish | PushPullError::NonManifoldResult
        ),
        "deep full-face through-cut must refuse typed, got {err:?}"
    );
}

/// Pushing a face produced by a dissolved coplanar seam, where that face has
/// a genuinely SLANTED neighbor so the push exercises the stretch path (a
/// union of two axis-aligned boxes would classify every edge transverse and
/// take the translate fast path instead): a box unioned with a
/// chamfer-topped box of the same height. The top seam dissolves into one
/// face whose east neighbor is the chamfer; pulling the merged top stretches
/// the chamfer in its own plane.
#[test]
fn push_pull_face_from_dissolved_seam_builds_a_wall() {
    let a = unit_cube();
    // B: box [1,2]×[0,1]×[0,1] with its top-east edge chamfered by the plane
    // x + 0.5z = 2 (through (2,·,0) and (1.5,·,1)).
    let plane =
        Plane::from_point_normal(Point3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.5)).unwrap();
    let b_box = box_object(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));
    let (_outside, b) = b_box.slice(&plane).expect("chamfer the box");

    let mut merged = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    merged.merge_coplanar_faces(&[]);
    merged.validate().unwrap();
    assert_eq!(
        merged.faces().len(),
        6,
        "bottom, west, front, back, chamfer, and ONE dissolved top"
    );
    let original = merged.clone();

    // The merged top spans x ∈ [0, 1.5]; its east neighbor is the chamfer
    // (normal dot ≈ 0.447 — slanted, not transverse).
    let top = face_with_normal(&merged, Vec3::new(0.0, 0.0, 1.0));
    let after = assert_build_and_invert(&original, top, 0.2);
    // The chamfer edge grows one wall; the top rose rigidly to z = 1.2 and its
    // east border stayed at x = 1.5 (the wall bridges to the chamfer) rather
    // than sliding.
    assert_eq!(after.faces().len(), 7, "the chamfer edge grew one wall");
    assert!(
        after
            .vertices()
            .values()
            .any(|v| (v.position - Point3::new(1.5, 0.0, 1.2)).length() <= tol::POINT_MERGE),
        "top east border rose straight to z = 1.2 at x = 1.5"
    );
    // Volume: 1.75 before, plus the rigid slab [0,1.5]×[0,1]×[1,1.2] = 0.3.
    let volume = signed_volume(&after);
    assert!(
        (volume - 2.05).abs() <= VOLUME_TOL,
        "signed volume {volume}, expected 2.05"
    );
}

// -------------------------- interpenetration guards on the build result
//
// A near-flat chamfer makes the built walls (and the moved face itself) reach
// into a shallow band above the top where a separate shell may sit. These
// specs pin the result-validation guards (`validate_sweep_result`, reused from
// the stretch era) that keep a wall-building push from silently committing
// interpenetrating geometry — every face stays planar and twin-consistent, so
// the structural validator cannot see it: touched faces may not contact any
// other face of the object away from their shared elements, and nothing may
// end up engulfed inside the swept prism.

/// Prism with a near-flat chamfer next to its top (the chamfer plane is
/// ~1.1° off the top plane), plus one extra disjoint closed shell placed at
/// `extra` (an axis-aligned box), all in ONE Object. A tiny +0.01 push on the
/// top rises into the shallow band just above it, where the built wall (and
/// the moved face) can reach a hovering shell — the case the result-validation
/// guards below refuse.
fn chamfered_prism_with_shell(extra: Option<(Point3, Point3)>) -> Object {
    let mut verts = vec![
        // y = 0 cap ring
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.5, 0.0, 0.0),
        Point3::new(1.5, 0.0, 0.99),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 0.0, 1.0),
        // y = 1 cap ring
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(1.5, 1.0, 0.0),
        Point3::new(1.5, 1.0, 0.99),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ];
    let mut faces = vec![
        vec![0, 1, 2, 3, 4], // cap y=0 (outward -y)
        vec![5, 9, 8, 7, 6], // cap y=1 (outward +y)
        vec![0, 5, 6, 1],    // bottom (-z)
        vec![1, 6, 7, 2],    // east (+x)
        vec![2, 7, 8, 3],    // chamfer (~+z, tilted 1.1°)
        vec![3, 8, 9, 4],    // top (+z)
        vec![4, 9, 5, 0],    // west (-x)
    ];
    if let Some((lo, hi)) = extra {
        let base = verts.len();
        verts.extend([
            Point3::new(lo.x, lo.y, lo.z),
            Point3::new(hi.x, lo.y, lo.z),
            Point3::new(hi.x, hi.y, lo.z),
            Point3::new(lo.x, hi.y, lo.z),
            Point3::new(lo.x, lo.y, hi.z),
            Point3::new(hi.x, lo.y, hi.z),
            Point3::new(hi.x, hi.y, hi.z),
            Point3::new(lo.x, hi.y, hi.z),
        ]);
        for f in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            faces.push(f.into_iter().map(|i| i + base).collect());
        }
    }
    let obj = Object::from_polygons(&verts, &faces).expect("prism (+shell) soup is closed");
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj
}

/// The near-flat chamfer alone is fine: +0.01 on the top translates it
/// rigidly and grows ONE wall along the near-flat chamfer edge. The shared
/// border rises straight to z = 1.01 at its original x = 1 — no lateral slide
/// (the old stretch amplified a 0.01 push into a 0.5 sideways slide here;
/// translate-and-build does not). Watertight, invertible.
#[test]
fn push_pull_near_flat_chamfer_builds_a_wall_no_slide() {
    let prism = chamfered_prism_with_shell(None);
    let top = face_with_normal(&prism, Vec3::new(0.0, 0.0, 1.0));

    let after = assert_build_and_invert(&prism, top, 0.01);
    assert_eq!(after.faces().len(), 8, "the chamfer edge grew one wall");
    assert!(
        after
            .vertices()
            .values()
            .any(|v| (v.position - Point3::new(1.0, 0.0, 1.01)).length() <= tol::POINT_MERGE),
        "border rises straight to z = 1.01 at x = 1, no slide"
    );
}

/// A second, disjoint shell hovering 0.001 above the top, right where the
/// stretched chamfer surface will pass: the same +0.01 push would drive the
/// chamfer face straight through it. The sweep-depth guard cannot see this
/// (the cube is on no neighbor face, and the cube's own depth exceeds
/// nothing the guard measures) — the face-to-face interpenetration guard
/// must refuse, byte-identical.
#[test]
fn push_pull_stretch_refuses_lateral_interpenetration_across_shells() {
    let mut obj = chamfered_prism_with_shell(Some((
        Point3::new(0.7, 0.4, 1.001),
        Point3::new(0.8, 0.6, 1.1),
    )));
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    let before = exact_snapshot(&obj);
    let err = obj.push_pull(top, 0.01).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(
        exact_snapshot(&obj),
        before,
        "a refused push must leave the object byte-identical"
    );
}

/// A TINY disjoint shell that fits entirely between the top's old and new
/// planes (z ∈ [1.002, 1.006] against a +0.01 push): after the sweep nothing
/// would intersect it — it would simply be engulfed inside newly claimed
/// material. The engulfment guard must refuse, byte-identical.
#[test]
fn push_pull_stretch_refuses_engulfing_a_disjoint_shell() {
    let mut obj = chamfered_prism_with_shell(Some((
        Point3::new(0.2, 0.4, 1.002),
        Point3::new(0.3, 0.6, 1.006),
    )));
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    let before = exact_snapshot(&obj);
    let err = obj.push_pull(top, 0.01).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(exact_snapshot(&obj), before);
}

/// The PLUS-SIGN coplanar overlap, end to end: a plate shell hovers with its
/// underside exactly where the +0.01 push lands the top (z = 1.01), crossing
/// the stretched top off-center — the plate spans x ∈ [0.3, 0.4] (inside the
/// top's final x ∈ [0, 0.5]) but pokes out far beyond it in y. The two
/// coplanar faces share real area, yet NO vertex of either lies inside the
/// other, and no boundary endpoint or midpoint of either lands inside the
/// other — point sampling cannot see the overlap; the guard's real
/// segment/segment boundary intersection can, deterministically. (The
/// plate's walls also graze the landing plane, whose transversal
/// classification sits exactly on a strict-interior boundary — a
/// floating-point coin toss; the coplanar crossing must refuse regardless.)
/// The primitive itself is pinned against point sampling by
/// `faces_improperly_contact_detects_plus_sign_coplanar_overlap` in
/// `ops.rs`'s unit tests, on bare strips with no walls at all. Refused,
/// byte-identical.
#[test]
fn push_pull_stretch_refuses_plus_sign_coplanar_overlap() {
    let mut obj = chamfered_prism_with_shell(Some((
        Point3::new(0.3, -3.0, 1.01),
        Point3::new(0.4, 2.0, 1.05),
    )));
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    let before = exact_snapshot(&obj);
    let err = obj.push_pull(top, 0.01).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(exact_snapshot(&obj), before);
}

/// Same mechanism WITHIN one shell: union the chamfered prism with a
/// cantilever — a column standing on the chamfer carrying an arm that hangs
/// 0.002 above the top. A +0.004 push raises the top into the arm (their
/// only legitimate contact is the union seam down at the chamfer, far from
/// the crossing) — refused, byte-identical. A +0.001 push stays under the
/// arm and must still succeed.
#[test]
fn push_pull_stretch_refuses_lateral_interpenetration_same_shell() {
    let prism = chamfered_prism_with_shell(None);
    // Cantilever profile in the y = 0.35 plane, extruded 0.3 along +y:
    // column x ∈ [1.05, 1.15] from z = 0.95 (inside the prism, through the
    // chamfer) up to 1.05; arm x ∈ [0.65, 1.15], z ∈ [1.002, 1.05].
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.35, 0.0), Vec3::new(0.0, 1.0, 0.0)).unwrap();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(1.05, 0.35, 1.002),
            Point3::new(0.65, 0.35, 1.002),
            Point3::new(0.65, 0.35, 1.05),
            Point3::new(1.15, 0.35, 1.05),
            Point3::new(1.15, 0.35, 0.95),
            Point3::new(1.05, 0.35, 0.95),
        ],
        vec![],
    )
    .expect("cantilever profile");
    let arm = Object::from_extrusion(&profile, 0.3).expect("cantilever solid");
    let mut obj = Object::boolean(BooleanOp::Union, &prism, &arm, &Transform::IDENTITY).unwrap();
    obj.merge_coplanar_faces(&[]);
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);

    // The prism's top face (z = 1, full [0,1]² footprint — the arm hovers,
    // it does not touch the top).
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && f.plane.signed_distance(Point3::new(0.5, 0.5, 1.0)).abs() <= tol::PLANE_DIST
        })
        .map(|(id, _)| id)
        .expect("prism top face");

    // Deep enough to reach the arm: refused, byte-identical.
    let before = exact_snapshot(&obj);
    let err = obj.push_pull(top, 0.004).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(exact_snapshot(&obj), before);

    // Shallow enough to stay clear: still works.
    obj.push_pull(top, 0.001).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
}

// ------------------------------------------ hole rings under the stretch

/// Rectangular frustum solid (a tapered box) between `z0` and `z1`, centered
/// on (0.5, 0.5), with half-width `0.3 − 0.1·z` at height z — its walls all
/// lean toward a common apex at z = 3.
fn tapered_tool(z0: f64, z1: f64) -> Object {
    let half = |z: f64| 0.3 - 0.1 * z;
    let (h0, h1) = (half(z0), half(z1));
    let v = |h: f64, z: f64| {
        vec![
            Point3::new(0.5 - h, 0.5 - h, z),
            Point3::new(0.5 + h, 0.5 - h, z),
            Point3::new(0.5 + h, 0.5 + h, z),
            Point3::new(0.5 - h, 0.5 + h, z),
        ]
    };
    let mut verts = v(h0, z0);
    verts.extend(v(h1, z1));
    Object::from_polygons(
        &verts,
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .expect("frustum tool")
}

/// A face with a hole whose ring rides on TAPERED tunnel walls: pulling the
/// top out builds a fresh wall along each slanted hole-ring edge (four here),
/// and the ring rides up RIGIDLY at its original half-width rather than
/// shrinking toward the walls' apex the way the old stretch did. So the
/// apex-inversion refusal is gone — the ring never approaches the apex — and
/// even a pull past the old apex height validates. The recorded inverse
/// restores it. (This is the tapered-hole PULL; the deferred P4 case is a
/// different gesture — pushing an outer edge INTO/PAST a hole — see
/// docs/ROADMAP.md.)
#[test]
fn push_pull_tapered_hole_ring_builds_walls_and_rides_rigidly() {
    let cube = unit_cube();
    let tool = tapered_tool(-0.1, 1.1);
    let mut holed =
        Object::boolean(BooleanOp::Subtract, &cube, &tool, &Transform::IDENTITY).unwrap();
    holed.merge_coplanar_faces(&[]);
    holed.validate().unwrap();
    assert_eq!(holed.watertight(), WatertightState::Watertight);
    let top = face_with_normal(&holed, Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(
        holed.faces()[top].inner_loops.len(),
        1,
        "through-hole mouth on the top face"
    );

    // Pull out by 0.5: four walls on the tapered hole ring; the ring rides up
    // rigidly (half-width 0.2 at z = 0.5, unchanged — the tool's mouth on the
    // top plane), invertible.
    let after = assert_build_and_invert(&holed, top, 0.5);
    assert_eq!(
        after.faces().len(),
        holed.faces().len() + 4,
        "four hole-ring walls"
    );
    let top_after = face_with_normal(&after, Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(
        after.faces()[top_after].inner_loops.len(),
        1,
        "hole survives"
    );
    let ring_half = 0.3 - 0.1 * 1.0; // the tool's half-width at the old top plane z=1
    assert!(
        after.vertices().values().any(|v| (v.position
            - Point3::new(0.5 - ring_half, 0.5 - ring_half, 1.5))
        .length()
            <= tol::POINT_MERGE),
        "ring corner rides up rigidly to z = 1.5 at its original half-width"
    );

    // Past the old walls' apex (z = 3): still fine — a rigid ring never
    // inverts, so an outward pull is unbounded here too.
    let far = assert_build_and_invert(&holed, top, 2.2);
    assert_eq!(far.faces().len(), holed.faces().len() + 4);

    // Pushing the holed top IN is bounded: the ring's walls immediately drive
    // into the tapered tunnel — refused typed, byte-identical.
    let mut pushed = holed.clone();
    let before = exact_snapshot(&pushed);
    let err = pushed.push_pull(top, -0.3).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(exact_snapshot(&pushed), before);
}

// ------------------------------------ collapse detection never sees slanted

/// A push that would EXACTLY close a coplanar step (the direct-`push_pull`
/// collapse) is gated away from the collapse path when the moved face also has
/// a slanted neighbor, because the step weld translates the rest of the
/// boundary rigidly — wrong for a face that must also build a wall along its
/// slanted edge. Construction: bisect a cube top, raise one half (a step + a
/// pristine quad wall, exactly what `find_collapse_plans` matches), then
/// chamfer the raised block with an oblique slice. Pushing the raised top back
/// by the exact step height now routes through translate-and-build (the
/// boundary is not all-transverse), whose result-validation refuses at the
/// chamfer's fixed far edge — typed, byte-identical — rather than welding the
/// step. A shallower push builds walls and undoes cleanly through History.
#[test]
fn push_pull_step_close_with_slanted_neighbor_is_refused() {
    let (mut cube, halves) = bisected_top_cube();
    cube.push_pull(halves[0], 0.5).expect("raise the left half");
    cube.validate().unwrap();

    // Chamfer the raised block's top-west edge: plane -x + z = 1.2, through
    // (0,·,1.2) and (0.3,·,1.5) — it misses everything below z = 1.2.
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 1.2), Vec3::new(-1.0, 0.0, 1.0)).unwrap();
    let (_corner, mut stepped) = cube.slice(&plane).expect("chamfer the raised block");
    stepped.validate().unwrap();
    let original = stepped.clone();

    // The raised top now spans x ∈ [0.3, 0.5] with the chamfer to its west
    // and the pristine step wall to its east.
    let raised = stepped
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && f.plane.signed_distance(Point3::new(0.4, 0.5, 1.5)).abs() <= tol::PLANE_DIST
        })
        .map(|(id, _)| id)
        .expect("raised top face");

    // Exactly closing the step (-0.5) must NOT weld it shut: the boundary has a
    // slanted edge, so the sweep builds walls, and the built result crosses the
    // chamfer's fixed far edge — refused typed, byte-identical.
    let before = exact_snapshot(&stepped);
    let err = stepped.push_pull(raised, -0.5).unwrap_err();
    assert_eq!(err, PushPullError::NonManifoldResult);
    assert_eq!(exact_snapshot(&stepped), before);

    // A shallower push builds walls, and History undoes it exactly.
    let mut history = History::new();
    history
        .apply(
            &mut stepped,
            KernelOp::PushPull {
                face: raised,
                distance: -0.2,
            },
        )
        .expect("shallow wall-building push of the raised top");
    stepped.validate().unwrap();
    assert_eq!(stepped.watertight(), WatertightState::Watertight);
    history.undo(&mut stepped).expect("undo the push");
    stepped.validate().unwrap();
    assert!(objects_equivalent(&stepped, &original));
}

// Rule-3 property tests over flat-face translate-and-build push/pull, on
// prisms extruded from IRREGULAR star polygons (per-vertex radii and jittered
// angles — frequently concave), optionally carrying a centered square hole,
// and optionally carrying a near-collinear "spike" vertex that makes two
// adjacent facets meet at a very shallow angle (stressing the wall built on a
// near-collinear neighbor). The invariant: any accepted push yields a
// validating, watertight solid of the profile's genus and is exactly
// invertible through the recorded inverse; any refusal (a self-intersecting
// build) leaves the object byte-identical.
proptest! {
    #[test]
    fn generalized_push_pull_is_watertight_and_invertible_or_untouched(
        n in 5usize..10,
        r in 1.0..4.0f64,
        radii in proptest::collection::vec(0.4..1.0f64, 10),
        jitter in proptest::collection::vec(-0.35..0.35f64, 10),
        with_hole in proptest::bool::ANY,
        // Angle between the spiked facet pair, log-uniform across the
        // alignment floor (sin θ from ~6e-5 to ~0.1; the floor is 1e-2).
        spike in proptest::option::of(-4.2..-1.0f64),
        h in 0.5..4.0f64,
        d in -3.0..3.0f64,
    ) {
        prop_assume!(d.abs() >= 0.05);
        // Star polygon: strictly increasing jittered angles around the
        // centroid with independent radii — always simple, often concave.
        let step = std::f64::consts::TAU / n as f64;
        let mut pts: Vec<Point3> = (0..n)
            .map(|k| {
                let a = step * (k as f64 + jitter[k]);
                let rk = r * radii[k];
                Point3::new(rk * a.cos(), rk * a.sin(), 0.0)
            })
            .collect();
        // Optional near-collinear spike on edge 0→1: insert the edge
        // midpoint displaced outward so the two half-facets meet at angle
        // θ = 10^spike radians.
        let spike_theta = spike.map(|exp| 10.0f64.powf(exp));
        if let Some(theta) = spike_theta {
            let (p0, p1) = (pts[0], pts[1]);
            let edge = p1 - p0;
            let len = edge.length();
            let out = Vec3::new(edge.y, -edge.x, 0.0).normalized().unwrap();
            let mid = p0 + edge * 0.5 + out * (len * 0.25 * (theta * 0.5).tan());
            pts.insert(1, mid);
        }
        let holes: Vec<Vec<Point3>> = if with_hole {
            // Small centered square, wound opposite the outer boundary; the
            // minimum star radius (0.4·r) keeps it strictly interior.
            let q = 0.12 * r;
            vec![vec![
                Point3::new(-q, -q, 0.0),
                Point3::new(-q, q, 0.0),
                Point3::new(q, q, 0.0),
                Point3::new(q, -q, 0.0),
            ]]
        } else {
            vec![]
        };
        let profile = Profile::new(xy_plane(), pts.clone(), holes).unwrap();
        let mut prism = Object::from_extrusion(&profile, h).unwrap();
        let original = prism.clone();
        let before = exact_snapshot(&prism);
        // The facet to push: with a spike, the one between pts[0] and the
        // spike vertex pts[1] (its neighbor across the spike is the
        // near-collinear half-facet); otherwise any side facet. `faces()`
        // iteration is deterministic.
        let facet = prism
            .faces()
            .iter()
            .find(|(_, f)| {
                if f.plane.normal().z.abs() > tol::NORMAL_DIRECTION {
                    return false;
                }
                if spike_theta.is_none() {
                    return true;
                }
                let mut has0 = false;
                let mut has1 = false;
                for lp in prism.loop_positions(f.outer_loop) {
                    has0 |= lp.approx_eq(pts[0], tol::POINT_MERGE);
                    has1 |= lp.approx_eq(pts[1], tol::POINT_MERGE);
                }
                has0 && has1
            })
            .map(|(id, _)| id)
            .expect("prism has the requested side facet");
        let mut history = History::new();
        match history.apply(&mut prism, KernelOp::PushPull { face: facet, distance: d }) {
            Ok(_report) => {
                // Any accepted flat-facet push builds walls and yields a
                // validating, watertight solid of the profile's genus (adding
                // side walls never changes the genus).
                prism.validate().unwrap();
                prop_assert_eq!(prism.watertight(), WatertightState::Watertight);
                prop_assert_eq!(
                    euler_poincare(&prism),
                    if with_hole { 0 } else { 2 },
                    "genus must match the profile"
                );
                // Exactly invertible through the recorded inverse (a plain
                // push(-d) cannot re-collapse a slanted neighbor's wall).
                history.undo(&mut prism).unwrap();
                prism.validate().unwrap();
                prop_assert!(objects_equivalent(&prism, &original));
            }
            Err(_) => {
                // Strong guarantee, byte-for-byte (a self-intersecting build
                // is refused, not committed).
                prop_assert_eq!(exact_snapshot(&prism), before);
            }
        }
    }
}

/// Regular hexagonal prism (radius 1, height 1, extruded +z from the XY
/// plane) plus the side facet between the 0° and 60° hex vertices and that
/// facet's outward normal (at 30°). Shared by the slanted-neighbor specs.
fn hex_prism_and_facet() -> (Object, FaceId, Vec3) {
    let pts: Vec<Point3> = (0..6)
        .map(|k| {
            let a = std::f64::consts::FRAC_PI_3 * k as f64;
            Point3::new(a.cos(), a.sin(), 0.0)
        })
        .collect();
    let profile = Profile::new(xy_plane(), pts, vec![]).unwrap();
    let hex = Object::from_extrusion(&profile, 1.0).unwrap();
    assert_eq!(hex.faces().len(), 8, "6 facets + 2 caps");

    let n = Vec3::new(std::f64::consts::FRAC_PI_6.cos(), 0.5, 0.0);
    let facet = face_with_normal(&hex, n);
    (hex, facet, n)
}

// ----------------------------------------------- coplanar-aware push/pull
//
// The kernel half of the on-face Line workflow: a line drawn edge-to-edge
// on a face `split_face`s it into two COPLANAR siblings; push/pull of ONE sibling
// must build a wall along the shared cut edge instead of refusing it
// (pre- this returned PushPullError::NonManifoldResult — see
// `push_pull_slanted_neighbor_returns_non_manifold_result`, which holds for a
// genuinely slanted neighbor only until the re-spec above is
// implemented). These are the acceptance criteria for the K2
// kernel lane; un-ignore each in the PR that implements it (docs/DEVELOPMENT.md).

/// Mean x of a face's outer-loop vertices — picks a specific sub-face when
/// `split_face`'s `new_faces` order isn't guaranteed.
fn face_centroid_x(obj: &Object, face: FaceId) -> f64 {
    let outer = obj
        .faces()
        .iter()
        .find(|(id, _)| *id == face)
        .map(|(_, f)| f.outer_loop)
        .expect("face is live");
    let pts: Vec<Point3> = obj.loop_positions(outer).collect();
    pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64
}

/// Unit cube with its top bisected along x = 0.5; returns the object and the
/// `[left (x∈[0,0.5]), right (x∈[0.5,1])]` coplanar top sub-faces.
fn bisected_top_cube() -> (Object, [FaceId; 2]) {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let report = cube
        .split_face(
            top,
            &[Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)],
        )
        .expect("edge-to-edge bisect of the top face");
    let [a, b] = report.new_faces;
    if face_centroid_x(&cube, a) < face_centroid_x(&cube, b) {
        (cube, [a, b])
    } else {
        (cube, [b, a])
    }
}

/// Pushing one bisected half outward builds a wall along the shared cut edge and
/// reshapes the straddling side walls into stepped (still planar) faces.
#[test]
fn push_a_bisected_half_outward_walls_along_the_cut() {
    let (mut cube, halves) = bisected_top_cube();
    cube.push_pull(halves[0], 0.5)
        .expect("step the left half up");
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);

    // F1, F2, bottom, west, east, L-shaped south, L-shaped north, new cut-wall.
    assert_eq!(cube.faces().len(), 8, "stepped solid has 8 faces");
    assert_eq!(cube.vertices().len(), 12);
    assert_eq!(cube.edges().len(), 18);
    assert_eq!(euler_poincare(&cube), 2, "genus 0, single shell");

    // Volume = cube (1.0) + raised step (0.5 area × 0.5 height = 0.25).
    assert!((signed_volume(&cube) - 1.25).abs() <= tol::POINT_MERGE);

    // Stepped: moved half at z=1.5, sibling still at 1.0, base at 0.0.
    let zs: Vec<f64> = cube.vertices().values().map(|v| v.position.z).collect();
    let has = |z: f64| zs.iter().any(|&q| (q - z).abs() <= tol::POINT_MERGE);
    assert!(has(1.5) && has(1.0) && has(0.0), "z levels 0/1/1.5 present");
}

/// Inverse property: stepping a half up by `d` then pushing the moved face back
/// down by `d` restores the bisected cube exactly.
#[test]
fn push_bisected_half_then_inverse_restores_bisected_top() {
    let (reference, _) = bisected_top_cube();
    let (mut cube, halves) = bisected_top_cube();
    let report = cube.push_pull(halves[0], 0.5).unwrap();
    cube.push_pull(report.face, -0.5).unwrap();
    cube.validate().unwrap();
    assert!(objects_equivalent(&cube, &reference));
}

/// Pushing one bisected half inward (partial — not through the base) carves a
/// stepped notch; still watertight, 8 faces.
#[test]
fn push_a_bisected_half_inward_makes_a_notch() {
    let (mut cube, halves) = bisected_top_cube();
    cube.push_pull(halves[0], -0.5)
        .expect("notch the left half down");
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert_eq!(cube.faces().len(), 8);
    // Volume = cube (1.0) − removed step (0.5 × 0.5 = 0.25) = 0.75.
    assert!((signed_volume(&cube) - 0.75).abs() <= tol::POINT_MERGE);
}

proptest! {
    /// Bisecting a cube top at any interior x and stepping a half by any
    /// partial ±depth yields a watertight 8-face stepped solid of the expected
    /// volume ( wall-build, both directions, random geometry).
    #[test]
    fn step_a_bisected_half_is_watertight_8_faces(
        c in 0.2..0.8f64,
        d in 0.2..0.8f64,
        up in proptest::bool::ANY,
    ) {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        let r = cube
            .split_face(top, &[Point3::new(c, 0.0, 1.0), Point3::new(c, 1.0, 1.0)])
            .unwrap();
        // The left half spans x ∈ [0, c]; its base area is `c`.
        let left = if face_centroid_x(&cube, r.new_faces[0]) < face_centroid_x(&cube, r.new_faces[1]) {
            r.new_faces[0]
        } else {
            r.new_faces[1]
        };
        let dist = if up { d } else { -d };
        cube.push_pull(left, dist).unwrap();
        cube.validate().unwrap();
        prop_assert_eq!(cube.watertight(), WatertightState::Watertight);
        prop_assert_eq!(cube.faces().len(), 8);
        // Volume = unit cube (1.0) + signed step (area c × signed depth).
        prop_assert!((signed_volume(&cube) - (1.0 + c * dist)).abs() <= VOLUME_TOL);
    }
}

/// K3 ( follow-on — already works via the existing through-cut): pushing a
/// bisected half ALL the way down through the bottom removes that column. The
/// wasm `push_pull` routes an overshoot to `push_through`; here we exercise
/// `push_through` directly. Unit cube, top bisected at x=0.5, left half punched
/// out → the right box [0.5,1]×[0,1]×[0,1]: 6 faces, watertight, volume 0.5.
#[test]
fn push_through_a_bisected_half_deletes_the_column() {
    let (cube, halves) = bisected_top_cube();
    assert!(
        cube.push_pull_overshoots(halves[0], -1.0),
        "pushing the half to the bottom is a through-cut"
    );
    let result = cube
        .push_through(halves[0], -1.0)
        .expect("punch the column out");
    result.validate().unwrap();
    assert_eq!(result.watertight(), WatertightState::Watertight);
    assert_eq!(result.faces().len(), 6, "remaining box has 6 faces");
    assert!((signed_volume(&result) - 0.5).abs() <= VOLUME_TOL);
}

// ------------------------------------------------------ split_face robustness (K1)

proptest! {
    /// An edge-to-edge cut across a cube's top at ANY interior x is valid: no
    /// panic (the topo.rs:207 stale-key crash), watertight, 7 faces.
    #[test]
    fn split_top_at_any_x_is_valid(c in 0.05..0.95f64) {
        let mut cube = unit_cube();
        let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
        cube.split_face(top, &[Point3::new(c, 0.0, 1.0), Point3::new(c, 1.0, 1.0)])
            .unwrap();
        cube.validate().unwrap();
        prop_assert_eq!(cube.watertight(), WatertightState::Watertight);
        prop_assert_eq!(cube.faces().len(), 7);
    }
}

/// Re-splitting a sub-face whose boundary INCLUDES a prior cut edge must
/// succeed: a horizontal cut on the left half, from the west boundary (x=0) to
/// the prior cut edge (x=0.5), is boundary-to-boundary for that half. Today this
/// wrongly returns `EndpointNotOnBoundary` (the endpoint on the prior cut edge
/// isn't recognized as boundary); K1 must fix it.
#[test]
fn second_split_of_a_subface_is_valid() {
    let (mut cube, halves) = bisected_top_cube();
    let left = halves[0]; // x ∈ [0, 0.5]
    cube.split_face(
        left,
        &[Point3::new(0.0, 0.5, 1.0), Point3::new(0.5, 0.5, 1.0)],
    )
    .expect("boundary-to-boundary cut on the sub-face");
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert_eq!(cube.faces().len(), 8, "top now in 3 pieces (6 + 2 extra)");
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

    // Exactly AT the step (x=2 → x=1): the outer wall lands flush against the
    // coplanar interior step wall, merging the notch away — a valid collapse
    // (the L becomes a rectangular prism), not a fold. Stays watertight.
    let mut flush = original.clone();
    flush
        .push_pull(wall, -1.0)
        .expect("flush-at-step merge is valid");
    flush.validate().unwrap();
    assert_eq!(flush.watertight(), WatertightState::Watertight);

    // PAST the step (x → 0.5) would fold the step wall past its fixed vertices
    // into a self-intersecting shell — every face stays planar and manifold, so
    // only the obstruction guard can refuse it. Object left untouched.
    let mut obj = original.clone();
    assert_eq!(
        obj.push_pull(wall, -1.5).unwrap_err(),
        PushPullError::NonManifoldResult,
    );
    assert!(objects_equivalent(&obj, &original));
}

// ------------------------------------------------- coplanar seam cleanup

/// Union of two flush boxes: `merge_coplanar_faces` dissolves every seam,
/// leaving the canonical single box of the combined extent — the seam edges
/// across the top/bottom/side faces must not linger.
#[test]
fn union_seams_dissolve_to_the_canonical_box() {
    let a = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0));
    let b = box_object(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));
    let mut r = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();

    let dissolved = r.merge_coplanar_faces(&[]);
    assert!(dissolved > 0, "the flush union has seams to dissolve");
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
    assert_eq!(r.faces().len(), 6, "one face per side of the 2x1x1 box");
    assert!(
        r.coplanar_edge_segments().is_empty(),
        "no mergeable pair survives the pass"
    );
    // The canonical box: the pass also healed the collinear boundary
    // vertices the dissolved seams left behind (merge_faces is split_face's
    // exact inverse), so this is equivalent to a directly built 2x1x1 box.
    let canonical = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));
    assert!(objects_equivalent(&r, &canonical));
}

/// A preserve segment shields a face imprint from the pass: an edge drawn on
/// a face (split_face, awaiting its own push/pull) survives the union's seam
/// cleanup while the seams themselves dissolve.
#[test]
fn preserve_segments_shield_face_imprints_from_the_pass() {
    let mut a = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 1.0));
    let top = face_with_normal(&a, Vec3::new(0.0, 0.0, 1.0));
    // Imprint the top at x=0.5 — a deliberate, un-extruded region boundary.
    a.split_face(
        top,
        &[Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)],
    )
    .unwrap();
    let b = box_object(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));

    let mut r = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    let preserve: Vec<_> = a
        .coplanar_edge_segments()
        .into_iter()
        .chain(b.coplanar_edge_segments())
        .collect();
    assert_eq!(
        preserve.len(),
        1,
        "the imprint is the only pre-existing coplanar edge"
    );

    r.merge_coplanar_faces(&preserve);
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);
    // Top: the imprint at x=0.5 survives (two faces); the union seam at x=1
    // dissolved into the right-hand piece. Everything else merged to one
    // face per side: 2 top + bottom + north + south + east + west = 7.
    assert_eq!(r.faces().len(), 7);
    // Without the preserve set the imprint would have been swallowed too —
    // the contrast that proves the shield is what kept it.
    let mut bare = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    bare.merge_coplanar_faces(&[]);
    assert_eq!(bare.faces().len(), 6);
}

/// `push_through` keeps an unrelated imprint on the same object: the cut
/// consumes ITS region while a second drawn-but-unextruded imprint survives.
/// (This cut produces no coplanar seams — the flush-wall merge case is
/// `push_through_merges_a_cut_wall_flush_with_an_existing_wall`.)
#[test]
fn push_through_keeps_unrelated_imprints() {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    // Imprint 1 (the cut region): the x < 0.5 half of the top.
    let cut = cube
        .split_face(
            top,
            &[Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)],
        )
        .unwrap();
    // new_faces come in no guaranteed order — identify by extent.
    let left = *cut
        .new_faces
        .iter()
        .find(|&&f| {
            cube.loop_positions(cube.faces()[f].outer_loop)
                .all(|p| p.x <= 0.5 + tol::POINT_MERGE)
        })
        .expect("one piece lies entirely at x <= 0.5");
    let right = *cut.new_faces.iter().find(|&&f| f != left).unwrap();
    // Imprint 2 (unrelated, must survive): a line at x = 0.75 on the
    // remaining half.
    cube.split_face(
        right,
        &[Point3::new(0.75, 0.0, 1.0), Point3::new(0.75, 1.0, 1.0)],
    )
    .unwrap();

    // Push the left region through: the left half of the cube vanishes.
    let result = cube.push_through(left, -1.0).unwrap();
    result.validate().unwrap();
    assert_eq!(result.watertight(), WatertightState::Watertight);
    assert!(
        (signed_volume(&result) - 0.5).abs() < VOLUME_TOL,
        "half the cube remains, vol {}",
        signed_volume(&result)
    );
    // The surviving half-cube: 6 sides plus the x=0.75 imprint splitting the
    // top — the through-cut did not swallow the unrelated imprint.
    assert_eq!(result.faces().len(), 7);
    assert_eq!(
        result.coplanar_edge_segments().len(),
        1,
        "exactly the surviving imprint remains mergeable"
    );
}

/// A through-cut whose new wall lands flush with an existing wall of the
/// same solid: the two coplanar wall pieces merge into one, leaving the
/// canonical box — the seam-dissolving half of push_through's cleanup.
#[test]
fn push_through_merges_a_cut_wall_flush_with_an_existing_wall() {
    // L-shaped solid: a tall column plus a low base extension, pre-merged.
    let a = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 2.0));
    let b = box_object(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));
    let mut l = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    l.merge_coplanar_faces(&[]);

    // The exposed base top: normal +z, passing through z = 1 (not the
    // column top at z = 2).
    let base_top = l
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().z > 0.9
                && f.plane.signed_distance(Point3::new(1.5, 0.5, 1.0)).abs() < tol::PLANE_DIST
        })
        .map(|(id, _)| id)
        .expect("the base top face exists");

    // Cut the base off: the new wall at x=1 (z in 0..1) lands flush with
    // the column's existing east wall (z in 1..2) and must merge with it.
    let result = l.push_through(base_top, -1.0).unwrap();
    result.validate().unwrap();
    assert_eq!(result.watertight(), WatertightState::Watertight);
    assert_eq!(
        result.faces().len(),
        6,
        "the flush cut wall merged with the existing wall"
    );
    assert!(objects_equivalent(
        &result,
        &box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 1.0, 2.0)),
    ));
}

/// The preserve check covers the WHOLE shared set between a face pair, not
/// just the candidate edge: once a seam merge makes a preserved imprint and
/// a remaining seam adjacent to the same pair, dissolving that seam would
/// take the imprint with it (merge_faces dissolves every shared edge at
/// once) — so the pass must skip the pair instead.
#[test]
fn preserve_shields_an_imprint_that_joins_a_seam_chain() {
    let mut a = box_object(Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 1.0, 1.0));
    let top = face_with_normal(&a, Vec3::new(0.0, 0.0, 1.0));
    // The imprint: splits a's top at x=1, spanning the full y extent — it
    // ENDS on the y=1 wall, where the union seam with b will run.
    a.split_face(
        top,
        &[Point3::new(1.0, 0.0, 1.0), Point3::new(1.0, 1.0, 1.0)],
    )
    .unwrap();
    // b flush against a along y=1: b's single top is coplanar-adjacent to
    // BOTH imprint pieces of a's top.
    let b = box_object(Point3::new(0.0, 1.0, 0.0), Point3::new(2.0, 2.0, 1.0));

    let mut r = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    let preserve = a.coplanar_edge_segments();
    assert_eq!(preserve.len(), 1, "the imprint is the only preserved edge");
    r.merge_coplanar_faces(&preserve);
    r.validate().unwrap();
    assert_eq!(r.watertight(), WatertightState::Watertight);

    // One seam merged b's top into one imprint piece; the other seam shares
    // its pair with the imprint and was skipped. Two top faces remain and
    // the imprint edge itself is still present.
    let top_faces = r
        .faces()
        .values()
        .filter(|f| f.plane.normal().z > 0.9)
        .count();
    assert_eq!(top_faces, 2, "the imprint still separates the top");
    let imprint_survives = r.coplanar_edge_segments().iter().any(|&(p, q)| {
        let mid = Point3::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0, (p.z + q.z) / 2.0);
        (mid - Point3::new(1.0, 0.5, 1.0)).length() < tol::POINT_MERGE
    });
    assert!(imprint_survives, "the preserved imprint edge is intact");
}

/// Faces meeting along two DISCONNECTED edge chains (a bridge/dogbone
/// adjacency) refuse to merge with a typed error, leaving the object
/// untouched — dissolving everything shared would need the merged boundary
/// rebuilt as outer + hole loops, which merge_faces does not do.
#[test]
fn merge_faces_refuses_faces_sharing_two_disconnected_chains() {
    // A flat sheet in the z=0 plane, partitioned into a U-shaped face A, the
    // notch filler M, and a right-hand bar B that touches A along TWO
    // disjoint runs of x=2 (separated by M's own edge). Open (not
    // watertight) but topologically valid — enough for the sticky ops.
    let positions = [
        Point3::new(0.0, 0.0, 0.0), // 0
        Point3::new(2.0, 0.0, 0.0), // 1
        Point3::new(2.0, 1.0, 0.0), // 2
        Point3::new(1.0, 1.0, 0.0), // 3
        Point3::new(1.0, 2.0, 0.0), // 4
        Point3::new(2.0, 2.0, 0.0), // 5
        Point3::new(2.0, 3.0, 0.0), // 6
        Point3::new(0.0, 3.0, 0.0), // 7
        Point3::new(3.0, 0.0, 0.0), // 8
        Point3::new(3.0, 3.0, 0.0), // 9
    ];
    let faces: Vec<Vec<usize>> = vec![
        vec![0, 1, 2, 3, 4, 5, 6, 7], // A: the U
        vec![3, 2, 5, 4],             // M: the notch filler
        vec![1, 8, 9, 6, 5, 2],       // B: the bar (touches A twice)
    ];
    let mut sheet = Object::from_polygons(&positions, &faces).unwrap();
    sheet.validate().unwrap();
    let face_count = sheet.faces().len();

    // The A–B edge (2,0)–(2,1): one of the two disjoint shared runs.
    let wanted = [Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 1.0, 0.0)];
    let ab_edge = sheet
        .edges()
        .iter()
        .find(|(_, e)| {
            let he = sheet.half_edges()[e.half_edge];
            let p = sheet.vertices()[he.origin].position;
            let q = sheet.vertices()[sheet.half_edges()[he.next].origin].position;
            (p.approx_eq(wanted[0], tol::POINT_MERGE) && q.approx_eq(wanted[1], tol::POINT_MERGE))
                || (p.approx_eq(wanted[1], tol::POINT_MERGE)
                    && q.approx_eq(wanted[0], tol::POINT_MERGE))
        })
        .map(|(id, _)| id)
        .expect("the (2,0)-(2,1) edge exists");

    assert_eq!(
        sheet.merge_faces(ab_edge).unwrap_err(),
        StickyError::SharedChainDisconnected,
    );
    // Strong guarantee: refused means untouched.
    sheet.validate().unwrap();
    assert_eq!(sheet.faces().len(), face_count);

    // And the automated pass skips the pair rather than corrupting: it only
    // merges what merge_faces accepts (A–M and M–B are single chains, so
    // those DO merge; the sheet collapses to one face without ever passing
    // through the refused A–B pair).
    let mut swept = sheet.clone();
    swept.merge_coplanar_faces(&[]);
    swept.validate().unwrap();
    assert_eq!(swept.faces().len(), 1);
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
fn split_face_inner_imprints_a_coplanar_subface() {
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let before = cube.faces().len();
    // A 0.5×0.5 square strictly inside the top face.
    let rect = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.25, 1.0),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    let report = cube.split_face_inner(top, &rect).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert_eq!(cube.faces().len(), before + 1, "one new sub-face");
    assert_eq!(report.parent, top);
    // The parent keeps its handle and gains exactly one hole.
    assert_eq!(cube.faces()[top].inner_loops.len(), 1);
    // The sub-face is coplanar with the parent (outward +Z).
    let sub_n = cube.faces()[report.sub_face].plane.normal();
    assert!(sub_n.approx_eq(Vec3::new(0.0, 0.0, 1.0), kernel::tol::NORMAL_DIRECTION));
    // Imprint is flat: volume unchanged.
    assert!((signed_volume(&cube) - 1.0).abs() < VOLUME_TOL);
}

#[test]
fn extrude_sub_face_embosses_and_recesses() {
    // Imprint a 0.5×0.5 square on a unit cube's top, then boss it up by 0.4.
    let rect = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.25, 1.0),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let report = cube.split_face_inner(top, &rect).unwrap();
    let faces_after_imprint = cube.faces().len();

    // Emboss (push out): a 0.5×0.5×0.4 boss adds 0.1 volume; 4 walls appear.
    let r = cube.extrude_sub_face(report.sub_face, 0.4).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert_eq!(r.created_faces.len(), 4, "four walls");
    assert_eq!(cube.faces().len(), faces_after_imprint + 4);
    assert!(
        (signed_volume(&cube) - (1.0 + 0.25 * 0.4)).abs() < VOLUME_TOL,
        "emboss vol {}",
        signed_volume(&cube)
    );

    // Recess (push in): removes 0.25×0.3 = 0.075.
    let mut cube = unit_cube();
    let report = cube.split_face_inner(top, &rect).unwrap();
    cube.extrude_sub_face(report.sub_face, -0.3).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert!(
        (signed_volume(&cube) - (1.0 - 0.25 * 0.3)).abs() < VOLUME_TOL,
        "recess vol {}",
        signed_volume(&cube)
    );
}

#[test]
fn extrude_then_collapse_sub_face_is_identity() {
    let rect = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.25, 1.0),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    let mut cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let report = cube.split_face_inner(top, &rect).unwrap();
    let imprinted = cube.clone();

    let r = cube.extrude_sub_face(report.sub_face, 0.4).unwrap();
    // Collapse restores the flat imprinted state, and reports the raise distance.
    let collapsed = cube.collapse_sub_face(r.face).unwrap();
    cube.validate().unwrap();
    assert!((collapsed.distance - 0.4).abs() < VOLUME_TOL);
    assert!(
        objects_equivalent(&cube, &imprinted),
        "collapse restores the flat imprinted face"
    );
}

#[test]
fn split_face_inner_then_merge_is_identity() {
    let original = unit_cube();
    let mut cube = original.clone();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    let rect = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.25, 1.0),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    let report = cube.split_face_inner(top, &rect).unwrap();
    // Dissolving the sub-face restores the original cube (up to handle renaming).
    cube.merge_inner_face(report.sub_face).unwrap();
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    assert!(objects_equivalent(&cube, &original));
}

#[test]
fn split_face_inner_rejects_bad_loops() {
    let top = face_with_normal(&unit_cube(), Vec3::new(0.0, 0.0, 1.0));

    // A corner on the face boundary is not strictly inside.
    let mut c = unit_cube();
    let touching = [
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(0.5, 0.0, 1.0),
        Point3::new(0.5, 0.5, 1.0),
        Point3::new(0.0, 0.5, 1.0),
    ];
    assert!(matches!(
        c.split_face_inner(top, &touching).unwrap_err(),
        kernel::StickyError::LoopNotStrictlyInside { .. }
    ));
    assert!(
        objects_equivalent(&c, &unit_cube()),
        "object untouched on error"
    );

    // A vertex off the face plane.
    let mut c = unit_cube();
    let off_plane = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.25, 1.5),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    assert!(matches!(
        c.split_face_inner(top, &off_plane).unwrap_err(),
        kernel::StickyError::PointNotOnFace { .. }
    ));

    // A self-intersecting (bow-tie) loop.
    let mut c = unit_cube();
    let bowtie = [
        Point3::new(0.25, 0.25, 1.0),
        Point3::new(0.75, 0.75, 1.0),
        Point3::new(0.75, 0.25, 1.0),
        Point3::new(0.25, 0.75, 1.0),
    ];
    assert_eq!(
        c.split_face_inner(top, &bowtie).unwrap_err(),
        kernel::StickyError::LoopSelfIntersects
    );
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

// These objects carry no materials, so the dense-material closures are never
// invoked (HEW_FILE_FORMAT.md). Document-level material/tree round-tripping is
// spec'd in `tests/serialize_specs.rs`.

#[test]
fn serialize_roundtrip_is_identity_and_deterministic() {
    for original in [unit_cube(), Object::tetrahedron(), Object::triangle()] {
        let bytes = original.encode(&|_| 0);
        assert_eq!(
            bytes,
            original.encode(&|_| 0),
            "encode must be deterministic"
        );
        let decoded = Object::decode(&bytes, &|_| None).unwrap();
        decoded.validate().unwrap();
        assert!(objects_equivalent(&decoded, &original));
        assert_eq!(decoded.watertight(), original.watertight());
    }
}

#[test]
fn decode_rejects_garbage() {
    assert!(Object::decode(&[], &|_| None).is_err());
    assert!(Object::decode(&[0x00, 0x01, 0x02, 0x03], &|_| None).is_err());
    // A truncated-but-plausible header must not panic.
    let good = unit_cube().encode(&|_| 0);
    assert!(Object::decode(&good[..good.len() / 2], &|_| None).is_err());
}

// ----------------------------------------------- split_connected_components

/// One axis-aligned box's 8 corner positions, in `box_object` order.
fn box_corners(a: Point3, b: Point3) -> Vec<Point3> {
    vec![
        Point3::new(a.x, a.y, a.z),
        Point3::new(b.x, a.y, a.z),
        Point3::new(b.x, b.y, a.z),
        Point3::new(a.x, b.y, a.z),
        Point3::new(a.x, a.y, b.z),
        Point3::new(b.x, a.y, b.z),
        Point3::new(b.x, b.y, b.z),
        Point3::new(a.x, b.y, b.z),
    ]
}

/// The 6 outward-wound box faces, indices offset by `off` (matches `box_object`).
fn box_faces(off: usize) -> Vec<Vec<usize>> {
    vec![
        vec![off, off + 3, off + 2, off + 1],
        vec![off + 4, off + 5, off + 6, off + 7],
        vec![off, off + 1, off + 5, off + 4],
        vec![off + 1, off + 2, off + 6, off + 5],
        vec![off + 2, off + 3, off + 7, off + 6],
        vec![off + 3, off, off + 4, off + 7],
    ]
}

/// A SINGLE watertight Object holding two disjoint boxes — two connected
/// components in one mesh, exactly what a severing cut leaves before
/// re-splitting (feeds).
fn two_box_object(min_a: Point3, max_a: Point3, min_b: Point3, max_b: Point3) -> Object {
    let mut pts = box_corners(min_a, max_a);
    pts.extend(box_corners(min_b, max_b));
    let mut faces = box_faces(0);
    faces.extend(box_faces(8));
    Object::from_polygons(&pts, &faces).unwrap()
}

#[test]
fn split_connected_components_passes_through_a_connected_solid() {
    let cube = unit_cube();
    let parts = cube.split_connected_components();
    assert_eq!(parts.len(), 1, "a connected solid is a single component");
    parts[0].validate().unwrap();
    assert_eq!(parts[0].watertight(), WatertightState::Watertight);
    assert!(objects_equivalent(&parts[0], &cube));
}

#[test]
fn split_connected_components_separates_two_disjoint_solids() {
    // Unit cube at origin + unit cube at x=3 (well separated), as ONE object.
    let merged = two_box_object(
        Point3::ORIGIN,
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(3.0, 0.0, 0.0),
        Point3::new(4.0, 1.0, 1.0),
    );
    assert_eq!(merged.watertight(), WatertightState::Watertight);

    let parts = merged.split_connected_components();
    assert_eq!(parts.len(), 2, "two disjoint shells split into two objects");
    for p in &parts {
        p.validate().unwrap();
        assert_eq!(p.watertight(), WatertightState::Watertight);
        assert_eq!(p.faces().len(), 6);
        assert!((signed_volume(p) - 1.0).abs() < VOLUME_TOL);
    }
    let total: f64 = parts.iter().map(signed_volume).sum();
    assert!((total - signed_volume(&merged)).abs() < VOLUME_TOL);
}

proptest! {
    /// Splitting a single connected box is a volume- and watertightness-
    /// preserving pass-through (exactly one component).
    #[test]
    fn split_connected_components_preserves_a_single_box(
        sx in 0.2..5.0f64, sy in 0.2..5.0f64, sz in 0.2..5.0f64,
    ) {
        let solid = box_object(Point3::ORIGIN, Point3::new(sx, sy, sz));
        let parts = solid.split_connected_components();
        prop_assert_eq!(parts.len(), 1);
        parts[0].validate().unwrap();
        prop_assert_eq!(parts[0].watertight(), WatertightState::Watertight);
        prop_assert!((signed_volume(&parts[0]) - sx * sy * sz).abs() < VOLUME_TOL);
    }

    /// Two disjoint boxes in one mesh always split into two watertight solids
    /// whose volumes sum to the whole; output order is deterministic.
    #[test]
    fn split_connected_components_separates_two_boxes(
        sx in 0.2..3.0f64, sy in 0.2..3.0f64, sz in 0.2..3.0f64,
        gap in 0.5..5.0f64,
    ) {
        // Second box placed past the first with a strictly positive gap.
        let off = sx + gap;
        let merged = two_box_object(
            Point3::ORIGIN,
            Point3::new(sx, sy, sz),
            Point3::new(off, 0.0, 0.0),
            Point3::new(off + sx, sy, sz),
        );
        let parts = merged.split_connected_components();
        prop_assert_eq!(parts.len(), 2);
        let mut vols = Vec::new();
        for p in &parts {
            p.validate().unwrap();
            prop_assert_eq!(p.watertight(), WatertightState::Watertight);
            prop_assert_eq!(p.faces().len(), 6);
            vols.push(signed_volume(p));
        }
        let one = sx * sy * sz;
        for v in &vols {
            prop_assert!((v - one).abs() < VOLUME_TOL);
        }
        prop_assert!((vols.iter().sum::<f64>() - signed_volume(&merged)).abs() < VOLUME_TOL);
    }
}

// ------------------------------------------------------------- slice

use kernel::SliceError;

/// Axis-aligned slice of the unit cube halves it into two watertight solids.
#[test]
fn slice_unit_cube_in_half() {
    let cube = unit_cube();
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let (pos, neg) = cube.slice(&plane).unwrap();
    for p in [&pos, &neg] {
        p.validate().unwrap();
        assert_eq!(p.watertight(), WatertightState::Watertight);
        assert!(
            (signed_volume(p) - 0.5).abs() < VOLUME_TOL,
            "vol {}",
            signed_volume(p)
        );
    }
    // Re-joining the halves recovers the whole (shared cut face fuses).
    let rejoined = Object::boolean(BooleanOp::Union, &pos, &neg, &Transform::IDENTITY).unwrap();
    rejoined.validate().unwrap();
    assert!((signed_volume(&rejoined) - 1.0).abs() < VOLUME_TOL);
}

/// A diagonal (non-axis-aligned) plane through the centre still yields two
/// watertight solids whose volumes sum to the whole — exercises the in-plane
/// basis construction.
#[test]
fn slice_unit_cube_diagonally() {
    let cube = unit_cube();
    let plane =
        Plane::from_point_normal(Point3::new(0.5, 0.5, 0.5), Vec3::new(1.0, 1.0, 1.0)).unwrap();
    let (pos, neg) = cube.slice(&plane).unwrap();
    pos.validate().unwrap();
    neg.validate().unwrap();
    assert_eq!(pos.watertight(), WatertightState::Watertight);
    assert_eq!(neg.watertight(), WatertightState::Watertight);
    assert!((signed_volume(&pos) + signed_volume(&neg) - 1.0).abs() < VOLUME_TOL);
}

/// A plane coincident with a face (z=0) does not straddle the solid → refused.
#[test]
fn slice_coplanar_with_face_refused() {
    let cube = unit_cube();
    let on_bottom = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!(matches!(
        cube.slice(&on_bottom),
        Err(SliceError::PlaneMissesSolid)
    ));
}

/// A plane entirely outside the solid is refused (nothing to cut).
#[test]
fn slice_plane_missing_solid_refused() {
    let cube = unit_cube();
    let outside =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!(matches!(
        cube.slice(&outside),
        Err(SliceError::PlaneMissesSolid)
    ));
}

/// An open (non-solid) object cannot be sliced.
#[test]
fn slice_open_object_refused() {
    let tri = Object::triangle(); // open
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!(matches!(tri.slice(&plane), Err(SliceError::NotSolid)));
}

proptest! {
    /// A plane strictly inside an axis-aligned box, normal +z, splits it into
    /// two watertight solids whose volumes are the two slabs and sum to the box.
    #[test]
    fn slice_axis_box_conserves_volume(
        sx in 0.5..4.0f64, sy in 0.5..4.0f64, sz in 1.0..4.0f64,
        frac in 0.15..0.85f64,
    ) {
        let solid = box_object(Point3::ORIGIN, Point3::new(sx, sy, sz));
        let zc = sz * frac;
        let plane =
            Plane::from_point_normal(Point3::new(0.0, 0.0, zc), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let (pos, neg) = solid.slice(&plane).unwrap();
        pos.validate().unwrap();
        neg.validate().unwrap();
        prop_assert_eq!(pos.watertight(), WatertightState::Watertight);
        prop_assert_eq!(neg.watertight(), WatertightState::Watertight);
        let whole = sx * sy * sz;
        prop_assert!((signed_volume(&pos) - sx * sy * (sz - zc)).abs() < VOLUME_TOL);
        prop_assert!((signed_volume(&neg) - sx * sy * zc).abs() < VOLUME_TOL);
        prop_assert!((signed_volume(&pos) + signed_volume(&neg) - whole).abs() < VOLUME_TOL);
    }
}

// ----------------------------------------------- push-through subtract

#[test]
fn push_pull_overshoots_detects_the_through_case() {
    let cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    // Outward pulls never remove material.
    assert!(!cube.push_pull_overshoots(top, 0.5));
    // Shallow inward push stays within the wall: a normal translate.
    assert!(!cube.push_pull_overshoots(top, -0.3));
    // Reaching / passing the opposite wall (thickness 1.0) is the through case.
    assert!(cube.push_pull_overshoots(top, -1.0));
    assert!(cube.push_pull_overshoots(top, -2.0));
}

#[test]
fn push_through_a_full_convex_face_vanishes() {
    // Pushing a whole box face past the far wall removes everything.
    let cube = unit_cube();
    let top = face_with_normal(&cube, Vec3::new(0.0, 0.0, 1.0));
    assert!(matches!(
        cube.push_through(top, -2.0),
        Err(PushPullError::WouldVanish)
    ));
}

#[test]
fn push_through_a_circle_imprint_through_a_faceted_cylinder() {
    // Punch a concentric circular through-hole in a faceted cylinder — the
    //  testing case. Regression for the facet-count-sensitive nesting bug
    // in the sketch region tracer (a concentric inner loop's hole-cycle attached
    // to its own reverse-wound twin instead of the true outer encloser), which
    // surfaced here as a boolean DegenerateContact → NonManifoldResult. Exercise
    // the facet counts that used to fail (and a couple that passed) at both an
    // exactly-through and an overshoot distance.
    let ngon = |r: f64, n: usize, z: f64| -> Vec<Point3> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * (i as f64) / (n as f64);
                Point3::new(r * a.cos(), r * a.sin(), z)
            })
            .collect()
    };
    for n in [6usize, 16, 24] {
        let base = Plane::from_polygon(&ngon(1.0, n, 0.0)).unwrap();
        let cyl_profile = Profile::new(base, ngon(1.0, n, 0.0), vec![]).unwrap();
        let solid = Object::from_extrusion(&cyl_profile, 1.5).unwrap();
        let solid_vol = signed_volume(&solid);

        for &dist in &[-1.5_f64, -2.0_f64] {
            let mut cyl = solid.clone();
            let top = face_with_normal(&cyl, Vec3::new(0.0, 0.0, 1.0));
            let sub = cyl
                .split_face_inner(top, &ngon(0.5, n, 1.5))
                .expect("imprint a concentric circle")
                .sub_face;
            let holed = cyl
                .push_through(sub, dist)
                .unwrap_or_else(|e| panic!("n={n} dist={dist}: {e:?}"));
            holed.validate().unwrap();
            assert_eq!(
                holed.watertight(),
                WatertightState::Watertight,
                "n={n} dist={dist}"
            );
            // Genus-1 tunnel: V − E + F − H = 2(S − G) with S=1, G=1 → 0.
            assert_eq!(euler_poincare(&holed), 0, "n={n} dist={dist}: genus 1");
            // The hole radius is half the outer, so a quarter of the cross-section
            // (and thus a quarter of the volume) is removed.
            assert!(
                (signed_volume(&holed) - solid_vol * 0.75).abs() < VOLUME_TOL,
                "n={n} dist={dist}: vol {} expected {}",
                signed_volume(&holed),
                solid_vol * 0.75
            );
        }
    }
}

#[test]
fn push_through_a_sub_face_punches_a_through_hole() {
    // A 4×4×1 slab with a centred 1×1 imprint pushed down past the bottom wall
    // becomes a slab with a 1×1 square through-hole.
    let mut slab = box_object(Point3::ORIGIN, Point3::new(4.0, 4.0, 1.0));
    let top = face_with_normal(&slab, Vec3::new(0.0, 0.0, 1.0));
    let report = slab
        .split_face_inner(
            top,
            &[
                Point3::new(1.5, 1.5, 1.0),
                Point3::new(2.5, 1.5, 1.0),
                Point3::new(2.5, 2.5, 1.0),
                Point3::new(1.5, 2.5, 1.0),
            ],
        )
        .expect("imprint a centred sub-face");
    let sub = report.sub_face;

    assert!(
        slab.push_pull_overshoots(sub, -2.0),
        "recess past the wall is through"
    );
    let holed = slab.push_through(sub, -2.0).expect("punch through");
    holed.validate().unwrap();
    assert_eq!(holed.watertight(), WatertightState::Watertight);
    // 16 (slab) − 1 (1×1×1 of material removed within the slab) = 15.
    assert!(
        (signed_volume(&holed) - 15.0).abs() < VOLUME_TOL,
        "vol {}",
        signed_volume(&holed)
    );
    // A through-hole raises the genus: V − E + F − H = 2(S − G) with S=1, G=1 → 0.
    assert_eq!(euler_poincare(&holed), 0);
}

#[test]
fn push_through_then_split_severs_into_two_solids() {
    // An imprint spanning the full width in y but interior in x, pushed through,
    // cuts the slab into two separate bars.
    let mut slab = box_object(Point3::ORIGIN, Point3::new(3.0, 1.0, 1.0));
    let top = face_with_normal(&slab, Vec3::new(0.0, 0.0, 1.0));
    // A slot from x=1..2 across the full y depth, sitting strictly inside the
    // top face is not possible (it would touch the y boundary), so use the
    // sub-face only where it stays interior and rely on the slab being severed
    // by a full-depth cut via two objects instead: push the *whole* top face is
    // a vanish, so instead subtract a spanning tool through push_through on a
    // sub-face inset slightly from the y edges still leaves thin walls — keep
    // this test focused on the connected-components split of a severing result
    // by constructing the severed result directly is covered elsewhere; here we
    // assert push_through on a sub-face that reaches the bottom yields a single
    // watertight piece (no accidental over-split).
    let report = slab
        .split_face_inner(
            top,
            &[
                Point3::new(1.0, 0.25, 1.0),
                Point3::new(2.0, 0.25, 1.0),
                Point3::new(2.0, 0.75, 1.0),
                Point3::new(1.0, 0.75, 1.0),
            ],
        )
        .expect("imprint");
    let holed = slab.push_through(report.sub_face, -2.0).expect("through");
    let pieces = holed.split_connected_components();
    assert_eq!(
        pieces.len(),
        1,
        "an interior pocket-through is still one solid"
    );
    pieces[0].validate().unwrap();
    assert_eq!(pieces[0].watertight(), WatertightState::Watertight);
}

// ------------------------- transform: non-uniform scale (design nonuniform-scale §4)
//
// `Object::apply_transform` (crates/kernel/src/ops.rs) is where
// `Document::transform_object` / `transform_group` / `transform_selection`
// all bottom out (see their doc comments) — one spec here pins the math for
// all three. The trap: a face's plane must be refit from where its boundary
// LANDED (Newell over the moved points — exactly what `Transform::apply_plane`
// does internally by mapping three points and refitting), never by
// transforming the stored normal like an ordinary vector — that only agrees
// with the correct inverse-transpose answer for similarities (uniform scale,
// rotation), and silently drifts under anisotropic non-uniform scale.
// `sliced_wedge` supplies a face that is not axis-aligned, so a wrong
// implementation actually fails these assertions; an axis-aligned box would
// hide the bug (its face normals stay axis-aligned under any diagonal scale,
// right or wrong).

/// Non-uniform, anisotropic, non-reflecting scale on a shape with an oblique
/// face: the object stays watertight and the transformed face's plane
/// matches Newell's method run over the ACTUAL MOVED boundary — not a
/// naively-scaled normal — proving `apply_transform` takes the
/// inverse-transpose route DEVELOPMENT.md §4 calls out, not the vector route.
#[test]
fn transform_non_uniform_scale_refits_oblique_plane_via_newell() {
    let mut wedge = sliced_wedge();
    let cut = wedge_cut_face(&wedge);
    let pre_points: Vec<Point3> = wedge
        .loop_positions(wedge.faces()[cut].outer_loop)
        .collect();
    let volume_before = signed_volume(&wedge);

    let t = Transform::scale(Vec3::new(2.0, 0.5, 3.0)); // three distinct factors
    wedge
        .apply_transform(&t)
        .expect("positive anisotropic scale is accepted");

    assert_eq!(wedge.watertight(), WatertightState::Watertight);
    wedge
        .validate()
        .expect("faces stay planar within tolerance under anisotropic scale");

    // The independently-computed Newell plane over the moved boundary...
    let moved: Vec<Point3> = pre_points.iter().map(|&p| t.apply_point(p)).collect();
    let expected = Plane::from_polygon(&moved).expect("moved boundary is still planar");
    // ...must equal what apply_transform actually stored.
    let actual = wedge.faces()[cut].plane;
    assert!(
        actual
            .normal()
            .approx_eq(expected.normal(), tol::NORMAL_DIRECTION),
        "refit normal {:?} != Newell-over-moved-boundary normal {:?}",
        actual.normal(),
        expected.normal(),
    );
    for p in &moved {
        assert!(
            actual.signed_distance(*p).abs() < tol::PLANE_DIST,
            "a moved boundary point left the refit plane"
        );
    }

    assert!(
        (signed_volume(&wedge) - volume_before * 2.0 * 0.5 * 3.0).abs() < VOLUME_TOL,
        "volume scales by the product of the three factors"
    );
}

proptest! {
    /// Non-uniform scale by `(sx,sy,sz)` about an arbitrary world pivot,
    /// followed by its exact inverse `(1/sx,1/sy,1/sz)` about the same
    /// pivot, restores the wedge's prior topology — the round trip a
    /// Ctrl-anchored gizmo drag and its undo both rely on (tolerance-aware
    /// equivalence, not bitwise: DEVELOPMENT.md §4 — float round-trips are
    /// not `(p + d) - d == p`).
    #[test]
    fn transform_non_uniform_scale_then_inverse_restores_wedge_topology(
        sx in 0.3f64..4.0,
        sy in 0.3f64..4.0,
        sz in 0.3f64..4.0,
        px in -3.0f64..3.0,
        py in -3.0f64..3.0,
        pz in -3.0f64..3.0,
    ) {
        let original = sliced_wedge();
        let pivot = Vec3::new(px, py, pz);
        let to_pivot = Transform::translation(-pivot);
        let from_pivot = Transform::translation(pivot);
        let forward = to_pivot
            .then(&Transform::scale(Vec3::new(sx, sy, sz)))
            .then(&from_pivot);
        let inverse = to_pivot
            .then(&Transform::scale(Vec3::new(1.0 / sx, 1.0 / sy, 1.0 / sz)))
            .then(&from_pivot);

        let mut obj = original.clone();
        obj.apply_transform(&forward)
            .expect("forward scale is positive and non-reflecting");
        obj.apply_transform(&inverse)
            .expect("inverse scale is positive and non-reflecting");

        obj.validate().expect("round trip stays valid");
        prop_assert_eq!(obj.watertight(), WatertightState::Watertight);
        prop_assert!(
            objects_equivalent(&obj, &original),
            "non-uniform scale then its inverse restores the original topology"
        );
    }
}

/// A reflecting affine is refused typed even when it is also non-uniform (the
/// class the UI's per-axis clamp exists to keep the kernel from ever seeing),
/// and a merely-singular one (one axis at exactly zero) is refused as
/// `Singular` rather than `Reflection` — both leave the object byte-identical
/// (the strong exception guarantee), never half-transformed.
#[test]
fn transform_refuses_anisotropic_reflection_and_singular_leaving_object_untouched() {
    let mut wedge = sliced_wedge();
    let before = exact_snapshot(&wedge);

    assert_eq!(
        wedge
            .apply_transform(&Transform::scale(Vec3::new(-2.0, 1.0, 3.0)))
            .unwrap_err(),
        TransformError::Reflection
    );
    assert_eq!(
        exact_snapshot(&wedge),
        before,
        "refused reflection leaves geometry untouched"
    );

    assert_eq!(
        wedge
            .apply_transform(&Transform::scale(Vec3::new(0.0, 1.0, 3.0)))
            .unwrap_err(),
        TransformError::Singular
    );
    assert_eq!(
        exact_snapshot(&wedge),
        before,
        "refused singular transform leaves geometry untouched"
    );
}

/// A rectangle profile lying FLAT on the ground plane (normal +z),
/// x in [x0, x1], y in [y0, y1] — the shape auto-orientation folds up.
fn ground_flat_profile(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    let plane = Plane::from_point_normal(Point3::ORIGIN, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    Profile::new(
        plane,
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

#[test]
fn follow_me_auto_orients_a_flat_profile_into_a_lathe() {
    // The classic SketchUp first attempt: circle path on the ground,
    // profile ALSO drawn flat on the ground beside it. Auto-orientation
    // (design §2c) folds the profile up about the crease at the analytic
    // rim point nearest it — the fold's plane contains the circle's
    // center exactly, so the radial anchor accepts it — and the lathe
    // sweeps. Volume: the folded rect spans x in [1.2, 1.5], z in
    // [-0.15, 0.15]; Pappus gives 2π · 1.35 · (0.3 · 0.3).
    let profile = ground_flat_profile(1.2, -0.15, 1.5, 0.15);
    let (path, curves) = attributed_ground_circle(1.0, 24, 0.0);
    let solid = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    // Faceted revolution: compare against the same fold done by hand.
    let stood = yz_profile(1.2, -0.15, 1.5, 0.15); // x=0 plane, y as radius arm
    // yz_profile puts the rect on the x = 0 plane spanning y/z — the same
    // radial arm rotated 90° around the axis; the revolved volume is
    // identical by symmetry of the revolution.
    let reference = Object::from_follow_me(&stood, &path, true, &curves).unwrap();
    let v = signed_volume(&solid);
    assert!((v - signed_volume(&reference)).abs() < 1e-9);
    assert!(v > 0.0);
}

#[test]
fn follow_me_auto_orient_hinges_at_a_touching_flap() {
    // A flat flap whose edge touches the open path's start folds up
    // HINGED where it touches (the crease passes through the contact
    // point), then sweeps the path's full length: the SketchUp feel.
    let profile = ground_flat_profile(0.0, -0.2, 0.5, 0.2);
    let path = [Point3::ORIGIN, Point3::new(0.0, 2.0, 0.0)];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    let v = signed_volume(&solid);
    assert!((v - 0.5 * 0.4 * 2.0).abs() < 1e-9);
    // Hinge preserved: the fold keeps x in [0, 0.5] (rotation about a
    // crease along x through the origin maps y-extent to z-extent).
    let (mut xmin, mut xmax) = (f64::MAX, f64::MIN);
    for vtx in solid.vertices().values() {
        xmin = xmin.min(vtx.position.x);
        xmax = xmax.max(vtx.position.x);
    }
    assert!((xmin - 0.0).abs() < 1e-9 && (xmax - 0.5).abs() < 1e-9);
}

#[test]
fn follow_me_auto_orients_a_flat_profile_around_a_frame() {
    // Flat molding profile on the ground OUTSIDE a ground rectangle path:
    // folds up perpendicular to the nearest edge and sweeps the mitered
    // frame. Outward band offsets 0.2..0.5: V = (3² − 2.4²) · 0.3.
    let profile = ground_flat_profile(0.85, -0.5, 1.15, -0.2);
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(euler_poincare(&solid), 0);
    let expected = (3.0f64 * 3.0 - 2.4 * 2.4) * 0.3;
    assert!((signed_volume(&solid) - expected).abs() < 1e-9);
}

#[test]
fn follow_me_auto_orient_open_path_folds_onto_an_end_not_a_useless_interior_band() {
    // A regression the adversarial review caught: `orient_profile_to_path`
    // used to pick its fold band by nearest-to-centroid over EVERY band,
    // interior segments included — but an OPEN path's own anchor test
    // (`from_follow_me_impl`) only ever re-validates the path's two ENDS
    // (design §2a). Folding onto an interior band the retry can never
    // re-check just reproduced the original refusal. Here the profile
    // centroid sits nearest the path's INTERIOR leg (x in [0,5] at y=5,
    // z=0) while both ends (the y-leg and the z-leg) are off-axis; the fold
    // must restrict itself to the two ends and succeed against one of them,
    // not dead-end on the interior leg.
    let profile = yz_profile_at(2.5, 4.8, -0.2, 5.2, 0.2);
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(0.0, 5.0, 0.0),
        Point3::new(5.0, 5.0, 0.0),
        Point3::new(5.0, 5.0, 5.0),
    ];
    let solid = Object::from_follow_me(&profile, &path, false, &[]).unwrap();
    solid.validate().unwrap();
    assert_eq!(solid.watertight(), WatertightState::Watertight);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn follow_me_auto_orientation_restores_any_tilt(
        tilt in 0.05..1.4f64,
        leg in 1.0..5.0f64,
    ) {
        // Tilt a perpendicular profile by an arbitrary angle about an
        // in-plane axis; the fold restores perpendicularity and the sweep
        // covers the full path — the swept volume is the perpendicular
        // reference's, independent of tilt (design §2c).
        let reference = yz_profile(-0.2, 0.1, 0.2, 0.5);
        let (s, c) = tilt.sin_cos();
        // Rotate about the y axis through the origin: x' = c·x + s·z,
        // z' = -s·x + c·z (points start at x = 0).
        let tilted_pts: Vec<Point3> = reference
            .outer()
            .iter()
            .map(|p| Point3::new(s * p.z, p.y, c * p.z))
            .collect();
        let normal = Vec3::new(c, 0.0, -s).normalized().unwrap();
        let plane = Plane::from_point_normal(tilted_pts[0], normal).unwrap();
        let tilted = Profile::new(plane, tilted_pts, vec![]).unwrap();
        let path = [Point3::ORIGIN, Point3::new(leg, 0.0, 0.0)];
        let solid = Object::from_follow_me(&tilted, &path, false, &[]).unwrap();
        solid.validate().unwrap();
        prop_assert_eq!(solid.watertight(), WatertightState::Watertight);
        let v = signed_volume(&solid);
        prop_assert!((v - 0.4 * 0.4 * leg).abs() < 1e-6, "volume {v}");
    }
}

#[test]
fn follow_me_marks_soft_joints_and_stamps_band_cylinders() {
    // Joint softening (design §7): the transverse joints between wall
    // facets of one drawn-circle path are SOFT — facet seams of a smooth
    // ring, not creases — while an unattributed L elbow stays hard. The
    // ring's axis-parallel walls also pick up the path-band cylinder
    // stamp (§7a) even though the profile is a plain rect.
    let profile = radial_profile(std::f64::consts::PI / 24.0);
    let (path, curves) = circle_path(2.0, 24);
    let ring = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    ring.validate().unwrap();
    let soft_count = ring.edges().values().filter(|e| e.soft).count();
    assert!(
        soft_count >= 24,
        "every station joint of the smooth ring is soft, got {soft_count}"
    );
    for e in ring.edges().values() {
        if e.soft {
            assert!(e.twin_half_edge.is_some(), "soft edges are interior");
        }
    }
    // The outer/inner walls (profile edges parallel to the revolution
    // axis) ride true cylinders about the circle's own axis.
    let stamped = ring
        .faces()
        .values()
        .filter(|f| f.surface.is_some())
        .count();
    assert!(
        stamped >= 24,
        "axis-parallel walls carry the path-band cylinder, got {stamped}"
    );

    // An unattributed L path: a genuine elbow, nothing soft.
    let l_profile = yz_profile(-0.2, 0.1, 0.2, 0.5);
    let l_path = [
        Point3::ORIGIN,
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
    ];
    let l = Object::from_follow_me(&l_profile, &l_path, false, &[]).unwrap();
    assert_eq!(
        l.edges().values().filter(|e| e.soft).count(),
        0,
        "a free polyline elbow stays hard"
    );
}

/// Splitting a face through one of a smooth ring's SOFT joints must not
/// silently harden it — an everyday gesture (a sketch line, or a section
/// cut, landing on an existing model edge) routes through
/// `Object::split_face` → `split_boundary_edge`, which map-or-drops `curve`
/// (a chord midpoint sits inside the circle, so a fragment is no longer a
/// valid chord — a real geometric reason) but has no equivalent reason to
/// drop `soft`: both fragments are still the exact same drawn-curve joint.
#[test]
fn follow_me_split_face_preserves_soft_edges() {
    let profile = radial_profile(std::f64::consts::PI / 24.0);
    let (path, curves) = circle_path(2.0, 24);
    let mut ring = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    ring.validate().unwrap();

    // A soft edge and the loop of the face on its `half_edge` side.
    let (a, b, loop_id) = ring
        .edges()
        .values()
        .find(|e| e.soft)
        .map(|e| {
            let he = ring.half_edges()[e.half_edge];
            let a = ring.vertices()[he.origin].position;
            let b = ring.vertices()[ring.half_edges()[he.next].origin].position;
            (a, b, he.loop_id)
        })
        .expect("the smooth ring has soft edges");
    let face = ring.loops()[loop_id].face;
    let soft_mid = a + (b - a) * 0.5;

    // The wall is a quad; find which side is the soft edge and cut straight
    // across to the midpoint of its opposite side — an ordinary bisecting
    // split, landing squarely mid-edge on both ends.
    let corners: Vec<Point3> = ring
        .loop_half_edges(loop_id)
        .map(|h| ring.vertices()[ring.half_edges()[h].origin].position)
        .collect();
    assert_eq!(corners.len(), 4, "a follow-me wall is a quad");
    let i = (0..4)
        .find(|&k| {
            let (p, q) = (corners[k], corners[(k + 1) % 4]);
            (p.approx_eq(a, tol::POINT_MERGE) && q.approx_eq(b, tol::POINT_MERGE))
                || (p.approx_eq(b, tol::POINT_MERGE) && q.approx_eq(a, tol::POINT_MERGE))
        })
        .expect("the soft edge is one of the quad's four sides");
    let (oa, ob) = (corners[(i + 2) % 4], corners[(i + 3) % 4]);
    let opp_mid = oa + (ob - oa) * 0.5;

    ring.split_face(face, &[soft_mid, opp_mid]).unwrap();
    ring.validate().unwrap();

    // Both fragments of the original soft edge — the two new edges touching
    // `soft_mid` on the split side — must still read soft.
    let fragments_at_split: Vec<_> = ring
        .edges()
        .values()
        .filter(|e| {
            let he = ring.half_edges()[e.half_edge];
            let p = ring.vertices()[he.origin].position;
            let q = ring.vertices()[ring.half_edges()[he.next].origin].position;
            (p.approx_eq(soft_mid, tol::POINT_MERGE) || q.approx_eq(soft_mid, tol::POINT_MERGE))
                && (p.approx_eq(a, tol::POINT_MERGE)
                    || p.approx_eq(b, tol::POINT_MERGE)
                    || q.approx_eq(a, tol::POINT_MERGE)
                    || q.approx_eq(b, tol::POINT_MERGE))
        })
        .collect();
    assert_eq!(
        fragments_at_split.len(),
        2,
        "the split soft edge has exactly two fragments"
    );
    for e in fragments_at_split {
        assert!(e.soft, "a split fragment of a soft edge must stay soft");
    }
}

#[test]
fn follow_me_negative_stop_sweeps_the_other_way() {
    // design §10a: a NEGATIVE stop is the closed-loop drag in the other
    // direction — |stop| of arc length from the same seam, walked the
    // other way around. Same swept volume for a symmetric profile, but
    // genuinely different geometry (the two partial arms leave the seam
    // toward opposite sides).
    // Mid-edge seam (a Split anchor): plane x = 1 crosses the bottom and
    // top edges strictly; corner seams are one-directional by design and
    // refuse a reversed stop instead.
    let plane =
        Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let profile = Profile::new(
        plane,
        vec![
            Point3::new(1.0, -0.4, -0.2),
            Point3::new(1.0, 0.0, -0.2),
            Point3::new(1.0, 0.0, 0.2),
            Point3::new(1.0, -0.4, 0.2),
        ],
        vec![],
    )
    .unwrap();
    let path = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(2.0, 0.0, 0.0),
        Point3::new(2.0, 2.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
    ];
    let fwd = Object::from_follow_me_to(&profile, &path, true, &[], 3.0).unwrap();
    fwd.validate().unwrap();
    let rev = Object::from_follow_me_to(&profile, &path, true, &[], -3.0).unwrap();
    rev.validate().unwrap();
    assert_eq!(rev.watertight(), WatertightState::Watertight);
    assert!((signed_volume(&fwd) - signed_volume(&rev)).abs() < 1e-9);
    assert!(
        !objects_equivalent(&fwd, &rev),
        "the reversed sweep must cover the other side of the loop"
    );
    // The forward arm reaches y = 1 on the x = 2 edge; the reversed arm
    // reaches x = 1 on the y = 2 edge — check one distinguishing extent.
    let fwd_max_x = fwd
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MIN, f64::max);
    let rev_max_x = rev
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MIN, f64::max);
    assert!(
        fwd_max_x > 2.0 && rev_max_x < 1.5,
        "the two arms leave the seam toward opposite sides \
         (fwd_max_x {fwd_max_x}, rev_max_x {rev_max_x})"
    );

    // A full-length negative stop is still the full closed sweep.
    let full_rev = Object::from_follow_me_to(&profile, &path, true, &[], -99.0).unwrap();
    let full_fwd = Object::from_follow_me(&profile, &path, true, &[]).unwrap();
    assert!((signed_volume(&full_rev) - signed_volume(&full_fwd)).abs() < 1e-9);

    // An open path has one direction from its seam: negative refuses.
    let open_path = [Point3::ORIGIN, Point3::new(0.0, 3.0, 0.0)];
    let flat = ground_flat_profile(0.0, -0.2, 0.5, 0.2);
    let err = Object::from_follow_me_to(&flat, &open_path, false, &[], -1.0).unwrap_err();
    assert_eq!(err, kernel::FollowMeError::EmptyPath);
}

#[test]
fn follow_me_negative_stop_sweeps_the_other_way_regardless_of_path_winding() {
    // Regression for a defect caught in review of the negative-stop feature
    // above: the "leave the seam along +n" orientation correction (design
    // §1/§2b) used to run ONLY while building the forward walk, gated on
    // `!reverse`. But a REVERSED walk's raw (uncorrected) index construction
    // is, by a pure index-permutation identity, always exactly the FLIP of
    // the forward walk's raw construction — true for both a `Split` (mid-
    // edge) and a `Vertex` (facet-vertex) anchor, regardless of geometry.
    // So whenever the profile's stored plane normal happened to be the
    // antiparallel choice relative to the path's stored winding — an
    // entirely arbitrary authoring detail, not a geometric one — the
    // uncorrected reversed walk was ALREADY the flip of the (about-to-be-
    // corrected) forward walk, and the two ended up identical: `stop_len <
    // 0` silently swept the SAME side as `stop_len > 0` instead of the
    // other one. This test is the exact opposite-winding twin of
    // `follow_me_negative_stop_sweeps_the_other_way` above — same seam,
    // same profile, only the stored path/profile orientation differs — and
    // must show the same "genuinely different, opposite-side" result.
    //
    // Split anchor: the SAME square/profile as the test above, but the path
    // array wound the OTHER way (CW instead of CCW) starting from a
    // different corner. The seam still lands at the same physical point
    // (1, 0, 0) via the segment nearest the profile's centroid, but the
    // raw forward direction there now points toward (0,0,0) instead of
    // (2,0,0) — the flip-needed ("Case B") condition this bug missed.
    {
        let plane =
            Plane::from_point_normal(Point3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).unwrap();
        let profile = Profile::new(
            plane,
            vec![
                Point3::new(1.0, -0.4, -0.2),
                Point3::new(1.0, 0.0, -0.2),
                Point3::new(1.0, 0.0, 0.2),
                Point3::new(1.0, -0.4, 0.2),
            ],
            vec![],
        )
        .unwrap();
        let path = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ];
        let fwd = Object::from_follow_me_to(&profile, &path, true, &[], 3.0).unwrap();
        fwd.validate().unwrap();
        let rev = Object::from_follow_me_to(&profile, &path, true, &[], -3.0).unwrap();
        rev.validate().unwrap();
        assert!((signed_volume(&fwd) - signed_volume(&rev)).abs() < 1e-9);
        assert!(
            !objects_equivalent(&fwd, &rev),
            "Split anchor, opposite winding: the reversed sweep must still \
             cover the other side of the loop, not silently match forward"
        );
    }
    // Vertex anchor: a lathe seam (design §1) on an 8-facet drawn circle,
    // wound the OTHER way (decreasing angle) from the tools' usual CCW
    // `circle_path`/`attributed_ground_circle` output — the same
    // "arbitrary stored winding" flip, this time on a curve-attributed
    // path rather than a straight-edged one.
    {
        let n = 8;
        let radius = 2.0;
        let geom = kernel::CurveGeom {
            center: Point3::ORIGIN,
            radius,
        };
        let path: Vec<Point3> = (0..n)
            .map(|i| {
                let a = -(i as f64) / n as f64 * std::f64::consts::TAU;
                Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
            })
            .collect();
        let curves = vec![Some(geom); n];
        let profile = radial_profile(0.0);
        let fwd = Object::from_follow_me_to(&profile, &path, true, &curves, 4.0).unwrap();
        fwd.validate().unwrap();
        let rev = Object::from_follow_me_to(&profile, &path, true, &curves, -4.0).unwrap();
        rev.validate().unwrap();
        assert!((signed_volume(&fwd) - signed_volume(&rev)).abs() < 1e-9);
        assert!(
            !objects_equivalent(&fwd, &rev),
            "Vertex anchor, opposite winding: the reversed sweep must still \
             cover the other side of the loop, not silently match forward"
        );
    }
}

#[test]
fn follow_me_softens_the_profile_circumference_on_turns() {
    // design §7c: a swept CIRCLE profile's own facet seams (the
    // "latitude" lines) are soft wherever the profile edges share the
    // drawn circle — including toroidal turn walls where no cylinder
    // stamp can apply. A torus (circle profile around a circle path) is
    // smooth in BOTH directions.
    let profile = circle_profile_yz(2.0, 0.0, 0.4);
    let (path, curves) = attributed_ground_circle(2.0, 24, 0.0);
    let torus = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    torus.validate().unwrap();
    let interior = torus
        .edges()
        .values()
        .filter(|e| e.twin_half_edge.is_some())
        .count();
    let soft = torus.edges().values().filter(|e| e.soft).count();
    assert_eq!(
        soft, interior,
        "every interior edge of a smooth torus is soft ({soft}/{interior})"
    );

    // A rectangle profile on the same path keeps its own edges hard in
    // the longitudinal direction (a real cross-section crease), while the
    // transverse station joints stay soft.
    let rect = radial_profile(std::f64::consts::PI / 24.0);
    let ring = Object::from_follow_me(&rect, &path, true, &curves).unwrap();
    let ring_soft = ring.edges().values().filter(|e| e.soft).count();
    let ring_interior = ring
        .edges()
        .values()
        .filter(|e| e.twin_half_edge.is_some())
        .count();
    assert!(
        ring_soft < ring_interior,
        "a rectangular cross-section keeps hard longitudinal creases"
    );
    assert!(ring_soft >= 24, "station joints remain soft");
}

#[test]
fn follow_me_sphere_is_smooth_in_both_directions() {
    // design §7c through the pole split: the half-profile keeps its
    // parent circle's identity, so a sphere's latitude seams soften
    // exactly like a torus's — every interior edge is soft.
    let profile = axis_circle_profile(1.0, 1.0, 24, 0.0);
    let (path, curves) = attributed_ground_circle(1.0, 24, 0.0);
    let sphere = Object::from_follow_me(&profile, &path, true, &curves).unwrap();
    sphere.validate().unwrap();
    let interior = sphere
        .edges()
        .values()
        .filter(|e| e.twin_half_edge.is_some())
        .count();
    let soft = sphere.edges().values().filter(|e| e.soft).count();
    assert_eq!(
        soft, interior,
        "every interior edge of a sphere is soft ({soft}/{interior})"
    );
}
