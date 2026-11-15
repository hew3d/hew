//! Property tests for the blend engine core (WP05 §11).

mod common;

use common::curves::*;
use kernel::{
    BlendError, BlendProfile, BlendSpec, BooleanOp, CornerStyle, EdgeId, Object, Point3,
    SurfaceRef, Transform, Vec3,
};
use proptest::prelude::*;

fn round(edges: Vec<EdgeId>, radius: f64, segments: u32) -> BlendSpec {
    BlendSpec {
        edges,
        profile: BlendProfile::Round { radius, segments },
        primary: None,
        corners: CornerStyle::Default,
        stops: None,
        handles: Vec::new(),
    }
}

fn top_edges(obj: &Object, x: f64, y: f64, z: f64) -> [EdgeId; 4] {
    let c = [p3(0.0, 0.0, z), p3(x, 0.0, z), p3(x, y, z), p3(0.0, y, z)];
    [0, 1, 2, 3].map(|i| edge_between(obj, c[i], c[(i + 1) % 4]))
}

fn sorted_points(obj: &Object) -> Vec<Point3> {
    let mut v: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    v.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_top_rim_subsets(
        x in 0.1..2.0f64, y in 0.1..2.0f64, z in 0.1..2.0f64,
        mask in 1u8..16, frac in 0.0..1.0f64, n in 2u32..9,
    ) {
        let obj = box_xyz(x, y, z);
        let all = top_edges(&obj, x, y, z);
        let edges: Vec<EdgeId> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| all[i]).collect();
        let r = 1e-3 + frac * (0.45 * x.min(y).min(z) - 1e-3);
        let before = obj.to_polygons();
        match obj.blend_edges(&round(edges, r, n)) {
            Ok((after, _)) => {
                assert_valid_solid(&after);
                prop_assert!(signed_volume(&after) < signed_volume(&obj));
            }
            Err(BlendError::RadiusTooLarge { .. } | BlendError::UnsupportedVertex { .. }) => {}
            Err(e) => prop_assert!(false, "unexpected refusal {e:?}"),
        }
        prop_assert_eq!(obj.to_polygons(), before);
    }

    #[test]
    fn plan_iff_apply(
        x in 0.1..2.0f64, z in 0.1..2.0f64, mask in 1u8..16, r in 1e-3..1.5f64,
    ) {
        let obj = box_xyz(x, 1.0, z);
        let all = top_edges(&obj, x, 1.0, z);
        let edges: Vec<EdgeId> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| all[i]).collect();
        let spec = round(edges, r, 4);
        let plans = obj.blend_plans_for_test(&spec);
        match obj.blend_edges(&spec) {
            Ok(_) => prop_assert!(plans),
            Err(BlendError::Obstructed { .. }) => {}
            Err(_) => prop_assert!(!plans),
        }
    }

    #[test]
    fn max_is_tight(z in 0.05..1.0f64, over in 1.05..4.0f64, mask in 1u8..16) {
        let obj = box_xyz(1.0, 1.0, z);
        let all = top_edges(&obj, 1.0, 1.0, z);
        let edges: Vec<EdgeId> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| all[i]).collect();
        let spec = round(edges.clone(), z * over, 4);
        let limit = obj.max_blend_size(&spec).unwrap();
        prop_assume!(limit.max < z * over);
        let at = |r: f64| obj.blend_edges(&round(edges.clone(), r, 4)).is_ok();
        prop_assert!(at(limit.max * (1.0 - 1e-6)), "fits just below {}", limit.max);
        prop_assert!(!at(limit.max * (1.0 + 1e-6)), "refuses just above {}", limit.max);
    }

    #[test]
    fn blend_commutes_with_similarity(
        angle in 0.0..std::f64::consts::TAU, s in 0.5..3.0f64,
        tx in -2.0..2.0f64, mask in 1u8..16,
    ) {
        let obj = unit_box();
        let all = top_edges(&obj, 1.0, 1.0, 1.0);
        let edges: Vec<EdgeId> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| all[i]).collect();
        let t = Transform::translation(Vec3::new(tx, 0.3, -0.7))
            .then(&Transform::rotation(Vec3::new(1.0, 2.0, 3.0), angle).unwrap())
            .then(&Transform::uniform_scale(s));
        let (mut a, _) = obj.blend_edges(&round(edges.clone(), 0.2, 4)).unwrap();
        a.apply_transform(&t).unwrap();
        let mut moved = obj.clone();
        moved.apply_transform(&t).unwrap();
        let (b, _) = moved.blend_edges(&round(edges, 0.2 * s, 4)).unwrap();
        let (pa, pb) = (sorted_points(&a), sorted_points(&b));
        prop_assert_eq!(pa.len(), pb.len());
        for p in &pa {
            prop_assert!(pb.iter().any(|q| p.approx_eq(*q, 1e-9)), "{p:?}");
        }
    }
}

#[test]
fn boolean_after_fillet() {
    let obj = unit_box();
    let all = top_edges(&obj, 1.0, 1.0, 1.0);
    let (filleted, report) = obj.blend_edges(&round(vec![all[0]], 0.2, 6)).unwrap();
    let id = report.blends[0];
    // A box biting through the middle of the fillet.
    let mut cutter = box_xyz(0.2, 2.0, 2.0);
    cutter
        .apply_transform(&Transform::translation(Vec3::new(0.4, -0.5, -0.5)))
        .unwrap();
    let out = Object::boolean(
        BooleanOp::Subtract,
        &filleted,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();
    assert_valid_solid(&out);
    let strips: Vec<_> = out
        .faces()
        .values()
        .filter(|f| f.blend.is_some_and(|t| t.id == id))
        .collect();
    assert_eq!(strips.len(), 12, "each facet split in two");
    assert!(
        strips
            .iter()
            .all(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn random_cylinder_rims(
        rc in 0.05..1.0f64, h in 0.05..1.0f64, ni in 0usize..4,
        frac in 0.01..0.9f64, top in any::<bool>(),
    ) {
        let n = [24, 32, 48, 96][ni];
        let obj = cylinder(rc, h, n);
        let z = if top { h } else { 0.0 };
        let rim: Vec<EdgeId> = obj
            .edges()
            .keys()
            .filter(|&e| {
                let (a, b) = obj.edge_endpoints(e).unwrap();
                (a.z - z).abs() < 1e-12 && (b.z - z).abs() < 1e-12
            })
            .collect();
        prop_assert_eq!(rim.len(), n);
        let r = frac * rc;
        let before = obj.to_polygons();
        match obj.blend_edges(&round(rim.clone(), r, 4)) {
            Ok((after, _)) => assert_valid_solid(&after),
            Err(BlendError::RadiusTooLarge { max, .. }) => {
                prop_assert!(max < r);
                prop_assert!(obj.blend_edges(&round(rim.clone(), max * (1.0 - 1e-6), 4)).is_ok());
            }
            Err(e) => prop_assert!(false, "unexpected refusal {e:?}"),
        }
        prop_assert_eq!(obj.to_polygons(), before);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Every successful blend removes back to the original (WP08).
    #[test]
    fn blend_then_remove_is_identity(
        x in 0.2..2.0f64, y in 0.2..2.0f64, z in 0.2..2.0f64,
        mask in 1u8..16, frac in 0.05..0.9f64, n in 2u32..7, chamfer in any::<bool>(),
    ) {
        let obj = box_xyz(x, y, z);
        let all = top_edges(&obj, x, y, z);
        let edges: Vec<EdgeId> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| all[i]).collect();
        let r = frac * 0.45 * x.min(y).min(z);
        let profile = if chamfer {
            BlendProfile::Chamfer { setback_a: r, setback_b: r }
        } else {
            BlendProfile::Round { radius: r, segments: n }
        };
        let s = BlendSpec { edges, profile, primary: None, corners: CornerStyle::Default, stops: None, handles: Vec::new() };
        if let Ok((blended, report)) = obj.blend_edges(&s) {
            let mut cur = blended;
            for &id in report.blends.iter().rev() {
                cur = cur.remove_blend_for_test(id).expect("removes").0;
            }
            prop_assert!(kernel::same_geometry_for_test(&cur, &obj, kernel::tol::BLEND_VERIFY));
        }
    }
}
