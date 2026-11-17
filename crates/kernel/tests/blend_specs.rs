//! Executable specs for the blend engine core (WP05): single-edge fillets
//! and chamfers, prismatic miters, the convex clip, pass-through chains,
//! the size limit and every refusal.

use crate::common;

use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendReport, BlendSpec, CornerStyle, EdgeId, EdgeSmoothness, FaceId,
    LimitFeature, Object, Point3, RadiusHandle, SurfaceRef, Vec3,
};

fn round(radius: f64, segments: u32) -> BlendProfile {
    BlendProfile::Round { radius, segments }
}

fn chamfer(a: f64, b: f64) -> BlendProfile {
    BlendProfile::Chamfer {
        setback_a: a,
        setback_b: b,
    }
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

/// Blends and checks the invariants every success shares: a valid solid,
/// every new face tagged as a strip of the one blend, every claimed strip
/// facet on its surface.
fn blend_ok(obj: &Object, s: &BlendSpec) -> (Object, BlendReport) {
    let (after, report) = obj.blend_edges(s).expect("blend succeeds");
    assert_valid_solid(&after);
    assert_eq!(report.blends.len(), 1);
    let id = report.blends[0];
    assert!(!report.new_faces.is_empty());
    for &f in &report.new_faces {
        let face = &after.faces()[f];
        let tag = face.blend.expect("new faces carry the blend tag");
        assert_eq!(tag.id, id);
        assert!(matches!(
            tag.role,
            kernel::BlendRole::Strip | kernel::BlendRole::Corner
        ));
        if let Some(claim) = face.surface {
            for p in after.loop_positions(face.outer_loop) {
                assert!(
                    claim.distance(p).abs() <= kernel::tol::PLANE_DIST,
                    "strip vertex {p:?} is on its claim"
                );
            }
        }
    }
    // Untouched faces carry no tag.
    let new: BTreeSet<FaceId> = report.new_faces.iter().copied().collect();
    for (f, face) in after.faces() {
        if !new.contains(&f) {
            assert!(face.blend.is_none());
        }
    }
    (after, report)
}

/// Edges of `obj` running parallel to `dir` between a blend face and a
/// face that is not one (the rails).
fn rails(obj: &Object, new_faces: &[FaceId], dir: Vec3) -> Vec<EdgeId> {
    let new: BTreeSet<FaceId> = new_faces.iter().copied().collect();
    obj.edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            let d = (b - a).normalized().unwrap();
            if d.dot(dir).abs() < 1.0 - 1e-9 {
                return false;
            }
            let faces = obj.edge_faces_for_test(e).unwrap();
            new.contains(&faces.0) != new.contains(&faces.1)
        })
        .collect()
}

/// The polygonal fillet's removed area per unit length: the kite between
/// the corner and the arc center less the N triangles of the polygon.
fn fillet_area(r: f64, theta: f64, n: usize) -> f64 {
    let phi = PI - theta;
    r * r * ((phi / 2.0).tan() - (n as f64 / 2.0) * (phi / n as f64).sin())
}

fn top_front_edge(obj: &Object, x: f64, z: f64) -> EdgeId {
    edge_between(obj, p3(0.0, 0.0, z), p3(x, 0.0, z))
}

// ------------------------------------------------------------ run-outs

#[test]
fn box_edge_fillet_counts() {
    let obj = unit_box();
    let e = top_front_edge(&obj, 1.0, 1.0);
    let (after, report) = blend_ok(&obj, &spec(vec![e], round(0.1, 6)));
    assert_eq!(after.faces().len(), 12);
    assert_eq!(report.new_faces.len(), 6);
    let claims: Vec<SurfaceRef> = report
        .new_faces
        .iter()
        .map(|&f| after.faces()[f].surface.expect("each facet claims"))
        .collect();
    assert!(matches!(claims[0], SurfaceRef::Cylinder { .. }));
    assert!(claims.iter().all(|c| c.same_surface(&claims[0])));
    // The run-outs into both end faces carry arc curves.
    let curved = after.edges().values().filter(|e| e.curve.is_some()).count();
    assert_eq!(curved, 12);
    for r in rails(&after, &report.new_faces, Vec3::new(1.0, 0.0, 0.0)) {
        assert_eq!(after.edge_smoothness(r), EdgeSmoothness::Tangent);
    }
    // Interior rulings are soft.
    let soft = after.edges().values().filter(|e| e.soft).count();
    assert_eq!(soft, 5);
}

#[test]
fn box_edge_fillet_volume() {
    let obj = unit_box();
    let e = top_front_edge(&obj, 1.0, 1.0);
    let (after, _) = blend_ok(&obj, &spec(vec![e], round(0.1, 6)));
    let delta = signed_volume(&obj) - signed_volume(&after);
    assert!(
        (delta - fillet_area(0.1, FRAC_PI_2, 6)).abs() <= 1e-12,
        "{delta}"
    );
}

#[test]
fn box_edge_chamfer_asymmetric() {
    let obj = unit_box();
    let e = top_front_edge(&obj, 1.0, 1.0);
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    let mut s = spec(vec![e], chamfer(0.1, 0.05));
    s.primary = Some(top);
    let (after, report) = blend_ok(&obj, &s);
    assert_eq!(report.new_faces.len(), 1);
    let ring: Vec<Point3> = after
        .loop_positions(after.faces()[report.new_faces[0]].outer_loop)
        .collect();
    assert_eq!(ring.len(), 4);
    for p in &ring {
        let on_top = (p.z - 1.0).abs() < 1e-12 && (p.y - 0.1).abs() < 1e-12;
        let on_front = p.y.abs() < 1e-12 && (p.z - 0.95).abs() < 1e-12;
        assert!(on_top || on_front, "{p:?}");
    }
    for r in rails(&after, &report.new_faces, Vec3::new(1.0, 0.0, 0.0)) {
        assert_eq!(after.edge_smoothness(r), EdgeSmoothness::Hard);
    }
}

#[test]
fn wedge_fillets_at_60_and_120() {
    for theta in [PI / 3.0, 2.0 * PI / 3.0] {
        let obj = wedge(theta);
        // The vertical edge at the origin.
        let e = edge_between(&obj, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0));
        let r = 0.1;
        let (after, report) = blend_ok(&obj, &spec(vec![e], round(r, 8)));
        let setback = r / (theta / 2.0).tan();
        // A rail point on the x axis face sits at the setback.
        let has_rail = after
            .vertices()
            .values()
            .any(|v| (v.position.x - setback).abs() < 1e-12 && v.position.y.abs() < 1e-12);
        assert!(has_rail, "theta {theta}");
        // The arc sweeps π − θ: the claims' normals at the rails differ by it.
        let claim = after.faces()[report.new_faces[0]].surface.unwrap();
        assert!(matches!(claim, SurfaceRef::Cylinder { .. }));
        let delta = signed_volume(&obj) - signed_volume(&after);
        assert!((delta - fillet_area(r, theta, 8)).abs() <= 1e-12, "{delta}");
    }
}

#[test]
fn concave_inner_edge_fillet() {
    let obj = l_block();
    // The reflex vertical edge at (1, 1).
    let e = edge_between(&obj, p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 1.0));
    let (after, report) = blend_ok(&obj, &spec(vec![e], round(0.1, 6)));
    assert_eq!(report.new_faces.len(), 6);
    let delta = signed_volume(&after) - signed_volume(&obj);
    assert!(
        (delta - fillet_area(0.1, FRAC_PI_2, 6)).abs() <= 1e-12,
        "material is added: {delta}"
    );
    let curved = after.edges().values().filter(|e| e.curve.is_some()).count();
    assert_eq!(curved, 12, "run-outs into the top and bottom");
}

#[test]
fn concave_inner_edge_chamfer() {
    let obj = l_block();
    let e = edge_between(&obj, p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 1.0));
    let (after, _) = blend_ok(&obj, &spec(vec![e], chamfer(0.1, 0.1)));
    let delta = signed_volume(&after) - signed_volume(&obj);
    assert!((delta - 0.005).abs() <= 1e-12, "{delta}");
}

// --------------------------------------------------------------- miters

/// The removed section of a quarter round (corner at the origin, inset h
/// along the face, depth along the wall): area and the inset of its
/// centroid.
fn quarter_section(r: f64, n: usize) -> (f64, f64) {
    let mut poly = vec![(0.0, 0.0)];
    for k in 0..=n {
        let a = FRAC_PI_2 * k as f64 / n as f64;
        poly.push((r - r * a.sin(), r - r * a.cos()));
    }
    let (mut area, mut cx) = (0.0, 0.0);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let cr = a.0 * b.1 - b.0 * a.1;
        area += cr;
        cx += (a.0 + b.0) * cr;
    }
    let area = area / 2.0;
    (area.abs(), cx / (6.0 * area))
}

#[test]
fn top_rim_prismatic_miter() {
    let obj = box_xyz(2.0, 1.0, 1.0);
    let top = [
        p3(0.0, 0.0, 1.0),
        p3(2.0, 0.0, 1.0),
        p3(2.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
    let edges: Vec<EdgeId> = (0..4)
        .map(|i| edge_between(&obj, top[i], top[(i + 1) % 4]))
        .collect();
    let (r, n) = (0.1, 6);
    let (after, report) = blend_ok(&obj, &spec(edges, round(r, n as u32)));
    assert_eq!(after.faces().len(), 6 + 4 * n);
    assert_eq!(report.new_faces.len(), 4 * n);
    // The vertical edges now end at z = 1 − r.
    let tops: Vec<f64> = after
        .edges()
        .keys()
        .filter_map(|e| {
            let (a, b) = after.edge_endpoints(e).unwrap();
            ((a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12).then(|| a.z.max(b.z))
        })
        .collect();
    assert_eq!(tops.len(), 4);
    assert!(tops.iter().all(|z| (z - (1.0 - r)).abs() < 1e-12));
    let (s, u) = quarter_section(r, n);
    let expected: f64 = [2.0, 1.0, 2.0, 1.0]
        .iter()
        .map(|l| l * s - 2.0 * s * u)
        .sum();
    let delta = signed_volume(&obj) - signed_volume(&after);
    assert!((delta - expected).abs() <= 1e-12, "{delta} vs {expected}");
}

#[test]
fn top_rim_chamfer_miter() {
    let obj = unit_box();
    let top = [
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
    let edges: Vec<EdgeId> = (0..4)
        .map(|i| edge_between(&obj, top[i], top[(i + 1) % 4]))
        .collect();
    let (after, report) = blend_ok(&obj, &spec(edges, chamfer(0.1, 0.1)));
    assert_eq!(report.new_faces.len(), 4);
    assert_eq!(after.faces().len(), 10);
}

#[test]
fn l_block_reflex_rim_miter() {
    let obj = l_block();
    let ring = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];
    let edges: Vec<EdgeId> = (0..6)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % 6]);
            edge_between(&obj, p3(a.0, a.1, 1.0), p3(b.0, b.1, 1.0))
        })
        .collect();
    let (after, report) = blend_ok(&obj, &spec(edges, round(0.1, 4)));
    assert_eq!(report.new_faces.len(), 24);
    assert!(signed_volume(&after) < signed_volume(&obj));
}

// ----------------------------------------------------------- the clip

#[test]
fn three_chamfers_meet_at_apex() {
    let obj = unit_box();
    let c = p3(1.0, 1.0, 1.0);
    let edges = vec![
        edge_between(&obj, c, p3(0.0, 1.0, 1.0)),
        edge_between(&obj, c, p3(1.0, 0.0, 1.0)),
        edge_between(&obj, c, p3(1.0, 1.0, 0.0)),
    ];
    let (after, _) = blend_ok(&obj, &spec(edges, chamfer(0.1, 0.1)));
    let apex = p3(0.95, 0.95, 0.95);
    assert!(
        after
            .vertices()
            .values()
            .any(|v| v.position.approx_eq(apex, 1e-12)),
        "the mitred apex"
    );
}

#[test]
fn pyramid_apex_chamfer_clip() {
    let obj = pyramid4();
    let apex = p3(0.0, 0.0, 1.0);
    let edges: Vec<EdgeId> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&obj, apex, p3(x, y, 0.0)))
        .collect();
    let (after, report) = blend_ok(&obj, &spec(edges, chamfer(0.05, 0.05)));
    assert_eq!(report.new_faces.len(), 4);
    assert!(signed_volume(&after) < signed_volume(&obj));
}

#[test]
fn box_corner_three_rounds_roll() {
    // Three rounds at one corner roll into a sphere fan (WP06 §5).
    let obj = unit_box();
    let c = p3(1.0, 1.0, 1.0);
    let edges = vec![
        edge_between(&obj, c, p3(0.0, 1.0, 1.0)),
        edge_between(&obj, c, p3(1.0, 0.0, 1.0)),
        edge_between(&obj, c, p3(1.0, 1.0, 0.0)),
    ];
    let (after, _) = blend_ok(&obj, &spec(edges, round(0.1, 4)));
    assert!(signed_volume(&after) < signed_volume(&obj));
}

// -------------------------------------------------------- pass-through

/// A unit box whose top face carries the imprinted rectangle
/// `[0.3, 0.6] × [0, 0.4]` touching the front top edge.
fn box_with_top_imprint() -> Object {
    let pts = [
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
        p3(0.3, 0.0, 1.0),
        p3(0.6, 0.0, 1.0),
        p3(0.6, 0.4, 1.0),
        p3(0.3, 0.4, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 8, 11, 10, 9, 5, 6, 7],
        vec![8, 9, 10, 11],
        vec![0, 1, 5, 9, 8, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    Object::from_polygons(&pts, &faces).unwrap()
}

#[test]
fn pass_through_with_imprint() {
    let obj = box_with_top_imprint();
    assert_valid_solid(&obj);
    let edges = vec![
        edge_between(&obj, p3(0.0, 0.0, 1.0), p3(0.3, 0.0, 1.0)),
        edge_between(&obj, p3(0.3, 0.0, 1.0), p3(0.6, 0.0, 1.0)),
        edge_between(&obj, p3(0.6, 0.0, 1.0), p3(1.0, 0.0, 1.0)),
    ];
    let r = 0.1;
    let (after, report) = blend_ok(&obj, &spec(edges, round(r, 4)));
    // One chain of three segments: 3 × 4 facets.
    assert_eq!(report.new_faces.len(), 12);
    // The imprint's side edges now start at the rail.
    for x in [0.3, 0.6] {
        edge_between(&after, p3(x, r, 1.0), p3(x, 0.4, 1.0));
    }
    let delta = signed_volume(&obj) - signed_volume(&after);
    assert!((delta - fillet_area(r, FRAC_PI_2, 4)).abs() <= 1e-12);
}

/// A unit box whose front face carries a trapezoid imprint from the front
/// top edge at x ∈ [0.3, 0.6] down to z = 0.5 at x ∈ [`b0`, `b1`]: its
/// slanted sides cross a round's front rail beside the stations. The
/// numbering makes the top face side A of every front top edge
/// (`top_side_a`) or the front faces side A of every one.
fn box_with_front_trapezoid(b0: f64, b1: f64, top_side_a: bool) -> (Object, Vec<EdgeId>) {
    let named = [
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
        p3(0.3, 0.0, 1.0),
        p3(0.6, 0.0, 1.0),
        p3(b0, 0.0, 0.5),
        p3(b1, 0.0, 0.5),
    ];
    // An edge's first face is the one whose loop runs it from the lower
    // point index to the higher: number the top edge's points along x, or
    // against it.
    let order: [usize; 12] = if top_side_a {
        [4, 8, 9, 5, 0, 1, 2, 3, 6, 7, 10, 11]
    } else {
        [5, 9, 8, 4, 0, 1, 2, 3, 6, 7, 10, 11]
    };
    let mut index = [0usize; 12];
    for (k, &n) in order.iter().enumerate() {
        index[n] = k;
    }
    let pts: Vec<Point3> = order.iter().map(|&n| named[n]).collect();
    let faces: Vec<Vec<usize>> = [
        vec![4, 8, 9, 5, 6, 7],
        vec![0, 1, 5, 9, 11, 10, 8, 4],
        vec![9, 8, 10, 11],
        vec![0, 3, 2, 1],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
    .iter()
    .map(|f| f.iter().map(|&n| index[n]).collect())
    .collect();
    let obj = Object::from_polygons(&pts, &faces).unwrap();
    assert_valid_solid(&obj);
    let edges: Vec<EdgeId> = [0.0, 0.3, 0.6, 1.0]
        .windows(2)
        .map(|w| edge_between(&obj, p3(w[0], 0.0, 1.0), p3(w[1], 0.0, 1.0)))
        .collect();
    let top = face_with_normal(&obj, Vec3::new(0.0, 0.0, 1.0));
    for &e in &edges {
        let (a, _) = obj.edge_faces_for_test(e).unwrap();
        assert_eq!(a == top, top_side_a, "the fixture's side A");
    }
    (obj, edges)
}

/// A round's result does not depend on which face is side A where a
/// slanted imprint on a side ends at a pass-through station: the station
/// point and the rail's crossing with the imprint lie on one rail, and the
/// face beside both runs them in its loop's order. Before, that order came
/// from distances to the face's previous loop edge, which tie for two
/// points on a rail parallel to it, and the tie broke by side A: with the
/// top as side A the patch did not stitch (`BlendDegenerate`).
#[test]
fn a_slanted_imprint_rounds_with_either_face_as_side_a() {
    for r in [0.01, 0.1, 0.2] {
        let mut results = Vec::new();
        for top_side_a in [true, false] {
            let (obj, edges) = box_with_front_trapezoid(0.2, 0.7, top_side_a);
            let (after, report) = blend_ok(&obj, &spec(edges, round(r, 4)));
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!((delta - fillet_area(r, FRAC_PI_2, 4)).abs() <= 1e-12, "{r}");
            results.push((after.faces().len(), report.new_faces.len()));
        }
        assert_eq!(results[0], results[1], "{r}");
    }
}

/// Two slanted imprint sides that cross the rail inside one strip segment,
/// one from each of its stations, land on that segment's ruling in order
/// along it. Before, each crossing kept its distance from its own station
/// (negative from the far one), so they sorted backward and the facet
/// crossed itself (`BlendDegenerate`), with either side A.
#[test]
fn two_imprint_crossings_inside_one_segment_land_in_order() {
    for r in [0.01, 0.1, 0.2] {
        for top_side_a in [true, false] {
            let (obj, edges) = box_with_front_trapezoid(0.4, 0.5, top_side_a);
            let (after, _) = blend_ok(&obj, &spec(edges, round(r, 4)));
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!(
                (delta - fillet_area(r, FRAC_PI_2, 4)).abs() <= 1e-12,
                "{r} {top_side_a}"
            );
            // The imprint's sides now start at the rail, inside the
            // middle segment.
            for (x0, x1) in [(0.3, 0.4), (0.6, 0.5)] {
                let at = |z: f64| p3(x0 + (x1 - x0) * (1.0 - z) / 0.5, 0.0, z);
                edge_between(&after, at(1.0 - r), at(0.5));
            }
        }
    }
}

/// A unit box whose top face is split by a line from the corner (0, 0, 1)
/// to `p`, a point on the right (x = 1) or back (y = 1) top edge; `None`
/// leaves the top whole. With the two top edges at that corner.
fn box_with_corner_split(p: Option<(f64, f64)>) -> (Object, Vec<EdgeId>) {
    let mut pts = vec![
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
    let mut faces = vec![vec![0, 3, 2, 1], vec![0, 1, 5, 4], vec![3, 0, 4, 7]];
    match p {
        None => faces.extend([vec![4, 5, 6, 7], vec![1, 2, 6, 5], vec![2, 3, 7, 6]]),
        Some((1.0, 1.0)) => faces.extend([
            vec![4, 5, 6],
            vec![4, 6, 7],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
        ]),
        Some((x, y)) => {
            pts.push(p3(x, y, 1.0));
            if x == 1.0 {
                faces.extend([
                    vec![4, 5, 8],
                    vec![4, 8, 6, 7],
                    vec![1, 2, 6, 8, 5],
                    vec![2, 3, 7, 6],
                ]);
            } else {
                faces.extend([
                    vec![4, 5, 6, 8],
                    vec![4, 8, 7],
                    vec![1, 2, 6, 5],
                    vec![2, 3, 7, 8, 6],
                ]);
            }
        }
    }
    let obj = Object::from_polygons(&pts, &faces).unwrap();
    assert_valid_solid(&obj);
    let edges = vec![
        edge_between(&obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0)),
        edge_between(&obj, p3(0.0, 0.0, 1.0), p3(0.0, 1.0, 1.0)),
    ];
    (obj, edges)
}

/// The split line of a blended corner, after the blend: one edge from
/// `far` (its far end, untouched) back along it, starting off the vertex.
fn shortened_split(after: &Object, far: Point3, from: Point3) -> usize {
    let d = (far - from).normalized().unwrap();
    after
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = after.edge_endpoints(e).unwrap();
            let (hit, o) = if a.approx_eq(far, 1e-9) {
                (true, b)
            } else if b.approx_eq(far, 1e-9) {
                (true, a)
            } else {
                (false, a)
            };
            hit && (a.z - b.z).abs() < 1e-12
                && (o - from).cross(d).length() < 1e-9
                && (o - from).length() > 1e-6
        })
        .count()
}

/// Two blended edges meeting at a corner cut the solid as the plain corner
/// does whether or not a coplanar line splits the face they share there,
/// wherever the line ends: the line is shortened to where it leaves the
/// cut. With equal setbacks on that face the corner miters; with unequal
/// ones (an asymmetric chamfer whose two edges put different setbacks on
/// it) it is clipped, the two pieces clipping as the one plane they lie
/// in. A line that ends inside the cut is a size limit. Before, the split
/// sent a mitered corner to the general clip, which cut less the steeper
/// the line (a 0.1 chamfer removed 0.008389 with the line to (1, 0.5) and
/// 0.008580 to (1, 0.2), against 0.009667 plain) and refused a round; and
/// an asymmetric chamfer's clipped corner refused the flat line and fell
/// to the swept blend, which cut 0.036402 at (1, 0.5) and 0.035228 at
/// (1, 0.2) for (0.3, 0.15) against 0.0421875 plain.
#[test]
fn a_coplanar_split_at_a_corner_cuts_as_the_plain_corner() {
    for profile in [
        round(0.1, 4),
        round(0.05, 4),
        chamfer(0.1, 0.1),
        chamfer(0.05, 0.05),
        chamfer(0.3, 0.15),
        chamfer(0.15, 0.3),
        chamfer(0.1, 0.05),
    ] {
        let (plain, edges) = box_with_corner_split(None);
        let top = face_with_normal(&plain, Vec3::new(0.0, 0.0, 1.0));
        let top_first: Vec<bool> = edges
            .iter()
            .map(|&e| plain.edge_faces_for_test(e).unwrap().0 == top)
            .collect();
        // The setback on the top along each edge (side A takes `setback_a`).
        let on_top: Vec<f64> = top_first
            .iter()
            .map(|&a| match profile {
                BlendProfile::Chamfer {
                    setback_a,
                    setback_b,
                } => {
                    if a {
                        setback_a
                    } else {
                        setback_b
                    }
                }
                _ => profile.size().unwrap(),
            })
            .collect();
        let (cut, plain_report) = blend_ok(&plain, &spec(edges, profile.clone()));
        let removed = signed_volume(&plain) - signed_volume(&cut);
        for p in [(1.0, 0.5), (1.0, 0.2), (0.5, 1.0), (1.0, 1.0)] {
            let (obj, edges) = box_with_corner_split(Some(p));
            // The same sides as the plain box, so the same setbacks.
            for (&e, &a) in edges.iter().zip(&top_first) {
                let first = obj.edge_faces_for_test(e).unwrap().0;
                assert_eq!(obj.faces()[first].plane.normal().z > 0.99, a, "{p:?}");
            }
            let outcome = obj.blend_edges(&spec(edges, profile.clone()));
            // The line ends inside the cut: beside the x edge closer than
            // its setback on the top, or beside the y edge.
            if p.1 < on_top[0] - 1e-9 || p.0 < on_top[1] - 1e-9 {
                match outcome {
                    Err(BlendError::RadiusTooLarge { max, limited_by }) => {
                        assert_eq!(limited_by, LimitFeature::Imprint, "{profile:?} {p:?}");
                        assert!(max > 0.0, "{profile:?} {p:?}");
                    }
                    other => panic!("{profile:?} {p:?}: {:?}", other.map(|_| ())),
                }
                continue;
            }
            let (after, report) = outcome.unwrap_or_else(|e| panic!("{profile:?} {p:?}: {e:?}"));
            assert_valid_solid(&after);
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!(
                (delta - removed).abs() <= 1e-12,
                "{profile:?} {p:?}: {delta} vs {removed}"
            );
            assert_eq!(
                report.new_faces.len(),
                plain_report.new_faces.len(),
                "{profile:?} {p:?}"
            );
            let far = p3(p.0, p.1, 1.0);
            assert_eq!(
                shortened_split(&after, far, p3(0.0, 0.0, 1.0)),
                1,
                "{profile:?} {p:?}: the split line is shortened"
            );
        }
    }
}

/// An L prism (the footprint (0,0), (2,0), (2,1), (1,1), (1,2), (0,2), 1
/// high) whose top is split by a line from the re-entrant corner (1,1) to
/// (`px`, 0); `None` leaves the top whole. With the two top edges at the
/// re-entrant corner.
fn l_prism_with_split(px: Option<f64>) -> (Object, Vec<EdgeId>) {
    let foot = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];
    let mut pts: Vec<Point3> = foot.iter().map(|&(x, y)| p3(x, y, 0.0)).collect();
    pts.extend(foot.iter().map(|&(x, y)| p3(x, y, 1.0)));
    let mut faces: Vec<Vec<usize>> = vec![vec![5, 4, 3, 2, 1, 0]];
    // Walls: the front (y = 0) carries the split point on its top edge.
    for i in 1..6 {
        let j = (i + 1) % 6;
        faces.push(vec![i, j, 6 + j, 6 + i]);
    }
    match px {
        None => {
            faces.push(vec![0, 1, 7, 6]);
            faces.push(vec![6, 7, 8, 9, 10, 11]);
        }
        Some(x) => {
            pts.push(p3(x, 0.0, 1.0));
            faces.push(vec![0, 1, 7, 12, 6]);
            faces.push(vec![12, 7, 8, 9]);
            faces.push(vec![9, 10, 11, 6, 12]);
        }
    }
    let obj = Object::from_polygons(&pts, &faces).unwrap();
    assert_valid_solid(&obj);
    let edges = vec![
        edge_between(&obj, p3(2.0, 1.0, 1.0), p3(1.0, 1.0, 1.0)),
        edge_between(&obj, p3(1.0, 1.0, 1.0), p3(1.0, 2.0, 1.0)),
    ];
    (obj, edges)
}

/// At a re-entrant corner of the face two blended edges share, a line from
/// the corner splitting that face leaves the cut behind the corner, on the
/// part of a strip's rail that reaches back past the vertex to the miter:
/// the corner cuts as the plain L does, wherever the line ends. Before, the
/// crossing was bounded by the strip's edge alone, so every line ending
/// short of x = 1 named a limit of 0 (`RadiusTooLarge`, `Imprint`).
#[test]
fn a_split_at_a_reentrant_corner_cuts_as_the_plain_corner() {
    for profile in [round(0.1, 4), chamfer(0.1, 0.1)] {
        let (plain, edges) = l_prism_with_split(None);
        let (cut, plain_report) = blend_ok(&plain, &spec(edges, profile.clone()));
        let removed = signed_volume(&plain) - signed_volume(&cut);
        for px in [0.05, 0.5, 0.95, 1.0, 1.5, 1.95] {
            let (obj, edges) = l_prism_with_split(Some(px));
            let (after, report) = blend_ok(&obj, &spec(edges, profile.clone()));
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!(
                (delta - removed).abs() <= 1e-12,
                "{profile:?} {px}: {delta} vs {removed}"
            );
            assert_eq!(report.new_faces.len(), plain_report.new_faces.len(), "{px}");
            assert_eq!(
                shortened_split(&after, p3(px, 0.0, 1.0), p3(1.0, 1.0, 1.0)),
                1,
                "{profile:?} {px}: the split line is shortened"
            );
        }
    }
}

/// A radius ramp's strip runs its rail from the corner's cut to the far
/// station, askew to the edge: a split line at the corner leaves the cut
/// on that rail, and the corner cuts as the plain one does, ramping up or
/// down, wherever the line ends. Before, the crossing was found along the
/// edge's direction, off the rail, and the corner refused
/// (`BlendDegenerate`, with limits of a few hundredths of a millimeter).
#[test]
fn a_split_corner_on_a_radius_ramp_cuts_as_the_plain_corner() {
    for ramp in [[(0.0, 0.05), (1.0, 0.1)], [(0.0, 0.1), (1.0, 0.05)]] {
        let with_ramp = |edges: Vec<EdgeId>| {
            let mut s = spec(edges, round(0.1, 4));
            s.handles = ramp
                .iter()
                .map(|&(at, radius)| RadiusHandle { at, radius })
                .collect();
            s
        };
        let (plain, edges) = box_with_corner_split(None);
        let (cut, plain_report) = blend_ok(&plain, &with_ramp(edges));
        let removed = signed_volume(&plain) - signed_volume(&cut);
        for p in [(1.0, 0.5), (0.5, 1.0), (1.0, 1.0)] {
            let (obj, edges) = box_with_corner_split(Some(p));
            let (after, report) = blend_ok(&obj, &with_ramp(edges));
            let delta = signed_volume(&obj) - signed_volume(&after);
            assert!(
                (delta - removed).abs() <= 1e-12,
                "{ramp:?} {p:?}: {delta} vs {removed}"
            );
            assert_eq!(
                report.new_faces.len(),
                plain_report.new_faces.len(),
                "{p:?}"
            );
            assert_eq!(
                shortened_split(&after, p3(p.0, p.1, 1.0), p3(0.0, 0.0, 1.0)),
                1,
                "{ramp:?} {p:?}: the split line is shortened"
            );
        }
    }
}

// -------------------------------------------------------------- refusals

#[test]
fn edge_is_smooth_refused() {
    let obj = cylinder(1.0, 1.0, 16);
    let seam = obj
        .edges()
        .keys()
        .find(|&e| obj.edge_smoothness(e) == EdgeSmoothness::Seam)
        .expect("a seam edge");
    assert_eq!(
        obj.blend_edges(&spec(vec![seam], round(0.1, 4)))
            .unwrap_err(),
        BlendError::EdgeIsSmooth
    );
}

#[test]
fn non_coaxial_cylinders_refused() {
    // Where two crossing cylinders meet, neither side's axis makes the edge
    // a rim and neither runs along it: outside the catalogue, so the swept
    // blend's (WP07 §4). One edge of the loop is an open chain whose ends
    // are not on a face (the next junction edge, unpicked, goes on from
    // each), so it refuses at its end (D-44, D-115), object untouched.
    let a = cylinder(0.5, 2.0, 24);
    let mut b = cylinder(0.3, 2.0, 24);
    b.apply_transform(
        &kernel::Transform::rotation(Vec3::new(1.0, 0.0, 0.0), FRAC_PI_2)
            .unwrap()
            .then(&kernel::Transform::translation(Vec3::new(0.0, 1.0, 1.0))),
    )
    .unwrap();
    let obj = Object::boolean(
        kernel::BooleanOp::Union,
        &a,
        &b,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let edge = obj
        .edges()
        .keys()
        .find(|&e| {
            let (x, y) = obj.edge_faces_for_test(e).unwrap();
            match (obj.faces()[x].surface, obj.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .expect("an edge between the two walls");
    let before = obj.to_polygons();
    assert!(matches!(
        obj.blend_edges(&spec(vec![edge], round(0.05, 4)))
            .unwrap_err(),
        BlendError::UnsupportedVertex { .. }
    ));
    assert_eq!(obj.to_polygons(), before);
}

#[test]
fn radius_too_large_names_limit() {
    let obj = box_xyz(1.0, 1.0, 0.1);
    let e = top_front_edge(&obj, 1.0, 0.1);
    match obj.blend_edges(&spec(vec![e], round(0.2, 4))).unwrap_err() {
        BlendError::RadiusTooLarge { max, limited_by } => {
            assert!((max - 0.1).abs() <= 2e-9, "{max}");
            assert_eq!(limited_by, LimitFeature::NeighborEdge);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn hole_limits_radius() {
    let obj = plate_with_hole(1.0, 1.0, 0.3, 24);
    let e = edge_between(&obj, p3(-0.5, -0.5, 1.0), p3(0.5, -0.5, 1.0));
    match obj.blend_edges(&spec(vec![e], round(0.3, 4))).unwrap_err() {
        BlendError::RadiusTooLarge { max, limited_by } => {
            assert_eq!(limited_by, LimitFeature::Hole);
            assert!(max > 0.15 && max < 0.2 + 1e-9, "{max}");
        }
        other => panic!("{other:?}"),
    }
}

/// A unit box with a sealed rectangular void running along x just inside
/// its front top edge: `[0.1, 0.9] × [0.05, 0.1] × [0.9, 0.95]`.
fn box_with_void() -> Object {
    let mut pts = vec![
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
    let (x0, x1, y0, y1, z0, z1) = (0.1, 0.9, 0.05, 0.1, 0.9, 0.95);
    pts.extend([
        p3(x0, y0, z0),
        p3(x1, y0, z0),
        p3(x1, y1, z0),
        p3(x0, y1, z0),
        p3(x0, y0, z1),
        p3(x1, y0, z1),
        p3(x1, y1, z1),
        p3(x0, y1, z1),
    ]);
    let outer = [
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let mut faces: Vec<Vec<usize>> = outer.to_vec();
    // The void's faces wind inward (reversed).
    for f in &outer {
        faces.push(f.iter().rev().map(|i| i + 8).collect());
    }
    Object::from_polygons(&pts, &faces).unwrap()
}

#[test]
fn obstruction_refused() {
    let obj = box_with_void();
    assert_valid_solid(&obj);
    let e = top_front_edge(&obj, 1.0, 1.0);
    blend_ok(&obj, &spec(vec![e], round(0.1, 8)));
    assert!(matches!(
        obj.blend_edges(&spec(vec![e], round(0.2, 8))).unwrap_err(),
        BlendError::Obstructed { .. }
    ));
}

#[test]
fn segments_out_of_range() {
    let obj = unit_box();
    let e = top_front_edge(&obj, 1.0, 1.0);
    for n in [1, 97] {
        assert_eq!(
            obj.blend_edges(&spec(vec![e], round(0.1, n))).unwrap_err(),
            BlendError::SegmentsOutOfRange { segments: n }
        );
    }
}

#[test]
fn strong_guarantee() {
    let cases: Vec<(Object, BlendSpec)> = {
        let b = box_xyz(1.0, 1.0, 0.1);
        let e = top_front_edge(&b, 1.0, 0.1);
        let v = box_with_void();
        let ev = top_front_edge(&v, 1.0, 1.0);
        let c = cylinder(1.0, 1.0, 16);
        let seam = c
            .edges()
            .keys()
            .find(|&x| c.edge_smoothness(x) == EdgeSmoothness::Seam)
            .unwrap();
        vec![
            (b.clone(), spec(vec![e], round(0.2, 4))),
            (b, spec(vec![e], round(0.1, 1))),
            (v, spec(vec![ev], round(0.2, 8))),
            (c, spec(vec![seam], round(0.1, 4))),
        ]
    };
    for (obj, s) in cases {
        let before = obj.to_polygons();
        assert!(obj.blend_edges(&s).is_err());
        assert_eq!(obj.to_polygons(), before);
    }
}
