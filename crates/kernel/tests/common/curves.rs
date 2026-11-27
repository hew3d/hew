//! Shared fixture builders for the true-curves specs (surface classes,
//! blends, shapes). Every solid is built through public kernel entry points
//! only. Extend this file in later work; never duplicate a helper inside a
//! test file.
#![allow(dead_code)]

use kernel::trig;
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use kernel::{
    BooleanOp, CurveGeom, EdgeId, FaceId, Object, Plane, Point3, Profile, Sketch, SurfaceRef,
    TopologyError, Transform, Vec3, WatertightState,
};

/// Distance within which two fixture points are "the same" when a helper
/// looks an element up by position. A test-side lookup slack, not a kernel
/// tolerance: fixtures are built from exact f64 inputs.
pub const LOOKUP_EPS: f64 = 1e-9;

pub fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// The ground plane z = 0, normal +Z.
pub fn ground() -> Plane {
    Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

/// An orthonormal in-plane basis `(u, v)` for unit normal `n`, `u × v = n`.
pub fn basis(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = helper.cross(n).normalized().unwrap();
    let v = n.cross(u);
    (u, v)
}

/// Builds an Object from convex-solid polygons, orienting every face so its
/// Newell normal points away from the solid's vertex centroid. Only valid
/// for convex solids (boxes, wedges, pyramids).
pub fn convex_solid(points: &[Point3], faces: &[Vec<usize>]) -> Object {
    let n = points.len() as f64;
    let c = points
        .iter()
        .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| acc + p.to_vec())
        / n;
    let oriented: Vec<Vec<usize>> = faces
        .iter()
        .map(|f| {
            let ring: Vec<Point3> = f.iter().map(|&i| points[i]).collect();
            let plane = Plane::from_polygon(&ring).unwrap();
            let fc = ring
                .iter()
                .fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| acc + p.to_vec())
                / ring.len() as f64;
            if plane.normal().dot(fc - c) < 0.0 {
                f.iter().rev().copied().collect()
            } else {
                f.clone()
            }
        })
        .collect();
    Object::from_polygons(points, &oriented).unwrap()
}

/// An axis-aligned box with one corner at the origin and the opposite
/// corner at `(x, y, z)`.
pub fn box_xyz(x: f64, y: f64, z: f64) -> Object {
    let pts = [
        p3(0.0, 0.0, 0.0),
        p3(x, 0.0, 0.0),
        p3(x, y, 0.0),
        p3(0.0, y, 0.0),
        p3(0.0, 0.0, z),
        p3(x, 0.0, z),
        p3(x, y, z),
        p3(0.0, y, z),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons(&pts, &faces).unwrap()
}

/// The 1 m cube at the origin.
pub fn unit_box() -> Object {
    box_xyz(1.0, 1.0, 1.0)
}

/// Extrudes the ground polygon `pts` (CCW seen from +Z) by `h` along +Z.
pub fn prism(pts: &[(f64, f64)], h: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0)).unwrap();
    }
    extrude_only_region(&s, h)
}

/// A unit-height prism whose vertical edge at the origin has dihedral
/// `theta` (the triangle `(0,0)`, `(1,0)`, `(cos θ, sin θ)` extruded 1 m).
pub fn wedge(theta: f64) -> Object {
    prism(
        &[(0.0, 0.0), (1.0, 0.0), (trig::cos(theta), trig::sin(theta))],
        1.0,
    )
}

/// Adds a closed analytic circle chain of `n` facets to `s`, in the plane
/// through `center` with unit `normal`, starting at angle 0 of `basis(normal)`.
pub fn add_circle(s: &mut Sketch, center: Point3, normal: Vec3, r: f64, n: usize) {
    let (u, v) = basis(normal);
    s.begin_curve_with(CurveGeom { center, radius: r }).unwrap();
    let p = |i: usize| {
        let a = TAU * (i % n) as f64 / n as f64;
        center + u * (r * trig::cos(a)) + v * (r * trig::sin(a))
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
}

/// Adds an analytic arc chain about `center` (ground plane) from angle `a0`
/// sweeping `sweep` radians in `n` facets.
pub fn add_arc(s: &mut Sketch, center: Point3, r: f64, a0: f64, sweep: f64, n: usize) {
    s.begin_curve_with(CurveGeom { center, radius: r }).unwrap();
    let p = |i: usize| {
        let a = a0 + sweep * i as f64 / n as f64;
        p3(
            center.x + r * trig::cos(a),
            center.y + r * trig::sin(a),
            center.z,
        )
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
}

/// A circle profile (with its `CurveGeom` attribution) of `n` facets.
pub fn circle_profile(center: Point3, normal: Vec3, r: f64, n: usize) -> Profile {
    let plane = Plane::from_point_normal(center, normal).unwrap();
    let mut s = Sketch::on_plane(plane);
    add_circle(&mut s, center, normal, r, n);
    let region = s.regions().keys().next().unwrap();
    s.profile(region).unwrap()
}

/// Extrudes the sole region of `s` (panics unless there is exactly one).
pub fn extrude_only_region(s: &Sketch, h: f64) -> Object {
    assert_eq!(s.regions().len(), 1, "fixture sketch has one region");
    let region = s.regions().keys().next().unwrap();
    Object::from_extrusion(&s.profile(region).unwrap(), h).unwrap()
}

/// Extrudes the region of `s` that has holes (panics unless exactly one).
pub fn extrude_holed_region(s: &Sketch, h: f64) -> Object {
    let holed: Vec<_> = s
        .regions()
        .keys()
        .filter(|&r| !s.profile(r).unwrap().holes().is_empty())
        .collect();
    assert_eq!(holed.len(), 1, "fixture sketch has one holed region");
    Object::from_extrusion(&s.profile(holed[0]).unwrap(), h).unwrap()
}

/// A cylinder of radius `r`, height `h`, `n` wall facets, base centered at
/// the origin on the ground; walls claim the cylinder.
pub fn cylinder(r: f64, h: f64, n: usize) -> Object {
    Object::from_extrusion(
        &circle_profile(p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), r, n),
        h,
    )
    .unwrap()
}

/// A `w × d` rectangle centered at the origin with corner arcs of radius
/// `r` (`n_arc` facets each), extruded `h`. Corner walls claim cylinders.
pub fn rounded_rect_prism(w: f64, d: f64, r: f64, h: f64, n_arc: usize) -> Object {
    let (hx, hy) = (w / 2.0, d / 2.0);
    let mut s = Sketch::on_plane(ground());
    // Corners CCW from +x+y; each arc sweeps a quarter turn.
    let corners = [
        (hx - r, hy - r, 0.0),
        (-hx + r, hy - r, PI / 2.0),
        (-hx + r, -hy + r, PI),
        (hx - r, -hy + r, 3.0 * PI / 2.0),
    ];
    for (i, &(cx, cy, a0)) in corners.iter().enumerate() {
        add_arc(&mut s, p3(cx, cy, 0.0), r, a0, PI / 2.0, n_arc);
        let end = (
            cx + r * trig::cos(a0 + PI / 2.0),
            cy + r * trig::sin(a0 + PI / 2.0),
        );
        let (nx, ny, na) = corners[(i + 1) % 4];
        let next_start = (nx + r * trig::cos(na), ny + r * trig::sin(na));
        s.add_segment(p3(end.0, end.1, 0.0), p3(next_start.0, next_start.1, 0.0))
            .unwrap();
    }
    extrude_only_region(&s, h)
}

/// A D-shaped post: the half disc `x ≥ 0` of radius `r` (`n` facets per full
/// turn, so `n/2` on the arc) closed by its diameter, extruded `h`.
pub fn d_post(r: f64, h: f64, n: usize) -> Object {
    let mut s = Sketch::on_plane(ground());
    add_arc(&mut s, p3(0.0, 0.0, 0.0), r, -PI / 2.0, PI, n / 2);
    s.add_segment(p3(0.0, r, 0.0), p3(0.0, -r, 0.0)).unwrap();
    extrude_only_region(&s, h)
}

/// A `w × w` plate of thickness `t`, centered at the origin, with a
/// through hole of radius `hole_r` (`n` facets) at its center.
pub fn plate_with_hole(w: f64, t: f64, hole_r: f64, n: usize) -> Object {
    let h = w / 2.0;
    let mut s = Sketch::on_plane(ground());
    let sq = [(-h, -h), (h, -h), (h, h), (-h, h)];
    for i in 0..4 {
        let (a, b) = (sq[i], sq[(i + 1) % 4]);
        s.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0)).unwrap();
    }
    add_circle(
        &mut s,
        p3(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        hole_r,
        n,
    );
    extrude_holed_region(&s, t)
}

/// A `w × w × t` plate (centered at the origin) with a cylindrical boss of
/// radius `boss_r` (`n` facets) standing `boss_h` on its top center.
pub fn plate_with_boss(w: f64, t: f64, boss_r: f64, boss_h: f64, n: usize) -> Object {
    let plate = box_xyz(w, w, t);
    let plate = {
        let mut p = plate;
        p.apply_transform(&Transform::translation(Vec3::new(-w / 2.0, -w / 2.0, 0.0)))
            .unwrap();
        p
    };
    // The boss starts inside the plate so the union has a proper overlap.
    let mut boss = cylinder(boss_r, boss_h + t / 2.0, n);
    boss.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, t / 2.0)))
        .unwrap();
    Object::boolean(BooleanOp::Union, &plate, &boss, &Transform::IDENTITY).unwrap()
}

/// An L-profile prism: `(0,0) (2,0) (2,1) (1,1) (1,2) (0,2)` extruded 1 m.
pub fn l_block() -> Object {
    prism(
        &[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ],
        1.0,
    )
}

/// A square pyramid: base `[-1, 1]²` on the ground, apex `(0, 0, 1)`.
pub fn pyramid4() -> Object {
    let pts = [
        p3(-1.0, -1.0, 0.0),
        p3(1.0, -1.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(-1.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
    ];
    let faces = vec![
        vec![0, 1, 2, 3],
        vec![0, 1, 4],
        vec![1, 2, 4],
        vec![2, 3, 4],
        vec![3, 0, 4],
    ];
    convex_solid(&pts, &faces)
}

/// The edge whose endpoints are `a` and `b` (either order), within
/// [`LOOKUP_EPS`]. Panics if absent.
pub fn edge_between(obj: &Object, a: Point3, b: Point3) -> EdgeId {
    obj.edges()
        .keys()
        .find(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            (p.approx_eq(a, LOOKUP_EPS) && q.approx_eq(b, LOOKUP_EPS))
                || (p.approx_eq(b, LOOKUP_EPS) && q.approx_eq(a, LOOKUP_EPS))
        })
        .unwrap_or_else(|| panic!("no edge between {a:?} and {b:?}"))
}

/// The unique face whose plane normal is `n` (within [`LOOKUP_EPS`]).
pub fn face_with_normal(obj: &Object, n: Vec3) -> FaceId {
    let hits: Vec<FaceId> = obj
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().approx_eq(n, LOOKUP_EPS))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(hits.len(), 1, "expected one face with normal {n:?}");
    hits[0]
}

/// `validate()` passes and the object is watertight.
pub fn assert_valid_solid(obj: &Object) {
    obj.validate().expect("object validates");
    assert_eq!(
        obj.watertight(),
        WatertightState::Watertight,
        "object is watertight"
    );
}

/// The enclosed volume by the divergence theorem over every face loop
/// (holes wind opposite, so they subtract).
pub fn signed_volume(obj: &Object) -> f64 {
    let mut v = 0.0;
    for f in obj.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let ring: Vec<Point3> = obj.loop_positions(l).collect();
            let o = ring[0].to_vec();
            for i in 1..ring.len() - 1 {
                let (a, b) = (ring[i].to_vec(), ring[i + 1].to_vec());
                v += o.dot(a.cross(b)) / 6.0;
            }
        }
    }
    v
}

/// Faces per claim class (`"none"` for unclaimed), by `SurfaceRef::kind_name`.
pub fn claim_census(obj: &Object) -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for f in obj.faces().values() {
        let k = f.surface.map_or("none", |s| s.kind_name());
        *m.entry(k).or_insert(0) += 1;
    }
    m
}

// ------------------------------------------------- claimed single facets

/// +Z.
pub const Z: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// A single open quad face object (its one face) built from four planar
/// corners, plus the face id.
pub fn facet(corners: [Point3; 4]) -> (Object, FaceId) {
    Plane::from_polygon(&corners).expect("fixture quad is planar");
    let obj = Object::from_polygons(&corners, &[vec![0, 1, 2, 3]]).unwrap();
    let f = obj.faces().keys().next().unwrap();
    (obj, f)
}

pub fn with_claim(obj: &Object, f: FaceId, s: SurfaceRef) -> Object {
    let mut o = obj.clone();
    o.set_face_surface_for_test(f, Some(s));
    o
}

pub fn assert_mismatch(obj: &Object, f: FaceId, s: SurfaceRef) {
    assert_eq!(
        with_claim(obj, f, s).validate(),
        Err(TopologyError::FaceSurfaceMismatch { face: f }),
        "claim {s:?} must be rejected"
    );
}

pub fn sphere_pt(r: f64, lat: f64, lon: f64) -> Point3 {
    p3(
        r * trig::cos(lat) * trig::cos(lon),
        r * trig::cos(lat) * trig::sin(lon),
        r * trig::sin(lat),
    )
}

/// The spherical quad between latitudes 0–15° and longitudes 0–15° of the
/// unit sphere: a planar trapezoid with its corners on the sphere.
pub fn sphere_facet() -> (Object, FaceId, SurfaceRef) {
    let d = 15f64.to_radians();
    let (o, f) = facet([
        sphere_pt(1.0, 0.0, 0.0),
        sphere_pt(1.0, 0.0, d),
        sphere_pt(1.0, d, d),
        sphere_pt(1.0, d, 0.0),
    ]);
    (
        o,
        f,
        SurfaceRef::Sphere {
            center: p3(0.0, 0.0, 0.0),
            radius: 1.0,
        },
    )
}

pub fn torus_pt(major: f64, minor: f64, phi: f64, psi: f64) -> Point3 {
    let rho = major + minor * trig::cos(psi);
    p3(
        rho * trig::cos(phi),
        rho * trig::sin(phi),
        minor * trig::sin(psi),
    )
}

/// The facet between meridians 0° and 15° and tube rows 0° and 30° of the
/// torus `major = 2, minor = 0.5` about +Z.
pub fn torus_facet() -> (Object, FaceId, SurfaceRef) {
    let (dphi, dpsi) = (15f64.to_radians(), 30f64.to_radians());
    let (o, f) = facet([
        torus_pt(2.0, 0.5, 0.0, 0.0),
        torus_pt(2.0, 0.5, dphi, 0.0),
        torus_pt(2.0, 0.5, dphi, dpsi),
        torus_pt(2.0, 0.5, 0.0, dpsi),
    ]);
    let s = SurfaceRef::Torus {
        center: p3(0.0, 0.0, 0.0),
        axis: Z,
        major_radius: 2.0,
        minor_radius: 0.5,
    };
    (o, f, s)
}

pub fn cone_pt(alpha: f64, theta: f64, t: f64) -> Point3 {
    let r = t * trig::tan(alpha);
    p3(r * trig::cos(theta), r * trig::sin(theta), t)
}

/// The trapezoid between generators 0° and 15° of the cone `apex = origin,
/// axis = +Z, α = 30°`, between `t = 1` and `t = 2`.
pub fn cone_facet() -> (Object, FaceId, SurfaceRef) {
    let (a, d) = (30f64.to_radians(), 15f64.to_radians());
    let (o, f) = facet([
        cone_pt(a, 0.0, 1.0),
        cone_pt(a, d, 1.0),
        cone_pt(a, d, 2.0),
        cone_pt(a, 0.0, 2.0),
    ]);
    let s = SurfaceRef::Cone {
        apex: p3(0.0, 0.0, 0.0),
        axis: Z,
        half_angle: a,
    };
    (o, f, s)
}

/// A wall facet of a 24-gon of radius 1 about +Z, between z = 0 and 1.
pub fn cylinder_facet() -> (Object, FaceId, SurfaceRef) {
    let d = PI / 12.0;
    let (o, f) = facet([
        p3(1.0, 0.0, 0.0),
        p3(trig::cos(d), trig::sin(d), 0.0),
        p3(trig::cos(d), trig::sin(d), 1.0),
        p3(1.0, 0.0, 1.0),
    ]);
    let s = SurfaceRef::Cylinder {
        axis_point: p3(0.0, 0.0, 0.0),
        axis: Z,
        radius: 1.0,
    };
    (o, f, s)
}

/// A closed UV sphere of `n_lat` latitude bands × `n_lon` longitudes,
/// quads split into triangles (pole bands are triangles), radius `r`,
/// every coordinate rounded through f32, built as an IMPORTED object.
/// `degenerate` triangles (picked by a fixed-seed LCG) are collapsed to
/// zero area by moving one of their vertices onto the midpoint of the
/// opposite edge.
pub fn imported_sphere(n_lat: usize, n_lon: usize, r: f64, degenerate: usize) -> Object {
    let (pts, tris) = sphere_soup(n_lat, n_lon, r, degenerate);
    Object::from_polygons_import_for_test(&pts, &tris).expect("imported sphere builds")
}

/// [`imported_sphere`] with every vertex moved along its radius by up to
/// `amp` of the radius either way (a fixed hash of its index, so the same
/// every run), then rounded through f32: a scan's noise.
pub fn noisy_sphere(n_lat: usize, n_lon: usize, r: f64, amp: f64) -> Object {
    let (mut pts, tris) = sphere_soup(n_lat, n_lon, r, 0);
    for (i, p) in pts.iter_mut().enumerate() {
        let h = (trig::sin(i as f64 * 12.9898) * 43758.5453).fract();
        let k = 1.0 + amp * (h - 0.5) * 2.0;
        *p = p3(
            (p.x * k) as f32 as f64,
            (p.y * k) as f32 as f64,
            (p.z * k) as f32 as f64,
        );
    }
    Object::from_polygons_import_for_test(&pts, &tris).expect("noisy sphere builds")
}

/// The raw soup of [`imported_sphere`].
pub fn sphere_soup(
    n_lat: usize,
    n_lon: usize,
    r: f64,
    degenerate: usize,
) -> (Vec<Point3>, Vec<Vec<usize>>) {
    let f32r = |v: f64| v as f32 as f64;
    let mut pts = vec![p3(0.0, 0.0, f32r(-r))];
    for i in 1..n_lat {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / n_lat as f64;
        for j in 0..n_lon {
            let lon = std::f64::consts::TAU * j as f64 / n_lon as f64;
            pts.push(p3(
                f32r(r * trig::cos(lat) * trig::cos(lon)),
                f32r(r * trig::cos(lat) * trig::sin(lon)),
                f32r(r * trig::sin(lat)),
            ));
        }
    }
    pts.push(p3(0.0, 0.0, f32r(r)));
    let top = pts.len() - 1;
    let ring = |i: usize, j: usize| 1 + (i - 1) * n_lon + (j % n_lon);
    let mut tris: Vec<Vec<usize>> = Vec::new();
    for j in 0..n_lon {
        tris.push(vec![0, ring(1, j + 1), ring(1, j)]);
    }
    for i in 1..n_lat - 1 {
        for j in 0..n_lon {
            let (a, b, c, d) = (
                ring(i, j),
                ring(i, j + 1),
                ring(i + 1, j + 1),
                ring(i + 1, j),
            );
            tris.push(vec![a, b, c]);
            tris.push(vec![a, c, d]);
        }
    }
    for j in 0..n_lon {
        tris.push(vec![ring(n_lat - 1, j), ring(n_lat - 1, j + 1), top]);
    }
    // Collapse `degenerate` triangles (deterministic LCG picks).
    let mut seed: u64 = 0x5eed_1234_abcd_0001;
    for _ in 0..degenerate {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let t = n_lon + (seed >> 33) as usize % (tris.len() - 2 * n_lon);
        let [a, b, c] = [tris[t][0], tris[t][1], tris[t][2]];
        let m = pts[b] + (pts[c] - pts[b]) * 0.5;
        pts[a] = m;
    }
    (pts, tris)
}

/// Adds an analytic arc chain on the ground through `a`, `mid` and `b`
/// (`n` facets, ends exactly `a` and `b`), as the Arc tool draws one from a
/// bulge point.
pub fn add_arc_through(s: &mut Sketch, a: (f64, f64), mid: (f64, f64), b: (f64, f64), n: usize) {
    let (ax, ay, mx, my, bx, by) = (a.0, a.1, mid.0, mid.1, b.0, b.1);
    let d = 2.0 * (ax * (my - by) + mx * (by - ay) + bx * (ay - my));
    let sq = |x: f64, y: f64| x * x + y * y;
    let cx = (sq(ax, ay) * (my - by) + sq(mx, my) * (by - ay) + sq(bx, by) * (ay - my)) / d;
    let cy = (sq(ax, ay) * (bx - mx) + sq(mx, my) * (ax - bx) + sq(bx, by) * (mx - ax)) / d;
    let r = ((ax - cx).powi(2) + (ay - cy).powi(2)).sqrt();
    let a0 = trig::atan2(ay - cy, ax - cx);
    let am = trig::atan2(my - cy, mx - cx);
    let mut a1 = trig::atan2(by - cy, bx - cx);
    // Sweep through the bulge point.
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(TAU);
    let sweep = if ccw(a0, am) <= ccw(a0, a1) {
        ccw(a0, a1)
    } else {
        a1 = a0 - (TAU - ccw(a0, a1));
        a1 - a0
    };
    s.begin_curve_with(CurveGeom {
        center: p3(cx, cy, 0.0),
        radius: r,
    })
    .unwrap();
    let pt = |i: usize| {
        if i == 0 {
            p3(ax, ay, 0.0)
        } else if i == n {
            p3(bx, by, 0.0)
        } else {
            let t = a0 + sweep * i as f64 / n as f64;
            p3(cx + r * trig::cos(t), cy + r * trig::sin(t), 0.0)
        }
    };
    for i in 0..n {
        s.add_segment(pt(i), pt(i + 1)).unwrap();
    }
    s.end_curve();
}

/// A fan blade 6 mm thick: a straight-sided plan whose tip is an arc
/// through `(tip_x, 0.02)`, extruded 6 mm. `tip_x = 1.3` makes the tip a
/// tangent semicircle; any other value kinks it where the arc meets the
/// sides, as a bulge placed by eye does.
pub fn fan_blade(tip_x: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    s.add_segment(p3(0.3, -0.08, 0.0), p3(1.2, -0.08, 0.0))
        .unwrap();
    add_arc_through(&mut s, (1.2, -0.08), (tip_x, 0.02), (1.2, 0.12), 12);
    s.add_segment(p3(1.2, 0.12, 0.0), p3(0.3, 0.12, 0.0))
        .unwrap();
    s.add_segment(p3(0.3, 0.12, 0.0), p3(0.3, -0.08, 0.0))
        .unwrap();
    extrude_only_region(&s, 0.006)
}

/// A dome by Revolve: a quarter disc of radius 0.5 (an `n_arc`-facet arc
/// and two lines) turned a full turn about its straight side in `segments`
/// steps, so the sphere meets a flat base.
pub fn revolved_dome(n_arc: usize, segments: u32) -> Object {
    use kernel::document::{RevolveProfile, RevolveSpec};
    let xz = Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
    let mut doc = kernel::Document::new();
    let s = doc.add_sketch(xz);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        sk.begin_curve_with(CurveGeom {
            center: p3(0.0, 0.0, 0.0),
            radius: 0.5,
        })
        .unwrap();
        let at = |k: usize| {
            let a = (PI / 2.0) * k as f64 / n_arc as f64;
            p3(0.5 * trig::cos(a), 0.0, 0.5 * trig::sin(a))
        };
        for k in 0..n_arc {
            sk.add_segment(at(k), at(k + 1)).unwrap();
        }
        sk.end_curve();
        sk.add_segment(p3(0.0, 0.0, 0.5), p3(0.0, 0.0, 0.0))
            .unwrap();
        sk.add_segment(p3(0.0, 0.0, 0.0), p3(0.5, 0.0, 0.0))
            .unwrap();
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (id, _) = doc
        .revolve(
            RevolveProfile::Region { sketch: s, region },
            &RevolveSpec {
                axis_point: p3(0.0, 0.0, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                angle: TAU,
                segments,
            },
        )
        .unwrap();
    doc.object(id).unwrap().clone()
}

/// A boat hull by Loft through three 24-gon circles (radii 0.35, 0.58,
/// 0.6 at z = 0, 0.5, 1), flat deck at z = 1.
pub fn lofted_hull() -> Object {
    use kernel::loft::{LoftProfile, LoftSpec};
    let ring = |r: f64, z: f64| LoftProfile {
        points: (0..24)
            .map(|k| {
                let a = TAU * k as f64 / 24.0;
                p3(r * trig::cos(a), r * trig::sin(a), z)
            })
            .collect(),
        curves: vec![
            Some(CurveGeom {
                center: p3(0.0, 0.0, z),
                radius: r,
            });
            24
        ],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    Object::from_loft(&LoftSpec {
        profiles: vec![ring(0.35, 0.0), ring(0.58, 0.5), ring(0.6, 1.0)],
        apex: None,
        rails: Vec::new(),
        stations: None,
    })
    .unwrap()
}

/// A cylinder of `n` claimed wall facets built straight from its polygons
/// (a sketch's circle costs the square of `n` to build), its walls claiming
/// the cylinder [`cylinder`] claims: the same faces, edges and chains.
pub fn claimed_cylinder(n: usize) -> Object {
    let claim = {
        let c = cylinder(0.1, 0.1, 24);
        c.faces().values().find_map(|f| f.surface).unwrap()
    };
    let ring = |z: f64| {
        (0..n).map(move |i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            p3(0.1 * kernel::trig::cos(a), 0.1 * kernel::trig::sin(a), z)
        })
    };
    let points: Vec<Point3> = ring(0.0).chain(ring(0.1)).collect();
    let mut faces: Vec<Vec<usize>> = vec![(0..n).collect(), (n..2 * n).collect()];
    for i in 0..n {
        let j = (i + 1) % n;
        faces.push(vec![i, j, n + j, n + i]);
    }
    let mut obj = convex_solid(&points, &faces);
    let walls: Vec<kernel::FaceId> = obj
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z.abs() < 0.5)
        .map(|(id, _)| id)
        .collect();
    for w in walls {
        obj.set_face_surface_for_test(w, Some(claim));
    }
    obj
}

/// The sharp edges of `obj` with both ends at height `z`.
pub fn sharp_edges_at_height(obj: &Object, z: f64) -> Vec<EdgeId> {
    obj.sharp_edges()
        .into_iter()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            (p.z - z).abs() <= LOOKUP_EPS && (q.z - z).abs() <= LOOKUP_EPS
        })
        .collect()
}

/// The pipe tee as the app builds it: a Circle (r 0.1, 32 sides, its first
/// vertex at angle `phase_main`) pushed up 0.8 at (5, 0); a second Circle
/// (r 0.07, 28 sides, first vertex at `phase_branch`) pushed up 0.8 at
/// (5, 3), turned a quarter about x and moved so it runs along +y from
/// just inside the main pipe at z 0.4; then Union. Returns the document,
/// the union and the junction loop as the Fillet tool picks it (the logical
/// edge of one junction edge).
pub fn app_pipe_tee(
    phase_main: f64,
    phase_branch: f64,
) -> (kernel::Document, kernel::ObjectId, Vec<EdgeId>) {
    app_pipe_tee_sized(0.07, 28, phase_main, phase_branch)
}

/// [`app_pipe_tee`] with the branch's radius and side count given: an
/// equal-radius tee is `app_pipe_tee_sized(0.1, 32, ..)`.
pub fn app_pipe_tee_sized(
    branch_r: f64,
    branch_sides: usize,
    phase_main: f64,
    phase_branch: f64,
) -> (kernel::Document, kernel::ObjectId, Vec<EdgeId>) {
    let mut doc = kernel::Document::new();
    let mut pipe = |c: Point3, r: f64, n: usize, phase: f64| {
        let s = doc.add_sketch(ground());
        {
            let sk = doc.sketch_mut(s).unwrap();
            sk.begin_curve_with(CurveGeom {
                center: c,
                radius: r,
            })
            .unwrap();
            let at = |i: usize| {
                let a = phase + TAU * (i % n) as f64 / n as f64;
                p3(c.x + r * trig::cos(a), c.y + r * trig::sin(a), 0.0)
            };
            for i in 0..n {
                sk.add_segment(at(i), at(i + 1)).unwrap();
            }
            sk.end_curve();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], 0.8).unwrap().0
    };
    let main = pipe(p3(5.0, 0.0, 0.0), 0.1, 32, phase_main);
    let branch = pipe(p3(5.0, 3.0, 0.0), branch_r, branch_sides, phase_branch);
    let lay = Transform::translation(Vec3::new(-5.0, -3.0, 0.0))
        .then(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -PI / 2.0).unwrap())
        .then(&Transform::translation(Vec3::new(5.0, 0.0006, 0.4)));
    doc.transform_object(branch, &lay).unwrap();
    let (tee, _) = doc.boolean(BooleanOp::Union, main, branch).unwrap();
    let obj = doc.object(tee).unwrap();
    let junction = obj
        .sharp_edges()
        .into_iter()
        .find(|&e| {
            let (a, b) = obj.edge_faces_for_test(e).unwrap();
            matches!(
                (obj.faces()[a].surface, obj.faces()[b].surface),
                (Some(SurfaceRef::Cylinder { axis: p, .. }), Some(SurfaceRef::Cylinder { axis: q, .. }))
                    if p.dot(q).abs() < 0.5
            )
        })
        .expect("a junction edge");
    let picked = obj.logical_edge(junction, false);
    (doc, tee, picked)
}

/// The scoreboard's grooved knob (`corpus/curves/grooved-knob.json`) as the
/// API builds it: a 20 mm x 25 mm rectangle in the y = 0 plane revolved a
/// full turn about z in 24 steps, then twelve 3 mm, 48-sided circles on the
/// 20 mm circle, at 3.75 degrees and every 30 degrees on from there, each
/// pushed 29 mm up from z = -2 mm, subtracted in one operation; then the
/// whole knob turned by `turn` radians about z. Returns the knob and its
/// twelve outer top-rim chains (the outer wall's top edges between the
/// grooves, two edges each), in order of angle.
pub fn grooved_knob(turn: f64) -> (Object, Vec<Vec<EdgeId>>) {
    use kernel::NodeId;
    use kernel::document::{RevolveProfile, RevolveSpec};
    let xz = Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
    let mut doc = kernel::Document::new();
    let s = doc.add_sketch(xz);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let ring = [
            p3(0.0, 0.0, 0.0),
            p3(0.02, 0.0, 0.0),
            p3(0.02, 0.0, 0.025),
            p3(0.0, 0.0, 0.025),
        ];
        for i in 0..4 {
            sk.add_segment(ring[i], ring[(i + 1) % 4]).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (knob, _) = doc
        .revolve(
            RevolveProfile::Region { sketch: s, region },
            &RevolveSpec {
                axis_point: p3(0.0, 0.0, 0.0),
                axis: Vec3::new(0.0, 0.0, 1.0),
                angle: TAU,
                segments: 24,
            },
        )
        .unwrap();
    let mut operands = vec![NodeId::Object(knob)];
    for k in 0..12 {
        let a = (3.75 + 30.0 * k as f64).to_radians();
        let c = p3(0.02 * trig::cos(a), 0.02 * trig::sin(a), -0.002);
        let plane = Plane::from_point_normal(c, Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let g = doc.add_sketch(plane);
        doc.begin_sketch_gesture(g).unwrap();
        {
            let sk = doc.sketch_mut(g).unwrap();
            sk.begin_curve_with(CurveGeom {
                center: c,
                radius: 0.003,
            })
            .unwrap();
            let at = |i: usize| {
                let b = TAU * (i % 48) as f64 / 48.0;
                p3(c.x + 0.003 * trig::cos(b), c.y + 0.003 * trig::sin(b), c.z)
            };
            for i in 0..48 {
                sk.add_segment(at(i), at(i + 1)).unwrap();
            }
            sk.end_curve();
        }
        doc.end_sketch_gesture(g).unwrap();
        let regions = doc.extrudable_regions(g).unwrap();
        let (groove, _) = doc.extrude_region(g, regions[0], 0.029).unwrap();
        operands.push(NodeId::Object(groove));
    }
    let (grooved, _) = doc
        .boolean_nodes_many(BooleanOp::Subtract, &operands)
        .unwrap();
    let NodeId::Object(grooved) = grooved else {
        panic!("the subtraction makes an object");
    };
    if turn != 0.0 {
        doc.transform_object(
            grooved,
            &Transform::rotation(Vec3::new(0.0, 0.0, 1.0), turn).unwrap(),
        )
        .unwrap();
    }
    let obj = doc.object(grooved).unwrap().clone();
    // The outer wall's top edges: on a chord of the revolved 24-gon at the
    // top, turned with the knob.
    let corner = |k: i32| {
        let a = (15.0 * k as f64).to_radians() + turn;
        p3(0.02 * trig::cos(a), 0.02 * trig::sin(a), 0.025)
    };
    let off = |p: Point3, a: Point3, b: Point3| {
        let d = b - a;
        let t = ((p - a).dot(d) / d.dot(d)).clamp(0.0, 1.0);
        (p - (a + d * t)).length()
    };
    let mut rim: Vec<(EdgeId, Point3, Point3)> = obj
        .edges()
        .keys()
        .filter_map(|e| {
            let (a, b) = obj.edge_endpoints(e)?;
            (0..24)
                .any(|k| {
                    off(a, corner(k), corner(k + 1)) <= LOOKUP_EPS
                        && off(b, corner(k), corner(k + 1)) <= LOOKUP_EPS
                })
                .then_some((e, a, b))
        })
        .collect();
    // Two rim edges are one chain where they share an end.
    let mut chains: Vec<Vec<(EdgeId, Point3, Point3)>> = Vec::new();
    while let Some(seed) = rim.pop() {
        let mut chain = vec![seed];
        while let Some(i) = rim.iter().position(|&(_, a, b)| {
            chain.iter().any(|&(_, p, q)| {
                [p, q]
                    .iter()
                    .any(|x| x.approx_eq(a, LOOKUP_EPS) || x.approx_eq(b, LOOKUP_EPS))
            })
        }) {
            chain.push(rim.remove(i));
        }
        chains.push(chain);
    }
    let angle = |c: &[(EdgeId, Point3, Point3)]| {
        let (_, a, b) = c[0];
        trig::atan2(a.y + b.y, a.x + b.x)
    };
    chains.sort_by(|x, y| angle(x).total_cmp(&angle(y)));
    let chains = chains
        .into_iter()
        .map(|c| c.into_iter().map(|(e, _, _)| e).collect())
        .collect();
    (obj, chains)
}
