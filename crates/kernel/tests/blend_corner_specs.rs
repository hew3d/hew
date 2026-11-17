//! Executable specs for blend corners (WP06 §5–7, §8): rolled sphere
//! corners, the mitered and flat options, and two-pass mixed corners.

use crate::common;

use std::f64::consts::PI;

use common::curves::*;
use kernel::{
    BlendError, BlendKind, BlendProfile, BlendReport, BlendRole, BlendSpec, BooleanOp, CornerStyle,
    EdgeId, Object, Point3, SurfaceRef, Transform, Vec3,
};

fn round(radius: f64, segments: u32) -> BlendProfile {
    BlendProfile::Round { radius, segments }
}

fn chamfer(s: f64) -> BlendProfile {
    BlendProfile::Chamfer {
        setback_a: s,
        setback_b: s,
    }
}

fn spec(edges: Vec<EdgeId>, profile: BlendProfile, corners: CornerStyle) -> BlendSpec {
    BlendSpec {
        edges,
        profile,
        primary: None,
        corners,
        stops: None,
        handles: Vec::new(),
    }
}

fn blend_ok(obj: &Object, s: &BlendSpec) -> (Object, BlendReport) {
    let (after, report) = obj.blend_edges(s).expect("blend succeeds");
    assert_valid_solid(&after);
    for &f in &report.new_faces {
        let face = &after.faces()[f];
        assert!(face.blend.is_some());
        if let Some(claim) = face.surface {
            for p in after.loop_positions(face.outer_loop) {
                assert!(claim.distance(p).abs() <= kernel::tol::PLANE_DIST);
            }
        }
    }
    (after, report)
}

fn all_edges(obj: &Object) -> Vec<EdgeId> {
    obj.edges().keys().collect()
}

fn corner_edges(obj: &Object, c: Point3, others: [Point3; 3]) -> Vec<EdgeId> {
    others.iter().map(|&o| edge_between(obj, c, o)).collect()
}

fn spheres(obj: &Object) -> Vec<SurfaceRef> {
    obj.faces()
        .values()
        .filter_map(|f| f.surface)
        .filter(|s| matches!(s, SurfaceRef::Sphere { .. }))
        .collect()
}

#[test]
fn rolled_box_corner_counts() {
    let obj = unit_box();
    let (after, _) = blend_ok(
        &obj,
        &spec(all_edges(&obj), round(0.1, 6), CornerStyle::Default),
    );
    assert_eq!(after.faces().len(), 366);
    let sph = spheres(&after);
    assert_eq!(sph.len(), 8 * 36);
    let mut groups: Vec<SurfaceRef> = Vec::new();
    for s in sph {
        if !groups.iter().any(|g| g.same_surface(&s)) {
            groups.push(s);
        }
    }
    assert_eq!(groups.len(), 8);
    // On a box corner the fan rows are latitude circles: every quad is
    // planar, so no fan face was split into triangles beyond the pole row.
    let corner_tris = after
        .faces()
        .values()
        .filter(|f| {
            f.blend.is_some_and(|t| t.role == BlendRole::Corner)
                && after.loop_positions(f.outer_loop).count() == 3
        })
        .count();
    assert_eq!(corner_tris, 8 * 6);
}

#[test]
fn rolled_corner_non_square() {
    let obj = wedge(PI / 3.0);
    let o = p3(0.0, 0.0, 0.0);
    let edges = corner_edges(
        &obj,
        o,
        [
            p3(0.0, 0.0, 1.0),
            p3(1.0, 0.0, 0.0),
            p3((PI / 3.0).cos(), (PI / 3.0).sin(), 0.0),
        ],
    );
    let (after, _) = blend_ok(&obj, &spec(edges, round(0.05, 4), CornerStyle::Default));
    assert!(!spheres(&after).is_empty());
}

/// A unit box with an open-topped box pocket `[0.25, 0.75]² × [0.5, 1]`.
fn box_pocket() -> Object {
    let b = unit_box();
    let mut c = box_xyz(0.5, 0.5, 1.0);
    c.apply_transform(&Transform::translation(Vec3::new(0.25, 0.25, 0.5)))
        .unwrap();
    Object::boolean(BooleanOp::Subtract, &b, &c, &Transform::IDENTITY).unwrap()
}

#[test]
fn concave_rolled_corner() {
    let obj = box_pocket();
    let c = p3(0.25, 0.25, 0.5);
    let edges = corner_edges(
        &obj,
        c,
        [
            p3(0.75, 0.25, 0.5),
            p3(0.25, 0.75, 0.5),
            p3(0.25, 0.25, 1.0),
        ],
    );
    let (after, _) = blend_ok(&obj, &spec(edges, round(0.05, 4), CornerStyle::Default));
    assert!(!spheres(&after).is_empty());
    assert!(signed_volume(&after) > signed_volume(&obj));
}

fn box_corner(obj: &Object) -> Vec<EdgeId> {
    let c = p3(1.0, 1.0, 1.0);
    corner_edges(
        obj,
        c,
        [p3(0.0, 1.0, 1.0), p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 0.0)],
    )
}

#[test]
fn mitered_three_fillets() {
    let obj = unit_box();
    let (after, _) = blend_ok(
        &obj,
        &spec(box_corner(&obj), round(0.1, 4), CornerStyle::Mitered),
    );
    assert!(spheres(&after).is_empty());
}

#[test]
fn flat_chamfer_corner() {
    let obj = unit_box();
    let (after, report) = blend_ok(
        &obj,
        &spec(box_corner(&obj), chamfer(0.1), CornerStyle::Flat),
    );
    let corners: Vec<_> = report
        .new_faces
        .iter()
        .filter(|&&f| {
            after.faces()[f]
                .blend
                .is_some_and(|t| t.role == BlendRole::Corner && t.kind == BlendKind::Chamfer)
        })
        .collect();
    assert_eq!(corners.len(), 1);
    assert_eq!(
        after
            .loop_positions(after.faces()[*corners[0]].outer_loop)
            .count(),
        3
    );
}

/// The inner top corner of the L-block at (1, 1, 1): the concave vertical
/// edge and the two top edges meeting it.
fn l_inner(obj: &Object) -> Vec<EdgeId> {
    let c = p3(1.0, 1.0, 1.0);
    corner_edges(
        obj,
        c,
        [p3(1.0, 1.0, 0.0), p3(2.0, 1.0, 1.0), p3(1.0, 2.0, 1.0)],
    )
}

#[test]
fn two_pass_l_inner_corner() {
    let obj = l_block();
    let edges = l_inner(&obj);
    let r = 0.05;
    let (after, report) = blend_ok(
        &obj,
        &spec(edges.clone(), round(r, 4), CornerStyle::Default),
    );
    // Two passes, one blend: the second pass takes the first's id, so the
    // corner is one feature to remove or resize.
    assert_eq!(report.blends.len(), 1);
    assert_eq!(after.blend_features().len(), 1);
    assert!(
        after
            .faces()
            .values()
            .any(|f| matches!(f.surface, Some(SurfaceRef::Torus { .. })))
    );
    // The same as doing the two passes by hand.
    let (o1, _) = blend_ok(
        &obj,
        &spec(vec![edges[0]], round(r, 4), CornerStyle::Default),
    );
    let along = |o: &Object, a: Point3, b: Point3| -> EdgeId {
        let d = (b - a).normalized().unwrap();
        o.edges()
            .keys()
            .find(|&e| {
                let (p, q) = o.edge_endpoints(e).unwrap();
                let on = |x: Point3| ((x - a) - d * (x - a).dot(d)).length() < 1e-9;
                on(p) && on(q) && (p.approx_eq(b, 1e-12) || q.approx_eq(b, 1e-12))
            })
            .unwrap()
    };
    let mut second: Vec<EdgeId> = Vec::new();
    for far in [p3(2.0, 1.0, 1.0), p3(1.0, 2.0, 1.0)] {
        for e in o1.logical_edge(along(&o1, p3(1.0, 1.0, 1.0), far), true) {
            if !second.contains(&e) {
                second.push(e);
            }
        }
    }
    let (by_hand, _) = blend_ok(&o1, &spec(second, round(r, 4), CornerStyle::Default));
    let points = |o: &Object| {
        let mut v: Vec<Point3> = o.vertices().values().map(|v| v.position).collect();
        v.sort_by(|a, b| {
            a.x.total_cmp(&b.x)
                .then(a.y.total_cmp(&b.y))
                .then(a.z.total_cmp(&b.z))
        });
        v
    };
    let (pa, pb) = (points(&after), points(&by_hand));
    assert_eq!(pa.len(), pb.len());
    for p in &pa {
        assert!(pb.iter().any(|q| p.approx_eq(*q, 1e-9)));
    }
}

#[test]
fn mixed_non_prismatic_refused() {
    // A tilted L: the odd (concave) edge leans, so it is not square to the
    // face the two convex edges share.
    let pts = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];
    let mut obj = prism(&pts, 1.0);
    let shear = Transform::from_affine(&[
        1.0, 0.0, 0.3, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]);
    obj.apply_transform(&shear).unwrap();
    let c = p3(1.3, 1.0, 1.0);
    let edges = corner_edges(
        &obj,
        c,
        [p3(1.0, 1.0, 0.0), p3(2.3, 1.0, 1.0), p3(1.3, 2.0, 1.0)],
    );
    assert!(matches!(
        obj.blend_edges(&spec(edges, round(0.05, 4), CornerStyle::Default))
            .unwrap_err(),
        BlendError::UnsupportedVertex { .. }
    ));
}

#[test]
fn kinked_blade_rounds_every_edge() {
    // A blade whose arc tip meets its sides at a slight kink (a bulge
    // placed by eye): each kink is a corner of three planar facets, one a
    // cylinder facet, and rolls a ball like a box corner (D-45).
    for tip_x in [1.305, 1.25] {
        let blade = fan_blade(tip_x);
        let before = signed_volume(&blade);
        for r in [0.002, 0.001] {
            let (after, report) = blend_ok(
                &blade,
                &spec(blade.sharp_edges(), round(r, 3), CornerStyle::Default),
            );
            assert_eq!(report.blends.len(), 1);
            assert!(signed_volume(&after) < before);
            assert!(!spheres(&after).is_empty(), "rolled corners claim spheres");
        }
    }
    // The tangent tip (no kink) still rounds.
    let blade = fan_blade(1.3);
    blend_ok(
        &blade,
        &spec(blade.sharp_edges(), round(0.002, 3), CornerStyle::Default),
    );
}

#[test]
fn half_thickness_round_names_its_real_limit() {
    // Rounding a 6 mm blade's every edge at 3 mm makes the top and bottom
    // fillets meet: refused, with the true maximum, not zero.
    let blade = fan_blade(1.3);
    match blade.blend_edges(&spec(
        blade.sharp_edges(),
        round(0.003, 3),
        CornerStyle::Default,
    )) {
        Err(BlendError::RadiusTooLarge { max, .. }) => {
            assert!(max > 0.0025 && max < 0.003, "max {max}");
        }
        other => panic!("expected RadiusTooLarge, got {other:?}"),
    }
}

/// The hard edges whose two faces both belong to a blend.
fn hard_inside_blends(obj: &Object) -> Vec<EdgeId> {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_faces_for_test(e).unwrap();
            obj.faces()[a].blend.is_some()
                && obj.faces()[b].blend.is_some()
                && obj.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
        })
        .collect()
}

#[test]
fn rolled_corners_draw_no_line_and_a_two_fillet_corner_keeps_its_crease() {
    // Every edge rounded: each corner is a rolled ball, smooth into its three
    // strips, so nothing inside the rounding is drawn as a hard line.
    let obj = prism(&[(0.0, 0.0), (0.3, 0.0), (0.3, 0.2), (0.0, 0.2)], 0.1);
    let (all, _) = blend_ok(
        &obj,
        &spec(obj.sharp_edges(), round(0.01, 6), CornerStyle::Default),
    );
    assert!(hard_inside_blends(&all).is_empty());
    // Only the top rim rounded: at each top corner two strips meet over the
    // still-sharp vertical edge, so they meet in a real crease (one per
    // corner, the vertical edge carried up through the rounding).
    let top = obj.face_loop_edges(
        face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0)),
        p3(0.15, 0.1, 0.1),
    );
    let (rim, _) = blend_ok(&obj, &spec(top, round(0.01, 6), CornerStyle::Default));
    let creases = hard_inside_blends(&rim);
    assert_eq!(
        creases.len(),
        4 * 6,
        "four corners, one seam of six segments each"
    );
}
