//! Executable specs for rounds along the seams of imported meshes (D-72,
//! D-73, D-74): the seam a click picks on a softened scan, the rolling ball
//! against the side surfaces where the facet walk refuses, and the size a
//! refusal names.

use crate::common;

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendSpec, BooleanOp, CornerStyle, EdgeId, EdgeSmoothness, Object,
    Transform, Vec3,
};

/// A softened scan-like sphere (radius 20 mm, about 1 mm facets, `amp` of
/// radial noise) joined to a disc whose top is at `z`, or with everything
/// below `z` cut away; the seam (or rim) at `z` and its edges.
fn scan_at(amp: f64, z: f64, union: bool) -> (Object, Vec<EdgeId>) {
    let (s, _) = noisy_sphere(60, 120, 0.02, amp)
        .soften_by_angle(20f64.to_radians())
        .unwrap();
    let h = if union { 0.01 } else { 0.03 };
    let mut disc = cylinder(0.03, h, 48);
    disc.apply_transform(&Transform::translation(Vec3::new(0.0, 0.0, z - h)))
        .unwrap();
    let op = if union {
        BooleanOp::Union
    } else {
        BooleanOp::Subtract
    };
    let o = Object::boolean(op, &s, &disc, &Transform::IDENTITY).unwrap();
    let seam = o
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = o.edge_endpoints(e).unwrap();
            (a.z - z).abs() < 1e-7
                && (b.z - z).abs() < 1e-7
                && a.x.hypot(a.y) < 0.0245
                && o.edge_smoothness(e) == EdgeSmoothness::Hard
        })
        .collect();
    (o, seam)
}

fn round(edges: Vec<EdgeId>, r: f64) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Round {
            radius: r,
            segments: 4,
        },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn sorted(mut v: Vec<EdgeId>) -> Vec<EdgeId> {
    v.sort();
    v
}

#[test]
fn a_click_on_a_scan_seam_picks_the_whole_seam() {
    // The scan side is broken into several faces by the creases its noise
    // leaves sharp; the flat side is one face. The seam a user sees is the
    // whole loop where the two meet, so a click on any of it takes all of
    // it.
    for amp in [0.003, 0.006, 0.008] {
        let (o, seam) = scan_at(amp, -0.012, true);
        assert_eq!(seam.len(), 240);
        for &e in [seam[0], seam[77], seam[180]].iter() {
            assert_eq!(
                sorted(o.picked_edge(e, true)),
                sorted(seam.clone()),
                "noise {amp}"
            );
        }
    }
}

#[test]
fn a_click_on_an_imported_box_edge_stops_at_its_corners() {
    // An imported mesh's seams follow a flat side only while they stay
    // tangent: a box's top edge stays one edge.
    let (pts, faces) = unit_box().to_polygons();
    let boxed = Object::from_polygons_import_for_test(&pts, &faces).unwrap();
    for e in boxed.edges().keys() {
        assert_eq!(boxed.picked_edge(e, true), vec![e]);
    }
}

#[test]
fn a_click_on_an_imported_block_stops_at_a_step_in_its_outline() {
    // An imported block whose front has a 10 mm step between 100 mm runs:
    // measured over the long runs the outline barely turns, but the step's
    // corners are corners. The front edge stays one edge.
    let block = prism(
        &[
            (0.0, 0.0),
            (0.1, 0.0),
            (0.1, 0.01),
            (0.2, 0.01),
            (0.2, 0.06),
            (0.0, 0.06),
        ],
        0.05,
    );
    let (pts, faces) = block.to_polygons();
    let block = Object::from_polygons_import_for_test(&pts, &faces).unwrap();
    for e in block.edges().keys() {
        assert_eq!(block.picked_edge(e, true), vec![e]);
    }
}

#[test]
fn the_ball_rolls_where_the_facet_walk_refuses() {
    // Along a noisy scan's seam the ball rides bending ridges and corners,
    // steps across hollows, and passes over bumps; the facet walk alone
    // refuses, the solve against the surfaces rounds the whole seam.
    let (o, seam) = scan_at(0.003, -0.012, true);
    let spec = round(seam, 0.001);
    assert!(o.blend_edges_walk_only_for_test(&spec).is_err());
    let (after, report) = o.blend_edges(&spec).expect("the seam rounds");
    assert_valid_solid(&after);
    assert!(!report.new_faces.is_empty());
    assert!(kernel::patch::self_intersection_for_test(&after).is_none());
}

#[test]
fn a_tiny_round_on_a_long_seam_refuses_instead_of_taking_millions_of_stations() {
    // 0.1 µm along a seam about 0.1 m round would be four million ball
    // stations (a quarter of the ball apart): refused at once, not built.
    let (o, seam) = scan_at(0.003, -0.012, true);
    assert!(o.blend_edges(&round(seam, 1e-7)).is_err());
}

#[test]
fn rounds_of_several_sizes_follow_a_noisy_seam() {
    for (amp, r) in [
        (0.006, 0.001),
        (0.006, 0.002),
        (0.004, 0.004),
        (0.008, 0.003),
    ] {
        let (o, seam) = scan_at(amp, -0.012, true);
        let (after, _) = o
            .blend_edges(&round(seam, r))
            .unwrap_or_else(|e| panic!("noise {amp}, radius {r}: {e:?}"));
        assert_valid_solid(&after);
    }
}

#[test]
fn a_sliced_scan_takes_a_round_on_its_rim() {
    // A convex rim where a scan was cut flat. The flat-face planners limit
    // the round by the facets' size, which on an imported mesh nobody drew:
    // the ball rolls instead.
    for (amp, z, r) in [
        (0.0, -0.012, 0.0005),
        (0.0, -0.006, 0.001),
        (0.003, 0.0, 0.002),
    ] {
        let (o, rim) = scan_at(amp, z, false);
        let (after, _) = o
            .blend_edges(&round(rim, r))
            .unwrap_or_else(|e| panic!("noise {amp}, cut at {z}, radius {r}: {e:?}"));
        assert_valid_solid(&after);
    }
}

#[test]
fn a_round_too_large_for_a_scan_names_one_that_fits() {
    // A round too large for a 1% noisy seam on a scan: the refusal is a
    // size, and that size rounds. (This first pinned 1 mm on the cut rim,
    // which refused only for one order of the result's elements: the same
    // rim rounds at 0.8 mm to 12 mm, and at 1 mm once a boolean keeps the
    // scan's untouched faces in place, D-224. The 20 mm round on the
    // joined seam is too large in its own right.)
    let (o, rim) = scan_at(0.008, -0.012, false);
    let (after, _) = o
        .blend_edges(&round(rim.clone(), 0.001))
        .expect("the cut rim rounds at 1 mm");
    assert_valid_solid(&after);
    // Not vacuous: rounding a convex rim only removes material, and what it
    // removes is bounded by the corner strip along the rim (rim length times
    // the (1 - pi/4) r^2 section of a quarter-round, doubled for slack).
    let (v0, v1) = (signed_volume(&o), signed_volume(&after));
    let rim_len: f64 = rim
        .iter()
        .map(|&e| {
            let (p, q) = o.edge_endpoints(e).unwrap();
            (q - p).length()
        })
        .sum();
    let bound = 2.0 * rim_len * (1.0 - std::f64::consts::FRAC_PI_4) * 0.001 * 0.001;
    assert!(v1 > 0.0 && v1 <= v0, "rounded volume {v1} against {v0}");
    assert!(
        v0 - v1 <= bound,
        "removed {} exceeds bound {bound}",
        v0 - v1
    );
    let (o, seam) = scan_at(0.008, -0.012, true);
    let err = o.blend_edges(&round(seam.clone(), 0.02)).unwrap_err();
    let BlendError::RadiusTooLarge { max, .. } = err else {
        panic!("a size, not {err:?}");
    };
    assert!(max > 0.0 && max < 0.02);
    let (after, _) = o
        .blend_edges(&round(seam, max))
        .expect("the size it names rounds");
    assert_valid_solid(&after);
}

#[test]
fn every_limit_a_scan_seam_names_is_a_size_that_rounds() {
    // The size a refusal names is one the commit builds, on seams and rims
    // of several noise levels: a limit below what rolls is a nuisance, one
    // that does not build is a wrong answer.
    for (amp, union) in [(0.003, true), (0.008, true), (0.008, false)] {
        let (o, seam) = scan_at(amp, -0.012, union);
        let err = o.blend_edges(&round(seam.clone(), 0.02)).unwrap_err();
        let BlendError::RadiusTooLarge { max, .. } = err else {
            panic!("noise {amp}, union {union}: a size, not {err:?}");
        };
        assert!(max > 0.0 && max < 0.02, "noise {amp}, union {union}: {max}");
        let (after, _) = o
            .blend_edges(&round(seam, max))
            .unwrap_or_else(|e| panic!("noise {amp}, union {union}: {max} refused: {e:?}"));
        assert_valid_solid(&after);
    }
}
