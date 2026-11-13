//! Executable specs for arcs drawn on a solid's face keeping their circle:
//! an arc cut edge to edge ([`Object::split_face_with_curves`]) or closed
//! into a pie or segment ([`Object::split_face_inner_with_curves`]) stamps
//! its facets' circle onto the solid's edges, so pushing either side raises
//! smooth arc walls and flat closing walls — exactly what the same shape
//! does from a ground sketch — and the arc survives moves, dissolves, undo,
//! and push-through.

use kernel::{
    CurveGeom, Document, FaceId, KernelOp, Object, Plane, Point3, StickyError, SurfaceRef,
    Transform, Vec3, tol,
};

mod face_arrangement_support;
use face_arrangement_support::{delete_unit_at, draw_loop_curves, move_unit_at, units_on};

// ---------------------------------------------------------------- helpers

/// A unit cube from polygon soup; its top is z = 1, x and y in [0, 1].
fn unit_cube() -> Object {
    let v = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    Object::from_polygons(
        &[
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(1.0, 1.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(0.0, 0.0, 1.0),
            v(1.0, 0.0, 1.0),
            v(1.0, 1.0, 1.0),
            v(0.0, 1.0, 1.0),
        ],
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .unwrap()
}

fn top(obj: &Object) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
        })
        .map(|(id, _)| id)
        .expect("a +z face")
}

/// `n` chord facets of the circle about `(cx, cy, 1)` of radius `r`, from
/// angle `a0` to `a1` (radians), as `n + 1` points on the top plane.
fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64, n: usize) -> Vec<Point3> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), 1.0)
        })
        .collect()
}

fn claims(obj: &Object) -> Vec<CurveGeom> {
    obj.edges().values().filter_map(|e| e.curve).collect()
}

fn claims_centered(obj: &Object, c: Point3) -> usize {
    claims(obj)
        .iter()
        .filter(|g| g.center.approx_eq(c, tol::POINT_MERGE))
        .count()
}

fn stamped_walls(obj: &Object) -> usize {
    obj.faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .count()
}

/// The face of `obj` whose outer loop has exactly `edges` edges.
fn face_with_edges(obj: &Object, edges: usize) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| obj.loop_half_edges(f.outer_loop).count() == edges)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("a face with {edges} edges"))
}

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// A unit cube built through the document (so every op records undo).
fn doc_cube() -> (Document, kernel::ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let pts = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ];
    {
        let sk = doc.sketch_mut(s).unwrap();
        for i in 0..4 {
            sk.add_segment(pts[i], pts[(i + 1) % 4]).unwrap();
        }
    }
    let regions = doc.extrudable_regions(s).unwrap();
    let (id, _) = doc.extrude_region(s, regions[0], 1.0).unwrap();
    (doc, id)
}

// ------------------------------------------------------- edge-to-edge arc

/// An arc cut from one point of the south edge to another (a D on the top)
/// carries its circle on every facet, and pushing the D up raises 18 smooth
/// cylinder walls whose raised rim carries the circle moved with it — the
/// general wall-building push path, since a D is no flat sub-face.
#[test]
fn an_arc_cut_edge_to_edge_keeps_its_circle_and_pushes_smooth() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let center = Point3::new(0.5, 0.0, 1.0);
    let g = CurveGeom {
        center,
        radius: 0.3,
    };
    let path = arc(0.5, 0.0, 0.3, 0.0, std::f64::consts::PI, 18);
    let report = obj
        .split_face_with_curves(t, &path, &vec![Some(g); 18])
        .expect("an arc cut edge to edge");
    obj.validate().unwrap();
    assert_eq!(report.new_edges.len(), 18);
    assert_eq!(
        claims_centered(&obj, center),
        18,
        "every facet claims the circle"
    );
    let d = face_with_edges(&obj, 19);
    assert!(
        report.new_faces.contains(&d),
        "the D is one of the two faces the cut made"
    );
    assert!(
        !obj.edge_curve_rims().is_empty(),
        "the flat arc's facets already offer a rim for snapping"
    );

    obj.push_pull(d, 0.3).expect("push the D up");
    obj.validate().unwrap();
    assert_eq!(
        stamped_walls(&obj),
        18,
        "every arc wall is a cylinder facet"
    );
    assert_eq!(
        claims_centered(&obj, Point3::new(0.5, 0.0, 1.3)),
        18,
        "the raised rim carries the circle up with it"
    );
    assert_eq!(
        claims_centered(&obj, center),
        18,
        "the base rim keeps its claim"
    );
    // The stamped walls now carry the rim (edge-claim rims de-duplicate
    // against wall rims), so the raised arc still offers its snaps.
    assert!(
        obj.analytic_rims().iter().any(|rim| rim.has_coverage()),
        "the raised arc offers center and quadrant snaps"
    );
}

/// A pie or segment is only partly a curve: the arc's facets claim the
/// circle, the closing lines do not. Bossing it raises smooth arc walls and
/// flat closing walls; only the arc's facets carry onto the raised rim.
#[test]
fn a_segment_bosses_smooth_arc_walls_and_a_flat_chord() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let center = Point3::new(0.5, 0.5, 1.0);
    let g = CurveGeom {
        center,
        radius: 0.3,
    };
    // 12 facets over 120°, closed by one chord (the implicit last→first edge).
    let pts = arc(0.5, 0.5, 0.3, 0.0, 2.0 * std::f64::consts::PI / 3.0, 12);
    let mut curves = vec![Some(g); 12];
    curves.push(None);
    let disk = obj
        .split_face_inner_with_curves(t, &pts, &curves)
        .expect("a segment imprint")
        .sub_face;
    obj.validate().unwrap();
    assert_eq!(claims_centered(&obj, center), 12, "the arc's facets claim");
    let loop_claims: Vec<Option<CurveGeom>> = obj
        .loop_half_edges(obj.faces()[disk].outer_loop)
        .map(|h| obj.edges()[obj.half_edges()[h].edge].curve)
        .collect();
    assert_eq!(loop_claims.len(), 13);
    assert_eq!(
        loop_claims.iter().filter(|c| c.is_some()).count(),
        12,
        "the arc's 12 facets claim the circle, the chord does not"
    );

    obj.extrude_sub_face(disk, 0.2).expect("boss the segment");
    obj.validate().unwrap();
    assert_eq!(
        stamped_walls(&obj),
        12,
        "arc walls smooth, the chord wall flat"
    );
    assert_eq!(
        claims_centered(&obj, Point3::new(0.5, 0.5, 1.2)),
        12,
        "the raised rim carries the arc, not the chord"
    );
}

/// The loop's winding is normalised before imprinting: a segment handed in
/// clockwise still lands its claims on the arc's facets, never on the chord
/// (whose endpoints also lie on the circle, so a misplaced claim would pass
/// validation silently).
#[test]
fn a_clockwise_segment_lands_its_claims_on_the_arc_facets() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.3,
    };
    let mut pts = arc(0.5, 0.5, 0.3, 0.0, 2.0 * std::f64::consts::PI / 3.0, 12);
    let mut curves = vec![Some(g); 12];
    curves.push(None);
    // Clockwise: reverse the points; edge k of the reversed loop is edge
    // 11-k of the original for the arc facets, and the chord stays last.
    pts.reverse();
    curves = (0..13).map(|i| curves[(2 * 13 - 2 - i) % 13]).collect();
    obj.split_face_inner_with_curves(t, &pts, &curves)
        .expect("a clockwise segment imprint");
    obj.validate().unwrap();
    // The chord is the one long edge (2·r·sin 60° ≈ 0.52); every claimed
    // edge is a short facet (2·r·sin 5° ≈ 0.052).
    for (id, e) in obj.edges().iter() {
        let (a, b) = obj.edge_endpoints(id).unwrap();
        let len = (b - a).length();
        if e.curve.is_some() {
            assert!(
                len < 0.1,
                "a claimed edge is a facet, not the chord ({len})"
            );
        }
    }
    assert_eq!(claims(&obj).len(), 12);
}

/// A claim must describe the edge it is stamped on: the wrong count, or an
/// edge whose endpoints are off the circle, is refused typed with the object
/// untouched.
#[test]
fn a_claim_that_does_not_describe_its_edge_is_refused() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.0, 1.0),
        radius: 0.3,
    };
    let path = arc(0.5, 0.0, 0.3, 0.0, std::f64::consts::PI, 18);
    let before = obj.clone();
    assert_eq!(
        obj.split_face_with_curves(t, &path, &vec![Some(g); 17])
            .unwrap_err(),
        StickyError::CurveClaimOffLoop,
        "one claim per edge, or none"
    );
    let wrong = CurveGeom {
        center: Point3::new(0.5, 0.0, 1.0),
        radius: 0.4,
    };
    assert_eq!(
        obj.split_face_with_curves(t, &path, &vec![Some(wrong); 18])
            .unwrap_err(),
        StickyError::CurveClaimOffLoop,
        "the points do not lie on a radius-0.4 circle"
    );
    assert_eq!(obj.faces().len(), before.faces().len(), "untouched");
    assert!(claims(&obj).is_empty());
}

/// A claim with a non-finite center can never agree with any edge — and
/// every distance comparison against NaN is `false`, so without an explicit
/// finiteness check such a claim would slip PAST the "must lie on the
/// circle" gate and be stamped onto the edges. Refused typed, untouched.
#[test]
fn a_claim_with_a_non_finite_center_is_refused() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let path = arc(0.5, 0.0, 0.3, 0.0, std::f64::consts::PI, 18);
    let before = obj.clone();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let g = CurveGeom {
            center: Point3::new(bad, 0.0, 1.0),
            radius: 0.3,
        };
        assert_eq!(
            obj.split_face_with_curves(t, &path, &vec![Some(g); 18])
                .unwrap_err(),
            StickyError::CurveClaimOffLoop,
            "a {bad} center describes no circle"
        );
    }
    assert_eq!(obj.faces().len(), before.faces().len(), "untouched");
    assert!(claims(&obj).is_empty());
}

// -------------------------------------------------------- document level

/// A segment drawn with its chord along the face's edge routes to a chord
/// cut whose claims are the arc's; sliding it along the edge moves the
/// circle with it, and undo brings the original circle back exactly.
#[test]
fn a_segment_drawn_up_to_the_edge_is_an_arc_chord_that_moves_and_undoes() {
    let (mut doc, id) = doc_cube();
    let t = top(doc.object(id).unwrap());
    let center = Point3::new(0.5, 0.0, 1.0);
    let g = CurveGeom {
        center,
        radius: 0.3,
    };
    // The loop: arc from (0.8, 0) over the top to (0.2, 0), closed by the
    // implicit edge (0.2, 0) → (0.8, 0), which lies ON the south edge.
    let pts = arc(0.5, 0.0, 0.3, 0.0, std::f64::consts::PI, 18);
    let mut curves = vec![Some(g); 18];
    curves.push(None);
    let top_pt = pts[9];
    let plane = face_arrangement_support::face_plane(&doc, id, t);
    draw_loop_curves(&mut doc, None, id, t, &pts, &curves).expect("a segment up to the edge");
    // The drawing unit is the arc's 18 facets (the chord lies on the edge,
    // which is the solid's own); its one curve run claims the circle.
    let run = |doc: &Document| -> kernel::DraftCurveRun {
        let units = units_on(doc, None, id, plane);
        assert_eq!(units.units.len(), 1, "one drawing unit");
        assert_eq!(units.units[0].curves.len(), 1, "one curve run");
        units.units[0].curves[0].clone()
    };
    let first = run(&doc);
    assert_eq!(first.edges.len(), 18, "the chord is the arc's 18 facets");
    assert!(first.geom.center.approx_eq(center, tol::POINT_MERGE));
    assert!((first.geom.radius - 0.3).abs() < tol::POINT_MERGE);

    // Slide it east along the edge: the circle's center slides with it.
    move_unit_at(
        &mut doc,
        None,
        id,
        plane,
        top_pt,
        &Transform::translation(Vec3::new(0.1, 0.0, 0.0)),
    )
    .expect("slide the arc along the edge");
    let moved = run(&doc);
    assert_eq!(moved.edges.len(), 18);
    assert!(
        moved
            .geom
            .center
            .approx_eq(Point3::new(0.6, 0.0, 1.0), tol::POINT_MERGE)
    );
    doc.object(id).unwrap().validate().unwrap();

    doc.undo().unwrap();
    let back = run(&doc);
    assert!(
        back.geom.center.approx_eq(center, tol::POINT_MERGE),
        "undo restores the original circle"
    );
    doc.redo().unwrap();
    let again = run(&doc);
    assert!(
        again
            .geom
            .center
            .approx_eq(Point3::new(0.6, 0.0, 1.0), tol::POINT_MERGE)
    );
}

/// Deleting a segment and undoing gives the segment back with its arc, not a
/// plain polygon: the dissolve snapshots every edge's claim.
#[test]
fn dissolving_a_segment_and_undoing_restores_its_arc() {
    let (mut doc, id) = doc_cube();
    let t = top(doc.object(id).unwrap());
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.3,
    };
    let pts = arc(0.5, 0.5, 0.3, 0.0, 2.0 * std::f64::consts::PI / 3.0, 12);
    let mut curves = vec![Some(g); 12];
    curves.push(None);
    let mid = pts[6];
    doc.apply_object_op(
        id,
        KernelOp::SplitFaceInner {
            face: t,
            loop_path: pts,
            restore: None,
            curves,
            edge_attrs: Vec::new(),
        },
    )
    .unwrap();
    let plane = face_arrangement_support::face_plane(&doc, id, t);
    // The arc's claimed facets, as the curve run the drawing unit carries.
    let arc_count = |doc: &Document| {
        units_on(doc, None, id, plane)
            .units
            .iter()
            .flat_map(|u| u.curves.iter())
            .map(|r| r.edges.len())
            .next()
    };
    assert_eq!(arc_count(&doc), Some(12));
    delete_unit_at(&mut doc, None, id, plane, mid).unwrap();
    assert_eq!(arc_count(&doc), None, "gone");
    doc.undo().unwrap();
    assert_eq!(arc_count(&doc), Some(12), "the arc is back");
    doc.redo().unwrap();
    doc.undo().unwrap();
    assert_eq!(arc_count(&doc), Some(12));
}

/// Pushing a flat segment THROUGH the solid cuts a tunnel whose arc walls
/// are stamped and whose chord wall is flat: only genuine facets reach the
/// swept tool, even though the chord's endpoints lie on the circle too.
#[test]
fn pushing_a_segment_through_stamps_the_arc_walls_only() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.3,
    };
    let pts = arc(0.5, 0.5, 0.3, 0.0, 2.0 * std::f64::consts::PI / 3.0, 12);
    // Claim every edge, the chord included (its endpoints are on the circle,
    // so the imprint accepts it): the push-through must still not stamp it.
    let disk = obj
        .split_face_inner_with_curve(t, &pts, Some(g))
        .unwrap()
        .sub_face;
    let through = obj.push_through(disk, -1.5).expect("a through-cut");
    through.validate().unwrap();
    assert_eq!(
        stamped_walls(&through),
        12,
        "the tunnel's arc walls are cylinder facets; its chord wall is flat"
    );
}

/// A circle at the density floor (24 facets, each subtending exactly one
/// 15° step) carried through a rotated, uniformly scaled pose still bosses
/// into 24 cylinder walls: the facet gate leaves room for the float noise a
/// similarity and the wasm boundary introduce, so an everyday hole never
/// silently loses its smooth walls at the threshold.
#[test]
fn a_floor_density_circle_stays_a_facet_ring_through_a_similarity() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let center = Point3::new(0.5, 0.5, 1.0);
    let g = CurveGeom {
        center,
        radius: 0.3,
    };
    let ring: Vec<Point3> = (0..24)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 24.0;
            Point3::new(0.5 + 0.3 * a.cos(), 0.5 + 0.3 * a.sin(), 1.0)
        })
        .collect();
    let disk = obj
        .split_face_inner_with_curve(t, &ring, Some(g))
        .unwrap()
        .sub_face;
    // An awkward similarity: a turn about a skew axis, scaled by 1.7.
    let axis = Vec3::new(0.3, -0.5, 0.8).normalized().unwrap();
    let xf = Transform::rotation(axis, 0.7)
        .unwrap()
        .then(&Transform::uniform_scale(1.7))
        .then(&Transform::translation(Vec3::new(2.0, -1.0, 0.5)));
    obj.apply_transform(&xf)
        .expect("a similarity keeps the claims");
    obj.validate().unwrap();
    assert_eq!(claims(&obj).len(), 24, "the claims mapped with the circle");

    obj.extrude_sub_face(disk, 0.2)
        .expect("boss the transformed circle");
    obj.validate().unwrap();
    assert_eq!(
        stamped_walls(&obj),
        24,
        "every floor-density facet still stamps its wall"
    );
}

/// Offsetting a segment's face hands the inset its arc: the offset loop
/// carries each arc facet's concentric circle, the imprint stamps them per
/// edge, and recessing the inset raises smooth arc walls (a hollowed
/// half-round cup's inner curve is as smooth as its outer one).
#[test]
fn offsetting_a_segment_and_recessing_the_inset_keeps_the_inner_arc_smooth() {
    let mut obj = unit_cube();
    let t = top(&obj);
    let center = Point3::new(0.5, 0.5, 1.0);
    let g = CurveGeom {
        center,
        radius: 0.3,
    };
    let pts = arc(0.5, 0.5, 0.3, 0.0, std::f64::consts::PI, 18);
    let mut curves = vec![Some(g); 18];
    curves.push(None);
    let seg = obj
        .split_face_inner_with_curves(t, &pts, &curves)
        .expect("a half-disc segment")
        .sub_face;

    let inset = kernel::offset_face_boundary(&obj, seg, -0.05).expect("offset inward");
    let inner_arc = inset.curves.iter().filter(|c| c.is_some()).count();
    assert!(
        inner_arc >= 16,
        "the arc facets offset as arcs ({inner_arc})"
    );
    assert!(
        inset.curves.iter().flatten().all(|c| {
            c.center.approx_eq(center, tol::POINT_MERGE) && (c.radius - 0.25).abs() < 1e-9
        }),
        "the offset arc is concentric, 5 cm in"
    );
    let ring = obj
        .split_face_inner_with_curves(seg, &inset.points, &inset.curves)
        .expect("imprint the inset with its arc")
        .sub_face;
    obj.validate().unwrap();

    obj.extrude_sub_face(ring, -0.2).expect("recess the inset");
    obj.validate().unwrap();
    assert_eq!(
        stamped_walls(&obj),
        inner_arc,
        "every inner arc wall is a cylinder facet; the straight wall is flat"
    );
    assert_eq!(
        claims_centered(&obj, Point3::new(0.5, 0.5, 0.8)),
        inner_arc,
        "the sunk rim carries the inner arc down with it"
    );
}
