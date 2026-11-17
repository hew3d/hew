//! Executable specs for the extended blends (WP07 §1–3, §5): coves,
//! bullnoses, router profiles, stopped and variable-radius fillets.

use crate::common;

use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI};

use common::curves::*;
use kernel::{
    BlendError, BlendKind, BlendProfile, BlendReport, BlendRole, BlendSpec, BlendStops,
    CornerStyle, CurveGeom, EdgeId, EdgeSmoothness, Object, Point3, ProfileSeg, ProfileShape,
    RadiusHandle, SurfaceRef, Vec3,
};
use proptest::prelude::*;

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
        assert!(face.blend.is_some());
        if let Some(claim) = face.surface {
            for p in after.loop_positions(face.outer_loop) {
                assert!(
                    claim.distance(p).abs() <= kernel::tol::PLANE_DIST,
                    "{p:?} {claim:?}"
                );
            }
        }
    }
    (after, report)
}

fn front_top(obj: &Object) -> EdgeId {
    edge_between(obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0))
}

fn claims(after: &Object, report: &BlendReport) -> Vec<SurfaceRef> {
    report
        .new_faces
        .iter()
        .filter_map(|&f| after.faces()[f].surface)
        .collect()
}

fn ogee(width: f64) -> BlendProfile {
    BlendProfile::Custom {
        shape: (kernel::bundled_profiles()[0].2)(),
        width,
        arc_segments: 4,
    }
}

// ----------------------------------------------------------------- cove

#[test]
fn cove_on_box_edge() {
    let obj = unit_box();
    let (r, n) = (0.1, 6);
    let (after, report) = blend_ok(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Cove {
                radius: r,
                segments: n,
            },
        ),
    );
    let cs = claims(&after, &report);
    assert_eq!(cs.len(), n as usize);
    for c in &cs {
        match *c {
            SurfaceRef::Cylinder {
                axis_point, radius, ..
            } => {
                assert!((radius - r).abs() < 1e-12);
                assert!(axis_point.y.abs() < 1e-12 && (axis_point.z - 1.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }
    // The rails meet the faces square: hard.
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    for e in after.edges().keys() {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        if new.contains(&x) != new.contains(&y) {
            let (a, b) = after.edge_endpoints(e).unwrap();
            if (b - a).normalized().unwrap().x.abs() > 1.0 - 1e-9 {
                assert_eq!(after.edge_smoothness(e), EdgeSmoothness::Hard);
            }
        }
    }
    let removed = signed_volume(&obj) - signed_volume(&after);
    let want = (n as f64 / 2.0) * r * r * (FRAC_PI_2 / n as f64).sin();
    assert!((removed - want).abs() < 1e-12, "{removed} vs {want}");
}

#[test]
fn cove_on_rim_is_torus_major_rc() {
    let obj = cylinder(0.5, 1.0, 48);
    let rim: Vec<EdgeId> = obj
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.z > 0.5 && b.z > 0.5
        })
        .collect();
    let (after, report) = blend_ok(
        &obj,
        &spec(
            rim,
            BlendProfile::Cove {
                radius: 0.05,
                segments: 4,
            },
        ),
    );
    for c in claims(&after, &report) {
        match c {
            SurfaceRef::Torus { major_radius, .. } => assert!((major_radius - 0.5).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn cove_on_concave_refused() {
    let obj = l_block();
    let e = edge_between(&obj, p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 1.0));
    assert_eq!(
        obj.blend_edges(&spec(
            vec![e],
            BlendProfile::Cove {
                radius: 0.05,
                segments: 4
            }
        ))
        .unwrap_err(),
        BlendError::ProfileNeedsConvexEdge
    );
}

// ------------------------------------------------------------- bullnose

#[test]
fn bullnose_board() {
    let obj = box_xyz(1.0, 0.1, 0.02);
    // The two long edges of the narrow front face (y = 0).
    let edges = vec![
        edge_between(&obj, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        edge_between(&obj, p3(0.0, 0.0, 0.02), p3(1.0, 0.0, 0.02)),
    ];
    let (after, report) = blend_ok(&obj, &spec(edges, BlendProfile::Bullnose { segments: 8 }));
    // The front face is gone.
    assert!(
        after
            .faces()
            .values()
            .all(|f| f.blend.is_some() || f.plane.normal().y > -1.0 + 1e-9)
    );
    let cs = claims(&after, &report);
    assert_eq!(cs.len(), 8);
    for c in cs {
        match c {
            SurfaceRef::Cylinder { radius, .. } => assert!((radius - 0.01).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
    }
    // The end faces gained the arc points.
    let end = face_with_normal(&after, Vec3::new(-1.0, 0.0, 0.0));
    assert_eq!(
        after.loop_positions(after.faces()[end].outer_loop).count(),
        2 + 9
    );
}

#[test]
fn bullnose_non_parallel_refused() {
    // A prism whose cross-section is a house: the roof's two sides are not
    // parallel.
    let obj = prism(
        &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 1.5), (0.0, 1.0)],
        1.0,
    );
    let edges = vec![
        edge_between(&obj, p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 1.0)),
        edge_between(&obj, p3(0.5, 1.5, 0.0), p3(0.5, 1.5, 1.0)),
    ];
    assert_eq!(
        obj.blend_edges(&spec(edges, BlendProfile::Bullnose { segments: 8 }))
            .unwrap_err(),
        BlendError::BullnoseNeedsParallelSides
    );
}

// ------------------------------------------------------------- profiles

#[test]
fn bundled_profiles_validate() {
    let all = kernel::bundled_profiles();
    assert_eq!(all.len(), 7);
    for (id, _, shape) in all {
        assert!(shape().sample(4).is_ok(), "{id}");
    }
}

#[test]
fn ogee_on_box_edge_is_two_cylinder_bands() {
    let obj = unit_box();
    let (after, report) = blend_ok(&obj, &spec(vec![front_top(&obj)], ogee(0.1)));
    let cs = claims(&after, &report);
    let mut groups: Vec<SurfaceRef> = Vec::new();
    for c in cs {
        match c {
            SurfaceRef::Cylinder { radius, .. } => assert!((radius - 0.05).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
        if !groups.iter().any(|g| g.same_surface(&c)) {
            groups.push(c);
        }
    }
    assert_eq!(groups.len(), 2);
    for &f in &report.new_faces {
        assert_eq!(after.faces()[f].blend.unwrap().kind, BlendKind::Profile);
    }
}

#[test]
fn profile_on_oblique_edge_is_generated() {
    let obj = wedge(2.0 * PI / 3.0);
    let e = edge_between(&obj, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0));
    let (after, report) = blend_ok(&obj, &spec(vec![e], ogee(0.05)));
    assert!(claims(&after, &report).is_empty());
}

#[test]
fn profile_too_wide() {
    let obj = box_xyz(1.0, 1.0, 0.05);
    let e = edge_between(&obj, p3(0.0, 0.0, 0.05), p3(1.0, 0.0, 0.05));
    match obj.blend_edges(&spec(vec![e], ogee(0.1))).unwrap_err() {
        BlendError::ProfileTooWide { needed, available } => {
            assert_eq!(needed, 0.1);
            assert!(available > 0.0 && available < 0.05 + 1e-9, "{available}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn profile_from_chain() {
    // A drawn quarter arc of radius 0.03 about (0, 0.03), from (0.03, 0.03)
    // down to (0, 0): in the profile frame, from (1, 0) to (0, 1).
    let r = 0.03;
    let c = p3(0.0, 0.0, 0.0);
    let n = 6;
    let points: Vec<Point3> = (0..=n)
        .map(|k| {
            let a = FRAC_PI_2 * k as f64 / n as f64;
            p3(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect();
    let curves = vec![
        Some(CurveGeom {
            center: c,
            radius: r
        });
        n
    ];
    let (shape, width) =
        ProfileShape::from_chain(&points, &curves, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!((width - r).abs() < 1e-12);
    assert_eq!(shape.segs.len(), 1);
    assert!(matches!(shape.segs[0], ProfileSeg::Arc { .. }));
}

// ------------------------------------------------------ stops, variable

#[test]
fn stopped_fillet() {
    let obj = unit_box();
    let n = 6;
    let mut s = spec(
        vec![front_top(&obj)],
        BlendProfile::Round {
            radius: 0.1,
            segments: n,
        },
    );
    s.stops = Some(BlendStops {
        start: 0.3,
        end: 0.2,
    });
    let (after, report) = blend_ok(&obj, &s);
    let runouts = report
        .new_faces
        .iter()
        .filter(|&&f| after.faces()[f].blend.unwrap().role == BlendRole::Runout)
        .count();
    assert_eq!(runouts, 2 * n as usize);
    // The edge stays sharp from each end to R.
    edge_between(&after, p3(0.0, 0.0, 1.0), p3(0.2, 0.0, 1.0));
    edge_between(&after, p3(0.9, 0.0, 1.0), p3(1.0, 0.0, 1.0));
}

#[test]
fn stop_out_of_range() {
    let obj = unit_box();
    let mut s = spec(
        vec![front_top(&obj)],
        BlendProfile::Round {
            radius: 0.1,
            segments: 4,
        },
    );
    s.stops = Some(BlendStops {
        start: 1.5,
        end: 0.0,
    });
    assert_eq!(obj.blend_edges(&s).unwrap_err(), BlendError::StopOutOfRange);
}

#[test]
fn variable_linear_is_cone() {
    let obj = unit_box();
    let mut s = spec(
        vec![front_top(&obj)],
        BlendProfile::Round {
            radius: 0.05,
            segments: 6,
        },
    );
    s.handles = vec![
        RadiusHandle {
            at: 0.0,
            radius: 0.05,
        },
        RadiusHandle {
            at: 1.0,
            radius: 0.1,
        },
    ];
    let (after, report) = blend_ok(&obj, &s);
    let cs = claims(&after, &report);
    assert_eq!(cs.len(), 6);
    assert!(cs.iter().all(|c| matches!(c, SurfaceRef::Cone { .. })));
}

#[test]
fn variable_with_handle_is_generated() {
    let obj = unit_box();
    let mut s = spec(
        vec![front_top(&obj)],
        BlendProfile::Round {
            radius: 0.05,
            segments: 4,
        },
    );
    s.handles = vec![RadiusHandle {
        at: 0.5,
        radius: 0.12,
    }];
    let (after, report) = blend_ok(&obj, &s);
    assert!(claims(&after, &report).is_empty());
    assert!(after.edges().values().filter(|e| e.soft).count() > 4 * 16);
}

#[test]
fn an_end_handle_within_single_precision_of_the_end_is_the_end() {
    // The Fillet tool measures its chain in single precision, so an end
    // handle it places at the end arrives a few ten-millionths past or
    // short of it. Either is the end; clearly past it is still refused.
    let obj = unit_box();
    let taper = |at1: f64| {
        let mut s = spec(
            vec![front_top(&obj)],
            BlendProfile::Round {
                radius: 0.05,
                segments: 6,
            },
        );
        s.handles = vec![
            RadiusHandle {
                at: 0.0,
                radius: 0.05,
            },
            RadiusHandle {
                at: at1,
                radius: 0.1,
            },
        ];
        s
    };
    let (exact, _) = blend_ok(&obj, &taper(1.0));
    for at1 in [1.0 + 5e-8, 1.0 - 5e-8] {
        let (near, _) = blend_ok(&obj, &taper(at1));
        assert!(kernel::same_geometry_for_test(&exact, &near, 1e-9));
    }
    assert!(matches!(
        obj.blend_edges(&taper(1.0 + 1e-3)),
        Err(BlendError::StopOutOfRange)
    ));
    // The start window too, and of two handles in one window the nearer
    // wins: an exact start with radius 0.05 beats one 5e-7 inside it.
    let with_start = |at0: f64, extra: Option<(f64, f64)>| {
        let mut s = taper(1.0);
        s.handles[0].at = at0;
        if let Some((at, radius)) = extra {
            s.handles.push(RadiusHandle { at, radius });
        }
        s
    };
    for at0 in [5e-8, -5e-8] {
        let (near, _) = blend_ok(&obj, &with_start(at0, None));
        assert!(kernel::same_geometry_for_test(&exact, &near, 1e-9));
    }
    let (nearer_wins, _) = blend_ok(&obj, &with_start(0.0, Some((5e-7, 0.09))));
    assert!(kernel::same_geometry_for_test(&exact, &nearer_wins, 1e-9));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn random_profiles_on_box_edges(pick in 0usize..7, width in 0.01..0.3f64) {
        let obj = box_xyz(1.0, 1.0, 0.2);
        let e = edge_between(&obj, p3(0.0, 0.0, 0.2), p3(1.0, 0.0, 0.2));
        let profile = BlendProfile::Custom {
            shape: (kernel::bundled_profiles()[pick].2)(),
            width,
            arc_segments: 4,
        };
        let before = obj.to_polygons();
        match obj.blend_edges(&spec(vec![e], profile)) {
            Ok((after, _)) => assert_valid_solid(&after),
            Err(BlendError::ProfileTooWide { .. }) => {}
            Err(other) => prop_assert!(false, "{other:?}"),
        }
        prop_assert_eq!(obj.to_polygons(), before);
    }
}

// ------------------------------------------------------------ swept blend
//
// The WP07 §4 swept blend: chains outside the analytic catalogue, and
// faceted chains the catalogue planners refuse.

fn swept_round(edges: Vec<EdgeId>, r: f64) -> BlendSpec {
    spec(
        edges,
        BlendProfile::Round {
            radius: r,
            segments: 4,
        },
    )
}

/// Two perpendicular pipes (r 0.1 along z, r 0.07 along +y) and the loop
/// where they meet.
fn pipe_tee() -> (Object, Vec<EdgeId>) {
    use kernel::{BooleanOp, Transform};
    let mut a = cylinder(0.1, 1.0, 48);
    a.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, -0.5)))
        .unwrap();
    let mut b = cylinder(0.07, 0.5, 48);
    b.apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -FRAC_PI_2).unwrap())
        .unwrap();
    let tee = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    let edges = tee
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = tee.edge_faces_for_test(e).unwrap();
            match (tee.faces()[x].surface, tee.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .collect();
    (tee, edges)
}

/// Every edge between two strip faces of the blend is soft.
fn strip_interior_soft(after: &Object, report: &BlendReport) -> bool {
    let strip: BTreeSet<_> = report
        .new_faces
        .iter()
        .copied()
        .filter(|&f| {
            after.faces()[f]
                .blend
                .is_some_and(|t| t.role == BlendRole::Strip)
        })
        .collect();
    after.edges().keys().all(|e| {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        !(strip.contains(&x) && strip.contains(&y)) || after.edges()[e].soft
    })
}

#[test]
fn swept_blend_pipe_tee() {
    let (tee, edges) = pipe_tee();
    let (after, report) = tee.blend_edges(&swept_round(edges, 0.01)).unwrap();
    assert_valid_solid(&after);
    assert!(claims(&after, &report).is_empty());
    assert!(strip_interior_soft(&after, &report));
    // A concave fillet fills the corner.
    assert!(signed_volume(&after) > signed_volume(&tee));
}

#[test]
fn swept_blend_rails_are_tangent_edges() {
    // The strip's facets are flat and unclaimed, but where they meet the
    // pipes they are the fillet's rails: hidden with the tangent edges, not
    // drawn as creases. The pipes' own rims stay creases.
    let (tee, edges) = pipe_tee();
    let (after, report) = tee.blend_edges(&swept_round(edges, 0.03)).unwrap();
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    let mut rails = 0;
    for e in after.edges().keys() {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        let got = after.edge_smoothness(e);
        if new.contains(&x) != new.contains(&y) {
            rails += 1;
            assert_ne!(got, EdgeSmoothness::Hard, "rail {e:?}");
        } else if !new.contains(&x)
            && after.faces()[x].surface.is_none() != after.faces()[y].surface.is_none()
        {
            assert_eq!(got, EdgeSmoothness::Hard, "rim {e:?}");
        }
    }
    assert!(rails > 0);
    let tangent = after
        .edges()
        .keys()
        .filter(|&e| after.edge_smoothness(e) == EdgeSmoothness::Tangent)
        .count();
    assert!(tangent > 0);
}

#[test]
fn a_cut_through_a_fillet_keeps_its_crease() {
    // A later cut meeting a claimed fillet is a real crease, drawn and
    // pickable, even a shallow one: about 26 and 9.5 degrees here.
    for d in [0.5528, 0.578] {
        cut_through_fillet_stays_hard(d);
    }
}

#[test]
fn a_fillet_ending_on_a_nearly_flat_side_keeps_its_end_crease() {
    // The strip's end against a side face meeting the next side at 8
    // degrees: the end is a crease all the way to the rail, at any facet
    // count.
    let c = 8f64.to_radians();
    let obj = prism(
        &[
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0 + c.cos(), c.sin()),
            (1.0 + c.cos(), 1.5),
            (0.0, 1.5),
        ],
        1.0,
    );
    let e = obj
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.y.abs() < 1e-9
                && b.y.abs() < 1e-9
                && a.z > 0.999
                && b.z > 0.999
                && a.x.max(b.x) < 1.0 + 1e-9
        })
        .unwrap();
    for segments in [8, 32, 64] {
        let (after, report) = blend_ok(
            &obj,
            &spec(
                vec![e],
                BlendProfile::Round {
                    radius: 0.1,
                    segments,
                },
            ),
        );
        let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
        let mut caps = 0;
        for x in after.edges().keys() {
            let (f, g) = after.edge_faces_for_test(x).unwrap();
            let (a, b) = after.edge_endpoints(x).unwrap();
            if new.contains(&f) != new.contains(&g) && a.x > 0.99 && b.x > 0.99 {
                caps += 1;
                assert_eq!(
                    after.edge_smoothness(x),
                    EdgeSmoothness::Hard,
                    "{segments}: {a:?}-{b:?}"
                );
            }
        }
        assert!(caps >= segments as usize, "{segments}: {caps}");
    }
}

fn cut_through_fillet_stays_hard(offset: f64) {
    use kernel::{BooleanOp, Transform};
    let obj = unit_box();
    let (rounded, _) = blend_ok(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Round {
                radius: 0.3,
                segments: 8,
            },
        ),
    );
    let n = Vec3::new(
        0.0,
        -std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    );
    let mut slab = box_xyz(4.0, 4.0, 4.0);
    slab.apply_transform(&Transform::translation(Vec3::new(-2.0, -2.0, 0.0)))
        .unwrap();
    slab.apply_transform(
        &Transform::rotation(Vec3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_4).unwrap(),
    )
    .unwrap();
    slab.apply_transform(&Transform::translation(n * offset))
        .unwrap();
    let cut = Object::boolean(BooleanOp::Subtract, &rounded, &slab, &Transform::IDENTITY).unwrap();
    let mut on_cut = 0;
    for e in cut.edges().keys() {
        let (x, y) = cut.edge_faces_for_test(e).unwrap();
        let tagged = |g: kernel::FaceId| cut.faces()[g].blend.is_some();
        let (a, b) = cut.edge_endpoints(e).unwrap();
        let m = (a.to_vec() + b.to_vec()) * 0.5;
        if tagged(x) != tagged(y) && (m.dot(n) - offset).abs() < 1e-6 {
            on_cut += 1;
            assert_eq!(
                cut.edge_smoothness(e),
                EdgeSmoothness::Hard,
                "{offset}: {e:?}"
            );
            // The strip beside the cut keeps its own surface normal at the
            // cut's corners: nothing of the cut face is averaged in.
            let strip = if tagged(x) { x } else { y };
            for v in [a, b] {
                let own = cut.faces()[strip]
                    .surface
                    .and_then(|sr| sr.normal_at(v))
                    .map(|n| {
                        if n.dot(cut.faces()[strip].plane.normal()) < 0.0 {
                            -n
                        } else {
                            n
                        }
                    });
                let own = own.expect("the strip carries a surface normal at the cut");
                // No entry means the corner is left to the strip's own path,
                // which is `own`; an entry must agree with it.
                if let Some(got) = shade_at(&cut, strip, v) {
                    assert!(angle_deg(own, got) < 1.0, "{offset}: {e:?} at {v:?}");
                }
            }
        }
    }
    assert!(on_cut > 0, "{offset}");
}

#[test]
fn swept_rail_crosses_seams() {
    // r = 0.03 is wider than a facet of the 0.07 pipe at 48 segments, so the
    // rails cross seams between the chain's vertices.
    let (tee, edges) = pipe_tee();
    let chain_vertices = edges.len();
    let (after, report) = tee.blend_edges(&swept_round(edges, 0.03)).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) > signed_volume(&tee));
    // Stations: more than the chain's vertices (seam stations inserted).
    let mut rail_pts: BTreeSet<[u64; 3]> = BTreeSet::new();
    for &f in &report.new_faces {
        for p in after.loop_positions(after.faces()[f].outer_loop) {
            rail_pts.insert([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
        }
    }
    assert!(rail_pts.len() > 5 * chain_vertices);
    // Every station's ball is tangent to the pipes: each strip vertex lies
    // within r of some point of the original surface, and no closer to the
    // pipe axes than the pipes' radii.
    for &f in &report.new_faces {
        for p in after.loop_positions(after.faces()[f].outer_loop) {
            assert!(p.x.hypot(p.y) >= 0.1 * (PI / 48.0).cos() - 1e-9, "{p:?}");
            assert!(
                p.x.hypot(p.z) >= 0.07 * (PI / 48.0).cos() - 1e-9 || p.y < 0.0,
                "{p:?}"
            );
        }
    }
}

#[test]
fn swept_blend_oblique_plane_cylinder() {
    use kernel::{BooleanOp, Transform};
    let cyl = cylinder(0.3, 1.0, 48);
    let mut cutter = box_xyz(2.0, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(-1.0, -1.0, 0.0)))
        .unwrap();
    cutter
        .apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 0.4).unwrap())
        .unwrap();
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, 0.6)))
        .unwrap();
    let cut = Object::boolean(BooleanOp::Subtract, &cyl, &cutter, &Transform::IDENTITY).unwrap();
    let rim: Vec<EdgeId> = cut
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = cut.edge_faces_for_test(e).unwrap();
            let (p, q) = cut.edge_endpoints(e).unwrap();
            (cut.faces()[x].surface.is_some() != cut.faces()[y].surface.is_some())
                && p.z > 0.3
                && q.z > 0.3
        })
        .collect();
    let (after, _) = cut.blend_edges(&swept_round(rim, 0.02)).unwrap();
    assert_valid_solid(&after);
}

#[test]
fn swept_leak_refused() {
    // A radius whose rail would run off the end of the smaller pipe (its
    // top rail point sits 0.1 + r along it, past its 0.5 length): refused,
    // object untouched.
    let (tee, edges) = pipe_tee();
    let before = tee.to_polygons();
    let res = tee.blend_edges(&swept_round(edges, 0.45));
    assert!(matches!(
        res,
        Err(BlendError::UnsupportedVertex { .. } | BlendError::RadiusTooLarge { .. })
    ));
    assert_eq!(tee.to_polygons(), before);
}

/// The faces a report made, by their blend tag's kind.
fn new_kinds(after: &Object, report: &BlendReport) -> Vec<kernel::BlendKind> {
    report
        .new_faces
        .iter()
        .filter_map(|&f| after.faces()[f].blend.map(|t| t.kind))
        .collect()
}

#[test]
fn swept_chamfer_on_the_pipe_tee() {
    // Every profile applies to every edge the rolling ball can take (D-43,
    // D-106): a chamfer's setback is measured over the side's facets.
    let (tee, edges) = pipe_tee();
    let before = signed_volume(&tee);
    let chamfer = BlendProfile::Chamfer {
        setback_a: 0.01,
        setback_b: 0.01,
    };
    let (after, report) = tee.blend_edges(&spec(edges.clone(), chamfer)).unwrap();
    assert_valid_solid(&after);
    // The junction is a valley: a chamfer fills it.
    assert!(signed_volume(&after) > before + 1e-9);
    let kinds = new_kinds(&after, &report);
    assert!(!kinds.is_empty());
    assert!(
        kinds.iter().all(|k| *k == kernel::BlendKind::Chamfer),
        "{kinds:?}"
    );
    // A chamfer strip is flat facets with no arc: no analytic claim, and
    // the rulings between its facets are sharp.
    assert!(
        report
            .new_faces
            .iter()
            .all(|&f| after.faces()[f].surface.is_none())
    );
    // Asymmetric: the rail on the branch side sits farther from the seam.
    let asym = BlendProfile::Chamfer {
        setback_a: 0.008,
        setback_b: 0.015,
    };
    let (after2, _) = tee.blend_edges(&spec(edges, asym)).unwrap();
    assert_valid_solid(&after2);
    assert!(signed_volume(&after2) > before + 1e-9);
    assert!((signed_volume(&after2) - signed_volume(&after)).abs() > 1e-9);
}

#[test]
fn swept_profile_on_the_pipe_tee() {
    // A profile sits within one facet of each side (the catalogue's rule
    // for profiles): small enough for the tee's 13 mm facets.
    let (tee, edges) = pipe_tee();
    let before = signed_volume(&tee);
    let (after, report) = tee.blend_edges(&spec(edges, ogee(0.004))).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) > before + 1e-9);
    assert!(
        new_kinds(&after, &report)
            .iter()
            .all(|k| *k == kernel::BlendKind::Profile)
    );
}

#[test]
fn swept_chamfer_on_an_oblique_cut() {
    use kernel::{BooleanOp, Transform};
    let cyl = cylinder(0.3, 1.0, 48);
    let mut cutter = box_xyz(2.0, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(-1.0, -1.0, 0.0)))
        .unwrap();
    cutter
        .apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 0.4).unwrap())
        .unwrap();
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, 0.6)))
        .unwrap();
    let cut = Object::boolean(BooleanOp::Subtract, &cyl, &cutter, &Transform::IDENTITY).unwrap();
    let rim: Vec<EdgeId> = cut
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = cut.edge_faces_for_test(e).unwrap();
            let (p, q) = cut.edge_endpoints(e).unwrap();
            (cut.faces()[x].surface.is_some() != cut.faces()[y].surface.is_some())
                && p.z > 0.3
                && q.z > 0.3
        })
        .collect();
    let before = signed_volume(&cut);
    let (after, _) = cut
        .blend_edges(&spec(
            rim.clone(),
            BlendProfile::Chamfer {
                setback_a: 0.02,
                setback_b: 0.02,
            },
        ))
        .unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) < before - 1e-9);
    // A cove on the same convex rim: its arc facets meet smoothly.
    let cove = BlendProfile::Cove {
        radius: 0.02,
        segments: 4,
    };
    let (after, report) = cut.blend_edges(&spec(rim.clone(), cove)).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) < before - 1e-9);
    assert!(
        strip_interior_soft(&after, &report),
        "a cove's arc facets meet smoothly"
    );
    assert!(
        new_kinds(&after, &report)
            .iter()
            .all(|k| *k == kernel::BlendKind::Fillet)
    );
    // And a profile on the same rim.
    let (after, report) = cut.blend_edges(&spec(rim, ogee(0.012))).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) < before - 1e-9);
    assert!(
        new_kinds(&after, &report)
            .iter()
            .all(|k| *k == kernel::BlendKind::Profile)
    );
}

/// The pipe tee with everything past x = 0.01 cut away: the junction is an
/// open chain whose two ends run into the flat cut face.
fn half_tee() -> (Object, Vec<EdgeId>) {
    use kernel::{BooleanOp, Transform};
    let (tee, _) = pipe_tee();
    let mut cutter = box_xyz(1.0, 1.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.01, -0.5, -1.0)))
        .unwrap();
    let half = Object::boolean(BooleanOp::Subtract, &tee, &cutter, &Transform::IDENTITY).unwrap();
    let edges = half
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = half.edge_faces_for_test(e).unwrap();
            match (half.faces()[x].surface, half.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .collect();
    (half, edges)
}

/// The edges between a strip face of `report` and another face that lie on
/// the plane x = 0.01: an open chain's end creases.
fn end_crease_edges(after: &Object, report: &BlendReport) -> Vec<EdgeId> {
    let strip: BTreeSet<_> = report
        .new_faces
        .iter()
        .copied()
        .filter(|&f| {
            after.faces()[f]
                .blend
                .is_some_and(|t| t.role == BlendRole::Strip)
        })
        .collect();
    after
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = after.edge_faces_for_test(e).unwrap();
            let (a, b) = after.edge_endpoints(e).unwrap();
            strip.contains(&x) != strip.contains(&y)
                && (a.x - 0.01).abs() < 1e-6
                && (b.x - 0.01).abs() < 1e-6
        })
        .collect()
}

#[test]
fn swept_open_chain_ends_on_the_cut_face() {
    // An open swept chain ends where its sides run into one face: the
    // strip ends in that face's plane and the face is cut along the end
    // section, a crease (D-44, D-115).
    let (half, edges) = half_tee();
    assert!(edges.len() > 4);
    let before = signed_volume(&half);
    let (after, report) = half.blend_edges(&swept_round(edges.clone(), 0.01)).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) > before + 1e-9);
    assert!(strip_interior_soft(&after, &report));
    let creases = end_crease_edges(&after, &report);
    // Four facets per end section at both ends.
    assert_eq!(creases.len(), 8, "{creases:?}");
    for e in creases {
        assert_eq!(after.edge_smoothness(e), EdgeSmoothness::Hard);
    }
    // The cut face is one face still, with the end sections in its loop.
    let cut_faces: Vec<_> = after
        .faces()
        .keys()
        .filter(|&f| {
            let n = after.faces()[f].plane.normal();
            n.x.abs() > 0.999
                && (after.faces()[f].plane.signed_distance(p3(0.01, 0.0, 0.0))).abs() < 1e-9
        })
        .collect();
    assert_eq!(cut_faces.len(), 1);
    // A chamfer and a profile end the same way.
    let chamfer = BlendProfile::Chamfer {
        setback_a: 0.008,
        setback_b: 0.008,
    };
    let (after, report) = half.blend_edges(&spec(edges.clone(), chamfer)).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) > before + 1e-9);
    let creases = end_crease_edges(&after, &report);
    assert_eq!(creases.len(), 2);
    for e in creases {
        assert_eq!(after.edge_smoothness(e), EdgeSmoothness::Hard);
    }
    let (after, report) = half.blend_edges(&spec(edges, ogee(0.004))).unwrap();
    assert_valid_solid(&after);
    assert!(signed_volume(&after) > before + 1e-9);
    let creases = end_crease_edges(&after, &report);
    assert!(!creases.is_empty());
    for e in creases {
        assert_eq!(after.edge_smoothness(e), EdgeSmoothness::Hard);
    }
}

/// The pipe tee with everything past `x` cut away.
fn tee_cut_at(x: f64) -> (Object, Vec<EdgeId>) {
    use kernel::{BooleanOp, Transform};
    let (tee, _) = pipe_tee();
    let mut cutter = box_xyz(1.0, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(x, -1.0, -1.0)))
        .unwrap();
    let half = Object::boolean(BooleanOp::Subtract, &tee, &cutter, &Transform::IDENTITY).unwrap();
    let edges = half
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = half.edge_faces_for_test(e).unwrap();
            match (half.faces()[x].surface, half.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .collect();
    (half, edges)
}

#[test]
fn swept_open_chain_round_at_any_cut() {
    // Wherever the cut falls against the facets (mid-facet, a sliver from a
    // seam, on a seam, the far side of the junction), and at radii from a
    // fraction of a facet to several, the round ends on the cut face: the
    // ball's rails lead or lag its station by the facets' tilt and are
    // carried into the end plane either way.
    for x in [0.0095, 0.02, 0.05, -0.03] {
        let (half, edges) = tee_cut_at(x);
        let before = signed_volume(&half);
        for r in [0.005, 0.01, 0.02] {
            let (after, report) = half
                .blend_edges(&swept_round(edges.clone(), r))
                .unwrap_or_else(|e| panic!("x {x} r {r}: {e:?}"));
            assert_valid_solid(&after);
            assert!(signed_volume(&after) > before + 1e-9, "x {x} r {r}");
            assert!(!report.new_faces.is_empty());
        }
    }
}

#[test]
fn two_open_swept_chains_ending_on_one_face() {
    // A pipe through another, cut past both junctions: two open chains end
    // on the one cut face, which is cut by both (the second chain cuts the
    // pieces the first left, not the face again).
    use kernel::{BooleanOp, Transform};
    let mut a = cylinder(0.1, 1.0, 48);
    a.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, -0.5)))
        .unwrap();
    let mut b = cylinder(0.07, 1.0, 48);
    b.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, -0.5)))
        .unwrap();
    b.apply_transform(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -FRAC_PI_2).unwrap())
        .unwrap();
    let cross = Object::boolean(BooleanOp::Union, &a, &b, &Transform::IDENTITY).unwrap();
    let mut cutter = box_xyz(1.0, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.01, -1.0, -1.0)))
        .unwrap();
    let half = Object::boolean(BooleanOp::Subtract, &cross, &cutter, &Transform::IDENTITY).unwrap();
    let edges: Vec<EdgeId> = half
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = half.edge_faces_for_test(e).unwrap();
            match (half.faces()[x].surface, half.faces()[y].surface) {
                (
                    Some(SurfaceRef::Cylinder { axis: p, .. }),
                    Some(SurfaceRef::Cylinder { axis: q, .. }),
                ) => p.dot(q).abs() < 0.5,
                _ => false,
            }
        })
        .collect();
    let before = signed_volume(&half);
    let cut_faces = |o: &Object| {
        o.faces()
            .keys()
            .filter(|&f| {
                let pl = o.faces()[f].plane;
                pl.normal().x.abs() > 0.999 && pl.signed_distance(p3(0.01, 0.0, 0.0)).abs() < 1e-9
            })
            .count()
    };
    for profile in [
        BlendProfile::Round {
            radius: 0.01,
            segments: 4,
        },
        BlendProfile::Chamfer {
            setback_a: 0.01,
            setback_b: 0.01,
        },
    ] {
        let (after, report) = half.blend_edges(&spec(edges.clone(), profile)).unwrap();
        assert_valid_solid(&after);
        assert!(signed_volume(&after) > before + 1e-9);
        assert_eq!(cut_faces(&after), 1);
        // Four ends, each a crease on the cut face.
        assert!(end_crease_edges(&after, &report).len() >= 4);
    }
}

#[test]
fn swept_open_chain_with_another_picked_edge_at_its_end_refuses() {
    // A second picked edge at an open chain's end vertex (the cut face's
    // edge up the main pipe from the chain's top end): no construction
    // there, the refusal names that vertex, object untouched.
    let (half, mut edges) = half_tee();
    let at = edges
        .iter()
        .flat_map(|&e| {
            let (a, b) = half.edge_endpoints(e).unwrap();
            [a, b]
        })
        .find(|p| (p.x - 0.01).abs() < 1e-9 && p.z > 0.0)
        .expect("the top end of the chain");
    assert!((at.z - 0.0692).abs() < 1e-3, "{at:?}");
    let extra = half
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = half.edge_endpoints(e).unwrap();
            (a.approx_eq(at, 1e-9) || b.approx_eq(at, 1e-9))
                && !edges.contains(&e)
                && (a.x - 0.01).abs() < 1e-9
                && (b.x - 0.01).abs() < 1e-9
                && (a.y - b.y).abs() < 1e-9
        })
        .expect("the cut-face edge up the main pipe from the end");
    edges.push(extra);
    let before = half.to_polygons();
    let err = half.blend_edges(&swept_round(edges, 0.01)).unwrap_err();
    match err {
        BlendError::UnsupportedVertex { at: p } => assert!(p.approx_eq(at, 1e-9), "{p:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(half.to_polygons(), before);
}

#[test]
fn swept_profile_too_wide_refuses_typed_and_untouched() {
    let (tee, edges) = pipe_tee();
    let before = tee.to_polygons();
    let res = tee.blend_edges(&spec(
        edges.clone(),
        BlendProfile::Chamfer {
            setback_a: 0.45,
            setback_b: 0.45,
        },
    ));
    assert!(
        matches!(
            res,
            Err(BlendError::RadiusTooLarge { .. }
                | BlendError::ProfileTooWide { .. }
                | BlendError::UnsupportedVertex { .. })
        ),
        "{res:?}"
    );
    assert_eq!(tee.to_polygons(), before);
    // Stops and handles still have no swept construction.
    let mut stopped = swept_round(edges, 0.01);
    stopped.stops = Some(BlendStops {
        start: 0.01,
        end: 0.01,
    });
    assert_eq!(
        tee.blend_edges(&stopped).unwrap_err(),
        BlendError::UnsupportedEdge
    );
}

#[test]
fn swept_imported_sphere_edge() {
    use kernel::{BooleanOp, Transform};
    // An imported faceted sphere sunk into a box: the loop where it leaves
    // the box's top is plane–plane facets the catalogue cannot turn.
    let sphere = imported_sphere(16, 24, 0.3, 0);
    let mut bx = box_xyz(1.2, 1.2, 0.5);
    bx.apply_transform(&Transform::translation(Vec3::new(-0.6, -0.6, -0.63)))
        .unwrap();
    let u = Object::boolean(BooleanOp::Union, &sphere, &bx, &Transform::IDENTITY).unwrap();
    let top = face_with_normal(&u, Vec3::new(0.0, 0.0, 1.0));
    let edges: Vec<EdgeId> = u
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = u.edge_faces_for_test(e).unwrap();
            let (p, q) = u.edge_endpoints(e).unwrap();
            (x == top) != (y == top) && p.x.hypot(p.y) < 0.35 && q.x.hypot(q.y) < 0.35
        })
        .collect();
    assert!(edges.len() > 20);
    let (after, report) = u.blend_edges(&swept_round(edges, 0.01)).unwrap();
    assert_valid_solid(&after);
    assert!(claims(&after, &report).is_empty());
    // No leak: every new face stays near the loop (§4.4 step 3).
    for &f in &report.new_faces {
        for p in after.loop_positions(after.faces()[f].outer_loop) {
            assert!((p.z + 0.13).abs() <= 0.03, "{p:?}");
        }
    }
    assert!(signed_volume(&after) > signed_volume(&u));
}

#[test]
fn lofted_hull_gunwale_rounds_from_one_click() {
    // A Loft's wall facets carry no claim but meet at soft seams: they are
    // one logical face, so one click on the deck rim picks the whole loop,
    // and it rounds (D-47, D-48).
    let hull = lofted_hull();
    let deck = sharp_edges_at_height(&hull, 1.0);
    assert_eq!(deck.len(), 24);
    let picked = hull.logical_edge(deck[0], false);
    assert_eq!(picked.len(), 24);
    let before = signed_volume(&hull);
    for r in [0.01, 0.03] {
        let (after, report) = blend_ok(
            &hull,
            &spec(
                picked.clone(),
                BlendProfile::Round {
                    radius: r,
                    segments: 4,
                },
            ),
        );
        assert_eq!(report.blends.len(), 1);
        assert!(signed_volume(&after) < before);
    }
}

/// A Loft through five rectangles of different proportions (the 3DBenchy
/// playtest's hull: half sizes 0.8 × 0.28 up to 1.0 × 0.56, a quarter
/// apart), each starting at its (+x, −y) corner.
fn rectangle_station_hull() -> Object {
    let rect = |w: f64, h: f64, z: f64| kernel::loft::LoftProfile {
        points: vec![p3(w, -h, z), p3(w, h, z), p3(-w, h, z), p3(-w, -h, z)],
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let dims = [
        (0.8, 0.28),
        (1.0, 0.44),
        (1.1, 0.52),
        (1.1, 0.56),
        (1.0, 0.56),
    ];
    Object::from_loft(&kernel::loft::LoftSpec {
        profiles: dims
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| rect(w, h, 0.25 * i as f64))
            .collect(),
        apex: None,
        rails: Vec::new(),
        stations: Some(4),
    })
    .unwrap()
}

/// The square cap of a circle-to-square loft: its rim is one chain round
/// the four sides (48 pieces, the walls smooth beside them, D-126), and a
/// round of it answers with a typed refusal, never a panic or a broken
/// solid. Recorded defect (D-126): the facet planners refuse
/// `UnsupportedVertex` at a vertex along a side, (1/3, −1/2, 0), where a
/// column and a fan diagonal meet the rim, so three wall facets meet there
/// and a pass-through station takes two; the swept retry refuses at the
/// square's corners (−1/2, −1/2, 0), where the chain turns a right angle
/// over a fan of wall facets, at every size but a window near 22 mm
/// (D-228: a swept chain tries more than one start), so a large size names
/// that window's size, which builds and removes back exactly.
#[test]
fn a_circle_to_square_cap_rim_refuses_with_a_typed_error() {
    let n = 48;
    let circle = kernel::loft::LoftProfile {
        points: (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                p3(0.5 * a.cos(), 0.5 * a.sin(), 1.0)
            })
            .collect(),
        curves: vec![
            Some(CurveGeom {
                center: p3(0.0, 0.0, 1.0),
                radius: 0.5,
            });
            n
        ],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let square = kernel::loft::LoftProfile {
        points: vec![
            p3(0.5, -0.5, 0.0),
            p3(0.5, 0.5, 0.0),
            p3(-0.5, 0.5, 0.0),
            p3(-0.5, -0.5, 0.0),
        ],
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    let o = Object::from_loft(&kernel::loft::LoftSpec {
        profiles: vec![circle, square],
        apex: None,
        rails: Vec::new(),
        stations: None,
    })
    .unwrap();
    let rim = sharp_edges_at_height(&o, 0.0);
    assert_eq!(rim.len(), 48);
    assert_eq!(
        o.logical_edge(rim[0], true).len(),
        48,
        "one chain round the cap"
    );
    for r in [0.01, 0.02] {
        match o.blend_edges(&round_of(&rim, r)) {
            Err(BlendError::UnsupportedVertex { at }) => {
                let want = p3(1.0 / 3.0, -0.5, 0.0);
                assert!((at - want).length() < 1e-12, "{r}: {at:?}");
            }
            other => panic!("{r}: {:?}", other.map(|_| ())),
        }
    }
    // 50 mm never builds as asked. It refuses typed: at the side vertex as
    // the smaller sizes do, or, where a smaller size rolls (today a window
    // near 22 mm, D-230), with that size, which builds and removes back
    // exactly. Either way, never a 50 mm result (review F16).
    for r in [0.03, 0.05, 0.1] {
        match o.blend_edges(&round_of(&rim, r)) {
            Err(BlendError::UnsupportedVertex { at }) => {
                let want = p3(1.0 / 3.0, -0.5, 0.0);
                assert!((at - want).length() < 1e-12, "{r}: {at:?}");
            }
            Err(BlendError::RadiusTooLarge { max, .. }) => {
                assert!(max > 0.0 && max < r, "{r}: {max}");
                let (after, report) = blend_ok(&o, &round_of(&rim, max));
                let (back, _) = after.remove_blend_for_test(report.blends[0]).unwrap();
                assert!(kernel::same_geometry_for_test(&back, &o, 1e-9));
            }
            other => panic!("{r}: {:?}", other.map(|_| ())),
        }
    }
}

/// Five rectangles of different proportions loft corner to corner (D-126
/// R1), so the deck is the top rectangle's four sides: one click picks one
/// side, the four round together at 25 and 50 mm, and the Fillet tool's
/// default 0.18 m names the limit the neighboring side sets (62.5 mm), as
/// on a drawn box. Before, the rectangles matched by perimeter: corners
/// of one met the middle of another's sides, the deck rim broke into
/// twelve pieces, and it took two clicks and a fading crease rule to pick
/// it (D-S23, superseded).
#[test]
fn a_rectangle_station_deck_is_four_sides_and_rounds() {
    let hull = rectangle_station_hull();
    let deck = sharp_edges_at_height(&hull, 1.0);
    assert_eq!(deck.len(), 4);
    for &e in &deck {
        assert_eq!(
            hull.logical_edge(e, false),
            vec![e],
            "one click picks one side"
        );
    }
    for r in [0.025, 0.05] {
        let (_, report) = blend_ok(
            &hull,
            &spec(
                deck.clone(),
                BlendProfile::Round {
                    radius: r,
                    segments: 4,
                },
            ),
        );
        assert_eq!(report.blends.len(), 1, "{r}");
    }
    match hull.blend_edges(&round_of(&deck, 0.18)) {
        Err(BlendError::RadiusTooLarge { max, limited_by }) => {
            assert!((max - 0.0625).abs() < 1e-6, "{max}");
            assert_eq!(limited_by, kernel::LimitFeature::NeighborEdge);
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    // Every column is a corner in every profile: the four creases run
    // the whole height, and nothing else on the wall is hard.
    let hard_wall = hull
        .edges()
        .keys()
        .filter(|&e| {
            let (p, q) = hull.edge_endpoints(e).unwrap();
            (p.z - q.z).abs() > 1e-9 && hull.edge_smoothness(e) == EdgeSmoothness::Hard
        })
        .count();
    assert_eq!(
        hard_wall,
        4 * 4 * 4,
        "four columns, four spans of four stations"
    );
}

/// The real-app check's S2 profiles (rectangles 1.0 × 0.6, 1.2 × 0.8 and
/// 1.1 × 0.7 at heights 0, 0.5 and 1, centered), as LoftProfiles, with the
/// middle one's (−x, +y) corner cut by a 2 cm chamfer: five corners
/// against four, so the Loft matches them by perimeter (D-126) and the
/// corners at different perimeter fractions become clusters of columns,
/// with short deck pieces beside the deck's corners (the geometry the
/// D-119..D-125 specs below were found on; corner-matched rectangles loft
/// four clean sides, see `rectangles_of_different_aspect_loft_corner_to_corner`).
fn s2_profiles() -> Vec<kernel::loft::LoftProfile> {
    let profile = |points: Vec<Point3>| kernel::loft::LoftProfile {
        points,
        curves: Vec::new(),
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    };
    s2_loops().into_iter().map(profile).collect()
}

/// The S2 profiles' loops (the middle one chamfered at one corner).
fn s2_loops() -> Vec<Vec<Point3>> {
    let rect =
        |w: f64, h: f64, z: f64| vec![p3(w, -h, z), p3(w, h, z), p3(-w, h, z), p3(-w, -h, z)];
    let c = 0.02;
    vec![
        rect(0.5, 0.3, 0.0),
        vec![
            p3(0.6, -0.4, 0.5),
            p3(0.6, 0.4, 0.5),
            p3(-0.6 + c, 0.4, 0.5),
            p3(-0.6, 0.4 - c, 0.5),
            p3(-0.6, -0.4, 0.5),
        ],
        rect(0.55, 0.35, 1.0),
    ]
}

/// The S2 hull with the Loft's default stations.
fn s2_deck() -> Object {
    Object::from_loft(&kernel::loft::LoftSpec {
        profiles: s2_profiles(),
        apex: None,
        rails: Vec::new(),
        stations: None,
    })
    .unwrap()
}

/// `radius` as a 6-segment round of `edges`.
fn round_of(edges: &[EdgeId], radius: f64) -> BlendSpec {
    spec(
        edges.to_vec(),
        BlendProfile::Round {
            radius,
            segments: 6,
        },
    )
}

/// Past the 14.3 mm the facet planners build on the S2 deck (the corner's
/// run-out reaches a deck vertex 12.8 mm from it), they refuse the corner
/// outright up to about 50 mm; since smaller sizes build, that is a size
/// refusal naming the limit and the short edge, in a commit and in a
/// preview alike, so a drag clamps there and a typed size names it.
/// Before, those sizes said "Can't blend this corner" (D-119; the limit
/// was 22.3 mm on the deck of three plain rectangles, which now loft
/// corner to corner, D-126).
#[test]
fn a_tapering_rectangle_deck_names_its_limit_past_the_corner_planner() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    for r in [0.03, 0.04, 0.05] {
        for preview in [false, true] {
            match hull.blend_edges_catalog_only_for_test(&round_of(&deck, r), preview) {
                Err(BlendError::RadiusTooLarge { max, limited_by }) => {
                    assert!((max - 0.01426).abs() < 2e-4, "{r} {preview}: {max}");
                    assert_eq!(limited_by, kernel::LimitFeature::ShortEdge, "{r} {preview}");
                }
                other => panic!("{r} {preview}: {:?}", other.map(|_| ())),
            }
        }
    }
}

/// Where the planners refuse a corner outright at the asked size, the
/// search for a smaller size that builds tries a bounded set: the 48 sizes
/// down three octaves where none builds (verticals asked smaller than the
/// rim they run into, whose corner the planners cannot build at any size),
/// and ten bisections more at most where one does (a unit spec in
/// `blend::limit`).
#[test]
fn the_hard_corner_search_is_bounded() {
    let mut rimmed = box_xyz(1.0, 1.0, 1.0);
    let top: Vec<EdgeId> = sharp_edges_at_height(&rimmed, 1.0);
    rimmed = blend_ok(&rimmed, &round_of(&top, 0.3)).0;
    let verticals: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&rimmed, p3(x, y, 0.0), p3(x, y, 0.7)))
        .collect();
    let before = kernel::limit_probes_for_test();
    let refused = rimmed.blend_edges_catalog_only_for_test(&round_of(&verticals, 0.1), false);
    let used = kernel::limit_probes_for_test() - before;
    assert!(
        matches!(refused, Err(BlendError::UnsupportedVertex { .. })),
        "{:?}",
        refused.map(|_| ())
    );
    assert!(used <= 48, "{used}");
}

/// Verticals asked smaller than a large rim they run into: no size builds
/// that corner, so the corner's refusal stands (the hard-corner search
/// finds nothing to name).
#[test]
fn a_vertical_fillet_below_an_existing_rim_still_refuses_the_corner() {
    let mut rimmed = box_xyz(1.0, 1.0, 1.0);
    let top: Vec<EdgeId> = sharp_edges_at_height(&rimmed, 1.0);
    rimmed = blend_ok(&rimmed, &round_of(&top, 0.3)).0;
    let verticals: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&rimmed, p3(x, y, 0.0), p3(x, y, 0.7)))
        .collect();
    for preview in [false, true] {
        let r = if preview {
            rimmed.blend_edges_as_preview_for_test(&round_of(&verticals, 0.1))
        } else {
            rimmed.blend_edges(&round_of(&verticals, 0.1))
        };
        assert!(
            matches!(r, Err(BlendError::UnsupportedVertex { .. })),
            "{preview}: {:?}",
            r.map(|_| ())
        );
    }
}

/// A Loft's wall is facets no one drew, met by smooth seams at the deck
/// rim: a round the facet planners limit by those facets rolls across
/// them as a swept blend, as on an imported mesh (D-119). The S2 deck
/// rounds whole at 40 mm, 0.1 m and the Fillet tool's default 0.18 m.
/// Before, those sizes clamped to the facet planners' 22.3 mm or refused.
#[test]
fn a_lofted_deck_rounds_as_a_swept_blend_past_the_facet_limit() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    for r in [0.04, 0.1, 0.18] {
        let (after, report) = blend_ok(&hull, &round_of(&deck, r));
        assert_eq!(report.blends.len(), 1, "{r}");
        assert!(signed_volume(&after) < signed_volume(&hull), "{r}");
        // The swept blend claims no surface (and is not restorable yet,
        // K-6).
        assert!(claims(&after, &report).is_empty(), "{r}");
    }
}

/// A drawn box has no smooth seams at its rims: a round too large for a
/// short edge still refuses as too large, naming the limit, with no swept
/// retry.
#[test]
fn a_drawn_box_still_clamps_at_its_short_edge() {
    // A slab 2 cm thick: its top rim's short ends are 2 cm, too short for
    // a 5 cm round.
    let slab = box_xyz(0.02, 1.0, 1.0);
    let rim = sharp_edges_at_height(&slab, 1.0);
    match slab.blend_edges(&round_of(&rim, 0.05)) {
        Err(BlendError::RadiusTooLarge { max, .. }) => assert!(max > 0.0 && max < 0.05),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // The same refusal as the facet planners alone give: no swept retry.
    assert_eq!(
        slab.blend_edges(&round_of(&rim, 0.05)).map(|_| ()),
        slab.blend_edges_catalog_only_for_test(&round_of(&rim, 0.05), false)
            .map(|_| ())
    );
}

/// Past the largest size that rolls on the S2 deck, the commit names it
/// ("limited by the shape of the model"): at least the 0.18 that rounds,
/// and a preview names the same (the search is bounded on a generated
/// wall; only an import's preview skips it).
#[test]
fn a_generated_wall_names_the_largest_size_that_rolls() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    let commit = match hull.blend_edges(&round_of(&deck, 0.5)) {
        Err(BlendError::RadiusTooLarge { max, .. }) => max,
        other => panic!("{:?}", other.map(|_| ())),
    };
    assert!(commit >= 0.18, "{commit}");
    match hull.blend_edges_as_preview_for_test(&round_of(&deck, 0.5)) {
        Err(BlendError::RadiusTooLarge { max, .. }) => assert_eq!(max, commit),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

/// The Fillet tool's default on the S2 deck previews as it commits: a
/// preview builds exactly when the commit does, with the same faces.
#[test]
fn preview_and_commit_agree_on_the_deck_default() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    let s = round_of(&deck, 0.18);
    let preview = hull.blend_edges_as_preview_for_test(&s);
    let commit = hull.blend_edges(&s);
    assert_eq!(preview.is_ok(), commit.is_ok());
    if let (Ok((p, _)), Ok((c, _))) = (preview, commit) {
        assert_eq!(p.faces().len(), c.faces().len());
        assert!(kernel::same_geometry_for_test(&p, &c, 1e-9));
    }
}

/// The S2 hull in a document, lofted through drawn rectangles (the middle
/// one chamfered at one corner, as `s2_profiles`): the same deck.
fn s2_document() -> (kernel::Document, kernel::ObjectId) {
    use kernel::math::Plane;
    let mut doc = kernel::Document::new();
    let mut sources = Vec::new();
    for lp in s2_loops() {
        let z = lp[0].z;
        let s = doc.add_sketch(
            Plane::from_point_normal(p3(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).unwrap(),
        );
        doc.begin_sketch_gesture(s).unwrap();
        {
            let sk = doc.sketch_mut(s).unwrap();
            // Drawn from the loops' first vertex, so the document's deck is
            // `s2_deck`'s.
            let n = lp.len();
            for i in 0..n {
                sk.add_segment(lp[i], lp[(i + 1) % n]).unwrap();
            }
        }
        doc.end_sketch_gesture(s).unwrap();
        let region = doc.extrudable_regions(s).unwrap()[0];
        sources.push(kernel::document::LoftSource::Region { sketch: s, region });
    }
    let (id, _) = doc.loft(&sources, &[], &[], None).unwrap();
    (doc, id)
}

/// A typed size previews as it commits on the S2 deck: where the commit
/// builds, the typed preview shows that size unclamped and the same
/// geometry; where the commit refuses, the typed preview does not show the
/// size as built. (A pointer preview takes one round of the swept blend's
/// station removal and may stop short of a size the commit's rounds
/// build; a typed size takes the commit's.)
#[test]
fn a_typed_size_previews_as_it_commits() {
    let (doc, id) = s2_document();
    let deck = sharp_edges_at_height(doc.object(id).unwrap(), 1.0);
    for r in [0.02, 0.04, 0.05, 0.1, 0.18, 0.5] {
        let s = round_of(&deck, r);
        let preview = doc.blend_preview_typed(id, &s);
        let mut committed = doc.clone();
        match committed.blend_edges(id, &s) {
            Ok(_) => {
                let p = preview.unwrap_or_else(|e| panic!("{r}: {e:?}"));
                assert!(p.clamped.is_none(), "{r}");
                assert!(
                    kernel::same_geometry_for_test(&p.after, committed.object(id).unwrap(), 1e-9),
                    "{r}"
                );
            }
            // A size refusal: the preview clamps where the commit names.
            Err(kernel::DocumentError::Blend(BlendError::RadiusTooLarge { max, .. })) => {
                let p = preview.unwrap_or_else(|e| panic!("{r}: {e:?}"));
                let clamp = p.clamped.unwrap_or_else(|| panic!("{r}: unclamped"));
                assert_eq!(clamp.max, max, "{r}");
            }
            // Any other refusal: the same one.
            Err(e) => assert_eq!(
                format!("{:?}", preview.map(|_| ())),
                format!("{:?}", Err::<(), _>(e)),
                "{r}"
            ),
        }
    }
}

/// Which sizes roll on a generated wall need not be monotone: a refusal
/// names the largest size its scan found to roll, which builds, and never
/// one below a size that rolls on its way down. Before, the halving search
/// from 50 mm stopped in a low window and named 23.4 mm; with swept
/// chains trying more than one start (D-228) the S2 deck rolls every size
/// from 17.5 mm to 0.3 m, so the refusals are now at 0.4 and 0.5 m.
#[test]
fn a_generated_wall_names_the_largest_size_it_found_to_roll() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    // 50 mm rolls since swept chains try more than one start (D-228).
    assert!(hull.blend_edges(&round_of(&deck, 0.05)).is_ok());
    for (asked, at_least) in [(0.4, 0.3), (0.5, 0.3)] {
        match hull.blend_edges(&round_of(&deck, asked)) {
            Err(BlendError::RadiusTooLarge { max, .. }) => {
                assert!(max >= at_least && max < asked, "{asked}: {max}");
                assert!(
                    hull.blend_edges(&round_of(&deck, max)).is_ok(),
                    "{asked}: {max} builds"
                );
            }
            other => panic!("{asked}: {:?}", other.map(|_| ())),
        }
    }
}

/// `max_blend_size` answers what the commit builds on a generated wall:
/// the asked size where it rolls as a swept blend, the commit's own named
/// limit where it does not. Before, it gave the facet planners' 22.3 mm
/// for sizes the commit builds.
#[test]
fn max_blend_size_answers_what_the_commit_builds_on_a_generated_wall() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    for r in [0.05, 0.1, 0.3] {
        let limit = hull.max_blend_size(&round_of(&deck, r)).unwrap();
        match hull.blend_edges(&round_of(&deck, r)) {
            Ok(_) => assert_eq!(limit.max, r, "{r}"),
            Err(BlendError::RadiusTooLarge { max, .. }) => assert_eq!(limit.max, max, "{r}"),
            Err(e) => panic!("{r}: {e:?}"),
        }
    }
}

/// A swept round on the S2 deck resizes with the preview agreeing with
/// the commit: where the commit builds, the preview shows it unclamped
/// and the same; past what rolls (0.5 m) the preview clamps where the
/// commit's refusal names (0.34375 m), a size that builds and never a
/// fraction of a millimeter from the facet planners alone (D-120). Resized
/// to 20 mm it builds (D-121), and to 10 mm, as a fresh 10 mm round does.
/// Removing the round gives the deck back. Before, a preview at 50 mm
/// clamped to 3.2 µm while the commit named 46.9 mm, and a resize to 20 mm
/// refused (`BlendDegenerate`: the restored deck rounded with the cap as
/// side A). Recorded (D-126): on this deck a fresh 20 mm round refuses,
/// the swept walk failing at exactly 20 mm though 19.9 and 20.1 mm build
/// (it names 17.5 mm), while the resize builds there: the swept blend's
/// stations follow the chain's direction, which the restore flips.
#[test]
fn a_swept_deck_round_resizes_as_its_preview_shows() {
    let (mut doc, id) = s2_document();
    let deck = sharp_edges_at_height(doc.object(id).unwrap(), 1.0);
    let before = doc.object(id).unwrap().clone();
    for start in [0.1, 0.18] {
        let mut d = doc.clone();
        let (report, _) = d.blend_edges(id, &round_of(&deck, start)).unwrap();
        let blend = report.blends[0];
        for r in [0.5, 0.05, 0.02, 0.01] {
            let profile = BlendProfile::Round {
                radius: r,
                segments: 6,
            };
            let preview = d.resize_blend_preview(id, blend, &profile);
            let mut committed = d.clone();
            let commit = committed.resize_blend(id, blend, &profile);
            if r == 0.5 {
                // Past what rolls: the clamped arm below.
                assert!(
                    matches!(
                        commit,
                        Err(kernel::DocumentError::Blend(
                            BlendError::RadiusTooLarge { .. }
                        ))
                    ),
                    "{start} → {r}: {:?}",
                    commit.as_ref().map(|_| ())
                );
            }
            if r == 0.02 || r == 0.01 {
                assert!(commit.is_ok(), "{start} → {r}: {:?}", commit.map(|_| ()));
            }
            match (preview, commit) {
                (Ok(p), Ok(_)) => {
                    assert!(p.clamped.is_none(), "{start} → {r}");
                    assert!(
                        kernel::same_geometry_for_test(
                            &p.after,
                            committed.object(id).unwrap(),
                            1e-9
                        ),
                        "{start} → {r}"
                    );
                }
                (
                    Ok(p),
                    Err(kernel::DocumentError::Blend(BlendError::RadiusTooLarge { max, .. })),
                ) => {
                    let clamp = p.clamped.expect("a clamped preview").max;
                    assert_eq!(
                        clamp, max,
                        "{start} → {r}: preview and commit name one limit"
                    );
                    assert!(max > 1e-3, "{start} → {r}: {max}");
                    let mut at = d.clone();
                    assert!(
                        at.resize_blend(
                            id,
                            blend,
                            &BlendProfile::Round {
                                radius: max,
                                segments: 6
                            }
                        )
                        .is_ok(),
                        "{start} → {r}: the named {max} builds"
                    );
                }
                (Err(pe), Err(ce)) => {
                    assert_eq!(format!("{pe:?}"), format!("{ce:?}"), "{start} → {r}")
                }
                (p, c) => panic!(
                    "{start} → {r}: preview {:?}, commit {:?}",
                    p.map(|_| ()),
                    c.map(|_| ())
                ),
            }
        }
        d.remove_blend(id, blend).unwrap();
        assert!(kernel::same_geometry_for_test(
            d.object(id).unwrap(),
            &before,
            1e-9
        ));
    }
    let _ = &mut doc;
}

/// A round's result does not depend on which of an edge's two faces is
/// side A: the facet planners round the S2 deck rim the same with the cap
/// or the wall as side A, and so a restored deck (whose edges' first faces
/// are the other ones) re-rounds as the fresh one does. Before, with the
/// cap as side A the wall's run at a faded column vertex beside the deck
/// corner came out backward (a distance tie between two points on the
/// rail), the patch did not stitch (`BlendDegenerate`), and a swept deck
/// round resized down refused. (The constructed cases are
/// `blend_specs::a_slanted_imprint_rounds_with_either_face_as_side_a` and
/// its neighbor; on this deck the facet planners build up to 14.3 mm.)
#[test]
fn a_round_does_not_depend_on_which_face_is_side_a() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    let top = face_with_normal(&hull, Vec3::new(0.0, 0.0, 1.0));
    let first_is_top = |o: &Object, edges: &[EdgeId]| {
        let t = face_with_normal(o, Vec3::new(0.0, 0.0, 1.0));
        edges
            .iter()
            .filter(|&&e| o.edge_faces_for_test(e).unwrap().0 == t)
            .count()
    };
    assert_eq!(
        first_is_top(&hull, &deck),
        0,
        "the wall is side A by default"
    );
    let planners = |o: &Object, s: &BlendSpec| {
        let (after, _) = o
            .blend_edges_catalog_only_for_test(s, false)
            .unwrap_or_else(|e| panic!("{e:?}"));
        (after.faces().len(), signed_volume(&after))
    };
    for r in [0.005, 0.01, 0.012] {
        let wall_a = planners(&hull, &round_of(&deck, r));
        let mut cap_side = round_of(&deck, r);
        cap_side.primary = Some(top);
        let cap_a = planners(&hull, &cap_side);
        assert_eq!(wall_a.0, cap_a.0, "{r}");
        assert!((wall_a.1 - cap_a.1).abs() < 1e-12, "{r}");
    }
    // Restored, then re-rounded: as fresh.
    let (after, report) = hull.blend_edges(&round_of(&deck, 0.1)).unwrap();
    let (back, edges) = after.remove_blend_for_test(report.blends[0]).unwrap();
    assert_eq!(
        first_is_top(&back, &edges),
        edges.len(),
        "the cap is side A after restore"
    );
    let fresh = planners(&hull, &round_of(&deck, 0.01));
    let again = planners(&back, &round_of(&edges, 0.01));
    assert_eq!(fresh.0, again.0);
    assert!((fresh.1 - again.1).abs() < 1e-12);
}

/// The real-app check's deck: rectangles 1.0 × 0.6, 1.2 × 0.8 and
/// 1.1 × 0.7 a half apart (the middle one chamfered at a corner, so they
/// match by perimeter), the top smaller than the middle, the Loft's
/// default stations. The whole rim rounds at small sizes too, under the
/// facet planners' 14.3 mm. Before, the stations were not planar (each
/// column's path took its own knots), so the band under the deck split
/// into triangles a hair out of plane, and the corner where such a
/// triangle's soft diagonal met the rim refused at 0.005, 0.01 and 0.02
/// (only from about 0.05 did the swept blend pass).
#[test]
fn a_tapering_rectangle_deck_rounds_at_small_sizes() {
    let hull = s2_deck();
    let deck = sharp_edges_at_height(&hull, 1.0);
    for r in [0.005, 0.01] {
        let (_, report) = blend_ok(
            &hull,
            &spec(
                deck.clone(),
                BlendProfile::Round {
                    radius: r,
                    segments: 4,
                },
            ),
        );
        assert_eq!(report.blends.len(), 1, "{r}");
    }
}

/// A loop of circles about the z axis, `n` sides from angle `phase`, at
/// height `z`.
fn loft_circle(r: f64, z: f64, n: usize, phase: f64) -> kernel::loft::LoftProfile {
    kernel::loft::LoftProfile {
        points: (0..n)
            .map(|k| {
                let a = phase + std::f64::consts::TAU * k as f64 / n as f64;
                p3(r * a.cos(), r * a.sin(), z)
            })
            .collect(),
        curves: vec![
            Some(CurveGeom {
                center: p3(0.0, 0.0, z),
                radius: r,
            });
            n
        ],
        smooth: Vec::new(),
        through: Vec::new(),
        start_offset: 0,
    }
}

/// A Loft through coaxial circles has coaxial rims: rounding the top or the
/// bottom rim makes an exact torus, as on any coaxial rim (the band beside
/// the rim is a faceted cone, which the fillet recognizes from its facets
/// and claims only for the plan; the wall stays one unclaimed surface).
/// Before, the rim went to the swept blend (no claim) or, with the app's
/// side counts, refused.
#[test]
fn a_lofted_coaxial_rim_rounds_as_a_torus() {
    let hulls = [
        lofted_hull(),
        Object::from_loft(&kernel::loft::LoftSpec {
            profiles: vec![
                loft_circle(0.35, 0.0, 60, 0.3),
                loft_circle(0.58, 0.5, 76, 1.1),
                loft_circle(0.6, 1.0, 80, 2.0),
            ],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap(),
    ];
    for (h, hull) in hulls.iter().enumerate() {
        for z in [0.0, 1.0] {
            let rim = sharp_edges_at_height(hull, z);
            let picked = hull.logical_edge(rim[0], false);
            assert_eq!(picked.len(), rim.len(), "hull {h}: one click picks the rim");
            let (after, report) = blend_ok(
                hull,
                &spec(
                    picked,
                    BlendProfile::Round {
                        radius: 0.02,
                        segments: 4,
                    },
                ),
            );
            let tori = claims(&after, &report);
            assert_eq!(tori.len(), report.new_faces.len(), "hull {h} z {z}");
            for t in tori {
                match t {
                    SurfaceRef::Torus { center, axis, .. } => {
                        assert!(axis.z.abs() > 1.0 - 1e-9, "{axis:?}");
                        assert!(center.x.hypot(center.y) < 1e-9, "{center:?}");
                    }
                    other => panic!("hull {h} z {z}: {other:?}"),
                }
            }
            // The wall the fillet trimmed carries no claim of its own.
            assert!(
                after
                    .faces()
                    .values()
                    .all(|f| f.blend.is_some() || f.surface.is_none()),
                "hull {h} z {z}"
            );
        }
    }
}

#[test]
fn app_drawn_pipe_tee_junction_rounds_whatever_the_facet_phase() {
    // The junction of two drawn pipes rounds as one swept loop at the sizes
    // the playtest used, for circles drawn at different facet phases (the
    // phase follows where the user clicks the rim). A ball riding two seams
    // turns its section, and a free station just after it, or two one-side
    // seam crossings a hair apart, folded the strip through itself; a rail
    // point a few nanometers off its facet edge, or on the edge of a facet
    // the rail only touches, split one face of the edge and not the other.
    for (pm, pb) in [
        (0.0, 0.0),
        (0.0, 0.2),
        (0.0, 0.7),
        (0.3, 0.0),
        (0.3, 0.2),
        (1.0, 0.2),
        (1.0, 1.5),
    ] {
        let (doc, tee, loop_edges) = app_pipe_tee(pm, pb);
        let obj = doc.object(tee).unwrap();
        let before = signed_volume(obj);
        for r in [0.01, 0.03, 0.05] {
            let (after, report) = blend_ok(
                obj,
                &spec(
                    loop_edges.clone(),
                    BlendProfile::Round {
                        radius: r,
                        segments: 4,
                    },
                ),
            );
            assert_eq!(report.blends.len(), 1, "phases {pm}/{pb} r {r}");
            // A concave junction fillet adds material.
            assert!(signed_volume(&after) > before, "phases {pm}/{pb} r {r}");
        }
    }
}

#[test]
fn playtest_pipe_tee_rounds_its_junction_from_the_tools_pick() {
    // The pipe tee the playtest built in the app (Circle, Push/Pull,
    // Rotate, Move, Union; branch off its axis by a few hundredths of a
    // millimeter), saved as drawn. The Fillet tool's hover picks the edge
    // nearest the click and expands it with tangent propagation; the loop
    // rounds at the playtest's sizes.
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playtest_pipe_tee.hew"),
    )
    .unwrap();
    let doc = kernel::Document::load(&bytes).unwrap();
    let id = doc.visible_object_ids()[0];
    let tee = doc.object(id).unwrap();
    let click = p3(5.046_498_3, 0.087_988_96, 0.349_076_35);
    let near = |e: EdgeId| {
        let (p, q) = tee.edge_endpoints(e).unwrap();
        let d = q - p;
        let t = ((click - p).dot(d) / d.dot(d)).clamp(0.0, 1.0);
        (p + d * t - click).length()
    };
    let edge = tee
        .sharp_edges()
        .into_iter()
        .min_by(|&a, &b| near(a).total_cmp(&near(b)))
        .unwrap();
    let picked = tee.logical_edge(edge, true);
    assert_eq!(picked.len(), 40, "the whole junction loop");
    let before = signed_volume(tee);
    for r in [0.01, 0.03, 0.05] {
        let (after, report) = blend_ok(
            tee,
            &spec(
                picked.clone(),
                BlendProfile::Round {
                    radius: r,
                    segments: 4,
                },
            ),
        );
        assert_eq!(report.blends.len(), 1);
        assert!(signed_volume(&after) > before);
    }
}

#[test]
fn face_pick_on_a_pipe_wall_facet_takes_the_nearer_rim_only() {
    // A click on a branch wall facet beside the junction picks the loop
    // nearest the click. The facet's own loop touches both the junction
    // and the branch's far rim; taking both mixed a swept loop with a
    // catalogue rim and refused (the playtest's tee). The wall is one
    // logical face, and its loop nearest the click is the junction.
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playtest_pipe_tee.hew"),
    )
    .unwrap();
    let doc = kernel::Document::load(&bytes).unwrap();
    let tee = doc.object(doc.visible_object_ids()[0]).unwrap();
    // A branch wall facet (claim axis along y) with a junction edge.
    let junction: Vec<EdgeId> = tee
        .sharp_edges()
        .into_iter()
        .filter(|&e| {
            let (a, b) = tee.edge_faces_for_test(e).unwrap();
            matches!(
                (tee.faces()[a].surface, tee.faces()[b].surface),
                (Some(SurfaceRef::Cylinder { axis: p, .. }), Some(SurfaceRef::Cylinder { axis: q, .. }))
                    if p.dot(q).abs() < 0.5
            )
        })
        .collect();
    assert_eq!(junction.len(), 40);
    let e = junction[0];
    let (a, b) = tee.edge_faces_for_test(e).unwrap();
    let branch_facet = [a, b]
        .into_iter()
        .find(|&f| matches!(tee.faces()[f].surface, Some(SurfaceRef::Cylinder { axis, .. }) if axis.y.abs() > 0.9))
        .unwrap();
    let (p, q) = tee.edge_endpoints(e).unwrap();
    // Just off the junction, on the branch side (+y).
    let near = p + (q - p) * 0.5 + Vec3::new(0.0, 0.005, 0.0);
    let picked = tee.face_loop_edges(branch_facet, near);
    let mut want = junction.clone();
    want.sort();
    let mut got = picked.clone();
    got.sort();
    assert_eq!(got, want, "the junction loop, not the far rim too");
    let (after, report) = blend_ok(
        tee,
        &spec(
            picked,
            BlendProfile::Round {
                radius: 0.03,
                segments: 4,
            },
        ),
    );
    assert_eq!(report.blends.len(), 1);
    assert!(signed_volume(&after) > signed_volume(tee));
    // Near the far end, the same facet picks the far rim.
    let far = tee.face_loop_edges(branch_facet, p3(5.0, 0.79, 0.47));
    // Exactly the far rim: every sharp edge at the branch's end, nothing else.
    let end_y = tee
        .vertices()
        .values()
        .map(|v| v.position.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut rim: Vec<EdgeId> = tee
        .sharp_edges()
        .into_iter()
        .filter(|&e| {
            let (p, q) = tee.edge_endpoints(e).unwrap();
            (p.y - end_y).abs() < 1e-9 && (q.y - end_y).abs() < 1e-9
        })
        .collect();
    rim.sort();
    let mut far = far;
    far.sort();
    assert!(!rim.is_empty());
    assert_eq!(far, rim, "the far rim, whole and alone");
}

// ---- smooth shading across a blend's strips and ridges ----------------------

/// The shading normal `face` gets at corner `p`, or `None` when the fan
/// leaves it to the face's own path.
fn shade_at(obj: &Object, face: kernel::FaceId, p: Point3) -> Option<Vec3> {
    obj.blend_shading_normals(face)
        .into_iter()
        .find(|(q, _)| *q == p)
        .map(|(_, n)| n)
}

fn angle_deg(a: Vec3, b: Vec3) -> f64 {
    a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
}

#[test]
fn a_swept_strip_shades_one_normal_per_vertex_and_meets_its_rails_tangent() {
    let (tee, edges) = pipe_tee();
    let (after, report) = tee.blend_edges(&swept_round(edges, 0.03)).unwrap();
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    assert!(
        claims(&after, &report).is_empty(),
        "a swept (unclaimed) strip"
    );
    let (mut across_strip, mut rail_vertices) = (0, 0);
    for e in after.edges().keys() {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        let (p, q) = after.edge_endpoints(e).unwrap();
        match (new.contains(&x), new.contains(&y)) {
            // Inside the strip: both facets read one normal at each end
            // (so no facet shows), and it is a unit vector.
            (true, true) => {
                for v in [p, q] {
                    let (a, b) = (shade_at(&after, x, v), shade_at(&after, y, v));
                    let (a, b) = (a.expect("fan"), b.expect("fan"));
                    assert!((a - b).length() < 1e-12, "{e:?} at {v:?}");
                    assert!((a.length() - 1.0).abs() < 1e-12);
                    across_strip += 1;
                }
            }
            // A rail: the strip takes the pipe wall's own normal there.
            (true, false) | (false, true) => {
                assert_ne!(
                    after.edge_smoothness(e),
                    EdgeSmoothness::Hard,
                    "a rail of a swept strip is not a hard edge: {e:?}"
                );
                let (strip, wall) = if new.contains(&x) { (x, y) } else { (y, x) };
                for v in [p, q] {
                    let got = shade_at(&after, strip, v).expect("anchored");
                    let want = match after.faces()[wall].surface {
                        Some(SurfaceRef::Cylinder {
                            axis_point, axis, ..
                        }) => {
                            let d = v - axis_point;
                            let r = (d - axis * d.dot(axis)).normalized().unwrap();
                            if r.dot(after.faces()[wall].plane.normal()) < 0.0 {
                                -r
                            } else {
                                r
                            }
                        }
                        _ => after.faces()[wall].plane.normal(),
                    };
                    assert!(
                        angle_deg(got, want) < 2.0,
                        "rail {e:?} at {v:?}: {}",
                        angle_deg(got, want)
                    );
                    rail_vertices += 1;
                }
            }
            (false, false) => {}
        }
    }
    assert!(across_strip > 0 && rail_vertices > 0);
}

#[test]
fn a_ridge_between_two_rim_rounds_shades_across() {
    // Rounding two edges that meet at a corner leaves a ridge where the
    // strips meet; both sides read the same normal along it.
    let obj = unit_box();
    let e1 = edge_between(&obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    let e2 = edge_between(&obj, p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 1.0));
    let (after, report) = blend_ok(
        &obj,
        &spec(
            vec![e1, e2],
            BlendProfile::Round {
                radius: 0.2,
                segments: 6,
            },
        ),
    );
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    let mut ridges = 0;
    for e in after.edges().keys() {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        if !(new.contains(&x) && new.contains(&y))
            || after.edge_smoothness(e) == EdgeSmoothness::Seam
        {
            continue;
        }
        let (p, q) = after.edge_endpoints(e).unwrap();
        for v in [p, q] {
            let (a, b) = (shade_at(&after, x, v), shade_at(&after, y, v));
            assert!(
                (a.unwrap() - b.unwrap()).length() < 1e-12,
                "ridge {e:?} at {v:?}"
            );
        }
        ridges += 1;
    }
    assert!(ridges > 0);
}

#[test]
fn a_user_crease_inside_a_blend_keeps_its_two_sides_apart() {
    let obj = unit_box();
    let e1 = edge_between(&obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0));
    let e2 = edge_between(&obj, p3(1.0, 0.0, 1.0), p3(1.0, 1.0, 1.0));
    let (mut after, report) = blend_ok(
        &obj,
        &spec(
            vec![e1, e2],
            BlendProfile::Round {
                radius: 0.2,
                segments: 6,
            },
        ),
    );
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    // The ridge edge nearest the vertical edge of the box (its lowest).
    let ridge = after
        .edges()
        .keys()
        .filter(|&e| {
            let (x, y) = after.edge_faces_for_test(e).unwrap();
            new.contains(&x) && new.contains(&y) && after.edge_smoothness(e) != EdgeSmoothness::Seam
        })
        .min_by(|&a, &b| {
            let z = |e| {
                let (p, q) = after.edge_endpoints(e).unwrap();
                p.z.min(q.z)
            };
            z(a).total_cmp(&z(b))
        })
        .unwrap();
    let (x, y) = after.edge_faces_for_test(ridge).unwrap();
    let (p, q) = after.edge_endpoints(ridge).unwrap();
    let v = if p.z < q.z { p } else { q };
    let joined = (shade_at(&after, x, v).unwrap() - shade_at(&after, y, v).unwrap()).length();
    assert!(joined < 1e-12);
    after.set_edge_crease_for_test(ridge, true);
    let apart = (shade_at(&after, x, v).map_or(after.faces()[x].plane.normal(), |n| n)
        - shade_at(&after, y, v).map_or(after.faces()[y].plane.normal(), |n| n))
    .length();
    assert!(apart > 0.1, "a crease keeps the sides apart: {apart}");
}

#[test]
fn a_strips_end_does_not_shade_into_its_cap() {
    // A single rounded edge of a box: the strip's end vertices sit on the
    // side faces (end caps, about 90 degrees from the strip's own rail
    // normal); the strip's normal there is its rail's, not a blend with the
    // cap's.
    let obj = unit_box();
    let (after, report) = blend_ok(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Round {
                radius: 0.3,
                segments: 8,
            },
        ),
    );
    let new: BTreeSet<_> = report.new_faces.iter().copied().collect();
    let mut ends = 0;
    for e in after.edges().keys() {
        let (x, y) = after.edge_faces_for_test(e).unwrap();
        if new.contains(&x) == new.contains(&y) {
            continue;
        }
        let (strip, cap) = if new.contains(&x) { (x, y) } else { (y, x) };
        let cn = after.faces()[cap].plane.normal();
        if after.edge_smoothness(e) != EdgeSmoothness::Hard {
            continue;
        }
        let (p, q) = after.edge_endpoints(e).unwrap();
        for v in [p, q] {
            let n = shade_at(&after, strip, v).expect("anchored by its rail");
            assert!(n.dot(cn).abs() < 1e-9, "end {e:?} at {v:?}");
            ends += 1;
        }
    }
    assert!(ends > 0);
}

#[test]
fn an_equal_radius_tee_refuses_naming_where_its_walls_run_flush() {
    // Playtest 2.8.6 (D-234): two pipes of one radius in a tee. Their walls
    // are tangent where the junction passes the branch's sides, so no ball
    // fits between them there. The refusal names that point, not a
    // generic corner, and it is a refusal at every size.
    let (doc, tee, loop_edges) = app_pipe_tee_sized(0.1, 32, 0.0, 0.0);
    let obj = doc.object(tee).unwrap();
    for r in [0.005, 0.01, 0.03] {
        let err = obj
            .blend_edges(&spec(
                loop_edges.clone(),
                BlendProfile::Round {
                    radius: r,
                    segments: 4,
                },
            ))
            .unwrap_err();
        let BlendError::BlendFacesFlush { at } = err else {
            panic!("r {r}: {err:?}");
        };
        // At the side of the branch, on the main pipe's wall.
        assert!(
            (at.x - 4.9).abs() < 1e-3 || (at.x - 5.1).abs() < 1e-3,
            "{at:?}"
        );
        assert!((at.z - 0.4).abs() < 2e-3, "{at:?}");
    }
    // A thinner branch still rounds (the junction's walls cross there).
    let (doc, tee, loop_edges) = app_pipe_tee_sized(0.07, 28, 0.0, 0.0);
    blend_ok(
        doc.object(tee).unwrap(),
        &spec(
            loop_edges,
            BlendProfile::Round {
                radius: 0.01,
                segments: 4,
            },
        ),
    );
}
