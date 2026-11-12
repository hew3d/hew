//! Executable specs for "a loop on one plane is a face, whichever sketches
//! hold its lines" (wireframe faces and solids, §4; rule 3).
//!
//! The contract under test:
//! - a draft lifts the **foreign edges** lying in its plane — lines of free
//!   sketches on other planes, and objects' edges that bound no face there —
//!   FIXED: drawing splits where it meets them, and nothing on the plane
//!   moves or removes them;
//! - a region a draft closes with foreign edges is written into the plane's
//!   free sketch (materialization), and a foreign line that bounds no region
//!   at home is TAKEN from its sketch in the same step;
//! - after a draft, or a move, copy, or vertex drag of free drawing, the
//!   **close pass** looks at every plane through a new edge and an edge it
//!   meets, and writes each loop the new edge closes there as a face in that
//!   plane's free sketch (`DraftReport::faces_closed`);
//! - detached drawing closes faces only with detached drawing and objects'
//!   edges whose two ends are both vertices of the detached shape (§16 R8;
//!   the detached-on-a-solid specs are in `cell_solid_specs`), and its faces
//!   are detached sketches;
//! - every edit and everything it caused is ONE undo step, undo restores the
//!   document exactly, and every state saves and reopens unchanged.

use std::f64::consts::PI;

use kernel::{Document, DraftReport, Plane, Point3, SketchId, Transform, Vec3};
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

/// A draft drawing `segs` on `on` (with the objects a draw tool would pass),
/// as one gesture.
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

/// One line `a–b` drawn on the vertical plane through it (a vertical line:
/// on the plane through it facing +x), as Line re-homes a slanted segment.
fn line(doc: &mut Document, a: Point3, b: Point3) -> DraftReport {
    draw(doc, home_of(a, b), &[(a, b)])
}

/// One line `a–b` drawn on the plane through it whose normal is `n`
/// (perpendicular to the line).
fn line_on(doc: &mut Document, a: Point3, b: Point3, n: Vec3) -> DraftReport {
    draw(doc, plane(a, n), &[(a, b)])
}

/// The vertical plane through `a–b`, or for a vertical line the plane
/// through it facing +x.
fn home_of(a: Point3, b: Point3) -> Plane {
    let d = b - a;
    let n = d.cross(Vec3::new(0., 0., 1.));
    if n.length() > 1e-6 {
        plane(a, n)
    } else {
        plane(a, Vec3::new(1., 0., 0.))
    }
}

/// A closed ring of segments through `pts`.
fn ring(pts: &[Point3]) -> Vec<(Point3, Point3)> {
    (0..pts.len())
        .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
        .collect()
}

/// Whether two closed point rings hold the same corners (as sets).
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

/// The sketch holding a face with exactly `corners`, if any.
fn face_with(doc: &Document, corners: &[Point3]) -> Option<SketchId> {
    faces(doc)
        .into_iter()
        .find(|(_, r)| same_corners(r, corners))
        .map(|(s, _)| s)
}

/// Whether any live sketch holds an edge `a–b` (either way).
fn holds_edge(doc: &Document, s: SketchId, a: Point3, b: Point3) -> bool {
    let Some(sk) = doc.sketch(s) else {
        return false;
    };
    sk.edges().values().any(|e| {
        let (c, d) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
        (c.approx_eq(a, 1e-9) && d.approx_eq(b, 1e-9))
            || (c.approx_eq(b, 1e-9) && d.approx_eq(a, 1e-9))
    })
}

/// The live sketches holding an edge `a–b`.
fn holders(doc: &Document, a: Point3, b: Point3) -> Vec<SketchId> {
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| holds_edge(doc, s, a, b))
        .collect()
}

/// The document saves, reopens, and reopens as the same document.
fn reopens(doc: &Document) {
    let bytes = doc.save();
    let back = Document::load(&bytes).expect("the saved bytes reopen");
    assert_eq!(
        back.state_hash(),
        doc.state_hash(),
        "a reopened document is the one saved"
    );
}

/// One undo returns to `before` exactly; one redo returns to `after`.
fn one_step(doc: &mut Document, before: u64) {
    let after = doc.state_hash();
    doc.undo().expect("undo");
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
    doc.redo().expect("redo");
    assert_eq!(doc.state_hash(), after, "redo returns the edit's state");
}

// The pyramid's points (Kurt's case): a 1 m rectangle on the ground and a
// point 1 m above its center.
const A: Point3 = Point3::new(0., 0., 0.);
const B: Point3 = Point3::new(1., 0., 0.);
const C: Point3 = Point3::new(1., 1., 0.);
const D: Point3 = Point3::new(0., 1., 0.);
const APEX: Point3 = Point3::new(0.5, 0.5, 1.);
const FOOT: Point3 = Point3::new(0.5, 0.5, 0.);

fn ground_rect(doc: &mut Document) -> SketchId {
    let r = draw(doc, ground(), &ring(&[A, B, C, D]));
    r.sketches[0]
}

// ------------------------------------------------------------ the triangle

/// §11: ground Rectangle, vertical line, two slanted lines. The second
/// slanted line closes a face on the inclined plane; the slanted lines'
/// home sketches no longer hold them; the ground region is intact.
#[test]
fn two_slanted_lines_to_a_rectangle_close_a_triangle() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let g = ground_rect(&mut doc);
    reopens(&doc);
    // The vertical line, on the plane y = 0.5.
    let r = line_on(&mut doc, FOOT, APEX, Vec3::new(0., 1., 0.));
    assert!(r.faces_closed.is_empty());
    reopens(&doc);
    let first = line(&mut doc, APEX, A);
    assert!(
        first.faces_closed.is_empty(),
        "one slanted line closes nothing"
    );
    let home_a = first.sketches[0];
    reopens(&doc);
    let before = doc.state_hash();
    let second = line(&mut doc, APEX, B);
    reopens(&doc);
    assert_eq!(
        second.faces_closed.len(),
        1,
        "the second line closes one face"
    );
    let tri = second.faces_closed[0];
    assert_eq!(
        face_with(&doc, &[APEX, A, B]),
        Some(tri),
        "the triangle is a face"
    );
    // On the inclined plane through the three points.
    let n = doc.sketch(tri).unwrap().plane();
    for q in [APEX, A, B] {
        assert!(n.signed_distance(q).abs() <= 1e-9);
    }
    // The slanted lines moved into the face's sketch: their own sketches
    // held nothing else and are gone.
    assert_eq!(holders(&doc, APEX, A), vec![tri]);
    assert_eq!(holders(&doc, APEX, B), vec![tri]);
    assert!(
        doc.sketch(home_a).is_none(),
        "an emptied home sketch is gone"
    );
    // The ground edge is copied, not taken: the ground region is intact.
    assert!(holds_edge(&doc, g, A, B) && holds_edge(&doc, tri, A, B));
    assert_eq!(face_with(&doc, &[A, B, C, D]), Some(g));
    // The vertical line is untouched.
    assert_eq!(holders(&doc, FOOT, APEX).len(), 1);
    one_step(&mut doc, before);
    reopens(&doc);
}

/// The whole pyramid: the four slanted lines close four triangles; the
/// fourth closes two at once.
#[test]
fn four_slanted_lines_close_the_pyramid_sides() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line_on(&mut doc, FOOT, APEX, Vec3::new(0., 1., 0.));
    let mut closed = Vec::new();
    for q in [A, B, C] {
        let before = doc.state_hash();
        let r = line(&mut doc, APEX, q);
        closed.push(r.faces_closed.len());
        one_step(&mut doc, before);
        reopens(&doc);
    }
    for (u, v) in [(A, B), (B, C)] {
        assert!(face_with(&doc, &[APEX, u, v]).is_some(), "side {u:?}-{v:?}");
    }
    // The fourth line closes the last two sides, and with them the whole
    // pyramid: those faces become a solid in the same step (rule 4,
    // stage 4), so no face is left.
    let before = doc.state_hash();
    let r = line(&mut doc, APEX, D);
    closed.push(r.faces_closed.len());
    assert_eq!(closed, vec![0, 1, 1, 2]);
    assert_eq!(r.cells.len(), 1, "the pyramid closes one cell");
    assert!(faces(&doc).is_empty(), "every face became the solid");
    assert_eq!(doc.visible_object_ids().len(), 1);
    one_step(&mut doc, before);
    reopens(&doc);
}

/// The reverse order (lines from the corners to the point) closes the same
/// face: a seed's far end matters as much as its near one.
#[test]
fn lines_drawn_toward_the_point_close_the_same_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, A, APEX);
    let r = line(&mut doc, B, APEX);
    assert_eq!(r.faces_closed.len(), 1);
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
    reopens(&doc);
}

// ------------------------------------------------------------ wire cube

/// §11: a cube from twelve lines, each on a plane that is no face of the
/// cube. Each closing edge makes its face; the last makes two.
#[test]
fn wire_cube_edge_by_edge() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let [a, b, c, d] = [A, B, C, D];
    let up = |q: Point3| p(q.x, q.y, 1.);
    let mut closed: Vec<usize> = Vec::new();
    let mut step = |doc: &mut Document, r: DraftReport| {
        closed.push(r.faces_closed.len());
        reopens(doc);
    };
    // The bottom, on the ground: its fourth line closes it by drafting.
    for (u, v) in ring(&[a, b, c, d]) {
        let r = draw(&mut doc, ground(), &[(u, v)]);
        step(&mut doc, r);
    }
    assert!(face_with(&doc, &[a, b, c, d]).is_some());
    // The verticals, each on a diagonal vertical plane.
    for q in [a, b, c, d] {
        let n = Vec3::new(1., if (q.x - q.y).abs() < 0.5 { -1. } else { 1. }, 0.);
        let r = line_on(&mut doc, q, up(q), n);
        step(&mut doc, r);
    }
    // The top edges, each on a plane tilted 45 degrees about it.
    let tops = [(a, b), (b, c), (c, d), (d, a)];
    for (i, &(u, v)) in tops.iter().enumerate() {
        let along = v - u;
        let out = along.cross(Vec3::new(0., 0., 1.));
        let n = out + Vec3::new(0., 0., out.length());
        let before = doc.state_hash();
        let r = line_on(&mut doc, up(u), up(v), n);
        step(&mut doc, r);
        if i == 3 {
            one_step(&mut doc, before);
        }
    }
    assert_eq!(closed, vec![0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 2]);
    // The last edge closes the last two faces and with them the cube: the
    // six faces become one solid in the same step (rule 4, stage 4), and
    // no line is left (each was a face's edge).
    assert!(faces(&doc).is_empty(), "every face became the solid");
    assert_eq!(doc.visible_object_ids().len(), 1);
    assert!(doc.sketch_ids().is_empty(), "no line is left");
    // One undo returns the five faces and the open top edge's neighbors.
    doc.undo().unwrap();
    assert_eq!(faces(&doc).len(), 4);
    assert!(face_with(&doc, &[a, b, c, d]).is_some());
    for (u, v) in [(a, b), (b, c), (c, d)] {
        assert!(face_with(&doc, &[u, v, up(v), up(u)]).is_some());
    }
}

/// §11: four lines that close in space but on no plane make no face.
#[test]
fn a_skew_quadrilateral_makes_no_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let lifted = p(1., 1., 0.3);
    let pts = [A, B, lifted, D];
    for (u, v) in ring(&pts) {
        let r = line(&mut doc, u, v);
        assert!(r.faces_closed.is_empty());
        reopens(&doc);
    }
    assert!(faces(&doc).is_empty(), "no face anywhere");
    for (u, v) in ring(&pts) {
        assert_eq!(
            holders(&doc, u, v).len(),
            1,
            "each line stays where it was drawn"
        );
    }
}

// ------------------------------------------------------------ foreign edges

/// §11: a foreign edge is never moved or removed by an edit on the plane it
/// is foreign to. A vertical rectangle stands on the ground; its bottom
/// edge lies in the ground. A ground drawing that crosses it splits there,
/// but erasing it from the ground refuses, and the vertical rectangle is
/// unchanged.
#[test]
fn a_foreign_edge_is_never_moved_or_removed() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    // The wall: x from 1 to 3 at y = 1, 1 m tall, on the plane y = 1.
    let wall = draw(
        &mut doc,
        plane(p(0., 1., 0.), Vec3::new(0., -1., 0.)),
        &ring(&[p(1., 1., 0.), p(3., 1., 0.), p(3., 1., 1.), p(1., 1., 1.)]),
    )
    .sketches[0];
    let wall_before = doc.sketch(wall).unwrap().clone();
    // A ground rectangle across the wall's foot: the foot splits it.
    let r = draw(
        &mut doc,
        ground(),
        &ring(&[p(0., 0., 0.), p(4., 0., 0.), p(4., 2., 0.), p(0., 2., 0.)]),
    );
    let g = r.sketches[0];
    assert!(r.faces_closed.is_empty());
    reopens(&doc);
    // The wall keeps its edges: its foot bounds its own face, so it is
    // copied, never taken.
    let wall_now = doc.sketch(wall).unwrap();
    assert_eq!(wall_now.edges().len(), wall_before.edges().len());
    assert!(holds_edge(&doc, wall, p(1., 1., 0.), p(3., 1., 0.)));
    // The ground rectangle closed nothing new with the foot alone (a loose
    // line across a region splits nothing): its one region stays one.
    let ground_faces = faces(&doc).into_iter().filter(|(s, _)| *s == g).count();
    assert_eq!(ground_faces, 1);
    // The foot cannot be named from the ground, and erasing it there by its
    // working-sketch edge is refused untouched.
    let before = doc.state_hash();
    let objects = doc.drawable_objects_on_plane(ground(), None);
    doc.begin_draft(ground(), None, &objects, "Erase").unwrap();
    let foot = doc
        .draft_sketch()
        .unwrap()
        .edges()
        .iter()
        .find_map(|(id, e)| {
            let w = doc.draft_sketch().unwrap();
            let (c, d) = (w.vertices()[e.from].position, w.vertices()[e.to].position);
            (c.y == 1.0 && d.y == 1.0 && c.z == 0.0 && d.z == 0.0).then_some(id)
        });
    let foot = foot.expect("the foot is in the ground's working sketch");
    doc.draft_sketch_mut().unwrap().remove_edge(foot).unwrap();
    assert!(matches!(
        doc.end_draft(),
        Err(kernel::DocumentError::DraftBoundaryEdgeTouched)
    ));
    doc.cancel_draft();
    assert_eq!(doc.state_hash(), before, "the refused edit changed nothing");
}

/// Review #4: a vertex drag moves its plane's drawing and never another
/// plane's line, as it did before foreign edges. A ground line's end that
/// a line drawn on a wall's plane also ends on moves alone; the wall's
/// line stays where it was, and one undo restores the document.
#[test]
fn a_vertex_drag_moves_a_corner_a_foreign_edge_ends_on() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (o, x, y) = (p(0., 0., 0.), p(1., 0., 0.), p(0., 1., 0.));
    // E on the vertical x–z plane, lying in the ground; G on the ground.
    let e = draw(&mut doc, plane(o, Vec3::new(0., 1., 0.)), &[(o, x)]).sketches[0];
    let g = draw(&mut doc, ground(), &[(o, y)]).sketches[0];
    let to = p(0., -0.5, 0.);
    let before = doc.state_hash();
    // The drag as the app's Move vertex makes it: one ground draft.
    let objects = doc.drawable_objects_on_plane(ground(), None);
    let has_e = |doc: &Document| {
        let w = doc.draft_sketch().unwrap();
        w.edges().values().any(|ed| {
            let (c, d) = (w.vertices()[ed.from].position, w.vertices()[ed.to].position);
            (c.approx_eq(o, 1e-12) && d.approx_eq(x, 1e-12))
                || (c.approx_eq(x, 1e-12) && d.approx_eq(o, 1e-12))
        })
    };
    // A refused drag (onto G's other end) and a drag of no vertex leave the
    // draft exactly as it was, E still lifted.
    doc.begin_draft(ground(), None, &objects, "Move vertex")
        .unwrap();
    assert!(has_e(&doc), "the lift holds E as foreign");
    assert!(matches!(doc.draft_drag_vertex(o, y), Some(Err(_))));
    assert!(has_e(&doc), "a refused drag keeps the lift");
    assert!(doc.draft_drag_vertex(p(5., 5., 0.), to).is_none());
    assert!(has_e(&doc), "a drag of no vertex keeps the lift");
    doc.cancel_draft();
    assert_eq!(doc.state_hash(), before);

    doc.begin_draft(ground(), None, &objects, "Move vertex")
        .unwrap();
    assert!(matches!(doc.draft_drag_vertex(o, to), Some(Ok(_))));
    assert!(!has_e(&doc), "the drag's lift dropped E");
    doc.end_draft().expect("the drag lowers");
    assert!(holds_edge(&doc, g, to, y), "G's end moved");
    assert!(!holds_edge(&doc, g, o, y));
    assert!(holds_edge(&doc, e, o, x), "E stays where it was");
    assert_eq!(doc.sketch(e).unwrap().edges().len(), 1);
    assert!(holders(&doc, to, x).is_empty(), "no line follows the drag");
    reopens(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
}

/// A wall drawn standing across a ground rectangle, its foot running from
/// edge to edge, splits the rectangle in two, as a line drawn there would:
/// its foot closes two faces on the ground, and stays the wall's edge too.
#[test]
fn a_wall_across_a_ground_rectangle_splits_it() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let corners = [p(0., 0., 0.), p(4., 0., 0.), p(4., 2., 0.), p(0., 2., 0.)];
    let g = draw(&mut doc, ground(), &ring(&corners)).sketches[0];
    let before = doc.state_hash();
    let r = draw(
        &mut doc,
        plane(p(0., 1., 0.), Vec3::new(0., -1., 0.)),
        &ring(&[p(0., 1., 0.), p(4., 1., 0.), p(4., 1., 1.), p(0., 1., 1.)]),
    );
    let wall = r.sketches[0];
    assert_eq!(r.faces_closed, vec![g]);
    let halves = [
        [p(0., 0., 0.), p(4., 0., 0.), p(4., 1., 0.), p(0., 1., 0.)],
        [p(0., 1., 0.), p(4., 1., 0.), p(4., 2., 0.), p(0., 2., 0.)],
    ];
    for h in &halves {
        assert_eq!(face_with(&doc, h), Some(g));
    }
    assert!(holds_edge(&doc, wall, p(0., 1., 0.), p(4., 1., 0.)));
    one_step(&mut doc, before);
    reopens(&doc);
}

/// A loop closed on a plane by a line from another plane is a face of that
/// plane: three ground-lying lines held by three vertical sketches, and a
/// ground draft that draws nothing near them, leaves them be; the ground
/// line that closes them makes the triangle a ground face, and takes the
/// loose lines from their sketches.
#[test]
fn a_ground_line_closing_foreign_lines_makes_a_ground_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (u, v, w) = (p(0., 0., 0.), p(2., 0., 0.), p(1., 1.5, 0.));
    // Two of the triangle's sides, each drawn on the vertical plane through
    // it (both lie in the ground).
    line(&mut doc, u, v);
    line(&mut doc, v, w);
    assert!(faces(&doc).is_empty());
    // The third, on the ground: its draft closes the triangle there.
    let before = doc.state_hash();
    let r = draw(&mut doc, ground(), &[(w, u)]);
    let g = r.sketches[0];
    assert!(
        r.faces_closed.is_empty(),
        "the face is on the draft's own plane"
    );
    assert_eq!(face_with(&doc, &[u, v, w]), Some(g));
    for (a, b) in [(u, v), (v, w), (w, u)] {
        assert_eq!(
            holders(&doc, a, b),
            vec![g],
            "the loose lines moved to the face"
        );
    }
    one_step(&mut doc, before);
    reopens(&doc);
}

/// An object's edge that bounds no face on a plane is foreign there: a
/// triangle from a box's top edge and two lines to a point above it closes
/// on the slanted plane, and the box is untouched.
#[test]
fn a_box_edge_closes_a_face_on_a_plane_it_has_no_face_on() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let s = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    let (bx, _) = doc.extrude_region(s, region, 1.0).unwrap();
    let box_hash = doc.object(bx).unwrap().clone();
    let top = |q: Point3| p(q.x, q.y, 1.);
    let ridge = p(0.5, 0.5, 2.);
    line(&mut doc, ridge, top(A));
    let before = doc.state_hash();
    let r = line(&mut doc, ridge, top(B));
    assert_eq!(r.faces_closed.len(), 1);
    assert_eq!(
        face_with(&doc, &[ridge, top(A), top(B)]),
        Some(r.faces_closed[0])
    );
    // The box keeps every edge and face it had.
    let now = doc.object(bx).unwrap();
    assert_eq!(now.faces().len(), box_hash.faces().len());
    assert_eq!(now.edges().len(), box_hash.edges().len());
    one_step(&mut doc, before);
    reopens(&doc);
}

/// A listed edge that another plane's line crosses names every piece of it
/// in a draft: the listing does not gather foreign edges (§4.4), the
/// draft's lift does and splits the edge where the line crosses, so the
/// key of the edge as listed resolves to the pieces that cover it, and an
/// edit of it (here a move) moves the whole line; the other plane's line
/// stays.
#[test]
fn a_listed_edge_crossed_by_another_planes_line_is_one_unit_in_a_draft() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (l0, l1) = (p(0., 0.5, 0.), p(2., 0.5, 0.));
    draw(&mut doc, ground(), &[(l0, l1)]);
    // A wall's foot, on the vertical plane x = 1, crossing L at (1, 0.5).
    let (f0, f1) = (p(1., 0., 0.), p(1., 1., 0.));
    let wall = draw(&mut doc, plane(f0, Vec3::new(1., 0., 0.)), &[(f0, f1)]).sketches[0];
    let listed = doc.plane_units(ground(), None, &[]).unwrap();
    let edge = listed
        .units
        .iter()
        .flat_map(|u| u.edges.iter())
        .find(|e| {
            (e.a.approx_eq(l0, 1e-12) && e.b.approx_eq(l1, 1e-12))
                || (e.a.approx_eq(l1, 1e-12) && e.b.approx_eq(l0, 1e-12))
        })
        .expect("L is listed whole")
        .clone();
    doc.begin_draft(ground(), None, &[], "Move").unwrap();
    let (ids, missing) = doc.draft_edges_for_keys(&[edge.key]);
    assert!(missing.is_empty(), "the key names L in the draft");
    assert_eq!(ids.len(), 2, "both pieces of L, split at the wall's foot");
    assert_eq!(doc.draft_edges_by_key(&[edge.key]), ids);
    let shift = Transform::translation(Vec3::new(0., 2., 0.));
    doc.draft_sketch_mut()
        .unwrap()
        .apply_transform_edges(&ids, &shift)
        .unwrap();
    doc.end_draft().expect("the move lowers");
    let (m0, m1) = (p(0., 2.5, 0.), p(2., 2.5, 0.));
    assert!(
        doc.sketch_ids()
            .into_iter()
            .any(|s| holds_edge(&doc, s, m0, m1)),
        "L moved whole"
    );
    assert!(holds_edge(&doc, wall, f0, f1), "the wall's foot stays");
    reopens(&doc);
}

/// A box standing on the ground lends the ground nothing: a ground
/// rectangle drawn around its footprint stays one region, as before.
#[test]
fn a_box_on_the_ground_lends_the_ground_nothing() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let s = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap();
    let r = draw(
        &mut doc,
        ground(),
        &ring(&[
            p(-1., -1., 0.),
            p(2., -1., 0.),
            p(2., 2., 0.),
            p(-1., 2., 0.),
        ]),
    );
    let g = r.sketches[0];
    let sk = doc.sketch(g).unwrap();
    assert_eq!(sk.edges().len(), 4);
    assert_eq!(sk.regions().len(), 1);
    assert!(sk.regions().values().all(|r| r.holes.is_empty()));
}

// ------------------------------------------------------------ detached

/// §11 / §4.3: detached lines and ordinary drawing never close a face
/// together; detached lines close one with each other, and that face is a
/// detached sketch.
#[test]
fn detached_and_ordinary_lines_do_not_close_together() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    // Two detached slanted lines to the ground rectangle: no face.
    for q in [A, B] {
        let r = doc
            .draft_detached_new(home_of(APEX, q), None, "Line", |w| {
                w.add_segment(APEX, q).map(|_| ())
            })
            .unwrap()
            .0;
        assert!(r.faces_closed.is_empty());
        reopens(&doc);
    }
    assert!(face_with(&doc, &[APEX, A, B]).is_none());

    // The other way round: a detached rectangle and ordinary lines.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    doc.draft_detached_new(ground(), None, "Rectangle", |w| {
        for (a, b) in ring(&[A, B, C, D]) {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .unwrap();
    line(&mut doc, APEX, A);
    let r = line(&mut doc, APEX, B);
    assert!(r.faces_closed.is_empty());
    assert!(face_with(&doc, &[APEX, A, B]).is_none());
    reopens(&doc);
}

#[test]
fn detached_lines_close_a_detached_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    doc.draft_detached_new(ground(), None, "Rectangle", |w| {
        for (a, b) in ring(&[A, B, C, D]) {
            w.add_segment(a, b)?;
        }
        Ok(())
    })
    .unwrap();
    let mut last = DraftReport::default();
    let mut before = 0;
    for q in [A, B] {
        before = doc.state_hash();
        last = doc
            .draft_detached_new(home_of(APEX, q), None, "Line", |w| {
                w.add_segment(APEX, q).map(|_| ())
            })
            .unwrap()
            .0;
        reopens(&doc);
    }
    assert_eq!(last.faces_closed.len(), 1);
    let tri = last.faces_closed[0];
    assert!(
        doc.sketch(tri).unwrap().detached(),
        "a face of detached lines is detached"
    );
    assert_eq!(face_with(&doc, &[APEX, A, B]), Some(tri));
    one_step(&mut doc, before);
    // An ordinary line from the apex to C closes nothing with them.
    let r = line(&mut doc, APEX, C);
    assert!(r.faces_closed.is_empty());
    reopens(&doc);
}

// ------------------------------------------------------------ sketch moves

/// The pass runs after a sketch move: a slanted line drawn elsewhere and
/// moved onto the pyramid's corner closes the triangle, in the move's own
/// undo step.
#[test]
fn moving_a_line_into_place_closes_the_face() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    let shift = Vec3::new(5., 0., 0.);
    let away = line(&mut doc, APEX + shift, B + shift).sketches[0];
    assert!(face_with(&doc, &[APEX, A, B]).is_none());
    let before = doc.state_hash();
    let change = doc
        .transform_sketch(away, &Transform::translation(-shift))
        .unwrap();
    let closed = doc.faces_closed_by_last_edit().to_vec();
    assert_eq!(closed.len(), 1);
    assert!(change.sketches_touched.contains(&closed[0]));
    assert_eq!(face_with(&doc, &[APEX, A, B]), Some(closed[0]));
    one_step(&mut doc, before);
    reopens(&doc);
}

/// The pass runs after an island move inside a sketch, a copy, and a
/// vertex drag.
#[test]
fn island_moves_copies_and_vertex_drags_close_faces() {
    // Island move: the line is one island of a sketch holding another.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    let h = home_of(APEX, B);
    let off = p(0.5, 0.5, 3.);
    let r = draw(
        &mut doc,
        h,
        &[
            (off, p(0.5, 0.5, 4.)),
            (
                APEX + Vec3::new(0.5, -0.5, 0.),
                B + Vec3::new(0.5, -0.5, 0.),
            ),
        ],
    );
    let s = r.sketches[0];
    let island = {
        let sk = doc.sketch(s).unwrap();
        sk.islands()
            .iter()
            .find(|(_, isl)| {
                isl.edges.iter().any(|&e| {
                    let ed = sk.edges()[e];
                    sk.vertices()[ed.from].position.z < 2.0
                })
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let before = doc.state_hash();
    doc.transform_sketch_island(s, island, &Transform::translation(Vec3::new(-0.5, 0.5, 0.)))
        .unwrap();
    assert_eq!(doc.faces_closed_by_last_edit().len(), 1);
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
    one_step(&mut doc, before);
    reopens(&doc);

    // Copy: a copy of the line lands on the corner.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    let shift = Vec3::new(0., 0., 5.);
    let s = line(&mut doc, APEX + shift, B + shift).sketches[0];
    let islands: Vec<_> = doc.sketch(s).unwrap().islands().keys().collect();
    let before = doc.state_hash();
    doc.copy_sketch_islands(s, &islands, &Transform::translation(-shift))
        .unwrap();
    assert_eq!(doc.faces_closed_by_last_edit().len(), 1);
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
    one_step(&mut doc, before);
    reopens(&doc);

    // Vertex drag: the line's far end is dragged onto B, on its plane.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    let short = p(0.75, 0.25, 0.5);
    let s = line(&mut doc, APEX, short).sketches[0];
    let v = {
        let sk = doc.sketch(s).unwrap();
        sk.vertices()
            .iter()
            .find(|(_, x)| x.position.approx_eq(short, 1e-12))
            .map(|(id, _)| id)
            .unwrap()
    };
    let before = doc.state_hash();
    doc.move_sketch_vertex(s, v, B).unwrap();
    assert_eq!(doc.faces_closed_by_last_edit().len(), 1);
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
    one_step(&mut doc, before);
    reopens(&doc);
}

/// The pass never runs on undo, redo or load: a document reopens as it was
/// saved, and undoing the closing line leaves the lines loose.
#[test]
fn undo_redo_and_load_do_not_close_faces() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    line(&mut doc, APEX, B);
    doc.undo().unwrap();
    assert!(face_with(&doc, &[APEX, A, B]).is_none());
    assert_eq!(holders(&doc, APEX, A).len(), 1);
    let back = Document::load(&doc.save()).unwrap();
    assert!(face_with(&back, &[APEX, A, B]).is_none());
    doc.redo().unwrap();
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
}

/// `close_faces_through` on its own: one undo step of its own, closing the
/// faces a set of edges closes (what a file drawn before this rule needs).
#[test]
fn close_faces_through_closes_what_old_lines_close() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    ground_rect(&mut doc);
    line(&mut doc, APEX, A);
    let s = line(&mut doc, APEX, B).sketches[0];
    // Undo the pass's effect by building the old state directly: undo, then
    // re-add the line as a plain sketch the way an old file holds it.
    doc.undo().unwrap();
    let old = doc.add_sketch(home_of(APEX, B));
    doc.sketch_mut(old).unwrap().add_segment(APEX, B).unwrap();
    let _ = s;
    assert!(face_with(&doc, &[APEX, A, B]).is_none());
    let before = doc.state_hash();
    let closed = doc.close_faces_through(&[(old, APEX, B)], None);
    assert_eq!(closed.len(), 1);
    assert!(face_with(&doc, &[APEX, A, B]).is_some());
    one_step(&mut doc, before);
    reopens(&doc);
}

// ------------------------------------------------------------ property

/// A random plane through the origin region, and an orthonormal basis of it.
fn random_frame(theta: f64, phi: f64, off: f64) -> (Point3, Vec3, Vec3, Vec3) {
    let n = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
    let helper = if n.z.abs() < 0.9 {
        Vec3::new(0., 0., 1.)
    } else {
        Vec3::new(1., 0., 0.)
    };
    let u = n.cross(helper).normalized().unwrap();
    let v = n.cross(u).normalized().unwrap();
    let o = Point3::ORIGIN + n * off;
    (o, n, u, v)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// §11 property: a random convex polygon on a random plane, drawn edge
    /// by edge with each edge on its own random plane through it, forms one
    /// face, on the polygon's plane, with the polygon's area.
    #[test]
    fn a_polygon_drawn_on_random_planes_closes_with_its_area(
        theta in 0.0f64..(2.0 * PI),
        phi in 0.2f64..(PI - 0.2),
        off in -2.0f64..2.0,
        n in 3usize..7,
        radii in proptest::collection::vec(0.5f64..2.0, 6),
        tilts in proptest::collection::vec(0.3f64..(PI - 0.3), 6),
    ) {
        let (o, normal, u, v) = random_frame(theta, phi, off);
        // A convex polygon: points on a circle at increasing angles.
        let pts2: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let a = 2.0 * PI * (i as f64) / (n as f64);
                (radii[0] * a.cos(), radii[0] * a.sin() * (0.5 + radii[i] / 4.0))
            })
            .collect();
        let pts: Vec<Point3> = pts2.iter().map(|&(x, y)| o + u * x + v * y).collect();
        let area = 0.5 * (0..n)
            .map(|i| {
                let (x0, y0) = pts2[i];
                let (x1, y1) = pts2[(i + 1) % n];
                x0 * y1 - x1 * y0
            })
            .sum::<f64>()
            .abs();
        let mut doc = Document::new();
        let mut closed = Vec::new();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            // The edge's own plane: the polygon's plane turned about the
            // edge by `tilts[i]` (never the polygon's plane itself).
            let d = (b - a).normalized().unwrap();
            let side = d.cross(normal);
            let hn = normal * tilts[i].cos() + side * tilts[i].sin();
            let r = draw(&mut doc, plane(a, hn), &[(a, b)]);
            closed.extend(r.faces_closed);
        }
        prop_assert_eq!(closed.len(), 1, "the last edge closes one face");
        let s = closed[0];
        let sk = doc.sketch(s).unwrap();
        prop_assert!((sk.plane().normal().cross(normal)).length() < 1e-9);
        let faced: Vec<_> = sk.regions().iter().filter(|(_, r)| !r.faceless).collect();
        prop_assert_eq!(faced.len(), 1);
        let got = sk.region_area(faced[0].0).unwrap();
        prop_assert!((got - area).abs() <= 1e-9 * area.max(1.0), "area {got} vs {area}");
        let back = Document::load(&doc.save()).unwrap();
        prop_assert_eq!(back.state_hash(), doc.state_hash());
    }
}
