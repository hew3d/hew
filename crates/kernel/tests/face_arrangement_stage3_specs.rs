//! Executable specs for ground-parity drawing on a solid's face, stage 3:
//! a closed loop drawn on a face that CROSSES the edges of the face's
//! coplanar group — an imprint's boundary, another imprint, a chord — or
//! runs flush against an imprint from outside imprints the way the same
//! shapes split a ground sketch. The drafting engine
//! ([`Document::draft`]) lowers each piece of the loop onto the face it
//! lies in: a piece inside an imprint cuts it from side to side, a piece
//! outside cuts the face around it from the imprint's hole loop back to the
//! same hole loop. All the cuts land as one undo entry.
//!
//! A loop meeting the group at a single point is topology too: the face
//! around it holds it as a hole sharing that vertex. What the face cannot
//! hold is never refused: a draw that would leave a face too small for a
//! saved file to reopen (twice its area under 1e-12 m²) stays as the
//! object's owned-sketch drawing (`DraftReport::kept_as_sketch`) with the
//! solid untouched; the part of a loop beyond the face's edge is free
//! drawing. Each is one undo entry with exact undo and redo.
//!
//! Every accepting spec checks the stage invariants (see
//! `face_arrangement_specs.rs`): I1 valid and watertight, I2
//! Euler–Poincaré unchanged, I3 group area conserved, I4 loops simple, I5 exact
//! undo/redo with byte-identical saves, I6 the face group equals the
//! ground's regions for the same lines, I7 the saved document reopens.

use std::f64::consts::PI;

use kernel::{CurveGeom, DraftReport, EdgeId, FaceId, KernelOp, Object, Point3, StickyError, tol};

use crate::face_arrangement_support;
use face_arrangement_support::*;

// --------------------------------------------------------------- helpers

/// `p` in the wall's 2-D frame.
fn xy(w: &Wall, p: Point3) -> (f64, f64) {
    ((p - w.origin).dot(w.u), (p - w.origin).dot(w.v))
}

/// Whether `p` lies inside the closed polygon `ring` (both 2-D), its
/// boundary within `tol` included.
fn in_closed_polygon(p: (f64, f64), ring: &[(f64, f64)], tol: f64) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let len2 = ex * ex + ey * ey;
        let t = (((p.0 - a.0) * ex + (p.1 - a.1) * ey) / len2).clamp(0.0, 1.0);
        let (qx, qy) = (a.0 + ex * t, a.1 + ey * t);
        if (p.0 - qx).hypot(p.1 - qy) <= tol {
            return true;
        }
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) * ex / ey {
            inside = !inside;
        }
    }
    inside
}

/// F9: the reported region is a face of the drawn shape — every vertex of
/// its outer ring lies inside the drawn loop or on it.
fn assert_region_in_loop(w: &Wall, region: FaceId, loop_path: &[Point3]) {
    let ring: Vec<(f64, f64)> = loop_path.iter().map(|&p| xy(w, p)).collect();
    let obj = w.obj();
    for p in obj.loop_positions(obj.faces()[region].outer_loop) {
        assert!(
            in_closed_polygon(xy(w, p), &ring, 1e-9),
            "the reported region reaches outside the drawn loop at {:?}",
            xy(w, p)
        );
    }
}

/// Draws `loop_path` on `seed` with I1–I7 and F9; returns the region face.
fn cross_and_check(
    w: &mut Wall,
    seed: FaceId,
    loop_path: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> FaceId {
    let region = draw_and_check(w, seed, loop_path.clone(), curve);
    assert_region_in_loop(w, region, &loop_path);
    region
}

/// Edges in the sketches the wall's object owns.
fn owned_edge_count(w: &Wall) -> usize {
    w.doc
        .object_owned_sketches(w.id)
        .iter()
        .map(|&s| w.doc.sketch(s).expect("sketch").edges().len())
        .sum()
}

/// Edges in the free sketches (no owner).
fn free_edge_count(w: &Wall) -> usize {
    w.doc
        .sketch_ids()
        .into_iter()
        .filter(|&s| w.doc.sketch_owner_object(s).is_none())
        .map(|s| w.doc.sketch(s).expect("sketch").edges().len())
        .sum()
}

/// After a draw that is the latest undo entry: the object is valid and the
/// group area is conserved (the group's face is only ever re-partitioned),
/// the document reopens, and undo and redo restore the saved bytes exactly,
/// three times over.
fn assert_undoable(w: &mut Wall, before: &Object, bytes0: &[u8], depth0: usize) {
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo entry");
    w.obj().validate().expect("valid after the draw");
    assert_eq!(w.obj().watertight(), before.watertight());
    assert_eq!(euler(w.obj()), euler(before), "V - E + F - H unchanged");
    let (a0, a1) = (group_area(before, &w.plane), group_area(w.obj(), &w.plane));
    assert!((a0 - a1).abs() <= 1e-12 * a0.abs(), "group area conserved");
    assert!(no_pinch(w.obj(), &w.plane), "no pinched loop committed");
    let bytes1 = w.doc.save();
    assert_reopens(&bytes1);
    let after = w.obj().clone();
    for _ in 0..3 {
        w.doc.undo().expect("undo the draw");
        assert!(objects_equivalent(w.obj(), before), "undo restores");
        assert_eq!(w.doc.save(), bytes0, "undo restores the saved bytes");
        w.doc.redo().expect("redo the draw");
        assert!(objects_equivalent(w.obj(), &after), "redo restores");
        assert_eq!(w.doc.save(), bytes1, "redo restores the saved bytes");
    }
}

/// Draws `loop_path` through one draft and checks [`assert_undoable`].
fn draw_any_and_check(
    w: &mut Wall,
    seed: FaceId,
    loop_path: &[Point3],
    curve: Option<CurveGeom>,
) -> DraftReport {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let report = w
        .draw_loop(seed, loop_path, curve)
        .expect("the loop draws")
        .0;
    assert_undoable(w, &before, &bytes0, depth0);
    report
}

/// A loop the face cannot hold stays as the object's owned-sketch drawing:
/// the draw reports it kept, the solid is untouched, nothing is free, and
/// the loop's edges (plus the splits where it meets the group) are in the
/// owned sketch. Undo/redo exact. Returns the owned edge count.
fn kept_and_check(
    w: &mut Wall,
    seed: FaceId,
    loop_path: &[Point3],
    curve: Option<CurveGeom>,
) -> usize {
    let before = w.obj().clone();
    let owned0 = owned_edge_count(w);
    let report = draw_any_and_check(w, seed, loop_path, curve);
    assert!(
        !report.kept_as_sketch.is_empty(),
        "reported as kept drawing"
    );
    assert!(
        objects_equivalent(w.obj(), &before),
        "the solid is untouched"
    );
    assert_eq!(free_edge_count(w), 0, "nothing is free");
    let owned = owned_edge_count(w) - owned0;
    assert!(owned >= loop_path.len(), "the whole loop is owned drawing");
    owned
}

/// The run holding `edge` separates two different faces of the group (what
/// the app's Delete acts on as a chord).
fn assert_interior_run(w: &Wall, edge: EdgeId) {
    let obj = w.obj();
    let e = obj.edges()[edge];
    let fa = obj.loops()[obj.half_edges()[e.half_edge].loop_id].face;
    let tw = e.twin_half_edge.expect("interior edge");
    let fb = obj.loops()[obj.half_edges()[tw].loop_id].face;
    let group = group_faces(obj, &w.plane);
    assert_ne!(fa, fb);
    assert!(group.contains(&fa) && group.contains(&fb), "a group chord");
}

/// The front wall carrying the maintainer's 9.4 × 3.8 mm rectangle:
/// `(wall, rect sub-face)`.
fn wall_with_rect() -> (Wall, FaceId) {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    (w, s)
}

/// Every face of the group, pushed out and in, then undone: the identity
/// (rule 3). `faces` is the expected group size.
fn push_pull_every_face_and_undo(w: &mut Wall, faces: usize) {
    let group = group_faces(w.obj(), &w.plane);
    assert_eq!(group.len(), faces);
    let before = w.obj().clone();
    let bytes = w.doc.save();
    // Faces re-mint on undo: address each by a ring snapshot.
    let rings: Vec<Rings> = group.iter().map(|&f| face_rings(&before, f)).collect();
    for ring in &rings {
        for d in [MM, -0.5 * MM] {
            let face = group_faces(w.obj(), &w.plane)
                .into_iter()
                .find(|&f| rings_match(&face_rings(w.obj(), f), ring, false))
                .expect("the face again");
            w.doc
                .apply_object_op(w.id, KernelOp::PushPull { face, distance: d })
                .unwrap_or_else(|e| panic!("push/pull {d} of a result face: {e:?}"));
            w.obj().validate().expect("valid after push/pull");
            w.doc.undo().expect("undo the push/pull");
            assert!(objects_equivalent(w.obj(), &before), "undo restores");
            assert_eq!(w.doc.save(), bytes, "undo restores the bytes");
        }
    }
}

// ------------------------------------------ a circle crossing an imprint

#[test]
fn a_circle_centered_on_an_imprint_side_splits_both_sides() {
    // Centered on the rectangle's right side, vertices 6 and 18 land on it:
    // the left half cuts the rectangle, the right half cuts the wall from
    // the rectangle's hole back to it. Every facet stays whole.
    for seed_on_rect in [true, false] {
        let (mut w, s) = wall_with_rect();
        let (cx, cy, r) = (RX + RW, RY + R, MM);
        let g = w.circle_geom(cx, cy, r);
        let ring = w.ring(cx, cy, r, 24, 0.0);
        let seed = if seed_on_rect { s } else { w.face };
        cross_and_check(&mut w, seed, ring, Some(g));
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
        assert_eq!(claims_on(w.obj(), g), 24);
    }
}

#[test]
fn a_circle_crossing_an_imprint_side_makes_a_lens_on_each_side() {
    // Turned 7.5° so no vertex sits on the side: two proper crossings of
    // the one side cut two facets mid-way (they lose the claim, F11).
    // Centered inside: the rectangle loses a disc-with-a-cap-cut-off, the
    // wall a cap. Centered outside: the rectangle loses a lens, the wall a
    // crescent.
    for dx in [-0.5 * MM, 0.5 * MM] {
        for seed_on_rect in [true, false] {
            let (mut w, s) = wall_with_rect();
            let (cx, cy, r) = (RX + RW + dx, RY + R, MM);
            let g = w.circle_geom(cx, cy, r);
            let ring = w.ring(cx, cy, r, 24, PI / 24.0);
            let seed = if seed_on_rect { s } else { w.face };
            cross_and_check(&mut w, seed, ring, Some(g));
            assert_eq!(group_faces(w.obj(), &w.plane).len(), 4, "dx {dx}");
            assert_eq!(claims_on(w.obj(), g), 22);
        }
    }
}

#[test]
fn a_circle_overlapping_an_imprint_by_five_microns_cuts_a_thin_lens() {
    // From outside, reaching 5 µm into the rectangle past its right side:
    // the lens inside the rectangle is 5 µm thick.
    let (mut w, _) = wall_with_rect();
    let (cx, cy, r) = (RX + RW + MM - 5e-6, RY + R, MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, 0.0);
    let wall = w.face;
    cross_and_check(&mut w, wall, ring, Some(g));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    assert_eq!(claims_on(w.obj(), g), 22);
}

#[test]
fn a_rectangle_overlapping_a_circle_imprint_matches_the_ground() {
    // A disc imprint, then a rectangle whose corner lies inside it: the
    // corner cuts the disc, the rest cuts the wall around the disc's hole.
    for seed_on_disc in [true, false] {
        let mut w = front_wall();
        let (cx, cy, r) = (8.0 * MM, 5.0 * MM, 2.0 * MM);
        let g = w.circle_geom(cx, cy, r);
        let disc_ring = w.ring(cx, cy, r, 24, 0.0);
        let wall = w.face;
        let disc = draw_region(&mut w, wall, disc_ring, Some(g));
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 2, "the disc whole");
        let rect = w.rect(9.3 * MM, 6.2 * MM, 4.0 * MM, 3.0 * MM);
        let seed = if seed_on_disc { disc } else { wall };
        cross_and_check(&mut w, seed, rect, None);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
        assert_eq!(claims_on(w.obj(), g), 22, "the two crossed facets drop it");
    }
}

// ------------------------------------------------- flush from outside

#[test]
fn a_rectangle_flush_against_an_imprint_from_outside_matches_the_ground() {
    // Part of the right side, then the whole right side with overhang
    // (anchored at the rectangle's two right corners).
    for (y, h) in [(RY + MM, MM), (RY - MM, RH + 2.0 * MM)] {
        for seed_on_rect in [false, true] {
            let (mut w, s) = wall_with_rect();
            let flush = w.rect(RX + RW, y, 2.0 * MM, h);
            let seed = if seed_on_rect { s } else { w.face };
            cross_and_check(&mut w, seed, flush, None);
            assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
        }
    }
}

// ------------------------------- the maintainer's µm-overshoot circle

/// The maintainer's rectangle drawn `short` too short, then the R = 1.9 mm
/// circle at its left end: centered 1.9 mm in from the left and bottom
/// (tangent there, overshooting the top by `short`), or centered on the
/// rectangle's height (overshooting top and bottom by `short / 2`).
fn overshoot_case(
    mut w: Wall,
    short: f64,
    centered: bool,
) -> (Wall, FaceId, Vec<Point3>, CurveGeom) {
    let rect = w.rect(RX, RY, RW, RH - short);
    let wall = w.face;
    let rect_face = draw_region(&mut w, wall, rect, None);
    let cy = if centered {
        RY + (RH - short) / 2.0
    } else {
        RY + R
    };
    let g = w.circle_geom(RX + R, cy, R);
    let ring = w.ring(RX + R, cy, R, 24, 0.0);
    (w, rect_face, ring, g)
}

#[test]
fn an_inscribed_circle_overshooting_a_short_rectangle_by_microns_matches_the_ground() {
    // 10.3 µm is the maintainer's 3.7897 mm rectangle. Each overshoot is a
    // thin sliver of the circle outside the rectangle, cut from the wall: a
    // triangle `h` high on a base of 2h / tan 7.5°. A saved file recomputes
    // each face's plane from its outer loop, which needs twice its area to
    // reach 1e-12 m², or, failing that, the sliver to be at least 10 nm
    // wide (`tol::MIN_FACE_WIDTH`; the width of this one is just under h).
    // So 0.5 µm slivers both ways round are faces now, and so is 0.25 µm per
    // side (0.5 µm centered), which the area floor alone used to refuse: it
    // is kept and reopens. 15 nm one-sided is a face; 7.5 nm per side (15 nm
    // centered) is too thin for one: the circle stays as the rectangle's
    // owned-sketch drawing and the solid is untouched.
    for short in [10.3e-6, 5e-6, 0.5e-6, 15e-9] {
        for centered in [false, true] {
            for wall in [front_wall as fn() -> Wall, tilted_wall] {
                let (mut w, s, ring, g) = overshoot_case(wall(), short, centered);
                let h = if centered { short / 2.0 } else { short };
                let base = 2.0 * h / (PI / 24.0).tan();
                let twice_area = base * h;
                let width = twice_area / ((base + 2.0 * (base * base / 4.0 + h * h).sqrt()) / 2.0);
                if twice_area < 1e-12 && width < tol::MIN_FACE_WIDTH {
                    let owned = kept_and_check(&mut w, s, &ring, Some(g));
                    assert_eq!(owned, 28, "the circle and its four splits");
                    assert_eq!(claims_on(w.obj(), g), 0, "no solid edge claims it");
                    continue;
                }
                cross_and_check(&mut w, s, ring, Some(g));
                let (faces, claims) = if centered { (7, 20) } else { (6, 22) };
                assert_eq!(group_faces(w.obj(), &w.plane).len(), faces);
                assert_eq!(claims_on(w.obj(), g), claims);
            }
        }
    }
}

#[test]
fn nanometre_overshoots_are_kept_drawing_the_solid_untouched() {
    // Nanometre overshoots of the inscribed circle past a short rectangle:
    // - a sliver whose segment midpoints sit within POINT_MERGE of the side
    //   (overshoots up to 2 nm) is one line to every tolerance gate, and a
    //   thicker nanometre sliver (3 nm, 3.25 nm per side) would be a face
    //   far too small for a saved file to reopen: the circle stays as the
    //   rectangle's owned-sketch drawing, the solid untouched, one undo
    //   entry. (An overshoot within POINT_MERGE per side, centered, is a
    //   tangency instead: `a_nanometre_overshoot_within_the_merge_tolerance_is_a_tangency`.)
    // `(short, centered, owned edges)`: the circle's 24 facets plus a split
    // at each point where it meets the rectangle's sides.
    let cases = [
        (1.5e-9, false, 26),
        (2e-9, false, 26),
        (3e-9, true, 28),
        (3e-9, false, 26),
        (6.5e-9, true, 28),
    ];
    for (short, centered, owned) in cases {
        for wall in [front_wall as fn() -> Wall, tilted_wall] {
            let (mut w, s, ring, g) = overshoot_case(wall(), short, centered);
            let what = format!("short {short}, centered {centered}");
            assert_eq!(kept_and_check(&mut w, s, &ring, Some(g)), owned, "{what}");
            assert_eq!(group_faces(w.obj(), &w.plane).len(), 2, "{what}");
            assert_eq!(claims_on(w.obj(), g), 0, "{what}");
        }
    }
}

#[test]
fn a_nanometre_overshoot_within_the_merge_tolerance_is_a_tangency() {
    // An overshoot within POINT_MERGE per side (1 nm, centered: 2 nm and
    // 1.5 nm total) IS a tangency: the vertex moves onto the side and no
    // sliver is made. The circle lands as topology like the exactly
    // inscribed one, with I1-I7 (the undo currently diverges).
    for short in [2e-9, 1.5e-9] {
        for wall in [front_wall as fn() -> Wall, tilted_wall] {
            let (mut w, s, ring, g) = overshoot_case(wall(), short, true);
            let before = w.obj().clone();
            let bytes0 = w.doc.save();
            let depth0 = w.doc.undo_depth();
            let report = w.draw_loop(s, &ring, Some(g)).expect("draws").0;
            assert!(report.kept_as_sketch.is_empty(), "short {short}");
            let snapped = snapped_like_the_planner(&before, &w.plane, &ring);
            assert_ground_parity(&w, &before, &snapped);
            assert_invariants(&mut w, &before, &bytes0, depth0);
        }
    }
}

// -------------------------------------- crossing a chord and a hole edge

#[test]
fn a_circle_crossing_a_chord_and_the_imprint_side_matches_the_ground() {
    // The rectangle split by a vertical chord at x = RX + 5 mm; a circle
    // centered on the chord's top end crosses the chord inside the
    // rectangle and the rectangle's top side on both sides of the chord:
    // one cut in each half of the rectangle, one in the wall from the
    // rectangle's hole back to it, over the chord's end vertex.
    let (mut w, s) = wall_with_rect();
    let x = RX + 5.0 * MM;
    w.doc
        .apply_object_op(
            w.id,
            KernelOp::SplitFace {
                face: s,
                path: vec![w.at(x, RY), w.at(x, RY + RH)],
                restore: None,
                curves: Vec::new(),
            },
        )
        .expect("chord the rectangle");
    let (cx, cy, r) = (x, RY + RH, MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, PI / 24.0);
    let left = face_at(&w, x - MM, RY + MM);
    cross_and_check(&mut w, left, ring, Some(g));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 6);
    assert_eq!(claims_on(w.obj(), g), 21, "three facets cut mid-way");
}

#[test]
fn a_circle_crossing_one_imprint_and_enclosing_another_hands_it_to_the_lune() {
    // A second imprint sits inside the part of the circle outside the
    // rectangle: the cut around the rectangle's hole encloses it, so it
    // moves to the new face (as on the ground, where it is a hole of the
    // crescent).
    let (mut w, _) = wall_with_rect();
    let wall = w.face;
    let small = w.rect(RX + RW + 1.0 * MM, RY + 1.5 * MM, 0.5 * MM, 0.5 * MM);
    draw_region(&mut w, wall, small, None);
    let (cx, cy, r) = (RX + RW + 0.5 * MM, RY + R, 1.5 * MM);
    let g = w.circle_geom(cx, cy, r);
    let ring = w.ring(cx, cy, r, 24, PI / 24.0);
    cross_and_check(&mut w, wall, ring, Some(g));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 5);
    let lune = face_at(&w, RX + RW + 0.7 * MM, RY + R);
    assert_eq!(
        w.obj().faces()[lune].inner_loops.len(),
        1,
        "the lune holds it"
    );
}

// ------------------------------------- joined boundaries at single points

/// A circle overlapping two separate imprints, and a rectangle from the
/// wall's edge across an imprint, need a bridge and land as topology
/// (`face_arrangement_stage4_specs.rs`). These two join two boundaries at a
/// single point each: they land as topology too, the faces holding holes
/// that share those points, as on the ground.
#[test]
fn configurations_touching_at_single_points_land_as_topology() {
    let expected_faces =
        std::collections::BTreeMap::from([("tangent to both", 3), ("crossing and tangent", 4)]);
    let mut cases: Vec<(&str, Wall, Vec<FaceId>, Vec<Point3>)> = Vec::new();
    {
        // A circle tangent to the wall's bottom edge and to the imprint's
        // bottom side, once each.
        let (w, s) = wall_with_rect();
        let wall = w.face;
        let ring = w.ring(8.0 * MM, 1.0 * MM, 1.0 * MM, 24, 0.0);
        cases.push(("tangent to both", w, vec![wall, s], ring));
    }
    {
        // A circle crossing the imprint and tangent to the wall's top edge.
        let mut w = front_wall();
        let wall = w.face;
        let s = imprint_mm(&mut w, wall, 3.0, 6.0, 9.0, 2.0);
        let ring = w.ring(8.0 * MM, 8.5 * MM, 1.5 * MM, 24, 0.0);
        cases.push(("crossing and tangent", w, vec![wall, s], ring));
    }
    for (what, mut w, seeds, ring) in cases {
        let before = w.obj().clone();
        for seed in seeds {
            draw_and_check(&mut w, seed, ring.clone(), None);
            assert_eq!(
                group_faces(w.obj(), &w.plane).len(),
                expected_faces[what],
                "{what}"
            );
            assert!(has_touching_loops(w.obj(), &w.plane), "{what}");
            assert_eq!(owned_edge_count(&w), 0, "{what}");
            w.doc.undo().expect("undo");
            assert!(objects_equivalent(w.obj(), &before), "{what}");
        }
    }
}

#[test]
fn a_circle_leaving_the_face_across_an_imprint_splits_at_the_edge() {
    // The circle dips 0.5 mm below the wall's bottom edge and crosses the
    // imprint: the parts over the wall are topology (wall and imprint each
    // cut), the crescent beyond the edge is free drawing closed by the
    // wall's own edge stretch. Group area is conserved, one undo entry.
    for seed_on_rect in [false, true] {
        let (mut w, s) = wall_with_rect();
        let ring = w.ring(8.0 * MM, 1.5 * MM, 2.0 * MM, 24, PI / 24.0);
        let seed = if seed_on_rect { s } else { w.face };
        let report = draw_any_and_check(&mut w, seed, &ring, None);
        assert!(report.kept_as_sketch.is_empty());
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
        assert_eq!(owned_edge_count(&w), 0);
        assert_eq!(free_edge_count(&w), 8, "the crescent beyond the edge");
    }
}

// ------------------------------------- crossing result faces: both sides

#[test]
fn a_crossing_circle_cuts_the_rectangle_and_the_wall_around_it() {
    // The circle across the rectangle's right side: the rectangle keeps its
    // remainder and a lens inside the circle, the wall keeps its remainder
    // (around the rectangle's hole) and a lune inside the circle.
    let (mut w, s) = wall_with_rect();
    let ring = w.ring(RX + RW, RY + R, MM, 24, PI / 24.0);
    for seed in [s, w.face] {
        let before = w.obj().clone();
        draw_and_check(&mut w, seed, ring.clone(), None);
        let inside = face_at(&w, RX + RW - 0.5 * MM, RY + R);
        let outside = face_at(&w, RX + RW + 0.5 * MM, RY + R);
        let rest_rect = face_at(&w, RX + MM, RY + MM);
        let rest_wall = face_at(&w, 19.0 * MM, 9.0 * MM);
        let mut all = [inside, outside, rest_rect, rest_wall];
        all.sort();
        assert!(all.windows(2).all(|p| p[0] != p[1]), "four distinct faces");
        assert!(w.obj().faces()[inside].inner_loops.is_empty(), "a lens");
        assert_eq!(w.obj().faces()[rest_wall].inner_loops.len(), 1);
        assert!(
            w.obj()
                .face_contains_point(inside, w.at(RX + RW - 0.5 * MM, RY + R))
        );
        w.doc.undo().expect("undo");
        assert!(objects_equivalent(w.obj(), &before));
    }
}

// ----------------------------------------------- pushing the results

#[test]
fn every_face_of_a_crossing_result_pushes_and_pulls_and_undoes() {
    // The circle across the right side.
    let (mut w, s) = wall_with_rect();
    let ring = w.ring(RX + RW, RY + R, MM, 24, PI / 24.0);
    let g = w.circle_geom(RX + RW, RY + R, MM);
    w.draw_loop(s, &ring, Some(g)).expect("crossing circle");
    push_pull_every_face_and_undo(&mut w, 4);
    // The 5 µm overshoot, its sliver included.
    let (mut w, s, ring, g) = overshoot_case(front_wall(), 5e-6, false);
    w.draw_loop(s, &ring, Some(g)).expect("overshooting circle");
    push_pull_every_face_and_undo(&mut w, 6);
    // The rectangle over the disc imprint.
    let mut w = front_wall();
    let wall = w.face;
    let g = w.circle_geom(8.0 * MM, 5.0 * MM, 2.0 * MM);
    let disc = w.ring(8.0 * MM, 5.0 * MM, 2.0 * MM, 24, 0.0);
    w.draw_loop(wall, &disc, Some(g)).expect("disc");
    let rect = w.rect(9.3 * MM, 6.2 * MM, 4.0 * MM, 3.0 * MM);
    w.draw_loop(wall, &rect, None).expect("overlapping rect");
    push_pull_every_face_and_undo(&mut w, 4);
}

// ------------------------------------- deleting lines of a crossing

#[test]
fn deleting_an_overshoot_sliver_s_lines_tidies_it_either_way() {
    // The 5 µm sliver above the rectangle's top: delete its arc (the
    // sliver merges into the wall, the disc keeps a flat top) or the
    // rectangle's top between the two crossings (the sliver merges into
    // the disc, a whole circle again). Both are stage 2 merges; both undo.
    for delete_arc in [true, false] {
        let (mut w, s, ring, g) = overshoot_case(front_wall(), 5e-6, false);
        let after_rect = (w.obj().clone(), w.doc.save());
        w.draw_loop(s, &ring, Some(g)).expect("overshooting circle");
        let after_circle = (w.obj().clone(), w.doc.save());
        let obj = w.obj();
        let top = vertex_at(obj, ring[6]);
        // The sliver's two crossing points: the vertices on the
        // rectangle's top within 0.1 mm of the overshooting vertex.
        let top_y = RY + RH - 5e-6;
        let mut xs: Vec<Point3> = obj
            .vertices()
            .values()
            .map(|v| v.position)
            .filter(|&p| {
                let (x, y) = xy(&w, p);
                (y - top_y).abs() < 1e-12 && (x - (RX + R)).abs() < 0.1 * MM
            })
            .collect();
        xs.sort_by(|a, b| xy(&w, *a).0.total_cmp(&xy(&w, *b).0));
        assert_eq!(xs.len(), 2, "two crossings of the top");
        let edge = if delete_arc {
            edge_at(obj, xs[0], top)
        } else {
            edge_at(obj, xs[0], xs[1])
        };
        assert_interior_run(&w, edge);
        merge_and_check(&mut w, edge);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 5);
        if !delete_arc {
            let disc = face_at(&w, RX + R, RY + R);
            assert_eq!(
                w.obj()
                    .loop_half_edges(w.obj().faces()[disc].outer_loop)
                    .count(),
                26,
                "the whole 24-gon, its two cut facets in two pieces each"
            );
        }
        w.doc.undo().expect("undo the delete");
        assert!(objects_equivalent(w.obj(), &after_circle.0));
        assert_eq!(w.doc.save(), after_circle.1);
        w.doc.undo().expect("undo the circle");
        assert!(objects_equivalent(w.obj(), &after_rect.0));
        assert_eq!(w.doc.save(), after_rect.1);
    }
}

#[test]
fn deleting_the_lines_of_a_circle_across_an_imprint_side_merges_back() {
    // The circle centered on the right side: delete the right half's arc
    // (the half outside merges into the wall) or the rectangle side
    // between the two halves (one disc straddling the side).
    for delete_arc in [true, false] {
        let (mut w, s) = wall_with_rect();
        let (cx, cy, r) = (RX + RW, RY + R, MM);
        let g = w.circle_geom(cx, cy, r);
        let ring = w.ring(cx, cy, r, 24, 0.0);
        w.draw_loop(s, &ring, Some(g))
            .expect("the circle across the side");
        let obj = w.obj();
        let edge = if delete_arc {
            edge_at(obj, vertex_at(obj, ring[0]), vertex_at(obj, ring[1]))
        } else {
            edge_at(obj, vertex_at(obj, ring[6]), vertex_at(obj, ring[18]))
        };
        assert_interior_run(&w, edge);
        merge_and_check(&mut w, edge);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
        assert_eq!(claims_on(w.obj(), g), if delete_arc { 12 } else { 24 });
    }
}

// --------------------------------------------- the lattice property

/// A deterministic xorshift64* stream.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % u64::from(hi - lo + 1)) as u32
    }
}

/// Distance from `p` to segment `a–b`.
fn seg_dist(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn ring_dist(p: Point3, ring: &[Point3]) -> f64 {
    (0..ring.len())
        .map(|i| seg_dist(p, ring[i], ring[(i + 1) % ring.len()]))
        .fold(f64::INFINITY, f64::min)
}

/// A random rectangle imprint and a random 24-gon on a 0.1 mm lattice
/// (turned 0°, so contacts are exact tangencies at its extreme vertices,
/// or 7.5° or 4.5°, so they are crossings), the circle inside the wall or
/// touching its edge (biased toward tangency to the wall's bottom and to
/// the imprint's sides), drawn from the wall or from the imprint. The oracle reads the
/// contact sites off the GROUND sketch of the same lines (its vertices on
/// both the loop and a group edge). Every draw lands as face topology,
/// with I1–I7 and ground parity — including the loops that join the wall's
/// outer edge and the imprint at single points, and those touching the
/// group at exactly one site (a hole sharing that vertex, on the ground as
/// on the face).
///
/// Draws whose contacts fall in the band between the merge tolerance and
/// 0.1 µm (where the ground's parameter tolerance and the face's metric
/// one may disagree) are skipped.
#[test]
fn crossing_circles_on_a_lattice_match_the_ground() {
    let mut rng = Rng(0x51_7cc1_b727_220a);
    let q = 0.1 * MM;
    let (mut accepted, mut crossing, mut joins, mut single) = (0, 0, 0, 0);
    let mut draws = 0;
    while draws < 500 {
        let (x0, y0) = (rng.range(10, 100), rng.range(10, 50));
        let (rw, rh) = (rng.range(20, 80), rng.range(15, 40));
        let r = rng.range(3, 40);
        let (mut cx, mut cy) = (rng.range(0, 200), rng.range(0, 100));
        match rng.next() % 8 {
            0 | 1 => cy = r,
            2 => cx = x0 + r,
            3 => cx = x0.saturating_sub(r),
            4 => cy = y0 + rh + r,
            _ => {}
        }
        if cx < r || cy < r || cx + r > 200 || cy + r > 100 {
            continue;
        }
        let phase = [0.0, 0.0, PI / 24.0, PI / 40.0][(rng.next() % 4) as usize];
        let from_imprint = rng.next().is_multiple_of(2);
        let f = |v: u32| f64::from(v) * q;
        let mut w = front_wall();
        let wall = w.face;
        let rect = w.rect(f(x0), f(y0), f(rw), f(rh));
        let imprint = draw_region(&mut w, wall, rect.clone(), None);
        let ring = w.ring(f(cx), f(cy), f(r), 24, phase);
        let g = w.circle_geom(f(cx), f(cy), f(r));
        let outer = w.rect(0.0, 0.0, 20.0 * MM, 10.0 * MM);

        // ---- the oracle, from the ground ----
        let before = w.obj().clone();
        let sk = ground_with(w.plane, &before, &ring);
        let group: Vec<Point3> = rect.iter().chain(outer.iter()).copied().collect();
        // Near-ties: a loop vertex near (not on) a group edge, a group
        // vertex near (not on) the loop, two ground vertices nearly
        // coincident.
        let band = |d: f64| d > tol::POINT_MERGE && d < 1e-7;
        let gv: Vec<Point3> = sk.vertices().values().map(|v| v.position).collect();
        let near_tie = ring
            .iter()
            .any(|&p| band(ring_dist(p, &rect)) || band(ring_dist(p, &outer)))
            || group.iter().any(|&p| band(ring_dist(p, &ring)))
            || (0..gv.len()).any(|i| (i + 1..gv.len()).any(|j| (gv[i] - gv[j]).length() < 1e-7));
        if near_tie {
            continue;
        }
        draws += 1;
        let on_loop = |p: Point3| ring_dist(p, &ring) <= tol::POINT_MERGE;
        let wall_sites = gv
            .iter()
            .filter(|&&p| on_loop(p) && ring_dist(p, &outer) <= tol::POINT_MERGE)
            .count();
        let rect_sites = gv
            .iter()
            .filter(|&&p| on_loop(p) && ring_dist(p, &rect) <= tol::POINT_MERGE)
            .count();
        let joins_both = wall_sites > 0 && rect_sites > 0;
        let one_site = wall_sites + rect_sites == 1;

        let seed = if from_imprint { imprint } else { wall };
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let what = format!(
            "rect ({x0},{y0}) {rw}x{rh}, circle ({cx},{cy}) r {r} phase {phase}: \
             {wall_sites} wall / {rect_sites} rect sites"
        );
        let report = w
            .draw_loop(seed, &ring, Some(g))
            .expect("the circle draws")
            .0;
        assert!(report.kept_as_sketch.is_empty(), "{what}");
        assert!(
            !ground_has_pinch(&sk),
            "the ground's rings are simple: {what}"
        );
        accepted += 1;
        if rect_sites > 0 && phase != 0.0 {
            crossing += 1;
        }
        if joins_both {
            joins += 1;
        } else if one_site {
            single += 1;
        }
        if joins_both || one_site {
            assert!(has_touching_loops(w.obj(), &w.plane), "{what}");
        }
        assert_ground_parity(&w, &before, &ring);
        assert_invariants(&mut w, &before, &bytes0, depth0);
    }
    assert!(
        accepted > 150 && crossing > 50 && joins > 5 && single > 5,
        "coverage: {accepted} accepted ({crossing} crossing), {joins} joining both, \
         {single} touching once"
    );
}

// ------------------------------------------- every result reopens (I7)

/// The repro family: a 9 × 4 mm imprint at (3, 2) mm, then a 2 × 2 mm
/// rectangle drawn on the wall overlapping its top-right corner by `d` mm
/// each way, so the corner piece is a `d × d` face.
fn corner_overlap(d: f64) -> (Wall, Vec<Point3>) {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 3.0, 2.0, 9.0, 4.0);
    let rect = w.rect((12.0 - d) * MM, (6.0 - d) * MM, 2.0 * MM, 2.0 * MM);
    (w, rect)
}

#[test]
fn a_corner_face_too_small_to_reopen_stays_owned_drawing() {
    // A face's plane is recomputed from its outer loop when a file opens,
    // which needs twice its area to reach 1e-12 m², or the face to be at
    // least 10 nm wide (`tol::MIN_FACE_WIDTH`; a square is as wide as its
    // side): a 1 µm square corner reopens, and so do the 0.7, 0.5 and 0.1 µm
    // squares the area floor alone used to refuse. 7 and 5 nm squares would
    // not, so the rectangle stays as the object's owned-sketch drawing (its
    // four sides and the two splits where it meets the imprint) and the
    // solid is untouched.
    let (mut w, rect) = corner_overlap(1e-3);
    let wall = w.face;
    cross_and_check(&mut w, wall, rect, None);
    let faces = group_faces(w.obj(), &w.plane).len();
    for d in [7e-4, 5e-4, 1e-4] {
        // Kept now, and reopens (`cross_and_check` reopens the result).
        let (mut w, rect) = corner_overlap(d);
        let wall = w.face;
        cross_and_check(&mut w, wall, rect, None);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), faces, "d {d}");
    }
    for d in [7e-6, 5e-6] {
        let (mut w, rect) = corner_overlap(d);
        let wall = w.face;
        let owned = kept_and_check(&mut w, wall, &rect, None);
        assert_eq!(owned, 6, "d {d}");
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 2, "d {d}");
    }
    // The same 0.1 µm corner reached off the lattice: kept now.
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 3.03, 2.0001, 9.0, 4.0);
    let rect = w.rect(12.0299 * MM, 6.0 * MM, 2.0 * MM, 1.0 * MM);
    cross_and_check(&mut w, wall, rect, None);
    // And a 5 nm one: too thin, kept as drawing.
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 3.03, 2.000005, 9.0, 4.0);
    let rect = w.rect(12.029995 * MM, 6.0 * MM, 2.0 * MM, 1.0 * MM);
    let owned = kept_and_check(&mut w, wall, &rect, None);
    assert_eq!(owned, 6);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 2);
}

#[test]
fn no_shrink_leaves_a_face_too_small_to_reopen() {
    // A transform is checked up front: a shrink that would leave a face
    // under the loader's limit (twice its area under 1e-12 m² and narrower
    // than 10 nm) is singular at that scale, refused before any vertex
    // moves. A uniform shrink below 1e-4 is refused by the transform's own
    // determinant floor before any face is looked at, so the face limit is
    // reached by squashing one axis: the box's 10 mm × 10 mm end walls at
    // (5e-4, 3e-7) are 5 µm × 3 nm, too thin to reopen.
    let w = front_wall();
    let mut obj = w.obj().clone();
    assert_eq!(
        obj.apply_transform(&kernel::Transform::uniform_scale(5e-5)),
        Err(kernel::TransformError::Singular)
    );
    assert!(objects_equivalent(&obj, w.obj()), "untouched");
    let squash = |y: f64, z: f64| kernel::Transform::scale(kernel::Vec3::new(1.0, y, z));
    assert_eq!(
        obj.apply_transform(&squash(5e-4, 3e-7)),
        Err(kernel::TransformError::Singular)
    );
    assert!(objects_equivalent(&obj, w.obj()), "untouched");
    // 5 µm × 30 nm end walls (twice the area 3e-13 m², under the old area
    // floor, but 30 nm wide) are kept now, and reopen.
    let mut thin = obj.clone();
    thin.apply_transform(&squash(5e-4, 3e-6))
        .expect("a 30 nm wide wall is kept");
    thin.validate().expect("valid");
    let bytes = thin.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).expect("reopens");
    assert_eq!(back.encode(&|_| 0), bytes, "reopens identically");
    obj.apply_transform(&kernel::Transform::uniform_scale(1e-4))
        .expect("a 1 µm square wall reopens");
    obj.validate().expect("valid");
}

#[test]
fn no_cut_commits_a_face_too_small_to_reopen() {
    // Defence in depth below the planner: a raw cut that would leave a
    // 5 nm corner triangle of the imprint (about 3 nm wide, under
    // `tol::MIN_FACE_WIDTH`) is refused by the always-on validator, on the
    // object and through the document alike.
    let (mut w, s) = wall_with_rect();
    let d = 5e-9;
    let (x1, y1) = (RX + RW, RY + RH);
    let path = vec![w.at(x1 - d, y1), w.at(x1, y1 - d)];
    let mut obj = w.obj().clone();
    assert_eq!(
        obj.split_face(s, &path).map(|_| ()),
        Err(StickyError::WouldCorrupt)
    );
    assert_op_refused(&mut w, split_op(s, path), StickyError::WouldCorrupt);
    // 0.5 µm, which the area floor alone used to refuse, is kept now: it
    // reopens and commits.
    {
        let (mut w, s) = wall_with_rect();
        let d = 0.5e-6;
        let path = vec![w.at(x1 - d, y1), w.at(x1, y1 - d)];
        split_and_check(&mut w, s, path);
    }
    // 2 µm reopens and commits.
    let d = 2e-6;
    let path = vec![w.at(x1 - d, y1), w.at(x1, y1 - d)];
    let s = face_at(&w, RX + MM, RY + MM);
    split_and_check(&mut w, s, path);
}

// ------------------------- contact measured by distance, not by product

#[test]
fn twelve_gon_circles_crossing_land_as_topology() {
    // A 2.4 µm first segment of the piece outside the imprint once read as
    // touching the imprint's long side (an area-scaled tolerance), so a cut
    // the planner accepted was refused PathNotSimple.
    let mut w = front_wall();
    let wall = w.face;
    let disc = w.ring(11.25 * MM, 3.0 * MM, 0.75 * MM, 12, 0.1309);
    draw_region(&mut w, wall, disc, None);
    let ring = w.ring(11.0 * MM, 2.5 * MM, 0.5 * MM, 12, 0.0);
    cross_and_check(&mut w, wall, ring, None);
}

/// Twice the area of `ring` (2-D).
fn twice_area(ring: &[(f64, f64)]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        .abs()
}

/// Shapes a few hundred nanometres to a few tens of microns from touching —
/// the maintainer's by-eye family — drawn over an imprint: a rectangle
/// over a rectangle's corner or side, a polygon circle over a polygon
/// circle, a circle inscribed in a rectangle a hair too short or tall. Every
/// draw lands as face topology with I1–I7 and ground parity, or is kept as
/// the object's owned-sketch drawing with the solid untouched where the
/// ground says it must or may be: a pinch, a region too small to reopen
/// (twice its area under 1e-12 m² and narrower than 10 nm, each with a
/// 20 % band), or features within 3 nm (a sliver the merge tolerance
/// cannot separate). Never refused.
#[test]
fn micron_offset_crossings_match_the_ground_or_stay_drawing_as_predicted() {
    let mut rng = Rng(0x0dd5_eed5_1c0f_fee5);
    let mut unit = move || (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
    let (mut accepted, mut kept_count) = (0usize, 0usize);
    for draw in 0..2100 {
        // An offset of 0.1–30 µm, log-uniform, either sign.
        let off = |u: &mut dyn FnMut() -> f64| {
            let m = 1e-7 * 300f64.powf(u());
            if u() < 0.5 { -m } else { m }
        };
        let mut w = front_wall();
        let wall = w.face;
        let family = draw % 3;
        let (imprint, ring, phase_note): (Vec<Point3>, Vec<Point3>, String) = match family {
            0 => {
                let (x, y) = (3.0 * MM + off(&mut unit), 2.0 * MM + off(&mut unit));
                let (iw, ih) = (9.0 * MM + off(&mut unit), 4.0 * MM + off(&mut unit));
                let imprint = w.rect(x, y, iw, ih);
                let (ox, oy) = (off(&mut unit), off(&mut unit));
                let ring = if unit() < 0.5 {
                    // Over the top-right corner.
                    w.rect(x + iw - ox, y + ih - oy, 2.0 * MM, 1.0 * MM)
                } else {
                    // Over the right side.
                    w.rect(x + iw - ox, y + 1.0 * MM, 2.0 * MM, 1.0 * MM + oy)
                };
                (imprint, ring, format!("rect {ox:e} {oy:e}"))
            }
            1 => {
                let n1 = if unit() < 0.5 { 12 } else { 24 };
                let n2 = if unit() < 0.5 { 12 } else { 24 };
                let (p1, p2) = (unit() * PI, unit() * PI);
                let imprint = w.ring(11.25 * MM, 3.0 * MM, 0.75 * MM, n1, p1);
                let a = unit() * 2.0 * PI;
                let dist = if unit() < 0.7 {
                    1.25 * MM - off(&mut unit)
                } else {
                    (0.3 + 0.9 * unit()) * MM
                };
                let ring = w.ring(
                    11.25 * MM + dist * a.cos(),
                    3.0 * MM + dist * a.sin(),
                    0.5 * MM,
                    n2,
                    p2,
                );
                (
                    imprint,
                    ring,
                    format!("circles {n1}/{n2} {p1} {p2} {a} {dist:e}"),
                )
            }
            _ => {
                let short = off(&mut unit);
                let imprint = w.rect(RX, RY, RW, RH - short);
                let phase = if unit() < 0.5 { 0.0 } else { unit() * PI };
                let ring = w.ring(RX + R, RY + R, R, 24, phase);
                (imprint, ring, format!("inscribed {short:e} {phase}"))
            }
        };
        let imprint_face = draw_region(&mut w, wall, imprint.clone(), None);
        let seed = if unit() < 0.5 { imprint_face } else { wall };
        let what = format!("draw {draw}: {phase_note}");

        // ---- the oracle, from the ground ----
        // Of the loop as the planner reads it: a vertex within POINT_MERGE
        // of an edge is on it (the ground judges by parameter, not
        // distance, and would see a sub-nanometre sliver there).
        let before = w.obj().clone();
        let snapped = snapped_like_the_planner(&before, &w.plane, &ring);
        let sk = ground_with(w.plane, &before, &snapped);
        let pos = |v| sk.vertices()[v].position;
        let gv: Vec<Point3> = sk.vertices().values().map(|v| v.position).collect();
        let on_loop = |p: Point3| ring_dist(p, &snapped) <= tol::POINT_MERGE;
        let sites = gv
            .iter()
            .filter(|&&p| on_loop(p) && ring_dist(p, &imprint) <= tol::POINT_MERGE)
            .count();
        let pinch = sites == 1;
        // Each region's twice-area and width (that over half its perimeter).
        let sizes: Vec<(f64, f64)> = ground_regions(&sk)
            .iter()
            .map(|(outer, _)| {
                let ring: Vec<(f64, f64)> = outer.iter().map(|&p| xy(&w, p)).collect();
                let a2 = twice_area(&ring);
                let perimeter: f64 = (0..ring.len())
                    .map(|i| {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        (b.0 - a.0).hypot(b.1 - a.1)
                    })
                    .sum();
                (a2, a2 / (perimeter / 2.0))
            })
            .collect();
        let too_small = |band: f64| {
            sizes
                .iter()
                .any(|&(a2, wd)| a2 < band * 1e-12 && wd < band * tol::MIN_FACE_WIDTH)
        };
        let tight = sk.edges().values().any(|e| {
            let (a, b) = (pos(e.from), pos(e.to));
            gv.iter()
                .any(|&p| !p.approx_eq(a, 0.0) && !p.approx_eq(b, 0.0) && seg_dist(p, a, b) < 3e-9)
        });
        let must_refuse = pinch || too_small(0.8);
        let may_refuse = must_refuse || too_small(1.2) || tight;

        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let report = w.draw_loop(seed, &ring, None).expect("the loop draws").0;
        let kept = !report.kept_as_sketch.is_empty();
        assert!(
            kept || !must_refuse,
            "landed though a pinch or sliver: {what}"
        );
        assert!(
            !kept || may_refuse,
            "kept though a face was possible: {what}"
        );
        if kept {
            kept_count += 1;
            assert!(objects_equivalent(w.obj(), &before), "{what}");
            assert_eq!(free_edge_count(&w), 0, "{what}");
            assert_undoable(&mut w, &before, &bytes0, depth0);
        } else {
            accepted += 1;
            assert_ground_parity(&w, &before, &snapped);
            assert_invariants(&mut w, &before, &bytes0, depth0);
        }
    }
    assert!(accepted > 1500, "{accepted} accepted, {kept_count} kept");
}

#[test]
fn an_inscribed_circle_over_a_micron_short_rectangle_draws_and_undoes_exactly() {
    // Three draws whose undo once diverged: (short, phase).
    for (short, phase) in [
        (2.3938619798418267e-5, 1.3098219338169863),
        (5.526584940657447e-6, 0.7847253344290198),
        (-1.0428588360979571e-5, 0.5230332028806968),
    ] {
        let mut w = front_wall();
        let wall = w.face;
        let rect = w.rect(RX, RY, RW, RH - short);
        let imprint = draw_region(&mut w, wall, rect, None);
        let ring = w.ring(RX + R, RY + R, R, 24, phase);
        let before = w.obj().clone();
        let snapped = snapped_like_the_planner(&before, &w.plane, &ring);
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let report = w.draw_loop(imprint, &ring, None).expect("draws").0;
        assert!(report.kept_as_sketch.is_empty(), "short {short}");
        assert_ground_parity(&w, &before, &snapped);
        assert_invariants(&mut w, &before, &bytes0, depth0);
    }
}
