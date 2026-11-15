//! WP17: Project onto (curved) faces — imprint, then straight or follow
//! depth.

mod common;

use common::curves::*;
use kernel::project::{DepthMode, DepthSpec, ImprintSpec, ProjectedRegion};
use kernel::shell::ShellSpec;
use kernel::{
    DocumentError, FaceId, Object, Point3, ShapeError, SurfaceRef, Vec3, WatertightState,
};

const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
const DOWN: Vec3 = Vec3::new(0.0, 0.0, -1.0);

/// A `w × h` rectangle centered at `c` in the plane through `c` with normal
/// `dir`, its sides along `u` and `dir × u`.
fn rect(c: Point3, dir: Vec3, u: Vec3, w: f64, h: f64) -> ProjectedRegion {
    let v = dir.cross(u);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let mut outer = vec![
        c + u * -hw + v * -hh,
        c + u * hw + v * -hh,
        c + u * hw + v * hh,
        c + u * -hw + v * hh,
    ];
    // Counter-clockwise seen from the target (against `dir`) is irrelevant
    // to Project; keep one winding.
    if (outer[1] - outer[0]).cross(outer[2] - outer[1]).dot(dir) > 0.0 {
        outer.reverse();
    }
    ProjectedRegion {
        outer,
        ..ProjectedRegion::default()
    }
}

/// An `n`-gon of radius `r` centered at `c` in the plane with normal `dir`.
fn ngon(c: Point3, dir: Vec3, u: Vec3, r: f64, n: usize) -> ProjectedRegion {
    let v = dir.cross(u);
    let outer = (0..n)
        .map(|k| {
            let t = k as f64 * std::f64::consts::TAU / n as f64 + 0.1;
            c + u * (r * t.cos()) + v * (r * t.sin())
        })
        .collect();
    ProjectedRegion {
        outer,
        ..ProjectedRegion::default()
    }
}

fn imprint(
    obj: &Object,
    dir: Vec3,
    regions: Vec<ProjectedRegion>,
) -> Result<(Object, Vec<FaceId>), ShapeError> {
    obj.project_imprint(&ImprintSpec { dir, regions })
}

fn polygon_area(pts: &[Point3]) -> f64 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for i in 0..pts.len() {
        n = n + pts[i].to_vec().cross(pts[(i + 1) % pts.len()].to_vec());
    }
    n.length() / 2.0
}

fn euler_genus(obj: &Object) -> i64 {
    let v = obj.vertices().len() as i64;
    let e = obj.edges().len() as i64;
    let f = obj.faces().len() as i64;
    // Faces with holes: each hole loop adds a boundary; count loops.
    let holes: i64 = obj
        .faces()
        .values()
        .map(|f| f.inner_loops.len() as i64)
        .sum();
    let chi = v - e + f - holes;
    let shells = obj.shells().len() as i64;
    (2 * shells - chi) / 2
}

/// A 48-facet cylinder (r 0.05, h 0.1) and a 0.02 square in front of it
/// at x = −0.2, centered at half height, projected along +x.
fn cylinder_and_square() -> (Object, Vec<ProjectedRegion>) {
    let c = cylinder(0.05, 0.1, 48);
    let sq = rect(p3(-0.2, 0.0, 0.05), X, Vec3::new(0.0, 1.0, 0.0), 0.02, 0.02);
    (c, vec![sq])
}

#[test]
fn square_onto_cylinder_wall() {
    let (c, sq) = cylinder_and_square();
    let (r, region) = imprint(&c, X, sq).unwrap();
    assert_valid_solid(&r);
    assert!(
        region.len() >= 3,
        "the square spans several facets: {}",
        region.len()
    );
    for &f in &region {
        let face = &r.faces()[f];
        assert!(
            matches!(face.surface, Some(SurfaceRef::Cylinder { radius, .. }) if radius == 0.05),
            "region sub-faces keep their cylinder claim"
        );
    }
    let area: f64 = region
        .iter()
        .map(|&f| {
            polygon_area(
                &r.loop_positions(r.faces()[f].outer_loop)
                    .collect::<Vec<_>>(),
            )
        })
        .sum();
    assert!(
        area > 0.0004 && area < 0.00042,
        "the draped area is a little over the square's: {area}"
    );
}

#[test]
fn back_side_not_imprinted() {
    let (c, sq) = cylinder_and_square();
    let (r, region) = imprint(&c, X, sq).unwrap();
    for &f in &region {
        assert!(
            r.faces()[f].plane.normal().x < 0.0,
            "only facets facing the shape"
        );
    }
    let back = |o: &Object| {
        o.faces()
            .values()
            .filter(|f| f.plane.normal().x > 0.1)
            .count()
    };
    assert_eq!(back(&r), back(&c), "the far side is untouched");
}

#[test]
fn misses() {
    let c = cylinder(0.05, 0.1, 48);
    let sq = rect(p3(-0.2, 0.0, 0.5), X, Vec3::new(0.0, 1.0, 0.0), 0.02, 0.02);
    assert_eq!(
        imprint(&c, X, vec![sq]).err(),
        Some(ShapeError::ProjectMisses)
    );
}

#[test]
fn planar_target_equals_imprint() {
    let b = box_xyz(1.0, 1.0, 1.0);
    let sq = rect(p3(0.5, 0.5, 2.0), DOWN, X, 0.2, 0.2);
    let want: Vec<Point3> = sq.outer.iter().map(|p| p3(p.x, p.y, 1.0)).collect();
    let (r, region) = imprint(&b, DOWN, vec![sq]).unwrap();
    assert_valid_solid(&r);
    assert_eq!(region.len(), 1);
    let got: Vec<Point3> = r.loop_positions(r.faces()[region[0]].outer_loop).collect();
    assert_eq!(got.len(), 4);
    for p in &want {
        assert!(
            got.iter().any(|q| q.approx_eq(*p, 1e-12)),
            "{p:?} in {got:?}"
        );
    }
    // The top is now the square and a frame with the square as its hole.
    assert_eq!(r.faces().len(), 7);
}

fn depth(
    obj: &Object,
    faces: Vec<FaceId>,
    dir: Vec3,
    depth: f64,
    mode: DepthMode,
) -> Result<Object, DocumentError> {
    let spec = DepthSpec {
        faces,
        dir,
        depth,
        mode,
        regions: Vec::new(),
    };
    match mode {
        DepthMode::Follow => obj
            .project_depth_follow(&spec)
            .map_err(DocumentError::Shape),
        DepthMode::Straight => kernel::project::project_depth_straight_for_test(obj, &spec),
    }
}

#[test]
fn straight_engrave() {
    let b = box_xyz(1.0, 1.0, 1.0);
    let (r, region) = imprint(&b, DOWN, vec![rect(p3(0.5, 0.5, 2.0), DOWN, X, 0.2, 0.2)]).unwrap();
    let e = depth(&r, region, DOWN, 0.001, DepthMode::Straight).unwrap();
    assert_valid_solid(&e);
    let dv = signed_volume(&b) - signed_volume(&e);
    assert!((dv - 0.04 * 0.001).abs() < 1e-12, "volume drop {dv}");
    // The pocket's four walls are parallelograms (rectangles here).
    let walls = e
        .faces()
        .values()
        .filter(|f| f.plane.normal().z.abs() < 1e-9)
        .filter(|f| {
            let pts: Vec<Point3> = e.loop_positions(f.outer_loop).collect();
            pts.iter().all(|p| p.z >= 0.999 - 1e-12)
        })
        .count();
    assert_eq!(walls, 4);
}

#[test]
fn emboss_negative_depth() {
    let b = box_xyz(1.0, 1.0, 1.0);
    let (r, region) = imprint(&b, DOWN, vec![rect(p3(0.5, 0.5, 2.0), DOWN, X, 0.2, 0.2)]).unwrap();
    let e = depth(&r, region, DOWN, -0.001, DepthMode::Straight).unwrap();
    assert_valid_solid(&e);
    let dv = signed_volume(&e) - signed_volume(&b);
    assert!((dv - 0.04 * 0.001).abs() < 1e-12, "volume gain {dv}");
}

#[test]
fn follow_deboss_on_cylinder() {
    let (c, sq) = cylinder_and_square();
    let (r, region) = imprint(&c, X, sq).unwrap();
    let e = depth(&r, region.clone(), X, 0.001, DepthMode::Follow).unwrap();
    assert_valid_solid(&e);
    let inner = e
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { radius, .. }) if (radius - 0.049).abs() < 1e-15))
        .count();
    assert_eq!(
        inner,
        region.len(),
        "every region facet sinks onto r − 0.001"
    );
    assert!(signed_volume(&e) < signed_volume(&c));
}

#[test]
fn straight_on_cylinder_wall() {
    let (c, sq) = cylinder_and_square();
    let (r, region) = imprint(&c, X, sq).unwrap();
    let e = depth(&r, region, X, 0.001, DepthMode::Straight).unwrap();
    assert_valid_solid(&e);
    let dv = signed_volume(&c) - signed_volume(&e);
    // The swept square loses at most its area × depth (it starts on the
    // curved wall) and at least a good part of it.
    assert!(
        dv > 0.5 * 0.0004 * 0.001 && dv <= 0.0004 * 0.001 + 1e-12,
        "volume drop {dv}"
    );
}

#[test]
fn porthole_through_wall() {
    let b = box_xyz(1.0, 1.0, 1.0);
    let top = face_with_normal(&b, Vec3::new(0.0, 0.0, 1.0));
    // A cup (top open): a hole through its side makes a handle-like solid.
    let cup = b
        .shelled(&ShellSpec {
            open: vec![top],
            thickness: 0.02,
        })
        .unwrap();
    let circle = ngon(p3(-0.5, 0.5, 0.5), X, Vec3::new(0.0, 1.0, 0.0), 0.1, 24);
    let hole_area = polygon_area(&circle.outer);
    let (r, region) = imprint(&cup, X, vec![circle.clone()]).unwrap();
    assert_eq!(region.len(), 1, "only the near wall's outside is visible");
    let e = depth(&r, region, X, 0.05, DepthMode::Straight).unwrap();
    assert_valid_solid(&e);
    assert_eq!(e.shells().len(), 1);
    assert_eq!(euler_genus(&e), 1, "the porthole goes through");
    let dv = signed_volume(&cup) - signed_volume(&e);
    assert!(
        (dv - hole_area * 0.02).abs() < 1e-12,
        "only the wall's plug is removed: {dv}"
    );

    // The closed hollow: the hole joins the outer and inner shells.
    let hollow = b
        .shelled(&ShellSpec {
            open: Vec::new(),
            thickness: 0.02,
        })
        .unwrap();
    let (r, region) = imprint(&hollow, X, vec![circle]).unwrap();
    let e = depth(&r, region, X, 0.05, DepthMode::Straight).unwrap();
    assert_valid_solid(&e);
    assert_eq!(e.shells().len(), 1, "one surface once the wall is pierced");
}

#[test]
fn sever_refused() {
    let plate = box_xyz(1.0, 0.2, 0.01);
    let strip = rect(p3(0.45, 0.1, 1.0), DOWN, X, 0.1, 0.4);
    let (r, region) = imprint(&plate, DOWN, vec![strip]).unwrap();
    assert_eq!(
        depth(&r, region, DOWN, 0.02, DepthMode::Straight).err(),
        Some(DocumentError::Shape(ShapeError::WouldSever))
    );
}

#[test]
fn engrave_everything_refused() {
    let plate = box_xyz(1.0, 0.2, 0.01);
    let all = rect(p3(0.5, 0.1, 1.0), DOWN, X, 2.0, 2.0);
    let (r, region) = imprint(&plate, DOWN, vec![all]).unwrap();
    assert_eq!(region.len(), 1, "the whole top is the region");
    assert_eq!(
        depth(&r, region, DOWN, 0.02, DepthMode::Straight).err(),
        Some(DocumentError::Shape(ShapeError::AmountOutOfRange))
    );
}

#[test]
fn follow_pierce_refused() {
    let b = box_xyz(1.0, 1.0, 1.0);
    let hollow = b
        .shelled(&ShellSpec {
            open: Vec::new(),
            thickness: 0.02,
        })
        .unwrap();
    let circle = ngon(p3(-0.5, 0.5, 0.5), X, Vec3::new(0.0, 1.0, 0.0), 0.1, 24);
    let (r, region) = imprint(&hollow, X, vec![circle]).unwrap();
    assert!(matches!(
        depth(&r, region, X, 0.05, DepthMode::Follow),
        Err(DocumentError::Shape(ShapeError::SelfIntersecting { .. }))
    ));
}

#[test]
fn imprint_refuses_bad_shapes() {
    let b = box_xyz(1.0, 1.0, 1.0);
    assert_eq!(
        imprint(&b, DOWN, Vec::new()).err(),
        Some(ShapeError::ProjectNeedsClosedShape)
    );
    // A square corner exactly over the box's edge line x = 1.
    let aligned = rect(p3(1.1, 0.5, 2.0), DOWN, X, 0.2, 0.2);
    assert_eq!(
        imprint(&b, DOWN, vec![aligned]).err(),
        Some(ShapeError::ShapeDegenerate)
    );
    let _ = WatertightState::Watertight;
}

#[test]
fn document_project_is_two_undo_steps() {
    let mut doc = kernel::Document::new();
    let ground =
        kernel::Plane::from_point_normal(p3(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
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
    let region = doc.extrudable_regions(s).unwrap()[0];
    let o = doc.extrude_region(s, region, 1.0).unwrap().0;
    let h0 = doc.state_hash();
    let spec = ImprintSpec {
        dir: DOWN,
        regions: vec![rect(p3(0.5, 0.5, 2.0), DOWN, X, 0.2, 0.2)],
    };
    let (pv, pf) = doc.project_imprint_preview(o, &spec).unwrap();
    assert_eq!(doc.state_hash(), h0);
    let (faces, _) = doc.project_imprint(o, &spec).unwrap();
    assert_eq!(faces.len(), pf.len());
    assert_eq!(doc.object(o).unwrap().faces().len(), pv.faces().len());
    let h1 = doc.state_hash();
    let d = DepthSpec {
        faces,
        dir: DOWN,
        depth: 0.001,
        mode: DepthMode::Straight,
        regions: Vec::new(),
    };
    doc.project_depth_preview(o, &d).unwrap();
    assert_eq!(doc.state_hash(), h1);
    doc.project_depth(o, &d).unwrap();
    let h2 = doc.state_hash();
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h1);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), h0);
    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.state_hash(), h2, "redo reproduces both steps");
}

/// A cylinder lying on its side above the ground, with the shape `draw`
/// makes on the ground projected straight up through it: the result.
fn porthole_up_through_a_lying_cylinder(draw: impl FnOnce(&mut kernel::Sketch)) -> Object {
    project_up_into_a_lying_cylinder(draw, 0.45, DepthMode::Straight)
}

/// A cylinder lying on its side above the ground, with the shape `draw`
/// makes on the ground projected up onto it and given `depth` in `mode`.
fn project_up_into_a_lying_cylinder(
    draw: impl FnOnce(&mut kernel::Sketch),
    depth: f64,
    mode: DepthMode,
) -> Object {
    let mut doc = kernel::Document::new();
    let side = kernel::Plane::from_point_normal(p3(0.0, 0.0, 0.0), X).unwrap();
    let s = doc.add_sketch(side);
    doc.begin_sketch_gesture(s).unwrap();
    add_circle(doc.sketch_mut(s).unwrap(), p3(0.0, 0.0, 0.3), X, 0.2, 48);
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let cylinder = doc.extrude_region(s, r, 0.6).unwrap().0;
    let g = doc.add_sketch(ground());
    doc.begin_sketch_gesture(g).unwrap();
    draw(doc.sketch_mut(g).unwrap());
    doc.end_sketch_gesture(g).unwrap();
    let spec = doc.project_spec_from_sketch(cylinder, g, &[]).unwrap();
    let (faces, _) = doc.project_imprint(cylinder, &spec).unwrap();
    doc.project_depth(
        cylinder,
        &DepthSpec {
            faces,
            dir: spec.dir,
            depth,
            mode,
            regions: spec.regions.clone(),
        },
    )
    .unwrap();
    doc.object(cylinder).unwrap().clone()
}

/// The hard edges of `obj` that run straight up (the hole's wall lines).
fn hard_vertical_edges(obj: &Object) -> usize {
    obj.edges()
        .keys()
        .filter(|&e| {
            let (a, b) = obj.edge_endpoints(e).unwrap();
            let d = b - a;
            d.z.abs() > 0.999 * d.length() && obj.edge_smoothness(e) == kernel::EdgeSmoothness::Hard
        })
        .count()
}

#[test]
fn a_circle_projected_through_a_cylinder_leaves_a_smooth_round_hole() {
    let e = porthole_up_through_a_lying_cylinder(|sk| {
        add_circle(sk, p3(0.3, 0.015, 0.0), Z, 0.05, 24);
    });
    assert_valid_solid(&e);
    assert_eq!(euler_genus(&e), 1, "the porthole goes through");
    assert_eq!(
        hard_vertical_edges(&e),
        0,
        "the hole's wall shades as one round surface"
    );
    assert!(
        e.faces().values().any(
            |f| matches!(f.surface, Some(SurfaceRef::Cylinder { radius, .. })
            if (radius - 0.05).abs() < 1e-12)
        ),
        "the hole's wall claims the drawn circle's cylinder"
    );
}

#[test]
fn a_hexagon_projected_through_a_cylinder_keeps_its_corners() {
    let e = porthole_up_through_a_lying_cylinder(|sk| {
        let p = |k: usize| {
            let t = k as f64 * std::f64::consts::TAU / 6.0 + 0.1;
            p3(0.3 + 0.05 * t.cos(), 0.015 + 0.05 * t.sin(), 0.0)
        };
        for k in 0..6 {
            sk.add_segment(p(k), p(k + 1)).unwrap();
        }
    });
    assert_valid_solid(&e);
    assert_eq!(euler_genus(&e), 1, "the porthole goes through");
    assert_eq!(
        hard_vertical_edges(&e),
        6,
        "one hard line per corner, none mid-side"
    );
}

#[test]
fn a_circle_debossed_along_a_cylinder_has_a_smooth_wall() {
    let depth = 0.005;
    let e = project_up_into_a_lying_cylinder(
        |sk| add_circle(sk, p3(0.3, 0.015, 0.0), Z, 0.05, 24),
        depth,
        DepthMode::Follow,
    );
    assert_valid_solid(&e);
    // The wall's seams run about the depth's length along the normals.
    let seams: Vec<_> = e
        .edges()
        .keys()
        .filter(|&k| {
            let (a, b) = e.edge_endpoints(k).unwrap();
            (0.85 * depth..1.3 * depth).contains(&(b - a).length())
        })
        .collect();
    assert!(
        seams.len() >= 24,
        "the wall has a seam per facet: {}",
        seams.len()
    );
    assert!(
        seams
            .iter()
            .all(|&k| e.edge_smoothness(k) != kernel::EdgeSmoothness::Hard),
        "the debossed circle's wall shades round"
    );
}

/// A label (a 0.3 × 0.2 rectangle on the plane x = −2) engraved 1 cm in
/// Follow mode onto the side of an upright cylinder of radius 0.5 whose
/// 24 facets come from a drawn circle (`claimed`) or from plain segments.
/// Returns the result and its wall seams (short level edges: the depth
/// where the rim crosses a facet edge, less at a corner inside a facet).
fn label_on_a_cylinder(claimed: bool) -> (Object, Vec<kernel::EdgeId>) {
    let mut doc = kernel::Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        if claimed {
            add_circle(sk, p3(0.0, 0.0, 0.0), Z, 0.5, 24);
        } else {
            let p = |k: usize| {
                let t = k as f64 * std::f64::consts::TAU / 24.0;
                p3(0.5 * t.cos(), 0.5 * t.sin(), 0.0)
            };
            for k in 0..24 {
                sk.add_segment(p(k), p(k + 1)).unwrap();
            }
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let r = doc.extrudable_regions(s).unwrap()[0];
    let cylinder = doc.extrude_region(s, r, 1.0).unwrap().0;
    let side = kernel::Plane::from_point_normal(p3(-2.0, 0.0, 0.0), X * -1.0).unwrap();
    let g = doc.add_sketch(side);
    doc.begin_sketch_gesture(g).unwrap();
    {
        let sk = doc.sketch_mut(g).unwrap();
        let c = [(-0.15, 0.4), (0.15, 0.4), (0.15, 0.6), (-0.15, 0.6)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(p3(-2.0, a.0, a.1), p3(-2.0, b.0, b.1))
                .unwrap();
        }
    }
    doc.end_sketch_gesture(g).unwrap();
    let spec = doc.project_spec_from_sketch(cylinder, g, &[]).unwrap();
    let (faces, _) = doc.project_imprint(cylinder, &spec).unwrap();
    let depth = 0.01;
    doc.project_depth(
        cylinder,
        &DepthSpec {
            faces,
            dir: spec.dir,
            depth,
            mode: DepthMode::Follow,
            regions: spec.regions.clone(),
        },
    )
    .unwrap();
    let e = doc.object(cylinder).unwrap().clone();
    let seams = e
        .edges()
        .keys()
        .filter(|&k| {
            let (a, b) = e.edge_endpoints(k).unwrap();
            let d = b - a;
            (0.3 * depth..1.1 * depth).contains(&d.length()) && d.z.abs() < 1e-9
        })
        .collect();
    (e, seams)
}

#[test]
fn a_followed_label_keeps_the_surface_creases_down_its_wall() {
    let hard = |e: &Object, seams: &[kernel::EdgeId]| {
        seams
            .iter()
            .filter(|&&k| e.edge_smoothness(k) == kernel::EdgeSmoothness::Hard)
            .count()
    };
    // Plain facets: each crease of the surface runs on down the wall.
    let (e, seams) = label_on_a_cylinder(false);
    assert_valid_solid(&e);
    assert!(
        seams.len() > 4,
        "the label crosses facet edges: {}",
        seams.len()
    );
    assert_eq!(
        hard(&e, &seams),
        seams.len(),
        "a crease continues down the wall"
    );
    // A drawn circle's facets shade smooth: only the label's 4 corners crease.
    let (e, seams) = label_on_a_cylinder(true);
    assert_valid_solid(&e);
    assert!(
        seams.len() > 4,
        "the label crosses facet seams: {}",
        seams.len()
    );
    assert_eq!(hard(&e, &seams), 4, "only the corners stay hard");
}
