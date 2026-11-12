//! Executable specs for "a closed set of faces is a solid" (wireframe faces
//! and solids, §5; rule 4) and for push-through keeping the source's name
//! and tags (§5.6).
//!
//! The contract under test:
//! - when an edit leaves flat faces (faced regions of free sketches) that
//!   enclose a space with no gap, they become ONE watertight solid in the
//!   same undo step, the contributing sketches losing the consumed regions'
//!   scaffolding (an emptied sketch is deleted);
//! - closed against a growable Object's face (a cappable rim), the shape
//!   grows that Object, which keeps its name, tags and imprints; drawn
//!   Detached, or against a non-growable Object, it is a solid of its own;
//!   a refused union leaves a solid of its own too, reported as such;
//! - a cell inside the solid it closes against changes nothing (§9 D3);
//! - flaps are pruned, a partition wall makes one solid per room (§9 D6),
//!   and a face with a hole can make a solid of genus one;
//! - a decline leaves the document untouched; every outcome validates,
//!   saves and reopens; one undo restores the drawing exactly;
//! - push-through's result pieces keep the source's name and tags.

use kernel::{
    CellDeclineReason, CellOutcome, CellScope, CellSeed, Document, DraftReport, NodeId, Object,
    ObjectId, Plane, Point3, SketchId, Transform, Vec3, WatertightState,
};
use proptest::prelude::*;

// ------------------------------------------------------------ fixtures

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane(at: Point3, n: Vec3) -> Plane {
    Plane::from_point_normal(at, n).unwrap()
}

fn ground() -> Plane {
    plane(p(0., 0., 0.), Vec3::new(0., 0., 1.))
}

/// A draft drawing `segs` on `on` with the objects a draw tool would pass.
fn draw(doc: &mut Document, on: Plane, segs: &[(Point3, Point3)]) -> DraftReport {
    let objects = doc.drawable_objects_on_plane(on, None);
    let segs = segs.to_vec();
    doc.draft(on, None, &objects, "Line", move |w| {
        for &(a, b) in &segs {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .expect("the draft lowers")
    .0
}

/// A detached draft drawing `segs` on `on` as one new shape.
fn draw_detached(doc: &mut Document, on: Plane, segs: &[(Point3, Point3)]) -> DraftReport {
    let segs = segs.to_vec();
    doc.draft_detached_new(on, None, "Line", move |w| {
        for &(a, b) in &segs {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .expect("the detached draft lowers")
    .0
}

/// The vertical plane through `a–b` (for a vertical line, facing +x).
fn home_of(a: Point3, b: Point3) -> Plane {
    let d = b - a;
    let n = d.cross(Vec3::new(0., 0., 1.));
    if n.length() > 1e-6 {
        plane(a, n)
    } else {
        plane(a, Vec3::new(1., 0., 0.))
    }
}

/// One line drawn on the vertical plane through it, as Line re-homes it.
fn line(doc: &mut Document, a: Point3, b: Point3) -> DraftReport {
    draw(doc, home_of(a, b), &[(a, b)])
}

fn line_on(doc: &mut Document, a: Point3, b: Point3, n: Vec3) -> DraftReport {
    draw(doc, plane(a, n), &[(a, b)])
}

fn ring(pts: &[Point3]) -> Vec<(Point3, Point3)> {
    (0..pts.len())
        .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
        .collect()
}

fn same_corners(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len() && a.iter().all(|&q| b.iter().any(|&r| q.approx_eq(r, 1e-9)))
}

/// Every faced region of every live world sketch: (sketch, outer ring).
fn faces(doc: &Document) -> Vec<(SketchId, Vec<Point3>)> {
    let mut out = Vec::new();
    for s in doc.sketch_ids() {
        let sk = doc.sketch(s).unwrap();
        for r in sk.regions().values().filter(|r| !r.faceless) {
            out.push((
                s,
                r.outer.iter().map(|&v| sk.vertices()[v].position).collect(),
            ));
        }
    }
    out
}

fn face_with(doc: &Document, corners: &[Point3]) -> Option<SketchId> {
    faces(doc)
        .into_iter()
        .find(|(_, r)| same_corners(r, corners))
        .map(|(s, _)| s)
}

/// Whether some live sketch holds an edge `a–b`.
fn held(doc: &Document, a: Point3, b: Point3) -> usize {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| {
            let sk = doc.sketch(s).unwrap();
            sk.edges().values().any(|e| {
                let (c, d) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
                (c.approx_eq(a, 1e-9) && d.approx_eq(b, 1e-9))
                    || (c.approx_eq(b, 1e-9) && d.approx_eq(a, 1e-9))
            })
        })
        .count()
}

/// The signed volume of `obj` (divergence theorem over every loop).
fn volume(obj: &Object) -> f64 {
    let mut six = 0.0;
    for f in obj.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let q: Vec<Vec3> = obj.loop_positions(l).map(|x| x.to_vec()).collect();
            for i in 1..q.len().saturating_sub(1) {
                six += q[0].dot(q[i].cross(q[i + 1]));
            }
        }
    }
    six / 6.0
}

/// Whether `obj` has a face whose outer loop has exactly `corners`.
fn object_face_with(obj: &Object, corners: &[Point3]) -> bool {
    obj.faces().values().any(|f| {
        let r: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
        same_corners(&r, corners)
    })
}

/// A solid: watertight, valid, with the expected volume.
fn solid(doc: &Document, id: ObjectId, vol: f64) {
    let obj = doc.object(id).expect("a visible object");
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj.validate().expect("the solid validates");
    assert!(
        (volume(obj) - vol).abs() <= 1e-9 * vol.max(1.0),
        "volume {} expected {vol}",
        volume(obj)
    );
}

fn reopens(doc: &Document) {
    let bytes = doc.save();
    let back = Document::load(&bytes).expect("the saved bytes reopen");
    assert_eq!(back.state_hash(), doc.state_hash(), "reopens unchanged");
}

fn one_step(doc: &mut Document, before: u64) {
    let after = doc.state_hash();
    doc.undo().expect("undo");
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
    doc.redo().expect("redo");
    assert_eq!(doc.state_hash(), after, "redo returns the edit's state");
}

/// The only `Created` object in `cells`.
fn created(cells: &[CellOutcome]) -> ObjectId {
    match cells {
        [CellOutcome::Created(id)] => *id,
        other => panic!("expected one new solid, got {other:?}"),
    }
}

const A: Point3 = Point3::new(0., 0., 0.);
const B: Point3 = Point3::new(1., 0., 0.);
const C: Point3 = Point3::new(1., 1., 0.);
const D: Point3 = Point3::new(0., 1., 0.);
const APEX: Point3 = Point3::new(0.5, 0.5, 1.);
const FOOT: Point3 = Point3::new(0.5, 0.5, 0.);

fn up(q: Point3, z: f64) -> Point3 {
    p(q.x, q.y, z)
}

/// A 1 m box standing on the ground, by extrusion.
fn unit_box(doc: &mut Document) -> ObjectId {
    let s = draw(doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap().0
}

// ------------------------------------------------------------ no caps

/// §11: Kurt's pyramid. The fourth slanted line closes the last two sides
/// and the five faces become one solid; every contributing sketch is gone,
/// the construction line inside stays, and one undo restores every edge.
#[test]
fn a_pyramid_of_lines_becomes_one_solid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let g = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    line_on(&mut doc, FOOT, APEX, Vec3::new(0., 1., 0.));
    for q in [A, B, C] {
        let r = line(&mut doc, APEX, q);
        assert!(r.cells.is_empty(), "nothing closes before the last line");
        assert!(doc.visible_object_ids().is_empty());
    }
    let sketches_before = doc.sketch_ids();
    let faces_before = faces(&doc).len();
    let before = doc.state_hash();
    let r = line(&mut doc, APEX, D);
    let id = created(&r.cells);
    solid(&doc, id, 1.0 / 3.0);
    assert_eq!(doc.object(id).unwrap().faces().len(), 5);
    assert_eq!(doc.visible_object_ids(), vec![id]);
    // Every face's sketch is gone; the construction line inside stays.
    assert!(faces(&doc).is_empty());
    assert!(doc.sketch(g).is_none(), "the ground sketch became the base");
    for s in sketches_before {
        let left = doc.sketch(s);
        assert!(
            left.is_none() || left.unwrap().regions().is_empty(),
            "no face is left in a contributing sketch"
        );
    }
    assert_eq!(held(&doc, FOOT, APEX), 1, "the construction line stays");
    reopens(&doc);
    // One undo: the last line and both fills gone, every other edge back.
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before);
    assert_eq!(faces(&doc).len(), faces_before);
    assert!(doc.visible_object_ids().is_empty());
    doc.redo().unwrap();
    assert_eq!(doc.visible_object_ids(), vec![id]);
    one_step(&mut doc, before);
}

/// The change feed reports what a drawn closing line did through its
/// follow-ups: the solid the cells search made, and the scaffolding it
/// consumed, in the same record as the line itself.
#[test]
fn a_drawn_solid_is_in_the_change_feed() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    draw(&mut doc, ground(), &ring(&[A, B, C, D]));
    line_on(&mut doc, FOOT, APEX, Vec3::new(0., 1., 0.));
    for q in [A, B, C] {
        line(&mut doc, APEX, q);
    }
    let generation = doc.history_generation();
    let r = line(&mut doc, APEX, D);
    let id = created(&r.cells);
    let changes = doc
        .changes_since_generation(generation)
        .expect("the ring reaches back one step");
    assert!(
        changes
            .added
            .iter()
            .any(|c| c.entity == kernel::EntityRef::Object(id)),
        "the commit's record names the solid it made: {changes:?}"
    );
}

/// §16 R20: a sketch move or copy that closes a solid reports it as a draw
/// does ([`Document::sketch_edit_report`]): the pyramid's last slanted line,
/// drawn away from it and moved (or copied) into place, closes the last two
/// sides and the solid, and the report names the moved sketch (or the copy)
/// first, the faces the pass closed, and the solid. One undo step.
#[test]
fn a_sketch_move_or_copy_reports_the_solid_it_closes() {
    for copy in [false, true] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        draw(&mut doc, ground(), &ring(&[A, B, C, D]));
        for q in [A, B, C] {
            line(&mut doc, APEX, q);
        }
        let away = Vec3::new(4., 0., 0.);
        let s = line(&mut doc, APEX + away, D + away).sketches[0];
        let islands: Vec<_> = doc.sketch(s).unwrap().islands().keys().collect();
        let back = Transform::translation(-away);
        let before = doc.state_hash();
        let change = if copy {
            doc.copy_sketch_islands(s, &islands, &back).unwrap().1
        } else {
            doc.transform_sketch_islands(s, &islands, &back).unwrap()
        };
        let report = doc.sketch_edit_report(&change);
        assert!(report.objects.is_empty() && report.kept_as_sketch.is_empty());
        assert_eq!(report.sketches, change.sketches_touched);
        if copy {
            assert_ne!(report.sketches[0], s, "the copy, first");
        } else {
            assert_eq!(report.sketches[0], s, "the moved sketch, first");
        }
        assert_eq!(report.faces_closed, doc.faces_closed_by_last_edit());
        assert!(!report.faces_closed.is_empty(), "the last sides closed");
        let id = created(&report.cells);
        solid(&doc, id, 1.0 / 3.0);
        reopens(&doc);
        one_step(&mut doc, before);
        // Any other edit after it reports nothing closed.
        doc.set_node_name(NodeId::Object(id), Some("Pyramid".into()))
            .unwrap();
        let other = doc.sketch_edit_report(&change);
        assert!(other.faces_closed.is_empty() && other.cells.is_empty());
    }
}

/// R14: loose-line trim is same-kind. An ordinary pyramid leaves a
/// Detached line along its edge alone, and a Detached pyramid leaves an
/// ordinary line along its edge alone.
#[test]
fn loose_line_trim_keeps_to_the_cells_kind() {
    for detached_cell in [false, true] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        // The other kind's loose line along the edge A–APEX.
        let (a, b) = (A, APEX);
        if detached_cell {
            draw(&mut doc, home_of(a, b), &[(a, b)]);
        } else {
            draw_detached(&mut doc, home_of(a, b), &[(a, b)]);
        }
        assert_eq!(held(&doc, A, APEX), 1);
        let mut last = DraftReport::default();
        if detached_cell {
            draw_detached(&mut doc, ground(), &ring(&[A, B, C, D]));
            for q in [A, B, C, D] {
                last = draw_detached(&mut doc, home_of(APEX, q), &[(APEX, q)]);
            }
        } else {
            draw(&mut doc, ground(), &ring(&[A, B, C, D]));
            for q in [A, B, C, D] {
                last = line(&mut doc, APEX, q);
            }
        }
        let id = created(&last.cells);
        solid(&doc, id, 1.0 / 3.0);
        assert_eq!(
            held(&doc, A, APEX),
            1,
            "detached cell {detached_cell}: the other kind's line stays"
        );
        reopens(&doc);
    }
}

/// One polyline as a raw ordinary sketch gesture (no close pass, no cells
/// search). Returns its sketch.
fn raw(doc: &mut Document, on: Plane, pts: &[Point3], closed: bool) -> SketchId {
    let s = doc.add_sketch(on);
    doc.begin_sketch_gesture(s).unwrap();
    let sk = doc.sketch_mut(s).unwrap();
    let n = if closed { pts.len() } else { pts.len() - 1 };
    for i in 0..n {
        sk.add_segment(pts[i], pts[(i + 1) % pts.len()]).unwrap();
    }
    doc.end_sketch_gesture(s).unwrap();
    s
}

/// R14 in a mixed cell: an open box of ordinary faces closed by a Detached
/// top. Each kind of loose line goes only along edges a face of its own
/// kind holds: an ordinary line along the vertical edge A–A′ (ordinary
/// walls only) goes and a Detached one there stays; an ordinary line along
/// the top edge A′–B′ (an ordinary wall's and the Detached top's) goes.
#[test]
fn a_mixed_cell_trims_each_kind_along_its_own_faces() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let [a1, b1, c1, d1] = [up(A, 1.), up(B, 1.), up(C, 1.), up(D, 1.)];
    raw(&mut doc, ground(), &[A, B, C, D], true);
    for (u, v, w, x) in [
        (A, B, b1, a1),
        (B, C, c1, b1),
        (C, D, d1, c1),
        (D, A, a1, d1),
    ] {
        let n = (v - u).cross(w - u);
        raw(&mut doc, plane(u, n), &[u, v, w, x], true);
    }
    let vertical = raw(&mut doc, home_of(A, B), &[A, a1], false);
    let top_edge = raw(&mut doc, plane(a1, Vec3::new(0., 1., 1.)), &[a1, b1], false);
    let detached_vertical =
        draw_detached(&mut doc, plane(A, Vec3::new(1., 1., 0.)), &[(A, a1)]).sketches[0];
    assert!(doc.visible_object_ids().is_empty(), "nothing closed yet");
    let last = draw_detached(
        &mut doc,
        plane(a1, Vec3::new(0., 0., 1.)),
        &ring(&[a1, b1, c1, d1]),
    );
    let id = created(&last.cells);
    solid(&doc, id, 1.0);
    let gone = |s: SketchId| doc.sketch(s).is_none_or(|sk| sk.edges().is_empty());
    assert!(
        gone(vertical),
        "the ordinary line along ordinary walls goes"
    );
    assert!(
        gone(top_edge),
        "the ordinary line along an ordinary wall's edge goes"
    );
    assert!(
        !gone(detached_vertical),
        "the Detached line along ordinary faces only stays"
    );
    assert_eq!(held(&doc, A, a1), 1);
    reopens(&doc);
}

/// §11: a wire cube, twelve lines each on a plane that is no face of the
/// cube: the last closes two faces and the cube becomes one solid.
#[test]
fn a_wire_cube_becomes_one_solid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    for (u, v) in ring(&[A, B, C, D]) {
        draw(&mut doc, ground(), &[(u, v)]);
    }
    for q in [A, B, C, D] {
        let n = Vec3::new(1., if (q.x - q.y).abs() < 0.5 { -1. } else { 1. }, 0.);
        line_on(&mut doc, q, up(q, 1.), n);
    }
    let tops = [(A, B), (B, C), (C, D), (D, A)];
    let mut last = DraftReport::default();
    let mut before = 0;
    for &(u, v) in &tops {
        let along = v - u;
        let out = along.cross(Vec3::new(0., 0., 1.));
        let n = out + Vec3::new(0., 0., out.length());
        before = doc.state_hash();
        last = line_on(&mut doc, up(u, 1.), up(v, 1.), n);
    }
    let id = created(&last.cells);
    solid(&doc, id, 1.0);
    assert_eq!(doc.object(id).unwrap().faces().len(), 6);
    assert!(doc.sketch_ids().is_empty(), "every line was a face's edge");
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §11: six Rectangles as a cube. The fifth Rectangle draws the last edge,
/// the close pass fills the sixth face itself, and the six become one
/// solid; every contributing sketch is deleted.
#[test]
fn six_rectangles_become_one_solid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let rects: [(Plane, [Point3; 4]); 5] = [
        (ground(), [A, B, C, D]),
        (
            plane(A, Vec3::new(0., -1., 0.)),
            [A, B, up(B, 1.), up(A, 1.)],
        ),
        (
            plane(B, Vec3::new(1., 0., 0.)),
            [B, C, up(C, 1.), up(B, 1.)],
        ),
        (
            plane(C, Vec3::new(0., 1., 0.)),
            [C, D, up(D, 1.), up(C, 1.)],
        ),
        (
            plane(up(A, 1.), Vec3::new(0., 0., 1.)),
            [up(A, 1.), up(B, 1.), up(C, 1.), up(D, 1.)],
        ),
    ];
    let mut last = DraftReport::default();
    let mut before = 0;
    for (i, (on, corners)) in rects.iter().enumerate() {
        before = doc.state_hash();
        last = draw(&mut doc, *on, &ring(corners));
        if i < 4 {
            assert!(last.cells.is_empty(), "rectangle {i} closes nothing");
        }
        reopens(&doc);
    }
    assert_eq!(last.faces_closed.len(), 1, "the sixth face fills itself");
    let id = created(&last.cells);
    solid(&doc, id, 1.0);
    assert_eq!(doc.object(id).unwrap().faces().len(), 6);
    assert!(doc.sketch_ids().is_empty(), "all six sketches are deleted");
    one_step(&mut doc, before);
    reopens(&doc);
}

/// §11: a flap on one edge does not stop the solid, and stays.
#[test]
fn a_flap_on_one_edge_stays_beside_the_solid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    draw(&mut doc, ground(), &ring(&[A, B, C, D]));
    // The flap: a rectangle hanging below edge A–B on the plane y = 0.
    let flap = [A, B, p(1., 0., -1.), p(0., 0., -1.)];
    draw(&mut doc, plane(A, Vec3::new(0., -1., 0.)), &ring(&flap));
    for q in [A, B, C] {
        line(&mut doc, APEX, q);
    }
    let r = line(&mut doc, APEX, D);
    let id = created(&r.cells);
    solid(&doc, id, 1.0 / 3.0);
    assert!(face_with(&doc, &flap).is_some(), "the flap stays a face");
    assert_eq!(faces(&doc).len(), 1);
    reopens(&doc);
}

/// §11 / §9 D6: two rooms closed by one last face are two solids.
#[test]
fn two_rooms_closed_by_one_face_are_two_solids() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (a, b, c, d) = (p(0., 0., 0.), p(2., 0., 0.), p(2., 1., 0.), p(0., 1., 0.));
    let (m0, m1) = (p(1., 0., 0.), p(1., 1., 0.));
    let mut segs = ring(&[a, b, c, d]);
    segs.push((m0, m1));
    draw(&mut doc, ground(), &segs);
    let walls: [(Plane, Vec<Point3>); 4] = [
        (
            plane(d, Vec3::new(0., 1., 0.)),
            vec![c, d, up(d, 1.), up(c, 1.)],
        ),
        (
            plane(a, Vec3::new(-1., 0., 0.)),
            vec![d, a, up(a, 1.), up(d, 1.)],
        ),
        (
            plane(b, Vec3::new(1., 0., 0.)),
            vec![b, c, up(c, 1.), up(b, 1.)],
        ),
        (
            plane(m0, Vec3::new(1., 0., 0.)),
            vec![m0, m1, up(m1, 1.), up(m0, 1.)],
        ),
    ];
    for (on, corners) in &walls {
        let r = draw(&mut doc, *on, &ring(corners));
        assert!(r.cells.is_empty());
    }
    assert!(doc.visible_object_ids().is_empty());
    let before = doc.state_hash();
    // The front wall closes both tops and both rooms at once.
    let r = draw(
        &mut doc,
        plane(a, Vec3::new(0., -1., 0.)),
        &ring(&[a, b, up(b, 1.), up(a, 1.)]),
    );
    assert_eq!(r.cells.len(), 2, "{:?}", r.cells);
    for c in &r.cells {
        let CellOutcome::Created(id) = c else {
            panic!("{c:?}")
        };
        solid(&doc, *id, 1.0);
    }
    assert!(faces(&doc).is_empty(), "the shared wall went to both rooms");
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §11: a face with a hole whose inner ring joins a tunnel makes one solid
/// of genus one (built from faces drawn without the close pass, so the
/// tunnel stays open, and closed through [`Document::close_cells`]).
#[test]
fn a_face_with_a_hole_joining_a_tunnel_is_one_solid_of_genus_one() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let sq = |lo: f64, hi: f64, z: f64| [p(lo, lo, z), p(hi, lo, z), p(hi, hi, z), p(lo, hi, z)];
    // A sketch from raw segments (no close pass runs), its inner square
    // made faceless: an annulus with an open hole.
    let raw =
        |doc: &mut Document, on: Plane, segs: &[(Point3, Point3)], hole: Option<[Point3; 4]>| {
            let s = doc.add_sketch(on);
            doc.begin_sketch_gesture(s).unwrap();
            let sk = doc.sketch_mut(s).unwrap();
            for &(a, b) in segs {
                sk.add_segment(a, b).unwrap();
            }
            if let Some(h) = hole {
                let rid = sk
                    .regions()
                    .iter()
                    .find(|(_, r)| {
                        let o: Vec<Point3> =
                            r.outer.iter().map(|&v| sk.vertices()[v].position).collect();
                        same_corners(&o, &h)
                    })
                    .map(|(id, _)| id)
                    .unwrap();
                sk.set_region_faceless(rid, true).unwrap();
            }
            doc.end_sketch_gesture(s).unwrap();
            s
        };
    let mut seeds = Vec::new();
    for z in [0., 1.] {
        let mut segs = ring(&sq(0., 3., z));
        segs.extend(ring(&sq(1., 2., z)));
        let on = plane(p(0., 0., z), Vec3::new(0., 0., 1.));
        seeds.push(raw(&mut doc, on, &segs, Some(sq(1., 2., z))));
    }
    for (lo, hi) in [(0., 3.), (1., 2.)] {
        let b = sq(lo, hi, 0.);
        for i in 0..4 {
            let (u, v) = (b[i], b[(i + 1) % 4]);
            let n = (v - u).cross(Vec3::new(0., 0., 1.));
            let quad = [u, v, up(v, 1.), up(u, 1.)];
            raw(&mut doc, plane(u, n), &ring(&quad), None);
        }
    }
    let before = doc.state_hash();
    let out = doc.close_cells(&[CellSeed::Sketch(seeds[0])], &CellScope::default());
    let id = created(&out);
    solid(&doc, id, 8.0);
    let obj = doc.object(id).unwrap();
    assert_eq!(obj.faces().len(), 10);
    assert_eq!(
        obj.faces()
            .values()
            .filter(|f| f.inner_loops.len() == 1)
            .count(),
        2,
        "the top and bottom are annuli"
    );
    // Genus one: V − E + F − H = 2 − 2g, with H the hole loops.
    let (v, e, f) = (obj.vertices().len(), obj.edges().len(), obj.faces().len());
    assert_eq!(
        v as i64 - e as i64 + f as i64 - 2,
        0,
        "Euler–Poincaré, genus 1"
    );
    reopens(&doc);
    one_step(&mut doc, before);
}

// ------------------------------------------------------------ caps

/// A hip roof drawn on a box's top: four rafters from a point above it.
fn hip_roof(doc: &mut Document, detached: bool) -> DraftReport {
    let ridge = p(0.5, 0.5, 1.5);
    let mut last = DraftReport::default();
    for q in [A, B, C, D] {
        let (a, b) = (ridge, up(q, 1.));
        last = if detached {
            draw_detached(doc, home_of(a, b), &[(a, b)])
        } else {
            line(doc, a, b)
        };
    }
    last
}

/// §11: a roof drawn on a box grows the box: one Object, its name, tags and
/// imprints kept, the old one hidden; one undo returns the box.
#[test]
fn a_roof_on_a_box_grows_it() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    doc.set_node_name(NodeId::Object(bx), Some("House".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(bx), vec!["Walls".into()])
        .unwrap();
    // An imprint on the front face.
    let door = [
        p(0.3, 0., 0.2),
        p(0.7, 0., 0.2),
        p(0.7, 0., 0.6),
        p(0.3, 0., 0.6),
    ];
    draw(&mut doc, plane(A, Vec3::new(0., -1., 0.)), &ring(&door));
    assert!(object_face_with(doc.object(bx).unwrap(), &door));
    let before_last = {
        let ridge = p(0.5, 0.5, 1.5);
        for q in [A, B, C] {
            line(&mut doc, ridge, up(q, 1.));
        }
        doc.state_hash()
    };
    let r = line(&mut doc, p(0.5, 0.5, 1.5), up(D, 1.));
    let [CellOutcome::Grew { from, to }] = r.cells[..] else {
        panic!("expected the box to grow, got {:?}", r.cells)
    };
    assert_eq!(from, bx);
    assert!(doc.object(bx).is_none(), "the old box is hidden");
    assert_eq!(doc.visible_object_ids(), vec![to]);
    solid(&doc, to, 1.0 + 1.0 / 6.0);
    assert_eq!(doc.object_name(to), Some("House"));
    assert_eq!(
        doc.node_tags(NodeId::Object(to)),
        &[vec!["Walls".to_string()]]
    );
    assert!(
        object_face_with(doc.object(to).unwrap(), &door),
        "the imprint survives"
    );
    assert!(faces(&doc).is_empty(), "the roof faces became the solid");
    reopens(&doc);
    one_step(&mut doc, before_last);
    doc.undo().unwrap();
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    assert_eq!(faces(&doc).len(), 2, "two roof faces back");
}

/// The forward change of a grow names every sketch it made: the grown
/// solid's copies of the old one's imprint sketches included.
#[test]
fn a_grow_reports_the_carried_imprint_sketches() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let door = [
        p(0.3, 0., 0.2),
        p(0.7, 0., 0.2),
        p(0.7, 0., 0.6),
        p(0.3, 0., 0.6),
    ];
    // A loose line on the front face stays an imprint sketch of the box.
    draw(
        &mut doc,
        plane(A, Vec3::new(0., -1., 0.)),
        &[(door[0], door[1])],
    );
    assert!(
        !doc.object_owned_sketches(bx).is_empty(),
        "an imprint sketch"
    );
    let ridge = p(0.5, 0.5, 1.5);
    for q in [A, B, C] {
        line(&mut doc, ridge, up(q, 1.));
    }
    let (a, b) = (ridge, up(D, 1.));
    let on = home_of(a, b);
    let objects = doc.drawable_objects_on_plane(on, None);
    let (r, change) = doc
        .draft(on, None, &objects, "Line", move |w| {
            w.add_segment(a, b)?;
            Ok(())
        })
        .unwrap();
    let [CellOutcome::Grew { to, .. }] = r.cells[..] else {
        panic!("expected the box to grow, got {:?}", r.cells)
    };
    let copies = doc.object_owned_sketches(to);
    assert!(!copies.is_empty(), "the imprint rides with the grown solid");
    for s in copies {
        assert!(
            change.sketches_touched.contains(&s),
            "the change names the carried sketch {s:?}: {:?}",
            change.sketches_touched
        );
    }
}

/// §11: the same roof drawn Detached is a solid of its own, caps included:
/// two Objects, the box untouched.
#[test]
fn a_detached_roof_on_a_box_is_its_own_solid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let r = hip_roof(&mut doc, true);
    let roof = created(&r.cells);
    solid(&doc, roof, 1.0 / 6.0);
    assert_eq!(
        doc.object(roof).unwrap().faces().len(),
        5,
        "the cap is a face"
    );
    let mut vis = doc.visible_object_ids();
    vis.sort();
    let mut want = vec![bx, roof];
    want.sort();
    assert_eq!(vis, want);
    let now = doc.object(bx).unwrap();
    assert_eq!(now.faces().len(), box_before.faces().len());
    assert!((volume(now) - 1.0).abs() < 1e-12);
    reopens(&doc);
}

/// §11 / §9 D3: a cell inside the solid it closes against changes nothing:
/// the faces stay, the box stays, and the outcome says why.
#[test]
fn a_cell_inside_a_solid_changes_nothing() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let low = p(0.5, 0.5, 0.5);
    let mut last = DraftReport::default();
    for q in [A, B, C, D] {
        last = line(&mut doc, low, up(q, 1.));
    }
    assert_eq!(
        last.cells,
        vec![CellOutcome::Declined(kernel::CellDecline {
            reason: CellDeclineReason::InsideSolid(bx),
            open_edges: Vec::new(),
            non_planar_loops: Vec::new(),
            overlapping_edges: Vec::new(),
        })]
    );
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    assert_eq!(
        doc.object(bx).unwrap().faces().len(),
        box_before.faces().len()
    );
    assert_eq!(faces(&doc).len(), 4, "the four faces stay");
    reopens(&doc);
}

/// §11: a union that refuses leaves the cell a solid of its own, and the
/// outcome says it could not be joined. The roof closes against an
/// imported open box (no bottom): its top is a face to cap on, but a
/// union needs two solids, so the engine refuses.
#[test]
fn a_refused_union_leaves_a_solid_of_its_own() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let positions = vec![A, B, C, D, up(A, 1.), up(B, 1.), up(C, 1.), up(D, 1.)];
    let faces_: Vec<Vec<usize>> = vec![
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces_.len();
    let open_box = kernel::MeshRecipe {
        name: "Open".into(),
        positions,
        faces: faces_,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    let scene = kernel::ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![kernel::ImportNode::Mesh(open_box)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    doc.ingest(scene, Vec::new()).unwrap();
    let shell = doc.visible_object_ids()[0];
    assert_eq!(
        doc.object(shell).unwrap().watertight(),
        WatertightState::Open
    );
    let ridge = p(0.5, 0.5, 1.5);
    for q in [A, B, C] {
        line(&mut doc, ridge, up(q, 1.));
    }
    let before = doc.state_hash();
    let r = line(&mut doc, ridge, up(D, 1.));
    let [CellOutcome::Unjoined { object, with }] = r.cells[..] else {
        panic!("expected a solid of its own, got {:?}", r.cells)
    };
    assert_eq!(with, shell);
    assert!(doc.object(shell).is_some(), "the shell is untouched");
    solid(&doc, object, 1.0 / 6.0);
    assert_eq!(
        doc.object(object).unwrap().faces().len(),
        5,
        "the cap is a face"
    );
    reopens(&doc);
    one_step(&mut doc, before);
}

/// Lens 4: an Object the view hides is never closed against, and a closed
/// group's member is never grown from the top level.
#[test]
fn hidden_and_grouped_objects_do_not_grow() {
    // Hidden: the box takes no part (§16 R11), so its top edges close no
    // roof face and the rafters stay lines; no cell.
    let mut doc = Document::new();
    let bx = unit_box(&mut doc);
    doc.set_view_hidden_objects([bx]);
    let r = hip_roof(&mut doc, false);
    assert!(r.cells.is_empty(), "{:?}", r.cells);
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    assert_eq!(faces(&doc).len(), 0);

    // Grouped: a closed group's member takes no part from the top level
    // (§16 R5, R11), so each roof face is drawn whole, the roof's rim fills
    // as a free face (the member's top covers nothing here), and the roof
    // is a solid of its own.
    let mut doc = Document::new();
    let bx = unit_box(&mut doc);
    doc.group_nodes(&[NodeId::Object(bx)]).unwrap();
    let ridge = p(0.5, 0.5, 1.5);
    let mut r = DraftReport::default();
    for (u, v) in ring(&[A, B, C, D]) {
        let (u, v) = (up(u, 1.), up(v, 1.));
        let n = (v - u).cross(ridge - u);
        r = draw(&mut doc, plane(u, n), &ring(&[u, v, ridge]));
    }
    let roof = created(&r.cells);
    solid(&doc, roof, 1.0 / 6.0);
    assert_eq!(
        doc.object(roof).unwrap().faces().len(),
        5,
        "four sides and a base"
    );
    assert!(doc.object(bx).is_some(), "the grouped box is untouched");
    reopens(&doc);
}

/// Lens 4, inside a definition: a member held by a group of the definition
/// takes no part in the definition's frame either (§16 R5, R11), so a hip
/// roof drawn there never grows it, the tree stays whole, and one undo
/// restores the document.
#[test]
fn a_grouped_definition_member_is_not_grown() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let (g, _) = doc.group_nodes(&[NodeId::Object(bx)]).unwrap();
    let (cid, _inst, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    let [NodeId::Object(member)] = doc.group_members(g).unwrap()[..] else {
        panic!("the group holds the box")
    };
    let member_before = doc.object(member).unwrap().clone();
    let draw_in = |doc: &mut Document, a: Point3, b: Point3| {
        let on = home_of(a, b);
        let objects = doc.drawable_objects_on_plane(on, Some(cid));
        doc.draft(on, Some(cid), &objects, "Line", move |w| {
            w.add_segment(a, b)?;
            Ok(())
        })
        .expect("the draft lowers")
        .0
    };
    let ridge = p(0.5, 0.5, 1.5);
    for q in [A, B, C] {
        draw_in(&mut doc, ridge, up(q, 1.));
    }
    let before = doc.state_hash();
    let r = draw_in(&mut doc, ridge, up(D, 1.));
    assert!(
        !r.cells
            .iter()
            .any(|c| matches!(c, CellOutcome::Grew { .. })),
        "the grouped member never grows: {:?}",
        r.cells
    );
    let now = doc.object(member).expect("the member stays shown");
    assert_eq!(now.faces().len(), member_before.faces().len());
    assert!((volume(now) - 1.0).abs() < 1e-12);
    assert_eq!(doc.node_parent(NodeId::Object(member)), Some(g));
    reopens(&doc);
    one_step(&mut doc, before);
}

// ------------------------------------------------------------ declines

/// A set of faces that closes nothing declines with its open edges, and
/// leaves the document untouched.
#[test]
fn open_faces_decline_untouched() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let g = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    line(&mut doc, APEX, A);
    line(&mut doc, APEX, B);
    line(&mut doc, APEX, C);
    let before = doc.state_hash();
    let depth = doc.history_entries().undo.len();
    let out = doc.close_cells(&[CellSeed::Sketch(g)], &CellScope::default());
    let [CellOutcome::Declined(d)] = &out[..] else {
        panic!("{out:?}")
    };
    assert_eq!(d.reason, CellDeclineReason::NotClosed);
    // Open: C–D and D–A of the ground, and the two free slanted edges.
    assert_eq!(d.open_edges.len(), 4, "{:?}", d.open_edges);
    assert_eq!(doc.state_hash(), before);
    assert_eq!(doc.history_entries().undo.len(), depth, "nothing recorded");
}

/// Lens 3, as §16 R22 rules it: a Detached rectangle drawn exactly on an
/// ordinary one is the same face drawn twice, not two faces lying over each
/// other. The pyramid closes on the ordinary base and the Detached
/// rectangle is left over, lying on the solid's base. (Faces that overlap
/// without being the same loop still decline:
/// `a_partial_overlap_declines_and_names_its_edges`.)
#[test]
fn the_same_face_drawn_twice_is_one_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    draw(&mut doc, ground(), &ring(&[A, B, C, D]));
    let twin = draw_detached(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    let kept = doc.sketch(twin).unwrap().clone();
    for q in [A, B, C] {
        line(&mut doc, APEX, q);
    }
    let before = doc.state_hash();
    let r = line(&mut doc, APEX, D);
    let id = created(&r.cells);
    solid(&doc, id, 1.0 / 3.0);
    assert_eq!(doc.sketch(twin), Some(&kept), "the Detached base stays");
    let rest: Vec<_> = faces(&doc).into_iter().map(|(s, _)| s).collect();
    assert_eq!(rest, vec![twin]);
    reopens(&doc);
    one_step(&mut doc, before);
}

// ------------------------------------------------------------ property

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// §11: random stacks of boxes drawn as faces yield solids whose
    /// volumes sum to the boxes'. The first box's faces make a new solid;
    /// each box drawn on top closes against it and grows it.
    #[test]
    fn stacks_of_boxes_drawn_as_faces_sum_their_volumes(
        dims in proptest::collection::vec((0.2f64..0.9, 0.2f64..0.9, 0.1f64..0.8), 1..4),
        order in 0usize..24,
    ) {
        let mut doc = Document::new();
        let mut z0 = 0.0;
        let mut total = 0.0;
        let mut size = 4.0;
        let mut ids: Vec<ObjectId> = Vec::new();
        for (k, &(fw, fd, h)) in dims.iter().enumerate() {
            // Each box narrower than the one below, centered on it.
            let (w, d) = (size * fw, size * fd);
            size = w.min(d);
            let (x0, y0) = (-w / 2.0, -d / 2.0);
            let c = [
                p(x0, y0, z0),
                p(x0 + w, y0, z0),
                p(x0 + w, y0 + d, z0),
                p(x0, y0 + d, z0),
            ];
            if k == 0 {
                draw(&mut doc, ground(), &ring(&c));
            }
            // The four walls in a permutation picked by `order`.
            let mut walls: Vec<usize> = (0..4).collect();
            let mut o = order;
            for i in (1..4).rev() {
                walls.swap(i, o % (i + 1));
                o /= i + 1;
            }
            let mut last = DraftReport::default();
            for i in walls {
                let (u, v) = (c[i], c[(i + 1) % 4]);
                let n = (v - u).cross(Vec3::new(0., 0., 1.));
                last = draw(&mut doc, plane(u, n), &ring(&[u, v, up(v, z0 + h), up(u, z0 + h)]));
            }
            total += w * d * h;
            prop_assert_eq!(last.cells.len(), 1, "box {} closes: {:?}", k, last.cells);
            match last.cells[0] {
                CellOutcome::Created(id) => {
                    prop_assert_eq!(k, 0);
                    ids.push(id);
                }
                CellOutcome::Grew { from, to } => {
                    prop_assert!(k > 0);
                    prop_assert_eq!(Some(&from), ids.last());
                    ids.push(to);
                }
                ref other => prop_assert!(false, "{:?}", other),
            }
            z0 += h;
        }
        let id = *ids.last().unwrap();
        prop_assert_eq!(doc.visible_object_ids(), vec![id]);
        let obj = doc.object(id).unwrap();
        prop_assert_eq!(obj.watertight(), WatertightState::Watertight);
        prop_assert!(obj.validate().is_ok());
        prop_assert!((volume(obj) - total).abs() <= 1e-9 * total.max(1.0),
            "volume {} expected {}", volume(obj), total);
        prop_assert!(faces(&doc).is_empty());
        let back = Document::load(&doc.save()).unwrap();
        prop_assert_eq!(back.state_hash(), doc.state_hash());
    }
}

// ------------------------------------------------------------ detached (§16 R6)

/// §16 R6: a detached gable base drawn before the rafters closes the same
/// face as one drawn after them, whichever plane each line was homed on.
#[test]
fn a_detached_gable_base_closes_before_or_after_the_rafters() {
    let (a, b, top) = (p(0., 0., 0.), p(0., 1., 0.), p(0., 0.5, 0.5));
    let gable = plane(a, Vec3::new(1., 0., 0.));
    // Rafters homed on planes other than the gable's.
    let r1 = plane(a, Vec3::new(1., 1., -1.));
    let r2 = plane(b, Vec3::new(1., 1., 1.));
    let run = |base_first: bool| {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        if base_first {
            draw_detached(&mut doc, gable, &[(a, b)]);
        }
        draw_detached(&mut doc, r1, &[(a, top)]);
        let mut last = draw_detached(&mut doc, r2, &[(b, top)]);
        if !base_first {
            last = draw_detached(&mut doc, gable, &[(a, b)]);
        }
        reopens(&doc);
        let fs = faces(&doc);
        assert_eq!(fs.len(), 1, "one face (base first: {base_first})");
        assert!(same_corners(&fs[0].1, &[a, b, top]));
        assert!(doc.sketch(fs[0].0).unwrap().detached());
        let _ = last;
        doc.sketch_ids().len()
    };
    assert_eq!(run(true), run(false), "the same drawing either way");
}

/// §16 R6 as R24 amends it: four separate detached Line gestures on one
/// plane that never share an endpoint make no face there, even where they
/// cross around a square (a `#`): detached shapes on one plane are never
/// arranged together, so a crossing is no vertex.
#[test]
fn four_detached_lines_on_one_plane_make_no_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    for (u, v) in [
        (p(-1., 0., 0.), p(2., 0., 0.)),
        (p(-1., 1., 0.), p(2., 1., 0.)),
        (p(0., -1., 0.), p(0., 2., 0.)),
        (p(1., -1., 0.), p(1., 2., 0.)),
    ] {
        let r = draw_detached(&mut doc, ground(), &[(u, v)]);
        assert!(r.faces_closed.is_empty());
    }
    assert!(faces(&doc).is_empty());
    assert_eq!(doc.sketch_ids().len(), 4);
    for s in doc.sketch_ids() {
        assert_eq!(doc.sketch(s).unwrap().edges().len(), 1, "each line whole");
    }
}

/// §16 R24: four detached Line gestures on one plane that share corners are
/// one connected set and make a face: the fourth closes the square, a new
/// detached face that takes the four loose lines (their emptied sketches
/// go), in the fourth line's undo step.
#[test]
fn four_detached_lines_sharing_corners_make_a_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let sides = ring(&[A, B, C, D]);
    for &(u, v) in &sides[..3] {
        let r = draw_detached(&mut doc, ground(), &[(u, v)]);
        assert!(r.faces_closed.is_empty());
    }
    assert!(faces(&doc).is_empty());
    let before = doc.state_hash();
    let (u, v) = sides[3];
    let r = draw_detached(&mut doc, ground(), &[(u, v)]);
    let square = face_with(&doc, &[A, B, C, D]).expect("the square is a face");
    assert_eq!(r.faces_closed, vec![square]);
    assert!(doc.sketch(square).unwrap().detached());
    assert_eq!(doc.sketch_ids(), vec![square], "the loose lines were taken");
    assert_eq!(faces(&doc).len(), 1);
    assert!(r.cells.is_empty(), "one flat face makes no solid");
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R24 keeps separate shapes separate: a region of the connected set
/// that a detached loop on the plane already holds is not made again. Two
/// detached rectangles that share a corner and overlap stay as drawn (no
/// piece of either becomes a face of the other), as do two that share a
/// side, and a rectangle whose face was erased is not given one back by a
/// line drawn from its corner.
#[test]
fn detached_shapes_sharing_corners_stay_as_drawn() {
    let wide = [A, p(2., 0., 0.), p(2., 1., 0.), D];
    let tall = [A, B, p(1., 2., 0.), p(0., 2., 0.)];
    let beside = [B, p(2., 0., 0.), p(2., 1., 0.), C];
    for second in [tall, beside] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let s1 = draw_detached(&mut doc, ground(), &ring(&wide)).sketches[0];
        let first = doc.sketch(s1).unwrap().clone();
        let r = draw_detached(&mut doc, ground(), &ring(&second));
        assert!(r.faces_closed.is_empty() && r.cells.is_empty());
        let s2 = r.sketches[0];
        assert_eq!(doc.sketch(s1), Some(&first), "the first shape is untouched");
        assert_eq!(doc.sketch_ids().len(), 2);
        let sk = doc.sketch(s2).unwrap();
        assert_eq!(sk.edges().len(), 4, "the second shape is as drawn");
        assert_eq!(sk.regions().len(), 1);
        reopens(&doc);
    }
    // A face erased stays erased.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let s1 = draw_detached(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    doc.begin_sketch_gesture(s1).unwrap();
    let sk = doc.sketch_mut(s1).unwrap();
    let rid = sk.regions().keys().next().unwrap();
    sk.set_region_faceless(rid, true).unwrap();
    doc.end_sketch_gesture(s1).unwrap();
    assert!(faces(&doc).is_empty());
    let r = draw_detached(&mut doc, ground(), &[(C, p(2., 2., 0.))]);
    assert!(r.faces_closed.is_empty());
    assert!(faces(&doc).is_empty(), "no face comes back");
}

// ------------------------------------------------------------ detached on a solid (§16 R8)

/// The plane Line homes a lone segment `a–b` on: the one holding it that
/// faces a three-quarter view most squarely (`rehomePlaneNormal`'s view
/// fallback), so it is neither a gable's nor a slope's plane.
fn view_home(a: Point3, b: Point3) -> Plane {
    let d = (b - a).normalized().unwrap();
    let v = Vec3::new(-0.6, 0.8, -0.5);
    plane(a, v - d * v.dot(d))
}

/// A gable roof's points on the unit box: the ridge 0.5 m above the top,
/// over its middle, from the front (y = 0) to the back (y = 1).
const R0: Point3 = Point3::new(0.5, 0., 1.5);
const R1: Point3 = Point3::new(0.5, 1., 1.5);

/// The roof the gable spec expects: one new Object of five faces (two
/// gables, two slopes, the cap on the box's top), 0.25 m³, the box as it
/// was, and no sketch face left.
fn gable_roof_made(doc: &Document, bx: ObjectId, box_before: &Object, last: &DraftReport) {
    let roof = created(&last.cells);
    solid(doc, roof, 0.25);
    let obj = doc.object(roof).unwrap();
    assert_eq!(obj.faces().len(), 5, "two gables, two slopes, the cap");
    let top = |q: Point3| up(q, 1.);
    assert!(object_face_with(obj, &[top(A), top(B), R0]), "front gable");
    assert!(object_face_with(obj, &[top(D), top(C), R1]), "back gable");
    let mut vis = doc.visible_object_ids();
    vis.sort();
    let mut want = vec![bx, roof];
    want.sort();
    assert_eq!(vis, want, "the box is not grown");
    let now = doc.object(bx).unwrap();
    assert_eq!(now.faces().len(), box_before.faces().len());
    assert_eq!(now.edges().len(), box_before.edges().len());
    assert!((volume(now) - 1.0).abs() < 1e-12, "the box is untouched");
    assert!(faces(doc).is_empty(), "every roof face went into the solid");
}

/// §16 R8 (Option Z, narrowed): a Detached gable roof on a box — a ridge
/// and four rafters, each its own Line gesture homed as Line homes it, and
/// no Line along the box's top — closes both gables against the box's top
/// edges (each edge's two ends are vertices of the detached shape) and the
/// slopes against its side top edges, and the faces become one new roof
/// Object with its cap; the box is untouched. One undo takes back the last
/// rafter and everything it caused.
#[test]
fn a_detached_gable_roof_closes_its_gables_against_the_box() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let top = |q: Point3| up(q, 1.);
    let r = draw_detached(&mut doc, view_home(R0, R1), &[(R0, R1)]);
    assert!(r.faces_closed.is_empty() && r.cells.is_empty());
    let r = draw_detached(&mut doc, view_home(R0, top(A)), &[(R0, top(A))]);
    assert!(r.faces_closed.is_empty(), "one rafter closes nothing");
    // The second rafter at the front closes the front gable on the box's
    // top edge: a detached face, the box's edge copied into it.
    let r = draw_detached(&mut doc, view_home(R0, top(B)), &[(R0, top(B))]);
    let gable = face_with(&doc, &[top(A), top(B), R0]).expect("the front gable is a face");
    assert!(r.faces_closed.contains(&gable));
    assert!(doc.sketch(gable).unwrap().detached(), "a detached face");
    assert!(r.cells.is_empty(), "no solid yet");
    reopens(&doc);
    let r = draw_detached(&mut doc, view_home(R1, top(D)), &[(R1, top(D))]);
    assert!(
        face_with(&doc, &[top(A), top(D), R1, R0]).is_some(),
        "the left slope closes against the box's left top edge"
    );
    assert!(r.cells.is_empty(), "no solid yet");
    reopens(&doc);
    let before = doc.state_hash();
    let last = draw_detached(&mut doc, view_home(R1, top(C)), &[(R1, top(C))]);
    gable_roof_made(&doc, bx, &box_before, &last);
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R8: the same roof with each gable drawn as ONE detached Line chain
/// on the gable's own plane (top corner, ridge end, top corner). The box's
/// top edge joins two vertices of the chain only once the chain is drawn,
/// so the draft closes the gable in its own gesture; the ridge then closes
/// both slopes and the roof is one new Object.
#[test]
fn a_detached_gable_drawn_as_one_chain_closes_in_its_own_gesture() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let top = |q: Point3| up(q, 1.);
    for (u, apex, v) in [(top(A), R0, top(B)), (top(D), R1, top(C))] {
        let on = plane(u, Vec3::new(0., 1., 0.));
        let r = draw_detached(&mut doc, on, &[(u, apex), (apex, v)]);
        let gable = face_with(&doc, &[u, v, apex]).expect("the gable closes in its own gesture");
        assert_eq!(r.sketches, vec![gable], "one new detached sketch, faced");
        assert!(doc.sketch(gable).unwrap().detached());
        assert_eq!(held(&doc, u, v), 1, "the box's top edge copied into it");
        assert!(r.cells.is_empty());
        reopens(&doc);
    }
    let before = doc.state_hash();
    let last = draw_detached(&mut doc, view_home(R0, R1), &[(R0, R1)]);
    assert_eq!(last.faces_closed.len(), 2, "the ridge closes both slopes");
    gable_roof_made(&doc, bx, &box_before, &last);
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R8 with §13's drag limit: a vertex drag never closes a face on its
/// own plane against a line it does not move, so dragging a detached
/// rafter's foot onto the box's top corner does not close the gable on the
/// gable plane, where a drawn rafter would; the drag lowers as a drag.
#[test]
fn a_drag_onto_a_box_corner_does_not_close_a_detached_gable() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    unit_box(&mut doc);
    let top = |q: Point3| up(q, 1.);
    let gable = plane(top(A), Vec3::new(0., 1., 0.));
    let off = p(1.3, 0., 1.);
    let s = draw_detached(&mut doc, gable, &[(top(A), R0), (R0, off)]).sketches[0];
    assert!(faces(&doc).is_empty());
    doc.begin_draft_detached(s, "Move vertex").unwrap();
    assert!(matches!(doc.draft_drag_vertex(off, top(B)), Some(Ok(_))));
    let r = doc.end_draft().expect("the drag lowers").0;
    assert!(r.faces_closed.is_empty() && r.cells.is_empty());
    assert!(faces(&doc).is_empty(), "no gable on the drag's own plane");
    let sk = doc.sketch(s).unwrap();
    assert_eq!(sk.edges().len(), 2, "the two rafters, nothing of the box");
    reopens(&doc);
}

/// §16 R23: the detached shape is the connected set, not every detached
/// line on the plane. A detached line Y (drawn on another plane) from the
/// box's corner P straight down, and a detached triangle X whose base
/// passes through P (P is no corner of X) and whose apex is the box's top
/// corner Q: Y shares no end with X, so the box's edge PQ is not foreign to
/// X and X stays one region, as with no Y at all. (Y, lying in X's plane,
/// still splits X's base at P, as any other plane's line lying there does.)
#[test]
fn a_detached_line_ending_on_another_shape_does_not_link_it_to_the_box() {
    let (pp, q) = (p(0., 0., 0.), p(0., 0., 1.));
    let shape_of = |with_y: bool| {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        unit_box(&mut doc);
        if with_y {
            draw_detached(
                &mut doc,
                plane(pp, Vec3::new(1., 0., 0.)),
                &[(pp, p(0., 0., -0.5))],
            );
        }
        let front = plane(pp, Vec3::new(0., -1., 0.));
        let x =
            draw_detached(&mut doc, front, &ring(&[p(-1., 0., -1.), p(1., 0., 1.), q])).sketches[0];
        reopens(&doc);
        let sk = doc.sketch(x).unwrap().clone();
        let holds_pq = sk.edges().values().any(|e| {
            let (c, d) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
            (c.approx_eq(pp, 1e-9) && d.approx_eq(q, 1e-9))
                || (c.approx_eq(q, 1e-9) && d.approx_eq(pp, 1e-9))
        });
        (sk.regions().len(), holds_pq)
    };
    assert_eq!(shape_of(false), (1, false));
    assert_eq!(shape_of(true), (1, false), "Y does not link X to the box");
}

/// §16 R8: a detached rectangle drawn around a box's footprint on the
/// ground shares no vertex with it, so it sees none of the box's edges:
/// one region, no hole, as in 1.4, and nothing closes.
#[test]
fn a_detached_rectangle_around_a_footprint_stays_one_region() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let r = draw_detached(
        &mut doc,
        ground(),
        &ring(&[
            p(-1., -1., 0.),
            p(2., -1., 0.),
            p(2., 2., 0.),
            p(-1., 2., 0.),
        ]),
    );
    assert!(r.faces_closed.is_empty() && r.cells.is_empty());
    let g = r.sketches[0];
    let sk = doc.sketch(g).unwrap();
    assert!(sk.detached());
    assert_eq!(sk.edges().len(), 4);
    assert_eq!(sk.regions().len(), 1);
    assert!(
        sk.regions()
            .values()
            .all(|r| r.holes.is_empty() && !r.faceless)
    );
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    assert_eq!(
        doc.object(bx).unwrap().edges().len(),
        box_before.edges().len()
    );
    reopens(&doc);
}

/// §16 R8: a detached rectangle drawn exactly on a box's footprint. Its
/// corners ARE the footprint's, so the box's bottom edges are foreign to it
/// — and the rectangle draws every one of them itself, so they add nothing:
/// the shape is one faced region, as drawn. Nothing closes on the box's
/// side planes (each holds one detached edge of it, no loop), and the cell
/// search makes nothing of one flat face (faces on one plane bound no
/// volume, so its rim is no cap) and, being automatic, reports nothing: no
/// Object is made, the box is untouched, and the shape stays a detached
/// face that extrudes to a new Object, as in 1.4.
#[test]
fn a_detached_rectangle_on_a_footprint_stays_a_shape() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let r = draw_detached(&mut doc, ground(), &ring(&[A, B, C, D]));
    assert!(r.faces_closed.is_empty());
    assert!(r.cells.is_empty(), "no solid, and no decline reported");
    let g = r.sketches[0];
    let sk = doc.sketch(g).unwrap();
    assert!(sk.detached());
    assert_eq!(sk.edges().len(), 4);
    assert_eq!(sk.regions().len(), 1);
    assert!(
        sk.regions()
            .values()
            .all(|r| r.holes.is_empty() && !r.faceless)
    );
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    assert_eq!(
        doc.object(bx).unwrap().faces().len(),
        box_before.faces().len()
    );
    reopens(&doc);
    // It still extrudes to a new Object of its own.
    let region = doc.extrudable_regions(g).unwrap()[0];
    let (other, _) = doc.extrude_region(g, region, 0.5).unwrap();
    assert_ne!(other, bx);
    solid(&doc, other, 0.5);
}

// ------------------------------------------------------------ every face of the set (§16 R24)

/// The feet of the ridge posts: the middles of the box's top front and
/// back edges.
const M0: Point3 = Point3::new(0.5, 0., 1.);
const M1: Point3 = Point3::new(0.5, 1., 1.);

/// The plane a roof Line is homed on: as Line homes a lone segment in a
/// three-quarter view (`view_home`); the gable's own plane, as when the
/// view faces the gable; or the gable's plane for an end's second rafter
/// only, so that the other lines at that end are other planes' lines lying
/// in the draft's plane.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RoofHome {
    View,
    Gable,
    SecondOnGable,
}

fn roof_line(
    doc: &mut Document,
    home: RoofHome,
    second: bool,
    a: Point3,
    b: Point3,
) -> DraftReport {
    let on = match home {
        RoofHome::Gable => plane(a, Vec3::new(0., 1., 0.)),
        RoofHome::SecondOnGable if second => plane(a, Vec3::new(0., 1., 0.)),
        _ => view_home(a, b),
    };
    draw_detached(doc, on, &[(a, b)])
}

/// The two half-gables at the end whose post stands on `m`: whether each is
/// a detached face.
fn half_gables(doc: &Document, m: Point3) -> [bool; 2] {
    let top = m + Vec3::new(0., 0., 0.5);
    let (near, far) = (p(0., m.y, 1.), p(1., m.y, 1.));
    [near, far].map(|corner| {
        face_with(doc, &[top, m, corner]).is_some_and(|s| doc.sketch(s).unwrap().detached())
    })
}

/// §16 R24, Kurt's `open-edges.hew` drawn fresh: a Detached post from the
/// middle of each top end edge straight up 0.5 m, each its own Line
/// gesture, then four Detached rafters, each its own gesture, from a post's
/// top to a top corner, in every order (near corner or far corner first at
/// each end, and the ends interleaved), each homed as Line homes it in a
/// three-quarter view or on the gable's plane (`RoofHome`). A first rafter
/// closes nothing; the second at an end fills BOTH half-gables there, the
/// one its own line bounds and the one the first rafter's side now closes
/// against the box's top edge — drawn on the gable's plane over the other
/// planes' lines, both in the rafter's own sketch. The ridge then closes both slopes and the roof is
/// its own solid Object (seven faces: four half-gables, two slopes, the
/// cap), the box untouched; one undo returns to the gables.
#[test]
fn every_half_gable_fills_when_its_ends_second_rafter_lands() {
    let top = |q: Point3| up(q, 1.);
    let (a, b, c, d) = (top(A), top(B), top(C), top(D));
    let orders: [[(Point3, Point3); 4]; 6] = [
        [(R0, a), (R0, b), (R1, d), (R1, c)],
        [(R0, a), (R0, b), (R1, c), (R1, d)],
        [(R0, b), (R0, a), (R1, d), (R1, c)],
        [(R0, b), (R0, a), (R1, c), (R1, d)],
        [(R0, b), (R1, d), (R0, a), (R1, c)],
        [(R1, c), (R0, a), (R1, d), (R0, b)],
    ];
    for home in [RoofHome::View, RoofHome::Gable, RoofHome::SecondOnGable] {
        for order in orders {
            let ctx = format!("{home:?} {order:?}");
            let mut doc = Document::new();
            doc.set_torture_mode(true);
            let bx = unit_box(&mut doc);
            let box_before = doc.object(bx).unwrap().clone();
            for (m, r) in [(M0, R0), (M1, R1)] {
                let rep = roof_line(&mut doc, home, false, m, r);
                assert!(rep.faces_closed.is_empty() && rep.cells.is_empty(), "{ctx}");
            }
            let mut drawn = [0usize; 2];
            for (apex, corner) in order {
                let end = usize::from(apex == R1);
                let rep = roof_line(&mut doc, home, drawn[end] == 1, apex, corner);
                drawn[end] += 1;
                let m = [M0, M1][end];
                let want = drawn[end] == 2;
                assert_eq!(
                    half_gables(&doc, m),
                    [want, want],
                    "{ctx}: end {end} after {} rafter(s)",
                    drawn[end]
                );
                if want && home == RoofHome::SecondOnGable {
                    let apex_top = m + Vec3::new(0., 0., 0.5);
                    let held_by = |corner: Point3| face_with(&doc, &[apex_top, m, corner]);
                    let own = rep.sketches[0];
                    assert_eq!(
                        held_by(p(0., m.y, 1.)),
                        Some(own),
                        "{ctx}: in the rafter's sketch"
                    );
                    assert_eq!(
                        held_by(p(1., m.y, 1.)),
                        Some(own),
                        "{ctx}: in the rafter's sketch"
                    );
                }
                assert!(doc.visible_object_ids() == vec![bx], "{ctx}: no solid yet");
                reopens(&doc);
            }
            assert_eq!(faces(&doc).len(), 4, "{ctx}: the four half-gables");
            let before = doc.state_hash();
            let last = draw_detached(&mut doc, view_home(R0, R1), &[(R0, R1)]);
            let roof = created(&last.cells);
            solid(&doc, roof, 0.25);
            let obj = doc.object(roof).unwrap();
            assert_eq!(
                obj.faces().len(),
                7,
                "{ctx}: four half-gables, two slopes, the cap"
            );
            for (m, r, near, far) in [(M0, R0, a, b), (M1, R1, d, c)] {
                assert!(object_face_with(obj, &[r, m, near]), "{ctx}");
                assert!(object_face_with(obj, &[r, m, far]), "{ctx}");
            }
            let mut vis = doc.visible_object_ids();
            vis.sort();
            let mut want = vec![bx, roof];
            want.sort();
            assert_eq!(vis, want, "{ctx}: the box is not grown");
            let now = doc.object(bx).unwrap();
            assert_eq!(now.faces().len(), box_before.faces().len(), "{ctx}");
            assert_eq!(now.edges().len(), box_before.edges().len(), "{ctx}");
            assert!(
                (volume(now) - 1.0).abs() < 1e-12,
                "{ctx}: the box is untouched"
            );
            assert!(
                faces(&doc).is_empty(),
                "{ctx}: every face went into the roof"
            );
            reopens(&doc);
            one_step(&mut doc, before);
            doc.undo().unwrap();
            assert_eq!(doc.state_hash(), before, "{ctx}");
            assert_eq!(
                doc.visible_object_ids(),
                vec![bx],
                "{ctx}: the box is back alone"
            );
            assert_eq!(faces(&doc).len(), 4, "{ctx}: the gables are back");
        }
    }
}

/// §16 R24 and R26, Kurt's `open-edges.hew` as saved by the build before
/// R24: the box, one half-gable at each end (post, half top edge, rafter to
/// the near corner) and both slopes, each slope holding the rafter to its
/// far corner. The other two half-gables were never formed, though every
/// line of them is there. The file reopens as saved (load runs no close
/// pass). Make Solid with its four sketches runs the close pass over their
/// lines first, which fills exactly those two half-gables, and makes the
/// roof its own solid: seven faces, 0.25 m³, the box untouched. It reopens
/// equal, and one undo returns to the file as opened: four faces, not six.
#[test]
fn the_open_edges_file_fills_its_half_gables_and_makes_the_roof() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/open-edges.hew"
    ))
    .expect("read fixture");
    let mut doc = Document::load(&bytes).expect("load fixture");
    doc.set_torture_mode(true);
    let [bx] = doc.visible_object_ids()[..] else {
        panic!("one box")
    };
    let box_before = doc.object(bx).unwrap().clone();
    let top = |q: Point3| up(q, 1.);
    let (a, b, c, d) = (top(A), top(B), top(C), top(D));
    let sketches = doc.sketch_ids();
    assert_eq!(sketches.len(), 4);
    assert!(sketches.iter().all(|&s| doc.sketch(s).unwrap().detached()));
    assert_eq!(half_gables(&doc, M0), [true, false], "as saved");
    assert_eq!(half_gables(&doc, M1), [true, false], "as saved");
    assert_eq!(faces(&doc).len(), 4);

    let before = doc.state_hash();
    let depth = doc.undo_depth();
    let (out, change) = doc.make_solid_from_sketches(&sketches).unwrap();
    let roof = created(&out);
    assert_eq!(doc.undo_depth(), depth + 1, "one undo step");
    assert!(change.objects_touched.contains(&roof));
    solid(&doc, roof, 0.25);
    let obj = doc.object(roof).unwrap();
    assert_eq!(
        obj.faces().len(),
        7,
        "four half-gables, two slopes, the cap"
    );
    for (m, r, near, far) in [(M0, R0, a, b), (M1, R1, d, c)] {
        assert!(object_face_with(obj, &[r, m, near]));
        assert!(object_face_with(obj, &[r, m, far]));
    }
    let mut vis = doc.visible_object_ids();
    vis.sort();
    let mut want = vec![bx, roof];
    want.sort();
    assert_eq!(vis, want, "the box is not grown");
    let now = doc.object(bx).unwrap();
    assert_eq!(now.faces().len(), box_before.faces().len());
    assert_eq!(now.edges().len(), box_before.edges().len());
    assert!((volume(now) - 1.0).abs() < 1e-12, "the box is untouched");
    assert!(faces(&doc).is_empty(), "every face went into the roof");
    reopens(&doc);
    one_step(&mut doc, before);
    doc.undo().unwrap();
    assert_eq!(faces(&doc).len(), 4, "the file as opened");
    assert_eq!(half_gables(&doc, M0), [true, false]);
    assert_eq!(doc.visible_object_ids(), vec![bx]);
}

/// §16 R26: Make Solid makes a solid or nothing. A ground Rectangle and two
/// Lines up to an apex over its front edge, drawn as raw gestures (as a
/// file reopens: no close pass ran): the pass would fill the front
/// triangle, but the shape stays open, so nothing is kept — the document
/// is as it was, no undo step is recorded — and the declines are those of
/// the document as it stands (the Rectangle's open rim).
#[test]
fn make_solid_keeps_no_face_when_no_solid_forms() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let floor = raw(&mut doc, ground(), &[A, B, C, D], true);
    let l1 = raw(&mut doc, home_of(A, APEX), &[A, APEX], false);
    let l2 = raw(&mut doc, home_of(B, APEX), &[B, APEX], false);
    assert_eq!(faces(&doc).len(), 1);
    // The pass alone would fill the front triangle.
    let mut probe = doc.clone();
    let mut lines = Vec::new();
    for s in [floor, l1, l2] {
        let sk = probe.sketch(s).unwrap();
        for e in sk.edges().values() {
            lines.push((
                s,
                sk.vertices()[e.from].position,
                sk.vertices()[e.to].position,
            ));
        }
    }
    assert!(!probe.close_faces_through(&lines, None).is_empty());
    assert!(face_with(&probe, &[A, B, APEX]).is_some());

    let before = doc.state_hash();
    let depth = doc.undo_depth();
    let alone = doc.clone().close_cells(
        &[
            CellSeed::Sketch(floor),
            CellSeed::Sketch(l1),
            CellSeed::Sketch(l2),
        ],
        &CellScope::default(),
    );
    let (out, change) = doc.make_solid_from_sketches(&[floor, l1, l2]).unwrap();
    assert!(
        out.iter().all(|c| matches!(c, CellOutcome::Declined(_))),
        "no solid: {out:?}"
    );
    assert_eq!(out, alone, "the declines of the document as it stands");
    assert_eq!(change, kernel::DocChange::default());
    assert_eq!(doc.state_hash(), before, "nothing is kept");
    assert_eq!(doc.undo_depth(), depth, "no undo step");
    assert!(face_with(&doc, &[A, B, APEX]).is_none());
    assert_eq!(faces(&doc).len(), 1);
}

/// §16 R26's fallback: when the faces the pass fills make the search
/// decline where the search alone would make a solid, Make Solid makes the
/// solid without them. A unit cube of raw ordinary faces (no search ran)
/// whose front is two halves, and a Detached outline of the whole front
/// left unfilled: its last corner was dragged into place, and a drag closes
/// nothing on its own plane (§13). With the pass the outline fills and
/// lies over both halves without being either loop, so the search declines
/// (`OverlappingFaces`, §16 R22 b); without it the cube closes. The cube is
/// made, the outline stays unfilled lines, and one undo returns the
/// document.
#[test]
fn make_solid_falls_back_to_the_search_alone() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let top = |q: Point3| up(q, 1.);
    // The outline first: a draw runs the automatic search, and the cube's
    // faces are drawn after it as raw gestures, which run none.
    let front = plane(A, Vec3::new(0., -1., 0.));
    let off = p(-0.4, 0., 0.3);
    let base = draw_detached(&mut doc, front, &[(A, B)]).sketches[0];
    let rest = draw_detached(
        &mut doc,
        front,
        &[(B, top(B)), (top(B), top(A)), (top(A), off)],
    )
    .sketches[0];
    doc.begin_draft_detached(rest, "Move vertex").unwrap();
    assert!(matches!(doc.draft_drag_vertex(off, A), Some(Ok(_))));
    let r = doc.end_draft().expect("the drag lowers").0;
    assert!(r.faces_closed.is_empty() && r.cells.is_empty());
    assert!(faces(&doc).is_empty(), "the drag left the outline unfilled");
    let m = p(0.5, 0., 0.);
    let mut sketches = vec![base, rest];
    for (n, q) in [
        (Vec3::new(0., 0., -1.), [A, D, C, B]),
        (Vec3::new(0., -1., 0.), [A, m, top(m), top(A)]),
        (Vec3::new(0., -1., 0.), [m, B, top(B), top(m)]),
        (Vec3::new(1., 0., 0.), [B, C, top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [C, D, top(D), top(C)]),
        (Vec3::new(-1., 0., 0.), [D, A, top(A), top(D)]),
        (Vec3::new(0., 0., 1.), [top(A), top(B), top(C), top(D)]),
    ] {
        sketches.push(raw(&mut doc, plane(q[0], n), &q, true));
    }
    assert!(doc.visible_object_ids().is_empty(), "no search ran");
    assert_eq!(faces(&doc).len(), 7);
    let base_before = doc.sketch(base).unwrap().clone();
    let rest_before = doc.sketch(rest).unwrap().clone();
    let before = doc.state_hash();
    let (out, _) = doc.make_solid_from_sketches(&sketches).unwrap();
    let cube = created(&out);
    solid(&doc, cube, 1.0);
    let whole = [A, B, top(B), top(A)];
    assert!(face_with(&doc, &whole).is_none(), "no fill kept");
    assert_eq!(doc.sketch(base), Some(&base_before));
    assert_eq!(doc.sketch(rest), Some(&rest_before));
    reopens(&doc);
    one_step(&mut doc, before);
}

// ------------------------------------------------------------ batch-4 review (§16 R28-R33)

/// Every live sketch's (edges, faced regions, regions with holes) count.
fn sketch_shape(doc: &Document, s: SketchId) -> (usize, usize) {
    let sk = doc.sketch(s).unwrap();
    (
        sk.edges().len(),
        sk.regions().values().filter(|r| !r.faceless).count(),
    )
}

/// One detached shape on the ground drawn either as one gesture (a closed
/// polyline, as Rectangle draws it) or as one Line gesture per side.
fn ground_ring(doc: &mut Document, pts: &[Point3], one_gesture: bool) {
    if one_gesture {
        draw_detached(doc, ground(), &ring(pts));
    } else {
        for (u, v) in ring(pts) {
            draw_detached(doc, ground(), &[(u, v)]);
        }
    }
}

/// §16 R28: a loop the connected set does not reach never blocks a face. A
/// big square and a small one inside it, sharing no corner, each drawn as
/// one gesture or as four Lines, in either order: both are faces, the big
/// one whole (no hole), overlapping, as two Rectangles are.
#[test]
fn nested_detached_shapes_both_fill_in_either_order() {
    let big = [p(0., 0., 0.), p(4., 0., 0.), p(4., 4., 0.), p(0., 4., 0.)];
    let small = [p(1., 1., 0.), p(2., 1., 0.), p(2., 2., 0.), p(1., 2., 0.)];
    for big_first in [true, false] {
        for big_lines in [false, true] {
            for small_lines in [false, true] {
                let ctx = format!("big first {big_first}, lines {big_lines}/{small_lines}");
                let mut doc = Document::new();
                doc.set_torture_mode(true);
                if big_first {
                    ground_ring(&mut doc, &big, !big_lines);
                    ground_ring(&mut doc, &small, !small_lines);
                } else {
                    ground_ring(&mut doc, &small, !small_lines);
                    ground_ring(&mut doc, &big, !big_lines);
                }
                let b = face_with(&doc, &big).unwrap_or_else(|| panic!("{ctx}: big face"));
                face_with(&doc, &small).unwrap_or_else(|| panic!("{ctx}: small face"));
                let bs = doc.sketch(b).unwrap();
                assert!(
                    bs.regions().values().all(|r| r.holes.is_empty()),
                    "{ctx}: the big face has no hole"
                );
                assert_eq!(faces(&doc).len(), 2, "{ctx}");
                reopens(&doc);
            }
        }
    }
}

/// §16 R28 (2), the documented order dependence: a face is final to later
/// detached gestures. A diagonal drawn after a square of four detached Lines
/// stays a loose line over the square's one face; drawn first, the sides
/// close two triangles.
#[test]
fn a_detached_diagonal_splits_a_square_only_when_drawn_first() {
    for diagonal_first in [false, true] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let mut diag = None;
        if diagonal_first {
            diag = Some(draw_detached(&mut doc, ground(), &[(A, C)]).sketches[0]);
        }
        for (u, v) in ring(&[A, B, C, D]) {
            draw_detached(&mut doc, ground(), &[(u, v)]);
        }
        if !diagonal_first {
            diag = Some(draw_detached(&mut doc, ground(), &[(A, C)]).sketches[0]);
        }
        if diagonal_first {
            assert!(face_with(&doc, &[A, B, C]).is_some());
            assert!(face_with(&doc, &[A, C, D]).is_some());
            assert_eq!(faces(&doc).len(), 2);
        } else {
            assert!(face_with(&doc, &[A, B, C, D]).is_some());
            assert_eq!(faces(&doc).len(), 1);
            assert_eq!(
                sketch_shape(&doc, diag.unwrap()),
                (1, 0),
                "the diagonal stays loose"
            );
        }
        reopens(&doc);
    }
}

/// §16 R29: the close pass arranges the connected set only. A detached line
/// crossing a square's site, sharing no end with it, drawn first, between
/// any two of the square's four Line gestures, or last: the square is one
/// face of four edges, and the crossing line is whole in its own sketch.
#[test]
fn a_crossing_detached_line_neither_splits_nor_blocks_a_square() {
    let cross = (p(0.5, -1., 0.), p(0.5, 2., 0.));
    let sides = ring(&[A, B, C, D]);
    for k in 0..=4 {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let mut line = None;
        for (i, &(u, v)) in sides.iter().enumerate() {
            if i == k {
                line = Some(draw_detached(&mut doc, ground(), &[cross]).sketches[0]);
            }
            draw_detached(&mut doc, ground(), &[(u, v)]);
        }
        if k == 4 {
            line = Some(draw_detached(&mut doc, ground(), &[cross]).sketches[0]);
        }
        let sq = face_with(&doc, &[A, B, C, D]).unwrap_or_else(|| panic!("k {k}: the square"));
        assert_eq!(sketch_shape(&doc, sq), (4, 1), "k {k}: four whole edges");
        let l = line.unwrap();
        assert_eq!(
            sketch_shape(&doc, l),
            (1, 0),
            "k {k}: the crossing line whole"
        );
        assert_eq!(held(&doc, cross.0, cross.1), 1, "k {k}");
        assert_eq!(faces(&doc).len(), 1, "k {k}");
        reopens(&doc);
    }
}

/// §16 R29, the draft path: a detached Rectangle drawn on a wall's plane
/// across a detached post that lies in that plane but was drawn on another
/// one, sharing no end with it, is one face of four edges; the post is
/// whole and stays its own sketch.
#[test]
fn a_detached_rectangle_across_a_post_from_another_plane_stays_whole() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (foot, top) = (p(0.5, 0., 0.), p(0.5, 0., 1.));
    let post =
        draw_detached(&mut doc, plane(foot, Vec3::new(1., 0., 0.)), &[(foot, top)]).sketches[0];
    let corners = [
        p(0., 0., 0.2),
        p(1., 0., 0.2),
        p(1., 0., 0.8),
        p(0., 0., 0.8),
    ];
    let wall = plane(corners[0], Vec3::new(0., -1., 0.));
    let before = doc.state_hash();
    let r = draw_detached(&mut doc, wall, &ring(&corners));
    let s = r.sketches[0];
    assert_eq!(face_with(&doc, &corners), Some(s));
    assert_eq!(sketch_shape(&doc, s), (4, 1), "one face, four whole edges");
    assert_eq!(sketch_shape(&doc, post), (1, 0), "the post is whole");
    assert_eq!(held(&doc, foot, top), 1);
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R29 with R25/R32, a drawn side along a stray post: a detached post
/// drawn on another plane runs along where a Rectangle's side will lie, past
/// both its corners, sharing no end with it. The Rectangle, as one gesture
/// or as four Line gestures, is one face of four edges, its side lying along
/// the post; the post stays whole, its own sketch.
#[test]
fn a_detached_side_along_a_stray_post_is_kept() {
    let (lo, hi) = (p(1., 0., -1.), p(1., 0., 3.));
    let corners = [p(1., 0., 0.), p(2., 0., 0.), p(2., 0., 1.), p(1., 0., 1.)];
    let wall = plane(corners[0], Vec3::new(0., -1., 0.));
    for one_gesture in [true, false] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let post =
            draw_detached(&mut doc, plane(lo, Vec3::new(1., 0., 0.)), &[(lo, hi)]).sketches[0];
        let post_before = doc.sketch(post).unwrap().clone();
        if one_gesture {
            draw_detached(&mut doc, wall, &ring(&corners));
        } else {
            for (u, v) in ring(&corners) {
                let r = draw_detached(&mut doc, wall, &[(u, v)]);
                assert_eq!(r.sketches.len(), 1, "each Line writes its sketch");
            }
        }
        let f = face_with(&doc, &corners)
            .unwrap_or_else(|| panic!("one gesture {one_gesture}: the face"));
        assert_eq!(sketch_shape(&doc, f), (4, 1), "one gesture {one_gesture}");
        assert_eq!(doc.sketch(post), Some(&post_before), "the post stays whole");
        assert_eq!(faces(&doc).len(), 1);
        reopens(&doc);
    }
}

/// The 8-facet half circle of radius 1 about the origin on the ground, from
/// (1, 0) to (-1, 0), drawn as one detached curve gesture.
fn detached_arc(doc: &mut Document) -> SketchId {
    let pts: Vec<Point3> = (0..=8)
        .map(|i| {
            let t = std::f64::consts::PI * f64::from(i) / 8.;
            p(t.cos(), t.sin(), 0.)
        })
        .collect();
    doc.draft_detached_new(ground(), None, "Arc", move |w| {
        w.begin_curve_with(kernel::CurveGeom {
            center: p(0., 0., 0.),
            radius: 1.,
        })?;
        for i in 0..8 {
            w.add_segment(pts[i], pts[i + 1])?;
        }
        w.end_curve();
        Ok(())
    })
    .expect("the arc lowers")
    .0
    .sketches[0]
}

/// The edges of `s` on a curve chain, and those with an analytic circle.
fn curved(doc: &Document, s: SketchId) -> (usize, usize) {
    let sk = doc.sketch(s).unwrap();
    let on: Vec<_> = sk.edges().values().filter_map(|e| e.curve).collect();
    let geom = on.iter().filter(|&&c| sk.curve_geom(c).is_some()).count();
    (on.len(), geom)
}

/// §16 R30: a curve taken into a face keeps its curve. A detached arc and
/// its chord, each its own gesture, in either order: one face whose eight
/// arc facets are one curve chain with its circle; it extrudes to a solid
/// whose curved wall is one cylinder; one undo of the closing gesture
/// returns the drawing before it, the arc's chain intact; reopen
/// round-trips.
#[test]
fn a_detached_arc_and_its_chord_keep_the_curve_in_either_order() {
    let (e0, e1) = (p(1., 0., 0.), p(-1., 0., 0.));
    for arc_first in [true, false] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let (first, before) = if arc_first {
            let s = detached_arc(&mut doc);
            assert_eq!(curved(&doc, s), (8, 8));
            (s, doc.state_hash())
        } else {
            let s = draw_detached(&mut doc, ground(), &[(e1, e0)]).sketches[0];
            (s, doc.state_hash())
        };
        let first_before = doc.sketch(first).unwrap().clone();
        let second = if arc_first {
            draw_detached(&mut doc, ground(), &[(e1, e0)]).sketches[0]
        } else {
            detached_arc(&mut doc)
        };
        let fs = faces(&doc);
        assert_eq!(fs.len(), 1, "arc first {arc_first}: one face");
        let face = fs[0].0;
        let _ = second;
        assert_eq!(
            curved(&doc, face),
            (8, 8),
            "arc first {arc_first}: the arc kept"
        );
        assert_eq!(doc.sketch(face).unwrap().edges().len(), 9);
        reopens(&doc);
        let after = doc.state_hash();
        doc.undo().unwrap();
        assert_eq!(doc.state_hash(), before, "one undo");
        assert_eq!(
            doc.sketch(first),
            Some(&first_before),
            "the first gesture as drawn"
        );
        doc.redo().unwrap();
        assert_eq!(doc.state_hash(), after);
        let region = doc.extrudable_regions(face).unwrap()[0];
        let (obj, _) = doc.extrude_region(face, region, 0.5).unwrap();
        // Eight fan triangles of the unit circle, each sin(π/8)/2, by 0.5.
        solid(&doc, obj, 2. * (std::f64::consts::PI / 8.).sin());
        let o = doc.object(obj).unwrap();
        let walls: Vec<_> = o.faces().values().filter_map(|f| f.surface).collect();
        assert_eq!(
            walls.len(),
            8,
            "arc first {arc_first}: eight facets on the cylinder"
        );
        assert!(
            walls.iter().all(|w| w.same_surface(&walls[0])),
            "arc first {arc_first}: one cylinder"
        );
        reopens(&doc);
    }
}

/// §16 R30, a take that leaves two runs: a chord from the arc's third
/// facet corner to its seventh closes a face of the four facets between
/// them; the arc's sketch keeps the two runs left at each end, each its own
/// curve chain with the circle, none of them plain segments.
#[test]
fn a_chord_across_part_of_an_arc_leaves_two_curved_runs() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let arc = detached_arc(&mut doc);
    let at = |i: u32| {
        let t = std::f64::consts::PI * f64::from(i) / 8.;
        p(t.cos(), t.sin(), 0.)
    };
    let before = doc.state_hash();
    draw_detached(&mut doc, ground(), &[(at(2), at(6))]);
    let fs = faces(&doc);
    assert_eq!(fs.len(), 1);
    assert_eq!(curved(&doc, fs[0].0), (4, 4), "the face's four facets");
    let sk = doc.sketch(arc).expect("the arc's ends stay");
    assert_eq!(sk.edges().len(), 4);
    assert_eq!(curved(&doc, arc), (4, 4), "no plain segment");
    let chains: std::collections::BTreeSet<_> =
        sk.edges().values().filter_map(|e| e.curve).collect();
    assert_eq!(chains.len(), 2, "one chain per run");
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R31: no Object face covers a detached shape. Four separate detached
/// Lines along a box's top edges, sharing corners, give one detached face of
/// four edges lying on the box's top, as a detached Rectangle there does;
/// the box is untouched.
#[test]
fn four_detached_lines_on_a_box_top_make_a_detached_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let top = |q: Point3| up(q, 1.);
    let corners = [top(A), top(B), top(C), top(D)];
    let lid = plane(top(A), Vec3::new(0., 0., 1.));
    for (u, v) in ring(&corners) {
        draw_detached(&mut doc, lid, &[(u, v)]);
    }
    let f = face_with(&doc, &corners).expect("one detached face");
    assert!(doc.sketch(f).unwrap().detached());
    assert_eq!(sketch_shape(&doc, f), (4, 1));
    assert_eq!(faces(&doc).len(), 1);
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    let now = doc.object(bx).unwrap();
    assert_eq!(now.faces().len(), box_before.faces().len());
    assert_eq!(now.edges().len(), box_before.edges().len());
    reopens(&doc);
}

/// §16 R32: take moves lines of the connected set only. A loose detached
/// line lying along the site of a square's side, sharing no vertex with it,
/// then the square as four detached Lines: the line's sketch is unchanged,
/// and the face has four whole edges.
#[test]
fn a_loose_line_the_square_does_not_reach_is_not_taken() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let l = draw_detached(&mut doc, ground(), &[(p(0.25, 0., 0.), p(0.75, 0., 0.))]).sketches[0];
    let l_before = doc.sketch(l).unwrap().clone();
    for (u, v) in ring(&[A, B, C, D]) {
        draw_detached(&mut doc, ground(), &[(u, v)]);
    }
    let f = face_with(&doc, &[A, B, C, D]).expect("the square");
    assert_eq!(sketch_shape(&doc, f), (4, 1), "four whole edges");
    assert_eq!(
        doc.sketch(l),
        Some(&l_before),
        "the loose line stays as drawn"
    );
    reopens(&doc);
}

/// §16 R30 against an ordinary draft (document fuzz seed `0452ee2a`): a
/// curve another plane lends keeps its circle in the lift, but stays that
/// plane's line. Three detached Rectangles on the ground, the first moved
/// and extruded into a box; a 25-facet circle drawn on the ground beside
/// it; then, on the box's underside (the ground facing down), a Line from
/// the circle's top point, which lies on the exact circle inside a facet
/// and on none of its corners. The draft lowers (it was refused as
/// touching a boundary edge: the lifted facet bent to the circle point),
/// the ground circle and the box are untouched, the Line is drawn, and one
/// undo removes it.
#[test]
fn a_line_from_a_lent_circles_point_leaves_the_circle_whole() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        ring(&[p(x0, y0, 0.), p(x1, y0, 0.), p(x1, y1, 0.), p(x0, y1, 0.)])
    };
    let first = draw_detached(&mut doc, ground(), &rect(0.5, 1., 3., 3.5)).sketches[0];
    draw_detached(&mut doc, ground(), &rect(0., 0., 0.5, 0.5));
    draw_detached(&mut doc, ground(), &rect(0., 0., 0.5, 0.5));
    doc.draft_detached(first, "Move", |w| {
        let es: Vec<_> = w.edges().keys().collect();
        w.apply_transform_edges(&es, &Transform::translation(Vec3::new(2.5, -2.5, 0.)))
    })
    .expect("the move lowers");
    let region = doc.extrudable_regions(first).unwrap()[0];
    let bx = doc.extrude_region(first, region, 1.25).unwrap().0;
    solid(&doc, bx, 2.5 * 2.5 * 1.25);
    let center = p(7.5, 0., 0.);
    let facet = |i: usize| {
        let t = std::f64::consts::TAU * i as f64 / 25.;
        p(7.5 + 1.5 * t.cos(), 1.5 * t.sin(), 0.)
    };
    let objects = doc.drawable_objects_on_plane(ground(), None);
    let circle = doc
        .draft(ground(), None, &objects, "Circle", |w| {
            w.begin_curve_with(kernel::CurveGeom {
                center,
                radius: 1.5,
            })?;
            for i in 0..25 {
                w.add_segment(facet(i), facet(i + 1))?;
            }
            w.end_curve();
            Ok(())
        })
        .expect("the circle lowers")
        .0
        .sketches[0];
    assert_eq!(curved(&doc, circle), (25, 25));
    let circle_before = doc.sketch(circle).unwrap().clone();
    let (faces0, edges0) = {
        let o = doc.object(bx).unwrap();
        (o.faces().len(), o.edges().len())
    };
    let before = doc.state_hash();
    let underside = plane(p(0., 0., 0.), Vec3::new(0., 0., -1.));
    let (top, end) = (p(7.5, 1.5, 0.), p(8., 1.5, 0.));
    doc.draft(underside, None, &[bx], "Line", |w| {
        w.add_segment(top, end).map(|_| ())
    })
    .expect("a line from the lent circle's point lowers");
    assert_eq!(
        doc.sketch(circle),
        Some(&circle_before),
        "the ground circle stays as drawn"
    );
    let o = doc.object(bx).unwrap();
    assert_eq!((o.faces().len(), o.edges().len()), (faces0, edges0));
    solid(&doc, bx, 2.5 * 2.5 * 1.25);
    assert_eq!(held(&doc, top, end), 1, "the Line is drawn");
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R33: a Make Solid that makes a solid keeps the whole pass. A raw cube
/// and a raw open pyramid start (a ground Rectangle and two Lines up to an
/// apex) in one selection: the cube is made, the pyramid declines, and the
/// front triangle the pass filled stays; one undo removes both.
#[test]
fn make_solid_keeps_the_pass_when_one_shape_closes() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let sh = |q: Point3| p(q.x + 10., q.y, q.z);
    let top = |q: Point3| up(sh(q), 1.);
    let mut sel = Vec::new();
    for (n, q) in [
        (Vec3::new(0., 0., -1.), [sh(A), sh(D), sh(C), sh(B)]),
        (Vec3::new(0., -1., 0.), [sh(A), sh(B), top(B), top(A)]),
        (Vec3::new(1., 0., 0.), [sh(B), sh(C), top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [sh(C), sh(D), top(D), top(C)]),
        (Vec3::new(-1., 0., 0.), [sh(D), sh(A), top(A), top(D)]),
        (Vec3::new(0., 0., 1.), [top(A), top(B), top(C), top(D)]),
    ] {
        sel.push(raw(&mut doc, plane(q[0], n), &q, true));
    }
    sel.push(raw(&mut doc, ground(), &[A, B, C, D], true));
    sel.push(raw(&mut doc, home_of(A, APEX), &[A, APEX], false));
    sel.push(raw(&mut doc, home_of(B, APEX), &[B, APEX], false));
    assert!(doc.visible_object_ids().is_empty());
    let before = doc.state_hash();
    let (out, _) = doc.make_solid_from_sketches(&sel).unwrap();
    let made: Vec<_> = out
        .iter()
        .filter(|c| matches!(c, CellOutcome::Created(_)))
        .collect();
    let declined = out
        .iter()
        .filter(|c| matches!(c, CellOutcome::Declined(_)))
        .count();
    assert_eq!((made.len(), declined), (1, 1), "{out:?}");
    let CellOutcome::Created(cube) = made[0] else {
        unreachable!()
    };
    solid(&doc, *cube, 1.0);
    assert!(
        face_with(&doc, &[A, B, APEX]).is_some(),
        "the pass's triangle stays"
    );
    reopens(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before, "one undo removes both");
    assert!(face_with(&doc, &[A, B, APEX]).is_none());
    assert!(doc.visible_object_ids().is_empty());
}

// ------------------------------------------------------------ one wall twice (§16 R22)

/// The corners of a box face: `x0..x1 × y0..y1 × z0..z1` with one extent
/// zero, as a ring.
fn quad(a: Point3, b: Point3, c: Point3, d: Point3) -> Vec<(Point3, Point3)> {
    ring(&[a, b, c, d])
}

/// §16 R22 (c), Kurt's `overlapping-edges.hew` as he drew it: a box of
/// Rectangles and Lines, every point snapped, height 0.333…, whose left
/// wall was first drawn as a Detached Rectangle. The ordinary walls close
/// the box anyway — the back wall closes the ordinary left wall on its own
/// (five faces make a loop) and the solid forms in that step: the two left
/// walls are one face to the cell search, the ordinary one is taken, and
/// the Detached Rectangle is left over, untouched, lying on the solid's
/// face. One undo returns the drawing as it was.
#[test]
fn a_box_with_its_wall_drawn_twice_closes_on_its_last_face() {
    let h = 0.333_023_568_236_291_76;
    let top = |q: Point3| up(q, h);
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    // Ground and front: Rectangles.
    draw(&mut doc, ground(), &quad(A, B, C, D));
    draw(
        &mut doc,
        plane(A, Vec3::new(0., 1., 0.)),
        &quad(A, top(A), top(B), B),
    );
    // The left wall, Detached.
    let detached = draw_detached(
        &mut doc,
        plane(A, Vec3::new(1., 0., 0.)),
        &quad(D, top(D), top(A), A),
    )
    .sketches[0];
    let wall = doc.sketch(detached).unwrap().clone();
    // The top: a Rectangle.
    let r = draw(
        &mut doc,
        plane(top(A), Vec3::new(0., 0., 1.)),
        &quad(top(B), top(C), top(D), top(A)),
    );
    assert!(r.cells.is_empty(), "no solid yet");
    // The right wall: four Lines on its plane.
    let right = plane(B, Vec3::new(1., 0., 0.));
    for (u, v) in [(B, C), (C, top(C)), (top(B), top(C)), (top(B), B)] {
        let r = draw(&mut doc, right, &[(u, v)]);
        assert!(r.cells.is_empty(), "no solid yet");
    }
    // The back wall: Lines too. The first Line, up the back left corner,
    // already closes it (the ground's, the top's and the right wall's
    // edges lie on its plane), and the box with it.
    let back = plane(D, Vec3::new(0., 1., 0.));
    let before = doc.state_hash();
    let last = draw(&mut doc, back, &[(D, top(D))]);
    assert!(
        face_with(&doc, &[D, top(D), top(C), C]).is_none(),
        "the back wall went into the solid"
    );
    let id = created(&last.cells);
    solid(&doc, id, h);
    assert_eq!(doc.object(id).unwrap().faces().len(), 6);
    // The Detached Rectangle is left over, as drawn; no other face is.
    assert_eq!(doc.sketch(detached), Some(&wall), "the detached wall stays");
    let left: Vec<_> = faces(&doc).into_iter().map(|(s, _)| s).collect();
    assert_eq!(left, vec![detached]);
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §16 R22 (c) on the file's own state: the six ordinary faces (raw sketch
/// gestures, which run no automatic search, as a file reopens) and the
/// detached duplicate make a solid from Make Solid, seeded by every sketch
/// or by the detached one alone; the Detached Rectangle is left over.
#[test]
fn make_solid_takes_the_ordinary_wall_of_a_pair() {
    let h = 0.333_023_568_236_291_76;
    let top = |q: Point3| up(q, h);
    let walls = [
        (Vec3::new(0., 0., 1.), [A, B, C, D]),
        (Vec3::new(0., 1., 0.), [A, top(A), top(B), B]),
        (Vec3::new(0., 0., 1.), [top(B), top(C), top(D), top(A)]),
        (Vec3::new(1., 0., 0.), [B, C, top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [D, top(D), top(C), C]),
        (Vec3::new(1., 0., 0.), [A, D, top(D), top(A)]),
    ];
    for seed_detached_only in [false, true] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let mut all = Vec::new();
        let raw = |doc: &mut Document, n: Vec3, q: &[Point3; 4]| {
            let s = doc.add_sketch(plane(q[0], n));
            doc.begin_sketch_gesture(s).unwrap();
            for (u, v) in ring(q) {
                doc.sketch_mut(s).unwrap().add_segment(u, v).unwrap();
            }
            doc.end_sketch_gesture(s).unwrap();
            s
        };
        // The detached left wall comes third, as in the file.
        for (i, (n, q)) in walls.iter().enumerate() {
            if i == 2 {
                let (n, q) = walls[5];
                let r = draw_detached(&mut doc, plane(q[0], n), &ring(&q));
                assert!(r.cells.is_empty());
                all.push(r.sketches[0]);
            }
            all.push(raw(&mut doc, *n, q));
        }
        assert!(doc.visible_object_ids().is_empty(), "no search ran");
        let detached = all[2];
        assert!(doc.sketch(detached).unwrap().detached());
        let wall = doc.sketch(detached).unwrap().clone();
        let seeds: Vec<CellSeed> = if seed_detached_only {
            vec![CellSeed::Sketch(detached)]
        } else {
            all.iter().map(|&s| CellSeed::Sketch(s)).collect()
        };
        let out = doc.close_cells(&seeds, &CellScope::default());
        let id = created(&out);
        solid(&doc, id, h);
        assert_eq!(doc.sketch(detached), Some(&wall), "the detached wall stays");
        let left: Vec<_> = faces(&doc).into_iter().map(|(s, _)| s).collect();
        assert_eq!(left, vec![detached]);
        reopens(&doc);
    }
}

/// §16 R22 (a): two Detached faces with the same loop are one face; the
/// earlier drawn is taken and the later one is left over. (The fifth wall
/// closes the box: the pass fills the top.)
#[test]
fn of_two_detached_walls_the_earlier_is_taken() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let top = |q: Point3| up(q, 1.);
    let left = plane(A, Vec3::new(1., 0., 0.));
    let first = draw_detached(&mut doc, left, &quad(A, D, top(D), top(A))).sketches[0];
    let second = draw_detached(&mut doc, left, &quad(A, D, top(D), top(A))).sketches[0];
    assert_ne!(first, second);
    let kept = doc.sketch(second).unwrap().clone();
    let mut outcomes = Vec::new();
    for (n, q) in [
        (Vec3::new(0., 0., 1.), [A, B, C, D]),
        (Vec3::new(0., 1., 0.), [A, top(A), top(B), B]),
        (Vec3::new(1., 0., 0.), [B, C, top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [D, top(D), top(C), C]),
    ] {
        outcomes.extend(draw_detached(&mut doc, plane(q[0], n), &ring(&q)).cells);
    }
    let id = created(&outcomes);
    solid(&doc, id, 1.0);
    assert!(
        doc.sketch(first).is_none(),
        "the earlier wall went into the solid"
    );
    assert_eq!(
        doc.sketch(second),
        Some(&kept),
        "the later one is left over"
    );
    let rest: Vec<_> = faces(&doc).into_iter().map(|(s, _)| s).collect();
    assert_eq!(rest, vec![second]);
    reopens(&doc);
}

/// §16 R22 (a): the same loop drawn once with an extra point along a side
/// is still the same loop. Two Detached left walls, one a plain rectangle
/// and one with a corner at the middle of its bottom side, are one face;
/// the box of them and four more Detached walls closes (the pass fills the
/// top), the earlier left wall is taken and the other is left over.
#[test]
fn a_twin_with_a_point_along_a_side_is_the_same_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let top = |q: Point3| up(q, 1.);
    let left = plane(A, Vec3::new(1., 0., 0.));
    let m = p(0., 0.5, 0.);
    let first = draw_detached(&mut doc, left, &quad(A, D, top(D), top(A))).sketches[0];
    let second = draw_detached(&mut doc, left, &ring(&[A, m, D, top(D), top(A)])).sketches[0];
    let kept = doc.sketch(second).unwrap().clone();
    let mut outcomes = Vec::new();
    for (n, q) in [
        (Vec3::new(0., 0., 1.), [A, B, C, D]),
        (Vec3::new(0., 1., 0.), [A, top(A), top(B), B]),
        (Vec3::new(1., 0., 0.), [B, C, top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [D, top(D), top(C), C]),
    ] {
        outcomes.extend(draw_detached(&mut doc, plane(q[0], n), &ring(&q)).cells);
    }
    let id = created(&outcomes);
    solid(&doc, id, 1.0);
    assert!(
        doc.sketch(first).is_none(),
        "the earlier wall went into the solid"
    );
    assert_eq!(doc.sketch(second), Some(&kept), "the other is left over");
    reopens(&doc);
}

/// §16 R22 (b): faces that lie over each other without being the same
/// loop still decline, typed, and name the edges where they do: a Detached
/// front wall over a box whose ordinary front wall a Line splits in two.
/// The automatic search makes nothing; Make Solid declines with
/// overlapping faces and changes nothing, and `faces_open_edges` lists the
/// overlap's edges (no open edge).
#[test]
fn a_partial_overlap_declines_and_names_its_edges() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let top = |q: Point3| up(q, 1.);
    let mid = p(0.5, 0., 0.);
    let front = plane(A, Vec3::new(0., -1., 0.));
    let whole = draw_detached(&mut doc, front, &quad(A, B, top(B), top(A))).sketches[0];
    let mut all = vec![whole];
    let mut auto = Vec::new();
    let r = draw(&mut doc, front, &[(mid, top(mid))]);
    all.extend(r.sketches);
    for (n, q) in [
        (Vec3::new(0., 0., -1.), [A, B, C, D]),
        (Vec3::new(0., -1., 0.), [A, B, top(B), top(A)]),
        (Vec3::new(1., 0., 0.), [B, C, top(C), top(B)]),
        (Vec3::new(0., 1., 0.), [C, D, top(D), top(C)]),
        (Vec3::new(-1., 0., 0.), [D, A, top(A), top(D)]),
        (Vec3::new(0., 0., 1.), [top(A), top(B), top(C), top(D)]),
    ] {
        let r = draw(&mut doc, plane(q[0], n), &ring(&q));
        all.extend(r.sketches);
        auto.extend(r.cells);
    }
    assert!(
        auto.iter().all(|c| matches!(c, CellOutcome::Declined(_))),
        "no solid: {auto:?}"
    );
    assert!(doc.visible_object_ids().is_empty());
    all.sort();
    all.dedup();
    let seeds: Vec<CellSeed> = all.iter().map(|&s| CellSeed::Sketch(s)).collect();
    let before = doc.state_hash();
    let out = doc.close_cells(&seeds, &CellScope::default());
    assert_eq!(doc.state_hash(), before, "a decline changes nothing");
    let [CellOutcome::Declined(d)] = &out[..] else {
        panic!("expected one decline, got {out:?}")
    };
    assert_eq!(d.reason, CellDeclineReason::OverlappingFaces);
    assert!(d.open_edges.is_empty());
    assert!(!d.overlapping_edges.is_empty());
    let on_front = |&(a, b): &(Point3, Point3)| a.y.abs() < 1e-12 && b.y.abs() < 1e-12;
    assert!(
        d.overlapping_edges.iter().all(on_front),
        "{:?}",
        d.overlapping_edges
    );
    let missing = doc.faces_open_edges(&all);
    assert!(missing.open_edges.is_empty() && missing.non_planar_loops.is_empty());
    assert!(!missing.overlapping_edges.is_empty());
    assert!(missing.overlapping_edges.iter().all(on_front));
    assert!(!missing.is_empty());
}

// ------------------------------------------------------------ §5.6

/// Imprints a rectangle on the top face of `obj` (at z = 1) and returns the
/// sub-face id holding `inside`.
fn top_sub_face(
    doc: &mut Document,
    obj: ObjectId,
    rect: [Point3; 4],
    inside: Point3,
) -> kernel::FaceId {
    let on = plane(p(0., 0., 1.), Vec3::new(0., 0., 1.));
    draw(doc, on, &ring(&rect));
    let o = doc.object(obj).unwrap();
    o.faces()
        .keys()
        .find(|&f| o.faces()[f].plane.normal().z > 0.5 && o.face_contains_point(f, inside))
        .unwrap()
}

/// §5.6: pushing a hole clean through a named, tagged Object keeps its name
/// and tags; undo and redo restore exactly.
#[test]
fn push_through_keeps_name_and_tags() {
    let mut doc = Document::new();
    let bx = unit_box(&mut doc);
    doc.set_node_name(NodeId::Object(bx), Some("Block".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(bx), vec!["Parts".into(), "A".into()])
        .unwrap();
    let hole = [
        p(0.4, 0.4, 1.),
        p(0.6, 0.4, 1.),
        p(0.6, 0.6, 1.),
        p(0.4, 0.6, 1.),
    ];
    let f = top_sub_face(&mut doc, bx, hole, p(0.5, 0.5, 1.));
    let before = doc.state_hash();
    let (results, _) = doc.push_pull_through(bx, f, -1.0).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(doc.object_name(results[0]), Some("Block"));
    assert_eq!(
        doc.node_tags(NodeId::Object(results[0])),
        &[vec!["Parts".to_string(), "A".to_string()]]
    );
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §5.6: when an inward push severs the solid, every piece carries the
/// name and tags.
#[test]
fn a_severing_push_names_and_tags_every_piece() {
    let mut doc = Document::new();
    let bx = unit_box(&mut doc);
    doc.set_node_name(NodeId::Object(bx), Some("Bar".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(bx), vec!["Cut".into()])
        .unwrap();
    let strip = [
        p(0.4, 0., 1.),
        p(0.6, 0., 1.),
        p(0.6, 1., 1.),
        p(0.4, 1., 1.),
    ];
    let f = top_sub_face(&mut doc, bx, strip, p(0.5, 0.5, 1.));
    let before = doc.state_hash();
    let (results, _) = doc.push_pull_through(bx, f, -1.0).unwrap();
    assert_eq!(results.len(), 2, "the push severs the bar");
    for r in &results {
        assert_eq!(doc.object_name(*r), Some("Bar"));
        assert_eq!(
            doc.node_tags(NodeId::Object(*r)),
            &[vec!["Cut".to_string()]]
        );
    }
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §5.6: the in-component variant keeps the member's name and tags too.
#[test]
fn push_through_in_a_component_keeps_name_and_tags() {
    let mut doc = Document::new();
    let bx = unit_box(&mut doc);
    doc.set_node_name(NodeId::Object(bx), Some("Leg".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(bx), vec!["Frame".into()])
        .unwrap();
    let (cid, _inst, _) = doc.make_component(&[NodeId::Object(bx)]).unwrap();
    let member = doc.def_members(cid).unwrap()[0];
    let name = doc.object_name(member).map(str::to_string);
    let tags = doc.node_tags(NodeId::Object(member)).to_vec();
    // Imprint a hole on the member's top face, in the definition's frame.
    let on = plane(p(0., 0., 1.), Vec3::new(0., 0., 1.));
    let hole = [
        p(0.4, 0.4, 1.),
        p(0.6, 0.4, 1.),
        p(0.6, 0.6, 1.),
        p(0.4, 0.6, 1.),
    ];
    let objects = doc.drawable_objects_on_plane(on, Some(cid));
    let segs = ring(&hole);
    doc.draft(on, Some(cid), &objects, "Rectangle", move |w| {
        for &(a, b) in &segs {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .unwrap();
    let o = doc.object(member).unwrap();
    let f = o
        .faces()
        .keys()
        .find(|&f| o.faces()[f].plane.normal().z > 0.5 && o.face_contains_point(f, p(0.5, 0.5, 1.)))
        .unwrap();
    let before = doc.state_hash();
    let (results, _) = doc
        .push_pull_through_in_component(cid, member, f, -1.0)
        .unwrap();
    for r in &results {
        assert_eq!(doc.object_name(*r).map(str::to_string), name);
        assert_eq!(doc.node_tags(NodeId::Object(*r)).to_vec(), tags);
    }
    reopens(&doc);
    one_step(&mut doc, before);
}
