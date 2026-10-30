//! Executable specs for the drafting engine (unified drafting §4,
//! `crates/kernel/src/document/draft.rs`): one way to edit the drawing on a
//! plane, whether it lies on the ground, on a solid's face, or across both.
//!
//! The contract under test:
//! - the plane's drawing (the face topology of every participating object's
//!   faces on it, those objects' owned sketches, the free sketches) is lifted
//!   into ONE working sketch, edited with any `Sketch` method, and lowered
//!   back by where each edge lies: an edge over a face that separates two
//!   regions is that object's face TOPOLOGY, a loose edge over a face (a free
//!   end, a bridge) or an island the face cannot hold (a pinch, a sliver) is
//!   the object's OWNED sketch, an edge over no face is FREE; a free region
//!   closed by a solid's edge keeps that stretch of the edge in the free
//!   sketch, so the region is closed in the sketch itself;
//! - a solid's own boundary edges can be split by drawing, never moved or
//!   removed (`DocumentError::DraftBoundaryEdgeTouched`);
//! - every draft is exactly ONE undo step, and undo / redo are byte-exact;
//! - on the ground (no objects) a draft is ordinary sketch editing.
//!
//! The scene specs follow `docs/design/unified-drafting/draft_scenes.py`
//! (scene numbers in the doc comments), drawn on a 10 x 6 box top.

use std::f64::consts::TAU;

use kernel::{
    CurveGeom, Document, DocumentError, FaceId, Material, NodeId, ObjectId, Plane, Point3, Rgba8,
    Sketch, SketchEdgeId, SketchError, SketchId, Transform, Vec3,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::{
    MM, Rings, Wall, face_rings, ground_regions, ground_with, group_faces, imprint_mm,
    multiset_match, no_pinch, rings_match, signed_area, straightened, top_of_box,
};

// ------------------------------------------------------------ fixtures

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A point on the box top (z = 3).
fn z3(x: f64, y: f64) -> Point3 {
    p(x, y, 3.0)
}

fn up() -> Vec3 {
    Vec3::new(0.0, 0.0, 1.0)
}

/// A 10 x 6 x 3 box on the ground.
fn boxdoc() -> (Document, ObjectId) {
    let mut doc = Document::new();
    let g = Plane::from_point_normal(p(0., 0., 0.), up()).unwrap();
    let s = doc.add_sketch(g);
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

/// A 10 x 6 x 3 box with a through tunnel `x0..x1 x y0..y1` (a ring region
/// extruded).
fn tunnel_doc(x0: f64, y0: f64, x1: f64, y1: f64) -> (Document, ObjectId) {
    let mut doc = Document::new();
    let g = Plane::from_point_normal(p(0., 0., 0.), up()).unwrap();
    let s = doc.add_sketch(g);
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in [(0., 0., 10., 6.), (x0, y0, x1, y1)]
        .iter()
        .flat_map(|&(ax, ay, bx, by)| {
            [
                (p(ax, ay, 0.), p(bx, ay, 0.)),
                (p(bx, ay, 0.), p(bx, by, 0.)),
                (p(bx, by, 0.), p(ax, by, 0.)),
                (p(ax, by, 0.), p(ax, ay, 0.)),
            ]
        })
    {
        sk.add_segment(a, b).unwrap();
    }
    let ring = doc
        .sketch(s)
        .unwrap()
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("the ring region");
    let (o, _) = doc.extrude_region(s, ring, 3.0).unwrap();
    // The tunnel's own outline stays behind as an unextruded region; retire
    // the sketch so the document holds only what the specs draw.
    doc.delete_sketch(s).unwrap();
    (doc, o)
}

fn top() -> Plane {
    Plane::from_point_normal(p(0., 0., 3.), up()).unwrap()
}

fn ground_plane() -> Plane {
    Plane::from_point_normal(p(0., 0., 0.), up()).unwrap()
}

/// A rectangle ring on the box top, counter-clockwise from above.
fn rect_ring(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point3> {
    vec![z3(x0, y0), z3(x1, y0), z3(x1, y1), z3(x0, y1)]
}

/// `n` points of the circle about `(cx, cy)`, vertex `i` at angle `TAU i / n`.
fn circle_ring(cx: f64, cy: f64, r: f64, n: usize) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = TAU * i as f64 / n as f64;
            z3(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn rg(outer: Vec<Point3>, holes: Vec<Vec<Point3>>) -> Rings {
    (outer, holes)
}

/// The topology faces of `o` facing up (the box top plane and its imprints).
fn top_face_ids(doc: &Document, o: ObjectId) -> Vec<FaceId> {
    let obj = doc.object(o).unwrap();
    obj.faces()
        .iter()
        .filter(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .collect()
}

fn tops(doc: &Document, o: ObjectId) -> usize {
    top_face_ids(doc, o).len()
}

fn top_rings(doc: &Document, o: ObjectId) -> Vec<Rings> {
    let obj = doc.object(o).unwrap();
    top_face_ids(doc, o)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect()
}

/// Net area (outer minus holes) of the up-facing faces.
fn top_area(doc: &Document, o: ObjectId) -> f64 {
    top_rings(doc, o)
        .iter()
        .map(|(o, hs)| {
            signed_area(o, up()).abs() - hs.iter().map(|h| signed_area(h, up()).abs()).sum::<f64>()
        })
        .sum()
}

/// A compact dump of rings for failure messages.
fn show(rings: &[Rings]) -> String {
    let pts = |r: &[Point3]| {
        r.iter()
            .map(|q| format!("({:.3},{:.3})", q.x, q.y))
            .collect::<Vec<_>>()
            .join(" ")
    };
    rings
        .iter()
        .map(|(o, hs)| {
            format!(
                "  outer[{}] {}{}",
                o.len(),
                pts(o),
                hs.iter()
                    .map(|h| format!("\n    hole[{}] {}", h.len(), pts(h)))
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The up-facing faces equal `expect` as a multiset of cyclic rings
/// (straight-through vertices and orientation aside).
fn assert_faces(doc: &Document, o: ObjectId, expect: &[Rings]) {
    let got = straightened(&top_rings(doc, o), up());
    let want = straightened(expect, up());
    assert!(
        multiset_match(&got, &want, true),
        "top faces differ: {} got vs {} expected\n got:\n{}\n want:\n{}",
        got.len(),
        want.len(),
        show(&got),
        show(&want)
    );
}

fn has_face(doc: &Document, o: ObjectId, want: &Rings) -> bool {
    let got = straightened(&top_rings(doc, o), up());
    let want = straightened(std::slice::from_ref(want), up());
    got.iter().any(|g| rings_match(g, &want[0], true))
}

/// The live sketches with drawing in them that no object owns.
fn free_sketches(doc: &Document) -> Vec<SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| doc.sketch_owner_object(s).is_none())
        .filter(|&s| !doc.sketch(s).unwrap().edges().is_empty())
        .collect()
}

/// Edges in the free sketches.
fn free_edges(doc: &Document) -> usize {
    free_sketches(doc)
        .iter()
        .map(|&s| doc.sketch(s).unwrap().edges().len())
        .sum()
}

/// Edges in the sketches `o` owns.
fn owned_edges(doc: &Document, o: ObjectId) -> usize {
    doc.object_owned_sketches(o)
        .iter()
        .map(|&s| doc.sketch(s).unwrap().edges().len())
        .sum()
}

/// Edges in every live sketch.
fn sketch_edges(doc: &Document) -> usize {
    doc.sketch_ids()
        .iter()
        .map(|&s| doc.sketch(s).unwrap().edges().len())
        .sum()
}

/// Solid edges carrying the analytic claim of `g`.
fn claims_of(doc: &Document, o: ObjectId, g: CurveGeom) -> usize {
    doc.object(o)
        .unwrap()
        .edges()
        .values()
        .filter_map(|e| e.curve)
        .filter(|c| c.center.approx_eq(g.center, 1e-9) && (c.radius - g.radius).abs() < 1e-9)
        .count()
}

fn claimed_edges(doc: &Document, o: ObjectId) -> usize {
    doc.object(o)
        .unwrap()
        .edges()
        .values()
        .filter(|e| e.curve.is_some())
        .count()
}

// -------------------------------------------------------------- drawing

fn draw_ring(w: &mut Sketch, ring: &[Point3]) -> Result<(), SketchError> {
    for i in 0..ring.len() {
        w.add_segment(ring[i], ring[(i + 1) % ring.len()])?;
    }
    Ok(())
}

fn rect(w: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> Result<(), SketchError> {
    draw_ring(w, &rect_ring(x0, y0, x1, y1))
}

/// A faceted circle carrying its exact circle (a `begin_curve_with` chain).
fn circle(w: &mut Sketch, cx: f64, cy: f64, r: f64, n: usize) -> Result<(), SketchError> {
    w.begin_curve_with(CurveGeom {
        center: z3(cx, cy),
        radius: r,
    })?;
    let ring = circle_ring(cx, cy, r, n);
    let drawn = draw_ring(w, &ring);
    w.end_curve();
    drawn
}

/// Whether `q` lies on segment `a`-`b` within a nanometre.
fn on_segment(q: Point3, a: Point3, b: Point3) -> bool {
    let d = b - a;
    let t = ((q - a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
    (a + d * t - q).length() < 1e-9
}

/// Every edge of `w` lying along one of `segs`.
fn edges_along(w: &Sketch, segs: &[(Point3, Point3)]) -> Vec<SketchEdgeId> {
    w.edges()
        .iter()
        .filter(|(_, e)| {
            let (a, b) = (w.vertices()[e.from].position, w.vertices()[e.to].position);
            segs.iter()
                .any(|&(s, t)| on_segment(a, s, t) && on_segment(b, s, t))
        })
        .map(|(id, _)| id)
        .collect()
}

fn rect_sides(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(Point3, Point3)> {
    let r = rect_ring(x0, y0, x1, y1);
    (0..4).map(|i| (r[i], r[(i + 1) % 4])).collect()
}

fn shift(dx: f64, dy: f64) -> Transform {
    Transform::translation(Vec3::new(dx, dy, 0.0))
}

fn undo_redo_exact(doc: &mut Document, before: &[u8]) {
    let after = doc.save();
    doc.undo().unwrap();
    assert_eq!(doc.save(), before, "undo exact");
    doc.redo().unwrap();
    assert_eq!(doc.save(), after, "redo exact");
}

/// One draft on `plane` over `objs`: exactly one undo step, undo / redo
/// byte-exact, and the saved result reopens to the same bytes. Leaves the
/// document redone (the draft applied).
fn draft_on(
    doc: &mut Document,
    plane: Plane,
    objs: &[ObjectId],
    label: &str,
    edit: impl FnOnce(&mut Sketch) -> Result<(), SketchError>,
) {
    let before = doc.save();
    let depth0 = doc.undo_depth();
    doc.draft(plane, None, objs, label, edit)
        .unwrap_or_else(|e| panic!("draft {label:?} refused: {e:?}"));
    assert_eq!(doc.undo_depth(), depth0 + 1, "{label}: one undo step");
    let after = doc.save();
    assert_eq!(
        Document::load(&after).expect("reopens").save(),
        after,
        "{label}: reopens to the same bytes"
    );
    doc.undo().unwrap();
    assert_eq!(doc.save(), before, "{label}: undo exact");
    assert_eq!(doc.undo_depth(), depth0, "{label}: undo depth");
    doc.redo().unwrap();
    assert_eq!(doc.save(), after, "{label}: redo exact");
}

fn draft_top(
    doc: &mut Document,
    o: ObjectId,
    label: &str,
    edit: impl FnOnce(&mut Sketch) -> Result<(), SketchError>,
) {
    draft_on(doc, top(), &[o], label, edit);
}

/// The whole object validates and every top face is pinch-free.
fn assert_sound(doc: &Document, o: ObjectId) {
    let obj = doc.object(o).unwrap();
    obj.validate().expect("valid");
    assert!(no_pinch(obj, &top()), "no pinched loop committed");
}

// ===================================================== the lead's probe

#[test]
fn loose_line_goes_to_owned_sketch() {
    let (mut doc, o) = boxdoc();
    let before = doc.save();
    doc.draft(top(), None, &[o], "Line", |w| {
        w.add_segment(p(0., 3., 3.), p(5., 3., 3.))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tops(&doc, o), 1);
    let owned = doc.object_owned_sketches(o);
    assert_eq!(owned.len(), 1);
    doc.undo().unwrap();
    assert_eq!(doc.save(), before);
    doc.redo().unwrap();
    assert_eq!(doc.object_owned_sketches(o).len(), 1);
}

#[test]
fn rectangle_inside_becomes_topology() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "Rect", |w| rect(w, 2., 2., 4., 4.))
        .unwrap();
    assert_eq!(tops(&doc, o), 2, "sub-face + top");
    assert!(doc.object_owned_sketches(o).is_empty());
    let before_undo = doc.save();
    doc.undo().unwrap();
    assert_eq!(tops(&doc, o), 1);
    doc.redo().unwrap();
    assert_eq!(doc.save(), before_undo);
}

#[test]
fn circle_overhanging_splits_at_edge() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "Circle", |w| circle(w, 9.5, 3., 1.2, 48))
        .unwrap();
    assert_eq!(tops(&doc, o), 2, "lens + rest");
    assert_eq!(free_sketches(&doc).len(), 1, "crescent in a free sketch");
}

#[test]
fn a_point_touch_is_a_face_in_a_touching_hole() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "Rect", |w| rect(w, 2., 1., 6., 5.))
        .unwrap();
    // A square touching the rect's left side at a single point (2,3).
    let diamond = vec![z3(2., 3.), z3(3., 2.), z3(4., 3.), z3(3., 4.)];
    doc.draft(top(), None, &[o], "Diamond", |w| draw_ring(w, &diamond))
        .unwrap();
    // The diamond is a face; the rectangle holds it as a hole sharing the
    // touch point with the rectangle's outer loop.
    assert_faces(
        &doc,
        o,
        &[
            rg(diamond.clone(), vec![]),
            rg(rect_ring(2., 1., 6., 5.), vec![diamond.clone()]),
            top_with_hole(rect_ring(2., 1., 6., 5.)),
        ],
    );
    assert!(doc.object_owned_sketches(o).is_empty(), "nothing is sketch");
    // The diamond's region pushes: it names its face.
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    let region = pu
        .regions
        .iter()
        .find(|r| r.outer.len() == 4 && r.outer.iter().any(|q| q.approx_eq(z3(3., 4.), 1e-9)))
        .expect("the diamond's region");
    assert!(matches!(region.over, Some((obj, Some(_))) if obj == o));
    assert_eq!(region.why, None);
}

#[test]
fn ground_drawing_is_free() {
    let (mut doc, _o) = boxdoc();
    doc.draft(ground_plane(), None, &[], "Line", |w| {
        w.add_segment(p(20., 0., 0.), p(25., 0., 0.))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(free_sketches(&doc).len(), 1);
}

#[test]
fn closing_a_loose_line_migrates_to_topology() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "L1", |w| {
        w.add_segment(p(0., 3., 3.), p(5., 3., 3.))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(doc.object_owned_sketches(o).len(), 1);
    let before = doc.save();
    doc.draft(top(), None, &[o], "L2", |w| {
        w.add_segment(p(5., 3., 3.), p(5., 6., 3.))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tops(&doc, o), 2, "the closed corner is a face now");
    assert!(
        doc.object_owned_sketches(o).is_empty(),
        "the loose line migrated into topology"
    );
    undo_redo_exact(&mut doc, &before);
}

#[test]
fn crossing_and_bridging() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "R1", |w| rect(w, 1., 1., 4., 5.))
        .unwrap();
    doc.draft(top(), None, &[o], "R2", |w| rect(w, 6., 1., 9., 5.))
        .unwrap();
    let before = doc.save();
    doc.draft(top(), None, &[o], "Bridge", |w| rect(w, 3., 2., 7., 4.))
        .unwrap();
    assert_eq!(tops(&doc, o), 6);
    undo_redo_exact(&mut doc, &before);
}

#[test]
fn deleting_a_line_merges_faces() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "Line", |w| {
        w.add_segment(p(5., 0., 3.), p(5., 6., 3.))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tops(&doc, o), 2);
    let before = doc.save();
    doc.draft(top(), None, &[o], "Erase", |w| {
        let e = w.edge_at_positions(p(5., 0., 3.), p(5., 6., 3.)).unwrap();
        w.remove_edge(e)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tops(&doc, o), 1);
    undo_redo_exact(&mut doc, &before);
}

#[test]
fn moving_a_shape_across_the_edge() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "R", |w| rect(w, 6., 2., 9., 4.))
        .unwrap();
    assert_eq!(tops(&doc, o), 2);
    let before = doc.save();
    doc.draft(top(), None, &[o], "Move", |w| {
        let es = edges_along(w, &rect_sides(6., 2., 9., 4.));
        w.apply_transform_edges(&es, &shift(2., 0.))
    })
    .unwrap();
    assert_eq!(
        tops(&doc, o),
        2,
        "the part on the solid is an imprint (chord)"
    );
    assert_eq!(free_sketches(&doc).len(), 1, "the overhang is free drawing");
    undo_redo_exact(&mut doc, &before);
    // Move it back fully onto the face: the free part rejoins topology.
    doc.draft(top(), None, &[o], "Back", |w| {
        let es: Vec<_> = [
            (p(8., 2., 3.), p(10., 2., 3.)),
            (p(10., 2., 3.), p(11., 2., 3.)),
            (p(11., 2., 3.), p(11., 4., 3.)),
            (p(11., 4., 3.), p(10., 4., 3.)),
            (p(10., 4., 3.), p(8., 4., 3.)),
            (p(8., 4., 3.), p(8., 2., 3.)),
        ]
        .iter()
        .map(|&(a, b)| w.edge_at_positions(a, b).expect("piece"))
        .collect();
        w.apply_transform_edges(&es, &shift(-2., 0.))
    })
    .unwrap();
    assert_eq!(tops(&doc, o), 2, "back on the face: one imprint again");
    assert_eq!(sketch_edges(&doc), 0, "no free drawing left");
}

#[test]
fn overhang_region_is_closed_in_its_free_sketch_and_survives_moving_the_solid() {
    let (mut doc, o) = boxdoc();
    doc.draft(top(), None, &[o], "R", |w| rect(w, 8., 2., 11., 4.))
        .unwrap();
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1);
    assert_eq!(
        doc.sketch(free[0]).unwrap().regions().len(),
        1,
        "the overhang is a closed region in its own sketch"
    );
    doc.transform_object(o, &Transform::translation(Vec3::new(0., 20., 0.)))
        .unwrap();
    assert_eq!(
        doc.sketch(free[0]).unwrap().regions().len(),
        1,
        "still closed after the box moved away"
    );
    // Extrude it: a new object.
    let r = doc
        .sketch(free[0])
        .unwrap()
        .regions()
        .keys()
        .next()
        .unwrap();
    doc.extrude_region(free[0], r, 1.0).unwrap();
}

// ====================================================== sheet 1 scenes

fn the_rect() -> Rings {
    rg(rect_ring(2., 1.5, 6., 4.5), vec![])
}

fn top_with_hole(hole: Vec<Point3>) -> Rings {
    rg(rect_ring(0., 0., 10., 6.), vec![hole])
}

/// Scene 1: a rectangle on the face is a sub-face twinned with a hole in
/// the face; nothing lands in a sketch.
#[test]
fn scene_01_rectangle_on_the_face() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    assert_faces(
        &doc,
        o,
        &[the_rect(), top_with_hole(rect_ring(2., 1.5, 6., 4.5))],
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 2: a circle inscribed in the rectangle, tangent at three facet
/// vertices: the disc plus three leftovers, and every arc edge keeps its
/// circle claim.
#[test]
fn scene_02_inscribed_circle_keeps_its_claims() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Circle", |w| circle(w, 3.5, 3., 1.5, 48));
    let c = circle_ring(3.5, 3., 1.5, 48);
    let range = |a: usize, b: usize| c[a..=b].to_vec();
    let lt: Vec<Point3> = [z3(2., 3.), z3(2., 4.5), z3(3.5, 4.5)]
        .into_iter()
        .chain(range(13, 23))
        .collect();
    let lb: Vec<Point3> = std::iter::once(z3(2., 3.))
        .chain(range(25, 35))
        .chain([z3(3.5, 1.5), z3(2., 1.5)])
        .collect();
    let right: Vec<Point3> = std::iter::once(z3(3.5, 1.5))
        .chain(range(37, 47))
        .chain(range(0, 11))
        .chain([z3(3.5, 4.5), z3(6., 4.5), z3(6., 1.5)])
        .collect();
    assert_faces(
        &doc,
        o,
        &[
            rg(c.clone(), vec![]),
            rg(lt, vec![]),
            rg(lb, vec![]),
            rg(right, vec![]),
            top_with_hole(rect_ring(2., 1.5, 6., 4.5)),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0, "everything is topology");
    let g = CurveGeom {
        center: z3(3.5, 3.),
        radius: 1.5,
    };
    assert_eq!(claims_of(&doc, o, g), 48, "every arc edge keeps its circle");
    assert_eq!(claimed_edges(&doc, o), 48, "and nothing else is claimed");
    assert_sound(&doc, o);
}

/// Scene 3: a circle centered on the rectangle's right side: two halves, the
/// inner one cut out of the rectangle, the outer one out of the face.
#[test]
fn scene_03_circle_crossing_one_side() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Circle", |w| circle(w, 6., 3., 1.2, 48));
    let c = circle_ring(6., 3., 1.2, 48);
    let left_half = c[12..=36].to_vec();
    let right_half: Vec<Point3> = c[36..].iter().chain(c[..=12].iter()).copied().collect();
    let rect_rest: Vec<Point3> = [z3(2., 1.5), z3(6., 1.5)]
        .into_iter()
        .chain(c[12..=36].iter().rev().copied())
        .chain([z3(6., 4.5), z3(2., 4.5)])
        .collect();
    let hole: Vec<Point3> = [z3(2., 1.5), z3(6., 1.5)]
        .into_iter()
        .chain(c[36..].iter().chain(c[..=12].iter()).copied())
        .chain([z3(6., 4.5), z3(2., 4.5)])
        .collect();
    assert_faces(
        &doc,
        o,
        &[
            rg(left_half, vec![]),
            rg(right_half, vec![]),
            rg(rect_rest, vec![]),
            top_with_hole(hole),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_eq!(
        claims_of(
            &doc,
            o,
            CurveGeom {
                center: z3(6., 3.),
                radius: 1.2
            }
        ),
        48
    );
    assert_sound(&doc, o);
}

/// Scene 4: a circle over two separate shapes joins them: six regions
/// (the five bounded ones plus the face around them), one step.
#[test]
fn scene_04_circle_bridging_two_rectangles() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R1", |w| rect(w, 1., 1.5, 4., 4.5));
    draft_top(&mut doc, o, "R2", |w| rect(w, 6., 1.5, 9., 4.5));
    assert_eq!(tops(&doc, o), 3);
    draft_top(&mut doc, o, "Circle", |w| circle(w, 5., 3., 1.5, 48));
    assert_eq!(tops(&doc, o), 6);
    assert!((top_area(&doc, o) - 60.0).abs() < 1e-9, "area conserved");
    // The face around the shapes holds ONE hole: the shapes are connected.
    assert!(
        top_rings(&doc, o)
            .iter()
            .any(|(_, hs)| hs.len() == 1 && signed_area(&hs[0], up()).abs() > 18.0),
        "one connected hole"
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 5: a rectangle from the face's edge across a shape: four regions;
/// the span along the face's edge inserts nothing.
#[test]
fn scene_05_rectangle_from_the_edge_across_a_shape() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 3., 2., 7., 4.));
    draft_top(&mut doc, o, "Strip", |w| rect(w, 0., 2.8, 5., 3.6));
    assert_faces(
        &doc,
        o,
        &[
            // the shape minus the strip's overlap (a C)
            rg(
                vec![
                    z3(3., 2.),
                    z3(7., 2.),
                    z3(7., 4.),
                    z3(3., 4.),
                    z3(3., 3.6),
                    z3(5., 3.6),
                    z3(5., 2.8),
                    z3(3., 2.8),
                ],
                vec![],
            ),
            // the overlap
            rg(rect_ring(3., 2.8, 5., 3.6), vec![]),
            // the strip outside the shape
            rg(rect_ring(0., 2.8, 3., 3.6), vec![]),
            // the face around both, notched at its left edge
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 6.),
                    z3(0., 6.),
                    z3(0., 3.6),
                    z3(3., 3.6),
                    z3(3., 4.),
                    z3(7., 4.),
                    z3(7., 2.),
                    z3(3., 2.),
                    z3(3., 2.8),
                    z3(0., 2.8),
                ],
                vec![],
            ),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 6: a line across the whole face through a shape: both ends meet an
/// edge, so every piece is topology and every region it crosses is cut.
#[test]
fn scene_06_line_across_the_whole_face_through_a_shape() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 3., 2., 7., 4.));
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(0., 3.), z3(10., 3.)).map(|_| ())
    });
    assert_faces(
        &doc,
        o,
        &[
            rg(rect_ring(3., 2., 7., 3.), vec![]),
            rg(rect_ring(3., 3., 7., 4.), vec![]),
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 3.),
                    z3(7., 3.),
                    z3(7., 2.),
                    z3(3., 2.),
                    z3(3., 3.),
                    z3(0., 3.),
                ],
                vec![],
            ),
            rg(
                vec![
                    z3(0., 3.),
                    z3(3., 3.),
                    z3(3., 4.),
                    z3(7., 4.),
                    z3(7., 3.),
                    z3(10., 3.),
                    z3(10., 6.),
                    z3(0., 6.),
                ],
                vec![],
            ),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0, "no loose end: nothing owned");
    assert_sound(&doc, o);
}

// ====================================================== sheet 2 scenes

/// Scene 7: a line that stops in the middle of the face is never topology:
/// it lives in a sketch the object owns.
#[test]
fn scene_07_loose_line_is_owned() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(0., 3.), z3(5., 3.)).map(|_| ())
    });
    assert_eq!(tops(&doc, o), 1, "the face is not cut");
    assert_eq!(doc.object_owned_sketches(o).len(), 1);
    assert_eq!(owned_edges(&doc, o), 1);
    assert!(free_sketches(&doc).is_empty());
    let owner = doc.object_owned_sketches(o)[0];
    assert_eq!(doc.sketch_owner_object(owner), Some(o));
    assert_eq!(doc.sketch_owner_component(owner), None);
    assert_faces(&doc, o, &[rg(rect_ring(0., 0., 10., 6.), vec![])]);
}

/// Scene 8: a circle touching a shape at one point is a disc face in a hole
/// of the shape that shares the touch point with the shape's outer loop;
/// nothing stays sketch drawing.
#[test]
fn scene_08_a_circle_touching_a_shape_at_one_point_is_a_face() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 2., 1., 6., 5.));
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let (report, _) = doc
        .draft(top(), None, &[o], "Circle", |w| circle(w, 3.5, 3., 1.5, 48))
        .expect("never refused");
    assert_eq!(doc.undo_depth(), depth0 + 1);
    assert!(report.kept_as_sketch.is_empty(), "nothing kept as drawing");
    let disc = circle_ring(3.5, 3., 1.5, 48);
    assert_faces(
        &doc,
        o,
        &[
            rg(disc.clone(), vec![]),
            rg(rect_ring(2., 1., 6., 5.), vec![disc]),
            top_with_hole(rect_ring(2., 1., 6., 5.)),
        ],
    );
    assert_eq!(owned_edges(&doc, o), 0);
    assert_eq!(free_edges(&doc), 0);
    assert_eq!(claimed_edges(&doc, o), 48, "every facet claims the circle");
    undo_redo_exact(&mut doc, &before);
}

/// Scene 9: a circle overhanging the face's edge splits at the edge: the
/// lens is topology (a chord), the crescent a free region CLOSED in its own
/// sketch by the solid's edge stretch.
#[test]
fn scene_09_overhanging_circle_splits_at_the_edge() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Circle", |w| circle(w, 9.5, 3., 1.2, 48));
    assert_eq!(tops(&doc, o), 2, "lens + the rest");
    // The face's own edge now carries the two crossing points.
    let crossings = top_rings(&doc, o)
        .iter()
        .flat_map(|(o, _)| o.iter().copied())
        .filter(|q| (q.x - 10.0).abs() < 1e-9 && q.y > 1.5 && q.y < 4.5)
        .count();
    assert!(
        crossings >= 2,
        "the solid's edge is split where the circle crosses"
    );
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1, "the crescent lives in one free sketch");
    let sk = doc.sketch(free[0]).unwrap();
    assert_eq!(sk.regions().len(), 1, "closed in its sketch");
    assert!(
        sk.edges().values().any(|e| {
            let (a, b) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
            (a.x - 10.0).abs() < 1e-9 && (b.x - 10.0).abs() < 1e-9
        }),
        "the solid's edge stretch closing the crescent is in the sketch"
    );
    // The crescent's arc keeps its circle in the sketch.
    assert!(
        sk.edges().values().any(|e| e.curve.is_some()),
        "the crescent's arc is still one curve"
    );
    assert!(claimed_edges(&doc, o) > 0, "the lens arc keeps its circle");
    assert_eq!(doc.object_owned_sketches(o).len(), 0);
    assert_sound(&doc, o);
}

/// Scene 10: a shape across a tunnel splits at the tunnel's rim: the parts
/// over the solid are topology, the parts over the tunnel are free regions
/// closed by the rim stretches.
#[test]
fn scene_10_shape_across_a_tunnel() {
    let (mut doc, o) = tunnel_doc(4., 2., 6., 4.);
    assert_eq!(tops(&doc, o), 1);
    draft_top(&mut doc, o, "Across", |w| rect(w, 3.5, 2.5, 6.5, 3.5));
    // Left stub, right stub, and the face around the tunnel and stubs.
    assert_faces(
        &doc,
        o,
        &[
            rg(rect_ring(3.5, 2.5, 4., 3.5), vec![]),
            rg(rect_ring(6., 2.5, 6.5, 3.5), vec![]),
            top_with_hole(vec![
                z3(4., 2.),
                z3(6., 2.),
                z3(6., 2.5),
                z3(6.5, 2.5),
                z3(6.5, 3.5),
                z3(6., 3.5),
                z3(6., 4.),
                z3(4., 4.),
                z3(4., 3.5),
                z3(3.5, 3.5),
                z3(3.5, 2.5),
                z3(4., 2.5),
            ]),
        ],
    );
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1, "the parts over the tunnel are free drawing");
    let regions = doc.sketch(free[0]).unwrap().regions().len();
    assert_eq!(regions, 3, "the strip and the two caps the rim cuts off");
    assert_eq!(doc.object_owned_sketches(o).len(), 0);
    assert_sound(&doc, o);
}

/// Scene 11: moving a shape onto another arranges exactly like drawing the
/// final configuration in a fresh document.
#[test]
fn scene_11_moving_a_shape_onto_another_equals_drawing_it_there() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R1", |w| rect(w, 1., 1.5, 4., 4.5));
    draft_top(&mut doc, o, "R2", |w| rect(w, 6., 1.5, 9., 4.5));
    draft_top(&mut doc, o, "Move", |w| {
        let es = edges_along(w, &rect_sides(1., 1.5, 4., 4.5));
        assert_eq!(es.len(), 4);
        w.apply_transform_edges(&es, &shift(3., 0.))
    });
    let (mut fresh, f) = boxdoc();
    draft_top(&mut fresh, f, "R1", |w| rect(w, 4., 1.5, 7., 4.5));
    draft_top(&mut fresh, f, "R2", |w| rect(w, 6., 1.5, 9., 4.5));
    let moved = top_rings(&doc, o);
    let drawn = top_rings(&fresh, f);
    assert!(
        multiset_match(
            &straightened(&moved, up()),
            &straightened(&drawn, up()),
            true
        ),
        "the move arranges like a draw"
    );
    // A minus B, the overlap, B minus A, and the face around them.
    assert_eq!(tops(&doc, o), 4);
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 12: moving a shape past the face's edge splits it (chord imprint +
/// a free region closed by the edge); moving it back rejoins one imprint.
#[test]
fn scene_12_moving_a_shape_past_the_edge_and_back() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 6., 1.5, 9., 4.5));
    draft_top(&mut doc, o, "Out", |w| {
        let es = edges_along(w, &rect_sides(6., 1.5, 9., 4.5));
        w.apply_transform_edges(&es, &shift(2., 0.))
    });
    assert_faces(
        &doc,
        o,
        &[
            rg(rect_ring(8., 1.5, 10., 4.5), vec![]),
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 1.5),
                    z3(8., 1.5),
                    z3(8., 4.5),
                    z3(10., 4.5),
                    z3(10., 6.),
                    z3(0., 6.),
                ],
                vec![],
            ),
        ],
    );
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1);
    assert_eq!(doc.sketch(free[0]).unwrap().regions().len(), 1, "closed");
    draft_top(&mut doc, o, "Back", |w| {
        let segs = [
            (z3(8., 1.5), z3(11., 1.5)),
            (z3(11., 1.5), z3(11., 4.5)),
            (z3(11., 4.5), z3(8., 4.5)),
            (z3(8., 4.5), z3(8., 1.5)),
        ];
        let es = edges_along(w, &segs);
        assert_eq!(es.len(), 6, "three topology pieces + three free");
        w.apply_transform_edges(&es, &shift(-2., 0.))
    });
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(6., 1.5, 9., 4.5),
            top_with_hole(rect_ring(6., 1.5, 9., 4.5)),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0, "the free part rejoined topology");
    assert_sound(&doc, o);
}

fn the_rect_at(x0: f64, y0: f64, x1: f64, y1: f64) -> Rings {
    rg(rect_ring(x0, y0, x1, y1), vec![])
}

// ====================================================== sheet 3 scenes

/// Scene 13: deleting the two corner lines of an inscribed-circle leftover
/// merges it into the face, whose hole re-routes along the arc.
///
/// KERNEL BUG (reported, `crates/kernel/src` untouched): the lowering keeps
/// the WHOLE island as owned-sketch drawing and strands one circle facet in a
/// free sketch, so the disc and the other leftovers stop being faces. See
/// `bug_erasing_a_rectangle_side_beside_a_tangent_circle` for the smallest
/// repro.
#[test]
fn scene_13_deleting_corner_lines_rounds_the_corner() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Circle", |w| circle(w, 3.5, 3., 1.5, 48));
    draft_top(&mut doc, o, "Erase corner", |w| {
        for (a, b) in [(z3(2., 3.), z3(2., 4.5)), (z3(2., 4.5), z3(3.5, 4.5))] {
            let e = w.edge_at_positions(a, b).expect("corner line");
            w.remove_edge(e)?;
        }
        Ok(())
    });
    let c = circle_ring(3.5, 3., 1.5, 48);
    let hole: Vec<Point3> = [z3(2., 1.5), z3(6., 1.5), z3(6., 4.5), z3(3.5, 4.5)]
        .into_iter()
        .chain(c[13..=23].iter().copied())
        .chain([z3(2., 3.)])
        .collect();
    let lb: Vec<Point3> = std::iter::once(z3(2., 3.))
        .chain(c[25..=35].iter().copied())
        .chain([z3(3.5, 1.5), z3(2., 1.5)])
        .collect();
    let right: Vec<Point3> = std::iter::once(z3(3.5, 1.5))
        .chain(c[37..].iter().copied())
        .chain(c[..=11].iter().copied())
        .chain([z3(3.5, 4.5), z3(6., 4.5), z3(6., 1.5)])
        .collect();
    assert_faces(
        &doc,
        o,
        &[
            rg(c, vec![]),
            rg(lb, vec![]),
            rg(right, vec![]),
            top_with_hole(hole),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 14: copying a shape (replaying its edges transformed) lands the
/// copy as a second sub-face.
#[test]
fn scene_14_copying_a_shape() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 1., 1.5, 4., 4.5));
    draft_top(&mut doc, o, "Copy", |w| {
        let es = edges_along(w, &rect_sides(1., 1.5, 4., 4.5));
        w.copy_edges_transformed(&es, &shift(5., 0.))
    });
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(1., 1.5, 4., 4.5),
            the_rect_at(6., 1.5, 9., 4.5),
            rg(
                rect_ring(0., 0., 10., 6.),
                vec![rect_ring(1., 1.5, 4., 4.5), rect_ring(6., 1.5, 9., 4.5)],
            ),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 14, by hand: the same copy by adding the transformed edges.
#[test]
fn scene_14_copy_by_adding_the_transformed_edges() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 1., 1.5, 4., 4.5));
    draft_top(&mut doc, o, "Copy", |w| {
        for (a, b) in rect_sides(1., 1.5, 4., 4.5) {
            w.add_segment(a + Vec3::new(5., 0., 0.), b + Vec3::new(5., 0., 0.))?;
        }
        Ok(())
    });
    assert_eq!(tops(&doc, o), 3);
    assert!(has_face(&doc, o, &the_rect_at(6., 1.5, 9., 4.5)));
}

/// Scene 15: a non-uniform scale of a circle drops its claim (map-or-drop):
/// the stretched ring is a plain polygon.
#[test]
fn scene_15_non_uniform_scale_drops_the_claim() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Circle", |w| circle(w, 5., 3., 1.5, 48));
    assert_eq!(claimed_edges(&doc, o), 48);
    let t = shift(-5., -3.)
        .then(&Transform::scale(Vec3::new(2.5 / 1.5, 1.2 / 1.5, 1.0)))
        .then(&shift(5., 3.));
    draft_top(&mut doc, o, "Stretch", |w| {
        // Only the circle's edges move (the solid's own edges are fixed).
        let es: Vec<_> = w
            .edges()
            .iter()
            .filter(|(_, e)| e.curve.is_some())
            .map(|(id, _)| id)
            .collect();
        assert_eq!(es.len(), 48);
        w.apply_transform_edges(&es, &t)
    });
    let stretched: Vec<Point3> = circle_ring(5., 3., 1.5, 48)
        .into_iter()
        .map(|q| t.apply_point(q))
        .collect();
    assert_faces(
        &doc,
        o,
        &[rg(stretched.clone(), vec![]), top_with_hole(stretched)],
    );
    assert_eq!(claimed_edges(&doc, o), 0, "the claim is dropped");
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 16: dragging one rectangle corner re-arranges the sub-face.
#[test]
fn scene_16_dragging_one_rectangle_corner() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Drag", |w| {
        let v = w
            .vertices()
            .iter()
            .find(|(_, v)| v.position.approx_eq(z3(6., 4.5), 1e-9))
            .map(|(id, _)| id)
            .expect("the corner");
        w.move_vertex_arranging(v, z3(7.5, 5.2)).map(|_| ())
    });
    let quad = vec![z3(2., 1.5), z3(6., 1.5), z3(7.5, 5.2), z3(2., 4.5)];
    assert_faces(&doc, o, &[rg(quad.clone(), vec![]), top_with_hole(quad)]);
    assert_eq!(sketch_edges(&doc), 0);
    assert_sound(&doc, o);
}

/// Scene 17: offsetting a shape outward past the face's edge: a ring region
/// on the solid and a strip over the void.
#[test]
fn scene_17_offset_outward_past_the_edge() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Shape", |w| rect(w, 6., 2., 9., 4.));
    draft_top(&mut doc, o, "Offset", |w| {
        let r = w
            .regions()
            .keys()
            .find(|&r| w.region_contains_point(r, z3(7.5, 3.)).unwrap())
            .expect("the shape's region");
        w.offset_region(r, 1.5).map(|_| ())
    });
    // The ring region (chord imprint around the shape), the shape, and the
    // face around them, notched at the edge.
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(6., 2., 9., 4.),
            rg(
                rect_ring(4.5, 0.5, 10., 5.5),
                vec![rect_ring(6., 2., 9., 4.)],
            ),
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 0.5),
                    z3(4.5, 0.5),
                    z3(4.5, 5.5),
                    z3(10., 5.5),
                    z3(10., 6.),
                    z3(0., 6.),
                ],
                vec![],
            ),
        ],
    );
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1, "the strip over the void");
    assert_eq!(doc.sketch(free[0]).unwrap().regions().len(), 1, "closed");
    assert_sound(&doc, o);
}

/// Scene 18: a shape drawn around another shape and a hole adopts both as
/// holes; the inner shape stays its own region.
#[test]
fn scene_18_shape_around_a_shape_and_a_hole() {
    let (mut doc, o) = tunnel_doc(6., 2.4, 7., 3.6);
    draft_top(&mut doc, o, "Inner", |w| rect(w, 3., 2.2, 4.5, 3.8));
    draft_top(&mut doc, o, "Around", |w| rect(w, 2., 1.2, 8., 4.8));
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(3., 2.2, 4.5, 3.8),
            rg(
                rect_ring(2., 1.2, 8., 4.8),
                vec![rect_ring(3., 2.2, 4.5, 3.8), rect_ring(6., 2.4, 7., 3.6)],
            ),
            rg(
                rect_ring(0., 0., 10., 6.),
                vec![rect_ring(2., 1.2, 8., 4.8)],
            ),
        ],
    );
    assert_eq!(sketch_edges(&doc), 0, "the tunnel's void holds nothing");
    assert_sound(&doc, o);
}

/// Smallest repro of the scene-13 bug: a rectangle imprint, a 48-gon centered
/// in it touching the top and bottom sides, then erasing the rectangle's
/// LEFT side. The left leftover merges into the face and two spurs remain
/// (loose ends attached at the tangent points); the spurs belong in the
/// owned sketch and the disc, the right leftover, and the face around them
/// stay topology. Observed: the disc and right leftover are demoted too (the
/// top is ONE face, 52 edges owned) and one circle facet lands in a FREE
/// sketch.
#[test]
fn bug_erasing_a_rectangle_side_beside_a_tangent_circle() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Circle", |w| circle(w, 4., 3., 1.5, 48));
    assert_eq!(tops(&doc, o), 4, "disc, two leftovers, the face around");
    draft_top(&mut doc, o, "Erase", |w| {
        let e = w.edge_at_positions(z3(2., 1.5), z3(2., 4.5)).unwrap();
        w.remove_edge(e).map(|_| ())
    });
    assert_eq!(tops(&doc, o), 3, "disc, right leftover, the merged face");
    assert_eq!(owned_edges(&doc, o), 2, "only the two spurs are drawing");
    assert_eq!(free_edges(&doc), 0);
}

// ============================================================ migration

/// Whether every segment of `a` is drawn by `b` and the other way round,
/// compared by sample points (the two may split a line at different
/// vertices, e.g. where topology heals a straight-through one).
fn same_drawing(a: &[(Point3, Point3)], b: &[(Point3, Point3)]) -> bool {
    let covered = |segs: &[(Point3, Point3)], by: &[(Point3, Point3)]| {
        segs.iter().all(|&(s, t)| {
            (0..=8).all(|k| {
                let q = s + (t - s) * (f64::from(k) / 8.0);
                by.iter().any(|&(c, d)| on_segment(q, c, d))
            })
        })
    };
    covered(a, b) && covered(b, a)
}

/// Every segment of the plane's drawing as lifted (topology, owned, free).
fn drawing_of(doc: &mut Document, plane: Plane, objs: &[ObjectId]) -> Vec<(Point3, Point3)> {
    doc.begin_draft(plane, None, objs, "read").unwrap();
    let w = doc.draft_sketch().unwrap();
    let segs = w
        .edges()
        .values()
        .map(|e| (w.vertices()[e.from].position, w.vertices()[e.to].position))
        .collect();
    assert!(doc.cancel_draft());
    segs
}

/// A loose line closed by a later stroke becomes topology: the owned sketch
/// is emptied (nothing left in any sketch) and the faces are exactly the
/// corner and the rest.
#[test]
fn migration_a_closed_loose_line_leaves_no_sketch_behind() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "L1", |w| {
        w.add_segment(z3(0., 3.), z3(5., 3.)).map(|_| ())
    });
    assert_eq!(owned_edges(&doc, o), 1);
    draft_top(&mut doc, o, "L2", |w| {
        w.add_segment(z3(5., 3.), z3(5., 6.)).map(|_| ())
    });
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(0., 3., 5., 6.),
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 6.),
                    z3(5., 6.),
                    z3(5., 3.),
                    z3(0., 3.),
                ],
                vec![],
            ),
        ],
    );
    assert!(doc.object_owned_sketches(o).is_empty());
    assert_eq!(sketch_edges(&doc), 0, "the owned sketch was emptied");
    assert_sound(&doc, o);
}

/// An imprint moved half off the face leaves topology for the free part
/// only: the chord stays an imprint, the overhang is free and closed.
#[test]
fn migration_an_imprint_moved_half_off_the_face_leaves_topology_for_the_free_part() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 7., 2., 9., 4.));
    assert_eq!(sketch_edges(&doc), 0);
    draft_top(&mut doc, o, "Half off", |w| {
        let es = edges_along(w, &rect_sides(7., 2., 9., 4.));
        w.apply_transform_edges(&es, &shift(2., 0.))
    });
    assert_faces(
        &doc,
        o,
        &[
            the_rect_at(9., 2., 10., 4.),
            rg(
                vec![
                    z3(0., 0.),
                    z3(10., 0.),
                    z3(10., 2.),
                    z3(9., 2.),
                    z3(9., 4.),
                    z3(10., 4.),
                    z3(10., 6.),
                    z3(0., 6.),
                ],
                vec![],
            ),
        ],
    );
    let free = free_sketches(&doc);
    assert_eq!(free.len(), 1);
    assert_eq!(doc.sketch(free[0]).unwrap().regions().len(), 1);
    // Free part: 3 own edges + the solid's edge stretch.
    assert_eq!(doc.sketch(free[0]).unwrap().edges().len(), 4);
    assert_sound(&doc, o);
}

/// Two squares joined by an overlapping filler; deleting the filler's four
/// outside edges leaves the two squares touching only at a corner. That
/// drawing cannot all be topology (a pinch); the draft must NOT refuse and
/// must not lose a single line of it.
#[test]
fn migration_deleting_a_line_that_leaves_a_pinch_keeps_the_leftover_drawing() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "S1", |w| rect(w, 2., 1., 4., 3.));
    draft_top(&mut doc, o, "M", |w| rect(w, 3., 2., 5., 4.));
    draft_top(&mut doc, o, "S2", |w| rect(w, 4., 3., 6., 5.));
    let drawn = drawing_of(&mut doc, top(), &[o]);
    let outside = [
        (z3(4., 2.), z3(5., 2.)),
        (z3(5., 2.), z3(5., 3.)),
        (z3(3., 3.), z3(3., 4.)),
        (z3(3., 4.), z3(4., 4.)),
    ];
    let expect: Vec<(Point3, Point3)> = {
        // Every drawn segment minus the four outside ones (as pieces).
        let mut keep = Vec::new();
        for &(a, b) in &drawn {
            let gone = outside
                .iter()
                .any(|&(c, d)| on_segment(a, c, d) && on_segment(b, c, d));
            if !gone {
                keep.push((a, b));
            }
        }
        keep
    };
    assert_eq!(expect.len() + 4, drawn.len(), "four pieces erased");
    let before = doc.save();
    let depth0 = doc.undo_depth();
    doc.begin_draft(top(), None, &[o], "Erase").unwrap();
    {
        let w = doc.draft_sketch_mut().unwrap();
        for &(a, b) in &outside {
            let e = w.edge_at_positions(a, b).expect("outside piece");
            w.remove_edge(e).unwrap();
        }
    }
    doc.end_draft().expect("a pinch is kept, never refused");
    assert_eq!(doc.undo_depth(), depth0 + 1, "one undo step");
    let left = drawing_of(&mut doc, top(), &[o]);
    assert!(
        same_drawing(&left, &expect),
        "the leftover drawing is intact"
    );
    assert_sound(&doc, o);
    undo_redo_exact(&mut doc, &before);
}

// ================================================================ paint

fn red(doc: &mut Document) -> kernel::MaterialId {
    doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)))
}

fn face_with(doc: &Document, o: ObjectId, want: &Rings) -> FaceId {
    let obj = doc.object(o).unwrap();
    top_face_ids(doc, o)
        .into_iter()
        .find(|&f| {
            let got = straightened(&[face_rings(obj, f)], up());
            let want = straightened(std::slice::from_ref(want), up());
            rings_match(&got[0], &want[0], true)
        })
        .unwrap_or_else(|| {
            panic!(
                "no top face with rings {}",
                show(std::slice::from_ref(want))
            )
        })
}

/// Painting an imprint sub-face, then drawing a line elsewhere on the face:
/// the painted face keeps its material (and nothing else gains it).
#[test]
fn paint_survives_a_line_drawn_elsewhere() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 2., 1.5, 6., 4.5));
    let m = red(&mut doc);
    let sub = face_with(&doc, o, &the_rect());
    doc.paint_face(o, sub, Some(m)).unwrap();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(8., 0.), z3(8., 6.)).map(|_| ())
    });
    assert_eq!(tops(&doc, o), 3, "the imprint and the face cut in two");
    let sub = face_with(&doc, o, &the_rect());
    assert_eq!(
        doc.face_material(o, sub),
        Some(m),
        "the painted face keeps it"
    );
    let painted = top_face_ids(&doc, o)
        .into_iter()
        .filter(|&f| doc.face_material(o, f) == Some(m))
        .count();
    assert_eq!(painted, 1, "and nothing else is painted");
}

/// A line drawn across the painted imprint: both halves keep its material.
#[test]
fn paint_survives_a_line_drawn_across_the_painted_imprint() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 2., 1.5, 6., 4.5));
    let m = red(&mut doc);
    let sub = face_with(&doc, o, &the_rect());
    doc.paint_face(o, sub, Some(m)).unwrap();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(4., 0.), z3(4., 6.)).map(|_| ())
    });
    let left = face_with(&doc, o, &the_rect_at(2., 1.5, 4., 4.5));
    let right = face_with(&doc, o, &the_rect_at(4., 1.5, 6., 4.5));
    assert_eq!(doc.face_material(o, left), Some(m), "left half");
    assert_eq!(doc.face_material(o, right), Some(m), "right half");
    let painted = top_face_ids(&doc, o)
        .into_iter()
        .filter(|&f| doc.face_material(o, f) == Some(m))
        .count();
    assert_eq!(painted, 2, "the faces outside the imprint stay unpainted");
}

/// A seam between two faces carrying different attribute dictionaries is
/// the solid's content, like a paint boundary: the draft lifts it as the
/// solid's own edge, so erasing it refuses rather than merging the faces
/// and dropping one dictionary, and the face it lies on offers no drawing
/// plane for it. With equal dictionaries the same seam is ordinary drawing
/// again.
#[test]
fn a_seam_between_different_dictionaries_is_not_drawing() {
    for (left_value, right_value, drawing) in [(1, 2, false), (1, 1, true)] {
        let (mut doc, o) = boxdoc();
        draft_top(&mut doc, o, "Line", |w| {
            w.add_segment(z3(4., 0.), z3(4., 6.)).map(|_| ())
        });
        assert_eq!(tops(&doc, o), 2);
        for f in top_face_ids(&doc, o) {
            let left = doc
                .object(o)
                .unwrap()
                .loop_positions(doc.object(o).unwrap().faces()[f].outer_loop)
                .all(|q| q.x <= 4.0 + 1e-9);
            let v = if left { left_value } else { right_value };
            doc.face_attr_set(o, f, "com.example.t", "k", kernel::AttrValue::Int(v))
                .unwrap();
        }
        let planes = doc.object_drawing_planes(o);
        assert_eq!(
            planes.iter().any(|q| (q.point().z - 3.0).abs() < 1e-9),
            drawing,
            "the top offers a drawing plane only for a drawn seam"
        );
        // Erasing the seam: drawing goes, and the faces merge; a
        // dictionary boundary is the solid's own edge and refuses, the
        // document untouched.
        let before = doc.save();
        doc.begin_draft(top(), None, &[o], "Erase").unwrap();
        {
            let w = doc.draft_sketch_mut().unwrap();
            let e = w
                .edge_at_positions(z3(4., 0.), z3(4., 6.))
                .expect("the seam is in the working sketch");
            w.remove_edge(e).unwrap();
        }
        let erased = doc.end_draft();
        assert_eq!(
            erased.is_ok(),
            drawing,
            "values {left_value} and {right_value}: {erased:?}"
        );
        if drawing {
            assert_eq!(tops(&doc, o), 1, "the halves merged");
        } else {
            doc.cancel_draft();
            assert_eq!(doc.save(), before, "a refused erase changes nothing");
            assert_eq!(tops(&doc, o), 2);
        }
    }
}

// ============================================================= refusals

/// A draft that removes a solid's own edge is refused with the document
/// untouched and the draft still open.
#[test]
fn refusal_removing_a_solids_own_edge() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 2., 1.5, 6., 4.5));
    let before = doc.save();
    let depth0 = doc.undo_depth();
    doc.begin_draft(top(), None, &[o], "Remove an edge")
        .unwrap();
    {
        let w = doc.draft_sketch_mut().unwrap();
        let e = w
            .edge_at_positions(z3(0., 0.), z3(10., 0.))
            .expect("box edge");
        w.remove_edge(e).unwrap();
    }
    assert_eq!(
        doc.end_draft().err(),
        Some(DocumentError::DraftBoundaryEdgeTouched)
    );
    assert_eq!(doc.save(), before, "document untouched");
    assert_eq!(doc.undo_depth(), depth0, "no undo entry");
    assert!(doc.draft_sketch().is_some(), "the draft is still open");
    assert!(doc.cancel_draft());
    assert!(doc.draft_sketch().is_none());
    assert_eq!(doc.save(), before);
}

/// Moving a solid's own edge (or dragging its corner) is refused the same
/// way; the edit may be reverted in the open draft and then commits.
#[test]
fn refusal_moving_a_solids_own_edge_or_corner() {
    let (mut doc, o) = boxdoc();
    let before = doc.save();
    let depth0 = doc.undo_depth();
    doc.begin_draft(top(), None, &[o], "Move an edge").unwrap();
    {
        let w = doc.draft_sketch_mut().unwrap();
        let es = edges_along(w, &[(z3(0., 0.), z3(10., 0.))]);
        assert_eq!(es.len(), 1);
        w.apply_transform_edges(&es, &shift(0., 1.)).unwrap();
    }
    assert_eq!(
        doc.end_draft().err(),
        Some(DocumentError::DraftBoundaryEdgeTouched)
    );
    assert!(doc.cancel_draft());
    doc.begin_draft(top(), None, &[o], "Drag a corner").unwrap();
    {
        let w = doc.draft_sketch_mut().unwrap();
        let v = w
            .vertices()
            .iter()
            .find(|(_, v)| v.position.approx_eq(z3(10., 6.), 1e-9))
            .map(|(id, _)| id)
            .unwrap();
        w.move_vertex_arranging(v, z3(11., 7.)).unwrap();
    }
    assert_eq!(
        doc.end_draft().err(),
        Some(DocumentError::DraftBoundaryEdgeTouched)
    );
    assert!(doc.cancel_draft());
    assert_eq!(doc.save(), before, "document untouched");
    assert_eq!(doc.undo_depth(), depth0);
}

/// The closure form cancels the draft on a refusal and leaves the document
/// untouched.
#[test]
fn refusal_closure_form_cancels_and_leaves_the_document_untouched() {
    let (mut doc, o) = boxdoc();
    let before = doc.save();
    let err = doc
        .draft(top(), None, &[o], "Remove", |w| {
            let e = w.edge_at_positions(z3(0., 0.), z3(10., 0.)).unwrap();
            w.remove_edge(e).map(|_| ())
        })
        .err();
    assert_eq!(err, Some(DocumentError::DraftBoundaryEdgeTouched));
    assert!(doc.draft_sketch().is_none(), "no draft left open");
    assert_eq!(doc.save(), before);
}

#[test]
fn refusal_begin_twice_and_end_with_none_open() {
    let (mut doc, o) = boxdoc();
    assert_eq!(doc.end_draft().err(), Some(DocumentError::DraftNotOpen));
    assert!(!doc.cancel_draft(), "nothing to cancel");
    assert!(doc.draft_sketch().is_none());
    assert!(doc.draft_sketch_mut().is_none());
    doc.begin_draft(top(), None, &[o], "One").unwrap();
    assert_eq!(
        doc.begin_draft(top(), None, &[o], "Two").err(),
        Some(DocumentError::DraftAlreadyOpen)
    );
    assert_eq!(
        doc.draft(top(), None, &[o], "Three", |_| Ok(())).err(),
        Some(DocumentError::DraftAlreadyOpen)
    );
    assert!(
        doc.draft_sketch().is_some(),
        "the first draft is still open"
    );
    assert!(doc.cancel_draft());
    doc.begin_draft(top(), None, &[o], "Again").unwrap();
    assert!(doc.cancel_draft());
}

/// A stale or hidden object cannot be drafted over.
#[test]
fn refusal_unknown_object() {
    let (mut doc, o) = boxdoc();
    doc.delete_node(NodeId::Object(o)).unwrap();
    assert_eq!(
        doc.begin_draft(top(), None, &[o], "Gone").err(),
        Some(DocumentError::UnknownObject)
    );
    assert!(doc.draft_sketch().is_none());
}

/// An unchanged draft records nothing.
#[test]
fn an_unchanged_draft_records_nothing() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "R", |w| rect(w, 2., 1.5, 6., 4.5));
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(8., 3.), z3(9., 3.)).map(|_| ())
    });
    let before = doc.save();
    let depth0 = doc.undo_depth();
    let redo0 = doc.redo_depth();
    doc.begin_draft(top(), None, &[o], "Nothing").unwrap();
    let (report, _) = doc.end_draft().unwrap();
    assert!(report.objects.is_empty() && report.sketches.is_empty());
    assert_eq!(doc.undo_depth(), depth0, "no undo entry");
    assert_eq!(doc.redo_depth(), redo0);
    assert_eq!(doc.save(), before);
    // Re-drawing a line that is already drawn changes nothing either.
    let (report, _) = doc
        .draft(top(), None, &[o], "Again", |w| {
            w.add_segment(z3(2., 1.5), z3(6., 1.5)).map(|_| ())
        })
        .unwrap();
    assert!(report.objects.is_empty() && report.sketches.is_empty());
    assert_eq!(doc.undo_depth(), depth0);
    assert_eq!(doc.save(), before);
}

// ======================================================== ground drafting

fn rect_z(w: &mut Sketch, z: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> Result<(), SketchError> {
    draw_ring(w, &[p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)])
}

/// A canonical, tolerance-free key set of a sketch's drawing.
fn segset(sk: &Sketch) -> std::collections::BTreeSet<[i64; 6]> {
    let q = |x: f64| (x * 1e9).round() as i64;
    sk.edges()
        .values()
        .map(|e| {
            let (a, b) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
            let (a, b) = ([q(a.x), q(a.y), q(a.z)], [q(b.x), q(b.y), q(b.z)]);
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            [a[0], a[1], a[2], b[0], b[1], b[2]]
        })
        .collect()
}

/// Drafting with no objects is ordinary sketch editing: draw, move onto,
/// erase, each equal to the same edit on a plain sketch, in ONE free sketch.
#[test]
fn ground_drafting_with_no_objects_equals_sketch_editing() {
    let mut doc = Document::new();
    let g = ground_plane();
    let mut control = Sketch::on_plane(g);
    type Edit = Box<dyn Fn(&mut Sketch) -> Result<(), SketchError>>;
    let edits: Vec<(&str, Edit)> = vec![
        ("A", Box::new(|w| rect_z(w, 0., 0., 0., 3., 3.))),
        ("B", Box::new(|w| rect_z(w, 0., 5., 0., 8., 3.))),
        (
            "Move B onto A",
            Box::new(|w| {
                let sides = [
                    (p(5., 0., 0.), p(8., 0., 0.)),
                    (p(8., 0., 0.), p(8., 3., 0.)),
                    (p(8., 3., 0.), p(5., 3., 0.)),
                    (p(5., 3., 0.), p(5., 0., 0.)),
                ];
                let es = edges_along(w, &sides);
                w.apply_transform_edges(&es, &shift(-3., 0.))
            }),
        ),
        (
            "Erase",
            Box::new(|w| {
                let e = w
                    .edge_at_positions(p(0., 0., 0.), p(2., 0., 0.))
                    .expect("piece of A's bottom");
                w.remove_edge(e).map(|_| ())
            }),
        ),
    ];
    for (label, edit) in &edits {
        let before = doc.save();
        let depth0 = doc.undo_depth();
        edit(&mut control).unwrap();
        doc.draft(g, None, &[], label, |w| edit(w)).unwrap();
        assert_eq!(doc.undo_depth(), depth0 + 1, "{label}: one step");
        assert_eq!(free_sketches(&doc).len(), 1, "{label}: one free sketch");
        let sk = doc.sketch(free_sketches(&doc)[0]).unwrap();
        assert_eq!(segset(sk), segset(&control), "{label}: same drawing");
        assert_eq!(sk.regions().len(), control.regions().len(), "{label}");
        undo_redo_exact(&mut doc, &before);
    }
    // Erasing everything leaves no drawing.
    doc.draft(g, None, &[], "Erase all", |w| {
        let es: Vec<_> = w.edges().keys().collect();
        for e in es {
            if w.edges().contains_key(e) {
                w.remove_edge(e)?;
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(free_sketches(&doc).is_empty(), "nothing left to draw");
}

/// A free region on the ground extrudes with the ordinary extrude: ground
/// drafting hands over plain sketches.
#[test]
fn ground_drafting_makes_ordinary_extrudable_regions() {
    let mut doc = Document::new();
    doc.draft(ground_plane(), None, &[], "A", |w| {
        rect_z(w, 0., 0., 0., 3., 3.)
    })
    .unwrap();
    let s = free_sketches(&doc)[0];
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (o, _) = doc.extrude_region(s, r, 1.0).unwrap();
    doc.object(o).unwrap().validate().unwrap();
}

// ================================================================ frames

/// Drafting inside a component definition (the plane in definition-local
/// coordinates): a loose line becomes an owned sketch that is ALSO a
/// definition sketch, renders through every instance, and undoes exactly.
#[test]
fn frame_drafting_inside_a_component_definition() {
    let (mut doc, o) = boxdoc();
    let (comp, _inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (second, _) = doc.place_instance(comp, shift(20., 0.)).unwrap();
    let members = doc.def_members(comp).unwrap();
    assert_eq!(members.len(), 1);
    let m = members[0];
    // A world draft over the (now definition-owned) member refuses: wrong frame.
    assert_eq!(
        doc.begin_draft(top(), None, &[m], "World").err(),
        Some(DocumentError::UnknownObject)
    );
    let before = doc.save();
    let depth0 = doc.undo_depth();
    doc.draft(top(), Some(comp), &members, "Line", |w| {
        w.add_segment(z3(0., 3.), z3(5., 3.)).map(|_| ())
    })
    .unwrap();
    assert_eq!(doc.undo_depth(), depth0 + 1);
    let owned = doc.object_owned_sketches(m);
    assert_eq!(owned.len(), 1, "an owned sketch");
    assert_eq!(doc.sketch_owner_object(owned[0]), Some(m));
    assert_eq!(
        doc.sketch_owner_component(owned[0]),
        Some(comp),
        "also a definition sketch"
    );
    assert!(
        doc.def_member_sketches(comp).unwrap().contains(&owned[0]),
        "renders through the definition's instances"
    );
    assert_eq!(doc.instances_of(comp).len(), 2);
    let _ = second;
    assert_eq!(tops(&doc, m), 1, "a loose line cuts nothing");
    undo_redo_exact(&mut doc, &before);
    assert!(doc.object_owned_sketches(m).len() == 1, "redone");
    // A closed shape inside the definition becomes member topology, for
    // every instance.
    doc.draft(top(), Some(comp), &members, "Rect", |w| {
        rect(w, 2., 1.5, 6., 4.5)
    })
    .unwrap();
    assert_eq!(tops(&doc, m), 2);
    let after = doc.save();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.save(), after);
}

/// Drafting on a member during an explode session (frame `None`, the member
/// is world geometry now): closing the session folds the drawing into the
/// definition.
#[test]
fn frame_drafting_during_an_explode_session_folds_into_the_definition() {
    let (mut doc, o) = boxdoc();
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let m = doc.def_members(comp).unwrap()[0];
    doc.open_explode_session(inst).unwrap();
    assert!(
        doc.visible_object_ids().contains(&m),
        "the member is world geometry"
    );
    let depth0 = doc.undo_depth();
    doc.draft(top(), None, &[m], "Line", |w| {
        w.add_segment(z3(0., 3.), z3(5., 3.)).map(|_| ())
    })
    .unwrap();
    assert_eq!(doc.undo_depth(), depth0 + 1);
    assert_eq!(doc.object_owned_sketches(m).len(), 1);
    let before_close = doc.save();
    doc.close_explode_session().unwrap();
    let owned = doc.object_owned_sketches(m);
    assert_eq!(owned.len(), 1, "the drawing survived the close");
    assert_eq!(
        doc.sketch_owner_component(owned[0]),
        Some(comp),
        "folded into the definition"
    );
    assert!(doc.def_member_sketches(comp).unwrap().contains(&owned[0]));
    assert_eq!(doc.sketch(owned[0]).unwrap().edges().len(), 1);
    let closed = doc.save();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.save(), closed, "close redoes exactly");
    let _ = before_close;
}

/// Two coincident boxes share the plane z = 0; drawing a rectangle across the
/// imprint drawn on the SECOND box must change only that box.
#[test]
fn coincident_objects_keep_their_own_imprints() {
    let mut doc = Document::new();
    let mut make = || {
        let g = Plane::from_point_normal(p(0., 0., 0.), up()).unwrap();
        let s = doc.add_sketch(g);
        let c = [p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.)];
        for i in 0..4 {
            doc.sketch_mut(s)
                .unwrap()
                .add_segment(c[i], c[(i + 1) % 4])
                .unwrap();
        }
        let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
        doc.extrude_region(s, r, 0.5).unwrap().0
    };
    let (a, b) = (make(), make());
    let plane = Plane::from_point_normal(p(0., 0., 0.), Vec3::new(0., 0., -1.)).unwrap();
    let bottom = |doc: &Document, o: ObjectId| {
        doc.object(o)
            .unwrap()
            .faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z < -0.9)
            .map(|(i, _)| i)
            .unwrap()
    };
    let face = bottom(&doc, b);
    doc.apply_object_op(
        b,
        kernel::KernelOp::SplitFaceInner {
            face,
            loop_path: vec![
                p(0.15, 0.25, 0.),
                p(0.15, 0.75, 0.),
                p(0.4, 0.75, 0.),
                p(0.4, 0.25, 0.),
            ],
            restore: None,
            curves: Vec::new(),
            edge_attrs: Vec::new(),
        },
    )
    .unwrap();
    let on_plane = |doc: &Document, o: ObjectId| group_faces(doc.object(o).unwrap(), &plane).len();
    assert_eq!((on_plane(&doc, a), on_plane(&doc, b)), (1, 2));
    doc.draft(plane, None, &[a, b], "Rect", |w| {
        let r = [
            p(0.3, 0.3, 0.),
            p(0.3, 0.5, 0.),
            p(0.7, 0.5, 0.),
            p(0.7, 0.3, 0.),
        ];
        for i in 0..4 {
            w.add_segment(r[i], r[(i + 1) % 4])?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(on_plane(&doc, a), 1, "the first box is untouched");
    assert_eq!(
        on_plane(&doc, b),
        4,
        "the second box holds the imprint and the rectangle"
    );
}

// ============================================================== property

/// One random second drawing on the box top, in half-millimetre lattice
/// units from the face's minimum corner.
#[derive(Debug, Clone)]
enum Shape {
    Rect { x: u32, y: u32, w: u32, h: u32 },
    Circle { cx: u32, cy: u32, r: u32 },
}

fn arb_shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        3 => (0u32..=64, 0u32..=64, 1u32..=30, 1u32..=30)
            .prop_map(|(x, y, w, h)| Shape::Rect { x, y, w, h }),
        2 => (0u32..=60, 0u32..=60, 2u32..=16)
            .prop_map(|(cx, cy, r)| Shape::Circle { cx, cy, r }),
    ]
}

const FACETS: usize = 24;

/// The shape's loop on the wall and its per-edge circle claims.
fn shape_loop(w: &Wall, s: &Shape) -> (Vec<Point3>, Vec<Option<CurveGeom>>) {
    let l = |k: u32| f64::from(k) * 0.5 * MM;
    match *s {
        Shape::Rect { x, y, w: rw, h } => (w.rect(l(x), l(y), l(rw), l(h)), Vec::new()),
        Shape::Circle { cx, cy, r } => {
            let g = w.circle_geom(l(cx), l(cy), l(r));
            (
                w.ring(l(cx), l(cy), l(r), FACETS, 0.0),
                vec![Some(g); FACETS],
            )
        }
    }
}

/// Draws the shape through the draft's working sketch.
fn draw_shape(
    sk: &mut Sketch,
    ring: &[Point3],
    curves: &[Option<CurveGeom>],
) -> Result<(), SketchError> {
    if let Some(Some(g)) = curves.first() {
        sk.begin_curve_with(*g)?;
        let drawn = draw_ring(sk, ring);
        sk.end_curve();
        drawn
    } else {
        draw_ring(sk, ring)
    }
}

fn group_rings(w: &Wall) -> Vec<Rings> {
    let obj = w.obj();
    group_faces(obj, &w.plane)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Parity with the ground: a random rectangle imprint on a 0.5 mm
    /// lattice, then a random circle or rectangle (which may touch, cross,
    /// bridge, overhang, or pinch) drawn through a draft. A draft never
    /// refuses. Where the whole drawing lands as face topology (no sketch
    /// edge is left), the faces equal the regions a ground sketch of the
    /// same lines makes (straight-through vertices aside); otherwise the
    /// drawing went to an owned or free sketch (or, when the shape was
    /// already drawn, changed nothing). The debug validator runs on every
    /// commit; every draft is one exact undo step.
    #[test]
    fn a_draft_matches_the_ground_and_never_refuses(
        rx in 4u32..=40, ry in 4u32..=40, rw in 4u32..=20, rh in 4u32..=20,
        shape in arb_shape(),
    ) {
        let half = |k: u32| f64::from(k) * 0.5;
        prop_assume!(rx + rw <= 58 && ry + rh <= 58);
        let mut b = top_of_box();
        {
            let seed = b.face;
            imprint_mm(&mut b, seed, half(rx), half(ry), half(rw), half(rh));
        }
        let (ring, curves) = shape_loop(&b, &shape);
        let n = up();

        // The ground oracle: the group's edges plus the shape's segments.
        let faces_before = group_rings(&b);
        let ground = ground_regions(&ground_with(b.plane, b.obj(), &ring));

        // A draft.
        let bytes0 = b.doc.save();
        let depth0 = b.doc.undo_depth();
        let plane = b.plane;
        let id = b.id;
        let res = b.doc.draft(plane, None, &[id], "Shape", |sk| draw_shape(sk, &ring, &curves));
        prop_assert!(res.is_ok(), "a draft refused: {:?}", res.err());
        b.obj().validate().expect("valid after the draft");
        prop_assert!(no_pinch(b.obj(), &b.plane), "no pinched loop committed");
        let faces_after = group_rings(&b);
        let extra = sketch_edges(&b.doc);
        if extra == 0 {
            prop_assert!(
                multiset_match(
                    &straightened(&faces_after, n),
                    &straightened(&ground, n),
                    true,
                ),
                "faces differ from the ground: {} (draft) vs {} (ground)",
                faces_after.len(),
                ground.len()
            );
        } else {
            let unchanged = multiset_match(
                &straightened(&faces_after, n),
                &straightened(&faces_before, n),
                true,
            );
            prop_assert!(
                extra > 0 || unchanged,
                "the drawing went nowhere (no sketch edge, faces unchanged)"
            );
        }
        // One exact undo step, and the result reopens.
        let changed = b.doc.undo_depth() != depth0;
        if changed {
            prop_assert_eq!(b.doc.undo_depth(), depth0 + 1, "one undo step");
            let after = b.doc.save();
            prop_assert_eq!(Document::load(&after).expect("reopens").save(), after.clone());
            b.doc.undo().unwrap();
            prop_assert_eq!(b.doc.save(), bytes0, "undo exact");
            b.doc.redo().unwrap();
            prop_assert_eq!(b.doc.save(), after, "redo exact");
        } else {
            prop_assert_eq!(b.doc.save(), bytes0, "an unchanged draft changes nothing");
        }
    }
}

// ================================================= review regressions

/// A circle drawn on a face lifts back as ONE chain, as on the ground, so
/// chain-granular edits see the whole circle.
#[test]
fn a_circle_on_a_face_lifts_as_one_chain() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Circle", |w| circle(w, 5., 3., 1., 12));
    assert_eq!(sketch_edges(&doc), 0, "the circle is topology");
    doc.begin_draft(top(), None, &[o], "Inspect").unwrap();
    let w = doc.draft_sketch().unwrap();
    let ring = circle_ring(5., 3., 1., 12);
    let facet = edges_along(w, &[(ring[0], ring[1])]);
    assert_eq!(facet.len(), 1);
    assert_eq!(w.curve_chain_at(facet[0]).len(), 12, "the whole circle");
    assert_eq!(w.curve_rims().len(), 1, "one rim");
    assert!(doc.cancel_draft());
}

/// A loose line ending on a face's edge leaves the face whole and pushable:
/// its end is a straight-through point on the ring, not a new corner.
#[test]
fn a_loose_line_on_the_edge_leaves_the_face_pushable() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(0., 3.), z3(4., 3.)).map(|_| ())
    });
    let units = doc.plane_units(top(), None, &[o]).unwrap();
    assert_eq!(units.regions.len(), 1);
    assert!(
        matches!(units.regions[0].over, Some((ob, Some(_))) if ob == o),
        "the region is the face: {:?}",
        units.regions[0].over
    );

    draft_top(&mut doc, o, "Rectangle", |w| rect(w, 6., 1., 8., 2.));
    let units = doc.plane_units(top(), None, &[o]).unwrap();
    assert_eq!(units.regions.len(), 2);
    for r in &units.regions {
        assert!(
            matches!(r.over, Some((ob, Some(_))) if ob == o),
            "every region is a face, the holed one too: {:?}",
            r.over
        );
    }
}

/// A closing line that joins drawing kept in two free sketches gathers the
/// closed shape into one of them, so its region extrudes.
#[test]
fn closing_drawing_split_across_free_sketches_gathers_it_into_one() {
    let mut doc = Document::new();
    let g = |x: f64, y: f64| p(x, y, 0.);
    let x = doc.add_sketch(ground_plane());
    {
        let sk = doc.sketch_mut(x).unwrap();
        sk.add_segment(g(0., 0.), g(4., 0.)).unwrap();
        sk.add_segment(g(4., 0.), g(4., 4.)).unwrap();
    }
    let y = doc.add_sketch(ground_plane());
    doc.sketch_mut(y)
        .unwrap()
        .add_segment(g(4., 4.), g(0., 4.))
        .unwrap();
    draft_on(&mut doc, ground_plane(), &[], "Line", |w| {
        w.add_segment(g(0., 4.), g(0., 0.)).map(|_| ())
    });
    assert_eq!(free_sketches(&doc).len(), 1, "one sketch holds the square");
    let units = doc.plane_units(ground_plane(), None, &[]).unwrap();
    assert_eq!(units.regions.len(), 1);
    assert!(units.regions[0].extrude.is_some(), "the square extrudes");
}

/// A draft is a picture of the document when it opened: an undo (or any
/// recorded change) closes it unlowered, so it can never write into an
/// object the undo took away or cut off the redo branch.
#[test]
fn a_change_under_an_open_draft_closes_it() {
    let (mut doc, o) = boxdoc();
    let depth = doc.undo_depth();
    doc.begin_draft(top(), None, &[o], "Rect").unwrap();
    rect(doc.draft_sketch_mut().unwrap(), 2., 1., 6., 4.).unwrap();
    doc.undo().unwrap(); // the extrusion goes away
    assert!(doc.draft_sketch().is_none(), "the draft is stale");
    assert!(matches!(doc.end_draft(), Err(DocumentError::DraftNotOpen)));
    assert_eq!(doc.undo_depth(), depth - 1, "nothing was recorded");
    assert!(doc.redo().is_ok(), "the redo branch survives");
    // A new draft replaces the stale one.
    doc.begin_draft(top(), None, &[o], "Again").unwrap();
    assert!(doc.cancel_draft());
}

/// A solid's own boundary edges are not drawing: their keys name nothing.
#[test]
fn a_solids_own_edge_key_names_nothing_in_a_draft() {
    let (mut doc, o) = boxdoc();
    doc.begin_draft(top(), None, &[o], "Probe").unwrap();
    let k = kernel::edge_key(&top(), z3(0., 0.), z3(10., 0.));
    assert!(doc.draft_edges_by_key(&[k]).is_empty());
    assert!(doc.cancel_draft());
}

/// A circle lists as one curve run; a line across it splits it into the
/// runs a click selects; a regular polygon's run says it is a polygon.
#[test]
fn units_list_curve_runs_a_click_selects() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Circle", |w| circle(w, 5., 3., 1., 12));
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    let runs: Vec<_> = pu.units.iter().flat_map(|u| u.curves.iter()).collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].edges.len(), 12);
    assert_eq!(runs[0].kind, kernel::SketchCurveKind::Circle);

    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(3., 3.), z3(7., 3.)).map(|_| ())
    });
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    let runs: Vec<_> = pu.units.iter().flat_map(|u| u.curves.iter()).collect();
    assert_eq!(
        runs.len(),
        2,
        "the crossing line splits the circle in two runs"
    );
    assert!(runs.iter().all(|r| r.curve_key == runs[0].curve_key));

    let mut ground = Document::new();
    ground
        .draft(ground_plane(), None, &[], "Polygon", |w| {
            w.begin_curve_with_kind(
                CurveGeom {
                    center: p(0., 0., 0.),
                    radius: 1.,
                },
                kernel::SketchCurveKind::Polygon,
            )?;
            let ring: Vec<Point3> = (0..6)
                .map(|i| {
                    let a = TAU * i as f64 / 6.;
                    p(a.cos(), a.sin(), 0.)
                })
                .collect();
            let r = draw_ring(w, &ring);
            w.end_curve();
            r
        })
        .unwrap();
    let pu = ground.plane_units(ground_plane(), None, &[]).unwrap();
    assert_eq!(pu.units[0].curves.len(), 1);
    assert_eq!(pu.units[0].curves[0].kind, kernel::SketchCurveKind::Polygon);
}

/// A shape flush to a face's corner and the face around it share edges;
/// their region keys still differ (a region key hashes its whole ring).
#[test]
fn regions_sharing_edges_have_distinct_keys() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rect", |w| rect(w, 0., 0., 3., 2.));
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    assert_eq!(pu.regions.len(), 2);
    assert_ne!(pu.regions[0].key, pu.regions[1].key);
}

/// An object's drawing planes: where a drawn shape split a face, and where
/// it owns drawing; a bare box has none.
#[test]
fn object_drawing_planes_find_split_faces_and_owned_drawing() {
    let (mut doc, o) = boxdoc();
    assert!(doc.object_drawing_planes(o).is_empty(), "a bare box");
    draft_top(&mut doc, o, "Rect", |w| rect(w, 2., 1., 6., 4.));
    let planes = doc.object_drawing_planes(o);
    assert_eq!(planes.len(), 1);
    assert!(planes[0].normal().z > 0.99 && (planes[0].signed_distance(z3(0., 0.))).abs() < 1e-9);
    // A loose line alone (owned drawing) on another box's top.
    let (mut doc2, o2) = boxdoc();
    draft_top(&mut doc2, o2, "Line", |w| {
        w.add_segment(z3(2., 2.), z3(4., 3.)).map(|_| ())
    });
    assert_eq!(doc2.object_drawing_planes(o2).len(), 1);
}

/// A line splitting a face into two differently painted halves is the
/// solid's content (its paint boundary), not drawing: it lists as no unit,
/// and drafting cannot erase it.
#[test]
fn a_painted_split_is_content_not_drawing() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(5., 0.), z3(5., 6.)).map(|_| ())
    });
    assert_eq!(doc.plane_units(top(), None, &[o]).unwrap().units.len(), 1);
    let m = red(&mut doc);
    let half = face_with(
        &doc,
        o,
        &rg(vec![z3(0., 0.), z3(5., 0.), z3(5., 6.), z3(0., 6.)], vec![]),
    );
    doc.paint_face(o, half, Some(m)).unwrap();
    assert!(
        doc.plane_units(top(), None, &[o]).unwrap().units.is_empty(),
        "the paint boundary is no drawing unit"
    );
    let before = doc.save();
    let erased = doc.draft(top(), None, &[o], "Erase", |w| {
        let e = w
            .edge_at_positions(z3(5., 0.), z3(5., 6.))
            .expect("the line");
        w.remove_edge(e).map(|_| ())
    });
    assert!(matches!(
        erased,
        Err(DocumentError::DraftBoundaryEdgeTouched)
    ));
    assert_eq!(doc.save(), before, "the refusal changes nothing");
}

/// A regular polygon drawn on a face keeps its identity in the solid: the
/// plane's units list it as one polygon run, it offers its center, and the
/// mark survives a move of the solid, undo/redo, and a save and reopen.
#[test]
fn a_polygon_on_a_face_stays_a_polygon() {
    let (mut doc, o) = boxdoc();
    let ring: Vec<Point3> = (0..6)
        .map(|i| {
            let a = TAU * i as f64 / 6.;
            z3(5. + a.cos(), 3. + a.sin())
        })
        .collect();
    draft_top(&mut doc, o, "Polygon", |w| {
        w.begin_curve_with_kind(
            CurveGeom {
                center: z3(5., 3.),
                radius: 1.,
            },
            kernel::SketchCurveKind::Polygon,
        )?;
        let r = draw_ring(w, &ring);
        w.end_curve();
        r
    });
    assert_eq!(sketch_edges(&doc), 0, "the polygon is face topology");
    let polygon_runs = |doc: &Document| -> usize {
        let pu = doc.plane_units(top(), None, &[o]).unwrap();
        pu.units
            .iter()
            .flat_map(|u| u.curves.iter())
            .filter(|r| r.kind == kernel::SketchCurveKind::Polygon && r.edges.len() == 6)
            .count()
    };
    assert_eq!(polygon_runs(&doc), 1);
    let centers = doc.object(o).unwrap().edge_polygon_centers();
    assert_eq!(centers.len(), 1);
    assert!(centers[0].0.approx_eq(z3(5., 3.), 1e-12));

    let reopened = Document::load(&doc.save()).expect("reopens");
    assert_eq!(reopened.save(), doc.save(), "the mark round-trips");
    assert_eq!(polygon_runs(&reopened), 1);

    doc.undo().unwrap();
    assert_eq!(polygon_runs(&doc), 0);
    doc.redo().unwrap();
    assert_eq!(polygon_runs(&doc), 1, "redo brings the polygon back whole");

    doc.transform_object(o, &shift(1., 0.)).unwrap();
    let moved = Plane::from_point_normal(p(0., 0., 3.), up()).unwrap();
    let pu = doc.plane_units(moved, None, &[o]).unwrap();
    assert!(
        pu.units
            .iter()
            .flat_map(|u| u.curves.iter())
            .any(|r| r.kind == kernel::SketchCurveKind::Polygon
                && r.geom.center.approx_eq(z3(6., 3.), 1e-12)),
        "the mark moves with the solid"
    );
}

/// The polygons drawn on one face with this circumcircle, by chain: each
/// polygon-kind curve of the lifted drawing and its edge count.
fn polygon_chains_on_top(doc: &mut Document, o: ObjectId) -> Vec<usize> {
    doc.begin_draft(top(), None, &[o], "Look").unwrap();
    let w = doc.draft_sketch().unwrap();
    let mut counts: std::collections::BTreeMap<_, usize> = std::collections::BTreeMap::new();
    for e in w.edges().values() {
        if let Some(c) = e.curve
            && w.curve_analytic(c)
                .is_some_and(|a| a.kind == kernel::SketchCurveKind::Polygon)
        {
            *counts.entry(c).or_default() += 1;
        }
    }
    doc.cancel_draft();
    counts.into_values().collect()
}

fn draw_polygon_on_top(doc: &mut Document, o: ObjectId, n: usize, phase: f64) {
    let ring: Vec<Point3> = (0..n)
        .map(|i| {
            let a = phase + TAU * i as f64 / n as f64;
            z3(5. + a.cos(), 3. + a.sin())
        })
        .collect();
    draft_top(doc, o, "Polygon", |w| {
        w.begin_curve_with_kind(
            CurveGeom {
                center: z3(5., 3.),
                radius: 1.,
            },
            kernel::SketchCurveKind::Polygon,
        )?;
        let r = draw_ring(w, &ring);
        w.end_curve();
        r
    });
}

/// Two polygons inscribed in one circle (a star's two triangles) stay two
/// polygons on a face, as on the ground: their sides cross, but neither
/// joins the other's chain.
#[test]
fn polygons_sharing_a_circle_stay_apart_on_a_face() {
    let (mut doc, o) = boxdoc();
    draw_polygon_on_top(&mut doc, o, 3, 0.0);
    draw_polygon_on_top(&mut doc, o, 3, TAU / 6.0);
    // Each side is cut twice by the other triangle: nine pieces apiece.
    assert_eq!(polygon_chains_on_top(&mut doc, o), vec![9, 9]);
    // ...and each keys apart, so a whole-curve pick names one triangle.
    let keys: std::collections::BTreeSet<u64> = doc
        .plane_units(top(), None, &[o])
        .unwrap()
        .units
        .iter()
        .flat_map(|u| u.curves.iter())
        .filter(|r| r.kind == kernel::SketchCurveKind::Polygon)
        .map(|r| r.curve_key)
        .collect();
    assert_eq!(keys.len(), 2, "two curve keys, one per triangle");
    let reopened = Document::load(&doc.save()).expect("reopens");
    assert_eq!(reopened.save(), doc.save());
}

/// A triangle and a square in one circle, started at one corner, stay two
/// polygons; a polygon whose corner is dragged inward is no longer regular
/// and lists as plain lines.
#[test]
fn polygon_marks_follow_the_shape() {
    let (mut doc, o) = boxdoc();
    draw_polygon_on_top(&mut doc, o, 3, 0.0);
    draw_polygon_on_top(&mut doc, o, 4, 0.0);
    let mut chains = polygon_chains_on_top(&mut doc, o);
    chains.sort_unstable();
    assert_eq!(chains.len(), 2, "two polygons: {chains:?}");

    let (mut doc, o) = boxdoc();
    draw_polygon_on_top(&mut doc, o, 6, 0.0);
    doc.draft(top(), None, &[o], "Edit Vertex", |w| {
        let v = w
            .vertices()
            .iter()
            .find(|(_, x)| x.position.approx_eq(z3(6., 3.), 1e-9))
            .map(|(id, _)| id)
            .expect("the hexagon's corner at angle 0");
        w.move_vertex_arranging(v, z3(5.7, 3.)).map(|_| ())
    })
    .unwrap();
    let mut chains = polygon_chains_on_top(&mut doc, o);
    chains.sort_unstable();
    assert_eq!(
        chains,
        vec![4],
        "the two sides at the dragged corner drop the mark; four stay"
    );
}

/// The same star drawn the way the Polygon tool draws it: off-grid
/// centres and a slightly different radius each time, so the crossings are
/// computed (and snapped) rather than exact. Both stay polygons.
#[test]
fn a_star_drawn_off_grid_stays_two_polygons() {
    let (mut doc, o) = boxdoc();
    let tri = |doc: &mut Document, cx: f64, cy: f64, r: f64, phase: f64| {
        let ring: Vec<Point3> = (0..3)
            .map(|i| {
                let a = phase + TAU * i as f64 / 3.0;
                z3(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        draft_top(doc, o, "Polygon", |w| {
            w.begin_curve_with_kind(
                CurveGeom {
                    center: z3(cx, cy),
                    radius: r,
                },
                kernel::SketchCurveKind::Polygon,
            )?;
            let res = draw_ring(w, &ring);
            w.end_curve();
            res
        });
    };
    tri(&mut doc, 4.999_871_3, 3.000_219_7, 1.000_013_1, 0.000_37);
    tri(
        &mut doc,
        4.999_871_3,
        3.000_219_7,
        0.999_947_9,
        TAU / 6.0 + 0.000_21,
    );
    let mut chains = polygon_chains_on_top(&mut doc, o);
    chains.sort_unstable();
    assert_eq!(chains, vec![9, 9]);
}

/// A star exactly as the desktop Polygon tool drew it (vertices read back
/// from the app): both triangles stay polygons.
#[test]
fn a_star_from_the_app_stays_two_polygons() {
    let (mut doc, o) = boxdoc();
    let tri = |doc: &mut Document, v: [(f64, f64); 3]| {
        let ring: Vec<Point3> = v.iter().map(|&(x, y)| z3(x, y)).collect();
        let c = z3(
            (v[0].0 + v[1].0 + v[2].0) / 3.0,
            (v[0].1 + v[1].1 + v[2].1) / 3.0,
        );
        let r = (ring[0] - c).length();
        draft_top(doc, o, "Polygon", |w| {
            w.begin_curve_with_kind(
                CurveGeom {
                    center: c,
                    radius: r,
                },
                kernel::SketchCurveKind::Polygon,
            )?;
            let res = draw_ring(w, &ring);
            w.end_curve();
            res
        });
    };
    tri(
        &mut doc,
        [
            (0.7516224392194942, 1.7723085993379057),
            (1.4999724392194946, 2.2043686732859613),
            (0.7516224392194952, 2.636428747234018),
        ],
    );
    tri(
        &mut doc,
        [
            (1.2505289652287763, 2.629946317942469),
            (0.5077831246595718, 2.2076155396215595),
            (1.244905227770136, 1.7755441622938566),
        ],
    );
    let mut chains = polygon_chains_on_top(&mut doc, o);
    chains.sort_unstable();
    assert_eq!(chains, vec![9, 9]);
}

/// Lines drawn between two circles' quadrants meet the circles even when
/// no facet corner sits at a quadrant (the circle drawn from a radius point
/// off the axes): the facet under each quadrant splits there, and the
/// circles and lines enclose the rounded rectangle between them.
#[test]
fn lines_to_a_circles_quadrants_close_regions() {
    for on_top in [true, false] {
        let (mut doc, o) = boxdoc();
        let z = if on_top { 3.0 } else { 0.0 };
        let at = |x: f64, y: f64| p(x, y, z);
        let ring = |cx: f64, cy: f64| -> Vec<Point3> {
            (0..32)
                .map(|i| {
                    let a = -0.0103 + TAU * i as f64 / 32.0;
                    at(cx + 0.5 * a.cos(), cy + 0.5 * a.sin())
                })
                .collect()
        };
        let plane = Plane::from_point_normal(at(0.0, 0.0), up()).unwrap();
        let objs: Vec<ObjectId> = if on_top { vec![o] } else { vec![] };
        let (gx, gy) = if on_top { (3.0, 3.0) } else { (13.0, 3.0) };
        for cx in [gx, gx + 2.0] {
            let pts = ring(cx, gy);
            doc.draft(plane, None, &objs, "Circle", |w| {
                w.begin_curve_with(CurveGeom {
                    center: at(cx, gy),
                    radius: 0.5,
                })?;
                let r = draw_ring(w, &pts);
                w.end_curve();
                r
            })
            .unwrap();
        }
        for dy in [0.5, -0.5] {
            doc.draft(plane, None, &objs, "Line", |w| {
                w.add_segment(at(gx, gy + dy), at(gx + 2.0, gy + dy))
                    .map(|_| ())
            })
            .unwrap();
        }
        let pu = doc.plane_units(plane, None, &objs).unwrap();
        assert_eq!(pu.units.len(), 1, "one connected drawing (top: {on_top})");
        // The rounded rectangle's middle (between the circles, inside the
        // lines) is a region of its own.
        let mid = at(gx + 1.0, gy);
        let holds = pu.regions.iter().any(|r| {
            let xs: Vec<f64> = r.outer.iter().map(|q| q.x).collect();
            let ys: Vec<f64> = r.outer.iter().map(|q| q.y).collect();
            let inside = xs.iter().cloned().fold(f64::MAX, f64::min) < mid.x
                && xs.iter().cloned().fold(f64::MIN, f64::max) > mid.x
                && ys.iter().cloned().fold(f64::MIN, f64::max) <= gy + 0.5 + 1e-9
                && ys.iter().cloned().fold(f64::MAX, f64::min) >= gy - 0.5 - 1e-9;
            inside && r.holes.is_empty()
        });
        assert!(
            holds,
            "the middle region exists (top: {on_top}); regions: {}",
            pu.regions.len()
        );
    }
}

/// A polygon pushed into a boss keeps its mark on both rims (the boss top's
/// centre still snaps), and an imprinted polygon slid across its face
/// carries the mark with it.
#[test]
fn a_pushed_or_slid_polygon_keeps_its_mark() {
    let (mut doc, o) = boxdoc();
    draw_polygon_on_top(&mut doc, o, 6, 0.0);
    let hex = |doc: &Document| {
        let obj = doc.object(o).unwrap();
        top_face_ids(doc, o)
            .into_iter()
            .find(|&f| obj.loop_positions(obj.faces()[f].outer_loop).count() == 6)
            .unwrap()
    };
    let centers = |doc: &Document| {
        let mut c: Vec<Point3> = doc
            .object(o)
            .unwrap()
            .edge_polygon_centers()
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        c.sort_by(|a, b| a.z.total_cmp(&b.z));
        c
    };
    let before = doc.save();

    let face = hex(&doc);
    doc.apply_object_op(
        o,
        kernel::KernelOp::ExtrudeSubFace {
            sub_face: face,
            distance: 0.5,
        },
    )
    .unwrap();
    let c = centers(&doc);
    assert_eq!(c.len(), 2, "the base rim and the boss top: {c:?}");
    assert!(c[0].approx_eq(z3(5., 3.), 1e-9));
    assert!(c[1].approx_eq(p(5., 3., 3.5), 1e-9));
    undo_redo_exact(&mut doc, &before);
    doc.undo().unwrap();

    let face = hex(&doc);
    doc.apply_object_op(
        o,
        kernel::KernelOp::TransformSubFace {
            sub_face: face,
            xf: shift(2., 0.),
            pinned: vec![],
        },
    )
    .unwrap();
    let c = centers(&doc);
    assert_eq!(c.len(), 1);
    assert!(
        c[0].approx_eq(z3(7., 3.), 1e-9),
        "the mark slid along: {c:?}"
    );
}

/// A shape moved across a solid's edge and back is whole again, as on the
/// ground: the points where the edge split its sides heal once they are off
/// the edge (and a line drawn in two collinear clicks keeps its vertex).
#[test]
fn a_shape_moved_across_the_edge_and_back_is_whole_again() {
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Rect", |w| rect(w, 7., 1., 9., 2.));
    let move_all = |doc: &mut Document, dx: f64| {
        doc.draft(top(), None, &[o], "Move", |w| {
            let ids = edges_along(
                w,
                &rect_sides(
                    7. + if dx < 0. { 2. } else { 0. },
                    1.,
                    9. + if dx < 0. { 2. } else { 0. },
                    2.,
                ),
            );
            w.apply_transform_edges(&ids, &shift(dx, 0.))
        })
        .unwrap();
    };
    move_all(&mut doc, 2.);
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    assert!(
        pu.units.iter().any(|u| u.edges.len() == 6),
        "straddling: split at x = 10"
    );
    move_all(&mut doc, -2.);
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    assert_eq!(pu.units.len(), 1);
    assert_eq!(
        pu.units[0].edges.len(),
        4,
        "the sides healed: {:?}",
        pu.units[0].edges
    );
    assert_faces(
        &doc,
        o,
        &[
            rg(rect_ring(7., 1., 9., 2.), vec![]),
            top_with_hole(rect_ring(7., 1., 9., 2.)),
        ],
    );
    let obj = doc.object(o).unwrap();
    let outer_counts: Vec<usize> = top_face_ids(&doc, o)
        .into_iter()
        .map(|f| obj.loop_positions(obj.faces()[f].outer_loop).count())
        .collect();
    assert!(
        outer_counts.iter().all(|&n| n == 4),
        "the solid's own edge is whole again too: {outer_counts:?}"
    );

    // A deliberate collinear vertex (two clicks along one line) survives.
    let (mut doc, o) = boxdoc();
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(2., 2.), z3(3., 2.)).map(|_| ())
    });
    draft_top(&mut doc, o, "Line", |w| {
        w.add_segment(z3(3., 2.), z3(4., 2.)).map(|_| ())
    });
    doc.draft(top(), None, &[o], "Move", |w| {
        let ids = edges_along(w, &[(z3(2., 2.), z3(4., 2.))]);
        w.apply_transform_edges(&ids, &shift(0., 1.))
    })
    .unwrap();
    let pu = doc.plane_units(top(), None, &[o]).unwrap();
    assert_eq!(pu.units[0].edges.len(), 2, "a vertex the user placed stays");
}

/// A loop with no face (sketch rule 6) stays one through the drafting
/// engine: an edit on its plane that leaves it alone rebuilds the sketch
/// without giving it a face, the plane's units list it as not
/// extrudable, and drawing over one of its edges in a draft gives it its
/// face as drawing anywhere else does.
#[test]
fn a_faceless_loop_survives_a_draft_edit_until_an_edge_is_drawn_over() {
    let g = |x: f64, y: f64| p(x, y, 0.0);
    let mut doc = Document::new();
    let s = doc.add_sketch(ground_plane());
    {
        let sk = doc.sketch_mut(s).unwrap();
        for (a, b) in [
            (g(0.0, 0.0), g(2.0, 0.0)),
            (g(2.0, 0.0), g(2.0, 2.0)),
            (g(2.0, 2.0), g(0.0, 2.0)),
            (g(0.0, 2.0), g(0.0, 0.0)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
        sk.make_regions_faceless();
    }
    assert!(doc.extrudable_regions(s).unwrap().is_empty());
    let units = doc.plane_units(ground_plane(), None, &[]).unwrap();
    assert_eq!(units.regions.len(), 1);
    assert!(units.regions[0].extrude.is_none(), "nothing to extrude");

    // A stub from a corner: the loop is rebuilt, still with no face.
    doc.draft(ground_plane(), None, &[], "Line", |w| {
        w.add_segment(g(2.0, 2.0), g(4.0, 4.0)).map(|_| ())
    })
    .unwrap();
    let live: Vec<_> = doc.sketch_ids();
    assert!(
        live.iter()
            .all(|&id| doc.extrudable_regions(id).unwrap().is_empty()),
        "a stub closes no loop"
    );
    // A line ending on an edge splits it without closing anything either.
    doc.draft(ground_plane(), None, &[], "Line", |w| {
        w.add_segment(g(1.0, 0.0), g(1.0, -1.0)).map(|_| ())
    })
    .unwrap();
    assert!(
        doc.sketch_ids()
            .iter()
            .all(|&id| doc.extrudable_regions(id).unwrap().is_empty())
    );
    // Drawn over, the loop has its face.
    doc.draft(ground_plane(), None, &[], "Line", |w| {
        w.add_segment(g(0.0, 0.0), g(0.0, 2.0)).map(|_| ())
    })
    .unwrap();
    assert_eq!(
        doc.sketch_ids()
            .iter()
            .map(|&id| doc.extrudable_regions(id).unwrap().len())
            .sum::<usize>(),
        1
    );
}

/// The whole frame's drawing listed in one pass is the same listing as
/// asking plane by plane: the same planes, each with the same units and
/// regions, objects with a face on a plane taking part, excluded ones not.
#[test]
fn the_free_planes_listed_at_once_match_the_listing_plane_by_plane() {
    let (mut doc, b) = boxdoc();
    // Drawing on the ground beside the box, on the box's top plane, and on
    // a plane of its own.
    let ground = doc.add_sketch(ground_plane());
    {
        let sk = doc.sketch_mut(ground).unwrap();
        sk.add_segment(p(11.0, 0.0, 0.0), p(12.0, 0.0, 0.0))
            .unwrap();
        sk.add_segment(p(12.0, 0.0, 0.0), p(12.0, 1.0, 0.0))
            .unwrap();
    }
    let on_top = doc.add_sketch(top());
    {
        let sk = doc.sketch_mut(on_top).unwrap();
        for (a, c) in [
            (p(0.2, 0.2, 3.0), p(0.6, 0.2, 3.0)),
            (p(0.6, 0.2, 3.0), p(0.6, 0.6, 3.0)),
            (p(0.6, 0.6, 3.0), p(0.2, 0.6, 3.0)),
            (p(0.2, 0.6, 3.0), p(0.2, 0.2, 3.0)),
        ] {
            sk.add_segment(a, c).unwrap();
        }
    }
    let tilted = Plane::from_point_normal(p(0.0, 0.0, 5.0), Vec3::new(0.0, 1.0, 1.0)).unwrap();
    let apart = doc.add_sketch(tilted);
    doc.sketch_mut(apart)
        .unwrap()
        .add_segment(p(0.0, 0.0, 5.0), p(1.0, 0.0, 5.0))
        .unwrap();

    let all = doc.free_plane_units(None, &std::collections::BTreeSet::new());
    assert_eq!(all.len(), 3, "three planes carry free drawing");
    for (plane, units) in &all {
        let objects = doc.drawable_objects_on_plane(*plane, None);
        let one = doc.plane_units(*plane, None, &objects).unwrap();
        assert_eq!(units.units.len(), one.units.len(), "units on {plane:?}");
        assert_eq!(
            units.units.iter().map(|u| u.key).collect::<Vec<_>>(),
            one.units.iter().map(|u| u.key).collect::<Vec<_>>()
        );
        assert_eq!(units.regions.len(), one.regions.len());
        for (r1, r2) in units.regions.iter().zip(&one.regions) {
            assert_eq!(r1.key, r2.key);
            assert_eq!(r1.over.map(|(o, _)| o), r2.over.map(|(o, _)| o));
        }
    }
    // The box takes part on its top plane: the square drawn there is a
    // region over its face.
    let top_units = all
        .iter()
        .find(|(pl, _)| {
            (pl.normal() - up()).length() < 1e-9
                && pl.signed_distance(p(0.0, 0.0, 3.0)).abs() < 1e-9
        })
        .map(|(_, u)| u)
        .expect("the top plane");
    assert!(
        top_units
            .regions
            .iter()
            .any(|r| r.over.is_some_and(|(o, _)| o == b))
    );
    // Excluded (hidden on screen), the box takes no part.
    let without = doc.free_plane_units(None, &std::collections::BTreeSet::from([b]));
    let top_without = without
        .iter()
        .find(|(pl, _)| {
            (pl.normal() - up()).length() < 1e-9
                && pl.signed_distance(p(0.0, 0.0, 3.0)).abs() < 1e-9
        })
        .map(|(_, u)| u)
        .expect("the top plane");
    assert!(top_without.regions.iter().all(|r| r.over.is_none()));
}

/// A face plane and a sketch plane that coincide within tolerance can
/// round to neighboring buckets of the one-pass listing; the object still
/// takes part, exactly as the plane-by-plane listing has it.
#[test]
fn the_one_pass_listing_finds_an_object_across_a_bucket_edge() {
    let mut doc = Document::new();
    let g = doc.add_sketch(ground_plane());
    {
        let sk = doc.sketch_mut(g).unwrap();
        for (a, b) in [
            (p(0., 0., 0.), p(1., 0., 0.)),
            (p(1., 0., 0.), p(1., 1., 0.)),
            (p(1., 1., 0.), p(0., 1., 0.)),
            (p(0., 1., 0.), p(0., 0., 0.)),
        ] {
            sk.add_segment(a, b).unwrap();
        }
    }
    let region = doc.extrudable_regions(g).unwrap()[0];
    // A height a hair below a half-micrometer boundary; the drawing's plane
    // a hair above it, within the plane tolerance of the box's top.
    let height = 0.00158749999995;
    let (b, _) = doc.extrude_region(g, region, height).unwrap();
    let plane = Plane::from_point_normal(p(0.0, 0.0, 0.0015875000001), up()).unwrap();
    let s = doc.add_sketch(plane);
    {
        let sk = doc.sketch_mut(s).unwrap();
        let z = 0.0015875000001;
        for (a, c) in [
            (p(0.2, 0.2, z), p(0.6, 0.2, z)),
            (p(0.6, 0.2, z), p(0.6, 0.6, z)),
            (p(0.6, 0.6, z), p(0.2, 0.6, z)),
            (p(0.2, 0.6, z), p(0.2, 0.2, z)),
        ] {
            sk.add_segment(a, c).unwrap();
        }
    }
    let all = doc.free_plane_units(None, &std::collections::BTreeSet::new());
    let (listed_plane, units) = all
        .iter()
        .find(|(pl, _)| pl.signed_distance(p(0.0, 0.0, height)).abs() < 1e-6)
        .expect("the top plane");
    let objects = doc.drawable_objects_on_plane(*listed_plane, None);
    assert_eq!(objects, vec![b], "the box has a face on the plane");
    let one = doc.plane_units(*listed_plane, None, &objects).unwrap();
    assert_eq!(
        units.regions.iter().filter(|r| r.over.is_some()).count(),
        one.regions.iter().filter(|r| r.over.is_some()).count(),
        "the square over the box's top is a region over its face either way"
    );
    assert!(
        units
            .regions
            .iter()
            .any(|r| r.over.is_some_and(|(o, _)| o == b))
    );
}
