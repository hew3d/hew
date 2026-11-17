//! Executable specs for ground-parity drawing on a solid's face, stage 2:
//! deleting a line on a face merges the regions on its two sides even when
//! one of them is the face a shape sits in. The mirrored pair behind it:
//!
//! - [`Object::split_face`] accepts a cut whose two ends both lie on the
//!   same HOLE loop of the face (a chord drawn around the outside of an
//!   imprint, from one of its sides to another): the face keeps its handle
//!   and its outer loop, the region between the cut and the hole becomes a
//!   new face (the lune), and the hole is rerouted along the cut;
//! - [`Object::merge_faces`] accepts a run of edges between one face's
//!   OUTER loop and a hole loop of the face around it: the inner face
//!   dissolves into the face holding the hole, which keeps its handle and
//!   attributes, and the hole is rerouted around what was dissolved.
//!
//! Each is the other's recorded inverse, so undo and redo are exact. The
//! maintainer's workflow — a rectangle imprint, a circle inscribed at one
//! end, then Delete on each corner leftover's rectangle edges — rounds the
//! rectangle's corners exactly as deleting the same lines in a ground
//! sketch does, and the rounded shape pushes and pulls with smooth walls.
//!
//! Every accepting spec checks the stage invariants (see
//! `face_arrangement_specs.rs`): I1 valid and watertight, I2 Euler–Poincaré
//! unchanged, I3 group area conserved, I4 no pinch, I5 exact undo/redo with
//! byte-identical saves, I6 the face group equals the ground's regions for
//! the same lines (drawn, or deleted), I7 the saved document reopens.

use kernel::topo::LoopKind;
use kernel::{
    BooleanOp, EdgeId, FaceId, KernelOp, KernelOpReport, Material, Object, Point3, Rgba8,
    StickyError, SurfaceRef, Transform, tol,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::*;

// --------------------------------------------------------------- helpers

/// A 30 × 30 × 10 mm box's top carrying a 10 × 10 mm square imprint at
/// (10, 10) mm: `(wall, square)`. The wall's frame is the world's.
fn top_with_square() -> (Wall, FaceId) {
    let mut w = top_of_box();
    let seed = w.face;
    let s = imprint_mm(&mut w, seed, 10.0, 10.0, 10.0, 10.0);
    (w, s)
}

// ------------------------------------------------ the hole-anchored split

#[test]
fn split_face_hole_chord_makes_a_lune() {
    let (mut w, s) = top_with_square();
    let p = w.face;
    let n = w.plane.normal();
    let path = vec![
        mm(&w, 12.0, 20.0),
        mm(&w, 12.0, 24.0),
        mm(&w, 18.0, 24.0),
        mm(&w, 18.0, 20.0),
    ];
    // The report, before any undo re-mints handles.
    let mut o = w.obj().clone();
    let r = o
        .split_face(p, &path)
        .expect("a hole chord splits the face");
    o.validate().expect("valid");
    assert_eq!(r.new_faces[0], p, "the face keeps its handle");
    assert_eq!(r.new_edges.len(), 3, "one edge per cut segment");
    assert_eq!(
        r.split_boundary_edges.len(),
        2,
        "both ends split a hole edge"
    );
    let lune = r.new_faces[1];
    assert_eq!(o.loops()[o.faces()[lune].outer_loop].kind, LoopKind::Outer);
    assert!(o.faces()[lune].inner_loops.is_empty());
    assert!(cyclic_match(
        &ring_of(&o, o.faces()[lune].outer_loop),
        &[
            mm(&w, 12.0, 20.0),
            mm(&w, 18.0, 20.0),
            mm(&w, 18.0, 24.0),
            mm(&w, 12.0, 24.0),
        ],
        false
    ));
    assert!((outer_area(&o, lune, n) - 24.0 * mm2()).abs() < 1e-9 * mm2());
    // The hole is rerouted along the cut; the face's outer loop is as it was.
    assert_eq!(o.faces()[p].inner_loops.len(), 1);
    assert!(cyclic_match(
        &ring_of(&o, o.faces()[p].inner_loops[0]),
        &[
            mm(&w, 10.0, 10.0),
            mm(&w, 10.0, 20.0),
            mm(&w, 12.0, 20.0),
            mm(&w, 12.0, 24.0),
            mm(&w, 18.0, 24.0),
            mm(&w, 18.0, 20.0),
            mm(&w, 20.0, 20.0),
            mm(&w, 20.0, 10.0),
        ],
        false
    ));
    assert_eq!(
        ring_of(&o, o.faces()[p].outer_loop),
        ring_of(w.obj(), w.obj().faces()[p].outer_loop)
    );
    // The square picked up both cut ends and now borders the lune.
    assert_eq!(o.loop_half_edges(o.faces()[s].outer_loop).count(), 6);
    // Through the document: I1–I7.
    split_and_check(&mut w, p, path);
}

#[test]
fn a_hole_chord_between_two_hole_corners_makes_a_lune() {
    let (mut w, _) = top_with_square();
    let p = w.face;
    let path = vec![
        mm(&w, 10.0, 20.0),
        mm(&w, 10.0, 25.0),
        mm(&w, 20.0, 25.0),
        mm(&w, 20.0, 20.0),
    ];
    let r = split_and_check(&mut w, p, path);
    assert_eq!(
        r.split_boundary_edges.len(),
        0,
        "both ends are hole corners"
    );
    let obj = w.obj();
    let lune = face_at(&w, 15.0 * MM, 22.0 * MM);
    assert_ne!(lune, p);
    assert_eq!(obj.loop_half_edges(obj.faces()[lune].outer_loop).count(), 4);
    assert_eq!(group_faces(obj, &w.plane).len(), 3);
}

#[test]
fn a_hole_chord_hands_the_holes_it_encloses_to_the_lune() {
    let (mut w, _) = top_with_square();
    let p = w.face;
    // T sits where the lune will be; U stays clear of it.
    imprint_mm(&mut w, p, 13.0, 21.5, 4.0, 1.0);
    imprint_mm(&mut w, p, 2.0, 2.0, 3.0, 3.0);
    let path = vec![
        mm(&w, 12.0, 20.0),
        mm(&w, 12.0, 24.0),
        mm(&w, 18.0, 24.0),
        mm(&w, 18.0, 20.0),
    ];
    split_and_check(&mut w, p, path);
    let obj = w.obj();
    let lune = face_at(&w, 12.5 * MM, 23.5 * MM);
    assert_eq!(
        obj.faces()[lune].inner_loops.len(),
        1,
        "T moved to the lune"
    );
    let t_ring = ring_of(obj, obj.faces()[lune].inner_loops[0]);
    assert!(t_ring.iter().all(|q| q.approx_eq(mm(&w, 13.0, 21.5), 1e-9)
        || q.approx_eq(mm(&w, 17.0, 21.5), 1e-9)
        || q.approx_eq(mm(&w, 17.0, 22.5), 1e-9)
        || q.approx_eq(mm(&w, 13.0, 22.5), 1e-9)));
    assert_eq!(obj.faces()[p].inner_loops.len(), 2, "the square's and U's");
}

#[test]
fn hole_chords_that_cannot_split_refuse_untouched() {
    let (mut w, _) = top_with_square();
    let p = w.face;
    imprint_mm(&mut w, p, 13.0, 21.5, 4.0, 1.0);
    imprint_mm(&mut w, p, 2.0, 2.0, 3.0, 3.0);
    let (o, u, v) = (w.origin, w.u, w.v);
    let at = move |x: f64, y: f64| o + u * (x * MM) + v * (y * MM);
    // Ends on different loops refuse exactly as before: the first end not
    // on the outer loop.
    assert_op_refused(
        &mut w,
        split_op(p, vec![at(12.0, 20.0), at(12.0, 30.0)]),
        StickyError::EndpointNotOnBoundary { which: 0 },
    );
    assert_op_refused(
        &mut w,
        split_op(p, vec![at(12.0, 30.0), at(12.0, 20.0)]),
        StickyError::EndpointNotOnBoundary { which: 1 },
    );
    assert_op_refused(
        &mut w,
        split_op(p, vec![at(10.0, 12.0), at(5.0, 3.0)]),
        StickyError::EndpointNotOnBoundary { which: 0 },
    );
    // A cut through another hole (T) cannot split the face in two.
    assert_op_refused(
        &mut w,
        split_op(
            p,
            vec![
                at(12.0, 20.0),
                at(12.0, 22.0),
                at(18.0, 22.0),
                at(18.0, 20.0),
            ],
        ),
        StickyError::PathNotSimple,
    );
    // Through the hole itself, or along its edge, is not on the face.
    assert_op_refused(
        &mut w,
        split_op(p, vec![at(12.0, 20.0), at(15.0, 15.0), at(18.0, 20.0)]),
        StickyError::PointNotOnFace { index: 1 },
    );
    assert_op_refused(
        &mut w,
        split_op(p, vec![at(12.0, 20.0), at(18.0, 20.0)]),
        StickyError::PathNotSimple,
    );
    // Touching the hole partway along.
    assert_op_refused(
        &mut w,
        split_op(
            p,
            vec![
                at(12.0, 20.0),
                at(14.0, 24.0),
                at(15.0, 20.0),
                at(16.0, 24.0),
                at(18.0, 20.0),
            ],
        ),
        StickyError::PathNotSimple,
    );
}

/// A cut around the outside of the square from side `i` at `ta` to side
/// `j` at `tb` (`t = 0` is the side's first corner), `d` mm out, walking
/// counter-clockwise (or clockwise) around the square.
fn around_square(w: &Wall, i: usize, ta: f64, j: usize, tb: f64, d: f64, ccw: bool) -> Vec<Point3> {
    let c = [(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)];
    let nrm = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];
    let on = |k: usize, t: f64| {
        let (a, b) = (c[k % 4], c[(k + 1) % 4]);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let off = |p: (f64, f64), n: (f64, f64)| (p.0 + n.0 * d, p.1 + n.1 * d);
    let corner = |m: usize| {
        let (a, b) = (nrm[(m + 3) % 4], nrm[m % 4]);
        off(off(c[m % 4], a), b)
    };
    let (pa, pb) = (on(i, ta), on(j, tb));
    let mut pts = vec![pa, off(pa, nrm[i])];
    if ccw {
        // Corners c[i+1] … c[j] going counter-clockwise; all four when the
        // walk starts and ends on one side behind itself.
        let steps = if i == j {
            if tb > ta { 0 } else { 4 }
        } else {
            (j + 4 - i) % 4
        };
        for s in 1..=steps {
            pts.push(corner(i + s));
        }
    } else {
        let steps = if i == j {
            if tb < ta { 0 } else { 4 }
        } else {
            (i + 4 - j) % 4
        };
        for s in 0..steps {
            pts.push(corner(i + 4 - s));
        }
    }
    pts.push(off(pb, nrm[j]));
    pts.push(pb);
    pts.into_iter().map(|(x, y)| mm(w, x, y)).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Rule 3 for the new pair: a cut around the outside of an imprint,
    /// from one of its sides to another (or the same one), splits the face
    /// it sits in; merging any edge of the cut is the identity — directly,
    /// and through the history with undo and redo exact.
    #[test]
    fn a_hole_chord_then_a_merge_is_the_identity(
        i in 0usize..4,
        j in 0usize..4,
        ta_q in 0u32..9,
        tb_q in 0u32..9,
        d in 0.5f64..4.0,
        ccw in any::<bool>(),
        pick in 0usize..16,
    ) {
        // t = 0 lands on a corner; otherwise inside the side.
        let t = |q: u32| if q == 0 { 0.0 } else { f64::from(q) / 10.0 };
        let (ta, tb) = (t(ta_q), t(tb_q));
        prop_assume!(i != j || (ta - tb).abs() >= 0.1);
        let (mut w, _) = top_with_square();
        let p = w.face;
        let path = around_square(&w, i, ta, j, tb, d, ccw);
        let original = w.obj().clone();

        // Directly on the object.
        let mut o = original.clone();
        let r = o.split_face(p, &path).expect("a hole chord splits");
        o.validate().expect("valid split");
        prop_assert_eq!(euler(&o), euler(&original));
        prop_assert!(no_pinch(&o, &w.plane));
        let a0 = group_area(&original, &w.plane);
        prop_assert!((group_area(&o, &w.plane) - a0).abs() <= 1e-12 * a0);
        let edge = r.new_edges[pick % r.new_edges.len()];
        let m = o.merge_faces(edge).expect("merging the cut back");
        prop_assert_eq!(m.merged_face, p, "the hole owner survives");
        o.validate().expect("valid merge");
        prop_assert!(objects_equivalent(&o, &original), "split then merge is the identity");

        // Through the history.
        let bytes0 = w.doc.save();
        let (report, _) = w.doc.apply_object_op(w.id, split_op(p, path)).expect("split");
        let KernelOpReport::FaceSplit(r) = report else { panic!("split report") };
        let split_state = w.obj().clone();
        let bytes1 = w.doc.save();
        let edge = r.new_edges[pick % r.new_edges.len()];
        w.doc.apply_object_op(w.id, KernelOp::MergeFaces { edge, keep_vertices: Vec::new() }).expect("merge");
        prop_assert!(objects_equivalent(w.obj(), &original));
        let bytes2 = w.doc.save();
        for _ in 0..2 {
            w.doc.undo().expect("undo the merge");
            prop_assert!(objects_equivalent(w.obj(), &split_state));
            prop_assert_eq!(w.doc.save(), bytes1.clone());
            w.doc.undo().expect("undo the split");
            prop_assert!(objects_equivalent(w.obj(), &original));
            prop_assert_eq!(w.doc.save(), bytes0.clone());
            w.doc.redo().expect("redo the split");
            prop_assert!(objects_equivalent(w.obj(), &split_state));
            prop_assert_eq!(w.doc.save(), bytes1.clone());
            w.doc.redo().expect("redo the merge");
            prop_assert!(objects_equivalent(w.obj(), &original));
            prop_assert_eq!(w.doc.save(), bytes2.clone());
        }
    }
}

// ---------------------------------------------- the merge into the hole

#[test]
fn deleting_a_corner_run_reroutes_the_hole_around_it() {
    let (mut w, s) = top_with_square();
    let p = w.face;
    // Cut the square's top-left corner off.
    w.doc
        .apply_object_op(
            w.id,
            split_op(s, vec![mm(&w, 10.0, 17.0), mm(&w, 13.0, 20.0)]),
        )
        .expect("corner cut");
    let corner_edge = edge_at(w.obj(), mm(&w, 10.0, 17.0), mm(&w, 10.0, 20.0));
    // The report before undo/redo re-mints anything.
    let mut o = w.obj().clone();
    let r = o
        .merge_faces(corner_edge)
        .expect("the corner merges into the face");
    assert_eq!(r.merged_face, p, "the face holding the hole survives");
    assert_eq!(r.removed_edges.len(), 2, "both corner edges dissolve");
    assert!(r.prior_attrs.iter().all(Option::is_some));
    let r = merge_and_check(&mut w, corner_edge);
    assert_eq!(r.merged_face, p);
    let obj = w.obj();
    assert_eq!(obj.faces()[p].inner_loops.len(), 1);
    assert!(cyclic_match(
        &ring_of(obj, obj.faces()[p].inner_loops[0]),
        &[
            mm(&w, 10.0, 10.0),
            mm(&w, 10.0, 17.0),
            mm(&w, 13.0, 20.0),
            mm(&w, 20.0, 20.0),
            mm(&w, 20.0, 10.0),
        ],
        false
    ));
    // The rest of the square is a plain imprint again.
    let rest = face_at(&w, 15.0 * MM, 15.0 * MM);
    assert_ne!(rest, p);
    assert!(
        obj.faces()[rest].inner_loops.is_empty(),
        "the rest is a plain face"
    );
    assert!(
        rings_match(
            &face_rings(obj, rest),
            &(ring_of(obj, obj.faces()[p].inner_loops[0]), Vec::new()),
            true
        ),
        "the rest fills the hole the face now holds"
    );
}

#[test]
fn a_run_covering_a_whole_imprint_refuses_covers_boundary() {
    let (mut w, _) = top_with_square();
    let e = edge_at(w.obj(), mm(&w, 10.0, 10.0), mm(&w, 20.0, 10.0));
    assert_op_refused(
        &mut w,
        KernelOp::MergeFaces {
            edge: e,
            keep_vertices: Vec::new(),
        },
        StickyError::SharedChainCoversBoundary,
    );
}

#[test]
fn a_face_meeting_the_hole_along_two_runs_merges_and_splits_the_hole() {
    let (mut w, s) = top_with_square();
    w.doc
        .apply_object_op(
            w.id,
            split_op(s, vec![mm(&w, 10.0, 13.0), mm(&w, 20.0, 13.0)]),
        )
        .expect("lower cut");
    let upper = face_at(&w, 15.0 * MM, 18.0 * MM);
    w.doc
        .apply_object_op(
            w.id,
            split_op(upper, vec![mm(&w, 10.0, 17.0), mm(&w, 20.0, 17.0)]),
        )
        .expect("upper cut");
    // The middle band meets the hole along its left and its right side.
    // Stage 4 merges across both runs (it was refused
    // SharedChainDisconnected until then): the hole splits in two. The
    // full I1–I7 check is `face_arrangement_stage4_specs.rs`.
    let e = edge_at(w.obj(), mm(&w, 20.0, 13.0), mm(&w, 20.0, 17.0));
    let p = w.face;
    let (report, _) = w
        .doc
        .apply_object_op(
            w.id,
            KernelOp::MergeFaces {
                edge: e,
                keep_vertices: Vec::new(),
            },
        )
        .expect("the band merges across both runs");
    let KernelOpReport::FaceMerge(r) = report else {
        panic!("a merge report")
    };
    assert_eq!(r.merged_face, p);
    assert_eq!(w.obj().faces()[p].inner_loops.len(), 2);
    w.obj().validate().expect("valid");
}

// ------------------------------------------------- the inscribed circle

/// The maintainer's state: the 9.4 × 3.8 mm rectangle on `w`, then the
/// R = 1.9 mm circle inscribed at its left end (tangent to the left, top,
/// and bottom sides at its own vertices 12, 6, 18). Returns the circle.
fn inscribed(w: &mut Wall) -> kernel::CurveGeom {
    let s = imprint_rect(w);
    let (cx, cy) = (RX + R, RY + R);
    let g = w.circle_geom(cx, cy, R);
    let ring = w.ring(cx, cy, R, 24, 0.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the inscribed circle imprints");
    g
}

/// The circle's vertex `k` as it sits in the object.
fn rim(w: &Wall, k: usize) -> Point3 {
    let p = w.ring(RX + R, RY + R, R, 24, 0.0)[k];
    vertex_at(w.obj(), p)
}

fn tl_corner(w: &Wall) -> Point3 {
    vertex_at(w.obj(), w.at(RX, RY + RH))
}

fn bl_corner(w: &Wall) -> Point3 {
    vertex_at(w.obj(), w.at(RX, RY))
}

#[test]
fn a_disc_touching_the_hole_at_a_vertex_refuses_disconnected() {
    let mut w = front_wall();
    inscribed(&mut w);
    let e = edge_at(w.obj(), tl_corner(&w), rim(&w, 6));
    w.doc
        .apply_object_op(
            w.id,
            KernelOp::MergeFaces {
                edge: e,
                keep_vertices: Vec::new(),
            },
        )
        .expect("top-left corner merges");
    // The disc now meets the wall's hole along the top-left arc AND at the
    // bottom tangent vertex: merging it would pinch the hole.
    let arc = edge_at(w.obj(), rim(&w, 7), rim(&w, 8));
    assert_op_refused(
        &mut w,
        KernelOp::MergeFaces {
            edge: arc,
            keep_vertices: Vec::new(),
        },
        StickyError::SharedChainDisconnected,
    );
}

/// The two faces on either side of an interior edge.
fn faces_of_edge(obj: &Object, e: EdgeId) -> (FaceId, FaceId) {
    let ed = obj.edges()[e];
    let side = |h| obj.loops()[obj.half_edges()[h].loop_id].face;
    (
        side(ed.half_edge),
        side(ed.twin_half_edge.expect("interior edge")),
    )
}

#[test]
fn every_leftover_run_of_the_inscribed_circle_borders_the_wall_and_the_disc() {
    let mut w = front_wall();
    let g = inscribed(&mut w);
    let obj = w.obj();
    let wall = w.face;
    let faces = group_faces(obj, &w.plane);
    assert_eq!(faces.len(), 5, "the wall, the disc, and three leftovers");
    let disc = face_at(&w, RX + R, RY + R);
    assert!(obj.faces()[disc].inner_loops.is_empty());
    assert_eq!(
        obj.loop_half_edges(obj.faces()[disc].outer_loop).count(),
        24
    );
    // No whole imprint is left: the wall's hole is not one face's outline.
    assert_eq!(obj.faces()[wall].inner_loops.len(), 1);
    let leftovers: Vec<FaceId> = faces
        .iter()
        .copied()
        .filter(|&f| f != wall && f != disc)
        .collect();
    assert_eq!(leftovers.len(), 3);
    // Each leftover meets the wall along rectangle edges that claim nothing
    // and the disc along arc facets that keep the circle.
    let mut wall_runs = 0;
    for &lf in &leftovers {
        let mut with_wall = 0;
        let mut with_disc = 0;
        for e in obj.edges().keys() {
            if obj.edges()[e].twin_half_edge.is_none() {
                continue;
            }
            let (a, b) = faces_of_edge(obj, e);
            if a != lf && b != lf {
                continue;
            }
            let other = if a == lf { b } else { a };
            if other == wall {
                with_wall += 1;
                assert!(
                    obj.edges()[e].curve.is_none(),
                    "rectangle edges claim nothing"
                );
            } else if other == disc {
                with_disc += 1;
                assert_eq!(obj.edges()[e].curve, Some(g), "arcs keep the circle");
            }
        }
        assert!(with_wall >= 1 && with_disc >= 1);
        if with_wall == 2 {
            // The top-left and bottom-left leftovers: tangent, corner, tangent.
            let corner_vertex = [tl_corner(&w), bl_corner(&w)];
            let ring = ring_of(obj, obj.faces()[lf].outer_loop);
            assert!(ring.iter().any(|p| {
                corner_vertex
                    .iter()
                    .any(|c| c.approx_eq(*p, tol::POINT_MERGE))
            }));
        }
        wall_runs += 1;
    }
    assert_eq!(wall_runs, 3, "each leftover's run along the rectangle");
}

#[test]
fn a_painted_leftover_is_not_a_chord_and_the_wall_keeps_its_paint() {
    let mut w = front_wall();
    inscribed(&mut w);
    let red = w
        .doc
        .add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let inside_tl = (RX + 0.15 * MM, RY + RH - 0.15 * MM);
    let tl = face_at(&w, inside_tl.0, inside_tl.1);
    w.doc.paint_face(w.id, tl, Some(red)).expect("paint");
    let wall = w.face;
    assert_ne!(tl, wall);
    // The merge itself still dissolves it (F7): the wall survives with its
    // own (default) paint, and undo gives the leftover its red back.
    let e = edge_at(w.obj(), tl_corner(&w), rim(&w, 6));
    merge_and_check(&mut w, e);
    let material_at = |w: &Wall| w.obj().faces()[face_at(w, inside_tl.0, inside_tl.1)].material;
    assert_eq!(face_at(&w, inside_tl.0, inside_tl.1), wall);
    assert_eq!(material_at(&w), None);
    w.doc.undo().expect("undo the merge");
    assert_eq!(material_at(&w), Some(red));
    w.doc.redo().expect("redo the merge");
    assert_eq!(material_at(&w), None);

    // A red wall absorbing an unpainted leftover stays red.
    let mut w = front_wall();
    inscribed(&mut w);
    let red = w
        .doc
        .add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let wall = w.face;
    w.doc.paint_face(w.id, wall, Some(red)).expect("paint");
    let e = edge_at(w.obj(), tl_corner(&w), rim(&w, 6));
    merge_and_check(&mut w, e);
    assert_eq!(material_at(&w), Some(red));
    w.doc.undo().expect("undo the merge");
    assert_eq!(material_at(&w), None);
}

#[test]
fn merging_each_leftover_run_alone_is_exact() {
    for (a, b) in [(0usize, 6usize), (1, 18), (2, 0)] {
        let mut w = front_wall();
        inscribed(&mut w);
        let corner = match a {
            0 => tl_corner(&w),
            1 => bl_corner(&w),
            // The right leftover's run: its bottom edge from the tangent.
            _ => vertex_at(w.obj(), w.at(RX + RW, RY)),
        };
        let other = if a == 2 { rim(&w, 18) } else { rim(&w, b) };
        let e = edge_at(w.obj(), corner, other);
        merge_and_check(&mut w, e);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    }
}

// ------------------------------------------- the rounded-corner workflow

/// The maintainer's workflow on `w`: rectangle, inscribed circle, Delete on
/// the top-left and bottom-left leftovers' rectangle edges (rounding those
/// corners), then Delete on the arc between the disc and the right
/// leftover — one imprint with two rounded corners, smooth when pushed.
fn rounded_corner_workflow(mut w: Wall) {
    let after_rect = {
        imprint_rect(&mut w);
        (w.obj().clone(), w.doc.save())
    };
    w.doc.undo().expect("undo the rectangle");
    let plain = w.obj().clone();
    w.doc.redo().expect("redo the rectangle");
    assert!(objects_equivalent(w.obj(), &after_rect.0));
    let s = face_at(&w, RX + RW - MM, RY + MM);
    let (cx, cy) = (RX + R, RY + R);
    let g = w.circle_geom(cx, cy, R);
    let ring = w.ring(cx, cy, R, 24, 0.0);
    w.draw_loop(s, &ring, Some(g))
        .expect("the inscribed circle imprints");
    let after_circle = (w.obj().clone(), w.doc.save());

    // Top-left: one Delete dissolves both rectangle segments (F2).
    let e = edge_at(w.obj(), tl_corner(&w), rim(&w, 6));
    let r = merge_and_check(&mut w, e);
    assert_eq!(r.merged_face, w.face);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    assert!(
        find_edge(w.obj(), w.at(RX, RY + RH), rim(&w, 12)).is_none(),
        "the left segment went with the top one"
    );
    // Bottom-left.
    let e = edge_at(w.obj(), bl_corner(&w), rim(&w, 12));
    merge_and_check(&mut w, e);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
    let obj = w.obj();
    let hole = ring_of(obj, obj.faces()[w.face].inner_loops[0]);
    assert_eq!(
        hole.len(),
        2 + 13,
        "two right corners plus the left half-circle"
    );
    assert_eq!(claims_on(obj, g), 24, "every facet keeps its circle");
    // The arc between the disc and the right leftover.
    let arc = edge_at(w.obj(), rim(&w, 0), rim(&w, 1));
    merge_and_check(&mut w, arc);
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 2);
    let shape = face_at(&w, cx, cy);
    assert_eq!(
        group_faces(obj, &w.plane).len(),
        2,
        "the wall and one imprint"
    );
    assert_eq!(obj.faces()[w.face].inner_loops.len(), 1);
    assert_ne!(shape, w.face);
    assert!(obj.faces()[shape].inner_loops.is_empty());
    let loop_path = ring_of(obj, obj.faces()[shape].outer_loop);
    assert_eq!(loop_path.len(), 15);
    assert!(
        rings_match(
            &face_rings(obj, shape),
            &(ring_of(obj, obj.faces()[w.face].inner_loops[0]), Vec::new()),
            true
        ),
        "the imprint fills the wall's hole"
    );
    let arcs = obj
        .loop_half_edges(obj.faces()[shape].outer_loop)
        .filter(|&h| obj.edges()[obj.half_edges()[h].edge].curve == Some(g))
        .count();
    assert_eq!(arcs, 12, "an arc-and-line shape, not a circle");
    assert_eq!(
        claims_on(obj, g),
        12,
        "the left half-circle keeps its claims"
    );
    let rounded = (w.obj().clone(), w.doc.save());

    // Push/pull out and in: twelve smooth walls on the rounded corners.
    for d in [MM, -MM] {
        let shape = face_at(&w, cx, cy);
        w.doc
            .apply_object_op(
                w.id,
                KernelOp::PushPull {
                    face: shape,
                    distance: d,
                },
            )
            .expect("push/pull the rounded shape");
        let obj = w.obj();
        obj.validate().expect("valid after push/pull");
        let cylinders = obj
            .faces()
            .values()
            .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
            .count();
        assert_eq!(cylinders, 12, "the corner walls are cylinder facets");
        w.doc.undo().expect("undo the push/pull");
        assert!(objects_equivalent(w.obj(), &rounded.0));
        assert_eq!(w.doc.save(), rounded.1);
    }
    // And through the wall.
    let shape = face_at(&w, cx, cy);
    let (pieces, _) = w
        .doc
        .push_pull_through(w.id, shape, -15.0 * MM)
        .expect("push the rounded shape through");
    let cylinders: usize = pieces
        .iter()
        .map(|&id| {
            let o = w.doc.object(id).expect("piece");
            o.validate().expect("valid piece");
            o.faces()
                .values()
                .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
                .count()
        })
        .sum();
    assert_eq!(
        cylinders, 12,
        "the tunnel's corner walls are cylinder facets"
    );
    w.doc.undo().expect("undo the push-through");
    assert!(objects_equivalent(w.obj(), &rounded.0));
    assert_eq!(w.doc.save(), rounded.1);

    // The whole chain back to the plain wall and forward again.
    for _ in 0..3 {
        w.doc.undo().expect("undo a delete");
    }
    assert!(objects_equivalent(w.obj(), &after_circle.0));
    assert_eq!(w.doc.save(), after_circle.1);
    w.doc.undo().expect("undo the circle");
    assert!(objects_equivalent(w.obj(), &after_rect.0));
    assert_eq!(w.doc.save(), after_rect.1);
    w.doc.undo().expect("undo the rectangle");
    assert!(objects_equivalent(w.obj(), &plain));
    for _ in 0..5 {
        w.doc.redo().expect("redo");
    }
    assert!(objects_equivalent(w.obj(), &rounded.0));
    assert_eq!(w.doc.save(), rounded.1);
}

#[test]
fn rounding_two_corners_of_a_wall_rect_by_deleting_lines() {
    rounded_corner_workflow(front_wall());
}

#[test]
fn rounding_two_corners_on_a_tilted_face_by_deleting_lines() {
    rounded_corner_workflow(tilted_wall());
}

// -------------------------------------------- moving a run along the hole

#[test]
fn a_leftover_run_slides_along_the_hole_and_undoes() {
    let (mut w, s) = top_with_square();
    // A notch off the square's top side.
    w.doc
        .apply_object_op(
            w.id,
            split_op(
                s,
                vec![
                    mm(&w, 12.0, 20.0),
                    mm(&w, 12.0, 18.0),
                    mm(&w, 14.0, 18.0),
                    mm(&w, 14.0, 20.0),
                ],
            ),
        )
        .expect("notch");
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let notch = face_at(&w, 13.0 * MM, 19.0 * MM);
    let wall = w.face;
    assert_ne!(notch, wall);
    // The notch's run along the hole is the square's top edge between the
    // notch's two corners; sliding it down the notch is one draft.
    let top = (mm(&w, 12.0, 20.0), mm(&w, 14.0, 20.0));
    let down = Transform::translation(w.v * (-MM));
    let depth0 = w.doc.undo_depth();
    let (_report, _) = draft_on_face(&mut w.doc, None, w.id, wall, "Move", |sk| {
        let e = sk.edge_at_positions(top.0, top.1).expect("the run");
        sk.apply_transform_edges(&[e], &down)
    })
    .expect("slide the run 1 mm down the notch");
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo step");
    let obj = w.obj();
    obj.validate().expect("valid");
    let moved = face_at(&w, 13.0 * MM, 18.5 * MM);
    assert!(cyclic_match(
        &ring_of(obj, obj.faces()[moved].outer_loop),
        &[
            mm(&w, 12.0, 18.0),
            mm(&w, 14.0, 18.0),
            mm(&w, 14.0, 19.0),
            mm(&w, 12.0, 19.0),
        ],
        false
    ));
    assert_eq!(face_at(&w, 13.0 * MM, 19.5 * MM), wall);
    w.doc.undo().expect("undo the slide");
    assert!(objects_equivalent(w.obj(), &before));
    assert_eq!(w.doc.save(), bytes0);
}

// ------------------------------------------------ booleans keep the lines

#[test]
fn the_boolean_preserve_list_holds_every_arrangement_edge() {
    let mut w = front_wall();
    inscribed(&mut w);
    let segs = w.obj().coplanar_edge_segments();
    let listed = |a: Point3, b: Point3| {
        segs.iter().any(|&(p, q)| {
            (p.approx_eq(a, tol::POINT_MERGE) && q.approx_eq(b, tol::POINT_MERGE))
                || (p.approx_eq(b, tol::POINT_MERGE) && q.approx_eq(a, tol::POINT_MERGE))
        })
    };
    // Outer/outer arcs, as before, and the leftover/wall runs (F8).
    assert!(listed(rim(&w, 0), rim(&w, 1)));
    assert!(listed(tl_corner(&w), rim(&w, 6)));
    assert!(listed(tl_corner(&w), rim(&w, 12)));
    assert!(listed(bl_corner(&w), rim(&w, 18)));
    let obj = w.obj();
    let wall_edges: Vec<(Point3, Point3)> = group_faces(obj, &w.plane)
        .into_iter()
        .filter(|&f| f != w.face)
        .flat_map(|f| {
            obj.loop_half_edges(obj.faces()[f].outer_loop)
                .map(|h| obj.edge_endpoints(obj.half_edges()[h].edge).expect("edge"))
                .collect::<Vec<_>>()
        })
        .collect();
    for (a, b) in wall_edges {
        assert!(listed(a, b), "arrangement edge {a:?} – {b:?} preserved");
    }
}

#[test]
fn a_union_keeps_every_arrangement_edge() {
    let mut w = front_wall();
    inscribed(&mut w);
    let obj = w.obj();
    let lines: Vec<(Point3, Point3)> = group_faces(obj, &w.plane)
        .into_iter()
        .filter(|&f| f != w.face)
        .flat_map(|f| {
            obj.loop_half_edges(obj.faces()[f].outer_loop)
                .map(|h| obj.edge_endpoints(obj.half_edges()[h].edge).expect("edge"))
                .collect::<Vec<_>>()
        })
        .collect();
    // A second box overlapping the first's back half, clear of the wall.
    let other = {
        let s = w.doc.add_sketch(ground());
        let c = [
            Point3::new(15.0 * MM, 4.0 * MM, 0.0),
            Point3::new(25.0 * MM, 4.0 * MM, 0.0),
            Point3::new(25.0 * MM, 8.0 * MM, 0.0),
            Point3::new(15.0 * MM, 8.0 * MM, 0.0),
        ];
        let sk = w.doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
        let regions = w.doc.extrudable_regions(s).expect("regions");
        w.doc
            .extrude_region(s, regions[0], 5.0 * MM)
            .expect("second box")
            .0
    };
    let (merged, _) = w.doc.boolean(BooleanOp::Union, w.id, other).expect("union");
    let res = w.doc.object(merged).expect("result");
    res.validate().expect("valid union");
    for (a, b) in lines {
        let mid = a + (b - a) * 0.5;
        let kept = res.edges().keys().any(|e| {
            let (p, q) = res.edge_endpoints(e).expect("edge");
            let pq = q - p;
            let t = (mid - p).dot(pq) / pq.length_squared();
            t > 0.0 && t < 1.0 && (p + pq * t).approx_eq(mid, tol::POINT_MERGE)
        });
        assert!(kept, "the union kept the arrangement edge {a:?} – {b:?}");
    }
}

// ------------------------------------------- cuts that cannot be a chord

/// Every loop of every face visits each of its vertices once.
fn no_pinch_anywhere(obj: &Object) -> bool {
    obj.faces().values().all(|face| {
        std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .all(|l| {
                let vs: Vec<_> = obj
                    .loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect();
                (0..vs.len()).all(|i| !vs[i + 1..].contains(&vs[i]))
            })
    })
}

#[test]
fn a_cut_whose_first_and_last_segments_cross_refuses() {
    // From the bottom edge to the left edge of the plain top, crossing
    // itself between its first and last segments.
    let mut w = top_of_box();
    let p = w.face;
    let path = vec![
        mm(&w, 10.0, 0.0),
        mm(&w, 20.0, 20.0),
        mm(&w, 25.0, 5.0),
        mm(&w, 0.0, 10.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
    // The same around an imprint, hole to hole.
    let (mut w, _) = top_with_square();
    let p = w.face;
    let path = vec![
        mm(&w, 10.0, 12.0),
        mm(&w, 4.0, 16.0),
        mm(&w, 4.0, 6.0),
        mm(&w, 10.0, 14.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
}

#[test]
fn a_cut_closing_on_one_boundary_point_refuses() {
    let mut w = top_of_box();
    let p = w.face;
    // From a corner of the face back to the same corner.
    let path = vec![
        mm(&w, 0.0, 0.0),
        mm(&w, 10.0, 5.0),
        mm(&w, 5.0, 10.0),
        mm(&w, 0.0, 0.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
    // From a point inside an edge back to the same point.
    let path = vec![
        mm(&w, 12.0, 0.0),
        mm(&w, 15.0, 5.0),
        mm(&w, 9.0, 5.0),
        mm(&w, 12.0, 0.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
    // Around an imprint, from one hole corner back to it.
    let (mut w, _) = top_with_square();
    let p = w.face;
    let path = vec![
        mm(&w, 10.0, 20.0),
        mm(&w, 5.0, 25.0),
        mm(&w, 5.0, 22.0),
        mm(&w, 10.0, 20.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
}

#[test]
fn a_cut_with_a_zero_length_segment_refuses() {
    let mut w = top_of_box();
    let p = w.face;
    let path = vec![
        mm(&w, 0.0, 10.0),
        mm(&w, 10.0, 10.0),
        mm(&w, 10.0, 10.0),
        mm(&w, 30.0, 10.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
    let (mut w, _) = top_with_square();
    let p = w.face;
    let path = vec![
        mm(&w, 10.0, 12.0),
        mm(&w, 5.0, 12.0),
        mm(&w, 5.0, 12.0),
        mm(&w, 5.0, 15.0),
        mm(&w, 10.0, 15.0),
    ];
    assert_op_refused(&mut w, split_op(p, path), StickyError::PathNotSimple);
}

// ------------------------------------------------- merges that would pinch

#[test]
fn a_merge_that_would_pinch_the_outer_loop_refuses_and_is_no_chord() {
    // A rectangle flush against the top's right edge, cut by a diagonal:
    // the lower triangle meets the face around it along its bottom edge and
    // at the rectangle's top-right corner. Merging it would make the face's
    // outer loop pass that corner twice.
    let mut w = top_of_box();
    let p = w.face;
    w.doc
        .apply_object_op(
            w.id,
            split_op(
                p,
                vec![
                    mm(&w, 30.0, 10.0),
                    mm(&w, 20.0, 10.0),
                    mm(&w, 20.0, 20.0),
                    mm(&w, 30.0, 20.0),
                ],
            ),
        )
        .expect("flush rectangle");
    let rect = face_at(&w, 25.0 * MM, 15.0 * MM);
    w.doc
        .apply_object_op(
            w.id,
            split_op(rect, vec![mm(&w, 20.0, 10.0), mm(&w, 30.0, 20.0)]),
        )
        .expect("diagonal");
    let low = face_at(&w, 27.0 * MM, 12.0 * MM);
    let outside = face_at(&w, 5.0 * MM, 5.0 * MM);
    assert_ne!(low, outside);
    let e = edge_at(w.obj(), mm(&w, 20.0, 10.0), mm(&w, 30.0, 10.0));
    assert_op_refused(
        &mut w,
        KernelOp::MergeFaces {
            edge: e,
            keep_vertices: Vec::new(),
        },
        StickyError::SharedChainDisconnected,
    );
    assert!(no_pinch_anywhere(w.obj()));
    // Every interior edge between group faces that merges, merges without
    // a pinch.
    let plane = w.plane;
    let group = group_faces(w.obj(), &plane);
    let edges: Vec<EdgeId> = w
        .obj()
        .edges()
        .keys()
        .filter(|&e| {
            w.obj().edges()[e].twin_half_edge.is_some() && {
                let (a, b) = faces_of_edge(w.obj(), e);
                group.contains(&a) && group.contains(&b)
            }
        })
        .collect();
    let mut merged = 0;
    for edge in edges {
        let mut o = w.obj().clone();
        if o.merge_faces(edge).is_ok() {
            merged += 1;
            o.validate().expect("valid merge");
            assert!(no_pinch_anywhere(&o));
        }
    }
    assert!(merged >= 1, "some edge of the arrangement merges");
}

// ----------------------------------- redo of a merge of differently painted faces

#[test]
fn redoing_a_merge_of_differently_painted_faces_reproduces_it_exactly() {
    for red_left in [true, false] {
        for flip in [false, true] {
            let mut w = top_of_box();
            let p = w.face;
            let cut = if flip {
                vec![mm(&w, 12.0, 30.0), mm(&w, 12.0, 0.0)]
            } else {
                vec![mm(&w, 12.0, 0.0), mm(&w, 12.0, 30.0)]
            };
            w.doc
                .apply_object_op(w.id, split_op(p, cut))
                .expect("cut the top in two");
            let red = w
                .doc
                .add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
            let x = if red_left { 5.0 } else { 20.0 };
            let side = face_at(&w, x * MM, 15.0 * MM);
            w.doc.paint_face(w.id, side, Some(red)).expect("paint");
            let e = edge_at(w.obj(), mm(&w, 12.0, 0.0), mm(&w, 12.0, 30.0));
            w.doc
                .apply_object_op(
                    w.id,
                    KernelOp::MergeFaces {
                        edge: e,
                        keep_vertices: Vec::new(),
                    },
                )
                .expect("merge");
            let merged = w.doc.save();
            let material = w.obj().faces()[face_at(&w, 15.0 * MM, 15.0 * MM)].material;
            for _ in 0..3 {
                w.doc.undo().expect("undo the merge");
                assert_eq!(
                    w.obj().faces()[face_at(&w, x * MM, 15.0 * MM)].material,
                    Some(red)
                );
                w.doc.redo().expect("redo the merge");
                assert_eq!(
                    w.obj().faces()[face_at(&w, 15.0 * MM, 15.0 * MM)].material,
                    material,
                    "redo keeps the survivor's paint (red_left {red_left}, flip {flip})"
                );
                assert_eq!(w.doc.save(), merged, "redo reproduces the merge exactly");
            }
        }
    }
}

// ------------------------------------------- moving a run along a hole

#[test]
fn moving_a_hole_run_off_the_hole_is_accepted_as_drawing() {
    let mut w = front_wall();
    inscribed(&mut w);
    let wall = w.face;
    // The top-left run (tangent, corner, tangent) slid sideways: drafting
    // moves lines like any others (the old chord move refused it).
    let (t1, c, t2) = (rim(&w, 6), tl_corner(&w), rim(&w, 12));
    let before = w.obj().clone();
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let xf = Transform::translation(w.u * MM);
    let (report, _) = draft_on_face(&mut w.doc, None, w.id, wall, "Move", |sk| {
        let es = [
            sk.edge_at_positions(t1, c).expect("run edge"),
            sk.edge_at_positions(c, t2).expect("run edge"),
        ];
        sk.apply_transform_edges(&es, &xf)
    })
    .expect("moving the run draws");
    let _ = report;
    let owned = w.doc.object_owned_sketches(w.id);
    assert_eq!(owned.len(), 1, "kept in the object's owned sketch");
    assert!(!w.doc.sketch(owned[0]).unwrap().edges().is_empty());
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 5);
    w.obj().validate().expect("valid");
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo step");
    let after = w.obj().clone();
    let bytes1 = w.doc.save();
    assert_reopens(&bytes1);
    w.doc.undo().expect("undo");
    assert!(objects_equivalent(w.obj(), &before));
    assert_eq!(w.doc.save(), bytes0);
    w.doc.redo().expect("redo");
    assert!(objects_equivalent(w.obj(), &after));
    assert_eq!(w.doc.save(), bytes1);
}
