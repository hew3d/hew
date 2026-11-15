//! WP16: the offset engine and Shell.

mod common;

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

use common::curves::*;
use kernel::shell::ShellSpec;
use kernel::{FaceId, Object, Point3, ShapeError, SurfaceRef, Vec3, WatertightState};

const T: f64 = 0.01;

fn shell(obj: &Object, open: Vec<FaceId>, thickness: f64) -> Result<Object, ShapeError> {
    obj.shelled(&ShellSpec { open, thickness })
}

fn top_of(obj: &Object) -> FaceId {
    face_with_normal(obj, Vec3::new(0.0, 0.0, 1.0))
}

fn bottom_of(obj: &Object) -> FaceId {
    face_with_normal(obj, Vec3::new(0.0, 0.0, -1.0))
}

/// Every claimed face's vertices lie on its claim within `PLANE_DIST`, and
/// the claim's plane invariants hold (the strict stamping predicate).
fn assert_claims_hold(obj: &Object) {
    for (f, face) in obj.faces() {
        let Some(s) = face.surface else { continue };
        let pts: Vec<Point3> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .flat_map(|l| obj.loop_positions(l).collect::<Vec<_>>())
            .collect();
        for &p in &pts {
            assert!(
                s.distance(p).abs() <= kernel::tol::PLANE_DIST,
                "face {f:?}: vertex {p:?} is {:e} off its claim {s:?}",
                s.distance(p)
            );
        }
        assert!(s.holds_on(&pts, &face.plane, kernel::tol::PLANE_DIST));
    }
}

fn claimed(obj: &Object) -> usize {
    obj.faces().values().filter(|f| f.surface.is_some()).count()
}

/// The shell keeps every claim: each claimed wall's offset claims too
/// (none dropped by `holds_on`). `open_claimed`: claimed faces left open.
fn assert_inner_claims_kept(before: &Object, after: &Object, open_claimed: usize) {
    assert_eq!(
        claimed(after),
        2 * (claimed(before) - open_claimed),
        "every inner face keeps its offset claim"
    );
}

fn shells(obj: &Object) -> usize {
    obj.shells().len()
}

fn radii(obj: &Object) -> BTreeMap<&'static str, Vec<f64>> {
    let mut out: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for face in obj.faces().values() {
        let (k, r) = match face.surface {
            Some(SurfaceRef::Cylinder { radius, .. }) => ("cylinder", radius),
            Some(SurfaceRef::Sphere { radius, .. }) => ("sphere", radius),
            Some(SurfaceRef::Torus { minor_radius, .. }) => ("torus", minor_radius),
            _ => continue,
        };
        let v = out.entry(k).or_default();
        if !v.iter().any(|&x| (x - r).abs() < 1e-12) {
            v.push(r);
        }
    }
    for v in out.values_mut() {
        v.sort_by(f64::total_cmp);
    }
    out
}

// ─────────────────────────────────────────────────────────────── fixtures

/// A box of half-extents `(a, b, c)` with every edge and corner rounded to
/// radius `r` (`n` facets per quarter turn): the Minkowski sum of the box
/// and a faceted sphere. Corners claim spheres, edges cylinders; the flat
/// faces are unclaimed.
fn rounded_box(a: f64, b: f64, c: f64, r: f64, n: usize) -> Object {
    let quadrant = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
    // Rows: south hemisphere (lat index 0..=n, z−), north (n..=2n, z+).
    let rows: Vec<(usize, f64)> = (0..=n)
        .map(|i| (i, -1.0))
        .chain((n..=2 * n).map(|i| (i, 1.0)))
        .collect();
    let cols: Vec<(usize, usize)> = (0..4)
        .flat_map(|q| (q * n..=(q + 1) * n).map(move |j| (j, q)))
        .collect();
    let mut pts: Vec<Point3> = Vec::new();
    let mut index: BTreeMap<(usize, usize, usize), usize> = BTreeMap::new();
    let mut grid = vec![vec![0usize; cols.len()]; rows.len()];
    for (ri, &(i, sz)) in rows.iter().enumerate() {
        let lat = -FRAC_PI_2 + i as f64 * PI / (2 * n) as f64;
        for (ci, &(j, q)) in cols.iter().enumerate() {
            let (sx, sy) = quadrant[q];
            let lon = j as f64 * FRAC_PI_2 / n as f64;
            let dir = if i == 0 {
                Vec3::new(0.0, 0.0, -1.0)
            } else if i == 2 * n {
                Vec3::new(0.0, 0.0, 1.0)
            } else {
                Vec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
            };
            // Pole points coincide within a quadrant.
            let k = if i == 0 || i == 2 * n {
                (ri, q, usize::MAX)
            } else {
                (ri, ci, 0)
            };
            let id = *index.entry(k).or_insert_with(|| {
                pts.push(p3(
                    sx * a + r * dir.x,
                    sy * b + r * dir.y,
                    sz * c + r * dir.z,
                ));
                pts.len() - 1
            });
            grid[ri][ci] = id;
        }
    }
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut claims: Vec<Option<SurfaceRef>> = Vec::new();
    let nc = cols.len();
    for k in 0..rows.len() - 1 {
        for m in 0..nc {
            let m1 = (m + 1) % nc;
            let mut f = vec![grid[k][m], grid[k][m1], grid[k + 1][m1], grid[k + 1][m]];
            f.dedup();
            if f.first() == f.last() {
                f.pop();
            }
            if f.len() < 3 {
                continue;
            }
            let cross_z = rows[k].1 != rows[k + 1].1;
            let (q0, q1) = (cols[m].1, cols[m1].1);
            let cross_q = q0 != q1;
            let (sx, sy) = quadrant[q0];
            let sz = rows[k].1;
            let corner = p3(sx * a, sy * b, sz * c);
            let claim = match (cross_z, cross_q) {
                (false, false) => Some(SurfaceRef::Sphere {
                    center: corner,
                    radius: r,
                }),
                (true, false) => Some(SurfaceRef::Cylinder {
                    axis_point: p3(sx * a, sy * b, 0.0),
                    axis: Vec3::new(0.0, 0.0, 1.0),
                    radius: r,
                }),
                (false, true) => {
                    let (tx, ty) = quadrant[q1];
                    let other = p3(tx * a, ty * b, sz * c);
                    Some(SurfaceRef::Cylinder {
                        axis_point: corner,
                        axis: (other - corner).normalized().unwrap(),
                        radius: r,
                    })
                }
                (true, true) => None,
            };
            faces.push(f);
            claims.push(claim);
        }
    }
    // The two flat caps: the four pole points of each pole row.
    for (ri, rev) in [(0usize, true), (rows.len() - 1, false)] {
        let mut ring: Vec<usize> = (0..4).map(|q| grid[ri][q * (n + 1)]).collect();
        if rev {
            ring.reverse();
        }
        faces.push(ring);
        claims.push(None);
    }
    let mut obj = Object::from_polygons(&pts, &faces).expect("rounded box builds");
    if signed_volume(&obj) < 0.0 {
        let flipped: Vec<Vec<usize>> = faces
            .iter()
            .map(|f| f.iter().rev().copied().collect())
            .collect();
        obj = Object::from_polygons(&pts, &flipped).unwrap();
    }
    stamp(obj, &faces, &pts, &claims)
}

/// Writes `claims[i]` on the face whose vertex set is `faces[i]`.
fn stamp(
    mut obj: Object,
    faces: &[Vec<usize>],
    pts: &[Point3],
    claims: &[Option<SurfaceRef>],
) -> Object {
    let ids: Vec<(FaceId, Vec<Point3>)> = obj
        .faces()
        .iter()
        .map(|(id, f)| (id, obj.loop_positions(f.outer_loop).collect()))
        .collect();
    for (f, claim) in faces.iter().zip(claims) {
        let Some(s) = claim else { continue };
        let want: Vec<Point3> = f.iter().map(|&i| pts[i]).collect();
        let id = ids
            .iter()
            .find(|(_, ring)| {
                ring.len() == want.len()
                    && want
                        .iter()
                        .all(|p| ring.iter().any(|q| q.approx_eq(*p, 1e-12)))
            })
            .map(|(id, _)| *id)
            .expect("stamped face exists");
        obj.set_face_surface_for_test(id, Some(*s));
    }
    obj.validate().expect("stamped fixture validates");
    obj
}

/// A surface of revolution about +z: `profile` is (radius, z) from the
/// bottom center to the top center (first and last radius 0), `claims[k]`
/// the claim of the band between profile points k and k+1; `m` facets
/// around.
fn revolved(profile: &[(f64, f64)], claims: &[Option<SurfaceRef>], m: usize) -> Object {
    let mut pts: Vec<Point3> = Vec::new();
    let mut rings: Vec<Vec<usize>> = Vec::new();
    for &(rho, z) in profile {
        if rho == 0.0 {
            pts.push(p3(0.0, 0.0, z));
            rings.push(vec![pts.len() - 1; m]);
        } else {
            let ring = (0..m)
                .map(|j| {
                    let t = j as f64 * 2.0 * PI / m as f64;
                    pts.push(p3(rho * t.cos(), rho * t.sin(), z));
                    pts.len() - 1
                })
                .collect();
            rings.push(ring);
        }
    }
    let mut faces = Vec::new();
    let mut fclaims = Vec::new();
    for k in 0..rings.len() - 1 {
        let (lo, hi) = (&rings[k], &rings[k + 1]);
        let flat_lo = lo.iter().all(|&i| i == lo[0]);
        let flat_hi = hi.iter().all(|&i| i == hi[0]);
        // A band from a center point to a ring in one plane is a flat cap.
        if flat_lo && (profile[k].1 == profile[k + 1].1) {
            faces.push(hi.iter().rev().copied().collect());
            fclaims.push(claims[k]);
            continue;
        }
        if flat_hi && (profile[k].1 == profile[k + 1].1) {
            faces.push(lo.clone());
            fclaims.push(claims[k]);
            continue;
        }
        for j in 0..m {
            let j1 = (j + 1) % m;
            let mut f = vec![lo[j], lo[j1], hi[j1], hi[j]];
            f.dedup();
            if f.first() == f.last() {
                f.pop();
            }
            faces.push(f);
            fclaims.push(claims[k]);
        }
    }
    // Drop the center points only flat caps would have used.
    let mut used = vec![false; pts.len()];
    for f in &faces {
        for &i in f {
            used[i] = true;
        }
    }
    let mut remap = vec![usize::MAX; pts.len()];
    let mut kept = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        if used[i] {
            remap[i] = kept.len();
            kept.push(*p);
        }
    }
    let pts = kept;
    let faces: Vec<Vec<usize>> = faces
        .iter()
        .map(|f| f.iter().map(|&i| remap[i]).collect())
        .collect();
    let obj = Object::from_polygons(&pts, &faces).expect("revolved solid builds");
    assert!(signed_volume(&obj) > 0.0, "fixture winds outward");
    stamp(obj, &faces, &pts, &fclaims)
}

const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

/// A rod of radius `r` and straight length `l` standing on the ground,
/// its top a hemisphere (`n` latitude rows), `m` facets around.
fn capsule_rod(r: f64, l: f64, n: usize, m: usize) -> Object {
    let mut profile = vec![(0.0, 0.0), (r, 0.0), (r, l)];
    let cyl = Some(SurfaceRef::Cylinder {
        axis_point: p3(0.0, 0.0, 0.0),
        axis: Z,
        radius: r,
    });
    let sph = Some(SurfaceRef::Sphere {
        center: p3(0.0, 0.0, l),
        radius: r,
    });
    let mut claims = vec![None, cyl];
    for i in 1..=n {
        let lat = i as f64 * FRAC_PI_2 / n as f64;
        profile.push(if i == n {
            (0.0, l + r)
        } else {
            (r * lat.cos(), l + r * lat.sin())
        });
        claims.push(sph);
    }
    revolved(&profile, &claims, m)
}

/// A spherical dome: the part of a sphere (radius `r`, center on the axis
/// at height `−h0`) above the ground plane, which cuts it at an angle.
fn dome(r: f64, h0: f64, n: usize, m: usize) -> Object {
    let base = (r * r - h0 * h0).sqrt();
    let lat0 = (h0 / r).asin();
    let mut profile = vec![(0.0, 0.0), (base, 0.0)];
    let sph = Some(SurfaceRef::Sphere {
        center: p3(0.0, 0.0, -h0),
        radius: r,
    });
    let mut claims = vec![None];
    for i in 1..=n {
        let lat = lat0 + i as f64 * (FRAC_PI_2 - lat0) / n as f64;
        profile.push(if i == n {
            (0.0, r - h0)
        } else {
            (r * lat.cos(), r * lat.sin() - h0)
        });
        claims.push(sph);
    }
    revolved(&profile, &claims, m)
}

/// A cylinder (radius `r`, height `h`) whose top rim is chamfered `c` at
/// 45°: a cone band between the wall and a smaller top disc.
fn chamfered_cylinder(r: f64, h: f64, c: f64, m: usize) -> Object {
    let profile = [(0.0, 0.0), (r, 0.0), (r, h - c), (r - c, h), (0.0, h)];
    let claims = [
        None,
        Some(SurfaceRef::Cylinder {
            axis_point: p3(0.0, 0.0, 0.0),
            axis: Z,
            radius: r,
        }),
        Some(SurfaceRef::Cone {
            apex: p3(0.0, 0.0, h - c + r),
            axis: Vec3::new(0.0, 0.0, -1.0),
            half_angle: std::f64::consts::FRAC_PI_4,
        }),
        None,
    ];
    revolved(&profile, &claims, m)
}

/// A closed hull through five stations of varying size and twist, its
/// warped side quads split into triangles joined by soft edges: generated,
/// unclaimed, smooth — the shape a loft makes.
fn soft_hull() -> Object {
    let m = 8;
    let stations = [
        (0.0, 0.05, 0.0),
        (0.03, 0.07, 0.1),
        (0.06, 0.06, 0.2),
        (0.09, 0.04, 0.3),
        (0.12, 0.05, 0.4),
    ];
    let mut pts = Vec::new();
    for &(z, rho, twist) in &stations {
        for j in 0..m {
            let t = j as f64 * 2.0 * PI / m as f64 + twist;
            pts.push(p3(rho * t.cos(), rho * t.sin(), z));
        }
    }
    let mut faces: Vec<Vec<usize>> = Vec::new();
    for k in 0..stations.len() - 1 {
        for j in 0..m {
            let j1 = (j + 1) % m;
            let (a, b, c, d) = (k * m + j, k * m + j1, (k + 1) * m + j1, (k + 1) * m + j);
            faces.push(vec![a, b, c]);
            faces.push(vec![a, c, d]);
        }
    }
    faces.push((0..m).rev().collect());
    faces.push(((stations.len() - 1) * m..stations.len() * m).collect());
    let mut obj = Object::from_polygons(&pts, &faces).expect("hull builds");
    let side: Vec<_> = obj
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = obj.edge_endpoints(e).unwrap();
            !(p.z == q.z && (p.z == 0.0 || p.z == 0.12))
        })
        .collect();
    for e in side {
        obj.set_edge_soft_for_test(e, true);
    }
    obj.validate().unwrap();
    obj
}

/// A pyramid over the square `[-1, 1]²` whose apex sits off center, so its
/// four side planes offset do not meet in one point.
fn irregular_pyramid() -> Object {
    let pts = [
        p3(-1.0, -1.0, 0.0),
        p3(1.0, -1.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(-1.0, 1.0, 0.0),
        p3(0.4, 0.15, 1.3),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![0, 1, 4],
        vec![1, 2, 4],
        vec![2, 3, 4],
        vec![3, 0, 4],
    ];
    Object::from_polygons(&pts, &faces).unwrap()
}

// ────────────────────────────────────────────────────────────────── specs

#[test]
fn open_box_inward() {
    let b = unit_box();
    let r = shell(&b, vec![top_of(&b)], T).unwrap();
    assert_valid_solid(&r);
    // 5 outer walls, 5 inner walls, and one rim face around the opening.
    assert_eq!(r.faces().len(), 11);
    let v = signed_volume(&r);
    assert!((v - (1.0 - 0.98 * 0.98 * 0.99)).abs() < 1e-9, "volume {v}");
    let rim: Vec<_> = r
        .faces()
        .values()
        .filter(|f| !f.inner_loops.is_empty())
        .collect();
    assert_eq!(
        rim.len(),
        1,
        "the rim is one face with the opening as its hole"
    );
    for l in std::iter::once(rim[0].outer_loop).chain(rim[0].inner_loops.iter().copied()) {
        assert!(r.loop_positions(l).all(|p| (p.z - 1.0).abs() < 1e-12));
    }
    // The outer walls keep their ids.
    for (id, _) in b.faces().iter().filter(|(id, _)| *id != top_of(&b)) {
        assert!(r.faces().contains_key(id));
    }
}

#[test]
fn hollow_box_two_shells_validates() {
    let b = unit_box();
    let r = shell(&b, Vec::new(), T).unwrap();
    assert_valid_solid(&r);
    assert_eq!(shells(&r), 2);
    let v = signed_volume(&r);
    assert!((v - (1.0 - 0.98f64.powi(3))).abs() < 1e-9, "volume {v}");
}

#[test]
fn outward_keeps_inner_size() {
    let b = unit_box();
    let r = shell(&b, vec![top_of(&b)], -T).unwrap();
    assert_valid_solid(&r);
    let v = signed_volume(&r);
    assert!((v - (1.02 * 1.02 * 1.01 - 1.0)).abs() < 1e-9, "volume {v}");
    // Every original wall but the top reappears with the same corners,
    // facing the other way.
    for (id, f) in b.faces() {
        if id == top_of(&b) {
            continue;
        }
        let ring: Vec<Point3> = b.loop_positions(f.outer_loop).collect();
        let n = f.plane.normal();
        assert!(
            r.faces().values().any(|g| {
                let other: Vec<Point3> = r.loop_positions(g.outer_loop).collect();
                g.plane.normal().approx_eq(n * -1.0, 1e-12)
                    && other.len() == ring.len()
                    && ring
                        .iter()
                        .all(|p| other.iter().any(|q| q.approx_eq(*p, 1e-12)))
            }),
            "inner copy of {id:?}"
        );
    }
}

#[test]
fn cylinder_cup() {
    let (rad, t) = (0.05, 0.005);
    let c = cylinder(rad, 0.1, 48);
    let r = shell(&c, vec![top_of(&c)], t).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&c, &r, 0);
    assert_eq!(radii(&r)["cylinder"].len(), 2);
    assert!((radii(&r)["cylinder"][0] - (rad - t)).abs() < 1e-15);
    // The rim: one annulus in the top plane.
    let rims: Vec<_> = r
        .faces()
        .values()
        .filter(|f| f.plane.normal().approx_eq(Z, 1e-12) && !f.inner_loops.is_empty())
        .collect();
    assert_eq!(rims.len(), 1);
    assert!(
        r.loop_positions(rims[0].outer_loop)
            .all(|p| (p.z - 0.1).abs() < 1e-12)
    );
}

#[test]
fn filleted_box_shell() {
    let (rad, t) = (0.03, 0.02);
    let b = rounded_box(0.05, 0.05, 0.05, rad, 4);
    let r = shell(&b, vec![top_of(&b)], t).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&b, &r, 0);
    let rs = radii(&r);
    assert_eq!(rs["sphere"].len(), 2);
    assert!((rs["sphere"][0] - (rad - t)).abs() < 1e-15);
    assert!((rs["cylinder"][0] - (rad - t)).abs() < 1e-15);
}

#[test]
fn loft_hull_shell() {
    let h = soft_hull();
    let r = shell(&h, Vec::new(), 0.002).unwrap();
    assert_valid_solid(&r);
    assert_eq!(shells(&r), 2);
    // The averaged vertex normals warp the inner quads' neighbors; every
    // inner face is planar (the triangles already were), and the inner
    // shell is as smooth as the outer.
    let soft = |o: &Object| o.edges().values().filter(|e| e.soft).count();
    assert_eq!(soft(&r), 2 * soft(&h));
}

#[test]
fn refined_vertices_lie_on_offset_claims() {
    // A rod with a hemispherical end, open at the flat bottom.
    let (rad, t) = (0.01, 0.002);
    let rod = capsule_rod(rad, 0.04, 6, 24);
    let r = shell(&rod, vec![bottom_of(&rod)], t).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&rod, &r, 0);
    let rs = radii(&r);
    assert!((rs["sphere"][0] - (rad - t)).abs() < 1e-15);
    assert!((rs["cylinder"][0] - (rad - t)).abs() < 1e-15);
    // Closed as well.
    let r = shell(&rod, Vec::new(), t).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&rod, &r, 0);

    // A dome cut by the ground at an angle: the base rim needs the
    // refinement (the tangent-plane solve alone sits off the offset
    // sphere).
    let d = dome(0.05, 0.02, 8, 32);
    let r = shell(&d, Vec::new(), 0.004).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&d, &r, 0);
    assert_eq!(radii(&r)["sphere"], vec![0.046, 0.05]);

    // A cone-chamfered rim: the inner cone claim verifies.
    let c = chamfered_cylinder(0.05, 0.08, 0.01, 32);
    let r = shell(&c, vec![bottom_of(&c)], 0.003).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    assert_inner_claims_kept(&c, &r, 0);
    let cones = r
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cone { .. })))
        .count();
    assert_eq!(cones, 64, "outer and inner chamfer bands both claim cones");
}

#[test]
fn too_thick() {
    let b = unit_box();
    assert!(matches!(
        shell(&b, vec![top_of(&b)], 0.6),
        Err(ShapeError::WallTooThick { .. })
    ));
    let c = cylinder(0.05, 0.1, 48);
    assert!(matches!(
        shell(&c, vec![top_of(&c)], 0.06),
        Err(ShapeError::WallTooThick { .. })
    ));
}

#[test]
fn pyramid_apex_symmetric_ok() {
    let p = pyramid4();
    let r = shell(&p, vec![bottom_of(&p)], 0.05).unwrap();
    assert_valid_solid(&r);
    let r = shell(&p, Vec::new(), 0.05).unwrap();
    assert_valid_solid(&r);
    assert_eq!(shells(&r), 2);
}

#[test]
fn irregular_valence4_refused() {
    let p = irregular_pyramid();
    assert!(matches!(
        shell(&p, vec![bottom_of(&p)], 0.05),
        Err(ShapeError::UnsupportedShellCorner { .. })
    ));
}

#[test]
fn refusals_name_the_cause() {
    let b = unit_box();
    assert_eq!(
        shell(&b, Vec::new(), 0.0).err(),
        Some(ShapeError::AmountOutOfRange)
    );
    assert_eq!(
        shell(&b, Vec::new(), f64::NAN).err(),
        Some(ShapeError::AmountOutOfRange)
    );
    let all: Vec<FaceId> = b.faces().keys().collect();
    assert_eq!(shell(&b, all, T).err(), Some(ShapeError::AmountOutOfRange));
    let (open, f) = facet([
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
    ]);
    assert_eq!(open.watertight(), WatertightState::Open);
    assert_eq!(
        shell(&open, vec![f], T).err(),
        Some(ShapeError::ShapeNeedsSolid)
    );
    let stale = cylinder(0.05, 0.1, 48).faces().keys().last().unwrap();
    assert_eq!(
        shell(&b, vec![stale], T).err(),
        Some(ShapeError::ShapeDegenerate)
    );
}

// ─────────────────────────────────────────────────────────────── document

fn doc_box(doc: &mut kernel::Document) -> kernel::ObjectId {
    let ground = kernel::Plane::from_point_normal(p3(0.0, 0.0, 0.0), Z).unwrap();
    let s = doc.add_sketch(ground);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, r, 1.0).unwrap().0
}

#[test]
fn document_shell_is_one_undo_step_and_preview_commits_nothing() {
    let mut doc = kernel::Document::new();
    let o = doc_box(&mut doc);
    let top = top_of(doc.object(o).unwrap());
    let spec = ShellSpec {
        open: vec![top],
        thickness: T,
    };
    let before = doc.state_hash();
    let preview = doc.shell_preview(o, &spec).unwrap();
    assert_eq!(doc.state_hash(), before, "a preview changes nothing");
    let change = doc.shell(o, &spec).unwrap();
    assert!(change.objects_touched.contains(&o));
    let after = doc.state_hash();
    assert_eq!(
        doc.object(o).unwrap().faces().len(),
        preview.faces().len(),
        "the commit is the preview"
    );
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    doc.redo().unwrap();
    assert_eq!(
        doc.state_hash(),
        after,
        "redo reproduces the shell bit for bit"
    );
    // A refusal leaves the document as it was.
    let bad = ShellSpec {
        open: Vec::new(),
        thickness: 0.0,
    };
    assert_eq!(
        doc.shell(o, &bad).err(),
        Some(kernel::DocumentError::Shape(ShapeError::AmountOutOfRange))
    );
    assert_eq!(doc.state_hash(), after);
}

/// Opened faces that meet each other: walls stand only where an opened face
/// meets a kept one, and a corner whose faces are all open (no kept face, no
/// offset point) is no refusal. Panicked before (a missing offset point).
#[test]
fn opened_faces_that_meet_leave_no_wall_between_them() {
    let b = unit_box();
    let f = |x: f64, y: f64, z: f64| face_with_normal(&b, Vec3::new(x, y, z));
    let (top, bottom) = (f(0.0, 0.0, 1.0), f(0.0, 0.0, -1.0));
    let (front, back) = (f(0.0, -1.0, 0.0), f(0.0, 1.0, 0.0));
    let (left, right) = (f(-1.0, 0.0, 0.0), f(1.0, 0.0, 0.0));
    // Inward: each kept face's slab, the overlaps counted once.
    let cases: [(&str, Vec<FaceId>, f64); 4] = [
        // bottom 1·1·.01 + left .01·1·.99 + right same + back .98·.01·.99
        (
            "two adjacent",
            vec![top, front],
            0.01 + 2.0 * 0.0099 + 0.009702,
        ),
        // bottom + left + back, around the corner all three opened faces share
        (
            "three at a corner",
            vec![top, front, right],
            0.01 + 0.0099 + 0.009801,
        ),
        (
            "all but the bottom",
            vec![top, front, right, left, back],
            0.01,
        ),
        // a tube: front and back 1·.01·1, left and right .01·.98·1
        ("opposite", vec![top, bottom], 2.0 * 0.01 + 2.0 * 0.0098),
    ];
    for (name, open, want) in cases {
        let r = shell(&b, open.clone(), T).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_valid_solid(&r);
        let v = signed_volume(&r);
        assert!((v - want).abs() < 1e-9, "{name}: volume {v}, want {want}");
        // Outward keeps the inside size: a valid solid as well.
        let o = shell(&b, open, -T).unwrap_or_else(|e| panic!("{name} outward: {e:?}"));
        assert_valid_solid(&o);
    }
}

/// A box with a square pocket in its top. Opening the pocket's floor, whose
/// offset ring grows past the opening (a concave face), shells with the
/// rim's rings the right way round; a pocket wall, whose offset ring
/// crosses the opening, refuses typed. Both failed validation before (the
/// hole lay outside the rim's outer boundary; a debug build panicked).
#[test]
fn opening_a_pocket_floor_shells_and_a_pocket_wall_refuses_typed() {
    let mut cutter = box_xyz(0.4, 0.4, 1.0);
    cutter
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.3, 0.3, 0.5)))
        .unwrap();
    let pocketed = Object::boolean(
        kernel::BooleanOp::Subtract,
        &unit_box(),
        &cutter,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let inside = |f: FaceId| {
        pocketed
            .loop_positions(pocketed.faces()[f].outer_loop)
            .all(|p| {
                (0.3 - 1e-9..=0.7 + 1e-9).contains(&p.x) && (0.3 - 1e-9..=0.7 + 1e-9).contains(&p.y)
            })
    };
    let pocket: Vec<FaceId> = pocketed.faces().keys().filter(|&f| inside(f)).collect();
    assert_eq!(pocket.len(), 5, "the floor and four walls");
    let bottom = bottom_of(&pocketed);
    let floor = *pocket
        .iter()
        .find(|&&f| pocketed.faces()[f].plane.normal().z > 0.999)
        .unwrap();
    for open in [vec![floor], vec![floor, bottom]] {
        for t in [T, -T] {
            let r =
                shell(&pocketed, open.clone(), t).unwrap_or_else(|e| panic!("{open:?} {t}: {e:?}"));
            assert_valid_solid(&r);
        }
    }
    for &wall in pocket.iter().filter(|&&f| f != floor) {
        for t in [T, -T] {
            match shell(&pocketed, vec![wall], t) {
                Ok(r) => assert_valid_solid(&r),
                Err(e) => assert!(
                    matches!(e, ShapeError::UnsupportedShellCorner { .. }),
                    "{e:?}"
                ),
            }
        }
    }
}

/// A pyramid with every sloped face opened leaves its base as a plate.
#[test]
fn a_pyramid_opened_on_every_side_leaves_its_base() {
    let p = irregular_pyramid();
    let base = bottom_of(&p);
    let open: Vec<FaceId> = p.faces().keys().filter(|&f| f != base).collect();
    let r = shell(&p, open, T).unwrap();
    assert_valid_solid(&r);
    assert_eq!(r.faces().len(), 6, "the base, its offset, and four rims");
    assert!((signed_volume(&r) - 4.0 * T).abs() < 1e-9);
}

// ──────────────────────────────────────────────────────── pocket floors

/// The unit box with a pocket `depth` deep cut in its top: a square of
/// half-width `a`, or a 32-sided round of radius `a`, centered.
fn pocketed_box(a: f64, depth: f64, round: bool) -> Object {
    pocketed_box_at(a, depth, round, 0.5, 0.5)
}

/// [`pocketed_box`] with the pocket centered at `(cx, cy)`.
fn pocketed_box_at(a: f64, depth: f64, round: bool, cx: f64, cy: f64) -> Object {
    let mut cutter = if round {
        cylinder(a, 1.0, 32)
    } else {
        box_xyz(2.0 * a, 2.0 * a, 1.0)
    };
    let at = if round {
        Vec3::new(cx, cy, 1.0 - depth)
    } else {
        Vec3::new(cx - a, cy - a, 1.0 - depth)
    };
    cutter
        .apply_transform(&kernel::Transform::translation(at))
        .unwrap();
    Object::boolean(
        kernel::BooleanOp::Subtract,
        &unit_box(),
        &cutter,
        &kernel::Transform::IDENTITY,
    )
    .unwrap()
}

/// The upward face at height `z` without holes: a pocket's floor.
fn floor_at(obj: &Object, z: f64) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().z > 0.999
                && f.inner_loops.is_empty()
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.z - z).abs() < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("the pocket floor")
}

/// The area of a face's outer loop.
fn face_area(obj: &Object, f: FaceId) -> f64 {
    let pts: Vec<Point3> = obj.loop_positions(obj.faces()[f].outer_loop).collect();
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..pts.len() {
        n = n + pts[i].to_vec().cross(pts[(i + 1) % pts.len()].to_vec());
    }
    0.5 * n.length()
}

/// The volume of the unit box with a centered pocket of floor area `area`
/// and depth `h`, its floor opened, shelled `t` inward: the box less its
/// inner offset, less the pocket's walls' offset collar under the top's
/// wall (a pocket deeper than the wall), or less the hole the pocket
/// leaves through the top's wall (one at most as deep), and less the
/// pocket itself. `grown`: the floor area grown by `t` (the walls' offset
/// ring).
fn pocket_floor_shell_volume(area: f64, grown: f64, h: f64, t: f64) -> f64 {
    let solid = 1.0 - area * h;
    let inner = (1.0 - 2.0 * t).powi(3);
    if t < h {
        solid - (inner - grown * (h - t))
    } else {
        solid - inner - area * (t - h)
    }
}

/// Opening a pocket's floor under a wall thinner than the pocket is deep:
/// the walls' inner faces stop at the floor's plane, where one rim face
/// rings the opening and faces down into the cavity; the volume is exact
/// both ways.
#[test]
fn a_pocket_floor_opens_with_its_rim_on_the_floor() {
    let (a, h) = (0.15, 0.067);
    let p = pocketed_box(a, h, false);
    let floor = floor_at(&p, 1.0 - h);
    let r = shell(&p, vec![floor], T).unwrap();
    assert_valid_solid(&r);
    let want = pocket_floor_shell_volume(4.0 * a * a, 4.0 * (a + T).powi(2), h, T);
    assert!((signed_volume(&r) - want).abs() < 1e-9);
    let rims: Vec<_> = r
        .faces()
        .values()
        .filter(|f| {
            !f.inner_loops.is_empty()
                && r.loop_positions(f.outer_loop)
                    .all(|q| (q.z - (1.0 - h)).abs() < 1e-12)
        })
        .collect();
    assert_eq!(rims.len(), 1, "one rim face around the opening");
    assert!(rims[0].plane.normal().z < -0.999, "it faces the cavity");
    // Outward: the walls grow into the pocket; the inner size is kept.
    let o = shell(&p, vec![floor], -T).unwrap();
    assert_valid_solid(&o);
    let d = T;
    let want = (1.0 + 2.0 * d).powi(3) - 4.0 * (a - d).powi(2) * (h + d) - (1.0 - 4.0 * a * a * h);
    assert!((signed_volume(&o) - want).abs() < 1e-9);
}

/// Opening a pocket's floor under a wall at least as thick as the pocket is
/// deep (refused before, the walls' offsets turning over): the walls run on
/// down through the top's wall and the opening is a hole through it with
/// the pocket's outline. Each wall stays one face (round walls keep their
/// cylinder and their soft seams), the floor's height is gone, and the
/// top's inner face carries the outline as its hole.
#[test]
fn a_pocket_floor_under_a_thicker_wall_opens_a_hole_through_the_top() {
    let (a, h, t) = (0.15, 0.067, 0.1);
    for round in [false, true] {
        let p = pocketed_box(a, h, round);
        let floor = floor_at(&p, 1.0 - h);
        let area = face_area(&p, floor);
        let r = shell(&p, vec![floor], t).unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        assert_valid_solid(&r);
        let want = pocket_floor_shell_volume(area, 0.0, h, t);
        let v = signed_volume(&r);
        assert!(
            (v - want).abs() < 1e-9,
            "round {round}: volume {v}, want {want}"
        );
        assert!(
            r.vertices()
                .values()
                .all(|q| (q.position.z - (1.0 - h)).abs() > 1e-6),
            "nothing is left at the floor's height"
        );
        let walls: Vec<_> = r
            .faces()
            .values()
            .filter(|f| {
                f.plane.normal().z.abs() < 1e-9
                    && r.loop_positions(f.outer_loop)
                        .all(|q| (q.x - 0.5).abs() < a + 1e-9 && (q.y - 0.5).abs() < a + 1e-9)
            })
            .collect();
        assert_eq!(walls.len(), if round { 32 } else { 4 }, "one face per wall");
        for w in &walls {
            let zs: Vec<f64> = r.loop_positions(w.outer_loop).map(|q| q.z).collect();
            let (lo, hi) = zs
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, u), &z| (l.min(z), u.max(z)));
            assert!((hi - 1.0).abs() < 1e-12 && (lo - (1.0 - t)).abs() < 1e-12);
            assert_eq!(
                w.surface.is_some(),
                round,
                "a round wall keeps its cylinder"
            );
        }
        assert_claims_hold(&r);
        if round {
            let soft_seams = r
                .edges()
                .values()
                .filter(|e| {
                    let (pa, pb) = (
                        r.vertices()[r.half_edges()[e.half_edge].origin].position,
                        r.vertices()[r.half_edges()[e.twin_half_edge.unwrap()].origin].position,
                    );
                    (pa.x - pb.x).abs() < 1e-12
                        && (pa.y - pb.y).abs() < 1e-12
                        && (pa.z - pb.z).abs() > 0.5 * t
                        && (pa.x - 0.5).hypot(pa.y - 0.5) < a + 1e-9
                })
                .filter(|e| e.soft)
                .count();
            assert_eq!(soft_seams, 32, "the seams down the round wall stay soft");
        }
        let ceiling: Vec<_> = r
            .faces()
            .values()
            .filter(|f| {
                f.plane.normal().z < -0.999
                    && r.loop_positions(f.outer_loop)
                        .all(|q| (q.z - (1.0 - t)).abs() < 1e-12)
            })
            .collect();
        assert_eq!(ceiling.len(), 1);
        assert_eq!(
            ceiling[0].inner_loops.len(),
            1,
            "the pocket's outline is its hole"
        );
    }
}

/// A wall exactly as thick as the pocket is deep: the walls are kept as
/// they are and the floor's outline becomes the hole in the top's inner
/// face, with no sliver between.
#[test]
fn a_wall_exactly_as_thick_as_the_pocket_is_deep_keeps_the_walls() {
    let (a, h) = (0.15, 0.1);
    let p = pocketed_box(a, h, false);
    let floor = floor_at(&p, 1.0 - h);
    let r = shell(&p, vec![floor], h).unwrap();
    assert_valid_solid(&r);
    let want = pocket_floor_shell_volume(4.0 * a * a, 0.0, h, h);
    assert!((signed_volume(&r) - want).abs() < 1e-9);
    // 6 outer faces, 4 walls, 6 inner faces: no rim.
    assert_eq!(r.faces().len(), 16);
    // Within a hair either side of the depth, the result is valid too.
    for t in [h - 1e-7, h - 1e-8, h - 2e-9, h - 1e-10, h + 1e-10, h + 1e-7] {
        let r = shell(&p, vec![floor], t).unwrap_or_else(|e| panic!("{t}: {e:?}"));
        assert_valid_solid(&r);
        let want = pocket_floor_shell_volume(4.0 * a * a, 4.0 * (a + t).powi(2), h, t);
        assert!((signed_volume(&r) - want).abs() < 1e-9, "{t}");
    }
}

/// A pocket drawn on the top and pushed in shells exactly like one cut by a
/// boolean, shallow or deep.
#[test]
fn a_pushed_pocket_shells_like_a_cut_one() {
    let (a, depths) = (0.15, [0.067, 0.3]);
    for h in depths {
        let mut p = unit_box();
        let top = top_of(&p);
        let ring = [
            p3(0.5 - a, 0.5 - a, 1.0),
            p3(0.5 + a, 0.5 - a, 1.0),
            p3(0.5 + a, 0.5 + a, 1.0),
            p3(0.5 - a, 0.5 + a, 1.0),
        ];
        let inset = p.split_face_inner(top, &ring).unwrap().sub_face;
        p.push_pull(inset, -h).unwrap();
        let floor = floor_at(&p, 1.0 - h);
        for t in [0.01, 0.1] {
            let r = shell(&p, vec![floor], t).unwrap_or_else(|e| panic!("{h} {t}: {e:?}"));
            assert_valid_solid(&r);
            let want = pocket_floor_shell_volume(4.0 * a * a, 4.0 * (a + t).powi(2), h, t);
            assert!((signed_volume(&r) - want).abs() < 1e-9, "{h} {t}");
        }
    }
}

/// Where the pocket's walls would run into the box's side walls, or into
/// another pocket's walls, the wall leaves no room: `WallTooThick`, typed
/// (a debug build panicked and a release build refused as degenerate).
#[test]
fn pocket_walls_that_meet_other_walls_refuse_wall_too_thick() {
    let p = pocketed_box(0.15, 0.5, false);
    for round in [false, true] {
        let p = pocketed_box(0.15, 0.5, round);
        let floor = floor_at(&p, 0.5);
        assert!(matches!(
            shell(&p, vec![floor], 0.2),
            Err(ShapeError::WallTooThick { .. })
        ));
    }
    // Shelled closed, the same pocket leaves no room either.
    assert!(matches!(
        shell(&p, Vec::new(), 0.2),
        Err(ShapeError::WallTooThick { .. })
    ));
    // Two pockets 0.1 apart under a 0.06 wall: their walls overlap.
    let mut cutter = box_xyz(0.2, 0.2, 1.0);
    cutter
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.6, 0.4, 0.7)))
        .unwrap();
    let mut first = box_xyz(0.2, 0.2, 1.0);
    first
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.3, 0.4, 0.7)))
        .unwrap();
    let id = kernel::Transform::IDENTITY;
    let two = Object::boolean(kernel::BooleanOp::Subtract, &unit_box(), &first, &id).unwrap();
    let two = Object::boolean(kernel::BooleanOp::Subtract, &two, &cutter, &id).unwrap();
    let floors: Vec<FaceId> = two
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z > 0.999 && f.inner_loops.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert_eq!(floors.len(), 2);
    assert!(matches!(
        shell(&two, floors.clone(), 0.06),
        Err(ShapeError::WallTooThick { .. })
    ));
    // Under a 0.04 wall both open, deep enough for the rim.
    let r = shell(&two, floors, 0.04).unwrap();
    assert_valid_solid(&r);
}

/// A pocket whose square walls lean in toward a smaller floor: under a
/// wall a little thicker than the pocket is deep, the leaning walls carry
/// on down to the top's inner face; under one so thick they would cross
/// first, `WallTooThick`.
#[test]
fn a_tapered_pocket_carries_its_walls_down_until_they_would_cross() {
    let (h, s0, s1) = (0.067, 0.05, 0.15);
    let k = (s1 - s0) / h; // half-width gained per unit of height
    let half = |z: f64| s0 + (z - (1.0 - h)) * k;
    let zt = 1.1;
    let mut pts = Vec::new();
    for (z, w) in [(1.0 - h, s0), (zt, half(zt))] {
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            pts.push(p3(0.5 + sx * w, 0.5 + sy * w, z));
        }
    }
    let faces = vec![
        vec![0, 1, 2, 3],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let cutter = convex_solid(&pts, &faces);
    let p = Object::boolean(
        kernel::BooleanOp::Subtract,
        &unit_box(),
        &cutter,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let floor = floor_at(&p, 1.0 - h);
    let t = 0.08;
    let r = shell(&p, vec![floor], t).unwrap();
    assert_valid_solid(&r);
    // The pocket (a frustum) and the hole under it (the frustum carried on
    // down to the inner face) come out of the hollow box.
    let frustum = |lo: f64, hi: f64| 4.0 * (half(hi).powi(3) - half(lo).powi(3)) / (3.0 * k);
    let want = 1.0 - (1.0 - 2.0 * t).powi(3) - frustum(1.0 - t, 1.0);
    let v = signed_volume(&r);
    assert!((v - want).abs() < 1e-9, "volume {v}, want {want}");
    // Carried down 0.3, the walls would cross (the floor would shrink past
    // nothing): the wall leaves no room.
    assert!(matches!(
        shell(&p, vec![floor], 0.3),
        Err(ShapeError::WallTooThick { .. })
    ));
}

/// The unit box less a square frustum pocket `h` deep: half-width `s0` at
/// the floor widening to `s1` at the top, centered.
fn tapered_pocket(h: f64, s0: f64, s1: f64) -> Object {
    let k = (s1 - s0) / h;
    let zt = 1.1;
    let mut pts = Vec::new();
    for (z, w) in [(1.0 - h, s0), (zt, s0 + (zt - (1.0 - h)) * k)] {
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            pts.push(p3(0.5 + sx * w, 0.5 + sy * w, z));
        }
    }
    let faces = vec![
        vec![0, 1, 2, 3],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::boolean(
        kernel::BooleanOp::Subtract,
        &unit_box(),
        &convex_solid(&pts, &faces),
        &kernel::Transform::IDENTITY,
    )
    .unwrap()
}

/// A tapered pocket's floor opened: the rim lies in the floor's plane under
/// a thin wall as under a straight-walled pocket, so the volume runs on
/// without a jump into the hole through the top as the wall reaches the
/// pocket's depth (it jumped about 1% a nanometer short of it, where the
/// rim stood square to the side edges instead).
#[test]
fn a_tapered_pocket_floor_rim_lies_on_the_floor_and_meets_the_hole_smoothly() {
    let (h, s0, s1) = (0.067, 0.05, 0.15);
    let k = (s1 - s0) / h;
    let zf = 1.0 - h;
    let half = |z: f64| s0 + (z - zf) * k;
    let p = tapered_pocket(h, s0, s1);
    let floor = floor_at(&p, zf);
    // The pocket and the hole (a frustum carried down), or the pocket's
    // walls' offset collar (each wall's plane moved `t` along its normal:
    // `t·√(1 + k²)` wider at every height) between the floor's plane and
    // the top's inner face.
    let cube = |a: f64, b: f64, sh: f64| 4.0 * ((b + sh).powi(3) - (a + sh).powi(3)) / (3.0 * k);
    let pocket = cube(half(zf), half(1.0), 0.0);
    let want = |t: f64| {
        let solid = 1.0 - pocket;
        let inner = (1.0 - 2.0 * t).powi(3);
        if t < h {
            let grow = t * (1.0 + k * k).sqrt();
            solid - (inner - cube(half(zf), half(1.0 - t), grow))
        } else {
            solid - inner - cube(half(1.0 - t), half(zf), 0.0)
        }
    };
    for t in [0.01, 0.03, 0.06, h - 1e-9, h - 1e-10, h, h + 1e-10, 0.07] {
        let r = shell(&p, vec![floor], t).unwrap_or_else(|e| panic!("{t}: {e:?}"));
        assert_valid_solid(&r);
        let v = signed_volume(&r);
        assert!(
            (v - want(t)).abs() < 1e-9,
            "{t}: volume {v}, want {}",
            want(t)
        );
        if t < h - 1e-6 {
            assert!(
                r.faces().values().any(|f| {
                    !f.inner_loops.is_empty()
                        && f.plane.normal().z < -0.999
                        && r.loop_positions(f.outer_loop)
                            .all(|q| (q.z - zf).abs() < 1e-12)
                }),
                "{t}: one rim face in the floor's plane"
            );
        }
    }
}

/// A box or slab opened on one face is no pocket, whatever face rings the
/// opening: past its size the wall still leaves no room. (The far face
/// once read as a pocket's top, which gave a solid grown past the box or
/// the box back with no hollow.)
#[test]
fn a_box_opened_on_one_face_is_not_a_pocket() {
    let cases: [(&str, Object, bool, &[f64]); 4] = [
        (
            "thin slab, bottom open",
            box_xyz(1.0, 1.0, 0.1),
            false,
            &[0.1, 0.15],
        ),
        (
            "thinner slab, top open",
            box_xyz(1.0, 1.0, 0.05),
            true,
            &[0.05, 0.06],
        ),
        (
            "unit cube, top open",
            unit_box(),
            true,
            &[0.5, 0.6, 1.0, 1.5],
        ),
        (
            "hexagonal prism, top open",
            prism(
                &[
                    (1.0, 0.0),
                    (0.5, 0.87),
                    (-0.5, 0.87),
                    (-1.0, 0.0),
                    (-0.5, -0.87),
                    (0.5, -0.87),
                ],
                0.2,
            ),
            true,
            &[0.2, 0.3],
        ),
    ];
    for (name, obj, top, ts) in cases {
        let face = if top { top_of(&obj) } else { bottom_of(&obj) };
        for &t in ts {
            assert!(
                matches!(
                    shell(&obj, vec![face], t),
                    Err(ShapeError::WallTooThick { .. })
                ),
                "{name} at {t}"
            );
        }
    }
}

/// Walls that just touch are walls meeting: a pocket whose walls' offset
/// ring reaches exactly to the side walls' inner faces, and a deep pocket
/// beside a shallow one whose walls touch, refuse `WallTooThick` like the
/// overlaps (they refused as passing through themselves).
#[test]
fn walls_that_just_touch_refuse_wall_too_thick() {
    // Half-width 0.1 grown by 0.2 reaches 0.8, where the side walls' inner
    // faces stand.
    let p = pocketed_box(0.1, 0.3, false);
    let floor = floor_at(&p, 0.7);
    assert!(matches!(
        shell(&p, vec![floor], 0.2),
        Err(ShapeError::WallTooThick { .. })
    ));
    // A pocket x 0.2..0.4, 0.3 deep (its ring grows to 0.5 under a 0.1
    // wall) beside one x 0.5..0.7, 0.05 deep (the hole through the top
    // keeps its outline): they touch at x = 0.5.
    let id = kernel::Transform::IDENTITY;
    let mut deep = box_xyz(0.2, 0.2, 1.0);
    deep.apply_transform(&kernel::Transform::translation(Vec3::new(0.2, 0.4, 0.7)))
        .unwrap();
    let mut shallow = box_xyz(0.2, 0.2, 1.0);
    shallow
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.5, 0.4, 0.95)))
        .unwrap();
    let two = Object::boolean(kernel::BooleanOp::Subtract, &unit_box(), &deep, &id).unwrap();
    let two = Object::boolean(kernel::BooleanOp::Subtract, &two, &shallow, &id).unwrap();
    let floors = vec![floor_at(&two, 0.7), floor_at(&two, 0.95)];
    assert!(matches!(
        shell(&two, floors.clone(), 0.1),
        Err(ShapeError::WallTooThick { .. })
    ));
    // A hair thinner, they part, and both open.
    let r = shell(&two, floors, 0.1 - 1e-6).unwrap();
    assert_valid_solid(&r);
}

/// Two pockets no deeper than the wall with a pillar between them thinner
/// than the wall: each opens as a hole through the top, and the pillar is
/// the material it was. Every point of it lies within the wall's thickness
/// of the pocket walls beside it, so the shell takes none of it away and
/// leaves it no thinner than it was; there are no offset walls to meet.
/// (Under deep pockets the walls' offsets would meet in the pillar, which
/// refuses: see `pocket_walls_that_meet_other_walls_refuse_wall_too_thick`.)
#[test]
fn a_thin_pillar_between_two_shallow_pockets_stays_as_it_was() {
    let id = kernel::Transform::IDENTITY;
    let mut first = box_xyz(0.25, 0.2, 1.0);
    first
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.2, 0.4, 0.95)))
        .unwrap();
    let mut second = box_xyz(0.25, 0.2, 1.0);
    second
        .apply_transform(&kernel::Transform::translation(Vec3::new(0.5, 0.4, 0.95)))
        .unwrap();
    let two = Object::boolean(kernel::BooleanOp::Subtract, &unit_box(), &first, &id).unwrap();
    let two = Object::boolean(kernel::BooleanOp::Subtract, &two, &second, &id).unwrap();
    let floors: Vec<FaceId> = two
        .faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z > 0.999 && f.inner_loops.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert_eq!(floors.len(), 2);
    let t = 0.1;
    let r = shell(&two, floors, t).unwrap();
    assert_valid_solid(&r);
    // The hollow box less two holes 0.25 × 0.2 through the 0.1 top.
    let want = 1.0 - (1.0 - 2.0 * t).powi(3) - 2.0 * 0.25 * 0.2 * t;
    assert!((signed_volume(&r) - want).abs() < 1e-9);
    // The pillar's two sides stand 0.05 apart from the top to the inner
    // face, as before.
    for x in [0.45, 0.5] {
        let side = r
            .faces()
            .values()
            .find(|f| {
                f.plane.normal().x.abs() > 0.999
                    && r.loop_positions(f.outer_loop)
                        .all(|q| (q.x - x).abs() < 1e-12)
            })
            .unwrap_or_else(|| panic!("a pillar side at x = {x}"));
        let zs: Vec<f64> = r.loop_positions(side.outer_loop).map(|q| q.z).collect();
        let lo = zs.iter().copied().fold(f64::MAX, f64::min);
        assert!((lo - (1.0 - t)).abs() < 1e-12);
    }
}

/// A 0.4 square pocket 0.1 deep whose four walls claim one sphere (centered
/// on the pocket's axis at mid-depth, through all eight corners).
fn sphere_walled_pocket() -> Object {
    let p = pocketed_box(0.2, 0.1, false);
    let center = p3(0.5, 0.5, 0.95);
    let radius = (0.2f64 * 0.2 + 0.2 * 0.2 + 0.05 * 0.05).sqrt();
    let walls: Vec<FaceId> = p
        .faces()
        .iter()
        .filter(|(_, f)| {
            f.plane.normal().z.abs() < 1e-9
                && p.loop_positions(f.outer_loop)
                    .all(|q| (q.x - 0.5).abs() < 0.2 + 1e-9 && (q.y - 0.5).abs() < 0.2 + 1e-9)
        })
        .map(|(k, _)| k)
        .collect();
    assert_eq!(walls.len(), 4);
    walls.into_iter().fold(p, |o, w| {
        with_claim(&o, w, SurfaceRef::Sphere { center, radius })
    })
}

/// Sphere-claimed pocket walls under a thin wall: the floor's offset points
/// move into the floor's plane along the offset sphere, so every inner wall
/// keeps its offset sphere (half of them lost it, the points having slid
/// along the side edges' chords off it).
#[test]
fn a_sphere_walled_pocket_floor_keeps_the_inner_spheres() {
    let p = sphere_walled_pocket();
    let floor = floor_at(&p, 0.9);
    let r = shell(&p, vec![floor], 0.03).unwrap();
    assert_valid_solid(&r);
    assert_inner_claims_kept(&p, &r, 0);
    assert_claims_hold(&r);
    // The rim lies in the floor's plane.
    assert!(r.faces().values().any(|f| {
        !f.inner_loops.is_empty()
            && f.plane.normal().z < -0.999
            && r.loop_positions(f.outer_loop)
                .all(|q| (q.z - 0.9).abs() < 1e-12)
    }));
}

/// Sphere-claimed pocket walls under a wall thicker than the pocket: the
/// walls carried down as flat faces leave the sphere, so they drop the
/// claim rather than keep one their geometry no longer fits (a debug build
/// panicked on the disagreeing claim).
#[test]
fn sphere_walls_carried_down_drop_the_claim_they_no_longer_fit() {
    let p = sphere_walled_pocket();
    let floor = floor_at(&p, 0.9);
    let r = shell(&p, vec![floor], 0.12).unwrap();
    assert_valid_solid(&r);
    assert_claims_hold(&r);
    let want = 1.0 - (1.0 - 0.24f64).powi(3) - 0.16 * 0.12;
    assert!((signed_volume(&r) - want).abs() < 1e-9);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// Any square or round pocket, centered or not, shallow or nearly
    /// through, its floor opened, any wall: a valid solid of exactly the
    /// expected volume (the rim on the floor for a pocket deeper than the
    /// wall, a hole through the top for one as deep or shallower), or a
    /// typed refusal exactly where the walls run into each other (the
    /// pocket's into the side walls') or the floor lies inside the bottom's
    /// wall. Every case asserts one or the other; within a hair of either
    /// limit, both are right.
    #[test]
    fn a_pocket_floor_shells_exactly_or_refuses_too_thick(
        a in 0.03f64..0.3,
        h in 0.02f64..0.95,
        t in 0.005f64..0.2,
        cx in 0.3f64..0.7,
        cy in 0.3f64..0.7,
        round in proptest::bool::ANY,
    ) {
        let p = pocketed_box_at(a, h, round, cx, cy);
        let floor = floor_at(&p, 1.0 - h);
        let area = face_area(&p, floor);
        // The walls' offset ring: the square grown by `t`; the round's
        // corners moved `t` out onto the offset cylinder.
        let grown = if round {
            let n = 32.0;
            0.5 * n * (a + t).powi(2) * (2.0 * PI / n).sin()
        } else {
            4.0 * (a + t).powi(2)
        };
        let tunnel = t >= h;
        // The top's inner face's hole: the walls' offset ring under a deep
        // pocket, the pocket's own outline under a shallow one. Its room to
        // the side walls' inner faces, `t` in from the sides, and (under a
        // deep pocket) the floor's height over the bottom's inner face.
        let reach = if tunnel { a } else { a + t };
        let side = (cx - reach).min(1.0 - cx - reach).min(cy - reach).min(1.0 - cy - reach) - t;
        let under = if tunnel { f64::INFINITY } else { 1.0 - h - t };
        let room = side.min(under);
        let r = shell(&p, vec![floor], t);
        let typed = |r: &Result<Object, ShapeError>| {
            matches!(
                r,
                Err(ShapeError::WallTooThick { .. } | ShapeError::SelfIntersecting { .. })
            )
        };
        match &r {
            Ok(obj) => {
                proptest::prop_assert!(room > -1e-6, "a result where the walls meet");
                assert_valid_solid(obj);
                let want = pocket_floor_shell_volume(area, grown, h, t);
                let v = signed_volume(obj);
                proptest::prop_assert!((v - want).abs() < 1e-9, "volume {} want {}", v, want);
            }
            Err(e) => {
                proptest::prop_assert!(typed(&r), "{:?}", e);
                proptest::prop_assert!(room < 1e-6, "{:?} with room {}", e, room);
                if side < -1e-6 {
                    proptest::prop_assert!(
                        matches!(e, ShapeError::WallTooThick { .. }),
                        "walls meeting the side walls: {:?}", e
                    );
                }
            }
        }
    }
}

/// Playtest B 4.2.6 (D-236): a 300 mm box, a 100 × 150 pocket 20 deep, its
/// floor opened at a 10 mm wall. Five millimeters from a side wall the
/// pocket's wall collar would run out through the box's side: refused
/// `WallTooThick`. Twenty-five millimeters in, the collar sits inside the
/// cavity and the volume is exact: 4,952 cm³, the number the playtest
/// measured for its "5 mm" pocket (so that pocket was not 5 mm in).
#[test]
fn a_pocket_floor_near_a_side_wall_refuses_and_one_further_in_is_exact() {
    let id = kernel::Transform::IDENTITY;
    let pocketed = |x: f64| {
        let mut cut = box_xyz(0.1, 0.15, 1.0);
        cut.apply_transform(&kernel::Transform::translation(Vec3::new(x, 0.075, 0.28)))
            .unwrap();
        Object::boolean(
            kernel::BooleanOp::Subtract,
            &box_xyz(0.3, 0.3, 0.3),
            &cut,
            &id,
        )
        .unwrap()
    };
    for x in [0.005, 0.015, 0.019] {
        let p = pocketed(x);
        let floor = floor_at(&p, 0.28);
        assert!(
            matches!(
                shell(&p, vec![floor], 0.01),
                Err(ShapeError::WallTooThick { .. })
            ),
            "{x}"
        );
    }
    let p = pocketed(0.025);
    let floor = floor_at(&p, 0.28);
    let r = shell(&p, vec![floor], 0.01).unwrap();
    assert_valid_solid(&r);
    // The box less the pocket, less the cavity (280³) with the pocket's
    // 120 × 170 collar 10 deep left in it.
    let want = 0.3f64.powi(3) - 0.1 * 0.15 * 0.02 - (0.28f64.powi(3) - 0.12 * 0.17 * 0.01);
    assert!(
        (signed_volume(&r) - want).abs() < 1e-12,
        "{}",
        signed_volume(&r)
    );
}
