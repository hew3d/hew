//! Executable specs for a circle drawn on a solid's face that touches or
//! crosses the face's OWN edges — the solid's boundary, where the face
//! meets a side face (GitHub #43). A circle on a face goes through the
//! drafting engine ([`Document::draft`]) exactly as a rectangle drawn up to
//! an edge does: the circle is arranged against every edge of the plane's
//! drawing, the part over the face becomes face topology, the part past the
//! edge a free sketch, and the whole draw is one undo step.
//!
//! The four cases, and what each does:
//!
//! - **Crossing one edge.** The face splits along the circle's arc into the
//!   lens inside and the rest; the crescent past the edge is a free sketch
//!   region closed by the solid's edge. Every whole facet of the circle
//!   that lands on the face keeps its analytic claim; a facet the edge cuts
//!   in two keeps it on neither piece, because the cut point is not on the
//!   circle (map-or-drop, never a claim that lies). This holds with the
//!   crossing mid-facet and with a facet vertex exactly on the edge (the
//!   Circle tool's rim snapped On Edge).
//! - **Crossing a corner**, and **through a corner vertex**: the same, with
//!   the face's corner on the lens's boundary.
//! - **Tangent at one point inside a square face.** The circle touches the
//!   face's edge at a single facet vertex. The disc becomes a face, and the
//!   face around it holds it as a hole sharing that one vertex with its
//!   outer loop: every loop stays simple, and loops of one face may share
//!   isolated vertices, never an edge (ARCHITECTURE §2.4). Every facet keeps
//!   its claim, the disc's region names its face (so Push/Pull pushes it),
//!   and the draw is one undo step. The tolerance decides the touch: a
//!   vertex within `tol::POINT_MERGE` of the edge is on it (a touching
//!   hole), one clearly inside is not (an ordinary hole), and either way the
//!   drawn circle is never nudged.
//! - **Tangent at a vertex.** A circle inside an L-shaped face touching its
//!   reflex corner at one point is a disc in a hole touching that corner the
//!   same way; a circle outside the face touching only its corner lands as
//!   a free sketch circle and leaves the solid untouched.
//!
//! The property test draws a random circle on the box top (a lattice that
//! makes tangencies, crossings, and corners common) and checks: the draft
//! never refuses, the solid validates and stays watertight, the up-facing
//! faces' areas sum to the original top, every loop is simple, every
//! claimed solid edge is a whole facet of the circle and every whole facet
//! that became solid topology carries the claim, and the draw is one exact
//! undo step that reopens byte-identical.

use kernel::topo::WatertightState;
use kernel::{CurveGeom, Document, Object, Point3, Vec3, tol};
use proptest::prelude::*;

use crate::face_arrangement_support;
use face_arrangement_support::*;

const FACETS: usize = 24;
/// The top of `top_of_box`, in mm.
const SIDE: f64 = 30.0;

fn top_area_mm2(w: &Wall) -> f64 {
    group_area(w.obj(), &w.plane) / (MM * MM)
}

/// A circle of `FACETS` facets about `(cx, cy)` mm with radius `r` mm,
/// vertex 0 at angle `phase`.
fn circle_mm(w: &Wall, cx: f64, cy: f64, r: f64, phase: f64) -> (Vec<Point3>, CurveGeom) {
    (
        w.ring(cx * MM, cy * MM, r * MM, FACETS, phase),
        w.circle_geom(cx * MM, cy * MM, r * MM),
    )
}

/// Whether a point (in the wall's frame) lies on the closed `0..side` mm
/// square, within the merge tolerance.
fn on_square(w: &Wall, p: Point3, side: f64) -> bool {
    let d = p - w.origin;
    let (x, y) = (d.dot(w.u), d.dot(w.v));
    let lim = side * MM + tol::POINT_MERGE;
    x >= -tol::POINT_MERGE && y >= -tol::POINT_MERGE && x <= lim && y <= lim
}

/// The circle's facets with both ends on the square: the facets that land
/// on the face whole.
fn whole_facets_on(w: &Wall, ring: &[Point3], side: f64) -> usize {
    (0..ring.len())
        .filter(|&i| on_square(w, ring[i], side) && on_square(w, ring[(i + 1) % ring.len()], side))
        .count()
}

/// Every solid edge carrying a claim carries `g` and joins two consecutive
/// ring vertices; every solid edge joining two consecutive ring vertices
/// carries `g`. Returns how many edges carry it.
fn assert_claims_are_whole_facets(obj: &Object, ring: &[Point3], g: CurveGeom) -> usize {
    let n = ring.len();
    let facet_of = |a: Point3, b: Point3| {
        (0..n).any(|i| {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            (a.approx_eq(p, tol::POINT_MERGE) && b.approx_eq(q, tol::POINT_MERGE))
                || (a.approx_eq(q, tol::POINT_MERGE) && b.approx_eq(p, tol::POINT_MERGE))
        })
    };
    let mut claimed = 0;
    for (e, edge) in obj.edges() {
        let (a, b) = obj.edge_endpoints(e).expect("live edge");
        match edge.curve {
            Some(c) => {
                assert!(
                    c.center.approx_eq(g.center, tol::POINT_MERGE)
                        && (c.radius - g.radius).abs() < 1e-15,
                    "a claim other than the drawn circle: {c:?}"
                );
                assert!(facet_of(a, b), "a claimed edge that is no whole facet");
                claimed += 1;
            }
            None => assert!(
                !facet_of(a, b),
                "a whole facet landed as topology without its claim"
            ),
        }
    }
    claimed
}

/// The drawn region whose outer ring is `ring` on the wall's plane.
fn region_of<'a>(
    units: &'a kernel::PlaneUnits,
    ring: &[Point3],
    n: Vec3,
) -> &'a kernel::DraftRegion {
    let want = without_straight(ring, n);
    units
        .regions
        .iter()
        .find(|r| cyclic_match(&without_straight(&r.outer, n), &want, true))
        .expect("the circle's region")
}

/// Free sketches (owned by no object) holding drawing.
fn free_sketches(doc: &Document) -> Vec<kernel::SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch_owner_object(s).is_none())
        .filter(|&s| !doc.sketch(s).unwrap().edges().is_empty())
        .collect()
}

/// Whether every edge of sketch `s` is a facet of the circle `g`.
fn sketch_keeps_circle(doc: &Document, s: kernel::SketchId, g: CurveGeom) -> bool {
    let sk = doc.sketch(s).unwrap();
    sk.edges().values().all(|e| {
        e.curve.and_then(|c| sk.curve_geom(c)).is_some_and(|c| {
            c.center.approx_eq(g.center, tol::POINT_MERGE) && (c.radius - g.radius).abs() < 1e-15
        })
    })
}

/// Draws a crossing circle through one draft, checks I1–I5 and I7, and
/// that the drawing split the face (lens + rest) with the claim on every
/// whole facet over the face and the part past the edge in one free sketch
/// as `crescents` regions.
fn draw_crossing(w: &mut Wall, ring: Vec<Point3>, g: CurveGeom, crescents: usize) {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let seed = w.face;
    let (report, _) = w.draw_loop(seed, &ring, Some(g)).expect("never refused");
    assert!(
        report.kept_as_sketch.is_empty(),
        "nothing kept as owned drawing"
    );
    assert_invariants(w, &before, &bytes0, depth0);
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 2, "the lens and the rest");
    assert!(
        (top_area_mm2(w) - SIDE * SIDE).abs() < 1e-9,
        "areas sum to the top"
    );
    let whole = whole_facets_on(w, &ring, SIDE);
    assert!(whole > 0);
    assert_eq!(
        assert_claims_are_whole_facets(obj, &ring, g),
        whole,
        "every whole facet over the face keeps the circle"
    );
    assert!(
        w.doc.object_owned_sketches(w.id).is_empty(),
        "no owned drawing"
    );
    let free = free_sketches(&w.doc);
    assert_eq!(free.len(), 1, "the overhang lives in one free sketch");
    let sk = w.doc.sketch(free[0]).unwrap();
    assert_eq!(
        sk.regions().len(),
        crescents,
        "closed there by the solid's edge"
    );
    assert!(
        sk.edges().values().any(|e| e.curve.is_some()),
        "the crescent's arc keeps its circle"
    );
    // The lens pushes: its region names a face.
    let units = units_on(&w.doc, None, w.id, w.plane);
    assert!(
        units
            .regions
            .iter()
            .any(|r| matches!(r.over, Some((o, Some(_))) if o == w.id)
                && r.outer
                    .iter()
                    .any(|p| ring.iter().any(|c| c.approx_eq(*p, tol::POINT_MERGE)))),
        "a region on the circle is a face of the solid"
    );
}

// ------------------------------------------------------------ crossing

#[test]
fn circle_crossing_one_edge_mid_facet_splits_the_face_and_keeps_its_claims() {
    let mut w = top_of_box();
    // Center 3 mm in from the bottom edge, radius 5 mm: 7 vertices past
    // the edge, the crossings mid-facet.
    let (ring, g) = circle_mm(&w, 15.0, 3.0, 5.0, 0.0);
    draw_crossing(&mut w, ring.clone(), g, 1);
    assert_eq!(whole_facets_on(&w, &ring, SIDE), 16);
}

#[test]
fn circle_with_its_rim_on_the_edge_crossing_it_splits_the_face_and_keeps_its_claims() {
    let mut w = top_of_box();
    // The Circle tool's rim snapped On Edge at (19, 0) about (15, 3): a
    // 3-4-5 triangle, so the radius is 5 mm and vertex 0 is the rim. The
    // circle crosses the edge at the rim vertex and again mid-facet.
    let phase = (-3.0f64).atan2(4.0);
    let (mut ring, g) = circle_mm(&w, 15.0, 3.0, 5.0, phase);
    assert!((ring[0] - w.at(19.0 * MM, 0.0)).length() < 1e-15);
    ring[0] = w.at(19.0 * MM, 0.0);
    draw_crossing(&mut w, ring.clone(), g, 1);
    assert_eq!(whole_facets_on(&w, &ring, SIDE), 16);
    // The face's edge is split at the rim.
    let obj = w.obj();
    assert!(
        group_faces(obj, &w.plane).into_iter().any(|f| {
            obj.loop_positions(obj.faces()[f].outer_loop)
                .any(|p| p.approx_eq(ring[0], tol::POINT_MERGE))
        }),
        "the rim vertex is on the face's boundary"
    );
}

#[test]
fn circle_crossing_a_corner_splits_the_face_and_keeps_its_claims() {
    let mut w = top_of_box();
    let (ring, g) = circle_mm(&w, 2.0, 3.0, 5.0, 0.0);
    draw_crossing(&mut w, ring, g, 1);
    // The lens holds the face's corner.
    let corner = w.at(0.0, 0.0);
    let obj = w.obj();
    let lens = group_faces(obj, &w.plane)
        .into_iter()
        .filter(|&f| {
            obj.loop_positions(obj.faces()[f].outer_loop)
                .any(|p| p.approx_eq(corner, tol::POINT_MERGE))
        })
        .count();
    assert_eq!(lens, 1, "the corner is on the lens only");
}

#[test]
fn circle_through_a_corner_vertex_splits_the_face_and_keeps_its_claims() {
    let mut w = top_of_box();
    // Vertex 0 exactly on the face's corner (0, 0), the center inside: the
    // circle leaves the face through the corner and comes back across both
    // sides.
    let r = 5.0 * 2f64.sqrt();
    let (mut ring, g) = circle_mm(&w, 5.0, 5.0, r, 1.25 * std::f64::consts::PI);
    assert!((ring[0] - w.at(0.0, 0.0)).length() < 1e-15);
    ring[0] = w.at(0.0, 0.0);
    // Past the edge: two crescents, one beyond each side, meeting only at
    // the corner.
    draw_crossing(&mut w, ring, g, 2);
}

// ------------------------------------------------------------- tangent

/// Draws a circle touching the face's boundary at a single point and checks
/// I1–I7: the disc is a face with every facet's claim, the face around it
/// holds a hole sharing the touch point with its outer loop, nothing stays
/// sketch drawing, and every region of the plane names a face (none says
/// why it cannot push).
fn draw_tangent_face(w: &mut Wall, ring: Vec<Point3>, g: CurveGeom) {
    let faces_before = group_faces(w.obj(), &w.plane).len();
    let area_before = top_area_mm2(w);
    let seed = w.face;
    draw_and_check(w, seed, ring.clone(), Some(g));
    let obj = w.obj();
    assert_eq!(
        group_faces(obj, &w.plane).len(),
        faces_before + 1,
        "the disc is a face"
    );
    assert!(has_touching_loops(obj, &w.plane), "in a touching hole");
    assert!((top_area_mm2(w) - area_before).abs() < 1e-9);
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert_eq!(
        assert_claims_are_whole_facets(obj, &ring, g),
        FACETS,
        "every facet keeps the circle"
    );
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
    assert!(free_sketches(&w.doc).is_empty());
    let units = units_on(&w.doc, None, w.id, w.plane);
    let disc = region_of(&units, &ring, w.plane.normal());
    assert!(
        matches!(disc.over, Some((o, Some(_))) if o == w.id),
        "the disc's region is a face of the solid"
    );
    for r in &units.regions {
        assert!(
            matches!(r.over, Some((_, Some(_)))) && r.why.is_none(),
            "every region on the plane is a face"
        );
    }
}

#[test]
fn tangent_circle_inside_a_square_face_becomes_a_face_in_a_touching_hole() {
    let mut w = top_of_box();
    // Radius 5 mm, 5 mm above the bottom edge: vertex 18 (270 degrees)
    // sits exactly on the edge, every other point inside.
    let (ring, g) = circle_mm(&w, 15.0, 5.0, 5.0, 0.0);
    assert!((ring[18] - w.at(15.0 * MM, 0.0)).length() < 1e-15);
    draw_tangent_face(&mut w, ring, g);
}

#[test]
fn tangent_within_the_merge_tolerance_is_on_the_edge_and_clear_of_it_is_inside() {
    // Half the merge tolerance inside: the tangent vertex is ON the edge
    // (a touching hole).
    let mut w = top_of_box();
    let (ring, g) = circle_mm(&w, 15.0, 5.0 + 0.5 * tol::POINT_MERGE / MM, 5.0, 0.0);
    draw_tangent_face(&mut w, ring, g);

    // Ten times the merge tolerance inside: the circle is clear of the edge
    // and imprints as a sub-face in an ordinary hole, every facet keeping
    // its claim.
    let mut w = top_of_box();
    let (ring, g) = circle_mm(&w, 15.0, 5.0 + 10.0 * tol::POINT_MERGE / MM, 5.0, 0.0);
    let seed = w.face;
    draw_and_check(&mut w, seed, ring.clone(), Some(g));
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 2, "the disc and the top");
    assert_eq!(assert_claims_are_whole_facets(obj, &ring, g), FACETS);
}

#[test]
fn tangent_circle_on_a_tilted_face_becomes_a_face_in_a_touching_hole() {
    let mut w = tilted_wall();
    // The tilted cap is 20 x 10 mm; tangent to its v = 0 edge.
    let (cx, cy, r) = (10.0 * MM, 3.0 * MM, 3.0 * MM);
    let ring = w.ring(cx, cy, r, FACETS, 0.0);
    let g = w.circle_geom(cx, cy, r);
    draw_tangent_face(&mut w, ring, g);
}

// ------------------------------------------------------ tangent at a vertex

/// An L-shaped prism 10 mm tall, drawing on its top: the outline
/// (0,0) (30,0) (30,10) (10,10) (10,30) (0,30) mm, reflex corner (10, 10).
fn top_of_l() -> Wall {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let pts: Vec<Point3> = [
        (0., 0.),
        (30., 0.),
        (30., 10.),
        (10., 10.),
        (10., 30.),
        (0., 30.),
    ]
    .iter()
    .map(|&(x, y)| Point3::new(x * MM, y * MM, 0.0))
    .collect();
    {
        let sk = doc.sketch_mut(s).unwrap();
        for i in 0..pts.len() {
            sk.add_segment(pts[i], pts[(i + 1) % pts.len()]).unwrap();
        }
    }
    let regions = doc.extrudable_regions(s).unwrap();
    let (id, _) = doc.extrude_region(s, regions[0], 10.0 * MM).unwrap();
    let obj = doc.object(id).unwrap();
    let up = Vec3::new(0.0, 0.0, 1.0);
    let face = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(up, 1e-9))
        .map(|(f, _)| f)
        .unwrap();
    let plane = obj.faces()[face].plane;
    let (u, v) = plane_basis(&plane);
    Wall {
        doc,
        id,
        face,
        plane,
        origin: Point3::new(0.0, 0.0, 10.0 * MM),
        u,
        v,
    }
}

#[test]
fn circle_tangent_at_a_reflex_corner_becomes_a_face_in_a_touching_hole() {
    let mut w = top_of_l();
    // About (7, 6) with radius 5: vertex 0 on the reflex corner (10, 10),
    // the rest inside the L.
    let phase = 4.0f64.atan2(3.0);
    let (mut ring, g) = circle_mm(&w, 7.0, 6.0, 5.0, phase);
    assert!((ring[0] - w.at(10.0 * MM, 10.0 * MM)).length() < 1e-15);
    ring[0] = w.at(10.0 * MM, 10.0 * MM);
    draw_tangent_face(&mut w, ring, g);
}

#[test]
fn circle_outside_touching_only_a_corner_is_a_free_circle_and_the_solid_is_untouched() {
    let mut w = top_of_box();
    // About (-3, -4) with radius 5: vertex 0 on the corner (0, 0), the rest
    // outside the face.
    let phase = 4.0f64.atan2(3.0);
    let (mut ring, g) = circle_mm(&w, -3.0, -4.0, 5.0, phase);
    assert!((ring[0] - w.at(0.0, 0.0)).length() < 1e-15);
    ring[0] = w.at(0.0, 0.0);
    let before = w.obj().clone();
    let depth0 = w.doc.undo_depth();
    let seed = w.face;
    let (report, _) = w.draw_loop(seed, &ring, Some(g)).expect("never refused");
    assert!(report.kept_as_sketch.is_empty());
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo step");
    assert!(
        objects_equivalent(w.obj(), &before),
        "the solid is untouched"
    );
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
    let free = free_sketches(&w.doc);
    assert_eq!(free.len(), 1);
    assert_eq!(w.doc.sketch(free[0]).unwrap().edges().len(), FACETS);
    assert!(sketch_keeps_circle(&w.doc, free[0], g));
    assert_eq!(
        w.doc.sketch(free[0]).unwrap().regions().len(),
        1,
        "a disc to extrude"
    );
}

// ------------------------------------------------------------ property

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// A random circle on the 30 mm box top, center and radius on a 0.5 mm
    /// lattice, so its vertices at 0/90/180/270 degrees often land exactly
    /// on an edge (tangent), past it (crossing), or on a corner.
    #[test]
    fn a_circle_touching_or_crossing_the_face_edge_keeps_the_solid_sound(
        cx in 0u32..=60, cy in 0u32..=60, r in 2u32..=24,
    ) {
        let half = |k: u32| f64::from(k) * 0.5;
        let mut w = top_of_box();
        let (ring, g) = circle_mm(&w, half(cx), half(cy), half(r), 0.0);
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let seed = w.face;
        let res = w.draw_loop(seed, &ring, Some(g));
        prop_assert!(res.is_ok(), "a draft refused: {:?}", res.err());
        let obj = w.obj();
        obj.validate().expect("valid after the draft");
        prop_assert_eq!(obj.watertight(), WatertightState::Watertight);
        prop_assert!(
            (top_area_mm2(&w) - SIDE * SIDE).abs() < 1e-9,
            "face areas sum to the original top: {}", top_area_mm2(&w)
        );
        prop_assert!(no_pinch(obj, &w.plane), "every loop is simple");
        assert_claims_are_whole_facets(obj, &ring, g);
        // Whatever stays drawing keeps the circle.
        for s in w.doc.sketch_ids() {
            let sk = w.doc.sketch(s).unwrap();
            prop_assert!(
                sk.edges().values().all(|e| e.curve.is_none()
                    || e.curve.and_then(|c| sk.curve_geom(c)).is_some_and(|c|
                        c.center.approx_eq(g.center, tol::POINT_MERGE))),
                "a sketch arc lost its circle"
            );
        }
        prop_assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo step");
        let after = w.doc.save();
        prop_assert_eq!(Document::load(&after).expect("reopens").save(), after.clone());
        w.doc.undo().unwrap();
        prop_assert_eq!(w.doc.save(), bytes0, "undo exact");
        w.doc.redo().unwrap();
        prop_assert_eq!(w.doc.save(), after, "redo exact");
    }
}

#[test]
fn the_lattice_reaches_every_case() {
    // The property's lattice is not vacuous: tangent, crossing, and corner
    // circles are all on it. (15, 5) r 5 is tangent; (15, 3) r 5 crosses;
    // (2, 3) r 5 crosses a corner.
    let w = top_of_box();
    let tangent = circle_mm(&w, 15.0, 5.0, 5.0, 0.0).0;
    assert_eq!(whole_facets_on(&w, &tangent, SIDE), FACETS);
    assert!(
        tangent
            .iter()
            .any(|&p| (p - w.origin).dot(w.v).abs() < 1e-15)
    );
    let crossing = circle_mm(&w, 15.0, 3.0, 5.0, 0.0).0;
    assert!(whole_facets_on(&w, &crossing, SIDE) < FACETS);
    let corner = circle_mm(&w, 2.0, 3.0, 5.0, 0.0).0;
    assert!(whole_facets_on(&w, &corner, SIDE) < FACETS);
}
