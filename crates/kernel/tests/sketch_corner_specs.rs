//! Executable specs for 2D corner blends (`Sketch::blend_corner`,
//! `Sketch::blendable_corners`, `Document::sketch_blend_corner`, and the
//! drafted-face check `Document::draft_check_corner`): rounding or
//! chamfering the vertex where exactly two straight sketch segments meet,
//! in world sketches and in face drawings (imprints).

use std::f64::consts::PI;

use kernel::{
    CornerProfile, CurveGeom, Document, KernelOp, Object, ObjectId, Plane, Point3, Sketch,
    SketchCurveKind, SketchError, SketchVertexId, Vec3,
};

use crate::common;
use common::curves::{add_arc, assert_valid_solid, claim_census, ground, p3};

/// Slack for comparing computed circle points; the kernel's own tolerances
/// are not the subject of these specs.
const EPS: f64 = 1e-12;

fn square(size: f64) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    let c = [(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        s.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0)).unwrap();
    }
    s
}

fn polygon(points: &[(f64, f64)]) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        s.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0)).unwrap();
    }
    s
}

fn vertex_at(s: &Sketch, p: Point3) -> SketchVertexId {
    s.vertices()
        .iter()
        .find(|(_, v)| v.position.approx_eq(p, 1e-9))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no sketch vertex at {p:?}"))
}

fn has_edge(s: &Sketch, a: Point3, b: Point3) -> bool {
    s.edges().values().any(|e| {
        let (p, q) = (s.vertices()[e.from].position, s.vertices()[e.to].position);
        (p.approx_eq(a, 1e-9) && q.approx_eq(b, 1e-9))
            || (p.approx_eq(b, 1e-9) && q.approx_eq(a, 1e-9))
    })
}

fn round(radius: f64, segments: u32) -> CornerProfile {
    CornerProfile::Round { radius, segments }
}

fn near(a: Point3, b: Point3) -> bool {
    a.approx_eq(b, EPS)
}

// ------------------------------------------------------------- the sketch

#[test]
fn rectangle_corner_round() {
    let mut s = square(1.0);
    let v = vertex_at(&s, p3(0.0, 0.0, 0.0));
    let report = s.blend_corner(v, round(0.2, 6)).expect("the corner rounds");

    // The region still closes.
    assert_eq!(s.regions().len(), 1, "one closed region");
    let region = s.regions().keys().next().unwrap();
    s.profile(region).expect("the region still has a profile");

    // The arc is one analytic circle chain: center (0.2, 0.2), radius 0.2.
    let curve = report.curve.expect("a round makes a curve chain");
    let analytic = s
        .curve_analytic(curve)
        .expect("the chain carries its circle");
    assert_eq!(analytic.kind, SketchCurveKind::Circle);
    assert!(near(analytic.geom.center, p3(0.2, 0.2, 0.0)));
    assert!((analytic.geom.radius - 0.2).abs() < EPS);

    // Both lines are trimmed to the tangent points, 0.8 long.
    let (t1, t2) = (p3(0.2, 0.0, 0.0), p3(0.0, 0.2, 0.0));
    assert!(
        s.vertices()
            .values()
            .all(|v| !near(v.position, p3(0.0, 0.0, 0.0)))
    );
    let bottom = s
        .edges()
        .values()
        .filter(|e| e.curve.is_none())
        .find(|e| {
            let (a, b) = (s.vertices()[e.from].position, s.vertices()[e.to].position);
            (near(a, t1) && near(b, p3(1.0, 0.0, 0.0)))
                || (near(b, t1) && near(a, p3(1.0, 0.0, 0.0)))
        })
        .expect("the bottom edge is trimmed to the tangent point");
    let len = (s.vertices()[bottom.from].position - s.vertices()[bottom.to].position).length();
    assert!((len - 0.8).abs() < EPS, "trimmed to 0.8, got {len}");
    assert!(
        has_edge(&s, t2, p3(0.0, 1.0, 0.0)),
        "the left edge is trimmed too"
    );

    // Six facets, every point on the circle, ends exactly the tangent points.
    let arc: Vec<_> = s
        .edges()
        .iter()
        .filter(|(_, e)| e.curve == Some(curve))
        .collect();
    assert_eq!(arc.len(), 6);
    for (_, e) in &arc {
        for v in [e.from, e.to] {
            let d = (s.vertices()[v].position - analytic.geom.center).length();
            assert!((d - 0.2).abs() < EPS, "arc point at distance {d}");
        }
    }
    assert!(
        s.vertices()
            .values()
            .any(|v| v.position == t1 || near(v.position, t1))
    );
    assert_eq!(
        report.new_edges.len(),
        2 + 6,
        "two trimmed lines and six facets"
    );
    assert!(report.new_edges.iter().all(|&e| s.edges().contains_key(e)));
}

#[test]
fn rectangle_four_corners_then_extrude() {
    let mut s = square(1.0);
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let v = vertex_at(&s, p3(x, y, 0.0));
        s.blend_corner(v, round(0.2, 6))
            .expect("each corner rounds");
    }
    assert_eq!(s.regions().len(), 1);
    assert!(
        s.blendable_corners().is_empty(),
        "no straight corners are left"
    );
    let region = s.regions().keys().next().unwrap();
    let obj = Object::from_extrusion(&s.profile(region).unwrap(), 1.0).unwrap();
    assert_valid_solid(&obj);

    // The four corner walls (six facets each) claim their cylinders...
    let census = claim_census(&obj);
    assert_eq!(census.get("cylinder"), Some(&24), "{census:?}");
    // ...and the seams between facets of one wall stay un-softened here:
    // smoothness is derived from the claims instead.
    assert!(
        obj.edges().values().all(|e| !e.soft),
        "extrusion marks no edge soft"
    );
    let (mut seams, mut tangents) = (0, 0);
    for e in obj.edges().keys() {
        match obj.edge_smoothness(e) {
            kernel::EdgeSmoothness::Seam => seams += 1,
            kernel::EdgeSmoothness::Tangent => tangents += 1,
            kernel::EdgeSmoothness::Hard => {}
        }
    }
    // Four arcs of six facets: five seams inside each arc, and the arc's
    // two joints with the straight walls are tangent.
    assert_eq!((seams, tangents), (4 * 5, 4 * 2));
}

#[test]
fn obtuse_and_acute_corners() {
    // A rhombus of side 2: 60 degrees at (0,0), 120 degrees at (2,0).
    let h = 3.0_f64.sqrt();
    let mut s = polygon(&[(0.0, 0.0), (2.0, 0.0), (3.0, h), (1.0, h)]);
    let r = 0.3;
    let acute = vertex_at(&s, p3(0.0, 0.0, 0.0));
    let obtuse = vertex_at(&s, p3(2.0, 0.0, 0.0));
    let ra = s
        .blend_corner(acute, round(r, 4))
        .expect("60 degree corner");
    let ro = s
        .blend_corner(obtuse, round(r, 4))
        .expect("120 degree corner");

    let setback_60 = r / (PI / 6.0).tan();
    let setback_120 = r / (PI / 3.0).tan();
    // The shared bottom edge lost one setback at each end.
    assert!(has_edge(
        &s,
        p3(setback_60, 0.0, 0.0),
        p3(2.0 - setback_120, 0.0, 0.0)
    ));
    // The arcs are tangent circles of radius r at the right distance from
    // the corner: center at r / sin(theta/2) along the bisector.
    let ca = s.curve_analytic(ra.curve.unwrap()).unwrap().geom;
    let co = s.curve_analytic(ro.curve.unwrap()).unwrap().geom;
    assert!(((ca.center - p3(0.0, 0.0, 0.0)).length() - r / (PI / 6.0).sin()).abs() < EPS);
    assert!(((co.center - p3(2.0, 0.0, 0.0)).length() - r / (PI / 3.0).sin()).abs() < EPS);
    assert!((ca.center.y - r).abs() < EPS, "tangent to the bottom edge");
    assert!((co.center.y - r).abs() < EPS, "tangent to the bottom edge");
    assert_eq!(s.regions().len(), 1);
}

#[test]
fn chamfer_corner() {
    let mut s = square(1.0);
    let v = vertex_at(&s, p3(0.0, 0.0, 0.0));
    let report = s
        .blend_corner(
            v,
            CornerProfile::Chamfer {
                setback_a: 0.3,
                setback_b: 0.1,
            },
        )
        .expect("the corner chamfers");
    assert!(report.curve.is_none(), "a chamfer is a plain segment");
    assert_eq!(report.new_edges.len(), 3);
    // `a` runs along the edge that starts the counter-clockwise order:
    // (0,1) -> (0,0) -> (1,0) is the order of the region's boundary here.
    let (ta, tb) = (p3(0.0, 0.3, 0.0), p3(0.1, 0.0, 0.0));
    assert!(has_edge(&s, ta, tb), "the chamfer joins the tangent points");
    assert!(has_edge(&s, ta, p3(0.0, 1.0, 0.0)));
    assert!(has_edge(&s, tb, p3(1.0, 0.0, 0.0)));
    assert_eq!(s.regions().len(), 1);
    assert!(s.profile(s.regions().keys().next().unwrap()).is_ok());
}

#[test]
fn too_large() {
    let s0 = square(1.0);
    let corner = vertex_at(&s0, p3(0.0, 0.0, 0.0));
    let max = {
        let mut s = s0.clone();
        match s.blend_corner(corner, round(2.0, 6)) {
            Err(SketchError::CornerRadiusTooLarge { max }) => max,
            other => panic!("expected CornerRadiusTooLarge, got {other:?}"),
        }
    };
    assert!(
        max > 0.999 && max < 1.0,
        "max is just under the edge: {max}"
    );

    let mut fits = s0.clone();
    fits.blend_corner(corner, round(max * (1.0 - 1e-6), 6))
        .expect("just under the maximum fits");
    let mut over = s0.clone();
    assert!(matches!(
        over.blend_corner(corner, round(max * (1.0 + 1e-6), 6)),
        Err(SketchError::CornerRadiusTooLarge { .. })
    ));
    assert_eq!(over.edges().len(), 4, "a refusal changes nothing");

    // A chamfer reports the largest uniform scale of both setbacks.
    let mut c = s0.clone();
    let Err(SketchError::CornerRadiusTooLarge { max: scale }) = c.blend_corner(
        corner,
        CornerProfile::Chamfer {
            setback_a: 2.0,
            setback_b: 0.5,
        },
    ) else {
        panic!("expected CornerRadiusTooLarge");
    };
    assert!((scale - 0.5).abs() < 1e-6, "scale {scale}");
    c.blend_corner(
        corner,
        CornerProfile::Chamfer {
            setback_a: 2.0 * scale * (1.0 - 1e-6),
            setback_b: 0.5 * scale * (1.0 - 1e-6),
        },
    )
    .expect("just under the scaled maximum fits");
}

#[test]
fn segments_out_of_range_and_degenerate_profiles_are_refused() {
    let mut s = square(1.0);
    let v = vertex_at(&s, p3(0.0, 0.0, 0.0));
    assert_eq!(
        s.blend_corner(v, round(0.2, 0)).unwrap_err(),
        SketchError::SegmentsBelowFloor
    );
    assert_eq!(
        s.blend_corner(v, round(0.2, 1025)).unwrap_err(),
        SketchError::SegmentsAboveCap
    );
    s.blend_corner(v, round(0.2, 1))
        .expect("one facet is allowed");
    let mut s = square(1.0);
    assert_eq!(
        s.blend_corner(v, round(0.0, 6)).unwrap_err(),
        SketchError::DegenerateCurve
    );
    assert_eq!(
        s.blend_corner(
            v,
            CornerProfile::Chamfer {
                setback_a: -1.0,
                setback_b: 0.1
            }
        )
        .unwrap_err(),
        SketchError::DegenerateSegment
    );
    // A stale vertex.
    let mut t = square(1.0);
    let w = vertex_at(&t, p3(1.0, 1.0, 0.0));
    t.blend_corner(w, round(0.1, 4)).unwrap();
    assert_eq!(
        t.blend_corner(w, round(0.1, 4)).unwrap_err(),
        SketchError::UnknownVertex
    );
}

#[test]
fn curve_vertex_refused() {
    // A quarter arc whose end meets a line: the vertex has two edges, one a
    // curve facet.
    let mut s = Sketch::on_plane(ground());
    add_arc(&mut s, p3(0.0, 0.0, 0.0), 1.0, 0.0, PI / 2.0, 8);
    s.add_segment(p3(0.0, 1.0, 0.0), p3(-1.0, 1.0, 0.0))
        .unwrap();
    let joint = vertex_at(&s, p3(0.0, 1.0, 0.0));
    let before = s.clone();
    assert_eq!(
        s.blend_corner(joint, round(0.1, 4)).unwrap_err(),
        SketchError::CornerNotBlendable
    );
    assert_eq!(s.edges().len(), before.edges().len(), "nothing changed");
    // An interior arc vertex too, and a free end (one edge).
    let inner = vertex_at(&s, {
        let a = PI / 2.0 / 8.0;
        p3(a.cos(), a.sin(), 0.0)
    });
    assert_eq!(
        s.blend_corner(inner, round(0.01, 4)).unwrap_err(),
        SketchError::CornerNotBlendable
    );
    let free = vertex_at(&s, p3(-1.0, 1.0, 0.0));
    assert_eq!(
        s.blend_corner(free, round(0.1, 4)).unwrap_err(),
        SketchError::CornerNotBlendable
    );
    assert!(s.blendable_corners().is_empty());
}

#[test]
fn junction_with_three_edges_refused() {
    let mut s = square(1.0);
    s.add_segment(p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)).unwrap();
    let v = vertex_at(&s, p3(0.0, 0.0, 0.0));
    assert_eq!(
        s.blend_corner(v, round(0.1, 4)).unwrap_err(),
        SketchError::CornerNotBlendable
    );
}

#[test]
fn collinear_refused() {
    let mut s = Sketch::on_plane(ground());
    s.add_segment(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)).unwrap();
    s.add_segment(p3(1.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)).unwrap();
    // Two collinear segments sharing an end keep that vertex.
    let mid = vertex_at(&s, p3(1.0, 0.0, 0.0));
    assert_eq!(
        s.blend_corner(mid, round(0.1, 4)).unwrap_err(),
        SketchError::CornerIsStraight
    );
    assert!(s.blendable_corners().is_empty());
}

#[test]
fn blendable_corners_lists_straight_corners_only() {
    let s = square(1.0);
    assert_eq!(s.blendable_corners().len(), 4);

    // A rectangle with a bump arc on top: the arc's two end vertices and
    // interior vertices are not blendable; the bottom corners are.
    let mut t = Sketch::on_plane(ground());
    t.add_segment(p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)).unwrap();
    t.add_segment(p3(2.0, 0.0, 0.0), p3(2.0, 1.0, 0.0)).unwrap();
    add_arc(&mut t, p3(1.0, 1.0, 0.0), 1.0, 0.0, PI, 8);
    t.add_segment(p3(0.0, 1.0, 0.0), p3(0.0, 0.0, 0.0)).unwrap();
    let corners = t.blendable_corners();
    assert_eq!(corners.len(), 2, "only the two bottom corners");
    for v in corners {
        assert!(t.vertices()[v].position.y == 0.0);
    }
}

#[test]
fn edit_is_deterministic() {
    let build = || {
        let mut s = square(1.0);
        let v = vertex_at(&s, p3(1.0, 1.0, 0.0));
        let r = s.blend_corner(v, round(0.25, 5)).unwrap();
        (s, r)
    };
    let (a, ra) = build();
    let (b, rb) = build();
    assert_eq!(ra, rb);
    let pos = |s: &Sketch| -> Vec<Point3> { s.vertices().values().map(|v| v.position).collect() };
    assert_eq!(pos(&a), pos(&b));
}

#[test]
fn tilted_plane_rounds_exactly_too() {
    // A sketch on a plane that is not axis-aligned.
    let n = Vec3::new(1.0, 2.0, 2.0).normalized().unwrap();
    let plane = Plane::from_point_normal(p3(0.0, 0.0, 0.0), n).unwrap();
    let (u, v) = common::curves::basis(n);
    let at = |x: f64, y: f64| Point3::ORIGIN + u * x + v * y;
    let mut s = Sketch::on_plane(plane);
    let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        s.add_segment(at(a.0, a.1), at(b.0, b.1)).unwrap();
    }
    let corner = vertex_at(&s, at(0.0, 0.0));
    let report = s.blend_corner(corner, round(0.2, 6)).expect("rounds");
    let geom: CurveGeom = s.curve_analytic(report.curve.unwrap()).unwrap().geom;
    assert!((geom.center - at(0.2, 0.2)).length() < 1e-9);
    assert_eq!(s.regions().len(), 1);
}

// --------------------------------------------------- documents and drafts

fn up() -> Vec3 {
    Vec3::new(0.0, 0.0, 1.0)
}

/// A unit box on the ground; its top face is at z = 1.
fn unit_box_doc() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in [
        (p3(0., 0., 0.), p3(1., 0., 0.)),
        (p3(1., 0., 0.), p3(1., 1., 0.)),
        (p3(1., 1., 0.), p3(0., 1., 0.)),
        (p3(0., 1., 0.), p3(0., 0., 0.)),
    ] {
        sk.add_segment(a, b).unwrap();
    }
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (o, _) = doc.extrude_region(s, r, 1.0).unwrap();
    (doc, o)
}

fn top() -> Plane {
    Plane::from_point_normal(p3(0., 0., 1.), up()).unwrap()
}

fn draw_rect_on_top(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64) {
    let z = 1.0;
    let sk = doc.draft_sketch_mut().expect("a draft is open");
    for (a, b) in [
        (p3(x0, y0, z), p3(x1, y0, z)),
        (p3(x1, y0, z), p3(x1, y1, z)),
        (p3(x1, y1, z), p3(x0, y1, z)),
        (p3(x0, y1, z), p3(x0, y0, z)),
    ] {
        sk.add_segment(a, b).unwrap();
    }
}

#[test]
fn imprint_corner_on_face() {
    let (mut doc, o) = unit_box_doc();
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let profile = round(0.1, 6);

    doc.begin_draft(top(), None, &[o], "Round corner").unwrap();
    draw_rect_on_top(&mut doc, 0.2, 0.2, 0.7, 0.7);
    let corner = p3(0.7, 0.7, 1.0);
    let v = doc
        .draft_check_corner(corner, &profile)
        .expect("inside the face");
    doc.draft_sketch_mut()
        .unwrap()
        .blend_corner(v, profile)
        .unwrap();
    doc.end_draft().expect("the draft lowers");
    assert_eq!(
        doc.undo_depth(),
        depth0 + 1,
        "the whole gesture is one undo step"
    );

    // The imprint face has the arc: 3 straight corners + 7 arc points.
    let obj = doc.object(o).unwrap();
    let imprint = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().approx_eq(up(), 1e-9)
                && f.plane.signed_distance(p3(0.0, 0.0, 1.0)).abs() < 1e-9
                && obj.loop_positions(f.outer_loop).count() == 10
        })
        .map(|(id, _)| id)
        .expect("a ten-vertex imprint loop with the arc in it");

    // Pushing it up gives a boss whose arc wall is Cylinder-claimed.
    doc.apply_object_op(
        o,
        KernelOp::PushPull {
            face: imprint,
            distance: 0.3,
        },
    )
    .expect("push the imprint");
    let obj = doc.object(o).unwrap();
    assert_valid_solid(obj);
    let census = claim_census(obj);
    assert_eq!(census.get("cylinder"), Some(&6), "{census:?}");

    // Undo twice (push/pull, draft) and the bytes are the original ones.
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.save(), before);
}

#[test]
fn imprint_corner_leaving_face_refused() {
    let (mut doc, o) = unit_box_doc();
    let before = doc.save();
    let profile = round(0.1, 6);

    doc.begin_draft(top(), None, &[o], "Round corner").unwrap();
    // A rectangle in the face's own corner: its corner is the solid's
    // boundary, which drafting never trims.
    draw_rect_on_top(&mut doc, 0.0, 0.0, 0.5, 0.5);
    let corner = p3(0.0, 0.0, 1.0);
    assert_eq!(
        doc.draft_check_corner(corner, &profile).unwrap_err(),
        SketchError::CornerLeavesFace
    );
    // The imprint's other corner is fine.
    doc.draft_check_corner(p3(0.5, 0.5, 1.0), &profile)
        .expect("the free corner is inside the face");
    assert!(doc.cancel_draft());
    assert_eq!(doc.save(), before, "nothing changed");
}

#[test]
fn imprint_corner_whose_arc_leaves_a_concave_face_is_refused() {
    // An L-shaped slab: its top face is concave. A wedge of drawing laid
    // across the notch has a corner whose rounding would bulge out of the
    // face.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let pts = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        sk.add_segment(p3(a.0, a.1, 0.0), p3(b.0, b.1, 0.0))
            .unwrap();
    }
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (o, _) = doc.extrude_region(s, r, 1.0).unwrap();

    // Two loose lines meeting at (0.95, 0.95) and opening toward the
    // notch's corner at (1, 1).
    doc.begin_draft(top(), None, &[o], "Round corner").unwrap();
    {
        let sk = doc.draft_sketch_mut().unwrap();
        sk.add_segment(p3(0.95, 0.95, 1.0), p3(1.9, 0.5, 1.0))
            .unwrap();
        sk.add_segment(p3(0.95, 0.95, 1.0), p3(0.5, 1.9, 1.0))
            .unwrap();
    }
    let corner = p3(0.95, 0.95, 1.0);
    doc.draft_check_corner(corner, &round(0.1, 6))
        .expect("a small round stays on the face");
    // At radius 2 the arc bulges past (1, 1), out of the L.
    assert_eq!(
        doc.draft_check_corner(corner, &round(2.0, 6)).unwrap_err(),
        SketchError::CornerLeavesFace
    );
    // The blend errors still come first.
    assert!(matches!(
        doc.draft_check_corner(corner, &round(20.0, 6)).unwrap_err(),
        SketchError::CornerRadiusTooLarge { .. }
    ));
    assert!(doc.cancel_draft());
}

#[test]
fn undo_redo() {
    // A world sketch: one undo step.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in [
        (p3(0., 0., 0.), p3(1., 0., 0.)),
        (p3(1., 0., 0.), p3(1., 1., 0.)),
        (p3(1., 1., 0.), p3(0., 1., 0.)),
        (p3(0., 1., 0.), p3(0., 0., 0.)),
    ] {
        sk.add_segment(a, b).unwrap();
    }
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let v = vertex_at(doc.sketch(s).unwrap(), p3(0.0, 0.0, 0.0));
    let (report, change) = doc
        .sketch_blend_corner(s, v, round(0.2, 6))
        .expect("the world sketch corner rounds");
    assert_eq!(change.sketches_touched, vec![s]);
    assert!(report.curve.is_some());
    assert_eq!(doc.undo_depth(), depth0 + 1, "one undo step");
    let after = doc.save();
    doc.undo().unwrap();
    assert_eq!(doc.save(), before, "undo restores the square");
    assert_eq!(doc.undo_depth(), depth0);
    doc.redo().unwrap();
    assert_eq!(doc.save(), after, "redo restores the rounded corner");

    // A refused blend records nothing and leaves no gesture open.
    let v2 = vertex_at(doc.sketch(s).unwrap(), p3(1.0, 1.0, 0.0));
    let depth = doc.undo_depth();
    assert!(doc.sketch_blend_corner(s, v2, round(5.0, 6)).is_err());
    assert_eq!(doc.undo_depth(), depth);
    doc.sketch_blend_corner(s, v2, round(0.1, 6))
        .expect("the next blend still works");

    // An imprint: the draft is one undo step, undo and redo byte-exact.
    let (mut doc, o) = unit_box_doc();
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let profile = round(0.1, 6);
    doc.begin_draft(top(), None, &[o], "Round corner").unwrap();
    draw_rect_on_top(&mut doc, 0.2, 0.2, 0.7, 0.7);
    let v = doc.draft_check_corner(p3(0.2, 0.2, 1.0), &profile).unwrap();
    doc.draft_sketch_mut()
        .unwrap()
        .blend_corner(v, profile)
        .unwrap();
    doc.end_draft().unwrap();
    assert_eq!(doc.undo_depth(), depth0 + 1);
    let after = doc.save();
    doc.undo().unwrap();
    assert_eq!(doc.save(), before);
    doc.redo().unwrap();
    assert_eq!(doc.save(), after);
}
