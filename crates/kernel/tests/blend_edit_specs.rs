//! Executable specs for editing blends (WP08): features, removal by
//! verified restoration, resize, corner re-solve, Push/Pull and Paint.

mod common;

use common::curves::*;
use kernel::{
    BlendError, BlendKind, BlendProfile, BlendSpec, CornerStyle, EdgeId, Object, Point3, Vec3,
};

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

fn round(r: f64, n: u32) -> BlendProfile {
    BlendProfile::Round {
        radius: r,
        segments: n,
    }
}

fn front_top(obj: &Object) -> EdgeId {
    edge_between(obj, p3(0.0, 0.0, 1.0), p3(1.0, 0.0, 1.0))
}

#[test]
fn remove_single_box_edge_fillet_restores_the_box() {
    let obj = unit_box();
    let (filleted, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let (back, edges) = filleted.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert_eq!(edges.len(), 1);
    assert!(kernel::same_geometry_for_test(
        &back,
        &obj,
        kernel::tol::BLEND_VERIFY
    ));
}

#[test]
fn remove_and_resize_keep_the_marks_of_the_edges_the_fillet_shortened() {
    // A soft edge and a creased edge end at the corner the fillet cuts:
    // the fillet shortens both and they keep their marks (D-69); Remove
    // Fillet brings them back at full length with those marks, and so does
    // Resize. Before, the full-length edges came back bare.
    let mut obj = unit_box();
    let soft = edge_between(&obj, p3(0.0, 0.0, 1.0), p3(0.0, 1.0, 1.0));
    let crease = edge_between(&obj, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0));
    obj.set_edge_soft_for_test(soft, true);
    obj.set_edge_crease_for_test(crease, true);
    obj.validate().unwrap();
    // The soft top edge and the creased vertical, cut back by radius `r`.
    let marks = |o: &Object, r: f64| {
        let soft = edge_between(o, p3(0.0, r, 1.0), p3(0.0, 1.0, 1.0));
        let crease = edge_between(o, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0 - r));
        (o.edges()[soft].soft, o.edges()[crease].crease)
    };
    let (filleted, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    assert_eq!(marks(&filleted, 0.1), (true, true));
    let (back, _) = filleted.remove_blend_for_test(report.blends[0]).unwrap();
    assert_valid_solid(&back);
    assert_eq!(marks(&back, 0.0), (true, true));
    assert!(kernel::same_geometry_for_test(
        &back,
        &obj,
        kernel::tol::BLEND_VERIFY
    ));
    let (resized, _) = filleted
        .resize_blend_for_test(report.blends[0], &round(0.2, 6))
        .unwrap();
    assert_eq!(marks(&resized, 0.2), (true, true));
}

#[test]
fn a_crease_on_a_fillets_own_rail_does_not_block_remove() {
    // The rail goes with the fillet, so a mark on it is not one Remove
    // must reproduce.
    let obj = unit_box();
    let (mut filleted, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let rail = edge_between(&filleted, p3(0.0, 0.1, 1.0), p3(1.0, 0.1, 1.0));
    filleted.set_edge_crease_for_test(rail, true);
    filleted.validate().unwrap();
    let (back, _) = filleted
        .remove_blend_for_test(report.blends[0])
        .expect("Remove still works");
    assert_valid_solid(&back);
}

fn removes_back(obj: &Object, s: &BlendSpec) {
    let (blended, report) = obj.blend_edges(s).expect("blend");
    let mut cur = blended;
    // Remove every pass (two-pass corners give two ids), newest first.
    for &id in report.blends.iter().rev() {
        let (back, _) = cur.remove_blend_for_test(id).expect("remove");
        assert_valid_solid(&back);
        cur = back;
    }
    assert!(
        kernel::same_geometry_for_test(&cur, obj, kernel::tol::BLEND_VERIFY),
        "restored geometry differs"
    );
}

#[test]
fn remove_restores_the_box() {
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
    removes_back(&obj, &spec(edges, round(0.1, 6)));
}

#[test]
fn remove_restores_cylinder_rim() {
    let obj = cylinder(0.5, 1.0, 48);
    let rim: Vec<EdgeId> = obj
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.z > 0.5 && b.z > 0.5
        })
        .collect();
    removes_back(&obj, &spec(rim, round(0.1, 6)));
}

#[test]
fn remove_restores_hemisphere_cap() {
    let obj = cylinder(0.05, 1.0, 48);
    let rim: Vec<EdgeId> = obj
        .edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            a.z > 0.5 && b.z > 0.5
        })
        .collect();
    removes_back(&obj, &spec(rim, round(0.05, 6)));
}

#[test]
fn remove_restores_bullnose_face() {
    let obj = box_xyz(1.0, 0.1, 0.02);
    let edges = vec![
        edge_between(&obj, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        edge_between(&obj, p3(0.0, 0.0, 0.02), p3(1.0, 0.0, 0.02)),
    ];
    removes_back(&obj, &spec(edges, BlendProfile::Bullnose { segments: 8 }));
}

#[test]
fn remove_restores_d_post_crossing() {
    let obj = d_post(0.5, 1.0, 48);
    let e = edge_between(&obj, p3(0.0, 0.5, 0.0), p3(0.0, 0.5, 1.0));
    removes_back(&obj, &spec(vec![e], round(0.15, 6)));
}

#[test]
fn remove_rolled_corners() {
    let obj = unit_box();
    let all: Vec<EdgeId> = obj.edges().keys().collect();
    removes_back(&obj, &spec(all, round(0.1, 6)));
}

#[test]
fn remove_chamfer_and_profile() {
    let obj = unit_box();
    removes_back(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Chamfer {
                setback_a: 0.1,
                setback_b: 0.05,
            },
        ),
    );
    removes_back(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Custom {
                shape: (kernel::bundled_profiles()[0].2)(),
                width: 0.1,
                arc_segments: 4,
            },
        ),
    );
}

#[test]
fn remove_cove_and_concave() {
    let obj = unit_box();
    removes_back(
        &obj,
        &spec(
            vec![front_top(&obj)],
            BlendProfile::Cove {
                radius: 0.1,
                segments: 4,
            },
        ),
    );
    let l = l_block();
    let e = edge_between(&l, p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 1.0));
    removes_back(&l, &spec(vec![e], round(0.1, 4)));
}

#[test]
fn cut_across_blend_still_removes() {
    // A slot cut square across a fillet leaves exactly a fillet on the
    // slotted box: the verified restoration removes it to the slotted box.
    let obj = unit_box();
    let mut slot = box_xyz(0.1, 0.5, 0.5);
    slot.apply_transform(&kernel::Transform::translation(Vec3::new(0.45, -0.1, 0.7)))
        .unwrap();
    let (f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.2, 6)))
        .unwrap();
    let cut = Object::boolean(
        kernel::BooleanOp::Subtract,
        &f,
        &slot,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    let (back, _) = cut.remove_blend_for_test(report.blends[0]).unwrap();
    let slotted = Object::boolean(
        kernel::BooleanOp::Subtract,
        &obj,
        &slot,
        &kernel::Transform::IDENTITY,
    )
    .unwrap();
    assert!(kernel::same_geometry_for_test(
        &back,
        &slotted,
        kernel::tol::BLEND_VERIFY
    ));
}

#[test]
fn moved_neighbor_refuses() {
    // The top face lifted by the ordinary Push/Pull tilts the strip facet
    // beside it: no round re-blends to that. (Pushing an end face only
    // stretches the strip, which still restores.)
    let obj = unit_box();
    let (mut f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.2, 6)))
        .unwrap();
    let top = face_with_normal(&f, Vec3::new(0.0, 0.0, 1.0));
    f.push_pull(top, 0.1).unwrap();
    let before = f.to_polygons();
    assert!(matches!(
        f.remove_blend_for_test(report.blends[0]),
        Err(BlendError::BlendModified)
    ));
    assert_eq!(f.to_polygons(), before);
}

#[test]
fn features_report_every_blend() {
    let obj = unit_box();
    let (a, _) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 4)))
        .unwrap();
    let e2 = edge_between(&a, p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0));
    let (b, _) = a.blend_edges(&spec(vec![e2], round(0.05, 4))).unwrap();
    let e3 = edge_between(&b, p3(0.0, 1.0, 1.0), p3(1.0, 1.0, 1.0));
    let (c, _) = b
        .blend_edges(&spec(
            vec![e3],
            BlendProfile::Chamfer {
                setback_a: 0.05,
                setback_b: 0.05,
            },
        ))
        .unwrap();
    let fs = c.blend_features();
    assert_eq!(fs.len(), 3);
    assert!(fs.iter().all(|f| f.intact), "{fs:?}");
    let sizes: Vec<f64> = fs.iter().map(|f| f.size.unwrap()).collect();
    assert!(
        (sizes[0] - 0.1).abs() < 1e-9
            && (sizes[1] - 0.05).abs() < 1e-9
            && (sizes[2] - 0.05).abs() < 1e-9
    );
    assert_eq!(fs[2].kind, BlendKind::Chamfer);
}

#[test]
fn resize_round() {
    let obj = unit_box();
    let (f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let (resized, _) = f
        .resize_blend_for_test(report.blends[0], &round(0.05, 6))
        .unwrap();
    let (fresh, _) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.05, 6)))
        .unwrap();
    assert!(kernel::same_geometry_for_test(&resized, &fresh, 1e-9));
}

#[test]
fn resize_switch_round_to_chamfer() {
    let obj = unit_box();
    let (f, report) = obj
        .blend_edges(&spec(vec![front_top(&obj)], round(0.1, 6)))
        .unwrap();
    let ch = BlendProfile::Chamfer {
        setback_a: 0.1,
        setback_b: 0.1,
    };
    let (resized, _) = f.resize_blend_for_test(report.blends[0], &ch).unwrap();
    let (fresh, _) = obj.blend_edges(&spec(vec![front_top(&obj)], ch)).unwrap();
    assert!(kernel::same_geometry_for_test(&resized, &fresh, 1e-9));
    let _ = Point3::new(0.0, 0.0, 0.0);
}

#[test]
fn remove_stopped() {
    let obj = unit_box();
    let mut s = spec(vec![front_top(&obj)], round(0.1, 6));
    s.stops = Some(kernel::BlendStops {
        start: 0.3,
        end: 0.2,
    });
    removes_back(&obj, &s);
}

#[test]
fn remove_tapered() {
    let obj = unit_box();
    let mut t = spec(vec![front_top(&obj)], round(0.05, 6));
    t.handles = vec![
        kernel::RadiusHandle {
            at: 0.0,
            radius: 0.05,
        },
        kernel::RadiusHandle {
            at: 1.0,
            radius: 0.1,
        },
    ];
    removes_back(&obj, &t);
}

fn verticals(obj: &Object) -> Vec<EdgeId> {
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(obj, p3(x, y, 0.0), p3(x, y, 1.0)))
        .collect()
}

fn rim_edges(obj: &Object) -> Vec<EdgeId> {
    let top = kernel_face_up(obj);
    obj.face_loop_edges(top, p3(0.5, 0.0, 1.0))
}

fn kernel_face_up(obj: &Object) -> kernel::FaceId {
    face_with_normal(obj, Vec3::new(0.0, 0.0, 1.0))
}

#[test]
fn resolve_rim_then_vertical_equals_vertical_then_rim() {
    let obj = unit_box();
    // Vertical first, then rim (no re-solve needed).
    let (v, _) = obj
        .blend_edges(&spec(verticals(&obj), round(0.2, 6)))
        .unwrap();
    let (vr, _) = v.blend_edges(&spec(rim_edges(&v), round(0.05, 4))).unwrap();
    // Rim first, then verticals: re-solved to the same thing.
    let (r, _) = obj
        .blend_edges(&spec(rim_edges(&obj), round(0.05, 4)))
        .unwrap();
    let ends: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&r, p3(x, y, 0.0), p3(x, y, 0.95)))
        .collect();
    let (rv, report) = r.blend_edges(&spec(ends, round(0.2, 6))).unwrap();
    assert_eq!(report.blends.len(), 2);
    assert!(kernel::same_geometry_for_test(&rv, &vr, 1e-9));
}

#[test]
fn resolve_equal_radii_rolls() {
    let obj = unit_box();
    let (r, _) = obj
        .blend_edges(&spec(rim_edges(&obj), round(0.1, 4)))
        .unwrap();
    let ends: Vec<EdgeId> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .iter()
        .map(|&(x, y)| edge_between(&r, p3(x, y, 0.0), p3(x, y, 0.9)))
        .collect();
    let (rv, _) = r.blend_edges(&spec(ends, round(0.1, 4))).unwrap();
    assert!(
        rv.faces()
            .values()
            .any(|f| matches!(f.surface, Some(kernel::SurfaceRef::Sphere { .. })))
    );
}

#[test]
fn remove_small_rim() {
    let obj = unit_box();
    removes_back(&obj, &spec(rim_edges(&obj), round(0.05, 4)));
}

#[test]
fn remove_two_opposite_edges() {
    let obj = box_xyz(0.2, 0.2, 0.2);
    let e1 = edge_between(&obj, p3(0.2, 0.0, 0.2), p3(0.2, 0.2, 0.2));
    let e2 = edge_between(&obj, p3(0.0, 0.2, 0.2), p3(0.0, 0.0, 0.2));
    removes_back(&obj, &spec(vec![e1, e2], round(0.0045, 2)));
}

#[test]
fn remove_three_rim_edges() {
    let obj = box_xyz(0.2, 0.2, 0.2);
    let c = [
        p3(0.0, 0.0, 0.2),
        p3(0.2, 0.0, 0.2),
        p3(0.2, 0.2, 0.2),
        p3(0.0, 0.2, 0.2),
    ];
    let e: Vec<EdgeId> = [0, 1, 3]
        .iter()
        .map(|&i| edge_between(&obj, c[i], c[(i + 1) % 4]))
        .collect();
    removes_back(&obj, &spec(e, round(0.0045, 2)));
}

#[test]
fn remove_two_opposite_fine() {
    let obj = box_xyz(0.2, 0.2, 0.2);
    let c = [
        p3(0.0, 0.0, 0.2),
        p3(0.2, 0.0, 0.2),
        p3(0.2, 0.2, 0.2),
        p3(0.0, 0.2, 0.2),
    ];
    let e: Vec<EdgeId> = [0, 2]
        .iter()
        .map(|&i| edge_between(&obj, c[i], c[(i + 1) % 4]))
        .collect();
    removes_back(&obj, &spec(e.clone(), round(0.0045, 4)));
    removes_back(&obj, &spec(e, round(0.0045, 6)));
}

#[test]
fn remove_u_large_radius() {
    let obj = box_xyz(0.2, 0.2, 0.2);
    let c = [
        p3(0.0, 0.0, 0.2),
        p3(0.2, 0.0, 0.2),
        p3(0.2, 0.2, 0.2),
        p3(0.0, 0.2, 0.2),
    ];
    let e: Vec<EdgeId> = [0, 1, 2]
        .iter()
        .map(|&i| edge_between(&obj, c[i], c[(i + 1) % 4]))
        .collect();
    removes_back(&obj, &spec(e, round(0.0567, 2)));
}
