//! Executable specs for the Offset tool's kernel operations
//! (DEVELOPMENT.md rule 3): [`kernel::offset_profile`],
//! [`kernel::Sketch::offset_region`], and [`kernel::offset_face_boundary`].
//!
//! The contract under test: every boundary loop offsets by a uniform
//! distance (positive grows the material, negative shrinks it); straight
//! edges offset to parallel lines with miter joins; analytic curve facets
//! offset to concentric true circles keeping the exact center; a distance
//! the shape cannot absorb is a typed refusal with the sketch/object
//! untouched — never a repaired or clamped result.

use kernel::{
    CurveGeom, Document, History, KernelOp, Object, Plane, Point3, Profile, Sketch, SketchError,
    offset_face_boundary, offset_profile, tol,
};
use proptest::prelude::*;

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn pt(x: f64, y: f64) -> Point3 {
    Point3::new(x, y, 0.0)
}

/// A ground sketch holding one axis-aligned rectangle.
fn rect_sketch(x0: f64, y0: f64, x1: f64, y1: f64) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        (pt(x0, y0), pt(x1, y0)),
        (pt(x1, y0), pt(x1, y1)),
        (pt(x1, y1), pt(x0, y1)),
        (pt(x0, y1), pt(x0, y0)),
    ] {
        s.add_segment(a, b).expect("rectangle segment");
    }
    assert_eq!(s.regions().len(), 1);
    s
}

/// A faceted circle committed as one analytic curve chain on a fresh ground
/// sketch; returns the sketch (with exactly one region).
fn circle_sketch(center: Point3, radius: f64, n: usize) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom { center, radius })
        .expect("curve opens");
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
            0.0,
        )
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).expect("circle facet");
    }
    s.end_curve();
    assert_eq!(s.regions().len(), 1, "circle closes one region");
    s
}

/// The sketch's sole region.
fn only_region(s: &Sketch) -> kernel::SketchRegionId {
    assert_eq!(s.regions().len(), 1, "expected exactly one region");
    s.regions().keys().next().unwrap()
}

/// True if some region of `s` has an outer boundary whose vertices are
/// exactly the axis-aligned rectangle `[x0,x1] × [y0,y1]`.
fn has_rect_region(s: &Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    s.regions().values().any(|r| {
        r.outer.len() == 4
            && [pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)]
                .iter()
                .all(|&c| {
                    r.outer
                        .iter()
                        .any(|&vid| s.vertices()[vid].position.approx_eq(c, tol::POINT_MERGE))
                })
    })
}

// ----------------------------------------------------- sketch region offset

/// Offsetting a rectangle inward inserts the smaller concentric rectangle:
/// the sketch now reads as a ring region (the original outer with the new
/// loop as its hole) plus the inner region, and both are extrudable.
#[test]
fn rectangle_inward_offset_nests_a_smaller_rectangle() {
    let mut s = rect_sketch(0.0, 0.0, 4.0, 3.0);
    let region = only_region(&s);
    let report = s.offset_region(region, -0.5).expect("offset succeeds");

    assert_eq!(report.new_edges.len(), 4, "four straight offset edges");
    assert!(report.new_curves.is_empty(), "no analytic runs on a rect");
    assert!(has_rect_region(&s, 0.5, 0.5, 3.5, 2.5), "inset rectangle");
    assert_eq!(s.regions().len(), 2, "ring + inner region");
    let ring = s
        .regions()
        .values()
        .find(|r| !r.holes.is_empty())
        .expect("the outer region carries the offset loop as a hole");
    assert_eq!(ring.holes.len(), 1);
    for rid in s.regions().keys().collect::<Vec<_>>() {
        assert!(s.profile(rid).is_ok(), "every region stays extrudable");
    }
}

/// Offsetting outward encloses the original: the new loop is the grown
/// rectangle and the original boundary becomes the hole of the new ring.
#[test]
fn rectangle_outward_offset_encloses_the_original() {
    let mut s = rect_sketch(0.0, 0.0, 2.0, 2.0);
    let region = only_region(&s);
    let report = s.offset_region(region, 0.75).expect("offset succeeds");

    assert_eq!(report.new_edges.len(), 4);
    assert!(has_rect_region(&s, -0.75, -0.75, 2.75, 2.75));
    assert_eq!(s.regions().len(), 2);
}

/// A region with a hole offsets BOTH loops: shrinking the material contracts
/// the outer boundary and grows the hole, keeping a uniform band.
#[test]
fn region_with_hole_offsets_both_loops() {
    let mut s = rect_sketch(0.0, 0.0, 6.0, 6.0);
    for (a, b) in [
        (pt(2.5, 2.5), pt(3.5, 2.5)),
        (pt(3.5, 2.5), pt(3.5, 3.5)),
        (pt(3.5, 3.5), pt(2.5, 3.5)),
        (pt(2.5, 3.5), pt(2.5, 2.5)),
    ] {
        s.add_segment(a, b).expect("hole segment");
    }
    assert_eq!(s.regions().len(), 2, "ring + inner island");
    let ring = s
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("holed region");

    let report = s.offset_region(ring, -0.25).expect("offset succeeds");
    assert_eq!(report.new_edges.len(), 8, "outer image + hole image");
    assert!(has_rect_region(&s, 0.25, 0.25, 5.75, 5.75), "outer shrank");
    assert!(has_rect_region(&s, 2.25, 2.25, 3.75, 3.75), "hole grew");
    for rid in s.regions().keys().collect::<Vec<_>>() {
        assert!(s.profile(rid).is_ok(), "every region stays extrudable");
    }
}

/// A drawn circle offsets analytically: the new loop is a true curve chain
/// keeping the exact center, with the radius offset by exactly the distance.
#[test]
fn circle_offset_keeps_exact_center_and_offsets_radius() {
    let center = pt(1.5, -2.0);
    let mut s = circle_sketch(center, 1.0, 24);
    let region = only_region(&s);
    let report = s.offset_region(region, 0.25).expect("offset succeeds");

    assert_eq!(report.new_curves.len(), 1, "one analytic run: the circle");
    let geom = s
        .curve_geom(report.new_curves[0])
        .expect("offset curve carries geometry");
    assert_eq!(geom.center, center, "center survives exactly");
    assert_eq!(geom.radius, 1.25, "radius offsets by exactly d");
    assert_eq!(report.new_edges.len(), 24, "facet count is preserved");
    assert_eq!(s.regions().len(), 2, "ring + inner disk");

    // The offset chain is one selectable curve, like a freshly drawn circle.
    let chain = s.curve_chain_at(report.new_edges[0]);
    assert_eq!(chain.len(), 24, "the offset circle selects as one curve");
}

/// A pie region mixing an analytic arc and straight edges: the arc offsets
/// concentrically and the straight edges to parallel lines; the junction
/// vertices land exactly on both offset primitives.
#[test]
fn pie_region_offsets_arc_and_lines_together() {
    let center = pt(0.0, 0.0);
    let radius = 2.0;
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom { center, radius })
        .expect("curve opens");
    let n = 12; // quarter arc, 12 facets
    let arc = |i: usize| {
        let a = std::f64::consts::FRAC_PI_2 * (i as f64) / (n as f64);
        pt(radius * a.cos(), radius * a.sin())
    };
    for i in 0..n {
        s.add_segment(arc(i), arc(i + 1)).expect("arc facet");
    }
    s.end_curve();
    s.add_segment(arc(n), center).expect("upper spoke");
    s.add_segment(center, arc(0)).expect("lower spoke");
    let region = only_region(&s);

    let report = s.offset_region(region, -0.5).expect("offset succeeds");
    assert_eq!(report.new_curves.len(), 1, "one arc run");
    let geom = s
        .curve_geom(report.new_curves[0])
        .expect("offset arc carries geometry");
    assert_eq!(geom.center, center);
    assert_eq!(geom.radius, 1.5, "arc shrinks toward its center");

    // Every offset arc vertex sits exactly on the offset circle, including
    // the two arc-line junctions.
    for &eid in &report.new_edges {
        if s.edge_curve(eid).is_some() {
            let e = s.edges()[eid];
            for vid in [e.from, e.to] {
                let d = (s.vertices()[vid].position - center).length();
                assert!(
                    (d - 1.5).abs() <= tol::PLANE_DIST,
                    "arc vertex off the offset circle: {d}"
                );
            }
        }
    }
    assert_eq!(s.regions().len(), 2);
}

/// A needle dart's miter join runs away as `d / sin(θ/2)`: before the miter
/// limit ([`tol::OFFSET_MITER_LIMIT`]) this emitted a "valid" loop with a
/// spike vertex thousands of units off a 100-unit profile — simple and
/// correctly wound, so no downstream check caught it. Sharpness across
/// three decades all refuses typed, and through the sketch path the
/// document is untouched.
#[test]
fn needle_dart_miter_refuses_typed_across_sharpness_decades() {
    for hw in [1e-3, 1e-4, 1e-6] {
        // Pure computation path.
        let outer = vec![pt(0.0, 0.0), pt(100.0, -hw), pt(100.0, hw)];
        let profile =
            Profile::new(ground(), outer, Vec::new()).expect("a needle dart is a valid profile");
        assert_eq!(
            offset_profile(&profile, 0.1).unwrap_err(),
            kernel::OffsetError::OffsetCollapsed,
            "hw={hw}: the dart tip's miter exceeds the limit"
        );

        // Sketch path: typed refusal, sketch untouched.
        let mut s = Sketch::on_plane(ground());
        for (a, b) in [
            (pt(0.0, 0.0), pt(100.0, -hw)),
            (pt(100.0, -hw), pt(100.0, hw)),
            (pt(100.0, hw), pt(0.0, 0.0)),
        ] {
            s.add_segment(a, b).expect("dart segment");
        }
        let region = only_region(&s);
        let before = s.clone();
        assert_eq!(
            s.offset_region(region, 0.1).unwrap_err(),
            SketchError::OffsetCollapsed,
            "hw={hw}: refused through the sketch path too"
        );
        assert_eq!(s, before, "hw={hw}: untouched on refusal");
    }
}

/// Ordinary sharp corners stay well inside the miter limit: 30° and 15°
/// wedges offset outward fine, the apex landing at its exact
/// `d / sin(θ/2)` miter displacement.
#[test]
fn sharp_but_sane_corners_still_offset() {
    for apex_deg in [30.0_f64, 15.0] {
        let a = apex_deg.to_radians();
        let outer = vec![
            pt(0.0, 0.0),
            pt(10.0, 0.0),
            pt(10.0 * a.cos(), 10.0 * a.sin()),
        ];
        let profile = Profile::new(ground(), outer, Vec::new()).expect("wedge profile");
        let off = offset_profile(&profile, 0.1).expect("a sane sharp corner offsets");

        // The apex vertex (index 0) sits at exactly d / sin(θ/2) from its
        // source — the miter is exact, merely bounded.
        let got = (off.outer.points[0] - pt(0.0, 0.0)).length();
        let expect = 0.1 / (a / 2.0).sin();
        assert!(
            (got - expect).abs() <= 1e-9,
            "apex {apex_deg}°: miter displacement {got} != {expect}"
        );
    }
}

/// Typed refusals, each leaving the sketch untouched (strong guarantee):
/// an inward offset past the inradius, an offset past a curve's radius, a
/// degenerate distance, and a stale region handle.
#[test]
fn collapsing_offsets_refuse_and_leave_the_sketch_untouched() {
    // Inward past the inradius: miter joins invert the loop.
    let mut s = rect_sketch(0.0, 0.0, 2.0, 2.0);
    let region = only_region(&s);
    let before = s.clone();
    assert_eq!(
        s.offset_region(region, -1.5).unwrap_err(),
        SketchError::OffsetCollapsed
    );
    assert_eq!(s, before, "untouched on refusal");

    // Exactly to the inradius: the loop degenerates to a point.
    assert_eq!(
        s.offset_region(region, -1.0).unwrap_err(),
        SketchError::OffsetCollapsed
    );
    assert_eq!(s, before);

    // A circle pushed past (and exactly to) zero radius.
    let mut c = circle_sketch(pt(0.0, 0.0), 1.0, 24);
    let cr = only_region(&c);
    let cbefore = c.clone();
    for d in [-1.0, -2.5] {
        assert_eq!(
            c.offset_region(cr, d).unwrap_err(),
            SketchError::OffsetCollapsed
        );
        assert_eq!(c, cbefore);
    }

    // Degenerate distances.
    for d in [0.0, tol::POINT_MERGE / 2.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            s.offset_region(region, d).unwrap_err(),
            SketchError::OffsetTooSmall
        );
    }
    assert_eq!(s, before);

    // A stale region handle.
    assert_eq!(
        s.offset_region(kernel::SketchRegionId::default(), -0.1)
            .unwrap_err(),
        SketchError::UnknownRegion
    );
}

/// An outward offset of a hole-carrying region past the hole's own extent
/// collapses the hole and refuses.
#[test]
fn outward_offset_that_swallows_a_hole_refuses() {
    let mut s = rect_sketch(0.0, 0.0, 6.0, 6.0);
    for (a, b) in [
        (pt(2.5, 2.5), pt(3.5, 2.5)),
        (pt(3.5, 2.5), pt(3.5, 3.5)),
        (pt(3.5, 3.5), pt(2.5, 3.5)),
        (pt(2.5, 3.5), pt(2.5, 2.5)),
    ] {
        s.add_segment(a, b).expect("hole segment");
    }
    let ring = s
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("holed region");
    let before = s.clone();
    assert_eq!(
        s.offset_region(ring, 0.75).unwrap_err(),
        SketchError::OffsetCollapsed,
        "growing the material past the hole's half-extent collapses it"
    );
    assert_eq!(s, before);
}

/// A hand-built curve facet subtending exactly half a turn puts its chord
/// midpoint ON the claimed circle's center: which side the material is on
/// cannot be read from the facet, and the answer must never be a
/// sub-tolerance dot product's noise (rule 6). Every distance refuses
/// typed, sketch untouched. (Tool-drawn curves never hit this — the density
/// floor keeps facet spans at a few degrees.)
#[test]
fn half_turn_facet_material_side_is_refused_not_guessed() {
    let center = pt(5.0, 5.0);
    let radius = 3.0;
    let p = |deg: f64| {
        let a = deg.to_radians();
        pt(center.x + radius * a.cos(), center.y + radius * a.sin())
    };
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom { center, radius })
        .expect("curve opens");
    s.add_segment(p(80.0), p(260.0)).expect("half-turn facet");
    s.add_segment(p(260.0), p(280.0)).expect("small facet");
    s.end_curve();
    s.add_segment(p(280.0), p(80.0)).expect("closing chord");
    let region = only_region(&s);

    let before = s.clone();
    for d in [0.2, -0.2, 1.0, -1.0] {
        assert_eq!(
            s.offset_region(region, d).unwrap_err(),
            SketchError::OffsetCollapsed,
            "d={d}: an undecidable facet claim must refuse, not guess"
        );
        assert_eq!(s, before, "d={d}: untouched on refusal");
    }
}

// -------------------------------------------------------------- undo/redo

/// A gesture-bracketed offset is one undo step, and undo restores the sketch
/// exactly — same handles, same geometry (DEVELOPMENT.md rule 9).
#[test]
fn offset_gesture_undo_restores_the_sketch_exactly() {
    let mut doc = Document::new();
    let sid = doc.add_sketch(ground());

    // Draw the rectangle inside its own gesture, as the tools do.
    doc.begin_sketch_gesture(sid).expect("gesture opens");
    {
        let s = doc.sketch_mut(sid).expect("sketch is live");
        for (a, b) in [
            (pt(0.0, 0.0), pt(4.0, 0.0)),
            (pt(4.0, 0.0), pt(4.0, 3.0)),
            (pt(4.0, 3.0), pt(0.0, 3.0)),
            (pt(0.0, 3.0), pt(0.0, 0.0)),
        ] {
            s.add_segment(a, b).expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(sid).expect("gesture closes");

    let before = doc.sketch(sid).expect("live").clone();
    let region = only_region(doc.sketch(sid).expect("live"));

    doc.begin_sketch_gesture(sid).expect("gesture opens");
    doc.sketch_mut(sid)
        .expect("sketch is live")
        .offset_region(region, -0.5)
        .expect("offset succeeds");
    doc.end_sketch_gesture(sid).expect("gesture closes");
    assert_eq!(doc.sketch(sid).expect("live").regions().len(), 2);

    doc.undo().expect("undo succeeds");
    assert_eq!(
        doc.sketch(sid).expect("live"),
        &before,
        "undo restores the pre-offset sketch exactly (handles included)"
    );

    doc.redo().expect("redo succeeds");
    assert_eq!(doc.sketch(sid).expect("live").regions().len(), 2);
}

// -------------------------------------------------------- solid face offset

/// A box top's boundary offsets inward to the exact inset rectangle, with no
/// analytic attribution; imprinting it through the History (as the tool
/// does) round-trips under undo.
#[test]
fn box_top_offset_imprints_and_undoes_exactly() {
    let s = rect_sketch(0.0, 0.0, 2.0, 2.0);
    let region = only_region(&s);
    let profile = s.profile(region).expect("profile");
    let mut obj = Object::from_extrusion(&profile, 1.0).expect("extrude");
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .expect("top face");

    let lp = offset_face_boundary(&obj, top, -0.25).expect("offset succeeds");
    assert_eq!(lp.points.len(), 4);
    assert!(lp.curves.iter().all(Option::is_none));
    for c in [
        pt(0.25, 0.25),
        pt(1.75, 0.25),
        pt(1.75, 1.75),
        pt(0.25, 1.75),
    ] {
        let expect = Point3::new(c.x, c.y, 1.0);
        assert!(
            lp.points
                .iter()
                .any(|p| p.approx_eq(expect, tol::POINT_MERGE)),
            "missing inset corner {expect:?}"
        );
    }

    let faces_before = obj.faces().len();
    let mut history = History::new();
    history
        .apply(
            &mut obj,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path: lp.points.clone(),
                restore: None,
                curve: None,
            },
        )
        .expect("imprint succeeds");
    assert_eq!(obj.faces().len(), faces_before + 1, "sub-face appeared");

    history.undo(&mut obj).expect("undo succeeds");
    assert_eq!(obj.faces().len(), faces_before, "undo removes the imprint");
    assert!(obj.validate().is_ok());
}

/// A drawn cylinder's cap boundary recovers its circle from the stamped
/// walls: the offset loop is a true concentric circle at the cap's height.
#[test]
fn cylinder_cap_offset_recovers_the_analytic_circle() {
    let s = circle_sketch(pt(1.0, 2.0), 1.0, 24);
    let region = only_region(&s);
    let profile = s.profile(region).expect("profile");
    let mut obj = Object::from_extrusion(&profile, 3.0).expect("extrude");
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .expect("top cap");

    let lp = offset_face_boundary(&obj, top, -0.25).expect("offset succeeds");
    assert_eq!(lp.points.len(), 24);
    let expected = CurveGeom {
        center: Point3::new(1.0, 2.0, 3.0),
        radius: 0.75,
    };
    assert!(
        lp.curves.iter().all(|c| *c == Some(expected)),
        "every offset facet claims the concentric cap circle"
    );
    for p in &lp.points {
        assert!(
            ((*p - expected.center).length() - 0.75).abs() <= tol::PLANE_DIST,
            "offset vertex off the circle"
        );
    }

    // The loop commits as an analytic imprint (the boss/recess workflow).
    obj.split_face_inner_with_curve(top, &lp.points, Some(expected))
        .expect("imprint succeeds");
    assert!(obj.validate().is_ok());
}

/// Face-side refusals: a stale face handle, an offset past the cap's
/// radius, and an outward loop refused by the imprint — object untouched.
#[test]
fn face_offset_refusals_are_typed() {
    let s = circle_sketch(pt(0.0, 0.0), 1.0, 24);
    let region = only_region(&s);
    let profile = s.profile(region).expect("profile");
    let mut obj = Object::from_extrusion(&profile, 1.0).expect("extrude");
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .expect("top cap");

    assert_eq!(
        offset_face_boundary(&obj, kernel::FaceId::default(), -0.1).unwrap_err(),
        kernel::FaceOffsetError::UnknownFace
    );
    assert_eq!(
        offset_face_boundary(&obj, top, -1.0).unwrap_err(),
        kernel::FaceOffsetError::Offset(kernel::OffsetError::OffsetCollapsed)
    );
    assert_eq!(
        offset_face_boundary(&obj, top, 0.0).unwrap_err(),
        kernel::FaceOffsetError::Offset(kernel::OffsetError::OffsetTooSmall)
    );

    // An outward loop computes fine but lies outside the face; the imprint
    // is the gate that refuses it (never committing invalid geometry).
    let out = offset_face_boundary(&obj, top, 0.5).expect("outward loop computes");
    let before = obj.clone();
    assert!(
        obj.split_face_inner_with_curve(top, &out.points, out.curves[0])
            .is_err(),
        "an outward loop cannot imprint"
    );
    assert!(kernel_objects_equal(&obj, &before), "untouched on refusal");
}

/// Bitwise structural equality via the canonical polygon soup — enough for
/// "untouched on refusal" (no mutation may have happened at all).
fn kernel_objects_equal(a: &Object, b: &Object) -> bool {
    let (pa, fa) = a.to_polygons();
    let (pb, fb) = b.to_polygons();
    pa.len() == pb.len()
        && fa == fb
        && pa
            .iter()
            .zip(pb.iter())
            .all(|(x, y)| x.approx_eq(*y, tol::POINT_MERGE))
}

// ------------------------------------------------------------- properties

/// A regular convex polygon: `n` vertices on radius `r` about `(cx, cy)`,
/// with a phase rotation. Always a valid single-region profile.
fn regular_polygon(n: usize, r: f64, cx: f64, cy: f64, phase: f64) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = phase + 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            pt(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

proptest! {
    /// Offsetting a convex polygon outward by `d` and the result back by
    /// `-d` restores every vertex within tolerance (miter joins are exact
    /// line intersections, so the round trip is the identity up to
    /// floating point).
    #[test]
    fn offset_round_trips_on_convex_polygons(
        n in 3usize..12,
        r in 0.5f64..50.0,
        cx in -100.0f64..100.0,
        cy in -100.0f64..100.0,
        phase in 0.0f64..std::f64::consts::TAU,
        d in 1e-3f64..10.0,
    ) {
        let outer = regular_polygon(n, r, cx, cy, phase);
        let profile = Profile::new(ground(), outer.clone(), Vec::new()).expect("valid polygon");
        let grown = offset_profile(&profile, d).expect("outward offset of a convex polygon");
        let grown_profile = Profile::new(ground(), grown.outer.points.clone(), Vec::new())
            .expect("offset loop is a valid profile");
        let back = offset_profile(&grown_profile, -d).expect("inverse offset");

        // Index-aligned: OffsetLoop preserves vertex order 1:1.
        for (p, q) in outer.iter().zip(back.outer.points.iter()) {
            prop_assert!(
                p.approx_eq(*q, 1e-9),
                "round trip drifted: {p:?} vs {q:?}"
            );
        }
    }

    /// Offsetting a drawn circle keeps the exact center and moves the radius
    /// by exactly the distance, and the sketch gains exactly the ring/disk
    /// pair of regions.
    #[test]
    fn circle_offsets_are_exactly_concentric(
        cx in -50.0f64..50.0,
        cy in -50.0f64..50.0,
        r in 0.1f64..25.0,
        n in 24usize..64,
        frac in -0.9f64..4.0,
    ) {
        // Keep |d| meaningfully non-degenerate.
        let d = r * if frac.abs() < 1e-3 { 0.5 } else { frac };
        let center = pt(cx, cy);
        let mut s = circle_sketch(center, r, n);
        let region = only_region(&s);
        let report = s.offset_region(region, d).expect("offset succeeds");

        prop_assert_eq!(report.new_curves.len(), 1);
        let geom = s.curve_geom(report.new_curves[0]).expect("geometry");
        prop_assert_eq!(geom.center, center, "center is exact");
        prop_assert_eq!(geom.radius, r + d, "radius moves by exactly d");
        prop_assert_eq!(s.regions().len(), 2, "ring + disk");
        for rid in s.regions().keys().collect::<Vec<_>>() {
            prop_assert!(s.profile(rid).is_ok(), "regions stay extrudable");
        }
    }

    /// An inward rectangle offset closes exactly one new region, and every
    /// resulting region remains a valid, extrudable profile.
    #[test]
    fn inward_rectangle_offsets_close_regions(
        w in 0.5f64..20.0,
        h in 0.5f64..20.0,
        frac in 0.05f64..0.45,
    ) {
        let d = -(w.min(h) * frac);
        let mut s = rect_sketch(0.0, 0.0, w, h);
        let region = only_region(&s);
        let report = s.offset_region(region, d).expect("offset succeeds");

        prop_assert_eq!(report.regions_created.len(), 1, "the inner region");
        prop_assert!(report.regions_removed.is_empty(), "outer keeps its id");
        prop_assert_eq!(s.regions().len(), 2);
        for rid in s.regions().keys().collect::<Vec<_>>() {
            prop_assert!(s.profile(rid).is_ok());
        }
    }
}
