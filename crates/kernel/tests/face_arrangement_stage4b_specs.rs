//! Executable specs for ground-parity drawing on a solid's face, stage 4b:
//! curve runs and open paths, through the drafting engine
//! ([`Document::draft`]).
//!
//! **Curves.** Once other shapes cut a drawn circle or arc on a face, it
//! lies on the solid as several chord runs, each between a different pair
//! of faces. [`Document::plane_units`] recovers the drawing: the facets of
//! one circle reachable through points nothing else uses are one curve run,
//! and an erase of every run of the circle (one draft, ONE undo entry)
//! gives the ground's result with the curve's edges removed — erasing a
//! circle drawn across a rectangle gives the rectangle back. An erase that
//! leaves two shapes meeting at a single point leaves the face around them
//! holding them as holes sharing that vertex, as on the ground. Erasing a
//! solid's own edge is refused `DraftBoundaryEdgeTouched`, nothing changed.
//!
//! **Open paths.** An open line chain drawn on a face is cut at every point
//! where it meets the face's coplanar group: a piece that separates two
//! regions is face topology (from the outer loop, from a hole back to the
//! same hole, or joining two boundaries), a loose piece is the object's
//! owned sketch, all as ONE undo entry. A line across a rectangle imprint
//! from one edge of the face to the other splits every face it crosses, as
//! on the ground; a dangling end is loose drawing, never a refusal.
//!
//! Every accepting spec checks the stage invariants (see
//! `face_arrangement_specs.rs`): I1 valid and watertight, I2 Euler–Poincaré
//! unchanged, I3 group area conserved, I4 loops simple, I5 exact undo/redo with
//! byte-identical saves, I6 the face group equals the ground's regions for
//! the same lines, I7 the saved document reopens.

use std::f64::consts::PI;

use kernel::{
    CurveGeom, DocChange, Document, DocumentError, DraftReport, EdgeId, FaceId, Object, PlaneUnits,
    Point3,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::*;

// --------------------------------------------------------------- helpers

/// The front wall with the maintainer's rectangle: `(wall, rect)`.
fn wall_with_rect() -> (Wall, FaceId) {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    (w, s)
}

fn group_rings(obj: &Object, w: &Wall) -> Vec<Rings> {
    group_faces(obj, &w.plane)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect()
}

fn same_circle(a: CurveGeom, b: CurveGeom) -> bool {
    a.center.approx_eq(b.center, 1e-12) && (a.radius - b.radius).abs() < 1e-12
}

/// The curve runs of the circle `g` among `units`, each as its edges'
/// endpoint pairs.
fn runs_in(units: &PlaneUnits, g: CurveGeom) -> Vec<Vec<(Point3, Point3)>> {
    let mut out = Vec::new();
    for u in &units.units {
        for run in u.curves.iter().filter(|r| same_circle(r.geom, g)) {
            out.push(
                run.edges
                    .iter()
                    .map(|k| {
                        let e = u.edges.iter().find(|e| e.key == *k).expect("run edge");
                        (e.a, e.b)
                    })
                    .collect(),
            );
        }
    }
    out
}

/// The curve runs of the circle `g` on the wall's plane.
fn circle_runs(w: &Wall, g: CurveGeom) -> Vec<Vec<(Point3, Point3)>> {
    runs_in(&units_on(&w.doc, None, w.id, w.plane), g)
}

/// Every drawing edge of the circle `g` (a `facets`-gon), as endpoint
/// pairs: the edges of its runs.
fn circle_segments(w: &Wall, g: CurveGeom, _facets: usize) -> Vec<(Point3, Point3)> {
    // The circle's runs list every edge of it, the pieces of facets a
    // crossing cut short included.
    circle_runs(w, g).into_iter().flatten().collect()
}

/// Whether the runs close around the circle: every endpoint is shared by
/// exactly two edges.
fn runs_close(runs: &[Vec<(Point3, Point3)>]) -> bool {
    let segs: Vec<(Point3, Point3)> = runs.iter().flatten().copied().collect();
    let ends: Vec<Point3> = segs.iter().flat_map(|&(a, b)| [a, b]).collect();
    ends.iter()
        .all(|p| ends.iter().filter(|q| q.approx_eq(*p, 1e-12)).count() == 2)
}

/// Every edge of `obj` claiming `g` lies on some run.
fn runs_hold_every_claim(obj: &Object, runs: &[Vec<(Point3, Point3)>], g: CurveGeom) {
    let segs: Vec<(Point3, Point3)> = runs.iter().flatten().copied().collect();
    for e in obj.edges().keys() {
        let Some(c) = obj.edges()[e].curve else {
            continue;
        };
        if !same_circle(c, g) {
            continue;
        }
        let (a, b) = obj.edge_endpoints(e).expect("edge");
        assert!(
            segs.iter()
                .any(|&(p, q)| (p.approx_eq(a, 1e-12) && q.approx_eq(b, 1e-12))
                    || (p.approx_eq(b, 1e-12) && q.approx_eq(a, 1e-12))),
            "claimed edge {a:?}–{b:?} is on a run of the circle"
        );
    }
}

/// Erases every edge of the circle `g` ([`circle_segments`]) in one draft.
fn delete_circle(
    w: &mut Wall,
    g: CurveGeom,
    facets: usize,
) -> Result<(DraftReport, DocChange), DocumentError> {
    let segs = circle_segments(w, g, facets);
    assert!(!segs.is_empty(), "the circle is on the plane");
    let objs = plane_objects(&w.doc, w.plane, None, w.id);
    w.doc.draft(w.plane, None, &objs, "Erase", |sk| {
        for &(a, b) in &segs {
            let e = sk
                .edge_at_positions(a, b)
                .unwrap_or_else(|| panic!("circle edge {a:?}-{b:?} in the working sketch"));
            sk.remove_edge(e)?;
        }
        Ok(())
    })
}

/// Erases the circle `g` through a draft with I1–I5 and I7, I6 against the
/// ground with every edge of the circle removed (straight-through vertices
/// aside), and checks the group then equals `expect` (the face group
/// before the circle was drawn) with no curve left.
fn delete_circle_and_check(w: &mut Wall, g: CurveGeom, facets: usize, expect: &[Rings]) {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let segs = circle_segments(w, g, facets);
    let (report, _) = delete_circle(w, g, facets).expect("the circle erases");
    assert!(
        report.kept_as_sketch.is_empty(),
        "everything stays topology"
    );
    assert_eq!(owned_edge_count(w), 0, "no loose edge is left behind");
    assert_eq!(sketch_edge_count(w), 0, "no sketch drawing is left");
    let n = w.plane.normal();
    let ground = straightened(&ground_regions(&ground_without(w, &before, &segs)), n);
    let faces = straightened(&group_rings(w.obj(), w), n);
    assert!(
        multiset_match(&faces, &ground, true),
        "the face group differs from the ground with the curve deleted"
    );
    assert!(
        multiset_match(&faces, &straightened(expect, n), true),
        "the drawing before the curve is back"
    );
    assert!(circle_runs(w, g).is_empty(), "no curve left");
    assert_invariants(w, &before, &bytes0, depth0);
}

/// The ground side of I6 for a deleted curve: a sketch on the group's
/// plane holding every group edge of `obj` except `remove`, the group's
/// outline first. (The ground adopts an island as a hole only when the
/// outline around it is already there — an island added first stays a
/// separate region with no hole cut for it — so the order is pinned here
/// rather than left to face keys.)
fn ground_without(w: &Wall, obj: &Object, remove: &[(Point3, Point3)]) -> kernel::Sketch {
    let faces = group_faces(obj, &w.plane);
    let mut edges: Vec<(bool, Point3, Point3)> = Vec::new();
    let mut seen: Vec<EdgeId> = Vec::new();
    for &f in &faces {
        let face = &obj.faces()[f];
        for l in std::iter::once(face.outer_loop).chain(face.inner_loops.iter().copied()) {
            for h in obj.loop_half_edges(l) {
                let e = obj.half_edges()[h].edge;
                if seen.contains(&e) {
                    continue;
                }
                seen.push(e);
                let (a, b) = obj.edge_endpoints(e).expect("edge");
                if remove
                    .iter()
                    .any(|&(p, q)| (p == a && q == b) || (p == b && q == a))
                {
                    continue;
                }
                let inner = obj.half_edges()[h].twin.is_some_and(|t| {
                    faces.contains(&obj.loops()[obj.half_edges()[t].loop_id].face)
                });
                edges.push((inner, a, b));
            }
        }
    }
    // Outline (false) before interior (true); stable otherwise.
    edges.sort_by_key(|e| e.0);
    let mut sk = kernel::Sketch::on_plane(w.plane);
    for (_, a, b) in edges {
        sk.add_segment(a, b).expect("group edge on the ground");
    }
    sk
}

/// Draws the open `path` on `seed` with I1–I7, I6 against the ground with
/// the path's segments added. The path lands wholly as face topology.
fn draw_path_and_check(w: &mut Wall, seed: FaceId, path: Vec<Point3>) -> DraftReport {
    draw_path_with_curves_and_check(w, seed, path, Vec::new())
}

fn draw_path_with_curves_and_check(
    w: &mut Wall,
    seed: FaceId,
    path: Vec<Point3>,
    curves: Vec<Option<CurveGeom>>,
) -> DraftReport {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) =
        draw_path(&mut w.doc, None, w.id, seed, &path, &curves).expect("the line draws");
    assert!(report.kept_as_sketch.is_empty());
    assert_group_matches_ground(w, &ground_edit(w.plane, &before, &path, &[]));
    assert_invariants(w, &before, &bytes0, depth0);
    report
}

/// Edges in the sketches the wall's object owns.
fn owned_edge_count(w: &Wall) -> usize {
    w.doc
        .object_owned_sketches(w.id)
        .iter()
        .map(|&s| w.doc.sketch(s).expect("sketch").edges().len())
        .sum()
}

/// Edges in every live sketch.
fn sketch_edge_count(w: &Wall) -> usize {
    w.doc
        .sketch_ids()
        .iter()
        .map(|&s| w.doc.sketch(s).expect("sketch").edges().len())
        .sum()
}

/// What holds after any accepted draft that is the latest undo entry:
/// valid object, one undo entry, a document that reopens, exact undo/redo.
fn assert_undoable(w: &mut Wall, before: &Object, bytes0: &[u8], depth0: usize) {
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo entry");
    w.obj().validate().expect("valid");
    let bytes1 = w.doc.save();
    assert_reopens(&bytes1);
    let after = w.obj().clone();
    for _ in 0..3 {
        w.doc.undo().expect("undo");
        assert!(objects_equivalent(w.obj(), before), "undo restores");
        assert_eq!(w.doc.save(), bytes0, "undo restores the saved bytes");
        w.doc.redo().expect("redo");
        assert!(objects_equivalent(w.obj(), &after), "redo restores");
        assert_eq!(w.doc.save(), bytes1, "redo restores the saved bytes");
    }
}

/// The open `path` is accepted as loose drawing: the solid is untouched
/// (the face group is as before), `owned` edges are the object's owned
/// sketch afterwards, nothing is free, one undo entry, exact undo/redo.
fn assert_path_owned(w: &mut Wall, seed: FaceId, path: &[Point3], owned: usize) -> DraftReport {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let owned0 = owned_edge_count(w);
    let (report, _) = draw_path(&mut w.doc, None, w.id, seed, path, &[]).expect("the line draws");
    assert!(
        objects_equivalent(w.obj(), &before),
        "the solid is untouched"
    );
    assert_eq!(owned_edge_count(w) - owned0, owned, "owned drawing");
    assert_undoable(w, &before, &bytes0, depth0);
    report
}

/// The rectangle with a circle drawn across its right side, centered
/// `dx` from it and turned by `phase` (`phase` 0 puts two vertices on the
/// side, `PI / 24` crosses two facets mid-way): `(wall, group before the
/// circle, circle)`.
fn rect_with_crossing_circle(dx: f64, phase: f64) -> (Wall, Vec<Rings>, CurveGeom) {
    let (mut w, s) = wall_with_rect();
    let rect_state = group_rings(w.obj(), &w);
    let (cx, cy, r) = (RX + RW + dx, RY + R, MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, phase);
    w.draw_loop(s, &ring, Some(g))
        .expect("the crossing circle draws");
    (w, rect_state, g)
}

/// The maintainer's inscribed circle (tangent to the rectangle's left,
/// top, and bottom sides at its own vertices): `(wall, group before, g)`.
fn rect_with_inscribed_circle() -> (Wall, Vec<Rings>, CurveGeom) {
    let (mut w, s) = wall_with_rect();
    let rect_state = group_rings(w.obj(), &w);
    let (cx, cy) = (RX + R, RY + R);
    let g = w.circle_geom(cx, cy, R);
    let ring = w.ring(cx, cy, R, 24, 0.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the inscribed circle draws");
    (w, rect_state, g)
}

/// A strip imprinted at (3, y0) mm and a circle of radius 1.5 mm centered
/// in it at (7, 3) mm, crossing its top and its bottom: four runs.
/// `aligned`: a 8 × 1.5 mm strip whose sides meet the circle's 30° vertices
/// (every facet whole); otherwise an 8 × 2 mm strip with the circle turned
/// 7.5° (the crossings cut facets mid-way).
fn strip_with_circle(aligned: bool) -> (Wall, Vec<Rings>, CurveGeom) {
    let mut w = front_wall();
    let wall = w.face;
    let s = if aligned {
        imprint_mm(&mut w, wall, 3.0, 2.25, 8.0, 1.5)
    } else {
        imprint_mm(&mut w, wall, 3.0, 2.0, 8.0, 2.0)
    };
    let rect_state = group_rings(w.obj(), &w);
    let (cx, cy, r) = (7.0 * MM, 3.0 * MM, 1.5 * MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, if aligned { 0.0 } else { PI / 24.0 });
    w.draw_loop(s, &ring, Some(g))
        .expect("the circle across the strip draws");
    (w, rect_state, g)
}

/// An arc of the circle about (10, −6) mm of radius 10 mm from the wall's
/// bottom edge at x = 2 mm up over a 4 × 3 mm rectangle at (8, 2) mm and
/// back down to the bottom edge at x = 18 mm, as `n` facets with claims.
fn arc_path(w: &Wall, n: usize) -> (Vec<Point3>, Vec<Option<CurveGeom>>, CurveGeom) {
    let (cx, cy, r) = (10.0 * MM, -6.0 * MM, 10.0 * MM);
    let g = w.circle_geom(cx, cy, r);
    let (a0, a1) = ((6.0f64).atan2(-8.0), (6.0f64).atan2(8.0));
    let mut path: Vec<Point3> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / n as f64;
            w.at(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    // The ends exactly on the bottom edge (on the circle exactly, too).
    path[0] = mm(w, 2.0, 0.0);
    path[n] = mm(w, 18.0, 0.0);
    (path, vec![Some(g); n], g)
}

fn wall_with_arc_over_rect() -> (Wall, Vec<Rings>, CurveGeom) {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 8.0, 2.0, 4.0, 3.0);
    let rect_state = group_rings(w.obj(), &w);
    let (path, curves, g) = arc_path(&w, 16);
    let seed = w.face;
    draw_path(&mut w.doc, None, w.id, seed, &path, &curves)
        .expect("the arc across the rectangle draws");
    (w, rect_state, g)
}

// --------------------------------------------------- curve runs: reading

#[test]
fn a_circle_crossing_an_imprint_side_is_one_closed_circle_of_two_runs() {
    // On its vertices (phase 0): one circle, two runs, every facet whole.
    for dx in [-0.5 * MM, 0.0, 0.5 * MM] {
        let (w, _, g) = rect_with_crossing_circle(dx, 0.0);
        let runs = circle_runs(&w, g);
        assert_eq!(runs.len(), 2, "dx {dx}");
        assert!(runs_close(&runs), "a whole circle");
        runs_hold_every_claim(w.obj(), &runs, g);
    }
}

#[test]
fn a_circle_cut_mid_facet_is_still_one_closed_circle_of_two_runs() {
    // Crossing two facets mid-way (whose fragments keep no claim, F11):
    // still one circle, two runs, closed around the circle.
    for dx in [-0.5 * MM, 0.0, 0.5 * MM] {
        let (w, _, g) = rect_with_crossing_circle(dx, PI / 24.0);
        let runs = circle_runs(&w, g);
        assert_eq!(runs.len(), 2, "dx {dx}");
        assert!(runs_close(&runs), "a whole circle, dx {dx}");
        runs_hold_every_claim(w.obj(), &runs, g);
    }
}

#[test]
fn an_inscribed_circle_is_one_closed_circle_of_three_runs() {
    let (w, _, g) = rect_with_inscribed_circle();
    let runs = circle_runs(&w, g);
    assert_eq!(runs.len(), 3);
    assert!(runs_close(&runs));
    runs_hold_every_claim(w.obj(), &runs, g);
    // The corner leftovers' straight runs along the wall belong to no run.
    let edges: usize = runs.iter().map(Vec::len).sum();
    assert_eq!(edges, 24, "exactly the circle's facets");
}

#[test]
fn a_circle_crossing_a_strip_twice_is_one_closed_circle_of_four_runs() {
    let (w, _, g) = strip_with_circle(true);
    let runs = circle_runs(&w, g);
    assert_eq!(runs.len(), 4);
    assert!(runs_close(&runs));
    runs_hold_every_claim(w.obj(), &runs, g);
}

#[test]
fn a_circle_cut_mid_facet_by_a_strip_is_still_one_closed_circle_of_four_runs() {
    let (w, _, g) = strip_with_circle(false);
    let runs = circle_runs(&w, g);
    assert_eq!(runs.len(), 4);
    assert!(runs_close(&runs));
    runs_hold_every_claim(w.obj(), &runs, g);
}

#[test]
fn an_arc_across_an_imprint_is_one_open_curve_including_a_pair_meeting_twice() {
    let (w, _, g) = wall_with_arc_over_rect();
    let runs = circle_runs(&w, g);
    assert!(!runs_close(&runs), "an arc");
    // Below the arc outside the rectangle, and above it, meet along both
    // of the arc's runs outside the rectangle; the rectangle's two halves
    // along the third.
    assert_eq!(runs.len(), 3);
    runs_hold_every_claim(w.obj(), &runs, g);
}

#[test]
fn a_whole_circle_imprint_and_a_single_arc_run_are_one_run_each() {
    // A circle clear of every edge is one closed run ("Circle" already); an
    // arc drawn edge to edge is one open run ("Arc" already).
    let mut w = front_wall();
    let wall = w.face;
    let g = w.circle_geom(5.0 * MM, 5.0 * MM, MM);
    let ring = w.ring(5.0 * MM, 5.0 * MM, MM, 24, 0.0);
    w.draw_loop(wall, &ring, Some(g)).expect("a clear circle");
    let runs = circle_runs(&w, g);
    assert_eq!(runs.len(), 1);
    assert!(runs_close(&runs));
    let mut w = front_wall();
    let wall = w.face;
    let (path, curves, g) = arc_path(&w, 16);
    draw_path(&mut w.doc, None, w.id, wall, &path, &curves).expect("an arc edge to edge");
    let runs = circle_runs(&w, g);
    assert_eq!(runs.len(), 1);
    assert!(!runs_close(&runs));
}

#[test]
fn curve_runs_survive_save_and_reopen() {
    let (w, _, g) = strip_with_circle(true);
    // The reopened document lists the same curve (the geometry is the same).
    let runs = circle_runs(&w, g);
    let reopened = Document::load(&w.doc.save()).expect("reopens");
    let again = runs_in(&units_on(&reopened, None, w.id, w.plane), g);
    assert_eq!(runs.len(), 4);
    assert_eq!(again.len(), runs.len());
    let (sa, sb): (Vec<_>, Vec<_>) = (
        runs.iter().flatten().copied().collect(),
        again.iter().flatten().copied().collect(),
    );
    assert_eq!(sa.len(), sb.len());
    for (p, q) in sa {
        assert!(
            sb.iter()
                .any(|&(x, y)| (x == p && y == q) || (x == q && y == p)),
            "every segment of the circle survives the round trip exactly"
        );
    }
}

// ------------------------------------------------- curves: erase

#[test]
fn deleting_a_circle_drawn_across_a_rectangle_gives_the_rectangle_back() {
    for phase in [0.0, PI / 24.0] {
        for dx in [-0.5 * MM, 0.0, 0.5 * MM] {
            let (mut w, rect_state, g) = rect_with_crossing_circle(dx, phase);
            delete_circle_and_check(&mut w, g, 24, &rect_state);
        }
    }
}

#[test]
fn deleting_an_inscribed_circle_gives_the_rectangle_back() {
    let (mut w, rect_state, g) = rect_with_inscribed_circle();
    delete_circle_and_check(&mut w, g, 24, &rect_state);
}

#[test]
fn deleting_a_circle_crossing_a_strip_twice_gives_the_strip_back() {
    let (mut w, rect_state, g) = strip_with_circle(true);
    delete_circle_and_check(&mut w, g, 24, &rect_state);
}

#[test]
fn deleting_an_arc_across_an_imprint_gives_the_face_back() {
    let (mut w, rect_state, g) = wall_with_arc_over_rect();
    delete_circle_and_check(&mut w, g, 24, &rect_state);
}

#[test]
fn deleting_a_circle_is_one_undo_entry_with_exact_undo_and_redo() {
    let (mut w, _, g) = rect_with_crossing_circle(0.5 * MM, PI / 24.0);
    let with_circle = w.obj().clone();
    let bytes_circle = w.doc.save();
    let depth = w.doc.undo_depth();
    delete_circle(&mut w, g, 24).expect("deletes");
    assert_eq!(w.doc.undo_depth(), depth + 1, "one entry");
    let without = w.obj().clone();
    let bytes_without = w.doc.save();
    for _ in 0..3 {
        w.doc.undo().expect("undo");
        assert!(objects_equivalent(w.obj(), &with_circle));
        assert_eq!(w.doc.save(), bytes_circle);
        assert_eq!(circle_runs(&w, g).len(), 2, "the circle is back");
        w.doc.redo().expect("redo");
        assert!(objects_equivalent(w.obj(), &without));
        assert_eq!(w.doc.save(), bytes_without);
    }
}

#[test]
fn deleting_the_circle_on_the_ground_gives_the_rectangle_back_too() {
    // Ground parity in its own terms: a sketch holding the rectangle and a
    // circle across its side, with the circle's edges removed, has exactly
    // the rectangle's regions again — what the solid's face group shows
    // after the circle's erase.
    let (mut w, rect_state, g) = rect_with_crossing_circle(0.5 * MM, PI / 24.0);
    let before = w.obj().clone();
    let segs = circle_segments(&w, g, 24);
    let ground = ground_regions(&ground_without(&w, &before, &segs));
    let n = w.plane.normal();
    assert!(multiset_match(
        &straightened(&ground, n),
        &straightened(&rect_state, n),
        true
    ));
    delete_circle(&mut w, g, 24).expect("deletes");
    assert!(multiset_match(
        &straightened(&group_rings(w.obj(), &w), n),
        &straightened(&ground, n),
        true
    ));
}

#[test]
fn deleting_a_circle_leaving_two_shapes_touching_at_a_point_keeps_both_faces() {
    // A triangle touching the rectangle's right side at one point, drawn
    // inside a circle across that side (it crosses the circle twice).
    // Without the circle the triangle and the rectangle meet at that single
    // point: the wall holds them as two holes sharing it, as the ground's
    // region does, and both stay faces.
    let (mut w, s) = wall_with_rect();
    let side_x = RX + RW;
    let (cy, r) = (RY + RH / 2.0, MM);
    let g = w.circle_geom(side_x, cy, r);
    let ring = w.ring(side_x, cy, r, 24, PI / 24.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the circle across the side");
    let tip = w.at(side_x, cy);
    let tri = vec![
        tip,
        w.at(side_x + 2.0 * MM, cy - MM),
        w.at(side_x + 2.0 * MM, cy + MM),
    ];
    let seed = face_at(&w, side_x + 3.0 * MM, cy - 3.0 * MM);
    w.draw_loop(seed, &tri, None)
        .expect("the triangle touching the side inside the circle");
    let segs = circle_segments(&w, g, 24);
    let ground = ground_without(&w, w.obj(), &segs);
    assert!(!ground_has_pinch(&ground), "the ground's rings are simple");
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = delete_circle(&mut w, g, 24).expect("the circle erases");
    assert!(report.kept_as_sketch.is_empty(), "nothing kept as drawing");
    assert_eq!(group_faces(&before, &w.plane).len(), 7);
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "the wall, the rectangle, and the triangle"
    );
    assert!(
        has_touching_loops(w.obj(), &w.plane),
        "holes sharing the tip"
    );
    assert_eq!(owned_edge_count(&w), 0);
    let n = w.plane.normal();
    assert!(multiset_match(
        &straightened(&group_rings(w.obj(), &w), n),
        &straightened(&ground_regions(&ground), n),
        true
    ));
    assert_invariants(&mut w, &before, &bytes0, depth0);
}

#[test]
fn erasing_a_solids_own_edge_refuses_untouched() {
    // A wall edge on the box's outline: no drawing edge, fixed.
    let (mut w, _) = wall_with_rect();
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let objs = plane_objects(&w.doc, w.plane, None, w.id);
    let outline = w.rect(0.0, 0.0, 20.0 * MM, 10.0 * MM);
    let err = w
        .doc
        .draft(w.plane, None, &objs, "Erase", |sk| {
            let e = sk
                .edge_at_positions(outline[0], outline[1])
                .expect("the wall's bottom edge");
            sk.remove_edge(e).map(|_| ())
        })
        .err();
    assert_eq!(err, Some(DocumentError::DraftBoundaryEdgeTouched));
    assert!(objects_equivalent(w.obj(), &before), "object untouched");
    assert_eq!(w.doc.save(), bytes0, "saved bytes untouched");
    assert_eq!(w.doc.undo_depth(), depth0, "no undo entry");
}

// ------------------------------------------------- open paths (lines)

#[test]
fn a_line_across_a_rect_imprint_from_wall_edge_to_wall_edge_splits_every_face() {
    // Straight, and bent inside the rectangle: the wall and the rectangle
    // each split in two, one undo entry.
    for bent in [false, true] {
        let (mut w, _) = wall_with_rect();
        let y = RY + RH / 2.0;
        let mut path = vec![w.at(0.0, y)];
        if bent {
            path.push(w.at(RX + RW / 2.0, y + 0.5 * MM));
        }
        path.push(w.at(20.0 * MM, y));
        let seed = w.face;
        draw_path_and_check(&mut w, seed, path);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    }
}

#[test]
fn a_line_drawn_from_the_rectangle_side_works_from_either_face() {
    // Seeded on the rectangle: the same plan, the same result.
    let (mut w, s) = wall_with_rect();
    let y = RY + RH / 2.0;
    let path = vec![w.at(0.0, y), w.at(20.0 * MM, y)];
    draw_path_and_check(&mut w, s, path);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
}

#[test]
fn a_line_from_an_imprint_side_around_its_corner_back_to_it_cuts_a_lune() {
    let (mut w, _) = wall_with_rect();
    let path = vec![
        w.at(RX + RW - MM, RY + RH),
        w.at(RX + RW + 2.0 * MM, RY + RH + 2.0 * MM),
        w.at(RX + RW, RY + MM),
    ];
    let seed = w.face;
    draw_path_and_check(&mut w, seed, path);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
}

#[test]
fn a_line_crossing_an_earlier_line_splits_both_halves() {
    let mut w = front_wall();
    let wall = w.face;
    let first = vec![mm(&w, 0.0, 5.0), mm(&w, 20.0, 5.0)];
    draw_path_and_check(&mut w, wall, first);
    let seed = face_at(&w, 10.0 * MM, 2.0 * MM);
    let second = vec![mm(&w, 10.0, 0.0), mm(&w, 10.0, 10.0)];
    draw_path_and_check(&mut w, seed, second);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
}

#[test]
fn a_line_across_two_imprints_bridges_them_and_splits_the_wall() {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 3.0, 2.0, 4.0, 3.0);
    imprint_mm(&mut w, wall, 9.0, 2.0, 4.0, 3.0);
    let path = vec![mm(&w, 0.0, 3.5), mm(&w, 20.0, 3.5)];
    let seed = w.face;
    draw_path_and_check(&mut w, seed, path);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 6);
}

#[test]
fn a_line_touching_the_edge_partway_cuts_at_every_touch() {
    // Bottom edge → up → bottom edge → up → bottom edge: two cuts, three
    // faces, the touch in the middle an anchor like any other.
    let mut w = front_wall();
    let wall = w.face;
    let path = vec![
        mm(&w, 5.0, 0.0),
        mm(&w, 8.0, 3.0),
        mm(&w, 11.0, 0.0),
        mm(&w, 14.0, 3.0),
        mm(&w, 17.0, 0.0),
    ];
    draw_path_and_check(&mut w, wall, path);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
}

#[test]
fn a_line_partly_along_an_edge_cuts_only_where_it_leaves_it() {
    // Along the bottom edge from x = 2 to x = 6 mm, then up to the top:
    // the run along the edge cuts nothing. (The ground splits the bottom
    // edge where the line started on it, a straight-through vertex the
    // solid does not keep; the regions are the same.)
    let mut w = front_wall();
    let wall = w.face;
    let path = vec![mm(&w, 2.0, 0.0), mm(&w, 6.0, 0.0), mm(&w, 6.0, 10.0)];
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    draw_path(&mut w.doc, None, w.id, wall, &path, &[]).expect("the line draws");
    let n = w.plane.normal();
    let ground = ground_regions(&ground_edit(w.plane, &before, &path, &[]));
    assert!(multiset_match(
        &straightened(&group_rings(w.obj(), &w), n),
        &straightened(&ground, n),
        true
    ));
    assert_invariants(&mut w, &before, &bytes0, depth0);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 2);
}

#[test]
fn a_dangling_line_is_loose_owned_drawing_the_solid_untouched() {
    let (mut w, _) = wall_with_rect();
    let wall = w.face;
    // Ends in the wall's interior.
    let into = vec![mm(&w, 0.0, 1.0), mm(&w, 2.0, 1.0)];
    assert_path_owned(&mut w, wall, &into, 1);
    w.doc.undo().expect("undo");
    // Starts in the interior, crosses the rectangle, ends on the edge.
    let out = vec![mm(&w, 1.0, 3.5), mm(&w, 20.0, 3.5)];
    // The part across the rectangle splits it (a chord from side to side);
    // the two stubs, from the interior to the rectangle's left side and
    // from its right side to the wall's edge, are loose owned drawing.
    let before = w.obj().clone();
    let (bytes0, depth0, owned0) = (w.doc.save(), w.doc.undo_depth(), owned_edge_count(&w));
    let (report, _) = draw_path(&mut w.doc, None, w.id, wall, &out, &[]).expect("the line draws");
    assert!(report.kept_as_sketch.is_empty());
    assert_eq!(group_faces(&before, &w.plane).len(), 2);
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "the rectangle splits"
    );
    assert_eq!(owned_edge_count(&w) - owned0, 2, "the two stubs");
    assert_undoable(&mut w, &before, &bytes0, depth0);
    w.doc.undo().expect("undo");
    // Touches nothing at all.
    let clear = vec![mm(&w, 15.0, 2.0), mm(&w, 16.0, 3.0)];
    assert_path_owned(&mut w, wall, &clear, 1);
}

#[test]
fn a_line_along_existing_edges_only_changes_nothing() {
    let (mut w, _) = wall_with_rect();
    let wall = w.face;
    let rect = w.rect(RX, RY, RW, RH);
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = draw_path(&mut w.doc, None, w.id, wall, &[rect[0], rect[1]], &[])
        .expect("an already drawn line");
    assert!(report.objects.is_empty() && report.sketches.is_empty());
    assert_eq!(w.doc.save(), bytes0, "saved bytes untouched");
    assert_eq!(w.doc.undo_depth(), depth0, "no undo entry");
}

#[test]
fn a_self_crossing_line_splits_the_wall_and_its_loop_is_a_face() {
    // The path runs from the wall's bottom edge to its top edge and crosses
    // itself once, closing a loop that hangs off the path at the crossing
    // point: the wall splits along the path, the loop is a face, and the
    // piece beside it holds it as a hole touching its outer loop at the
    // crossing, as on the ground.
    let mut w = front_wall();
    let wall = w.face;
    let path = vec![
        mm(&w, 5.0, 0.0),
        mm(&w, 9.0, 6.0),
        mm(&w, 9.0, 3.0),
        mm(&w, 5.0, 6.0),
        mm(&w, 5.0, 10.0),
    ];
    let before = w.obj().clone();
    let sk = ground_edit(w.plane, &before, &path, &[]);
    assert!(!ground_has_pinch(&sk), "the ground's rings are simple");
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let owned0 = owned_edge_count(&w);
    let (report, _) = draw_path(&mut w.doc, None, w.id, wall, &path, &[]).expect("the line draws");
    assert!(report.kept_as_sketch.is_empty(), "nothing the face refuses");
    assert_group_matches_ground(&w, &sk);
    assert_eq!(ground_regions(&sk).len(), 3);
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "two halves and the loop"
    );
    assert!(
        has_touching_loops(w.obj(), &w.plane),
        "the loop's touching hole"
    );
    assert_eq!(owned_edge_count(&w), owned0, "no loose drawing");
    assert_invariants(&mut w, &before, &bytes0, depth0);
}

#[test]
fn a_line_meeting_one_point_of_an_imprint_cuts_the_wall_around_it() {
    // From the wall's left edge to the rectangle's corner and on to the
    // wall's top edge, outside the rectangle: the cut splits the wall, and
    // the corner piece holds the rectangle as a hole touching its outer
    // loop at the corner, as on the ground.
    let (mut w, _) = wall_with_rect();
    let wall = w.face;
    let path = vec![
        w.at(0.0, RY - MM),
        w.at(RX, RY),
        w.at(RX - 2.0 * MM, 10.0 * MM),
    ];
    let before = w.obj().clone();
    let sk = ground_edit(w.plane, &before, &path, &[]);
    assert!(!ground_has_pinch(&sk), "the ground's rings are simple");
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = draw_path(&mut w.doc, None, w.id, wall, &path, &[]).expect("the line draws");
    assert!(report.kept_as_sketch.is_empty());
    assert_group_matches_ground(&w, &sk);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
    assert!(has_touching_loops(w.obj(), &w.plane), "the touching hole");
    assert_invariants(&mut w, &before, &bytes0, depth0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Ground parity for line chains: a random rectangle imprint on a
    /// 0.5 mm lattice and a random chain from the wall's left edge to its
    /// right edge through one or two lattice points. The face group equals
    /// the ground's regions with I1–I7; nothing is kept as owned drawing
    /// (a chain through one corner of the imprint included: the ground's
    /// rings are simple, a hole sharing that corner).
    #[test]
    fn line_chains_match_the_ground(
        rx in 2u32..20, ry in 2u32..12, rw in 2u32..12, rh in 2u32..8,
        y0 in 0u32..=20, ys in proptest::collection::vec(0u32..=20, 1..3),
        y1 in 0u32..=20,
    ) {
        let half = |k: u32| f64::from(k) * 0.5;
        let (rx, ry, rw, rh) = (half(rx), half(ry), half(rw), half(rh));
        prop_assume!(rx + rw <= 19.0 && ry + rh <= 9.5);
        let mut w = front_wall();
        let wall = w.face;
        imprint_mm(&mut w, wall, rx, ry, rw, rh);
        let n = ys.len();
        let mut path = vec![mm(&w, 0.0, half(y0))];
        for (i, &y) in ys.iter().enumerate() {
            let x = 20.0 * (i + 1) as f64 / (n + 1) as f64;
            path.push(mm(&w, x, half(y)));
        }
        path.push(mm(&w, 20.0, half(y1)));
        let before = w.obj().clone();
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let seed = w.face;
        let sk = ground_edit(w.plane, &before, &path, &[]);
        let (report, _) = draw_path(&mut w.doc, None, w.id, seed, &path, &[])
            .expect("the chain draws");
        prop_assert!(report.kept_as_sketch.is_empty(), "nothing kept as drawing");
        prop_assert!(!ground_has_pinch(&sk), "the ground's rings are simple");
        assert_group_matches_ground(&w, &sk);
        // A chain lying wholly on one boundary of the wall (every point on
        // its bottom edge, or every point on its top edge) draws nothing
        // new: the draw succeeds and changes nothing, so it adds no undo
        // entry (`a_chain_wholly_on_a_boundary_changes_nothing`). Every
        // other chain keeps I1–I7, one undo entry included.
        let all_on = |y: u32| y0 == y && y1 == y && ys.iter().all(|&v| v == y);
        if all_on(0) || all_on(20) {
            prop_assert_eq!(w.doc.undo_depth(), depth0, "no undo entry");
            prop_assert_eq!(w.doc.save(), bytes0, "the bytes are unchanged");
        } else {
            assert_invariants(&mut w, &before, &bytes0, depth0);
        }
    }
}

/// The shrunk `line_chains_match_the_ground` case (rx = ry = rw = rh = 2,
/// every y = 0): a chain lying wholly on the wall's bottom edge, then the
/// same along its top edge. The ground keeps the face as it was, so the
/// draw is Ok and changes nothing: the bytes and the undo depth are as
/// before, and no drawing is kept.
#[test]
fn a_chain_wholly_on_a_boundary_changes_nothing() {
    for y in [0.0, 10.0] {
        let mut w = front_wall();
        let wall = w.face;
        imprint_mm(&mut w, wall, 1.0, 1.0, 1.0, 1.0);
        let path = vec![mm(&w, 0.0, y), mm(&w, 10.0, y), mm(&w, 20.0, y)];
        let before = w.obj().clone();
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let sk = ground_edit(w.plane, &before, &path, &[]);
        let (report, _) = draw_path(&mut w.doc, None, w.id, wall, &path, &[])
            .expect("a chain on the boundary draws");
        assert!(report.kept_as_sketch.is_empty(), "y = {y}: nothing kept");
        assert_group_matches_ground(&w, &sk);
        assert_eq!(w.doc.undo_depth(), depth0, "y = {y}: no undo entry");
        assert_eq!(w.doc.save(), bytes0, "y = {y}: the bytes are unchanged");
        assert!(objects_equivalent(w.obj(), &before), "y = {y}: unchanged");
    }
}

/// The shrunk `line_chains_match_the_ground` case: a chain running along
/// the wall's top edge, then down to the far corner. The ground keeps the
/// vertex where the along-the-edge span ends; the solid inserts none. Same
/// regions.
#[test]
fn a_chain_along_an_edge_then_across_matches_the_ground() {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 1.0, 1.0, 1.0, 1.0);
    let path = vec![
        mm(&w, 0.0, 10.0),
        mm(&w, 20.0 / 3.0, 10.0),
        mm(&w, 40.0 / 3.0, 10.0),
        mm(&w, 20.0, 0.0),
    ];
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let sk = ground_edit(w.plane, &before, &path, &[]);
    let seed = w.face;
    draw_path(&mut w.doc, None, w.id, seed, &path, &[]).expect("the chain cuts the wall");
    assert_group_matches_ground(&w, &sk);
    assert_invariants(&mut w, &before, &bytes0, depth0);
}

// ------------------------------------- runs whose facets lost every claim

#[test]
fn a_circle_whose_runs_lost_every_claim_is_still_one_whole_circle() {
    // The circle crosses the rectangle's top and right side mid-facet: the
    // cut facets keep no claim (F11), and some runs consist of nothing but
    // such fragments. They are still pieces of the drawn circle: the runs
    // hold every one, close, and erase back to the rectangle.
    let mut w = front_wall();
    let wall = w.face;
    let s = imprint_mm(&mut w, wall, 2.0, 1.0, 6.0, 5.97);
    let before = group_rings(w.obj(), &w);
    let (cx, cy, r) = (7.5 * MM, 5.5 * MM, 1.5 * MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, 0.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the circle across the corner draws");
    let runs = circle_runs(&w, g);
    assert!(runs_close(&runs), "a whole circle");
    // Every chord lying on the circle's facets (each point and each
    // segment midpoint within one facet's sagitta of the rim) is on a run.
    let sagitta = r * (1.0 - (PI / 24.0).cos());
    let on_rim = |p: Point3| {
        let d = (p - g.center).length();
        d <= r + 1e-12 && d >= r - sagitta - 1e-12
    };
    let obj = w.obj();
    let group = group_faces(obj, &w.plane);
    let face_of = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
    let mut chords = 0;
    for e in obj.edges().keys() {
        let edge = obj.edges()[e];
        let Some(tw) = edge.twin_half_edge else {
            continue;
        };
        let (fa, fb) = (face_of(edge.half_edge), face_of(tw));
        if fa == fb || !group.contains(&fa) || !group.contains(&fb) {
            continue;
        }
        let (a, b) = obj.edge_endpoints(e).expect("edge");
        if on_rim(a) && on_rim(b) && on_rim(a + (b - a) * 0.5) {
            chords += 1;
            assert!(
                runs.iter()
                    .flatten()
                    .any(|&(p, q)| (p.approx_eq(a, 1e-12) && q.approx_eq(b, 1e-12))
                        || (p.approx_eq(b, 1e-12) && q.approx_eq(a, 1e-12))),
                "the chord {a:?}-{b:?} is on a run"
            );
        }
    }
    assert_eq!(runs.iter().map(Vec::len).sum::<usize>(), chords);
    delete_circle_and_check(&mut w, g, 24, &before);
}

#[test]
fn erasing_a_circle_whose_runs_lost_every_claim_gives_the_rectangle_back() {
    // The same scene: the erase (every facet and fragment on the rim)
    // restores the rectangle exactly, with no loose edge left behind.
    let mut w = front_wall();
    let wall = w.face;
    let s = imprint_mm(&mut w, wall, 2.0, 1.0, 6.0, 5.97);
    let before = group_rings(w.obj(), &w);
    let (cx, cy, r) = (7.5 * MM, 5.5 * MM, 1.5 * MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, 0.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the circle across the corner draws");
    delete_circle_and_check(&mut w, g, 24, &before);
}

proptest! {
    // About two in five scenes land the circle cut into runs, so 800
    // cases delete a cut-apart circle a few hundred times.
    #![proptest_config(ProptestConfig::with_cases(800))]

    /// Deleting a drawn circle restores what was there before it, whatever
    /// it crossed: one to three random rectangles on the wall, then a
    /// 24-gon circle at a random phase. Erasing every run of the circle
    /// gives back the face group from before the circle (straight-through
    /// vertices aside), with I1–I7 and exact undo/redo.
    #[test]
    fn erasing_a_cut_apart_circle_restores_the_drawing_before_it(
        rects in proptest::collection::vec((0.5f64..15.0, 0.5f64..7.0, 1.0f64..8.0, 1.0f64..5.0), 1..4),
        cx in 1.0f64..19.0, cy in 1.0f64..9.0, r in 0.4f64..3.5, phase in 0.0f64..(2.0 * PI),
    ) {
        let mut w = front_wall();
        for (x, y, rw, rh) in rects {
            if x + rw > 19.5 || y + rh > 9.5 {
                continue;
            }
            let seed = face_at(&w, 19.8 * MM, 9.8 * MM);
            let rect = w.rect(x * MM, y * MM, rw * MM, rh * MM);
            // A rectangle the face cannot hold (a single-point touch) stays
            // as owned drawing; it is simply not drawn here.
            let (report, _) = w.draw_loop(seed, &rect, None).expect("the rectangle draws");
            if !report.kept_as_sketch.is_empty() {
                w.doc.undo().expect("undo the kept rectangle");
            }
        }
        prop_assume!(cx - r > 0.2 && cx + r < 19.8 && cy - r > 0.2 && cy + r < 9.8);
        let before = group_rings(w.obj(), &w);
        let g = w.circle_geom(cx * MM, cy * MM, r * MM);
        let ring = w.ring(cx * MM, cy * MM, r * MM, 24, phase);
        let Some(seed) = group_faces(w.obj(), &w.plane)
            .into_iter()
            .find(|&f| w.obj().face_contains_point(f, ring[0]))
        else {
            return Ok(());
        };
        let (report, _) = w.draw_loop(seed, &ring, Some(g)).expect("the circle draws");
        if !report.kept_as_sketch.is_empty() {
            return Ok(());
        }
        prop_assert!(!circle_runs(&w, g).is_empty());
        delete_circle_and_check(&mut w, g, 24, &before);
    }
}

#[test]
fn erasing_a_circle_that_poked_a_micron_past_a_side_restores_the_rectangle() {
    // Shrunk property cases: a 24-gon circle that crosses a rectangle's
    // side 2.9 µm above its corner, and one whose lowest vertex pokes
    // 15 µm past the bottom side. Erasing the circle gives the rectangle
    // its face back.
    for (rect, (cx, cy, r)) in [
        (
            (4.151527370947905, 5.075230374247028, 5.268310331836831, 1.0),
            (7.351718651525126, 3.207942747027647, 2.803236265179927),
        ),
        (
            (8.807391862710505, 2.4013831908149372, 1.0, 1.0),
            (9.87616814461048, 4.585555058268728, 2.1994210451570675),
        ),
    ] {
        let mut w = front_wall();
        let seed = face_at(&w, 19.8 * MM, 9.8 * MM);
        let rect = w.rect(rect.0 * MM, rect.1 * MM, rect.2 * MM, rect.3 * MM);
        w.draw_loop(seed, &rect, None).expect("the rectangle draws");
        let before = group_rings(w.obj(), &w);
        let g = w.circle_geom(cx * MM, cy * MM, r * MM);
        let ring = w.ring(cx * MM, cy * MM, r * MM, 24, 0.0);
        let seed = face_at(&w, 19.8 * MM, 9.8 * MM);
        w.draw_loop(seed, &ring, Some(g)).expect("the circle draws");
        delete_circle_and_check(&mut w, g, 24, &before);
    }
}

// ------------------------------- the drawn region beside an imprint

#[test]
fn a_rectangle_flush_with_an_imprint_on_two_sides_names_its_region() {
    // The widest vertex-free scanline band of the drawn loop lies wholly
    // inside the imprint it encloses; the region is still found (it lies
    // beside the imprint, in another band).
    for x0 in [2.0, 3.0] {
        let mut w = top_of_box();
        let top = w.face;
        imprint_mm(&mut w, top, 3.0, 16.0, 13.0, 10.0);
        let wall = face_at(&w, 1.0 * MM, 1.0 * MM);
        let rect = w.rect(x0 * MM, 16.0 * MM, (27.0 - x0) * MM, 10.0 * MM);
        draw_and_check(&mut w, wall, rect, None);
    }
}
