//! Executable specs for detached shapes (`Sketch::detached`): a free sketch
//! that takes part in no plane arrangement. It is its own drafting context
//! (`Document::draft_detached` / `draft_detached_new`): nothing drawn later
//! on its plane joins it, it joins nothing when moved, and it never becomes
//! face topology. Extruding one of its regions always makes a new Object.
//!
//! The contract under test:
//! - a plane lift ignores detached sketches, so plain drawing over one
//!   lowers exactly as if it were absent, and the detached sketch is
//!   untouched (`==`, handles included);
//! - a detached draft lifts its one sketch alone and lowers everything back
//!   into it (or into ONE new detached sketch), arranging only with itself;
//! - every rebuild, copy, detach, and bake carries the flag; undo and redo
//!   restore it byte-exactly;
//! - a detached sketch is never object-owned (I9), and the listing names it
//!   as its own entry.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use kernel::{
    CurveGeom, Document, DocumentError, DraftStore, ObjectId, Plane, PlaneListing, Point3, Sketch,
    SketchError, SketchId, Transform, Vec3, WatertightState,
};
use proptest::prelude::*;

// ------------------------------------------------------------ fixtures

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn up() -> Vec3 {
    Vec3::new(0.0, 0.0, 1.0)
}

/// The box top (z = 3).
fn top() -> Plane {
    Plane::from_point_normal(p(0., 0., 3.), up()).unwrap()
}

fn ground_plane() -> Plane {
    Plane::from_point_normal(p(0., 0., 0.), up()).unwrap()
}

/// A 10 x 6 x 3 box on the ground; its drawing sketch is consumed.
fn boxdoc() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground_plane());
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in [
        (p(0., 0., 0.), p(10., 0., 0.)),
        (p(10., 0., 0.), p(10., 6., 0.)),
        (p(10., 6., 0.), p(0., 6., 0.)),
        (p(0., 6., 0.), p(0., 0., 0.)),
    ] {
        sk.add_segment(a, b).unwrap();
    }
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (o, _) = doc.extrude_region(s, r, 3.0).unwrap();
    (doc, o)
}

fn shift(dx: f64, dy: f64) -> Transform {
    Transform::translation(Vec3::new(dx, dy, 0.0))
}

/// A closed ring of segments.
fn draw_ring(w: &mut Sketch, ring: &[Point3]) -> Result<(), SketchError> {
    for i in 0..ring.len() {
        w.add_segment(ring[i], ring[(i + 1) % ring.len()])?;
    }
    Ok(())
}

/// An axis-aligned rectangle at height `z`.
fn rect(w: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> Result<(), SketchError> {
    draw_ring(w, &[p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)])
}

/// `n` facets of the horizontal circle about `c`, carrying the exact circle.
fn circle(w: &mut Sketch, c: Point3, r: f64, n: usize) -> Result<(), SketchError> {
    w.begin_curve_with(CurveGeom {
        center: c,
        radius: r,
    })?;
    let ring: Vec<Point3> = (0..n)
        .map(|i| {
            let a = TAU * i as f64 / n as f64;
            p(c.x + r * a.cos(), c.y + r * a.sin(), c.z)
        })
        .collect();
    let drawn = draw_ring(w, &ring);
    w.end_curve();
    drawn
}

/// The live world sketches carrying the detached flag.
fn detached_sketches(doc: &Document) -> Vec<SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch(s).unwrap().detached())
        .collect()
}

/// A slot-order-independent print of an object's faces: each face's rings
/// as sorted rounded points, the faces sorted. Equal prints are the same
/// face structure.
fn ring_print(doc: &Document, o: ObjectId) -> Vec<Vec<Vec<(i64, i64, i64)>>> {
    let q = |x: f64| (x * 1e7).round() as i64;
    let obj = doc.object(o).unwrap();
    let mut faces: Vec<Vec<Vec<(i64, i64, i64)>>> = obj
        .faces()
        .values()
        .map(|f| {
            let mut rings: Vec<Vec<(i64, i64, i64)>> = std::iter::once(f.outer_loop)
                .chain(f.inner_loops.iter().copied())
                .map(|l| {
                    let mut r: Vec<(i64, i64, i64)> = obj
                        .loop_positions(l)
                        .map(|v| (q(v.x), q(v.y), q(v.z)))
                        .collect();
                    r.sort_unstable();
                    r
                })
                .collect();
            rings.sort();
            rings
        })
        .collect();
    faces.sort();
    faces
}

/// A rounded point.
type Q3 = (i64, i64, i64);

/// The edges of a sketch as sorted rounded endpoint pairs.
fn edge_print(sk: &Sketch) -> Vec<(Q3, Q3)> {
    let q = |v: Point3| {
        (
            (v.x * 1e7).round() as i64,
            (v.y * 1e7).round() as i64,
            (v.z * 1e7).round() as i64,
        )
    };
    let mut out: Vec<_> = sk
        .edges()
        .values()
        .map(|e| {
            let (a, b) = (
                q(sk.vertices()[e.from].position),
                q(sk.vertices()[e.to].position),
            );
            if a <= b { (a, b) } else { (b, a) }
        })
        .collect();
    out.sort_unstable();
    out
}

/// The edge prints of every live non-detached world sketch, sorted.
fn plain_sketch_prints(doc: &Document) -> Vec<Vec<(Q3, Q3)>> {
    let mut out: Vec<_> = doc
        .sketch_ids()
        .into_iter()
        .map(|s| doc.sketch(s).unwrap())
        .filter(|sk| !sk.detached() && !sk.edges().is_empty())
        .map(edge_print)
        .collect();
    out.sort();
    out
}

/// Whether any edge of any live object lies exactly on an edge of `sk`
/// (both ends within a nanometre).
fn object_edge_on_sketch(doc: &Document, sk: &Sketch) -> bool {
    let near = |a: Point3, b: Point3| a.approx_eq(b, 1e-9);
    let segs: Vec<(Point3, Point3)> = sk
        .edges()
        .values()
        .map(|e| (sk.vertices()[e.from].position, sk.vertices()[e.to].position))
        .collect();
    doc.visible_object_ids().into_iter().any(|o| {
        let obj = doc.object(o).unwrap();
        obj.edges().keys().any(|e| {
            obj.edge_endpoints(e).is_some_and(|(a, b)| {
                segs.iter()
                    .any(|&(c, d)| (near(a, c) && near(b, d)) || (near(a, d) && near(b, c)))
            })
        })
    })
}

/// Draws a NEW detached shape on `plane` as one undo step and returns it.
fn draw_detached(
    doc: &mut Document,
    plane: Plane,
    edit: impl FnOnce(&mut Sketch) -> Result<(), SketchError>,
) -> SketchId {
    let depth0 = doc.undo_depth();
    let (report, change) = doc
        .draft_detached_new(plane, None, "Detached", edit)
        .expect("a detached draft commits");
    assert_eq!(doc.undo_depth(), depth0 + 1, "one undo step");
    assert!(report.objects.is_empty(), "no object changes");
    assert_eq!(report.sketches.len(), 1, "one new sketch: {report:?}");
    assert_eq!(change.sketches_touched, report.sketches);
    let s = report.sketches[0];
    assert!(
        doc.sketch(s).unwrap().detached(),
        "the new sketch is detached"
    );
    assert!(doc.sketch_owner_object(s).is_none());
    s
}

/// The one chain of a detached circle: every edge carries `g`.
fn is_whole_circle(sk: &Sketch, g: CurveGeom, n: usize) -> bool {
    let Some((first, e)) = sk.edges().iter().next() else {
        return false;
    };
    let Some(c) = e.curve else {
        return false;
    };
    sk.edges().len() == n
        && sk.edges().values().all(|e| e.curve == Some(c))
        && sk.curve_chain_at(first).len() == n
        && sk
            .curve_geom(c)
            .is_some_and(|h| h.center.approx_eq(g.center, 1e-12) && h.radius == g.radius)
        && sk.regions().len() == 1
}

// ================================================================ specs

/// Spec 1: Getting Started §8: a detached circle centered on the box's top
/// corner leaves the face whole. One detached sketch holds the full
/// 48-facet circle with its curve claim; the top plane's listing has none
/// of it; its own listing is one unit and one region that extrudes.
#[test]
fn detached_circle_on_a_box_corner_leaves_the_face_whole() {
    let (mut doc, o) = boxdoc();
    let print0 = ring_print(&doc, o);
    let g = CurveGeom {
        center: p(10., 6., 3.),
        radius: 1.0,
    };
    let s = draw_detached(&mut doc, top(), |w| circle(w, g.center, g.radius, 48));
    assert_eq!(ring_print(&doc, o), print0, "the box gained no imprint");
    assert_eq!(detached_sketches(&doc), vec![s]);
    assert!(is_whole_circle(doc.sketch(s).unwrap(), g, 48));

    let top_units = doc.plane_units(top(), None, &[o]).unwrap();
    assert!(top_units.units.is_empty(), "nothing drawn on the face");
    assert_eq!(top_units.regions.len(), 1, "the face alone");
    assert!(
        top_units
            .units
            .iter()
            .flat_map(|u| &u.edges)
            .all(|e| e.store != DraftStore::Free(s))
    );

    let own = doc.detached_sketch_units(s).unwrap();
    assert_eq!(own.units.len(), 1, "one unit");
    assert_eq!(own.units[0].edges.len(), 48);
    assert!(
        own.units[0]
            .edges
            .iter()
            .all(|e| e.store == DraftStore::Free(s))
    );
    assert_eq!(own.units[0].curves.len(), 1, "one run");
    assert_eq!(own.regions.len(), 1, "one region");
    assert!(own.regions[0].over.is_none(), "over no face");
    assert!(
        matches!(own.regions[0].extrude, Some((sk, _)) if sk == s),
        "extrudes as the sketch's own region: {:?}",
        own.regions[0].extrude
    );
}

/// Spec 2: Extruding the detached region makes a new watertight Object; the
/// emptied sketch is gone and the box is untouched.
#[test]
fn extruding_a_detached_region_makes_a_new_object() {
    let (mut doc, o) = boxdoc();
    let print0 = ring_print(&doc, o);
    let g = CurveGeom {
        center: p(10., 6., 3.),
        radius: 1.0,
    };
    let s = draw_detached(&mut doc, top(), |w| circle(w, g.center, g.radius, 48));
    let (_, rid) = doc.detached_sketch_units(s).unwrap().regions[0]
        .extrude
        .unwrap();
    let (cyl, _) = doc.extrude_region(s, rid, -2.0).expect("extrudes");
    assert_ne!(cyl, o);
    assert_eq!(doc.visible_object_ids().len(), 2, "two objects");
    for id in [o, cyl] {
        let obj = doc.object(id).unwrap();
        obj.validate().expect("valid");
        assert_eq!(obj.watertight(), WatertightState::Watertight);
    }
    assert!(doc.sketch(s).is_none(), "the emptied sketch is deleted");
    assert!(detached_sketches(&doc).is_empty());
    assert_eq!(ring_print(&doc, o), print0, "the box is unchanged");
}

/// Spec 3: A plain rectangle drawn through a detached circle lowers as if the
/// circle were absent, and the circle is untouched.
#[test]
fn a_plain_shape_drawn_over_a_detached_one_ignores_it() {
    // Straddles the box's corner: part face topology, part free drawing.
    let rect_edit = |w: &mut Sketch| rect(w, 8., 5., 12., 7., 3.);

    let (mut with, o) = boxdoc();
    let s = draw_detached(&mut with, top(), |w| circle(w, p(10., 6., 3.), 1.0, 48));
    let circle0 = with.sketch(s).unwrap().clone();
    with.draft(top(), None, &[o], "Rectangle", rect_edit)
        .expect("the rectangle commits");
    assert_eq!(with.sketch(s).unwrap(), &circle0, "the circle is untouched");

    let (mut without, o2) = boxdoc();
    without
        .draft(top(), None, &[o2], "Rectangle", rect_edit)
        .expect("the rectangle commits");
    assert_eq!(
        ring_print(&with, o),
        ring_print(&without, o2),
        "the face splits as if the circle were absent"
    );
    assert_eq!(
        plain_sketch_prints(&with),
        plain_sketch_prints(&without),
        "the free drawing is as if the circle were absent"
    );
    assert!(!object_edge_on_sketch(&with, &circle0));
}

/// Spec 4: A detached circle in the void moved (in its own context) onto the
/// box's face stays one detached sketch; the face gains nothing.
#[test]
fn a_detached_shape_moved_onto_a_face_stays_detached() {
    let (mut doc, o) = boxdoc();
    let print0 = ring_print(&doc, o);
    let s = draw_detached(&mut doc, top(), |w| circle(w, p(14., 3., 3.), 1.0, 48));
    let depth0 = doc.undo_depth();
    let (report, _) = doc
        .draft_detached(s, "Move", |w| {
            let es: Vec<_> = w.edges().keys().collect();
            w.apply_transform_edges(&es, &shift(-9., 0.))
        })
        .expect("the move commits");
    assert_eq!(doc.undo_depth(), depth0 + 1, "one undo step");
    assert_eq!(report.sketches, vec![s], "written back into the sketch");
    assert!(report.objects.is_empty());
    assert_eq!(detached_sketches(&doc), vec![s]);
    let g = CurveGeom {
        center: p(5., 3., 3.),
        radius: 1.0,
    };
    assert!(
        is_whole_circle(doc.sketch(s).unwrap(), g, 48),
        "moved whole"
    );
    assert_eq!(ring_print(&doc, o), print0, "zero imprints");
    assert!(doc.plane_units(top(), None, &[o]).unwrap().units.is_empty());
}

/// Spec 5: A plain rectangle moved onto a detached circle lands as it would
/// with no circle there, and the circle is untouched.
#[test]
fn a_plain_shape_moved_onto_a_detached_one_ignores_it() {
    let draw_and_move = |doc: &mut Document, o: ObjectId| {
        doc.draft(top(), None, &[o], "Rectangle", |w| {
            rect(w, 12., 1., 14., 2., 3.)
        })
        .expect("draws");
        doc.draft(top(), None, &[o], "Move", |w| {
            let es: Vec<_> = w
                .edges()
                .iter()
                .filter(|(_, e)| w.vertices()[e.from].position.x >= 12.0 - 1e-9)
                .filter(|(_, e)| w.vertices()[e.to].position.x >= 12.0 - 1e-9)
                .map(|(id, _)| id)
                .collect();
            assert_eq!(es.len(), 4, "the rectangle's four sides");
            w.apply_transform_edges(&es, &shift(-7.5, 1.5))
        })
        .expect("moves");
    };

    let (mut with, o) = boxdoc();
    let s = draw_detached(&mut with, top(), |w| circle(w, p(5., 3., 3.), 1.0, 48));
    let circle0 = with.sketch(s).unwrap().clone();
    draw_and_move(&mut with, o);
    assert_eq!(with.sketch(s).unwrap(), &circle0, "the circle is untouched");
    assert!(!object_edge_on_sketch(&with, &circle0));

    let (mut without, o2) = boxdoc();
    draw_and_move(&mut without, o2);
    assert_eq!(ring_print(&with, o), ring_print(&without, o2));
    assert_eq!(plain_sketch_prints(&with), plain_sketch_prints(&without));
}

/// Spec 6: Two overlapping detached circles stay two sketches, each a whole
/// circle.
#[test]
fn two_detached_shapes_never_merge() {
    let mut doc = Document::new();
    let g1 = CurveGeom {
        center: p(0., 0., 0.),
        radius: 1.0,
    };
    let g2 = CurveGeom {
        center: p(1., 0., 0.),
        radius: 1.0,
    };
    let a = draw_detached(&mut doc, ground_plane(), |w| circle(w, g1.center, 1.0, 48));
    let a0 = doc.sketch(a).unwrap().clone();
    let b = draw_detached(&mut doc, ground_plane(), |w| circle(w, g2.center, 1.0, 48));
    assert_ne!(a, b);
    assert_eq!(doc.sketch(a).unwrap(), &a0, "the first is untouched");
    assert!(is_whole_circle(doc.sketch(a).unwrap(), g1, 48));
    assert!(is_whole_circle(doc.sketch(b).unwrap(), g2, 48));
    assert_eq!(detached_sketches(&doc).len(), 2);
}

/// Spec 7: A Line chain is one gesture per segment: the first creates the
/// detached sketch, the next ones continue in it, the last closes a region.
#[test]
fn detached_line_chain_continues_in_its_sketch() {
    let (mut doc, o) = boxdoc();
    let print0 = ring_print(&doc, o);
    let (a, b, c) = (p(2., 1., 3.), p(6., 1., 3.), p(4., 4., 3.));
    let s = draw_detached(&mut doc, top(), |w| w.add_segment(a, b).map(|_| ()));
    for (from, to) in [(b, c), (c, a)] {
        let (report, _) = doc
            .draft_detached(s, "Line", |w| w.add_segment(from, to).map(|_| ()))
            .expect("continues");
        assert_eq!(report.sketches, vec![s]);
    }
    let sk = doc.sketch(s).unwrap();
    assert_eq!(sk.edges().len(), 3);
    assert_eq!(sk.regions().len(), 1, "the chain closed one region");
    assert_eq!(detached_sketches(&doc), vec![s]);
    assert_eq!(ring_print(&doc, o), print0, "over the face, yet no imprint");
}

/// Spec 8: Inside its own context a detached shape arranges with itself: a line
/// across a detached rectangle splits it into two regions of the one sketch.
#[test]
fn detached_drawing_arranges_with_itself() {
    let (mut doc, o) = boxdoc();
    let print0 = ring_print(&doc, o);
    let s = draw_detached(&mut doc, top(), |w| rect(w, 2., 1., 6., 4., 3.));
    doc.draft_detached(s, "Line", |w| {
        w.add_segment(p(4., 1., 3.), p(4., 4., 3.)).map(|_| ())
    })
    .expect("the line commits");
    let sk = doc.sketch(s).unwrap();
    assert_eq!(sk.regions().len(), 2, "two regions");
    assert_eq!(sk.edges().len(), 7, "the line split both long sides");
    assert_eq!(detached_sketches(&doc), vec![s]);
    assert_eq!(ring_print(&doc, o), print0);
}

/// Spec 9: A copy of a detached unit is a NEW detached sketch, never an island
/// of the original — in plane (even overlapping it) and out of plane.
#[test]
fn copy_of_a_detached_unit_is_a_new_detached_sketch() {
    let (mut doc, _) = boxdoc();
    let s = draw_detached(&mut doc, top(), |w| circle(w, p(5., 3., 3.), 1.0, 48));
    let s0 = doc.sketch(s).unwrap().clone();
    let islands: Vec<_> = s0.islands().keys().collect();

    // In plane, overlapping the original.
    let (c1, _) = doc
        .copy_sketch_islands(s, &islands, &shift(1., 0.))
        .expect("in-plane copy");
    assert_ne!(c1, s);
    assert_eq!(doc.sketch(s).unwrap(), &s0, "the source is untouched");
    let copy = doc.sketch(c1).unwrap();
    assert!(copy.detached(), "the copy is detached");
    assert_eq!(copy.plane(), s0.plane(), "on the same plane");
    let g = CurveGeom {
        center: p(6., 3., 3.),
        radius: 1.0,
    };
    assert!(is_whole_circle(copy, g, 48), "a whole circle of its own");

    // Out of plane: a quarter turn about a line through the circle.
    let rot = Transform::translation(Vec3::new(-5., -3., -3.))
        .then(&Transform::rotation(Vec3::new(1., 0., 0.), FRAC_PI_2).unwrap())
        .then(&Transform::translation(Vec3::new(5., 3., 8.)));
    let (c2, _) = doc
        .copy_sketch_islands(s, &islands, &rot)
        .expect("out-of-plane copy");
    let copy = doc.sketch(c2).unwrap();
    assert!(copy.detached(), "the out-of-plane copy is detached");
    assert!(copy.plane().normal().z.abs() < 1e-9, "on the turned plane");
    assert_eq!(doc.sketch(s).unwrap(), &s0);
    assert_eq!(detached_sketches(&doc).len(), 3);
}

/// Spec 10: Out-of-plane moves carry the flag through both arms: the
/// whole-sketch bake and the detach of some islands.
#[test]
fn out_of_plane_rotate_keeps_the_flag() {
    let mut doc = Document::new();
    let rot = Transform::rotation(Vec3::new(1., 0., 0.), FRAC_PI_2).unwrap();

    // Bake: the islands are the whole sketch.
    let s = draw_detached(&mut doc, ground_plane(), |w| {
        circle(w, p(5., 3., 0.), 1.0, 24)
    });
    let islands: Vec<_> = doc.sketch(s).unwrap().islands().keys().collect();
    doc.transform_sketch_islands(s, &islands, &rot)
        .expect("bakes");
    let sk = doc.sketch(s).unwrap();
    assert!(sk.detached(), "baked, still detached");
    assert!(sk.plane().normal().z.abs() < 1e-9, "on the turned plane");

    // Detach: one of two islands leaves the plane.
    let t = draw_detached(&mut doc, ground_plane(), |w| {
        rect(w, 0., 0., 1., 1., 0.)?;
        rect(w, 3., 0., 4., 1., 0.)
    });
    let isl: Vec<_> = doc.sketch(t).unwrap().islands().keys().collect();
    assert_eq!(isl.len(), 2);
    let change = doc
        .transform_sketch_islands(t, &isl[..1], &rot)
        .expect("detaches");
    assert_eq!(change.sketches_touched.len(), 2, "[source, detached]");
    let d = change.sketches_touched[1];
    assert!(
        doc.sketch(d).unwrap().detached(),
        "the detached part is detached"
    );
    let src = doc.sketch(t).unwrap();
    assert!(src.detached(), "the source stays detached");
    assert_eq!(src.islands().len(), 1);
}

/// Spec 11: Undo and redo restore the flag byte-exactly through drawing (1),
/// extruding (2), and moving (4).
#[test]
fn undo_redo_round_trips_the_flag() {
    let (mut doc, _) = boxdoc();
    let round_trip = |doc: &mut Document, before: &[u8]| {
        let after = doc.save();
        assert_eq!(
            Document::load(&after).expect("reopens").save(),
            after,
            "reopens to the same bytes"
        );
        doc.undo().unwrap();
        assert_eq!(doc.save(), before, "undo exact");
        doc.redo().unwrap();
        assert_eq!(doc.save(), after, "redo exact");
    };

    let b0 = doc.save();
    let s = draw_detached(&mut doc, top(), |w| circle(w, p(14., 3., 3.), 1.0, 48));
    round_trip(&mut doc, &b0);
    assert!(doc.sketch(s).unwrap().detached());

    let b1 = doc.save();
    doc.draft_detached(s, "Move", |w| {
        let es: Vec<_> = w.edges().keys().collect();
        w.apply_transform_edges(&es, &shift(-9., 0.))
    })
    .unwrap();
    round_trip(&mut doc, &b1);
    assert!(doc.sketch(s).unwrap().detached());

    let b2 = doc.save();
    let (_, rid) = doc.detached_sketch_units(s).unwrap().regions[0]
        .extrude
        .unwrap();
    doc.extrude_region(s, rid, 2.0).unwrap();
    round_trip(&mut doc, &b2);
    doc.undo().unwrap();
    assert!(
        doc.sketch(s).unwrap().detached(),
        "the consumed sketch comes back detached"
    );
}

/// Spec 12: A detached context opens only on a live, unowned, detached sketch.
#[test]
fn detached_lift_refuses_owned_and_plain_sketches() {
    let (mut doc, o) = boxdoc();
    // A plain free sketch.
    let plain = doc.add_sketch(ground_plane());
    doc.sketch_mut(plain)
        .unwrap()
        .add_segment(p(20., 0., 0.), p(21., 0., 0.))
        .unwrap();
    // An object-owned sketch: a loose line on the box top.
    doc.draft(top(), None, &[o], "Line", |w| {
        w.add_segment(p(2., 2., 3.), p(4., 2., 3.)).map(|_| ())
    })
    .unwrap();
    let owned = doc.object_owned_sketches(o)[0];
    // A deleted detached sketch.
    let gone = draw_detached(&mut doc, top(), |w| rect(w, 1., 1., 2., 2., 3.));
    doc.delete_sketch(gone).unwrap();

    for s in [plain, owned, gone] {
        assert_eq!(
            doc.begin_draft_detached(s, "Edit"),
            Err(DocumentError::UnknownSketch),
            "{s:?}"
        );
        assert_eq!(
            doc.detached_sketch_units(s),
            Err(DocumentError::UnknownSketch)
        );
    }
    assert!(doc.draft_sketch().is_none(), "no draft opened");

    // And a detached sketch can never become object-owned (I9).
    let s = draw_detached(&mut doc, top(), |w| rect(w, 5., 1., 6., 2., 3.));
    let before = doc.save();
    assert_eq!(
        doc.set_sketch_object_owner(s, Some(o)),
        Err(DocumentError::SketchOwnerMismatch)
    );
    assert_eq!(doc.save(), before, "the refusal changed nothing");
}

/// Spec 13: Extruding one of two regions leaves the shared edge and the other
/// region; the sketch keeps the flag.
#[test]
fn extrude_leaving_edges_keeps_the_flag() {
    let mut doc = Document::new();
    let s = draw_detached(&mut doc, ground_plane(), |w| {
        rect(w, 0., 0., 4., 2., 0.)?;
        w.add_segment(p(2., 0., 0.), p(2., 2., 0.)).map(|_| ())
    });
    assert_eq!(doc.sketch(s).unwrap().regions().len(), 2);
    let rid = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    doc.extrude_region(s, rid, 1.0).expect("extrudes");
    let sk = doc.sketch(s).expect("the sketch keeps edges");
    assert!(sk.detached(), "and keeps the flag");
    assert_eq!(sk.regions().len(), 1);
    assert_eq!(sk.edges().len(), 4, "the other region, whole");
}

/// Spec 14: The one-pass listing names each detached sketch as its own entry,
/// after the planes; no plane's entry holds its drawing.
#[test]
fn plane_units_all_lists_a_detached_sketch_as_its_own_entry() {
    let (mut doc, o) = boxdoc();
    // Plain drawing on the top (free, past the edge) and on the ground.
    doc.draft(top(), None, &[o], "Rectangle", |w| {
        rect(w, 12., 1., 14., 2., 3.)
    })
    .unwrap();
    let g = doc.add_sketch(ground_plane());
    doc.sketch_mut(g)
        .unwrap()
        .add_segment(p(20., 0., 0.), p(21., 0., 0.))
        .unwrap();
    let s = draw_detached(&mut doc, top(), |w| circle(w, p(5., 3., 3.), 1.0, 48));
    let t = draw_detached(&mut doc, top(), |w| rect(w, 1., 1., 2., 2., 3.));

    let all = doc.free_plane_units(None, &std::collections::BTreeSet::new());
    let seps: Vec<(usize, SketchId, Plane)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, (l, _))| match *l {
            PlaneListing::Detached { sketch, plane } => Some((i, sketch, plane)),
            PlaneListing::Plane(_) => None,
        })
        .collect();
    assert_eq!(
        seps.iter().map(|x| x.1).collect::<Vec<_>>(),
        {
            let mut v = vec![s, t];
            v.sort();
            v
        },
        "one entry per detached sketch, in id order"
    );
    let planes = all.len() - seps.len();
    assert_eq!(planes, 2, "the top and the ground");
    assert!(
        seps.iter().all(|&(i, _, _)| i >= planes),
        "detached entries follow the planes"
    );
    for &(i, sketch, plane) in &seps {
        assert_eq!(plane, doc.sketch(sketch).unwrap().plane());
        assert_eq!(all[i].1, doc.detached_sketch_units(sketch).unwrap());
        assert_eq!(all[i].0.plane(), plane);
    }
    for (l, units) in &all {
        if let PlaneListing::Plane(_) = l {
            assert!(
                units
                    .units
                    .iter()
                    .flat_map(|u| &u.edges)
                    .all(|e| e.store != DraftStore::Free(s) && e.store != DraftStore::Free(t)),
                "a plane's listing holds no detached drawing"
            );
        }
    }
    assert_eq!(doc.detached_sketches_on(top(), None), {
        let mut v = vec![s, t];
        v.sort();
        v
    });
    assert!(doc.detached_sketches_on(ground_plane(), None).is_empty());
}

/// The offset preview's source: a detached region's profile is read in the
/// sketch's own context ([`Document::detached_region_profile`]), keyed as its
/// listing keys it; the plane's arrangement does not hold it, and a plain
/// or stale sketch refuses.
#[test]
fn detached_region_profile_reads_the_sketchs_own_region() {
    let (mut doc, o) = boxdoc();
    let s = draw_detached(&mut doc, top(), |w| rect(w, 8., 4., 12., 5., 3.));
    let units = doc.detached_sketch_units(s).unwrap();
    assert_eq!(units.regions.len(), 1, "one whole rectangle over the edge");
    let key = units.regions[0].key;
    let profile = doc.detached_region_profile(s, key).unwrap();
    assert_eq!(profile.outer().len(), 4);
    let off = kernel::offset_profile(&profile, 0.5).unwrap();
    assert_eq!(off.outer.points.len(), 4, "grows as one rectangle");
    assert!(matches!(
        doc.plane_region_profile(top(), None, &[o], key),
        Err(DocumentError::Sketch(SketchError::UnknownRegion))
    ));
    assert!(matches!(
        doc.detached_region_profile(s, key ^ 1),
        Err(DocumentError::Sketch(SketchError::UnknownRegion))
    ));
    let g = doc.add_sketch(ground_plane());
    doc.sketch_mut(g)
        .unwrap()
        .add_segment(p(20., 0., 0.), p(21., 0., 0.))
        .unwrap();
    assert!(matches!(
        doc.detached_region_profile(g, key),
        Err(DocumentError::UnknownSketch)
    ));
}

// ======================================================= property test

/// A plain (non-detached) op on the plane of the property.
#[derive(Debug, Clone)]
enum PlainOp {
    /// A lattice rectangle drafted on the plane.
    Rect { a: (i32, i32), b: (i32, i32) },
    /// A drawing unit of the plane moved by lattice steps.
    Move { sel: usize, d: (i32, i32) },
    /// A region over the void extruded.
    Extrude { sel: usize, down: bool },
}

fn arb_plain_op() -> impl Strategy<Value = PlainOp> {
    prop_oneof![
        3 => ((-4..24i32, -4..16i32), (1..10i32, 1..10i32))
            .prop_map(|(a, b)| PlainOp::Rect { a, b }),
        2 => (any::<usize>(), (-6..=6i32, -6..=6i32))
            .prop_map(|(sel, d)| PlainOp::Move { sel, d }),
        1 => (any::<usize>(), any::<bool>()).prop_map(|(sel, down)| PlainOp::Extrude { sel, down }),
    ]
}

fn arb_axis() -> impl Strategy<Value = Vec3> {
    (-1.0..1.0f64, -1.0..1.0f64, -1.0..1.0f64)
        .prop_filter("degenerate axis", |&(x, y, z)| {
            x * x + y * y + z * z >= 0.01
        })
        .prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// For a random detached sketch S on a random plane (the top of a box
    /// turned and moved at random) and a random sequence of plain drafts,
    /// unit moves, and extrusions on that plane, S is `==` before and after
    /// every op (edges, regions, curves, handles), stays detached and
    /// unowned, and no object ever gains an edge lying on one of S's.
    #[test]
    fn a_detached_shape_is_untouched_by_plain_drafting(
        axis in arb_axis(),
        angle in -PI..PI,
        offset in (-5.0..5.0f64, -5.0..5.0f64, -5.0..5.0f64),
        circular in any::<bool>(),
        at in (-4..22i32, -4..14i32),
        size in (1..8i32, 1..8i32),
        ops in proptest::collection::vec(arb_plain_op(), 1..7),
    ) {
        let (mut doc, o) = boxdoc();
        let t = Transform::rotation(axis, angle)
            .unwrap()
            .then(&Transform::translation(Vec3::new(offset.0, offset.1, offset.2)));
        doc.transform_object(o, &t).expect("the box moves");
        let n = t.apply_vector(up()).normalized().unwrap();
        let obj = doc.object(o).unwrap();
        let plane = obj
            .faces()
            .values()
            .map(|f| f.plane)
            .find(|pl| pl.normal().dot(n) > 1.0 - 1e-9)
            .expect("the turned top");
        // Box-local lattice (0.5) on the top; S sits off it by 0.13 so no
        // plain drawing can share an end with S by construction.
        let at_l = |x: f64, y: f64| t.apply_point(p(x, y, 3.0));
        let lat = |i: i32| f64::from(i) * 0.5;
        let (sx, sy) = (lat(at.0) + 0.13, lat(at.1) + 0.13);
        let (sw, sh) = (lat(size.0), lat(size.1));
        let s = doc
            .draft_detached_new(plane, None, "S", |w| {
                if circular {
                    let c = at_l(sx, sy);
                    let u = t.apply_vector(Vec3::new(1., 0., 0.));
                    let v = t.apply_vector(Vec3::new(0., 1., 0.));
                    let ring: Vec<Point3> = (0..24)
                        .map(|i| {
                            let a = TAU * f64::from(i) / 24.0;
                            c + u * (sw * a.cos()) + v * (sw * a.sin())
                        })
                        .collect();
                    w.begin_curve_with(CurveGeom { center: c, radius: sw })?;
                    let r = draw_ring(w, &ring);
                    w.end_curve();
                    r
                } else {
                    draw_ring(w, &[
                        at_l(sx, sy),
                        at_l(sx + sw, sy),
                        at_l(sx + sw, sy + sh),
                        at_l(sx, sy + sh),
                    ])
                }
            })
            .expect("S is drawn")
            .0
            .sketches[0];
        let s0 = doc.sketch(s).unwrap().clone();
        prop_assert!(s0.detached());
        prop_assert!(!object_edge_on_sketch(&doc, &s0));

        for (k, op) in ops.iter().enumerate() {
            let objs = doc.drawable_objects_on_plane(plane, None);
            match *op {
                PlainOp::Rect { a, b } => {
                    let (x0, y0) = (lat(a.0), lat(a.1));
                    let (x1, y1) = (x0 + lat(b.0), y0 + lat(b.1));
                    let _ = doc.draft(plane, None, &objs, "Rect", |w| {
                        draw_ring(w, &[at_l(x0, y0), at_l(x1, y0), at_l(x1, y1), at_l(x0, y1)])
                    });
                }
                PlainOp::Move { sel, d } => {
                    let Ok(pu) = doc.plane_units(plane, None, &objs) else { continue };
                    if pu.units.is_empty() {
                        continue;
                    }
                    let unit = pu.units[sel % pu.units.len()].clone();
                    let by = t.apply_vector(Vec3::new(lat(d.0), lat(d.1), 0.0));
                    let _ = doc.draft(plane, None, &objs, "Move", |w| {
                        let es: Vec<_> = unit
                            .edges
                            .iter()
                            .filter_map(|e| w.edge_at_positions(e.a, e.b))
                            .collect();
                        w.apply_transform_edges(&es, &Transform::translation(by))
                    });
                }
                PlainOp::Extrude { sel, down } => {
                    let Ok(pu) = doc.plane_units(plane, None, &objs) else { continue };
                    let free: Vec<_> = pu.regions.iter().filter_map(|r| r.extrude).collect();
                    if free.is_empty() {
                        continue;
                    }
                    let (sk, rid) = free[sel % free.len()];
                    prop_assert_ne!(sk, s, "a plane listing offered S's region");
                    let _ = doc.extrude_region(sk, rid, if down { -1.0 } else { 1.0 });
                }
            }
            let now = doc.sketch(s);
            prop_assert!(now.is_some(), "op {} ({:?}) removed S", k, op);
            prop_assert!(now.unwrap() == &s0, "op {} ({:?}) changed S", k, op);
            prop_assert!(doc.sketch_owner_object(s).is_none());
            prop_assert!(
                !object_edge_on_sketch(&doc, &s0),
                "op {} ({:?}) gave an object an edge of S", k, op
            );
        }
    }
}
