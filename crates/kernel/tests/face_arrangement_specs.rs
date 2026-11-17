//! Executable specs for ground-parity drawing on a solid's face (the
//! drafting engine, [`Document::draft`]): a closed loop drawn on a face that
//! TOUCHES the face group's edges at isolated points — a circle tangent to
//! a rectangle imprint's sides, a circle whose rim vertex was snapped onto
//! an edge, a polygon whose corners sit on edges — becomes face topology
//! (boundary-to-boundary cuts) in one undo entry. The result matches the
//! regions the ground gives for the same shapes in one sketch.
//!
//! A loop touching the group at exactly one point is face topology too: the
//! face around it holds it as a hole sharing that one vertex with the
//! face's outer loop (or with another hole), as the ground's region does.
//!
//! Every accepting spec checks the stage invariants:
//! - I1 `validate()` passes and watertightness is unchanged;
//! - I2 `V − E + F − H` over the object is unchanged (a hole touching the
//!   outer loop at a vertex is one boundary with it, not an annulus);
//! - I3 the summed area of the face group is unchanged;
//! - I4 no loop of a group face repeats a vertex (every loop is simple);
//! - I5 undo restores the prior object, redo the result, and saves after
//!   several undo/redo cycles are byte-identical;
//! - I6 the group's faces equal the ground's regions for the same edges;
//! - I7 the saved document reopens and saves back to the same bytes.

use std::f64::consts::{PI, TAU};

use kernel::{
    CurveGeom, DocumentError, FaceId, KernelOp, Object, Point3, SketchError, SurfaceRef, Vec3, tol,
};
use proptest::prelude::*;

use crate::face_arrangement_support;
use face_arrangement_support::*;

// ------------------------------------------------- the inscribed circle

/// The maintainer's scenario on the front wall: the rectangle, then a
/// circle of radius 1.9 mm centered 1.9 mm in from the rectangle's left
/// end, tangent to its left, top, and bottom sides at its own vertices
/// 12, 6, and 18. Disc + three leftovers, as on the ground; every facet
/// keeps its circle.
fn inscribed_on(mut w: Wall, phase: f64, snap_rim_to_left_mid: bool) {
    let s = imprint_rect(&mut w);
    let (cx, cy) = (RX + R, RY + R);
    let mut ring = w.ring(cx, cy, R, 24, phase);
    let mut g = w.circle_geom(cx, cy, R);
    if snap_rim_to_left_mid {
        // The rim click snapped to the left side's midpoint: vertex 0 is
        // that exact point and the radius is measured to it (CircleTool).
        let rect = w.rect(RX, RY, RW, RH);
        let mid = rect[3] + (rect[0] - rect[3]) * 0.5;
        ring[0] = mid;
        g.radius = (mid - g.center).length();
    }
    let before = w.obj().clone();
    w.draw_loop(s, &ring, Some(g))
        .expect("the inscribed circle draws");
    // The drawn region is the disc.
    let region = region_face(&w, &ring).expect("the disc");
    let obj = w.obj();
    let disc = &obj.faces()[region];
    assert_eq!(obj.loop_half_edges(disc.outer_loop).count(), 24);
    assert!(obj.face_contains_point(region, w.at(cx, cy)));
    w.doc.undo().expect("undo");
    assert!(objects_equivalent(w.obj(), &before));
    // Undo re-mints handles: find the rectangle again by a point in it.
    let s = face_at(&w, RX + RW - MM, RY + MM);
    draw_and_check(&mut w, s, ring, Some(g));
    let obj = w.obj();
    // The wall plus disc + three leftovers.
    assert_eq!(group_faces(obj, &w.plane).len(), 5);
    assert_eq!(claims_on(obj, g), 24, "every facet keeps its circle");
}

#[test]
fn inscribed_circle_on_a_wall_rect_makes_four_faces() {
    inscribed_on(front_wall(), 0.0, false);
}

#[test]
fn inscribed_circle_on_a_tilted_face_makes_four_faces() {
    inscribed_on(tilted_wall(), 0.0, false);
}

#[test]
fn inscribed_circle_with_its_rim_snapped_to_the_left_edge_midpoint_makes_four_faces() {
    inscribed_on(front_wall(), PI, true);
}

#[test]
fn inscribed_circle_with_a_snapped_rim_on_a_tilted_face_makes_four_faces() {
    inscribed_on(tilted_wall(), PI, true);
}

#[test]
fn two_point_tangent_makes_three_faces() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    // Centered along the rectangle: tangent to top (6) and bottom (18).
    let (cx, cy) = (RX + RW / 2.0, RY + R);
    let g = w.circle_geom(cx, cy, R);
    let ring = w.ring(cx, cy, R, 24, 0.0);
    draw_and_check(&mut w, s, ring, Some(g));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    assert_eq!(claims_on(w.obj(), g), 24);
}

#[test]
fn polygon_vertices_on_three_edges_make_four_faces() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    // A triangle with one corner on each of the bottom, right, and top
    // sides: three single-segment chords.
    let tri = vec![
        w.at(RX + 2.0 * MM, RY),
        w.at(RX + RW, RY + 1.5 * MM),
        w.at(RX + 4.0 * MM, RY + RH),
    ];
    draw_and_check(&mut w, s, tri, None);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 5);
}

#[test]
fn single_point_tangent_makes_the_disc_a_face_in_a_touching_hole() {
    // A loop touching the group at exactly one point is face topology: the
    // disc is a face, and the rectangle holds it as a hole sharing that one
    // vertex with the rectangle's outer loop, as the ground does.
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    // Radius 1 mm under the top side: tangent at vertex 6 only.
    let r = 1.0 * MM;
    let ring = w.ring(RX + 4.0 * MM, RY + RH - r, r, 24, 0.0);
    let g = w.circle_geom(RX + 4.0 * MM, RY + RH - r, r);
    draw_and_check(&mut w, s, ring, Some(g));
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "wall + rectangle + disc"
    );
    assert!(has_touching_loops(w.obj(), &w.plane));
    assert_eq!(claims_on(w.obj(), g), 24);
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
}

#[test]
fn shared_corner_only_makes_faces_inside_and_outside_a_sub_face() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    // Inside the rectangle, touching its corner: a hole in the rectangle
    // sharing the corner with its outer loop.
    let inside = vec![
        w.at(RX, RY),
        w.at(RX + 2.0 * MM, RY + 1.0 * MM),
        w.at(RX + 1.0 * MM, RY + 2.0 * MM),
    ];
    draw_and_check(&mut w, s, inside, None);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
    // Outside it, touching the same corner: the wall holds the rectangle and
    // the triangle as two holes sharing that corner.
    let outside = vec![
        w.at(RX, RY),
        w.at(RX - 1.0 * MM, RY - 2.0 * MM),
        w.at(RX - 2.0 * MM, RY - 1.0 * MM),
    ];
    let wall = w.face;
    draw_and_check(&mut w, wall, outside, None);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    let obj = w.obj();
    let wall_face = group_faces(obj, &w.plane)
        .into_iter()
        .find(|&f| obj.faces()[f].inner_loops.len() == 2)
        .expect("the wall holds two holes");
    let holes: Vec<Vec<Point3>> = obj.faces()[wall_face]
        .inner_loops
        .iter()
        .map(|&l| ring_of(obj, l))
        .collect();
    let corner = w.at(RX, RY);
    assert!(
        holes
            .iter()
            .all(|h| h.iter().any(|p| p.approx_eq(corner, tol::POINT_MERGE))),
        "both holes hold the shared corner"
    );
}

#[test]
fn crossing_a_chord_sibling_splits_both() {
    let mut w = top_of_box();
    // Split the top in two along u = 15 mm.
    let chord = vec![w.at(15.0 * MM, 0.0), w.at(15.0 * MM, 30.0 * MM)];
    let (report, _) = w
        .doc
        .apply_object_op(
            w.id,
            KernelOp::SplitFace {
                face: w.face,
                path: chord,
                restore: None,
                curves: Vec::new(),
            },
        )
        .expect("chord split");
    let kernel::KernelOpReport::FaceSplit(r) = report else {
        panic!("a split report");
    };
    // A circle centered on the chord, turned 7.5° so the chord crosses two
    // facets mid-way: those facets are cut and their fragments drop the
    // claim (design F11); the other 22 keep it.
    let (cx, cy) = (15.0 * MM, 15.0 * MM);
    let g = w.circle_geom(cx, cy, 4.0 * MM);
    let ring = w.ring(cx, cy, 4.0 * MM, 24, PI / 24.0);
    let seed = r.new_faces[0];
    draw_and_check(&mut w, seed, ring, Some(g));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    assert_eq!(claims_on(w.obj(), g), 22);
}

#[test]
fn pieces_anchored_on_a_hole_imprint_from_stage_3() {
    // Refused in stage 1; from stage 3 the piece outside the rectangle
    // cuts the wall from the rectangle's hole back to it
    // (`face_arrangement_stage3_specs.rs` covers these cases in depth).
    // A circle centered on the rectangle's right side crosses it.
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let ring = w.ring(RX + RW, RY + R, 1.0 * MM, 24, 0.0);
    draw_and_check(&mut w, s, ring, None);
    // Two cuts: the circle's part inside the rectangle and the part outside.
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    // A rectangle flush against the right side from outside.
    let mut w = front_wall();
    imprint_rect(&mut w);
    let wall = w.face;
    let flush = w.rect(RX + RW, RY + 1.0 * MM, 2.0 * MM, 1.0 * MM);
    draw_and_check(&mut w, wall, flush, None);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
}

#[test]
fn a_loop_clear_of_every_edge_still_lands_as_a_sub_face() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let ring = w.ring(RX + 4.0 * MM, RY + R, 1.0 * MM, 24, 0.0);
    draw_and_check(&mut w, s, ring, None);
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "a sub-face in the rectangle"
    );
    // Around the whole rectangle, clicked inside it: lands on the wall and
    // adopts the rectangle.
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let around = w.rect(RX - 1.0 * MM, RY - 1.0 * MM, RW + 2.0 * MM, RH + 2.0 * MM);
    draw_and_check(&mut w, s, around, None);
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        3,
        "wall, band, rectangle"
    );
}

// ----------------------------------------------------- pushing the result

/// The inscribed result, ready for push/pull: `(wall, disc, g)`.
fn inscribed_result(phase: f64) -> (Wall, FaceId, CurveGeom) {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let (cx, cy) = (RX + R, RY + R);
    let g = w.circle_geom(cx, cy, R);
    let ring = w.ring(cx, cy, R, 24, phase);
    let region = draw_region(&mut w, s, ring, Some(g));
    (w, region, g)
}

fn stamped_walls(obj: &Object) -> usize {
    obj.faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .count()
}

#[test]
fn disc_boss_stamps_cylinder_walls_and_a_push_through_stamps_the_tunnel() {
    let (mut w, disc, _) = inscribed_result(0.0);
    let before = w.obj().clone();
    w.doc
        .apply_object_op(
            w.id,
            KernelOp::PushPull {
                face: disc,
                distance: 1.0 * MM,
            },
        )
        .expect("boss the disc");
    w.obj().validate().expect("valid boss");
    assert_eq!(
        stamped_walls(w.obj()),
        24,
        "the boss walls are cylinder facets"
    );
    w.doc.undo().expect("undo the boss");
    assert!(objects_equivalent(w.obj(), &before));

    let disc = w
        .obj()
        .faces()
        .keys()
        .find(|&f| w.obj().face_contains_point(f, w.at(RX + R, RY + R)))
        .expect("disc");
    let through = w
        .obj()
        .push_through(disc, -15.0 * MM)
        .expect("push the disc through");
    through.validate().expect("valid tunnel");
    assert_eq!(
        stamped_walls(&through),
        24,
        "the tunnel walls are cylinder facets"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Rule 3 on the arrangement's faces: pushing or pulling any face of
    /// the group (disc, a leftover, the wall) and undoing is the identity.
    #[test]
    fn push_pull_of_every_result_face_then_undo_is_the_identity(
        snapped in any::<bool>(),
        pick in 0usize..5,
        d in prop::sample::select(vec![-MM, -0.5 * MM, 0.5 * MM, MM]),
    ) {
        let (mut w, _, _) = inscribed_result(if snapped { PI } else { 0.0 });
        let faces = group_faces(w.obj(), &w.plane);
        prop_assert_eq!(faces.len(), 5);
        let face = faces[pick];
        let before = w.obj().clone();
        w.doc
            .apply_object_op(w.id, KernelOp::PushPull { face, distance: d })
            .expect("push/pull a result face");
        prop_assert!(w.obj().validate().is_ok());
        w.doc.undo().expect("undo the push/pull");
        prop_assert!(objects_equivalent(w.obj(), &before));
    }

    /// Random rectangle imprint + random 24-gon on a 0.1 mm lattice, biased
    /// toward tangency: accepted with I1–I7 (inside the rectangle, touching
    /// it at two or more points or none; crossing it, from stage 3; or
    /// wholly around it). A circle touching the rectangle at exactly one
    /// point is a disc in a hole of the rectangle sharing that point, as on
    /// the ground.
    #[test]
    fn touching_circles_on_a_lattice_match_the_ground_or_refuse_as_predicted(
        wq in 20u32..=94,
        hq in 20u32..=60,
        rq in 3u32..=30,
        xmode in 0u8..3,
        ymode in 0u8..3,
        xr in 0u32..1000,
        yr in 0u32..1000,
    ) {
        let q = 0.1 * MM;
        let (rw, rh, r) = (f64::from(wq) * q, f64::from(hq) * q, f64::from(rq) * q);
        let pick = |mode: u8, span_q: u32, rnd: u32| -> Option<u32> {
            let c = match mode {
                0 => rq,
                1 => span_q.checked_sub(rq)?,
                _ => 1 + rnd % span_q.saturating_sub(1).max(1),
            };
            (c > 0 && c < span_q).then_some(c)
        };
        let (Some(cxq), Some(cyq)) = (pick(xmode, wq, xr), pick(ymode, hq, yr)) else {
            return Ok(());
        };
        let mut w = front_wall();
        let rect = w.rect(RX, RY, rw, rh);
        let wall = w.face;
        let s = draw_region(&mut w, wall, rect, None);
        let (cx, cy) = (RX + f64::from(cxq) * q, RY + f64::from(cyq) * q);
        let ring = w.ring(cx, cy, r, 24, 0.0);
        let g = w.circle_geom(cx, cy, r);

        // Prediction from the lattice: the 24-gon's extreme vertices are
        // exactly r from the center along ±u, ±v.
        let gaps = [cxq as i64 - rq as i64, (wq - cxq) as i64 - rq as i64,
                    cyq as i64 - rq as i64, (hq - cyq) as i64 - rq as i64];
        let crosses = gaps.iter().any(|&g| g < 0);
        let touches = gaps.iter().filter(|&&g| g == 0).count();
        // A circle reaching the wall's own edge is out of this property's
        // scope; one wholly around the rectangle encloses it (a sub-face
        // on the wall adopting the rectangle) — decided by each corner's
        // signed distance to the 24-gon's sides, skipping near-ties.
        if cx - r <= 0.0 || cy - r <= 0.0 || cx + r >= 20.0 * MM || cy + r >= 10.0 * MM {
            return Ok(());
        }
        let local: Vec<(f64, f64)> = (0..24)
            .map(|i| {
                let a = TAU * f64::from(i) / 24.0;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        let depth_in = |p: (f64, f64)| {
            (0..24)
                .map(|i| {
                    let (a, b) = (local[i], local[(i + 1) % 24]);
                    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
                    (ex * (p.1 - a.1) - ey * (p.0 - a.0)) / ex.hypot(ey)
                })
                .fold(f64::INFINITY, f64::min)
        };
        let corners = [(RX, RY), (RX + rw, RY), (RX + rw, RY + rh), (RX, RY + rh)];
        if corners.iter().any(|&c| depth_in(c).abs() < 1e-7) {
            return Ok(());
        }
        let encloses = corners.iter().all(|&c| depth_in(c) > 0.0);

        let before = w.obj().clone();
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        if encloses {
            w.draw_loop(s, &ring, Some(g)).expect("predicted enclosing acceptance");
            // Around the rectangle: a sub-face of the wall adopting it.
            prop_assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
            assert_ground_parity(&w, &before, &ring);
            assert_invariants(&mut w, &before, &bytes0, depth0);
        } else if crosses {
            // The pieces outside the rectangle cut the wall from its hole
            // back to it. Facets the rectangle crosses mid-way lose their
            // claim (F11), so no claim count is pinned.
            w.draw_loop(s, &ring, Some(g)).expect("predicted crossing acceptance");
            assert_ground_parity(&w, &before, &ring);
            assert_invariants(&mut w, &before, &bytes0, depth0);
        } else {
            w.draw_loop(s, &ring, Some(g)).expect("predicted acceptance");
            // Clear of the sides: one sub-face (3 faces); touching at one
            // point: the disc in a touching hole (3 faces); at `touches`
            // >= 2 points: the circle plus its leftovers.
            prop_assert_eq!(
                group_faces(w.obj(), &w.plane).len(),
                if touches == 0 { 3 } else { 2 + touches }
            );
            assert_ground_parity(&w, &before, &ring);
            assert_invariants(&mut w, &before, &bytes0, depth0);
            prop_assert_eq!(claims_on(w.obj(), g), 24);
        }
    }
}

// ------------------------------------------------------ degenerate draws

#[test]
fn a_loop_that_is_existing_edges_changes_nothing() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let boundary = w.rect(RX, RY, RW, RH);
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let (report, _) = w.draw_loop(s, &boundary, None).expect("draws");
    assert!(report.objects.is_empty() && report.sketches.is_empty());
    assert_eq!(w.doc.save(), bytes0, "an unchanged draft changes nothing");
    assert_eq!(w.doc.undo_depth(), depth0, "and records no undo entry");
}

#[test]
fn an_off_plane_loop_refuses_typed_and_leaves_the_document_untouched() {
    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let clear = w.ring(RX + 4.0 * MM, RY + R, 1.0 * MM, 24, 0.0);
    let off_plane: Vec<Point3> = clear.iter().map(|&p| p + w.plane.normal() * 1e-6).collect();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let err = w.draw_loop(s, &off_plane, None).expect_err("off the plane");
    assert!(
        matches!(
            err,
            DocumentError::Sketch(SketchError::PointOffPlane { .. })
        ),
        "got {err:?}"
    );
    assert_eq!(w.doc.save(), bytes0, "saved bytes untouched");
    assert_eq!(w.doc.undo_depth(), depth0, "no undo entry");
    assert!(w.doc.draft_sketch().is_none(), "no draft left open");
}

// ------------------------------------------- the region a draw reports

/// A 30 × 20 mm wall: the y = 0 side of a 30 × 10 × 20 mm box (u = +x,
/// v = +z).
fn wall_30x20() -> Wall {
    wall_of_box(ground(), 30.0 * MM, 10.0 * MM, 20.0 * MM, |_| {
        Vec3::new(0.0, -1.0, 0.0)
    })
}

/// A plain 30 × 20 mm top: the +z cap of a 30 × 20 × 10 mm box. Its frame
/// is the world's (`at(x, y)` is `(x, y, 10 mm)` exactly).
fn top_30x20() -> Wall {
    wall_of_box(ground(), 30.0 * MM, 20.0 * MM, 10.0 * MM, |_| {
        Vec3::new(0.0, 0.0, 1.0)
    })
}

/// `(outer area, net area)` of `f`, holes subtracted from the net.
fn face_areas(obj: &Object, f: FaceId, n: Vec3) -> (f64, f64) {
    let face = &obj.faces()[f];
    let area = |l| signed_area(&obj.loop_positions(l).collect::<Vec<_>>(), n);
    let outer = area(face.outer_loop);
    let net = outer + face.inner_loops.iter().map(|&l| area(l)).sum::<f64>();
    (outer, net)
}

/// The reported `region` is the drawn region — `outer` sides, `holes`
/// holes, the given outer and net areas (mm²) — and pushing it 1 mm out
/// raises exactly that region: the faces 1 mm off the plane carry its net
/// area. Undone afterwards.
fn assert_region_is_drawn(
    w: &mut Wall,
    region: FaceId,
    outer: usize,
    holes: usize,
    outer_mm2: f64,
    net_mm2: f64,
) {
    let n = w.plane.normal();
    let obj = w.obj();
    let face = &obj.faces()[region];
    assert_eq!(
        obj.loop_half_edges(face.outer_loop).count(),
        outer,
        "outer sides"
    );
    assert_eq!(face.inner_loops.len(), holes, "holes");
    let (a_out, a_net) = face_areas(obj, region, n);
    let mm2 = MM * MM;
    assert!(
        (a_out - outer_mm2 * mm2).abs() < 1e-9 * mm2,
        "outer area {a_out}"
    );
    assert!(
        (a_net - net_mm2 * mm2).abs() < 1e-9 * mm2,
        "net area {a_net}"
    );
    let before = w.obj().clone();
    w.doc
        .apply_object_op(
            w.id,
            KernelOp::PushPull {
                face: region,
                distance: MM,
            },
        )
        .expect("push the region out");
    let obj = w.obj();
    obj.validate().expect("valid after the push");
    let raised: f64 = obj
        .faces()
        .iter()
        .filter(|(_, f)| {
            (f.plane.normal() - n).length() < 1e-9
                && obj
                    .loop_positions(f.outer_loop)
                    .all(|p| (w.plane.signed_distance(p) - MM).abs() < 1e-9)
        })
        .map(|(f, _)| face_areas(obj, f, n).1)
        .sum();
    assert!(
        (raised - net_mm2 * mm2).abs() < 1e-9 * mm2,
        "the push raised the drawn region: {raised} vs {net_mm2} mm²"
    );
    w.doc.undo().expect("undo the push");
    assert!(objects_equivalent(w.obj(), &before));
}

/// Draws `loop_path` on a fresh `fixture()` and checks the reported
/// region (before any undo re-mints handles), then draws it on another
/// fresh fixture and checks I1–I7.
fn region_case(
    fixture: impl Fn() -> (Wall, FaceId, Vec<Point3>, Option<CurveGeom>),
    outer: usize,
    holes: usize,
    outer_mm2: f64,
    net_mm2: f64,
) {
    let (mut w, seed, loop_path, curve) = fixture();
    let region = draw_region(&mut w, seed, loop_path, curve);
    assert_region_is_drawn(&mut w, region, outer, holes, outer_mm2, net_mm2);
    let (mut w, seed, loop_path, curve) = fixture();
    draw_and_check(&mut w, seed, loop_path, curve);
}

/// A wall or top with a rectangle imprinted at `(x, y)`, `w × h` mm.
fn with_imprint(mut w: Wall, x: f64, y: f64, rw: f64, rh: f64) -> (Wall, FaceId) {
    let rect = w.rect(x * MM, y * MM, rw * MM, rh * MM);
    let wall = w.face;
    let region = draw_region(&mut w, wall, rect, None);
    (w, region)
}

#[test]
fn an_edge_hugging_loop_around_an_imprint_reports_the_drawn_region() {
    // Flush against the wall's bottom and both sides, enclosing the
    // imprint, drawn from the wall: the region is the band with the
    // imprint as its hole.
    region_case(
        || {
            let (w, _) = with_imprint(wall_30x20(), 12.0, 8.0, 6.0, 4.0);
            let band = w.rect(0.0, 0.0, 30.0 * MM, 16.0 * MM);
            let wall = w.face;
            (w, wall, band, None)
        },
        4,
        1,
        480.0,
        456.0,
    );
}

#[test]
fn a_flush_rect_clicked_on_the_imprint_it_encloses_reports_the_drawn_region() {
    // Flush with a 100 mm top's left edge, enclosing a 40 mm square
    // imprint, started on the square.
    region_case(
        || {
            let top = wall_of_box(ground(), 100.0 * MM, 100.0 * MM, 10.0 * MM, |_| {
                Vec3::new(0.0, 0.0, 1.0)
            });
            let (w, square) = with_imprint(top, 20.0, 20.0, 40.0, 40.0);
            let drawn = w.rect(0.0, 10.0 * MM, 70.0 * MM, 60.0 * MM);
            (w, square, drawn, None)
        },
        4,
        1,
        4200.0,
        2600.0,
    );
}

#[test]
fn a_circle_tangent_twice_around_an_imprint_reports_the_drawn_region() {
    // R = 10 mm about the wall's center: tangent to its top and bottom at
    // vertices 6 and 18, the imprint inside, drawn from the wall.
    let r = 10.0 * MM;
    let gon = 12.0 * 100.0 * (TAU / 24.0).sin();
    region_case(
        || {
            let (w, _) = with_imprint(wall_30x20(), 12.0, 8.0, 6.0, 4.0);
            let g = w.circle_geom(15.0 * MM, 10.0 * MM, r);
            let ring = w.ring(15.0 * MM, 10.0 * MM, r, 24, 0.0);
            let wall = w.face;
            (w, wall, ring, Some(g))
        },
        24,
        1,
        gon,
        gon - 24.0,
    );
}

// -------------------------------- contacts closer than POINT_MERGE apart

/// Draws a loop that meets the face at one point and pokes a hair past it:
/// the part over the face is a face in a hole touching the face's edge at
/// that point, the hair past the edge is a loose free-sketch edge, and the
/// draw passes I1–I5 and I7.
fn draw_touching_with_a_hair_past(w: &mut Wall, loop_path: Vec<Point3>) {
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let face = w.face;
    let (report, _) = w.draw_loop(face, &loop_path, None).expect("never refused");
    assert!(report.kept_as_sketch.is_empty(), "nothing kept as drawing");
    assert_eq!(
        group_faces(w.obj(), &w.plane).len(),
        2,
        "the face + the loop"
    );
    assert!(has_touching_loops(w.obj(), &w.plane), "a touching hole");
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
    assert_eq!(report.sketches.len(), 1, "the free hair past the edge");
    assert!(min_vertex_gap(w.obj()) >= tol::POINT_MERGE);
    assert_invariants(w, &before, &bytes0, depth0);
}

#[test]
fn a_loop_whose_two_contacts_resolve_to_one_corner_touches_it_as_a_face() {
    // The apex sits 1.1 nm past the top-right corner along both axes
    // (1.56 nm off it): both sides pass within POINT_MERGE of the corner,
    // so the loop meets the face at that one corner only, and its apex
    // lies off the face. The part over the face is a face in a hole
    // touching the corner; the overhang is a free sketch.
    let mut w = top_30x20();
    let tri = vec![
        w.at(0.015, 0.016),
        w.at(0.030_000_001_1, 0.020_000_001_1),
        w.at(0.006_000_000_3, 0.012),
    ];
    draw_touching_with_a_hair_past(&mut w, tri);
}

#[test]
fn a_needle_whose_apex_grazes_an_edge_touches_it_as_a_face() {
    // The apex is 1.1 nm above the top edge, so both sides cross it, but
    // only 22 pm apart: two vertices closer than POINT_MERGE if cut. They
    // meet the edge as one point: the needle over the face is a face in a
    // hole touching the edge there, the sliver past it a free sketch.
    let mut w = top_30x20();
    let needle = vec![
        w.at(0.0149, 0.010),
        w.at(0.0151, 0.010),
        w.at(0.015, 0.020_000_001_1),
    ];
    draw_touching_with_a_hair_past(&mut w, needle);
}

#[test]
fn loop_vertices_a_hair_off_an_edge_split_it_straight_and_undo_exactly() {
    // A vertex within POINT_MERGE of an edge but not on it is a contact;
    // the edge is split ON its carrier, so the face's area is conserved
    // and undo's merge restores the edge bit for bit (I3, I5). I6 is
    // checked against the ground of the loop as the planner snaps it
    // (see `snapped_like_the_planner`).
    for d in [1e-13, 1e-12, 1e-11, 1e-10, 9e-10] {
        for d in [d, -d] {
            let mut w = top_30x20();
            let tri = vec![
                w.at(10.0 * MM, d),
                w.at(30.0 * MM + d, 8.0 * MM),
                w.at(12.0 * MM, 12.0 * MM),
            ];
            let face = w.face;
            let before = w.obj().clone();
            let bytes0 = w.doc.save();
            let depth0 = w.doc.undo_depth();
            w.draw_loop(face, &tri, None)
                .expect("the touching triangle draws");
            // Two chords (the old planner's `Chords(2)`): the triangle, the
            // corner its long side cuts off, and the rest.
            assert_eq!(
                group_faces(w.obj(), &w.plane).len(),
                3,
                "offset {d}: the triangle, the cut-off corner, and the rest"
            );
            let snapped = snapped_like_the_planner(&before, &w.plane, &tri);
            assert_ground_parity(&w, &before, &snapped);
            assert_invariants(&mut w, &before, &bytes0, depth0);

            // A circle tangent to the top and bottom edges, shifted `d`
            // across them: both tangent vertices move onto their edges
            // and every facet keeps its circle.
            let mut w = top_30x20();
            let (cx, cy, r) = (15.0 * MM, 10.0 * MM + d, 10.0 * MM);
            let g = w.circle_geom(cx, cy, r);
            let ring = w.ring(cx, cy, r, 24, 0.0);
            let face = w.face;
            let before = w.obj().clone();
            let bytes0 = w.doc.save();
            let depth0 = w.doc.undo_depth();
            w.draw_loop(face, &ring, Some(g))
                .expect("the shifted tangent circle draws");
            assert_eq!(
                group_faces(w.obj(), &w.plane).len(),
                3,
                "offset {d}: the disc and two leftovers"
            );
            assert_eq!(claims_on(w.obj(), g), 24, "offset {d}");
            let snapped = snapped_like_the_planner(&before, &w.plane, &ring);
            assert_ground_parity(&w, &before, &snapped);
            assert_invariants(&mut w, &before, &bytes0, depth0);
        }
    }
}

/// The smallest distance between two distinct vertices of `obj`.
fn min_vertex_gap(obj: &Object) -> f64 {
    let ps: Vec<Point3> = obj.vertices().values().map(|v| v.position).collect();
    let mut best = f64::INFINITY;
    for i in 0..ps.len() {
        for j in i + 1..ps.len() {
            best = best.min((ps[i] - ps[j]).length());
        }
    }
    best
}

/// A deterministic xorshift64* stream.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Rule 4 at the tolerance boundary: loops whose vertices sit on the
/// face's corners and edges, each such vertex perturbed by up to ±`amp`
/// per axis (1.1 nm straddles POINT_MERGE), never panic. Each draw is
/// refused with the object, saves, and undo depth untouched, or accepted
/// with I1–I7 holding and no two vertices of the result closer than
/// POINT_MERGE; a loop wholly on the face equals the ground's regions.
#[test]
fn corner_and_edge_touching_loops_never_panic() {
    merge_tolerance_fuzz(0.0);
}

#[test]
fn corner_and_edge_touching_loops_jittered_at_the_merge_tolerance_never_panic() {
    merge_tolerance_fuzz(1.1e-9);
}

fn merge_tolerance_fuzz(amp: f64) {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut accepted, mut refused) = (0usize, 0usize);
    let (mut unchanged, mut on_topology, mut with_sketch) = (0usize, 0usize, 0usize);
    let jitter = |rng: &mut Rng, p: (f64, f64)| {
        let mut d = || match rng.below(6) {
            0 => -amp,
            1 => amp,
            2 => 0.0,
            _ => (rng.unit() * 2.0 - 1.0) * amp,
        };
        let dx = d();
        (p.0 + dx, p.1 + d())
    };
    for draw in 0..2400 {
        // Even draws target the plain top's own corners and edges; odd
        // draws target a 10 × 10 mm imprint's, drawn on the imprint.
        let (mut w, seed, (x0, y0, x1, y1)) = if draw % 2 == 0 {
            let w = top_30x20();
            let face = w.face;
            (w, face, (0.0, 0.0, 30.0 * MM, 20.0 * MM))
        } else {
            let (w, sq) = with_imprint(top_30x20(), 10.0, 5.0, 10.0, 10.0);
            (w, sq, (10.0 * MM, 5.0 * MM, 20.0 * MM, 15.0 * MM))
        };
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let on_edge = |k: usize, t: f64| {
            let (a, b) = (corners[k % 4], corners[(k + 1) % 4]);
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let k = rng.below(4);
        let t = 0.15 + 0.7 * rng.unit();
        let t2 = 0.15 + 0.7 * rng.unit();
        let k2 = k + 1 + rng.below(3);
        let inner = |p: (f64, f64), f: f64| {
            // `p` pulled toward the target's center by the fraction `f`.
            (p.0 + (cx - p.0) * f, p.1 + (cy - p.1) * f)
        };
        let pts: Vec<(f64, f64)> = match rng.below(6) {
            // Apex at a corner: one touch.
            0 => {
                let c = jitter(&mut rng, corners[k]);
                let (a, b) = (on_edge(k, 0.5), on_edge(k + 3, 0.5));
                vec![c, inner(a, 0.4), inner(b, 0.4)]
            }
            // Two vertices on two different edges, one inside.
            1 => vec![
                jitter(&mut rng, on_edge(k, t)),
                jitter(&mut rng, on_edge(k2, t2)),
                inner(on_edge(k + 2, 0.5), 0.6),
            ],
            // A needle grazing an edge.
            2 => {
                let apex = on_edge(k, t);
                let base = inner(apex, 0.5);
                let (ex, ey) = (
                    corners[(k + 1) % 4].0 - corners[k].0,
                    corners[(k + 1) % 4].1 - corners[k].1,
                );
                let len = ex.hypot(ey);
                let (hx, hy) = (ex / len * 0.1 * MM, ey / len * 0.1 * MM);
                vec![
                    (base.0 - hx, base.1 - hy),
                    (base.0 + hx, base.1 + hy),
                    jitter(&mut rng, apex),
                ]
            }
            // A diamond with a corner on every edge.
            3 => (0..4)
                .map(|j| jitter(&mut rng, on_edge(k + j, 0.5)))
                .collect(),
            // A corner plus an edge point.
            4 => vec![
                jitter(&mut rng, corners[k]),
                jitter(&mut rng, on_edge(k + 1, t)),
                inner(on_edge(k + 2, 0.5), 0.5),
            ],
            // Two points on one edge with an apex inside.
            _ => {
                let (lo, hi) = (t.min(t2), t.max(t2) + 0.01);
                vec![
                    jitter(&mut rng, on_edge(k, lo)),
                    jitter(&mut rng, on_edge(k, hi.min(0.95))),
                    inner(on_edge(k, 0.5), 0.5),
                ]
            }
        };
        let loop_path: Vec<Point3> = pts.iter().map(|&(x, y)| w.at(x, y)).collect();
        let face = seed;
        let before = w.obj().clone();
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            w.draw_loop(face, &loop_path, None)
                .map(|(report, _)| report)
        }));
        let Ok(result) = result else {
            panic!("draw {draw} panicked on {pts:?}");
        };
        match result {
            Ok(report) => {
                accepted += 1;
                let sketch_edges: usize = w
                    .doc
                    .sketch_ids()
                    .iter()
                    .map(|&s| w.doc.sketch(s).map_or(0, |sk| sk.edges().len()))
                    .sum();
                assert!(
                    min_vertex_gap(w.obj()) >= tol::POINT_MERGE,
                    "draw {draw} left vertices closer than POINT_MERGE: {pts:?}"
                );
                if w.doc.undo_depth() == depth0 {
                    // Nothing was drawn (the loop was already there).
                    unchanged += 1;
                    assert_eq!(w.doc.save(), bytes0, "draw {draw}: an unchanged draft");
                    continue;
                }
                if sketch_edges == 0 {
                    // Wholly on the face: the group equals the ground's
                    // regions for the loop as the engine snaps it.
                    on_topology += 1;
                    // A loop point the engine kept as a vertex of its own
                    // (within a hair of an edge's end but not merged with it)
                    // is where the engine put it.
                    let snapped: Vec<Point3> =
                        snapped_like_the_planner(&before, &w.plane, &loop_path)
                            .into_iter()
                            .zip(&loop_path)
                            .map(|(q, &p)| {
                                w.obj()
                                    .vertices()
                                    .values()
                                    .map(|v| v.position)
                                    .find(|v| {
                                        v.approx_eq(p, 2.0 * tol::POINT_MERGE)
                                            && !before.vertices().values().any(|b| b.position == *v)
                                    })
                                    .unwrap_or(q)
                            })
                            .collect();
                    assert_ground_parity(&w, &before, &snapped);
                } else {
                    // Part of the drawing lies off the face (a hair past an
                    // edge): it stays sketch drawing; the faces hold the
                    // rest.
                    with_sketch += 1;
                    let _ = &report;
                }
                assert_invariants(&mut w, &before, &bytes0, depth0);
            }
            Err(e) => {
                refused += 1;
                let _ = sticky(e);
                assert!(
                    objects_equivalent(w.obj(), &before),
                    "draw {draw} touched the object"
                );
                assert_eq!(w.doc.save(), bytes0, "draw {draw} touched the saved bytes");
                assert_eq!(w.doc.undo_depth(), depth0, "draw {draw} left an undo entry");
            }
        }
    }
    eprintln!(
        "{accepted} accepted ({on_topology} on topology, {with_sketch} with sketch drawing, \
         {unchanged} unchanged), {refused} refused"
    );
    // Exactly on the edges and corners, every loop lands as face topology
    // (a one-point touch included); jittered, the loops a hair past an edge
    // leave that hair as sketch drawing.
    assert!(
        on_topology > 100
            && if amp == 0.0 {
                with_sketch == 0
            } else {
                with_sketch > 100
            },
        "{accepted} accepted ({on_topology} on topology, {with_sketch} with sketch drawing), {refused} refused"
    );
}

// ------------------------------------- a loop around nested imprints

/// Imprints a `rw × rh` mm rectangle at `(x, y)` mm on `seed`.
fn imprint_on(w: &mut Wall, seed: FaceId, x: f64, y: f64, rw: f64, rh: f64) -> FaceId {
    let rect = w.rect(x * MM, y * MM, rw * MM, rh * MM);
    draw_region(w, seed, rect, None)
}

/// The 30 × 20 mm top with imprint A (14 × 14 mm at (8, 3)) holding two
/// side-by-side imprints B1 and B2 (4 × 10 mm at (10, 5) and (16, 5)).
fn nested_layout() -> (Wall, [FaceId; 3]) {
    let (mut w, a) = with_imprint(top_30x20(), 8.0, 3.0, 14.0, 14.0);
    let b1 = imprint_on(&mut w, a, 10.0, 5.0, 4.0, 10.0);
    let b2 = imprint_on(&mut w, a, 16.0, 5.0, 4.0, 10.0);
    (w, [a, b1, b2])
}

/// The area (mm²) a loop of `(x, y)` mm points encloses.
fn loop_area_mm2(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

#[test]
fn a_loop_around_an_imprint_holding_two_imprints_reports_the_drawn_region() {
    // Touching the top's left and right edges, enclosing A (and so B1 and
    // B2): the region is the hexagon with A as its one hole.
    let pts = [
        (0.0, 4.312),
        (3.607, 1.0),
        (23.186, 1.0),
        (30.0, 1.451),
        (26.866, 19.0),
        (6.731, 19.0),
    ];
    let hexagon = loop_area_mm2(&pts);
    region_case(
        || {
            let (w, _) = nested_layout();
            let ring: Vec<Point3> = pts.iter().map(|&(x, y)| w.at(x * MM, y * MM)).collect();
            let face = w.face;
            (w, face, ring, None)
        },
        6,
        1,
        hexagon,
        hexagon - 196.0,
    );
}

#[test]
fn a_flush_rect_around_three_deep_imprints_reports_the_drawn_region() {
    // A ⊃ B ⊃ C, and a rectangle flush with the top's bottom and sides
    // enclosing A: the region is the band with A as its one hole.
    region_case(
        || {
            let (mut w, a) = with_imprint(top_30x20(), 8.0, 3.0, 14.0, 12.0);
            let b = imprint_on(&mut w, a, 10.0, 5.0, 10.0, 8.0);
            imprint_on(&mut w, b, 12.0, 7.0, 6.0, 4.0);
            let band = w.rect(0.0, 0.0, 30.0 * MM, 17.0 * MM);
            let face = w.face;
            (w, face, band, None)
        },
        4,
        1,
        510.0,
        510.0 - 168.0,
    );
}

/// Random loops around the A + B1 + B2 layout, touching the top's edges
/// at two or three points (or none): whatever the scanline happens to
/// cross, the reported region is never one of the imprints and its net
/// area is the loop's minus A's.
#[test]
fn random_loops_around_nested_imprints_report_the_drawn_region() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (mut accepted, mut kept) = (0usize, 0usize);
    for draw in 0..1000 {
        let (mut w, imprints) = nested_layout();
        let mut r = |lo: f64, hi: f64| lo + (hi - lo) * rng.unit();
        let left = if r(0.0, 1.0) < 0.85 { 0.0 } else { r(0.3, 1.0) };
        let right = if r(0.0, 1.0) < 0.85 {
            30.0
        } else {
            r(29.0, 29.7)
        };
        let mut pts = vec![(left, r(2.0, 18.0)), (r(1.0, 7.0), r(0.5, 2.5))];
        for _ in 0..(r(0.0, 3.0) as usize) {
            pts.push((r(8.0, 22.0), r(0.5, 2.5)));
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.push((r(23.0, 29.0), r(0.5, 2.5)));
        pts.push((right, r(2.0, 18.0)));
        pts.push((r(23.0, 29.0), r(17.5, 19.5)));
        if r(0.0, 1.0) < 0.3 {
            pts.push((r(9.0, 21.0), 20.0));
        }
        pts.push((r(1.0, 7.0), r(17.5, 19.5)));
        let ring: Vec<Point3> = pts.iter().map(|&(x, y)| w.at(x * MM, y * MM)).collect();
        let face = w.face;
        let Ok((report, _)) = w.draw_loop(face, &ring, None) else {
            continue;
        };
        if !report.kept_as_sketch.is_empty() {
            // The loop touches the face at one point (a pinch) or overhangs
            // it: kept as drawing, which the old path refused.
            kept += 1;
            continue;
        }
        let Some(region) = region_face(&w, &ring) else {
            panic!("draw {draw}: no face carries the drawn region for {pts:?}");
        };
        accepted += 1;
        assert!(
            !imprints.contains(&region),
            "draw {draw}: the region is a pre-draw imprint for {pts:?}"
        );
        let (_, net) = face_areas(w.obj(), region, w.plane.normal());
        let want = (loop_area_mm2(&pts) - 196.0) * MM * MM;
        assert!(
            (net - want).abs() < 1e-9 * want,
            "draw {draw}: region net area {net} vs {want} for {pts:?}"
        );
    }
    assert!(
        accepted >= 800,
        "{accepted} accepted, {kept} kept as drawing"
    );
}
