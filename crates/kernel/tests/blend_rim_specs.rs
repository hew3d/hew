//! Executable specs for rim and along-axis blends (WP06 §2–4, §8): tori,
//! hemispheres and cones around circles, line–arc–line chains, and seam
//! crossing on a faceted cylinder.

use crate::common;
use kernel::trig;

use std::collections::BTreeSet;

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendReport, BlendSpec, BooleanOp, CornerStyle, EdgeId, FaceId,
    LimitFeature, Object, Point3, SurfaceRef, Transform, Vec3,
};

fn round(radius: f64, segments: u32) -> BlendProfile {
    BlendProfile::Round { radius, segments }
}

fn spec(edges: Vec<EdgeId>, profile: BlendProfile) -> BlendSpec {
    BlendSpec {
        edges,
        profile,
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn blend_ok(obj: &Object, s: &BlendSpec) -> (Object, BlendReport) {
    let (after, report) = obj.blend_edges(s).expect("blend succeeds");
    assert_valid_solid(&after);
    for &f in &report.new_faces {
        let face = &after.faces()[f];
        assert!(report.blends.contains(&face.blend.expect("tagged").id));
        if let Some(claim) = face.surface {
            for p in after.loop_positions(face.outer_loop) {
                assert!(
                    claim.distance(p).abs() <= kernel::tol::PLANE_DIST,
                    "{p:?} on {claim:?}"
                );
            }
        }
    }
    (after, report)
}

/// Every edge of `obj` whose endpoints both lie at height `z` and at
/// distance `r` from the vertical axis through `(cx, cy)`.
fn circle_edges(obj: &Object, cx: f64, cy: f64, z: f64, r: f64) -> Vec<EdgeId> {
    let on =
        |p: Point3| (p.z - z).abs() < 1e-9 && (trig::hypot(p.x - cx, p.y - cy) - r).abs() < 1e-9;
    obj.edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            on(a) && on(b)
        })
        .collect()
}

fn new_claims(after: &Object, report: &BlendReport) -> Vec<SurfaceRef> {
    report
        .new_faces
        .iter()
        .filter_map(|&f| after.faces()[f].surface)
        .collect()
}

fn torus_radii(s: &SurfaceRef) -> (f64, f64) {
    match *s {
        SurfaceRef::Torus {
            major_radius,
            minor_radius,
            ..
        } => (major_radius, minor_radius),
        other => panic!("not a torus: {other:?}"),
    }
}

#[test]
fn cylinder_top_rim_torus() {
    let obj = cylinder(0.5, 1.0, 48);
    let rim = circle_edges(&obj, 0.0, 0.0, 1.0, 0.5);
    assert_eq!(rim.len(), 48);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.1, 6)));
    assert_eq!(report.new_faces.len(), 48 * 6);
    let claims = new_claims(&after, &report);
    assert_eq!(claims.len(), 48 * 6);
    assert!(claims.iter().all(|c| c.same_surface(&claims[0])));
    let (major, minor) = torus_radii(&claims[0]);
    assert!((major - 0.4).abs() < 1e-12 && (minor - 0.1).abs() < 1e-12);
    // The cap is a 48-gon of radius 0.4.
    let cap = face_with_normal(&after, Vec3::new(0.0, 0.0, 1.0));
    let ring: Vec<Point3> = after
        .loop_positions(after.faces()[cap].outer_loop)
        .collect();
    assert_eq!(ring.len(), 48);
    assert!(
        ring.iter()
            .all(|p| (trig::hypot(p.x, p.y) - 0.4).abs() < 1e-12)
    );
    // Every rail is an arc of a circle about the axis.
    let rails_with_curves = after
        .edges()
        .values()
        .filter(|e| {
            e.curve
                .is_some_and(|c| (c.radius - 0.4).abs() < 1e-12 || (c.radius - 0.5).abs() < 1e-12)
        })
        .count();
    assert!(rails_with_curves >= 2 * 48, "{rails_with_curves}");
}

#[test]
fn hole_mouth_torus() {
    let obj = plate_with_hole(1.0, 0.2, 0.2, 32);
    let rim = circle_edges(&obj, 0.0, 0.0, 0.2, 0.2);
    assert_eq!(rim.len(), 32);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.02, 4)));
    let claims = new_claims(&after, &report);
    assert_eq!(claims.len(), 32 * 4);
    let (major, _) = torus_radii(&claims[0]);
    assert!((major - 0.22).abs() < 1e-12, "{major}");
}

#[test]
fn boss_base_concave_torus() {
    let obj = plate_with_boss(1.0, 0.2, 0.2, 0.3, 32);
    let rim = circle_edges(&obj, 0.0, 0.0, 0.2, 0.2);
    assert_eq!(rim.len(), 32);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.02, 4)));
    let (major, _) = torus_radii(&new_claims(&after, &report)[0]);
    assert!((major - 0.22).abs() < 1e-12, "{major}");
    assert!(signed_volume(&after) > signed_volume(&obj));
}

/// A unit box (centered on the z axis) with a blind round pocket of radius
/// 0.25 and depth 0.5 in its top.
fn pocket_block() -> Object {
    let mut b = unit_box();
    b.apply_transform(&Transform::translation(Vec3::new(-0.5, -0.5, 0.0)))
        .unwrap();
    let mut c = cylinder(0.25, 0.6, 32);
    c.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, 0.5)))
        .unwrap();
    Object::boolean(BooleanOp::Subtract, &b, &c, &Transform::IDENTITY).unwrap()
}

#[test]
fn pocket_floor_concave_torus() {
    let obj = pocket_block();
    let rim = circle_edges(&obj, 0.0, 0.0, 0.5, 0.25);
    assert_eq!(rim.len(), 32);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.05, 4)));
    let (major, _) = torus_radii(&new_claims(&after, &report)[0]);
    assert!((major - 0.2).abs() < 1e-12, "{major}");
    assert!(signed_volume(&after) > signed_volume(&obj));
}

#[test]
fn rod_end_hemisphere() {
    let obj = cylinder(0.05, 1.0, 48);
    let rim = circle_edges(&obj, 0.0, 0.0, 1.0, 0.05);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.05, 6)));
    let claims = new_claims(&after, &report);
    assert!(!claims.is_empty());
    assert!(
        claims.iter().all(
            |c| matches!(c, SurfaceRef::Sphere { radius, .. } if (radius - 0.05).abs() < 1e-12)
        )
    );
    // The cap is gone: no face faces straight up; one pole vertex.
    assert!(
        after
            .faces()
            .values()
            .all(|f| f.plane.normal().z < 1.0 - 1e-9 || f.blend.is_some())
    );
    let poles = after
        .vertices()
        .values()
        .filter(|v| v.position.approx_eq(p3(0.0, 0.0, 1.0), 1e-12))
        .count();
    assert_eq!(poles, 1);
}

#[test]
fn rim_overshoot_refused() {
    let obj = cylinder(0.05, 1.0, 48);
    let rim = circle_edges(&obj, 0.0, 0.0, 1.0, 0.05);
    match obj.blend_edges(&spec(rim, round(0.06, 6))).unwrap_err() {
        BlendError::RadiusTooLarge { max, limited_by } => {
            assert_eq!(limited_by, LimitFeature::RimRadius);
            assert!((max - 0.05).abs() <= 1e-8, "{max}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn rim_chamfer_cone() {
    let obj = cylinder(0.5, 1.0, 48);
    let rim = circle_edges(&obj, 0.0, 0.0, 1.0, 0.5);
    let (after, report) = blend_ok(
        &obj,
        &spec(
            rim,
            BlendProfile::Chamfer {
                setback_a: 0.05,
                setback_b: 0.05,
            },
        ),
    );
    let claims = new_claims(&after, &report);
    assert_eq!(claims.len(), 48);
    for c in &claims {
        match *c {
            SurfaceRef::Cone {
                apex, half_angle, ..
            } => {
                assert!(apex.x.abs() < 1e-12 && apex.y.abs() < 1e-12, "{apex:?}");
                assert!((half_angle - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn fillet_after_rim_chamfer_is_coaxial_torus() {
    let obj = cylinder(0.5, 1.0, 48);
    let rim = circle_edges(&obj, 0.0, 0.0, 1.0, 0.5);
    let (chamfered, chamfer) = blend_ok(
        &obj,
        &spec(
            rim,
            BlendProfile::Chamfer {
                setback_a: 0.05,
                setback_b: 0.05,
            },
        ),
    );
    // The sharp edge between the cone band and the wall, at z = 0.95.
    let lower = circle_edges(&chamfered, 0.0, 0.0, 0.95, 0.5);
    assert_eq!(lower.len(), 48);
    let (after, mut report) = blend_ok(&chamfered, &spec(lower, round(0.01, 4)));
    // The corner re-solve rebuilds the chamfer under its own id and reports
    // it too; the claims under test are the fillet's.
    report.new_faces.retain(|&f| {
        after.faces()[f]
            .blend
            .is_some_and(|t| !chamfer.blends.contains(&t.id))
    });
    let claims = new_claims(&after, &report);
    assert!(!claims.is_empty());
    for c in &claims {
        match *c {
            SurfaceRef::Torus { center, axis, .. } => {
                assert!(center.x.abs() < 1e-12 && center.y.abs() < 1e-12);
                assert!(axis.z.abs() > 1.0 - 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn rounded_rect_rim_line_arc_chain() {
    let obj = rounded_rect_prism(1.0, 0.6, 0.1, 0.3, 6);
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    let rim = obj.face_loop_edges(top, p3(0.5, 0.0, 0.3));
    assert_eq!(rim.len(), 4 + 4 * 6);
    let (after, report) = blend_ok(&obj, &spec(rim, round(0.05, 4)));
    let mut kinds = BTreeSet::new();
    for c in new_claims(&after, &report) {
        kinds.insert(c.kind_name());
        if let SurfaceRef::Torus { major_radius, .. } = c {
            assert!((major_radius - 0.05).abs() < 1e-12);
        }
    }
    assert_eq!(kinds, BTreeSet::from(["cylinder", "torus"]));
    assert_eq!(report.new_faces.len(), (4 + 4 * 6) * 4);
}

#[test]
fn verticals_then_rim() {
    let obj = unit_box();
    let verticals: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&obj, p3(x, y, 0.0), p3(x, y, 1.0)))
        .collect();
    let (rounded, _) = blend_ok(&obj, &spec(verticals, round(0.2, 6)));
    let top = face_with_normal(&rounded, Vec3::new(0.0, 0.0, 1.0));
    let rim = rounded.face_loop_edges(top, p3(0.5, 0.0, 1.0));
    assert_eq!(rim.len(), 4 + 4 * 6);
    let (after, report) = blend_ok(&rounded, &spec(rim, round(0.05, 4)));
    let tori: Vec<f64> = new_claims(&after, &report)
        .iter()
        .filter_map(|c| match *c {
            SurfaceRef::Torus { major_radius, .. } => Some(major_radius),
            _ => None,
        })
        .collect();
    assert_eq!(tori.len(), 4 * 6 * 4);
    assert!(tori.iter().all(|m| (m - 0.15).abs() < 1e-12));
}

/// The straight edge of a D-post at (0, 0.5) and the wall facets'
/// claim.
fn d_post_edge(obj: &Object) -> (EdgeId, SurfaceRef) {
    let e = edge_between(obj, p3(0.0, 0.5, 0.0), p3(0.0, 0.5, 1.0));
    let wall = obj
        .faces()
        .values()
        .find_map(|f| f.surface)
        .expect("the wall claims its cylinder");
    (e, wall)
}

fn wall_facets(obj: &Object, wall: &SurfaceRef) -> Vec<FaceId> {
    obj.faces()
        .iter()
        .filter(|(_, f)| f.blend.is_none() && f.surface.is_some_and(|s| s.same_surface(wall)))
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn d_post_within_one_facet() {
    let obj = d_post(0.5, 1.0, 48);
    let (e, wall) = d_post_edge(&obj);
    let before = wall_facets(&obj, &wall).len();
    let (after, report) = blend_ok(&obj, &spec(vec![e], round(0.002, 4)));
    assert_eq!(wall_facets(&after, &wall).len(), before);
    let claims = new_claims(&after, &report);
    assert_eq!(claims.len(), 4);
    assert!(
        claims
            .iter()
            .all(|c| matches!(c, SurfaceRef::Cylinder { .. }))
    );
}

#[test]
fn d_post_crossing_seams() {
    let obj = d_post(0.5, 1.0, 48);
    let (e, wall) = d_post_edge(&obj);
    let before = wall_facets(&obj, &wall).len();
    let (after, report) = blend_ok(&obj, &spec(vec![e], round(0.15, 6)));
    assert_eq!(
        wall_facets(&after, &wall).len(),
        before - 3,
        "3 facets consumed"
    );
    let claims = new_claims(&after, &report);
    assert_eq!(claims.len(), 6);
    assert!(
        claims
            .iter()
            .all(|c| matches!(c, SurfaceRef::Cylinder { .. }))
    );
    // Top and bottom lost the three seam vertices.
    for z in [0.0, 1.0] {
        let cap = face_with_normal(
            &after,
            Vec3::new(0.0, 0.0, if z > 0.5 { 1.0 } else { -1.0 }),
        );
        let n_before = {
            let c = face_with_normal(&obj, Vec3::new(0.0, 0.0, if z > 0.5 { 1.0 } else { -1.0 }));
            obj.loop_positions(obj.faces()[c].outer_loop).count()
        };
        let n_after = after.loop_positions(after.faces()[cap].outer_loop).count();
        // v and three seam vertices go; E_0..E_6 come in.
        assert_eq!(n_after, n_before - 4 + 7);
    }
}

#[test]
fn revolved_dome_rim_is_a_torus_rim_from_one_click() {
    // Where a revolved dome's sphere meets its flat base, the rim turns
    // about the line through the sphere's center along the base's normal.
    // One click (no tangent propagation) picks the whole rim: the base's
    // revolved wedges are one logical face (D-46, D-47).
    let dome = revolved_dome(6, 24);
    let rim = sharp_edges_at_height(&dome, 0.0);
    assert_eq!(rim.len(), 24);
    let picked = dome.logical_edge(rim[0], false);
    assert_eq!(picked.len(), 24);
    let before = signed_volume(&dome);
    for r in [0.005, 0.02] {
        let (after, report) = blend_ok(&dome, &spec(picked.clone(), round(r, 4)));
        let claims = new_claims(&after, &report);
        assert!(!claims.is_empty());
        for c in &claims {
            let (major, minor) = torus_radii(c);
            assert!((minor - r).abs() <= 1e-9);
            assert!(major < 0.5);
        }
        assert!(signed_volume(&after) < before);
    }
    // The base's revolved wedges stay one face: their seams, shortened to
    // the rail, stay soft rather than coming back as drawn lines.
    let (after, _) = blend_ok(&dome, &spec(picked.clone(), round(0.02, 4)));
    let spokes: Vec<EdgeId> = after
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = after.edge_endpoints(e).unwrap();
            a.z.abs() < 1e-9
                && b.z.abs() < 1e-9
                && (trig::hypot(a.x, a.y) < 1e-9 || trig::hypot(b.x, b.y) < 1e-9)
        })
        .collect();
    assert_eq!(spokes.len(), 24);
    assert!(spokes.iter().all(|&e| after.edges()[e].soft));
    // The fillet meets the faceted sphere tangentially: no crease drawn
    // anywhere (its rails are tangent edges, the facets seams).
    assert!(
        after
            .edges()
            .keys()
            .all(|e| after.edge_smoothness(e) != kernel::EdgeSmoothness::Hard)
    );
    // Too large for the lowest band, the torus rim refuses; the ball rolls
    // on across the bands as a swept blend instead (D-228: a swept chain
    // tries more than one start), which removes back exactly.
    let (after, report) = blend_ok(&dome, &spec(picked.clone(), round(0.2, 4)));
    assert!(new_claims(&after, &report).is_empty());
    assert!(signed_volume(&after) < before);
    let (back, _) = after.remove_blend_for_test(report.blends[0]).unwrap();
    assert!(kernel::same_geometry_for_test(&back, &dome, 1e-9));
    // Past what rolls: refused, naming a size that builds.
    match dome.blend_edges(&spec(picked.clone(), round(0.3, 4))) {
        Err(BlendError::RadiusTooLarge { max, .. }) => {
            assert!(max > 0.2 && max < 0.3, "{max}");
            assert!(dome.blend_edges(&spec(picked, round(max, 4))).is_ok());
        }
        other => panic!("expected RadiusTooLarge, got {:?}", other.map(|_| ())),
    }
}

/// `cylinder(0.5, 1, 24)` with its top cap split by a chord between the
/// rim vertices `a` and `b` (indices around from +x), the walls keeping
/// their cylinder claims.
fn cylinder_with_cap_chord(a: usize, b: usize) -> Object {
    let c = cylinder(0.5, 1.0, 24);
    let (pts, faces) = c.to_polygons();
    let same_ring = |ps: &[Point3], ring: &[usize]| {
        ps.len() == ring.len()
            && ring
                .iter()
                .all(|&i| ps.iter().any(|p| p.approx_eq(pts[i], 1e-12)))
    };
    let at = |ring: &[usize], k: usize| {
        let want = std::f64::consts::TAU * k as f64 / 24.0;
        ring.iter()
            .position(|&i| {
                let d = trig::atan2(pts[i].y, pts[i].x) - want + std::f64::consts::PI;
                (d.rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs() < 1e-9
            })
            .unwrap()
    };
    let mut rings: Vec<Vec<usize>> = Vec::new();
    let mut claims = Vec::new();
    for f in &faces {
        if f.len() == 24 && f.iter().all(|&i| (pts[i].z - 1.0).abs() < 1e-12) {
            let (ia, ib) = (at(f, a), at(f, b));
            for (from, to) in [(ia, ib), (ib, ia)] {
                let mut piece = Vec::new();
                let mut k = from;
                loop {
                    piece.push(f[k]);
                    if k == to {
                        break;
                    }
                    k = (k + 1) % f.len();
                }
                rings.push(piece);
                claims.push(None);
            }
        } else {
            claims.push(
                c.faces()
                    .values()
                    .find(|g| {
                        let ps: Vec<Point3> = c.loop_positions(g.outer_loop).collect();
                        same_ring(&ps, f)
                    })
                    .and_then(|g| g.surface),
            );
            rings.push(f.clone());
        }
    }
    let mut obj = Object::from_polygons(&pts, &rings).unwrap();
    let ids: Vec<(kernel::FaceId, Vec<Point3>)> = obj
        .faces()
        .iter()
        .map(|(id, f)| (id, obj.loop_positions(f.outer_loop).collect()))
        .collect();
    for (ring, claim) in rings.iter().zip(&claims) {
        let Some(s) = claim else { continue };
        let id = ids.iter().find(|(_, ps)| same_ring(ps, ring)).unwrap().0;
        obj.set_face_surface_for_test(id, Some(*s));
    }
    assert_valid_solid(&obj);
    obj
}

/// A chord drawn across a rim's cap (a line between two rim vertices)
/// does not change the rim's round, and now runs between its crossings
/// with the cap's rail: where the rail crosses it beside a
/// station, behind the station it is found on the previous segment's own
/// rail, which turns from the next segment's at a rim vertex. Before, a
/// crossing behind the station was found on the next segment's line, off
/// the rail, and any chord but a diameter refused (`BlendDegenerate`).
#[test]
fn a_chord_across_a_rim_cap_rounds_as_the_plain_rim() {
    let plain = cylinder(0.5, 1.0, 24);
    for r in [0.02, 0.05] {
        let rim = sharp_edges_at_height(&plain, 1.0);
        let (cut, report) = blend_ok(&plain, &spec(rim, round(r, 4)));
        let removed = signed_volume(&plain) - signed_volume(&cut);
        for (a, b) in [(0, 8), (0, 12), (3, 20)] {
            let obj = cylinder_with_cap_chord(a, b);
            let rim: Vec<EdgeId> = sharp_edges_at_height(&obj, 1.0)
                .into_iter()
                .filter(|&e| {
                    let (p, q) = obj.edge_endpoints(e).unwrap();
                    (p - q).length() < 0.2
                })
                .collect();
            assert_eq!(rim.len(), 24);
            let (after, r2) = blend_ok(&obj, &spec(rim, round(r, 4)));
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!(
                (delta - removed).abs() <= 1e-12,
                "{r} {a}-{b}: {delta} vs {removed}"
            );
            assert_eq!(r2.new_faces.len(), report.new_faces.len(), "{r} {a}-{b}");
            // The chord now runs between its crossings with the cap's rail
            // (the rim polygon at radius 0.5 − r: a rim round sections
            // radially at each vertex), along the chord it was.
            let corner = |k: usize| {
                let t = std::f64::consts::TAU * k as f64 / 24.0;
                (trig::cos(t), trig::sin(t))
            };
            let (ca, cb) = (corner(a), corner(b));
            let on_chord = |p: Point3| {
                let (dx, dy) = (cb.0 - ca.0, cb.1 - ca.1);
                ((p.x / 0.5 - ca.0) * dy - (p.y / 0.5 - ca.1) * dx).abs() < 1e-9
            };
            let on_rail = |p: Point3| {
                let rr = 0.5 - r;
                (0..24).any(|k| {
                    let (u, w) = (corner(k), corner(k + 1));
                    let (u, w) = (p3(rr * u.0, rr * u.1, 1.0), p3(rr * w.0, rr * w.1, 1.0));
                    let along = (p - u).dot(w - u) / (w - u).dot(w - u);
                    (-1e-9..=1.0 + 1e-9).contains(&along)
                        && (u + (w - u) * along - p).length() < 1e-9
                })
            };
            let chords: Vec<(Point3, Point3)> = after
                .edges()
                .keys()
                .filter_map(|e| after.edge_endpoints(e))
                .filter(|&(p, q)| {
                    (p.z - 1.0).abs() < 1e-12
                        && (q.z - 1.0).abs() < 1e-12
                        && on_chord(p)
                        && on_chord(q)
                        && (p - q).length() > 0.1
                })
                .collect();
            assert_eq!(chords.len(), 1, "{r} {a}-{b}: one chord");
            let (p, q) = chords[0];
            assert!(
                on_rail(p) && on_rail(q),
                "{r} {a}-{b}: {p:?} {q:?} on the rail"
            );
        }
    }
}
