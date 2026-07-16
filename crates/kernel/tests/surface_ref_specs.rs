//! Executable specs for analytic surface references on solid faces
//! (the true-curves design, stage 2): extrusion stamps the side walls of
//! arc/circle profile edges with the cylinder they are chord facets of; the
//! reference rides every op under the map-or-drop contract; it persists in
//! geometry buffer v4; and the validator holds a present reference to the
//! geometry it claims.

use kernel::{
    CurveGeom, Document, Object, Plane, Point3, Sketch, SurfaceRef, Transform, Vec3, tol,
};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// A faceted circle committed as one analytic curve chain on a fresh ground
/// sketch; returns the sketch (with exactly one region).
fn circle_sketch(center: Point3, radius: f64, n: usize) -> Sketch {
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom { center, radius }).unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
            0.0,
        )
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    assert_eq!(s.regions().len(), 1, "circle closes one region");
    s
}

/// Extrudes the sketch's sole region by `h`.
fn extrude(s: &Sketch, h: f64) -> Object {
    let region = s.regions().keys().next().unwrap();
    let profile = s.profile(region).unwrap();
    Object::from_extrusion(&profile, h).unwrap()
}

/// The distinct surface references present on `obj`, plus how many faces
/// carry one.
fn surface_census(obj: &Object) -> (Vec<SurfaceRef>, usize) {
    let mut distinct: Vec<SurfaceRef> = Vec::new();
    let mut carrying = 0;
    for f in obj.faces().values() {
        if let Some(sr) = f.surface {
            carrying += 1;
            if !distinct.contains(&sr) {
                distinct.push(sr);
            }
        }
    }
    (distinct, carrying)
}

// ------------------------------------------------------------- extrusion

#[test]
fn extruded_circle_walls_carry_their_cylinder() {
    let s = circle_sketch(Point3::new(1.0, 2.0, 0.0), 0.5, 24);
    let obj = extrude(&s, 1.0);
    obj.validate().unwrap();

    // 24 walls + 2 caps; every wall carries the same cylinder, caps none.
    assert_eq!(obj.faces().len(), 26);
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 24, "every wall facet is attributed");
    assert_eq!(distinct.len(), 1, "one cylinder, shared bitwise");
    let SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = distinct[0];
    assert!(axis_point.approx_eq(Point3::new(1.0, 2.0, 0.0), tol::POINT_MERGE));
    assert!(axis.approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION));
    assert_eq!(radius, 0.5, "the exact drawn radius, not a chord measure");

    // Caps are the two faces with no reference.
    let caps = obj.faces().values().filter(|f| f.surface.is_none()).count();
    assert_eq!(caps, 2);
}

#[test]
fn plain_polygon_extrusion_carries_nothing() {
    // Same 24-gon drawn WITHOUT geometry: identity-only chain.
    let mut s = Sketch::on_plane(ground());
    s.begin_curve();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / 24.0;
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..24 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let obj = extrude(&s, 1.0);
    assert_eq!(surface_census(&obj).1, 0, "no geometry, no claims");
}

#[test]
fn mixed_arc_and_chord_profile_attributes_only_the_arc_walls() {
    // Half-disc: a semicircular arc chain (with geometry) closed by a chord.
    let mut s = Sketch::on_plane(ground());
    let center = Point3::new(0.0, 0.0, 0.0);
    s.begin_curve_with(CurveGeom {
        center,
        radius: 1.0,
    })
    .unwrap();
    let n = 12; // facets over the upper half
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    // Chord back along y=0 (a plain line).
    s.add_segment(Point3::new(-1.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    assert_eq!(s.regions().len(), 1);

    let obj = extrude(&s, 0.5);
    obj.validate().unwrap();
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, n, "exactly the arc's facets are attributed");
    assert_eq!(distinct.len(), 1);
}

#[test]
fn hole_walls_carry_the_hole_curves_cylinder() {
    // A big plain square with a circular hole drawn as an analytic chain.
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((-3.0, -3.0), (3.0, -3.0)),
        ((3.0, -3.0), (3.0, 3.0)),
        ((3.0, 3.0), (-3.0, 3.0)),
        ((-3.0, 3.0), (-3.0, -3.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / 16.0;
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..16 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();

    // The annular region: square outer, circle hole.
    let region = s
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("annulus exists");
    let profile = s.profile(region).unwrap();
    let obj = Object::from_extrusion(&profile, 1.0).unwrap();
    obj.validate().unwrap();

    // 4 outer walls unattributed; 16 tunnel walls attributed.
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 16);
    assert_eq!(distinct.len(), 1);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 1.0);
}

#[test]
fn negative_distance_extrusion_keeps_attribution() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let obj = extrude(&s, -1.0);
    obj.validate().unwrap();
    assert_eq!(surface_census(&obj).1, 24);
}

// ------------------------------------------------------------ propagation

#[test]
fn boolean_subtract_inherits_attribution_per_arrangement_fragment() {
    let n = 24usize;
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, n);
    let cyl = extrude(&s, 1.0);
    // Subtract a box whose x = 0.55 wall crosses facet INTERIORS: for a
    // 24-gon, x = 0.5 = cos(60°) passes exactly through existing vertices
    // (no facet would actually be split), so the cutter plane deliberately
    // sits between vertex abscissae, forcing the arrangement to cut chord
    // facets and produce genuine sub-face fragments.
    let cut_x = 0.55;
    let cutter = {
        let mut cs = Sketch::on_plane(ground());
        for (a, b) in [
            ((cut_x, -2.0), (2.0, -2.0)),
            ((2.0, -2.0), (2.0, 2.0)),
            ((2.0, 2.0), (cut_x, 2.0)),
            ((cut_x, 2.0), (cut_x, -2.0)),
        ] {
            cs.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        extrude(&cs, 1.0)
    };
    let r = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();
    r.validate().unwrap();

    // Per-fragment contract: a result face is a wall fragment of the
    // original cylinder iff its plane sits at the chord-plane (apothem)
    // distance from the axis — the cutter's own walls sit elsewhere. Every
    // such fragment must carry the original cylinder reference; every other
    // face (cap fragments, cutter-derived walls) must carry none.
    let apothem = (std::f64::consts::PI / n as f64).cos();
    let expected = SurfaceRef::Cylinder {
        axis_point: Point3::new(0.0, 0.0, 0.0),
        axis: Vec3::new(0.0, 0.0, 1.0),
        radius: 1.0,
    };
    let mut wall_fragments = 0;
    let mut seam_cut_fragments = 0;
    for f in r.faces().values() {
        let nrm = f.plane.normal();
        let on_chord_plane = nrm.z.abs() < 1e-9
            && (f.plane.signed_distance(Point3::ORIGIN).abs() - apothem).abs() < 1e-9;
        if on_chord_plane {
            wall_fragments += 1;
            assert_eq!(
                f.surface,
                Some(expected),
                "every wall fragment carries the original cylinder"
            );
            // A fragment with a vertex ON the cutter plane was genuinely
            // produced by the arrangement (the seam split its facet).
            if r.loop_positions(f.outer_loop)
                .any(|p| (p.x - cut_x).abs() < 1e-9)
            {
                seam_cut_fragments += 1;
            }
        } else {
            assert_eq!(
                f.surface, None,
                "fragments of unattributed faces stay unattributed"
            );
        }
    }
    assert!(wall_fragments > 0, "some wall fragments survive");
    assert!(
        seam_cut_fragments >= 2,
        "the cutter plane crosses facet interiors, so arrangement-split          fragments exist (top and bottom of the cut on each side)"
    );
}

// The seam-dissolve gate (`mergeable_edge_endpoints`): an attributed facet
// never dissolves into a coplanar neighbor with a DIFFERENT surface claim,
// while equal claims still dissolve exactly as unattributed coplanar faces
// always have. This is the gate the boolean-result cleanup and push-through
// rely on to keep one wall's analytic identity from bleeding across a seam.

/// A profile with a plain straight edge collinear with a single-facet arc
/// chain: extrusion yields two COPLANAR adjacent walls, one attributed, one
/// not. The dissolve pass must leave them separate.
#[test]
fn seam_dissolve_refuses_across_differing_surface_claims() {
    let mut s = Sketch::on_plane(ground());
    // Plain edge (0,0) -> (1,0), then a one-facet "arc" chain continuing
    // collinearly (1,0) -> (2,0): a chord of a large circle centered off to
    // the side (center on the perpendicular bisector x = 1.5).
    s.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let radius = (0.25f64 + 25.0).sqrt(); // |(2,0)-(1.5,5)|
    s.begin_curve_with(CurveGeom {
        center: Point3::new(1.5, 5.0, 0.0),
        radius,
    })
    .unwrap();
    s.add_segment(Point3::new(1.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0))
        .unwrap();
    s.end_curve();
    // Close the profile.
    for (a, b) in [
        ((2.0, 0.0), (2.0, 1.0)),
        ((2.0, 1.0), (0.0, 1.0)),
        ((0.0, 1.0), (0.0, 0.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let mut obj = extrude(&s, 1.0);
    obj.validate().unwrap();

    // Two coplanar adjacent walls along y = 0: one attributed, one not.
    let y0_walls = |o: &Object| {
        o.faces()
            .values()
            .filter(|f| f.plane.normal().approx_eq(Vec3::new(0.0, -1.0, 0.0), 1e-9))
            .count()
    };
    assert_eq!(y0_walls(&obj), 2, "collinear profile edges give two walls");
    assert_eq!(surface_census(&obj).1, 1, "exactly one wall is attributed");

    let dissolved = obj.merge_coplanar_faces(&[]);
    assert_eq!(
        dissolved, 0,
        "differing surface claims must block the dissolve"
    );
    assert_eq!(y0_walls(&obj), 2, "both walls survive, attribution intact");
    assert_eq!(surface_census(&obj).1, 1);
    obj.validate().unwrap();
}

/// The complementary case: two coplanar fragments of the SAME cylinder
/// (equal SurfaceRef) dissolve exactly as unattributed coplanar faces do —
/// the gate must not over-block.
#[test]
fn seam_dissolve_still_merges_equal_surface_claims() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 8);
    let mut obj = extrude(&s, 2.0);
    let wall = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(id, _)| id)
        .unwrap();
    let wall_surface = obj.faces()[wall].surface;

    // Split the wall at mid-height: two coplanar fragments, equal claims.
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[wall].outer_loop).collect();
    let mid =
        |a: Point3, b: Point3| Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0);
    obj.split_face(wall, &[mid(ring[0], ring[3]), mid(ring[1], ring[2])])
        .unwrap();
    assert_eq!(obj.faces().len(), 11, "8 walls + 1 split + 2 caps");

    let dissolved = obj.merge_coplanar_faces(&[]);
    assert_eq!(
        dissolved, 1,
        "equal claims dissolve like plain coplanar faces"
    );
    assert_eq!(obj.faces().len(), 10);
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 8, "the re-merged wall keeps the claim");
    assert_eq!(distinct, vec![wall_surface.unwrap()]);
    obj.validate().unwrap();
}

#[test]
fn transform_maps_or_drops_attribution() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 12);
    let mut obj = extrude(&s, 1.0);

    // Rigid move + uniform scale: mapped exactly.
    obj.apply_transform(&Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();
    obj.apply_transform(&Transform::uniform_scale(2.0)).unwrap();
    obj.validate().unwrap();
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 12);
    let SurfaceRef::Cylinder {
        axis_point, radius, ..
    } = distinct[0];
    assert!(axis_point.approx_eq(Point3::new(10.0, 0.0, 0.0), 1e-9));
    assert!((radius - 2.0).abs() < 1e-12, "radius scales uniformly");

    // Rotation about a non-axis direction: still a similarity — kept, and
    // the axis follows the rotation (validator agrees).
    let rot = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 0.5).unwrap();
    obj.apply_transform(&rot).unwrap();
    obj.validate().unwrap();
    assert_eq!(surface_census(&obj).1, 12);

    // Non-uniform scale: the section becomes an ellipse — dropped, carrier
    // untouched.
    obj.apply_transform(&Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    obj.validate().unwrap();
    assert_eq!(surface_census(&obj).1, 0, "non-similarity drops all claims");
}

#[test]
fn push_pull_on_a_cap_stretches_walls_and_keeps_their_attribution() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let mut obj = extrude(&s, 1.0);

    // The top cap: the face with no surface whose normal is +z.
    let top = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.surface.is_none() && f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9)
        })
        .map(|(id, _)| id)
        .unwrap();
    obj.push_pull(top, 1.0).unwrap();
    obj.validate().unwrap();

    // Walls stretched to height 2; every one still on the same cylinder.
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 24);
    assert_eq!(distinct.len(), 1);
}

#[test]
fn extruding_an_imprinted_sub_face_drops_its_inherited_claim() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 8);
    let mut obj = extrude(&s, 2.0);

    // Pick a wall facet and imprint a small quad strictly inside it.
    let wall = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(id, _)| id)
        .unwrap();
    let plane = obj.faces()[wall].plane;
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[wall].outer_loop).collect();
    let centroid = {
        let inv = 1.0 / ring.len() as f64;
        ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
            Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
        })
    };
    // A small in-plane square around the centroid.
    let n = plane.normal();
    let u = ring[1] - ring[0];
    let u = u.normalized().unwrap();
    let v = n.cross(u);
    let d = 0.05;
    let loop_path: Vec<Point3> = [
        centroid + u * -d + v * -d,
        centroid + u * d + v * -d,
        centroid + u * d + v * d,
        centroid + u * -d + v * d,
    ]
    .to_vec();
    let report = obj.split_face_inner(wall, &loop_path).unwrap();
    // The imprinted sub-face inherits the wall's claim (same chord plane).
    assert_eq!(
        obj.faces()[report.sub_face].surface,
        obj.faces()[wall].surface,
        "imprint inherits"
    );

    // Raising it moves it off the chord plane: the claim drops.
    obj.extrude_sub_face(report.sub_face, 0.02).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.faces()[report.sub_face].surface, None);
    assert!(
        obj.faces()[wall].surface.is_some(),
        "the parent keeps its own"
    );
}

// ------------------------------------------------------------ persistence

#[test]
fn surface_refs_round_trip_through_save_load() {
    let mut doc = Document::new();
    let sk = doc.add_sketch(ground());
    {
        let s = doc.sketch_mut(sk).unwrap();
        s.begin_curve_with(CurveGeom {
            center: Point3::new(1.0, 2.0, 0.0),
            radius: 0.5,
        })
        .unwrap();
        let p = |i: usize| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / 16.0;
            Point3::new(1.0 + 0.5 * a.cos(), 2.0 + 0.5 * a.sin(), 0.0)
        };
        for i in 0..16 {
            s.add_segment(p(i), p(i + 1)).unwrap();
        }
        s.end_curve();
    }
    let region = doc.sketch(sk).unwrap().regions().keys().next().unwrap();
    let (obj_id, _) = doc.extrude_region(sk, region, 1.0).unwrap();
    let before = surface_census(doc.object(obj_id).unwrap());
    assert_eq!(before.1, 16);

    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    let obj2 = doc2.visible_object_ids()[0];
    let after = surface_census(doc2.object(obj2).unwrap());
    assert_eq!(after, before, "geometry buffer v4 preserves the references");

    // Byte-stable re-save.
    assert_eq!(doc2.save(), bytes);
}

// ------------------------------------------------- whole-wall push/pull
// the true-curves design §4.6: push/pull on an attributed cylinder wall
// facet acts on the LOGICAL wall — a radial offset of every face claiming
// the same cylinder — never a translate of the one facet. Refusals are
// typed; the object is untouched on error.

/// Distance from `p` to the +Z axis line through `axis_point`.
fn dist_to_axis(p: Point3, axis_point: Point3) -> f64 {
    let d = p - axis_point;
    (d.x * d.x + d.y * d.y).sqrt() + 0.0 * d.z // axis is +Z in every test here
}

/// Some attributed wall face of `obj`.
fn any_wall(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.surface.is_some())
        .map(|(fid, _)| fid)
        .expect("an attributed wall exists")
}

#[test]
fn wall_offset_outward_grows_the_whole_cylinder() {
    let center = Point3::new(1.0, 2.0, 0.0);
    let s = circle_sketch(center, 0.5, 24);
    let mut obj = extrude(&s, 1.0);
    let wall = any_wall(&obj);

    let report = obj.push_pull(wall, 0.25).unwrap();
    obj.validate().unwrap();

    // No topology change: same face handle, nothing created or removed.
    assert_eq!(report.face, wall);
    assert!(report.created_faces.is_empty() && report.removed_faces.is_empty());
    assert_eq!(obj.faces().len(), 26);
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);

    // Every wall claims the new radius, bitwise-shared; caps still claim none.
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 24);
    assert_eq!(distinct.len(), 1);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 0.75, "surface reference maps to the offset radius");

    // And the carrier agrees: every wall vertex sits on the new cylinder.
    for f in obj.faces().values().filter(|f| f.surface.is_some()) {
        for p in obj.loop_positions(f.outer_loop) {
            assert!(
                (dist_to_axis(p, center) - 0.75).abs() <= tol::POINT_MERGE,
                "wall vertices moved to the new radius"
            );
        }
    }
}

#[test]
fn wall_offset_inward_then_outward_is_the_identity() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let mut obj = extrude(&s, 2.0);
    let before: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let wall = any_wall(&obj);

    let r1 = obj.push_pull(wall, -0.4).unwrap();
    obj.push_pull(r1.face, 0.4).unwrap();
    obj.validate().unwrap();

    let after: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert!(
            b.approx_eq(*a, tol::POINT_MERGE),
            "round-trip restores geometry"
        );
    }
    let (distinct, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert!((radius - 1.0).abs() <= tol::POINT_MERGE);
}

#[test]
fn wall_offset_to_nothing_refuses_typed() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 24);
    let mut obj = extrude(&s, 1.0);
    let snapshot: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let wall = any_wall(&obj);

    let err = obj.push_pull(wall, -0.5).unwrap_err();
    assert_eq!(err, kernel::PushPullError::RadiusVanishes);
    let err = obj.push_pull(wall, -0.7).unwrap_err();
    assert_eq!(err, kernel::PushPullError::RadiusVanishes);

    // Strong guarantee: untouched.
    let now: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    assert_eq!(snapshot.len(), now.len());
    for (b, a) in snapshot.iter().zip(&now) {
        assert!(b.approx_eq(*a, 0.0_f64.max(tol::POINT_MERGE)));
    }
    let (distinct, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 0.5);
}

#[test]
fn hole_wall_offset_resizes_the_hole() {
    // A drilled block: square outer profile, analytic circular hole.
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((-3.0, -3.0), (3.0, -3.0)),
        ((3.0, -3.0), (3.0, 3.0)),
        ((3.0, 3.0), (-3.0, 3.0)),
        ((-3.0, 3.0), (-3.0, -3.0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / 16.0;
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..16 {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    let region = s
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    let profile = s.profile(region).unwrap();
    let mut obj = Object::from_extrusion(&profile, 1.0).unwrap();
    let wall = any_wall(&obj);

    // A tunnel wall faces INTO the tunnel (toward the axis): a positive
    // push/pull distance moves the wall along its normal, toward the axis —
    // the hole SHRINKS (material is added), mirroring the outer-wall sign.
    obj.push_pull(wall, 0.25).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 16);
    assert_eq!(distinct.len(), 1);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 0.75, "positive distance shrinks a hole");
    // The block's outer corners never moved.
    assert!(
        obj.vertices().values().any(|v| v
            .position
            .approx_eq(Point3::new(3.0, 3.0, 0.0), tol::POINT_MERGE)),
        "unrelated geometry is untouched"
    );
}

#[test]
fn d_profile_chord_wall_translates_with_the_arc() {
    // D-shape: semicircular arc chain closed by a straight chord. The chord
    // wall's four vertices ALL lie on the arc's end seams, so the radial
    // offset maps the whole chord wall affinely — it stays planar and keeps
    // carrying no surface claim.
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let n = 12;
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s.add_segment(Point3::new(-1.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let mut obj = extrude(&s, 0.5);
    let wall = any_wall(&obj);

    obj.push_pull(wall, 0.5).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, n);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 1.5);
    // The chord wall stretched to the new seam positions but stayed a
    // plane — and gained no claim it never had.
    let chord = obj
        .faces()
        .values()
        .find(|f| f.plane.normal().approx_eq(Vec3::new(0.0, -1.0, 0.0), 1e-9))
        .expect("chord wall survives");
    assert_eq!(chord.surface, None);
    for pnt in obj.loop_positions(chord.outer_loop) {
        assert!((pnt.x.abs() - 1.5).abs() <= tol::POINT_MERGE || pnt.x.abs() <= 1.5);
    }
}

/// A 2×2 square with its (2,2) corner rounded by a quarter-circle fillet
/// (center (1,1), radius 1), extruded to height 0.5.
fn rounded_corner_solid() -> Object {
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((0.0, 0.0), (2.0, 0.0)),
        ((2.0, 0.0), (2.0, 1.0)), // tangent wall, ends at fillet start
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    s.begin_curve_with(CurveGeom {
        center: Point3::new(1.0, 1.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let n = 6;
    let p = |i: usize| {
        let a = std::f64::consts::PI / 2.0 * (i as f64) / (n as f64);
        Point3::new(1.0 + a.cos(), 1.0 + a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    for (a, b) in [((1.0, 2.0), (0.0, 2.0)), ((0.0, 2.0), (0.0, 0.0))] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    assert_eq!(s.regions().len(), 1);
    extrude(&s, 0.5)
}

#[test]
fn rounded_corner_tangent_walls_pivot_with_the_fillet() {
    // A square with one rounded corner: the straight walls tangent to the
    // fillet share one vertical seam with the arc band. Offsetting the
    // fillet radius moves that seam radially; a prism wall's two vertical
    // edges stay parallel, so the tangent wall PIVOTS about its far edge —
    // planar, watertight, and the far corner never moves.
    let mut obj = rounded_corner_solid();
    let wall = any_wall(&obj);

    obj.push_pull(wall, 0.2).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
    let (distinct, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 1.2, "fillet radius offsets");

    // The fillet's seam vertex moved radially: (2,1) -> (2.2,1).
    assert!(
        obj.vertices().values().any(|v| v
            .position
            .approx_eq(Point3::new(2.2, 1.0, 0.0), tol::POINT_MERGE)),
        "seam vertex followed the radius"
    );
    // The tangent wall's far corner is pinned.
    assert!(
        obj.vertices().values().any(|v| v
            .position
            .approx_eq(Point3::new(2.0, 0.0, 0.0), tol::POINT_MERGE)),
        "far corner never moves"
    );
}

#[test]
fn bossed_neighbor_wall_refuses_the_offset_typed() {
    // Emboss a sub-face on the tangent wall (x = 2 plane), raising a boss:
    // the parent wall now carries a hole loop pinned at x = 2 while its
    // outer seam follows the fillet — offsetting the fillet would bend the
    // parent off any plane. Typed refusal, object untouched.
    let mut obj = rounded_corner_solid();
    let tangent = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.surface.is_none() && f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9)
        })
        .map(|(fid, _)| fid)
        .expect("tangent wall at x = 2");
    let boss_loop = vec![
        Point3::new(2.0, 0.3, 0.1),
        Point3::new(2.0, 0.7, 0.1),
        Point3::new(2.0, 0.7, 0.4),
        Point3::new(2.0, 0.3, 0.4),
    ];
    let split = obj.split_face_inner(tangent, &boss_loop).unwrap();
    obj.extrude_sub_face(split.sub_face, 0.1).unwrap();
    obj.validate().unwrap();
    let snapshot: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let wall = any_wall(&obj);

    let err = obj.push_pull(wall, 0.2).unwrap_err();
    assert_eq!(err, kernel::PushPullError::WallNeighborNonPlanar);
    let now: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    for (b, a) in snapshot.iter().zip(&now) {
        assert!(b.approx_eq(*a, tol::POINT_MERGE), "untouched on refusal");
    }
}

#[test]
fn boolean_cut_face_follows_the_radial_offset() {
    // A cylinder with a flat planar cut through its wall: every vertex of
    // the cut face lies on wall fragments, so the cut face maps under the
    // same cross-section affine — it translates outward proportionally
    // (x = 0.55 -> 0.55·k) and stays planar. The wall fragments all map to
    // the new radius together.
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let cyl = extrude(&s, 1.0);
    let cutter = {
        let mut cs = Sketch::on_plane(ground());
        for (a, b) in [
            ((0.55, -2.0), (2.0, -2.0)),
            ((2.0, -2.0), (2.0, 2.0)),
            ((2.0, 2.0), (0.55, 2.0)),
            ((0.55, 2.0), (0.55, -2.0)),
        ] {
            cs.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        extrude(&cs, 1.0)
    };
    let mut obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();
    let wall = any_wall(&obj);
    obj.push_pull(wall, 0.2).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);

    // Every attributed fragment claims the offset radius.
    let (distinct, _) = surface_census(&obj);
    assert_eq!(distinct.len(), 1);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 1.2);

    // The cut face followed the cross-section scale: x = 0.55·1.2 = 0.66.
    let cut = obj
        .faces()
        .values()
        .find(|f| f.plane.normal().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9))
        .expect("cut face survives");
    for p in obj.loop_positions(cut.outer_loop) {
        assert!(
            (p.x - 0.66).abs() <= 1e-9,
            "cut face translated with the cross-section"
        );
    }
}

#[test]
fn drilled_cylinder_offsets_outer_and_hole_walls_independently() {
    // Tube: subtract a coaxial thin cylinder from a fat one. Outer band and
    // tunnel band claim different cylinders; each offsets alone, and the
    // annular caps reshape in-plane.
    let fat = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let thin = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.4, 16), 1.0);
    let mut obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &fat,
        &thin,
        &Transform::IDENTITY,
    )
    .unwrap();
    obj.validate().unwrap();
    let (distinct, _) = surface_census(&obj);
    assert_eq!(distinct.len(), 2, "outer cylinder + tunnel cylinder");

    // Offset the OUTER wall out by 0.5: outer radius 1.5, tunnel unchanged.
    let outer_wall = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            matches!(f.surface, Some(SurfaceRef::Cylinder { radius, .. }) if (radius - 1.0).abs() < 1e-12)
        })
        .map(|(fid, _)| fid)
        .unwrap();
    obj.push_pull(outer_wall, 0.5).unwrap();
    obj.validate().unwrap();
    let radii: Vec<f64> = {
        let (d, _) = surface_census(&obj);
        d.iter()
            .map(|s| {
                let SurfaceRef::Cylinder { radius, .. } = s;
                *radius
            })
            .collect()
    };
    assert!(radii.contains(&1.5) && radii.contains(&0.4));

    // Offset the TUNNEL wall toward its axis (positive distance = hole
    // shrinks): tunnel radius 0.3.
    let hole_wall = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            matches!(f.surface, Some(SurfaceRef::Cylinder { radius, .. }) if (radius - 0.4).abs() < 1e-12)
        })
        .map(|(fid, _)| fid)
        .unwrap();
    obj.push_pull(hole_wall, 0.1).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
    let radii: Vec<f64> = {
        let (d, _) = surface_census(&obj);
        d.iter()
            .map(|s| {
                let SurfaceRef::Cylinder { radius, .. } = s;
                *radius
            })
            .collect()
    };
    assert!(radii.contains(&1.5) && radii.iter().any(|r| (r - 0.3).abs() < 1e-12));
}

#[test]
fn cap_imprint_blocks_a_shrink_that_would_orphan_it() {
    // A ring imprinted on the top cap near the rim: shrinking the wall past
    // it would leave the imprint outside its own face. Refused; a small
    // shrink that clears it succeeds.
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let mut obj = extrude(&s, 1.0);
    let top_cap = obj
        .faces()
        .iter()
        .find(|(_, f)| f.surface.is_none() && f.plane.normal().z > 0.5)
        .map(|(fid, _)| fid)
        .unwrap();
    let d = 0.55; // square imprint reaching to |x|,|y| = 0.55 (inside r=1)
    let loop_path = vec![
        Point3::new(-d, -d, 1.0),
        Point3::new(d, -d, 1.0),
        Point3::new(d, d, 1.0),
        Point3::new(-d, d, 1.0),
    ];
    obj.split_face_inner(top_cap, &loop_path).unwrap();
    let wall = any_wall(&obj);

    // Shrinking to r=0.5 would put the rim inside the imprint: refuse.
    let err = obj.push_pull(wall, -0.5).unwrap_err();
    assert_eq!(err, kernel::PushPullError::NonManifoldResult);

    // Shrinking to r=0.9 clears the imprint's corners (|corner| ≈ 0.78).
    obj.push_pull(wall, -0.1).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
}

#[test]
fn attributed_walls_never_report_overshoot() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 0.5, 24);
    let obj = extrude(&s, 1.0);
    let wall = any_wall(&obj);
    assert!(
        !obj.push_pull_overshoots(wall, -10.0),
        "wall facets never route to push-through"
    );
}

#[test]
fn wall_offset_undoes_and_redoes_through_history() {
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let mut obj = extrude(&s, 1.0);
    let wall = any_wall(&obj);
    let before: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();

    let mut history = kernel::History::new();
    history
        .apply(
            &mut obj,
            kernel::KernelOp::PushPull {
                face: wall,
                distance: 0.3,
            },
        )
        .unwrap();
    let (d, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = d[0];
    assert_eq!(radius, 1.3);

    history.undo(&mut obj).unwrap();
    let after: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    for (b, a) in before.iter().zip(&after) {
        assert!(b.approx_eq(*a, tol::POINT_MERGE), "undo restores geometry");
    }
    let (d, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = d[0];
    assert!(
        (radius - 1.0).abs() <= tol::POINT_MERGE,
        "undo restores the claim"
    );

    history.redo(&mut obj).unwrap();
    let (d, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = d[0];
    assert!((radius - 1.3).abs() <= tol::POINT_MERGE);
}

proptest::proptest! {
    /// Any legal radial offset keeps the solid watertight and validated,
    /// maps the claim to exactly the offset radius, and inverts within
    /// tolerance — for any facet count, radius, height, and offset.
    #[test]
    fn prop_wall_offset_roundtrip(
        n in 8usize..40,
        r in 0.1f64..4.0,
        h in 0.1f64..3.0,
        frac in -0.85f64..2.0,
    ) {
        let d = r * frac;
        proptest::prop_assume!(d.abs() > 1e-6);
        let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), r, n);
        let mut obj = extrude(&s, h);
        let wall = any_wall(&obj);
        let before: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();

        let report = obj.push_pull(wall, d).unwrap();
        obj.validate().unwrap();
        proptest::prop_assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
        let (distinct, carrying) = surface_census(&obj);
        proptest::prop_assert_eq!(carrying, n);
        proptest::prop_assert_eq!(distinct.len(), 1);
        let SurfaceRef::Cylinder { radius, .. } = distinct[0];
        proptest::prop_assert!((radius - (r + d)).abs() <= 1e-9 * (1.0 + r));

        obj.push_pull(report.face, -d).unwrap();
        obj.validate().unwrap();
        let after: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
        proptest::prop_assert_eq!(before.len(), after.len());
        for (b, a) in before.iter().zip(&after) {
            proptest::prop_assert!(b.approx_eq(*a, 1e-9 * (1.0 + r)));
        }
    }
}

// ---------------------------------------------- radial offset interpenetration
// the true-curves design §4.6 (review follow-up F2): the whole-wall
// radial offset re-validates only faces sharing a moved vertex, so without
// dedicated guards a grown wall could pass straight through — or sweep
// cleanly past — geometry it shares nothing with, and the result would be
// planar, twin-consistent, and even report Watertight. These specs pin the
// guards; their semantics mirror the stretch-mode guards on the
// generalized-push/pull branch so the two unify at integration.

/// A plain (unattributed) rectangular solid over `[x0,x1] x [y0,y1]`,
/// extruded from the ground plane to height `h`.
fn rect_solid(x0: f64, x1: f64, y0: f64, y1: f64, h: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x1, y1), (x0, y1)),
        ((x0, y1), (x0, y0)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    extrude(&s, h)
}

/// The empirically confirmed engulfment repro: a two-shell Object (cylinder
/// r=1 plus a disjoint box fully inside the grow band), grown past the box.
/// Returns (object, wall face) ready for the offending push/pull.
fn cylinder_with_disjoint_box(box_x0: f64, box_x1: f64) -> (Object, kernel::FaceId) {
    let cyl = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let boxo = rect_solid(box_x0, box_x1, -0.5, 0.5, 0.6);
    // Raise the box to z in [0.2, 0.8]: strictly inside the wall's axial
    // span, so nothing about it is flush with the caps.
    let obj = Object::boolean(
        kernel::BooleanOp::Union,
        &cyl,
        &boxo,
        &Transform::translation(Vec3::new(0.0, 0.0, 0.2)),
    )
    .unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.shells().len(), 2, "disjoint union keeps two shells");
    let wall = any_wall(&obj);
    (obj, wall)
}

#[test]
fn wall_grow_that_would_engulf_a_disjoint_shell_refuses() {
    // Box at x in [1.5, 2.5]: growing r=1 -> 3 sweeps cleanly PAST it —
    // afterwards nothing intersects, the result validates and reports
    // Watertight, with the box entombed inside the cylinder's material.
    // The engulfment guard must refuse, byte-identically untouched.
    let (mut obj, wall) = cylinder_with_disjoint_box(1.5, 2.5);
    let before_verts: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let before_faces = obj.faces().len();

    let err = obj.push_pull(wall, 2.0).unwrap_err();
    assert_eq!(err, kernel::PushPullError::NonManifoldResult);

    // Strong guarantee, bitwise: a refusal must not have moved anything.
    let now: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    assert_eq!(before_verts, now);
    assert_eq!(before_faces, obj.faces().len());
    let (distinct, _) = surface_census(&obj);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 1.0, "the claim still says the old radius");
}

#[test]
fn wall_grow_into_another_shell_refuses() {
    // Box at x in [2.5, 3.5]: the grown wall (r=3, chords at apothem 2.97)
    // lands INSIDE the box — crossing contact rather than clean engulfment.
    let (mut obj, wall) = cylinder_with_disjoint_box(2.5, 3.5);
    let before_verts: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();

    let err = obj.push_pull(wall, 2.0).unwrap_err();
    assert_eq!(err, kernel::PushPullError::NonManifoldResult);

    let now: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    assert_eq!(before_verts, now);
}

#[test]
fn wall_grow_with_clearance_still_succeeds() {
    // Control: the same two-shell construction with the box at x in
    // [3.5, 4.5] — growing to r=3 leaves real clearance, and the guards
    // must not turn a legitimate grow into a refusal.
    let (mut obj, wall) = cylinder_with_disjoint_box(3.5, 4.5);

    obj.push_pull(wall, 2.0).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), kernel::WatertightState::Watertight);
    assert_eq!(obj.shells().len(), 2, "still two shells");
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 24);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert_eq!(radius, 3.0);
    // The box never moved.
    assert!(
        obj.vertices().values().any(|v| v
            .position
            .approx_eq(Point3::new(3.5, -0.5, 0.2), tol::POINT_MERGE)),
        "unrelated shell is untouched"
    );
}

// ------------------------------------------------------------ analytic rims
// the true-curves design — the rim-circle query quadrant/tangent
// inference derives from: exact circles at the claiming faces' axial
// extremes plus the angular range the facets actually cover.

#[test]
fn full_circle_rims_cover_everything_and_carry_four_quadrants() {
    let center = Point3::new(1.0, 2.0, 0.0);
    let obj = extrude(&circle_sketch(center, 0.5, 24), 1.0);
    let rims = obj.analytic_rims();
    assert_eq!(rims.len(), 2, "bottom and top rims");
    assert!(rims[0].coverage.is_none(), "full circle");
    assert!(rims[0].covers(1.234), "everything is covered");

    // Bottom rim at z=0, top at z=1; both carry exactly four quadrant
    // points, each exactly `radius` from the exact center.
    assert!(rims[0].center.approx_eq(center, tol::POINT_MERGE));
    assert!(
        rims[1]
            .center
            .approx_eq(Point3::new(1.0, 2.0, 1.0), tol::POINT_MERGE)
    );
    for rim in &rims {
        let quadrants = rim.quadrant_points();
        assert_eq!(quadrants.len(), 4);
        for q in &quadrants {
            let d = *q - rim.center;
            assert!((d.length() - 0.5).abs() <= tol::POINT_MERGE);
            assert!(
                d.dot(rim.axis).abs() <= tol::POINT_MERGE,
                "in the rim plane"
            );
        }
        // Multiple-of-4 drawn circles put vertices at the quadrant angles;
        // the quadrant points must be the TRUE circle points regardless.
        assert!(
            quadrants
                .iter()
                .any(|q| q.approx_eq(Point3::new(1.5, 2.0, rim.center.z), 1e-12)),
            "+X cardinal present"
        );
    }
}

#[test]
fn partial_arc_rims_cover_only_the_drawn_range() {
    // Upper half-disc (semicircle + chord): quadrant points exist at +Y and
    // at the two arc endpoints (+X/-X), but NOT at -Y where no facet is.
    let mut s = Sketch::on_plane(ground());
    s.begin_curve_with(CurveGeom {
        center: Point3::new(0.0, 0.0, 0.0),
        radius: 1.0,
    })
    .unwrap();
    let n = 12;
    let p = |i: usize| {
        let a = std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(a.cos(), a.sin(), 0.0)
    };
    for i in 0..n {
        s.add_segment(p(i), p(i + 1)).unwrap();
    }
    s.end_curve();
    s.add_segment(Point3::new(-1.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let obj = extrude(&s, 0.5);

    let rims = obj.analytic_rims();
    assert_eq!(rims.len(), 2);
    let rim = &rims[0];
    assert!(rim.coverage.is_some(), "a half circle is not full coverage");
    let quadrants = rim.quadrant_points();
    assert_eq!(quadrants.len(), 3, "+Y and the two endpoints; never -Y");
    assert!(
        quadrants
            .iter()
            .any(|q| q.approx_eq(Point3::new(0.0, 1.0, 0.0), 1e-12)),
        "apex +Y quadrant"
    );
    assert!(
        !quadrants.iter().any(|q| q.y < -tol::POINT_MERGE),
        "the uncovered -Y quadrant is never offered"
    );
}

#[test]
fn notch_cut_into_one_rim_uncovers_only_that_rim() {
    // Coverage is PER RIM, not per surface: a notch boolean-cut into the
    // TOP rim removes that rim's arc without touching the bottom rim, so
    // quadrant/tangent snaps must vanish on the notched arc while the
    // intact bottom circle keeps offering them. (A merged, per-surface
    // coverage would let the intact bottom rim mask the notch.)
    //
    // Cylinder r=1, h=1 about the origin; the cutter is a box over
    // x in [0.55, 2], y in [-0.4, 0.4], z in [0.5, 1.1] — it eats the +X
    // side of the top half of the wall (and the cap above it) only.
    let cyl = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let cutter = {
        let mut cs = Sketch::on_plane(ground());
        for (a, b) in [
            ((0.55, -0.4), (2.0, -0.4)),
            ((2.0, -0.4), (2.0, 0.4)),
            ((2.0, 0.4), (0.55, 0.4)),
            ((0.55, 0.4), (0.55, -0.4)),
        ] {
            cs.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        extrude(&cs, 0.6)
    };
    let obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::translation(Vec3::new(0.0, 0.0, 0.5)),
    )
    .unwrap();
    obj.validate().unwrap();

    let rims = obj.analytic_rims();
    assert_eq!(rims.len(), 2, "one claimed cylinder, two rims");
    let bottom = &rims[0];
    let top = &rims[1];
    assert!(
        bottom
            .center
            .approx_eq(Point3::new(0.0, 0.0, 0.0), tol::POINT_MERGE)
    );
    assert!(
        top.center
            .approx_eq(Point3::new(0.0, 0.0, 1.0), tol::POINT_MERGE)
    );

    // The bottom rim survives intact: full coverage, all four quadrants.
    assert!(
        bottom.coverage.is_none(),
        "the un-notched bottom rim still covers the full circle \
         (got {:?})",
        bottom.coverage
    );
    assert_eq!(bottom.quadrant_points().len(), 4);

    // The top rim lost its +X arc (the notch spans |y| <= 0.4 at x > 0.55,
    // roughly +/-23 degrees around +X): the +X quadrant must be refused,
    // while -X and +/-Y stay covered.
    let top_quadrants = top.quadrant_points();
    assert!(
        !top_quadrants
            .iter()
            .any(|q| q.approx_eq(Point3::new(1.0, 0.0, 1.0), 1e-9)),
        "the notched +X arc no longer exists on the top rim; offering a \
         quadrant snap there would snap to empty space"
    );
    assert!(
        top_quadrants
            .iter()
            .any(|q| q.approx_eq(Point3::new(-1.0, 0.0, 1.0), 1e-9)),
        "-X stays covered on the top rim"
    );
    assert!(
        top_quadrants
            .iter()
            .any(|q| q.approx_eq(Point3::new(0.0, 1.0, 1.0), 1e-9)),
        "+Y stays covered on the top rim"
    );
    assert_eq!(top_quadrants.len(), 3);
}

#[test]
fn boolean_trimmed_wall_still_reports_its_rims() {
    // The cut cylinder from the propagation spec: fragments keep claiming
    // the cylinder, so rims (and centers) survive; coverage shrinks to the
    // remaining angular range around the cut.
    let s = circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24);
    let cyl = extrude(&s, 1.0);
    let cutter = {
        let mut cs = Sketch::on_plane(ground());
        for (a, b) in [
            ((0.55, -2.0), (2.0, -2.0)),
            ((2.0, -2.0), (2.0, 2.0)),
            ((2.0, 2.0), (0.55, 2.0)),
            ((0.55, 2.0), (0.55, -2.0)),
        ] {
            cs.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        extrude(&cs, 1.0)
    };
    let obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();
    let rims = obj.analytic_rims();
    assert_eq!(rims.len(), 2);
    // -X is far from the cut (x = 0.55): still covered. The quadrant set
    // includes it.
    assert!(
        rims[0]
            .quadrant_points()
            .iter()
            .any(|q| q.approx_eq(Point3::new(-1.0, 0.0, 0.0), 1e-9)),
        "-X quadrant survives the cut"
    );
}

#[test]
fn slant_cut_rim_offers_no_center() {
    // Slice the ENTIRE top off a cylinder with a slanted plane: every wall
    // facet's top edge slopes, so no boundary edge survives at the new
    // axial extreme — the top "rim" retains zero arc. Its center is the
    // center of no surviving circle (it floats in the air above the slant
    // face at the single highest surviving vertex's station) and must not
    // be offered; the intact bottom rim keeps its center.
    let cyl = extrude(&circle_sketch(Point3::new(0.0, 0.0, 0.0), 1.0, 24), 1.0);
    let cutter = {
        // A quad drawn on the plane z = 0.6 - 0.2x (tilted about Y),
        // extruded along the plane's normal: the prism's bottom face is
        // the slanted cutting plane, crossing the wall between z=0.4 and
        // z=0.8 all the way around.
        let corners = [
            Point3::new(-3.0, -3.0, 1.2),
            Point3::new(3.0, -3.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(-3.0, 3.0, 1.2),
        ];
        let plane = Plane::from_polygon(&corners).unwrap();
        let mut cs = Sketch::on_plane(plane);
        for i in 0..4 {
            cs.add_segment(corners[i], corners[(i + 1) % 4]).unwrap();
        }
        extrude(&cs, 2.0)
    };
    let obj = Object::boolean(
        kernel::BooleanOp::Subtract,
        &cyl,
        &cutter,
        &Transform::IDENTITY,
    )
    .unwrap();
    obj.validate().unwrap();

    let rims = obj.analytic_rims();
    assert_eq!(rims.len(), 2, "the rims themselves are still reported");
    assert!(rims[0].coverage.is_none(), "bottom rim intact");
    assert_eq!(
        rims[1].coverage,
        Some(Vec::new()),
        "top rim has zero surviving arc"
    );
    assert!(rims[1].quadrant_points().is_empty());
    assert!(rims[0].has_coverage());
    assert!(!rims[1].has_coverage());

    // The Center query is gated on surviving coverage: exactly one center,
    // the bottom one.
    let centers = obj.analytic_cap_centers();
    assert_eq!(
        centers.len(),
        1,
        "no Center is offered for a rim with zero surviving arc"
    );
    assert!(
        centers[0]
            .0
            .approx_eq(Point3::new(0.0, 0.0, 0.0), tol::POINT_MERGE)
    );
}

// -------------------------------- imprint → push-through (playtest fix C3)
// the true-curves design, playtest fix C3: a circle drawn on a solid face
// carries its analytic identity onto the imprinted solid edges
// (`Object::split_face_inner_with_curve` → `Edge::curve`), so pushing that
// face THROUGH the solid re-attributes the tunnel walls as
// `SurfaceRef::Cylinder`. Without it the circle dies at the imprint, the
// tunnel walls are bare facets, and a whole-wall push refuses.

/// A watertight axis-aligned box `[-hw,hw] × [-hd,hd] × [0,h]`.
fn box_obj(hw: f64, hd: f64, h: f64) -> Object {
    let mut s = Sketch::on_plane(ground());
    for (a, b) in [
        ((-hw, -hd), (hw, -hd)),
        ((hw, -hd), (hw, hd)),
        ((hw, hd), (-hw, hd)),
        ((-hw, hd), (-hw, -hd)),
    ] {
        s.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
            .unwrap();
    }
    let region = s.regions().keys().next().unwrap();
    Object::from_extrusion(&s.profile(region).unwrap(), h).unwrap()
}

/// The top cap of a box (normal +Z, whole boundary at `z ≈ h`).
fn top_cap(obj: &Object, h: f64) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| {
            f.plane
                .normal()
                .approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION)
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.z - h).abs() <= tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .expect("a +Z top cap at height h")
}

/// A 24-gon circle loop at height `z`, centered at `(cx, cy, z)`, radius `r`.
fn circle_loop(cx: f64, cy: f64, z: f64, r: f64, n: usize) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), z)
        })
        .collect()
}

/// Imprint a circle on the box top, carrying its identity; return
/// `(object, sub_face)`.
fn imprint_circle(hw: f64, hd: f64, h: f64, r: f64) -> (Object, kernel::FaceId) {
    let mut obj = box_obj(hw, hd, h);
    let top = top_cap(&obj, h);
    let loop_pts = circle_loop(0.0, 0.0, h, r, 24);
    let report = obj
        .split_face_inner_with_curve(
            top,
            &loop_pts,
            Some(CurveGeom {
                center: Point3::new(0.0, 0.0, h),
                radius: r,
            }),
        )
        .unwrap();
    (obj, report.sub_face)
}

#[test]
fn imprinted_circle_pushed_through_stamps_the_tunnel_walls() {
    let (h, r) = (1.0, 0.5);
    let (obj, disk) = imprint_circle(2.0, 2.0, h, r);
    obj.validate().unwrap();
    // The flat imprint claims no cylinder yet — the disk is planar.
    assert_eq!(
        surface_census(&obj).1,
        0,
        "a flat imprint claims no cylinder"
    );

    // Push the disk straight down through the whole box.
    let drilled = obj.push_through(disk, -(h + 1.0)).unwrap();
    drilled.validate().unwrap();

    // Every tunnel wall now carries the drawn circle, axis +Z through center.
    let (distinct, carrying) = surface_census(&drilled);
    assert_eq!(carrying, 24, "24 tunnel walls, all attributed");
    assert_eq!(distinct.len(), 1, "one cylinder");
    let SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = distinct[0];
    assert!(
        axis_point.approx_eq(Point3::new(0.0, 0.0, h), tol::POINT_MERGE),
        "axis passes through the drawn center"
    );
    assert!(
        axis.approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION),
        "axis is the push direction"
    );
    assert_eq!(radius, r, "the exact drawn radius, not a chord measure");
}

#[test]
fn tunnel_wall_offsets_the_hole_radius_and_undoes_exactly() {
    let (h, r) = (1.0, 0.5);
    let (obj, disk) = imprint_circle(2.0, 2.0, h, r);
    let mut drilled = obj.push_through(disk, -(h + 1.0)).unwrap();

    // A tunnel wall faces the axis; a positive push adds material (shrinks
    // the hole), exactly like a drilled hole (see `hole_wall_offset_*`).
    let wall = any_wall(&drilled);
    let before: Vec<Point3> = drilled.vertices().values().map(|v| v.position).collect();
    let report = drilled.push_pull(wall, 0.2).unwrap();
    drilled.validate().unwrap();
    let (distinct, _) = surface_census(&drilled);
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert!(
        (radius - 0.3).abs() <= tol::POINT_MERGE,
        "hole radius shrank by the push distance (0.5 → 0.3)"
    );

    // Exact round-trip: push the same wall back out.
    drilled.push_pull(report.face, -0.2).unwrap();
    drilled.validate().unwrap();
    let after: Vec<Point3> = drilled.vertices().values().map(|v| v.position).collect();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert!(
            b.approx_eq(*a, tol::POINT_MERGE),
            "the hole returns to radius 0.5"
        );
    }
}

#[test]
fn shrinking_the_tunnel_past_its_radius_refuses_typed() {
    let (h, r) = (1.0, 0.5);
    let (obj, disk) = imprint_circle(2.0, 2.0, h, r);
    let mut drilled = obj.push_through(disk, -(h + 1.0)).unwrap();
    let wall = any_wall(&drilled);

    // Shrinking by the full radius drives the wall to radius 0.
    let err = drilled.push_pull(wall, r).unwrap_err();
    assert_eq!(err, kernel::PushPullError::RadiusVanishes);
}

#[test]
fn imprint_curve_maps_under_similarity_drops_under_shear() {
    // Similarity (uniform scale ×2): the mapped circle still stamps the
    // tunnel at the scaled radius.
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    obj.apply_transform(&Transform::uniform_scale(2.0)).unwrap();
    obj.validate().unwrap();
    let drilled = obj.push_through(disk, -(2.0 * h + 1.0)).unwrap();
    let (distinct, carrying) = surface_census(&drilled);
    assert_eq!(carrying, 24, "similarity keeps the claim");
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert!((radius - 1.0).abs() <= tol::POINT_MERGE, "radius scaled ×2");

    // Non-similarity (non-uniform scale): the section is an ellipse, so the
    // claim DROPS — the tunnel walls are honest flat facets (map-or-drop).
    let (mut obj2, disk2) = imprint_circle(2.0, 2.0, h, r);
    obj2.apply_transform(&Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    obj2.validate().unwrap();
    let drilled2 = obj2.push_through(disk2, -(h + 1.0)).unwrap();
    assert_eq!(
        surface_census(&drilled2).1,
        0,
        "a non-uniform scale drops the circle claim; walls stay flat"
    );
}

#[test]
fn a_wrong_curve_claim_at_imprint_is_refused_not_repaired() {
    // The caller owns the analytic truth; the kernel never fits a circle to
    // the points. A claim whose radius contradicts the loop is caught by the
    // validator teeth and the imprint refuses (no silent repair).
    let (h, r) = (1.0, 0.5);
    let mut obj = box_obj(2.0, 2.0, h);
    let top = top_cap(&obj, h);
    let loop_pts = circle_loop(0.0, 0.0, h, r, 24);
    let err = obj
        .split_face_inner_with_curve(
            top,
            &loop_pts,
            Some(CurveGeom {
                center: Point3::new(0.0, 0.0, h),
                radius: 0.9, // wrong: the points lie on radius 0.5
            }),
        )
        .unwrap_err();
    assert_eq!(err, kernel::StickyError::CurveClaimOffLoop);
    // Untouched: no imprint happened.
    assert_eq!(obj.faces().len(), 6, "the box is unchanged");
}

#[test]
fn thickening_a_box_after_bossing_an_imprinted_circle_map_or_drops_the_claim() {
    // Map-or-drop for `Edge::curve` on a SUBSET-vertex move (adversarial
    // review, critical). Imprint a circle (rim edges carry the claim), boss it
    // up (the base ring keeps a valid claim), then push the holed top face to
    // thicken the box. Thickening moves the base-ring vertices off the stored
    // circle: the claim must map-or-DROP, never stay stale — a stale claim
    // panicked `check_invariants` in debug and false-refused
    // `NonManifoldResult` in release, a spurious failure of an op unrelated to
    // the circle. A no-curve control (a plain rectangular imprint) already
    // succeeds, isolating the cause to the new field.
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        24,
        "the imprint stamps the 24 rim edges"
    );

    // Boss the disk up: the base ring (shared with the parent hole) keeps its
    // valid claim; the raised top and walls are fresh, unclaimed geometry.
    obj.extrude_sub_face(disk, 0.3).unwrap();
    obj.validate().unwrap();
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        24,
        "the base ring retains its claim after bossing (unmoved)"
    );

    // Thicken: push the holed top face up. This moves the base-ring vertices
    // off the stored circle. Must NOT panic (debug) or refuse (release).
    let top = top_cap(&obj, h);
    obj.push_pull(top, 0.2)
        .expect("thickening an unrelated face must not be refused by a stale claim");
    // Every surviving claim validates against the moved geometry — the moved
    // ring's now-stale claim was dropped (validate would reject a stale one).
    obj.validate().unwrap();
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        0,
        "the moved ring's claim was dropped (map-or-drop), leaving none stale"
    );
}

/// Undoing a boss-wall push keeps `Edge::curve` claims consistent. A boss
/// raised from an imprinted circle now carries `SurfaceRef::Cylinder` on its
/// side walls (playtest Part A), so pushing a boss wall routes to the
/// WHOLE-WALL radial offset (`offset_cylinder_wall`), whose recorded inverse
/// is the ordinary `PushPull{-d}` — not the slanted-neighbor translate-and-
/// build's `UnbuildPushPull`. The base ring carries circle claims that the
/// radial offset moves off their stored circle; the offset's map-or-drop
/// clears them, and undo must leave nothing stale. (Before Part A this same
/// geometry exercised `UnbuildPushPull` — a boss wall was a bare facet whose
/// slanted neighbors built walls; that path's `drop_stale_edge_curves` remains
/// as defensive code, with structural coverage in op_specs' wedge specs.)
#[test]
fn undoing_a_boss_wall_push_keeps_edge_curve_claims_consistent() {
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    obj.extrude_sub_face(disk, 0.3).unwrap();
    obj.validate().unwrap();

    // A boss side wall: a facet whose vertices all sit near the axis (radius
    // ~r, not the box walls at hw = 2.0) and span the boss height (some above
    // the base plane z = h), with a roughly radial (horizontal) normal.
    let wall = obj
        .faces()
        .iter()
        .find(|(_, f)| {
            f.plane.normal().z.abs() < 0.3
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (p.x * p.x + p.y * p.y).sqrt() < r + 0.1)
                && obj
                    .loop_positions(f.outer_loop)
                    .any(|p| p.z > h + tol::POINT_MERGE)
        })
        .map(|(id, _)| id)
        .expect("the boss has radial side-wall facets");

    let mut history = kernel::History::new();
    let report = history
        .apply(
            &mut obj,
            kernel::KernelOp::PushPull {
                face: wall,
                distance: 0.1,
            },
        )
        .expect("pushing a boss side wall outward succeeds");
    let kernel::KernelOpReport::PushPull(pp) = &report else {
        panic!("push_pull yields a PushPull report");
    };
    assert!(
        !pp.requires_unbuild_inverse,
        "a stamped boss wall push is a radial offset, inverted by PushPull{{-d}}"
    );
    obj.validate().unwrap();

    // Undo dispatches the ordinary PushPull inverse; it must succeed and leave
    // no stale claim (validate rejects a stale one).
    history
        .undo(&mut obj)
        .expect("undo of the boss-wall push succeeds");
    obj.validate()
        .expect("no stale Edge::curve claim survives the inverse offset");
}

// -------------------------------------------- boss stamping (playtest Part A)
// the true-curves design §4.6, clause `extrude_sub_face`: the pull-UP
// mirror of the push-THROUGH tunnel stamping (C3). Bossing a sub-face raised
// from an imprinted circle stamps its side walls `SurfaceRef::Cylinder`, so
// the boss shades smooth and a wall push offsets its radius. A mixed/partial
// boundary loop stamps nothing (map-or-drop). Red-check: disabling the stamp
// drops `carrying` to 0 (the raised disk cap and the box faces carry none).

#[test]
fn bossing_an_imprinted_circle_stamps_the_boss_walls() {
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    // The flat imprint claims no cylinder; the box has 6 faces, none attributed.
    assert_eq!(
        surface_census(&obj).1,
        0,
        "a flat imprint claims no cylinder"
    );

    // Boss the disk UP into a cylinder standing on the box top.
    obj.extrude_sub_face(disk, 0.3).unwrap();
    obj.validate().unwrap();

    // Every one of the 24 boss side walls carries the drawn circle, axis +Z
    // through the center, radius the exact drawn radius.
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(carrying, 24, "24 boss walls, all attributed");
    assert_eq!(distinct.len(), 1, "one cylinder");
    let SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = distinct[0];
    assert!(
        axis_point.approx_eq(Point3::new(0.0, 0.0, h), tol::POINT_MERGE),
        "axis passes through the drawn center on the base plane"
    );
    assert!(
        axis.approx_eq(Vec3::new(0.0, 0.0, 1.0), tol::NORMAL_DIRECTION),
        "axis is the sweep (pull-up) direction"
    );
    assert_eq!(radius, r, "the exact drawn radius, not a chord measure");
}

#[test]
fn bossed_wall_push_offsets_the_radius_not_a_single_facet() {
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    obj.extrude_sub_face(disk, 0.3).unwrap();

    // A boss wall faces AWAY from the axis; a positive push adds material and
    // grows the radius (unlike a hole wall, which shrinks). The whole logical
    // wall moves — all 24 facets share the new radius — not a single facet.
    let wall = any_wall(&obj);
    let before: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let report = obj.push_pull(wall, 0.1).unwrap();
    obj.validate().unwrap();
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(
        carrying, 24,
        "still 24 attributed walls — no facet was split off"
    );
    assert_eq!(distinct.len(), 1, "still one cylinder");
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert!(
        (radius - 0.6).abs() <= tol::POINT_MERGE,
        "the boss radius grew by the push distance (0.5 → 0.6)"
    );

    // Exact round-trip: push the same wall back in.
    obj.push_pull(report.face, -0.1).unwrap();
    obj.validate().unwrap();
    let after: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(&after) {
        assert!(
            b.approx_eq(*a, tol::POINT_MERGE),
            "the boss returns to radius 0.5"
        );
    }
}

#[test]
fn shrinking_a_boss_wall_past_its_radius_refuses_typed() {
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    obj.extrude_sub_face(disk, 0.3).unwrap();
    let wall = any_wall(&obj);

    // Shrinking by the full radius drives the wall to radius 0.
    let err = obj.push_pull(wall, -r).unwrap_err();
    assert_eq!(err, kernel::PushPullError::RadiusVanishes);
}

#[test]
fn bossing_a_rectangular_imprint_stays_flat() {
    // A mixed/partial loop stamps nothing — a rectangle's sides are on no one
    // circle, so `split_face_inner` leaves the sub-face's boundary edges with
    // no `Edge::curve` claim and the boss walls stay honest flat facets.
    let h = 1.0;
    let mut obj = box_obj(2.0, 2.0, h);
    let top = top_cap(&obj, h);
    let rect = [
        Point3::new(-0.4, -0.3, h),
        Point3::new(0.4, -0.3, h),
        Point3::new(0.4, 0.3, h),
        Point3::new(-0.4, 0.3, h),
    ];
    let report = obj.split_face_inner(top, &rect).unwrap();
    obj.extrude_sub_face(report.sub_face, 0.3).unwrap();
    obj.validate().unwrap();
    assert_eq!(
        surface_census(&obj).1,
        0,
        "a rectangular boss has no analytic circle to stamp"
    );
}

#[test]
fn bossing_an_arc_closed_by_a_chord_stays_flat() {
    // Adversarial (review F2/F6): a loop of 20 short arc-chords (0→300°) plus
    // ONE long straight closing chord (300°→360°, a 60° secant) — every vertex
    // lies on the circle, so `split_face_inner_with_curve` stamps the SAME
    // curve on all 21 edges and the weak "every edge carries a matching curve"
    // test would pass. But the closing secant is a flat wall, not a cylinder
    // facet: the ring is non-uniform (its steps are 20×15° + 1×60°), so the
    // strengthened full-circle-ring check must REFUSE to stamp (map-or-drop —
    // stamp-wrong is worse than don't-stamp). Red-check: drop the uniformity
    // test and this wrongly stamps 21 cylinder walls.
    let (h, r) = (1.0, 0.5);
    let mut obj = box_obj(2.0, 2.0, h);
    let top = top_cap(&obj, h);
    // 21 vertices at 0,15,…,300°, all on the circle; the loop closes 300°→0°.
    let loop_pts: Vec<Point3> = (0..21)
        .map(|i| {
            let a = std::f64::consts::PI / 12.0 * (i as f64); // 15° steps
            Point3::new(r * a.cos(), r * a.sin(), h)
        })
        .collect();
    let disk = obj
        .split_face_inner_with_curve(
            top,
            &loop_pts,
            Some(CurveGeom {
                center: Point3::new(0.0, 0.0, h),
                radius: r,
            }),
        )
        .unwrap()
        .sub_face;
    // The imprint DID stamp every edge (all endpoints are on the circle)…
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        21,
        "the imprint stamps all 21 arc+chord edges (endpoints on the circle)"
    );
    // …but bossing must NOT sweep the secant into a cylinder wall.
    obj.extrude_sub_face(disk, 0.3).unwrap();
    obj.validate().unwrap();
    assert_eq!(
        surface_census(&obj).1,
        0,
        "an arc closed by a straight chord is not a full circle — no stamp"
    );
}

/// Imprint an `n`-point concyclic loop (uniform, radius `r` at z=h, centered
/// at origin) carrying one shared circle claim, boss it up, and return the
/// object plus how many boss walls carry a surface.
fn boss_concyclic(n: usize, r: f64, h: f64) -> (Object, usize) {
    let mut obj = box_obj(2.0, 2.0, h);
    let top = top_cap(&obj, h);
    let disk = obj
        .split_face_inner_with_curve(
            top,
            &circle_loop(0.0, 0.0, h, r, n),
            Some(CurveGeom {
                center: Point3::new(0.0, 0.0, h),
                radius: r,
            }),
        )
        .unwrap()
        .sub_face;
    // Every edge carries the shared claim (all vertices lie on the circle).
    assert_eq!(
        obj.edges().values().filter(|e| e.curve.is_some()).count(),
        n,
        "the imprint stamps all {n} concyclic edges"
    );
    obj.extrude_sub_face(disk, 0.3).unwrap();
    let carrying = surface_census(&obj).1;
    (obj, carrying)
}

#[test]
fn bossing_an_equilateral_triangle_on_a_circle_stays_flat() {
    // Adversarial re-review (major): the uniformity test alone passes ANY
    // regular n-gon (every step is exactly 2π/n), so a coarse but uniform ring
    // slips through. An equilateral triangle — 3 concyclic points, 120° facets
    // — carries the SAME claim on all 3 edges (validate even passes: a secant
    // wall's plane is parallel to the axis and within radius) yet its walls are
    // flat secants, not cylinder facets. The absolute density floor
    // (`MIN_CIRCLE_SEGMENTS`) must reject it. Red-check: drop the density gate
    // and this stamps 3 cylinder walls.
    let (obj, carrying) = boss_concyclic(3, 0.5, 1.0);
    obj.validate().unwrap();
    assert_eq!(
        carrying, 0,
        "a triangle inscribed in a circle is not a circle — no stamp"
    );
}

#[test]
fn bossing_a_skip_connected_12gon_stays_flat() {
    // The same break via a homogeneous COARSE ring at a higher count: 12
    // concyclic points at 30° steps (every other vertex of a 24-gon). Uniform,
    // so only the density floor catches it. Red-check: drop the density gate
    // and this stamps 12 secant walls.
    let (obj, carrying) = boss_concyclic(12, 0.5, 1.0);
    obj.validate().unwrap();
    assert_eq!(
        carrying, 0,
        "a 30°-facet 12-gon is too coarse to be a circle — no stamp"
    );
}

#[test]
fn bossing_a_48gon_circle_stamps_every_facet() {
    // The floor accepts the tool's 24-segment minimum AND finer adaptive
    // counts: a 48-gon (7.5° facets) is a genuine circle and stamps every wall.
    let (obj, carrying) = boss_concyclic(48, 0.5, 1.0);
    obj.validate().unwrap();
    assert_eq!(
        carrying, 48,
        "a 48-gon (finer than the 24 floor) stamps all facets"
    );
}

/// A boss whose wall was then whole-wall offset must survive a full History
/// unwind/replay (DEVELOPMENT.md rule 9). The boss stamps `Face::surface`
/// from `Edge::curve`, which the offset drops; without restoring the surface
/// on rule-9 alignment, the re-done boss comes back a bare facet, the re-done
/// offset reroutes to translate-and-build, and replay DIVERGES. Red-check:
/// drop the `surface` restore in `StateProof::verify_and_align` and the redo
/// diverges here.
#[test]
fn a_bossed_wall_offset_round_trips_through_history() {
    use kernel::{History, KernelOp};
    let (h, r) = (1.0, 0.5);
    let (mut obj, disk) = imprint_circle(2.0, 2.0, h, r);
    let mut history = History::new();

    history
        .apply(
            &mut obj,
            KernelOp::ExtrudeSubFace {
                sub_face: disk,
                distance: 0.3,
            },
        )
        .expect("boss");
    let wall = any_wall(&obj);
    history
        .apply(
            &mut obj,
            KernelOp::PushPull {
                face: wall,
                distance: 0.1,
            },
        )
        .expect("offset the boss wall");

    // Full unwind, then full replay — every undo and redo must succeed
    // (no InverseDiverged), and the replayed boss wall must again carry a
    // cylinder (its surface was restored on alignment, not left bare).
    let mut n = 0;
    while history.can_undo() {
        history
            .undo(&mut obj)
            .unwrap_or_else(|e| panic!("undo #{n}: {e}"));
        n += 1;
    }
    n = 0;
    while history.can_redo() {
        history
            .redo(&mut obj)
            .unwrap_or_else(|e| panic!("redo #{n}: {e}"));
        n += 1;
    }
    obj.validate().unwrap();
    let (distinct, carrying) = surface_census(&obj);
    assert_eq!(
        carrying, 24,
        "the replayed boss walls carry the cylinder again"
    );
    let SurfaceRef::Cylinder { radius, .. } = distinct[0];
    assert!(
        (radius - 0.6).abs() <= tol::POINT_MERGE,
        "and at the offset radius (0.5 → 0.6)"
    );
}
