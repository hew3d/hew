//! Executable specs for ground-parity drawing on a solid's face, stage 4:
//! the local re-trace primitive, [`Object::retrace_face`], and what it
//! unlocks.
//!
//! `retrace_face(face, insert, remove)` inserts and/or removes several
//! paths in one face at once and re-traces the face's loops by rotational
//! order at every vertex — the half-edge form of the ground's region
//! tracer. With the arguments swapped it is its own inverse (except that a
//! plain swapped call heals away an inserted end that met the joint of a
//! straight run already there), recorded in the history as one
//! [`KernelOp::RetraceFace`] with an exact undo that keeps such joints. A path
//! may join two different boundary loops of the face (a BRIDGE) as long as
//! the paths inserted together leave no bridge behind: every committed loop
//! stays simple, and loops of one face share at most isolated vertices,
//! never an edge.
//!
//! With it:
//! - a loop whose pieces connect two different boundary loops of one face
//!   is accepted (a circle overlapping two separate imprints; a shape drawn
//!   from the face's outer edge across an imprint; a rectangle spanning
//!   three imprints; a rectangle flush between two imprints, whose holes
//!   merge into one) — the result equals the ground sketch of the same
//!   lines;
//! - deleting a line between two faces that meet along SEVERAL runs merges
//!   them across every run: two faces side by side gain a hole, a face in a
//!   hole splits that hole into several. Each such run is a chord feature.
//!
//! A loop touching another boundary at a single point (F1) is face topology
//! too: the walk the re-trace takes through that point is split there, so
//! the face around the loop holds it as a hole sharing the vertex.
//!
//! Every accepting spec checks the stage invariants (see
//! `face_arrangement_specs.rs`): I1 valid and watertight, I2 Euler–Poincaré
//! unchanged, I3 group area conserved, I4 loops simple, I5 exact undo/redo with
//! byte-identical saves, I6 the face group equals the ground's regions for
//! the same lines, I7 the saved document reopens.
//!
//! Also here: transforms and rescales that would leave a face too small for
//! a saved file to reopen refuse typed instead of panicking.

use std::f64::consts::PI;

use kernel::{
    Document, DocumentError, EdgeId, FaceId, KernelOp, KernelOpReport, NodeId, Object, Plane,
    Point3, RetracePath, Sketch, StickyError, Transform, TransformError, Vec3, tol,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::*;

// --------------------------------------------------------------- helpers

/// The front wall carrying two 4 × 3 mm imprints at (3, 2) and (9, 2) mm:
/// `(wall, a, b)`.
fn wall_with_two() -> (Wall, FaceId, FaceId) {
    let mut w = front_wall();
    let wall = w.face;
    let a = imprint_mm(&mut w, wall, 3.0, 2.0, 4.0, 3.0);
    let b = imprint_mm(&mut w, wall, 9.0, 2.0, 4.0, 3.0);
    (w, a, b)
}

/// The two bridges between the imprints' facing sides that cut the strip
/// between them off the wall: top from `a` to `b`, bottom back.
fn strip_bridges(w: &Wall) -> Vec<RetracePath> {
    vec![
        RetracePath {
            path: vec![mm(w, 7.0, 4.0), mm(w, 8.0, 4.5), mm(w, 9.0, 4.0)],
            curves: Vec::new(),
            polygons: Vec::new(),
        },
        RetracePath {
            path: vec![mm(w, 9.0, 3.0), mm(w, 8.0, 2.5), mm(w, 7.0, 3.0)],
            curves: Vec::new(),
            polygons: Vec::new(),
        },
    ]
}

/// Every group face follows the loop rule (ARCHITECTURE §2.4): each loop
/// visits no vertex twice (no pinch), and no edge has the face on both
/// sides (no bridge, inside one loop or between two). Loops of one face may
/// share isolated vertices.
fn group_faces_follow_the_loop_rule(obj: &Object, plane: &Plane) -> bool {
    group_faces(obj, plane).into_iter().all(|f| {
        let face = &obj.faces()[f];
        let loops: Vec<kernel::LoopId> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .collect();
        let simple = loops.iter().all(|&l| {
            let vs: Vec<_> = obj
                .loop_half_edges(l)
                .map(|h| obj.half_edges()[h].origin)
                .collect();
            (0..vs.len()).all(|i| !vs[i + 1..].contains(&vs[i]))
        });
        let no_shared_edge = loops.iter().all(|&l| {
            obj.loop_half_edges(l).all(|h| {
                obj.half_edges()[h]
                    .twin
                    .is_none_or(|t| obj.loops()[obj.half_edges()[t].loop_id].face != f)
            })
        });
        simple && no_shared_edge
    })
}

/// Draws `loop_path` from `seed` with I1–I7, I6 against the ground, and
/// every group face following the loop rule; the whole loop lands as face
/// topology.
fn bridge_and_check(w: &mut Wall, seed: FaceId, loop_path: Vec<Point3>) {
    draw_and_check(w, seed, loop_path, None);
    assert!(
        group_faces_follow_the_loop_rule(w.obj(), &w.plane),
        "no pinch or bridge"
    );
}

/// Every face of the group, pushed out and in, then undone: the identity
/// (rule 3). `faces` is the expected group size.
fn push_pull_every_face_and_undo(w: &mut Wall, faces: usize) {
    let group = group_faces(w.obj(), &w.plane);
    assert_eq!(group.len(), faces);
    let before = w.obj().clone();
    let bytes = w.doc.save();
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

/// The run holding `edge` lies between two distinct faces of the group
/// (what the app's Delete acts on).
fn assert_chord_feature(w: &Wall, edge: EdgeId) {
    let obj = w.obj();
    let e = obj.edges()[edge];
    let fa = obj.loops()[obj.half_edges()[e.half_edge].loop_id].face;
    let tw = e.twin_half_edge.expect("interior edge");
    let fb = obj.loops()[obj.half_edges()[tw].loop_id].face;
    let group = group_faces(obj, &w.plane);
    assert!(
        fa != fb && group.contains(&fa) && group.contains(&fb),
        "the run separates two faces of the group"
    );
}

/// `obj`'s edge between `a` and `b`, by exact vertex positions.
fn edge_between(obj: &Object, a: Point3, b: Point3) -> EdgeId {
    edge_at(obj, vertex_at(obj, a), vertex_at(obj, b))
}

/// Deletes the run holding `edge` — every run its two faces share —
/// through the document with I1–I5 and I7, and I6 against the ground with
/// every shared edge removed (straight-through vertices aside).
fn merge_runs_and_check(w: &mut Wall, edge: EdgeId) -> kernel::FaceMergeReport {
    let before = w.obj().clone();
    let runs = run_segments(&before, edge);
    let report = apply_and_check(
        w,
        KernelOp::MergeFaces {
            edge,
            keep_vertices: Vec::new(),
        },
    );
    let n = w.plane.normal();
    let ground = straightened(
        &ground_regions(&ground_edit(w.plane, &before, &[], &runs)),
        n,
    );
    let obj = w.obj();
    let faces: Vec<Rings> = group_faces(obj, &w.plane)
        .into_iter()
        .map(|f| face_rings(obj, f))
        .collect();
    assert!(
        multiset_match(&straightened(&faces, n), &ground, true),
        "the face group differs from the ground"
    );
    assert!(group_faces_follow_the_loop_rule(obj, &w.plane));
    match report {
        KernelOpReport::FaceMerge(r) => r,
        other => panic!("expected a merge report, got {other:?}"),
    }
}

// ----------------------------------------------- the primitive itself

#[test]
fn retracing_two_bridges_cuts_the_strip_between_two_imprints() {
    let (w, a, b) = wall_with_two();
    let p = w.face;
    let original = w.obj().clone();
    let mut o = original.clone();
    let r = o
        .retrace_face(p, &strip_bridges(&w), &[])
        .expect("two bridges between the holes retrace");
    o.validate().expect("valid");
    assert_eq!(euler(&o), euler(&original), "V - E + F - H unchanged");
    assert!(group_faces_follow_the_loop_rule(&o, &w.plane));
    assert_eq!(group_faces(&o, &w.plane).len(), 4, "wall, strip, a, b");
    assert_eq!(r.new_edges.len(), 2, "one edge list per inserted path");
    assert!(r.new_edges.iter().all(|e| e.len() == 2));
    assert_eq!(r.faces.len(), 2, "the wall became two faces");
    // The wall keeps one hole, around a, the strip, and b together.
    let wall_now = r
        .faces
        .iter()
        .copied()
        .find(|&f| !o.faces()[f].inner_loops.is_empty())
        .expect("the face with the hole");
    assert_eq!(o.faces()[wall_now].inner_loops.len(), 1, "two holes merged");
    assert!(o.faces().contains_key(a) && o.faces().contains_key(b));
    // Here, with no inserted end on a straight joint, the same primitive
    // with the arguments swapped is the inverse.
    let removed: Vec<Vec<Point3>> = r.inserted.iter().map(|p| p.path.clone()).collect();
    let back = o
        .retrace_face(r.faces[0], &[], &removed)
        .expect("removing the bridges again");
    o.validate().expect("valid");
    assert!(
        objects_equivalent(&o, &original),
        "retrace then its inverse is the identity"
    );
    assert_eq!(back.faces.len(), 1, "one face again");
}

#[test]
fn retracing_through_the_history_undoes_and_redoes_exactly() {
    let (mut w, _, _) = wall_with_two();
    let p = w.face;
    let insert = strip_bridges(&w);
    let report = apply_and_check(
        &mut w,
        KernelOp::RetraceFace {
            face: p,
            insert,
            remove: Vec::new(),
            restore: Vec::new(),
            keep_vertices: Vec::new(),
        },
    );
    assert!(matches!(report, KernelOpReport::FaceRetrace(_)));
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    let before = Document::load(&w.doc.save()).expect("reopens");
    assert_eq!(before.save(), w.doc.save());
}

#[test]
fn a_lone_bridge_refuses_untouched() {
    // One bridge joins the two holes without splitting the wall: the
    // result would hold a bridge, which no committed face may.
    let (mut w, _, _) = wall_with_two();
    let p = w.face;
    let one = vec![strip_bridges(&w).remove(0)];
    let mut o = w.obj().clone();
    assert_eq!(
        o.retrace_face(p, &one, &[]).map(|_| ()),
        Err(StickyError::PathNotSimple)
    );
    assert!(objects_equivalent(&o, w.obj()), "untouched");
    assert_op_refused(
        &mut w,
        KernelOp::RetraceFace {
            face: p,
            insert: one,
            remove: Vec::new(),
            restore: Vec::new(),
            keep_vertices: Vec::new(),
        },
        StickyError::PathNotSimple,
    );
}

#[test]
fn retrace_refuses_crossing_or_off_boundary_paths_untouched() {
    let (w, _, _) = wall_with_two();
    let p = w.face;
    let mut o = w.obj().clone();
    // Two bridges crossing each other.
    let crossing = vec![
        RetracePath {
            path: vec![mm(&w, 7.0, 4.0), mm(&w, 9.0, 3.0)],
            curves: Vec::new(),
            polygons: Vec::new(),
        },
        RetracePath {
            path: vec![mm(&w, 9.0, 4.0), mm(&w, 7.0, 3.0)],
            curves: Vec::new(),
            polygons: Vec::new(),
        },
    ];
    assert_eq!(
        o.retrace_face(p, &crossing, &[]).map(|_| ()),
        Err(StickyError::PathNotSimple)
    );
    // An end in the middle of the face.
    let loose = vec![RetracePath {
        path: vec![mm(&w, 7.0, 4.0), mm(&w, 8.0, 4.5)],
        curves: Vec::new(),
        polygons: Vec::new(),
    }];
    assert_eq!(
        o.retrace_face(p, &loose, &[]).map(|_| ()),
        Err(StickyError::EndpointNotOnBoundary { which: 1 })
    );
    // A chain that is no edge of the object.
    assert_eq!(
        o.retrace_face(p, &[], &[vec![mm(&w, 7.0, 4.0), mm(&w, 9.0, 4.0)]])
            .map(|_| ()),
        Err(StickyError::UnknownEdge)
    );
    assert!(objects_equivalent(&o, w.obj()), "untouched");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    /// Rule 3 for the re-trace: two random bridges between the imprints'
    /// facing sides, then the same primitive with the arguments swapped,
    /// is the identity — directly, and through the history with undo and
    /// redo exact.
    #[test]
    fn retrace_then_its_inverse_is_the_identity(
        at in 3.2f64..4.8,
        ab_gap in 0.2f64..1.0,
        bt in 3.2f64..4.8,
        bb_gap in 0.2f64..1.0,
        mt in 0.0f64..1.0,
        mb in 0.0f64..1.0,
    ) {
        let (ab, bb) = (at - ab_gap, bt - bb_gap);
        prop_assume!(ab > 2.1 && bb > 2.1);
        let (mut w, _, _) = wall_with_two();
        let p = w.face;
        // The midpoints: the top one above the bottom one by at least 0.1.
        let lo = 2.2_f64;
        let mid_b = lo + mb * (at.min(bt) - lo - 0.2);
        let mid_t = mid_b + 0.1 + mt * (4.9 - mid_b - 0.1);
        prop_assume!(mid_t > mid_b + 0.1 - 1e-12);
        let insert = vec![
            RetracePath {
                path: vec![mm(&w, 7.0, at), mm(&w, 8.0, mid_t), mm(&w, 9.0, bt)],
                curves: Vec::new(),
                polygons: Vec::new(),
            },
            RetracePath {
                path: vec![mm(&w, 9.0, bb), mm(&w, 8.0, mid_b), mm(&w, 7.0, ab)],
                curves: Vec::new(),
                polygons: Vec::new(),
            },
        ];
        let original = w.obj().clone();
        let mut o = original.clone();
        let r = o.retrace_face(p, &insert, &[]).expect("bridges retrace");
        o.validate().expect("valid");
        prop_assert_eq!(euler(&o), euler(&original));
        prop_assert!(group_faces_follow_the_loop_rule(&o, &w.plane));
        let a0 = group_area(&original, &w.plane);
        prop_assert!((group_area(&o, &w.plane) - a0).abs() <= 1e-12 * a0);
        let removed: Vec<Vec<Point3>> = r.inserted.iter().map(|p| p.path.clone()).collect();
        o.retrace_face(r.faces[1], &[], &removed).expect("the inverse");
        prop_assert!(objects_equivalent(&o, &original));

        let bytes0 = w.doc.save();
        w.doc
            .apply_object_op(
                w.id,
                KernelOp::RetraceFace { face: p, insert, remove: Vec::new(), restore: Vec::new(), keep_vertices: Vec::new() },
            )
            .expect("through the history");
        let after = w.obj().clone();
        let bytes1 = w.doc.save();
        for _ in 0..2 {
            w.doc.undo().expect("undo");
            prop_assert!(objects_equivalent(w.obj(), &original));
            prop_assert_eq!(w.doc.save(), bytes0.clone());
            w.doc.redo().expect("redo");
            prop_assert!(objects_equivalent(w.obj(), &after));
            prop_assert_eq!(w.doc.save(), bytes1.clone());
        }
    }
}

// ------------------------------------------------------------ bridges

#[test]
fn a_circle_overlapping_two_imprints_matches_the_ground() {
    for pick in 0..3 {
        let (mut w, a, b) = wall_with_two();
        let seed = [w.face, a, b][pick];
        let ring = w.ring(8.0 * MM, 3.5 * MM, 1.5 * MM, 24, PI / 24.0);
        bridge_and_check(&mut w, seed, ring);
        // a and b each lose a lens; the strip between them is a face; the
        // wall keeps one hole around all of it.
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 6);
        let wall_now = face_at(&w, 1.0 * MM, 1.0 * MM);
        assert_eq!(w.obj().faces()[wall_now].inner_loops.len(), 1);
    }
}

#[test]
fn a_rectangle_from_the_wall_edge_across_an_imprint_matches_the_ground() {
    for on_imprint in [false, true] {
        let mut w = front_wall();
        let s = imprint_rect(&mut w);
        let seed = if on_imprint { s } else { w.face };
        let rect = w.rect(5.0 * MM, 0.0, 2.0 * MM, 3.0 * MM);
        bridge_and_check(&mut w, seed, rect);
        // The wall, the piece between its edge and the imprint, the
        // imprint's overlap, and the rest of the imprint.
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
        let wall_now = face_at(&w, 1.0 * MM, 8.0 * MM);
        assert!(
            w.obj().faces()[wall_now].inner_loops.is_empty(),
            "the shapes reach the wall's edge: no hole is left"
        );
    }
}

#[test]
fn a_rectangle_spanning_three_imprints_matches_the_ground() {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 2.0, 2.0, 3.0, 6.0);
    let mid = imprint_mm(&mut w, wall, 8.5, 2.0, 3.0, 6.0);
    imprint_mm(&mut w, wall, 15.0, 2.0, 3.0, 6.0);
    let rect = w.rect(3.5 * MM, 4.0 * MM, 13.0 * MM, 2.0 * MM);
    bridge_and_check(&mut w, mid, rect);
    // Each end imprint: its overlap and the rest (a C); the middle one:
    // above, overlap, below; the two strips between; the wall.
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 10);
    push_pull_every_face_and_undo(&mut w, 10);
}

#[test]
fn a_rectangle_flush_between_two_imprints_merges_their_holes() {
    let (mut w, _, _) = wall_with_two();
    let wall = w.face;
    assert_eq!(w.obj().faces()[wall].inner_loops.len(), 2);
    let before_draw = (w.obj().clone(), w.doc.save());
    let rect = w.rect(7.0 * MM, 3.0 * MM, 2.0 * MM, 1.0 * MM);
    bridge_and_check(&mut w, wall, rect);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    let wall_now = face_at(&w, 1.0 * MM, 1.0 * MM);
    assert_eq!(
        w.obj().faces()[wall_now].inner_loops.len(),
        1,
        "two holes merged into one"
    );
    // Deleting the strip's top line merges it into the wall along BOTH of
    // its runs (top and bottom): the hole splits back into two, and the
    // imprints' sides heal — the wall before the draw.
    let top = edge_between(w.obj(), mm(&w, 7.0, 4.0), mm(&w, 9.0, 4.0));
    assert_chord_feature(&w, top);
    merge_runs_and_check(&mut w, top);
    assert!(objects_equivalent(w.obj(), &before_draw.0));
    // The undo chain: the strip back, then the plain wall.
    w.doc.undo().expect("undo the delete");
    w.doc.undo().expect("undo the draw");
    assert_eq!(w.doc.save(), before_draw.1);
}

#[test]
fn every_face_of_a_bridged_result_pushes_and_pulls_and_undoes() {
    let (mut w, _, _) = wall_with_two();
    let ring = w.ring(8.0 * MM, 3.5 * MM, 1.5 * MM, 24, PI / 24.0);
    let g = w.circle_geom(8.0 * MM, 3.5 * MM, 1.5 * MM);
    let wall = w.face;
    w.draw_loop(wall, &ring, Some(g))
        .expect("the circle over two imprints");
    push_pull_every_face_and_undo(&mut w, 6);

    let mut w = front_wall();
    let s = imprint_rect(&mut w);
    let rect = w.rect(5.0 * MM, 0.0, 2.0 * MM, 3.0 * MM);
    w.draw_loop(s, &rect, None)
        .expect("from the wall's edge across the imprint");
    push_pull_every_face_and_undo(&mut w, 4);
}

#[test]
fn a_bridged_circle_keeps_its_claims_on_every_whole_facet() {
    let (mut w, _, _) = wall_with_two();
    let g = w.circle_geom(8.0 * MM, 3.5 * MM, 1.5 * MM);
    let ring = w.ring(8.0 * MM, 3.5 * MM, 1.5 * MM, 24, PI / 24.0);
    let wall = w.face;
    w.draw_loop(wall, &ring, Some(g))
        .expect("the circle over two imprints");
    // Four proper crossings cut four facets (F11); the other 20 keep it.
    assert_eq!(claims_on(w.obj(), g), 20);
}

#[test]
fn single_point_touches_land_as_touching_holes() {
    // A circle tangent once to the wall's bottom edge and once to the
    // imprint's bottom side: the disc is a face, and the wall holds it as a
    // hole sharing one point with the wall's outer loop and one with the
    // imprint's hole, as on the ground (F1).
    for pick in [false, true] {
        let mut w = front_wall();
        let s = imprint_rect(&mut w);
        let ring = w.ring(8.0 * MM, 1.0 * MM, 1.0 * MM, 24, 0.0);
        let seed = if pick { s } else { w.face };
        draw_and_check(&mut w, seed, ring, None);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
        assert!(has_touching_loops(w.obj(), &w.plane));
        assert!(w.doc.object_owned_sketches(w.id).is_empty());
    }
    // A circle crossing an imprint and tangent to the wall's top edge.
    for pick in [false, true] {
        let mut w = front_wall();
        let wall = w.face;
        let s = imprint_mm(&mut w, wall, 3.0, 6.0, 9.0, 2.0);
        let ring = w.ring(8.0 * MM, 8.5 * MM, 1.5 * MM, 24, 0.0);
        let seed = if pick { s } else { w.face };
        draw_and_check(&mut w, seed, ring, None);
        assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
        assert!(has_touching_loops(w.obj(), &w.plane));
        assert!(w.doc.object_owned_sketches(w.id).is_empty());
    }
    // Two imprints sharing only a corner with the drawn rectangle's sides:
    // three holes of the wall meeting at two points.
    let (mut w, _, _) = wall_with_two();
    let wall = w.face;
    let rect = w.rect(7.0 * MM, 5.0 * MM, 2.0 * MM, 1.0 * MM);
    draw_and_check(&mut w, wall, rect, None);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 4);
    assert!(has_touching_loops(w.obj(), &w.plane));
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
}

// --------------------------------------------- merges along several runs

#[test]
fn deleting_a_band_meeting_the_hole_along_two_runs_splits_the_hole() {
    let mut w = top_of_box();
    let seed = w.face;
    let s = imprint_mm(&mut w, seed, 10.0, 10.0, 10.0, 10.0);
    let p = w.face;
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
    let e = edge_at(w.obj(), mm(&w, 20.0, 13.0), mm(&w, 20.0, 17.0));
    assert_chord_feature(&w, e);
    let r = merge_runs_and_check(&mut w, e);
    assert_eq!(r.merged_face, p, "the face holding the hole survives");
    let obj = w.obj();
    assert_eq!(obj.faces()[p].inner_loops.len(), 2, "the hole split in two");
    assert_eq!(group_faces(obj, &w.plane).len(), 3);
    assert!(group_faces_follow_the_loop_rule(obj, &w.plane));
    // Each half of the square is an imprint of its own again.
    let subs = group_faces(obj, &w.plane)
        .into_iter()
        .filter(|&f| f != p && obj.faces()[f].inner_loops.is_empty())
        .count();
    assert_eq!(subs, 2);
    push_pull_every_face_and_undo(&mut w, 3);
}

#[test]
fn deleting_a_line_two_faces_share_in_two_runs_leaves_a_hole() {
    // The top split in two by a chord at y = 15, then a square drawn across
    // the chord: the two halves now meet along the chord left AND right of
    // the square. Deleting either part merges them across both; the square
    // becomes a hole of the merged face.
    for left in [true, false] {
        let mut w = top_of_box();
        let p = w.face;
        w.doc
            .apply_object_op(
                w.id,
                split_op(p, vec![mm(&w, 0.0, 15.0), mm(&w, 30.0, 15.0)]),
            )
            .expect("the chord");
        let lower = face_at(&w, 5.0 * MM, 5.0 * MM);
        let square = w.rect(10.0 * MM, 10.0 * MM, 10.0 * MM, 10.0 * MM);
        w.draw_loop(lower, &square, None)
            .expect("the square across the chord");
        let e = if left {
            edge_between(w.obj(), mm(&w, 0.0, 15.0), mm(&w, 10.0, 15.0))
        } else {
            edge_between(w.obj(), mm(&w, 20.0, 15.0), mm(&w, 30.0, 15.0))
        };
        assert_chord_feature(&w, e);
        let r = merge_runs_and_check(&mut w, e);
        let obj = w.obj();
        assert_eq!(obj.faces()[r.merged_face].inner_loops.len(), 1, "a hole");
        assert_eq!(group_faces(obj, &w.plane).len(), 3);
        push_pull_every_face_and_undo(&mut w, 3);
    }
}

#[test]
fn a_multi_run_merge_restores_both_sides_paint_on_undo() {
    let mut w = top_of_box();
    let p = w.face;
    w.doc
        .apply_object_op(
            w.id,
            split_op(p, vec![mm(&w, 0.0, 15.0), mm(&w, 30.0, 15.0)]),
        )
        .expect("the chord");
    let lower = face_at(&w, 5.0 * MM, 5.0 * MM);
    let square = w.rect(10.0 * MM, 10.0 * MM, 10.0 * MM, 10.0 * MM);
    w.draw_loop(lower, &square, None)
        .expect("the square across the chord");
    let e = edge_between(w.obj(), mm(&w, 0.0, 15.0), mm(&w, 10.0, 15.0));
    let mut o = w.obj().clone();
    let r = o.merge_faces(e).expect("merges across both runs");
    assert!(r.prior_attrs.iter().all(Option::is_some));
    // Both chord parts dissolve, and their outer ends, left straight
    // through on the box's edges, heal away with their edges.
    assert_eq!(r.removed_edges.len(), 4);
    o.validate().expect("valid");
}

// ------------------------------------------------- the lattice property

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

/// Two random rectangle imprints and a random 24-gon on a 0.1 mm lattice
/// (turned 0° for exact tangencies at extreme vertices, or 7.5° / 4.5° for
/// crossings), biased toward circles that reach both imprints or an
/// imprint and the wall's edge, drawn from the wall or either imprint.
/// The ground sketch of the same lines is the oracle: every draw is
/// accepted wholly as topology, with I1–I7 and ground parity — a loop
/// meeting the group at single sites (F1) included, its face holding holes
/// that share those vertices exactly as the ground's regions do.
///
/// Draws with contacts in the band between the merge tolerance and 0.1 µm
/// (where the ground's parameter tolerance and the face's metric one may
/// disagree) are skipped.
#[test]
fn circles_over_two_imprints_on_a_lattice_match_the_ground() {
    let mut rng = Rng(0x2bd1_e5c4_09a7_7f13);
    let q = 0.1 * MM;
    let (mut accepted, mut bridged, mut touching) = (0, 0, 0);
    let mut draws = 0;
    while draws < 400 {
        let (ax, ay) = (rng.range(5, 60), rng.range(5, 50));
        let (aw, ah) = (rng.range(15, 40), rng.range(15, 40));
        let gap = rng.range(2, 30);
        let bx = ax + aw + gap;
        let (by, bw, bh) = (rng.range(5, 50), rng.range(15, 40), rng.range(15, 40));
        if bx + bw > 195 || ay + ah > 95 || by + bh > 95 {
            continue;
        }
        let r = rng.range(5, 40);
        let (mut cx, mut cy) = (ax + aw + gap / 2, rng.range(0, 100));
        match rng.next() % 6 {
            0 => cy = r,
            1 => cx = rng.range(0, 200),
            2 => cy = ay + ah / 2,
            _ => {}
        }
        if cx < r || cy < r || cx + r > 200 || cy + r > 100 {
            continue;
        }
        let phase = [0.0, PI / 24.0, PI / 40.0][(rng.next() % 3) as usize];
        let pick = rng.next() % 3;
        let f = |v: u32| f64::from(v) * q;
        let mut w = front_wall();
        let wall = w.face;
        let ra = w.rect(f(ax), f(ay), f(aw), f(ah));
        let rb = w.rect(f(bx), f(by), f(bw), f(bh));
        let a = draw_region(&mut w, wall, ra.clone(), None);
        let b = draw_region(&mut w, wall, rb.clone(), None);
        let ring = w.ring(f(cx), f(cy), f(r), 24, phase);
        let g = w.circle_geom(f(cx), f(cy), f(r));
        let outer = w.rect(0.0, 0.0, 20.0 * MM, 10.0 * MM);

        let before = w.obj().clone();
        let sk = ground_with(w.plane, &before, &ring);
        let group: Vec<Point3> = ra
            .iter()
            .chain(rb.iter())
            .chain(outer.iter())
            .copied()
            .collect();
        let band = |d: f64| d > tol::POINT_MERGE && d < 1e-7;
        let gv: Vec<Point3> = sk.vertices().values().map(|v| v.position).collect();
        let near_tie = ring.iter().any(|&p| {
            band(ring_dist(p, &ra)) || band(ring_dist(p, &rb)) || band(ring_dist(p, &outer))
        }) || group.iter().any(|&p| band(ring_dist(p, &ring)))
            || (0..gv.len()).any(|i| (i + 1..gv.len()).any(|j| (gv[i] - gv[j]).length() < 1e-7));
        if near_tie {
            continue;
        }
        draws += 1;
        let on_loop = |p: Point3| ring_dist(p, &ring) <= tol::POINT_MERGE;
        let sites_on = |rect: &[Point3]| {
            gv.iter()
                .filter(|&&p| on_loop(p) && ring_dist(p, rect) <= tol::POINT_MERGE)
                .count()
        };
        let (sa, sb, so) = (sites_on(&ra), sites_on(&rb), sites_on(&outer));
        assert!(!ground_has_pinch(&sk), "the ground's rings are simple");
        let joins = [sa > 0, sb > 0, so > 0].iter().filter(|&&x| x).count() >= 2;

        let seed = [wall, a, b][pick as usize];
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let what = format!(
            "a ({ax},{ay}) {aw}x{ah}, b ({bx},{by}) {bw}x{bh}, circle ({cx},{cy}) r {r} \
             phase {phase}: sites a {sa} b {sb} wall {so}"
        );
        match w.draw_loop(seed, &ring, Some(g)) {
            Ok((report, _)) => {
                assert!(
                    report.kept_as_sketch.is_empty(),
                    "the loop lands wholly as topology: {what}"
                );
                accepted += 1;
                if joins {
                    bridged += 1;
                }
                if has_touching_loops(w.obj(), &w.plane) {
                    touching += 1;
                }
                assert_ground_parity(&w, &before, &ring);
                assert_invariants(&mut w, &before, &bytes0, depth0);
            }
            Err(e) => panic!("the draft refused ({e:?}): {what}"),
        }
    }
    assert!(
        accepted > 150 && bridged > 40 && touching > 5,
        "coverage: {accepted} accepted ({bridged} joining two boundaries), \
         {touching} with touching holes"
    );
}

// ------------------------------------ transforms that cannot be reopened

#[test]
fn a_near_singular_squash_maps_the_solid_and_refuses_the_sketch_typed() {
    // A 1 km cube squashed by (1e6, 1e-9, 1e-8): invertible (det 1e-11)
    // and every face's ring keeps a reopenable area, but the inverse-
    // transpose map of an x-facing normal comes out 1e-17 long — that
    // plane has no direction. Each face takes the refit of its mapped ring
    // instead, which exists and holds its points: the squash is accepted
    // and valid. Only a face that neither plane fits is refused typed,
    // before any vertex moves, never a panic (`face_plane_specs::
    // a_plane_the_map_squashes_to_no_direction_falls_back_to_the_refit`).
    // (The squash (1e6, 1e-7, 1e-6) is kept either way.) A sketch on an
    // x-facing plane has no ring to fall back to, and the same squash
    // refuses it `Singular`, leaving it untouched.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        let c = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1000.0, 0.0, 0.0),
            Point3::new(1000.0, 1000.0, 0.0),
            Point3::new(0.0, 1000.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let region = doc.extrudable_regions(s).expect("regions")[0];
    let (id, _) = doc.extrude_region(s, region, 1000.0).expect("extrude");
    let xf = Transform::scale(Vec3::new(1e6, 1e-9, 1e-8));
    let obj = doc.object(id).expect("object").clone();
    let mut mapped = obj.clone();
    mapped
        .apply_transform(&xf)
        .expect("derived planes map through their rings");
    mapped.validate().expect("valid");
    let kept = Transform::scale(Vec3::new(1e6, 1e-7, 1e-6));
    let mut squashed = obj.clone();
    squashed.apply_transform(&kept).expect("kept now");
    squashed.validate().expect("valid");
    let bytes = squashed.encode(&|_| 0);
    let back = Object::decode(&bytes, &|_| None).expect("reopens");
    assert_eq!(back.encode(&|_| 0), bytes, "reopens identically");
    let mut sk = Sketch::on_plane(
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 0.0, 1.0),
        ])
        .expect("an x-facing plane"),
    );
    sk.add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 1000.0, 0.0))
        .expect("segment");
    let plane = sk.plane();
    let mut sk_kept = sk.clone();
    assert_eq!(sk.apply_transform(&xf), Err(TransformError::Singular));
    assert_eq!(sk.plane(), plane, "the sketch is untouched");
    sk_kept.apply_transform(&kept).expect("kept now");
}

/// A document with one 10 mm cube.
fn cube_doc() -> (Document, kernel::ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        let c = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0 * MM, 0.0, 0.0),
            Point3::new(10.0 * MM, 10.0 * MM, 0.0),
            Point3::new(0.0, 10.0 * MM, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let region = doc.extrudable_regions(s).expect("regions")[0];
    let (id, _) = doc.extrude_region(s, region, 10.0 * MM).expect("extrude");
    (doc, id)
}

#[test]
fn rescaling_the_document_below_the_reopen_limit_refuses_untouched() {
    // At 1e-5 the cube's 10 mm faces would be 0.1 µm squares — twice their
    // area 2e-14 m², under what a saved file can recompute a plane from.
    let (mut doc, _) = cube_doc();
    let (bytes0, depth0) = (doc.save(), doc.undo_depth());
    assert_eq!(
        doc.rescale_document(1e-5).map(|_| ()),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert_eq!(doc.save(), bytes0, "the document is untouched");
    assert_eq!(doc.undo_depth(), depth0);
    // A rescale that fits commits, and its undo and redo are exact.
    doc.rescale_document(0.5).expect("halving fits");
    let bytes1 = doc.save();
    doc.undo().expect("undo");
    assert_eq!(doc.save(), bytes0);
    doc.redo().expect("redo the rescale");
    assert_eq!(doc.save(), bytes1);
}

#[test]
fn rescaling_with_a_big_and_a_tiny_object_refuses_before_scaling_either() {
    let (mut doc, _) = cube_doc();
    // A 10 m cube: at 1e-5 it is 0.1 mm, fine — but the 10 mm cube is not.
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        let c = [
            Point3::new(100.0, 0.0, 0.0),
            Point3::new(110.0, 0.0, 0.0),
            Point3::new(110.0, 10.0, 0.0),
            Point3::new(100.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let region = doc.extrudable_regions(s).expect("regions")[0];
    doc.extrude_region(s, region, 10.0).expect("extrude");
    let bytes0 = doc.save();
    assert!(doc.rescale_document(1e-5).is_err());
    assert_eq!(doc.save(), bytes0, "nothing was scaled");
}

#[test]
fn rescaling_a_session_below_the_reopen_limit_refuses_untouched() {
    let (mut doc, id) = cube_doc();
    let (group, _) = doc.group_nodes(&[NodeId::Object(id)]).expect("group");
    doc.open_group_session(group).expect("open");
    let (bytes0, depth0) = (doc.save(), doc.undo_depth());
    assert_eq!(
        doc.rescale_session(1e-5, Point3::new(0.0, 0.0, 0.0))
            .map(|_| ()),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert_eq!(doc.save(), bytes0, "the document is untouched");
    assert_eq!(doc.undo_depth(), depth0);
    doc.rescale_session(2.0, Point3::new(0.0, 0.0, 0.0))
        .expect("doubling fits");
    doc.undo().expect("undo");
    assert_eq!(doc.save(), bytes0);
    doc.redo().expect("redo");
}

#[test]
fn closing_a_session_whose_unbake_would_vanish_a_face_refuses_typed() {
    // A component instance scaled up a thousandfold; inside its session a
    // 1 µm cube is drawn. Unbaked into the definition it would be 1 nm —
    // too small for a file to reopen — so the close refuses typed, with
    // the session still open, rather than panicking.
    let (mut doc, id) = cube_doc();
    let (_, inst) = doc
        .make_component(&[NodeId::Object(id)])
        .map(|(c, i, _)| (c, i))
        .expect("component");
    doc.transform_selection(
        &[NodeId::Instance(inst)],
        &[],
        &Transform::uniform_scale(1000.0),
    )
    .expect("scale the instance");
    doc.open_explode_session(inst).expect("open the session");
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        let um = 1e-6;
        let c = [
            Point3::new(50.0, 50.0, 0.0),
            Point3::new(50.0 + um, 50.0, 0.0),
            Point3::new(50.0 + um, 50.0 + um, 0.0),
            Point3::new(50.0, 50.0 + um, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
        }
    }
    let region = doc.extrudable_regions(s).expect("regions")[0];
    doc.extrude_region(s, region, 1e-6).expect("a 1 µm cube");
    let bytes0 = doc.save();
    assert_eq!(
        doc.close_explode_session().map(|_| ()),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert_eq!(doc.save(), bytes0, "untouched, the session still open");
    // A save mid-session serializes the "as if closed" view without
    // panicking.
    assert!(!doc.save_for_persistence().is_empty());
}

/// A plain path for a re-trace.
fn plain(path: Vec<Point3>) -> RetracePath {
    RetracePath {
        path,
        curves: Vec::new(),
        polygons: Vec::new(),
    }
}

/// One re-trace through the history, checked as [`apply_and_check`] does
/// (undo and redo exact, three rounds).
fn retrace_and_check(
    w: &mut Wall,
    face: FaceId,
    insert: Vec<Vec<Point3>>,
    remove: Vec<Vec<Point3>>,
) {
    apply_and_check(
        w,
        KernelOp::RetraceFace {
            face,
            insert: insert.into_iter().map(plain).collect(),
            remove,
            restore: Vec::new(),
            keep_vertices: Vec::new(),
        },
    );
}

/// The wall's diagonal from `(0, 0)` to `(20, 10)` mm drawn as two
/// collinear edges meeting at the point `t` of the way along it; the
/// joint.
fn diagonal_with_joint(w: &mut Wall, t: f64) -> Point3 {
    let p = w.face;
    let (a, b) = (mm(w, 0.0, 0.0), mm(w, 20.0, 10.0));
    let joint = mm(w, 20.0 * t, 10.0 * t);
    retrace_and_check(w, p, vec![vec![a, joint, b]], Vec::new());
    joint
}

/// `path` drawn on `face`: a re-trace, or with `split` a face split.
fn draw_line(w: &mut Wall, face: FaceId, path: Vec<Point3>, split: bool) {
    if split {
        apply_and_check(w, split_op(face, path));
    } else {
        retrace_and_check(w, face, vec![path], Vec::new());
    }
}

/// The front wall's diagonal with a joint `t` of the way along it, then a
/// line from that joint to the top edge at `x` mm, splitting it: a
/// re-trace, or with `split` a face split. Each through the history, undo
/// and redo exact (I5).
fn line_from_a_collinear_joint(t: f64, x: f64, split: bool) {
    line_from_a_collinear_joint_on(front_wall(), t, x, split);
}

fn line_from_a_collinear_joint_on(mut w: Wall, t: f64, x: f64, split: bool) {
    let joint = diagonal_with_joint(&mut w, t);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 2);
    let upper = face_at(&w, 1.0 * MM, 9.0 * MM);
    let top = mm(&w, x, 10.0);
    draw_line(&mut w, upper, vec![joint, top], split);
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
}

/// The wall carries an imprint (a hole with a face in it) below the
/// diagonal; the line runs from the diagonal's joint down to the bottom
/// edge, in the face holding the hole.
fn joint_on_a_holed_face(split: bool) {
    let mut w = front_wall();
    let wall = w.face;
    imprint_mm(&mut w, wall, 14.0, 1.0, 4.0, 2.0);
    let joint = diagonal_with_joint(&mut w, 0.5);
    let lower = face_at(&w, 19.0 * MM, 1.0 * MM);
    assert_eq!(
        w.obj().faces()[lower].inner_loops.len(),
        1,
        "it holds the hole"
    );
    let bottom = mm(&w, 10.0, 0.0);
    draw_line(&mut w, lower, vec![joint, bottom], split);
}

#[test]
fn undoing_a_line_from_a_joint_on_a_holed_face_keeps_the_joint() {
    joint_on_a_holed_face(false);
}

#[test]
fn undoing_a_split_from_a_joint_on_a_holed_face_keeps_the_joint() {
    joint_on_a_holed_face(true);
}

/// An imprint whose bottom edge runs as two collinear edges meeting at
/// `(6, 2)` mm: the joint lies on the hole's own boundary.
fn wall_with_a_jointed_hole() -> (Wall, Point3) {
    let mut w = front_wall();
    let wall = w.face;
    let ring = vec![
        mm(&w, 4.0, 2.0),
        mm(&w, 6.0, 2.0),
        mm(&w, 8.0, 2.0),
        mm(&w, 8.0, 5.0),
        mm(&w, 4.0, 5.0),
    ];
    apply_and_check(
        &mut w,
        KernelOp::SplitFaceInner {
            face: wall,
            loop_path: ring,
            restore: None,
            curves: Vec::new(),
            edge_attrs: Vec::new(),
        },
    );
    let joint = mm(&w, 6.0, 2.0);
    (w, joint)
}

/// Two lines from the hole's boundary down to the outer edge, one from the
/// joint, cutting off the strip between them in one re-trace: undo keeps
/// the joint on the hole.
#[test]
fn undoing_lines_from_a_joint_on_a_hole_keeps_the_joint() {
    let (mut w, joint) = wall_with_a_jointed_hole();
    let wall = w.face;
    let paths = vec![
        vec![joint, mm(&w, 6.0, 0.0)],
        vec![mm(&w, 8.0, 3.0), mm(&w, 10.0, 0.0)],
    ];
    retrace_and_check(&mut w, wall, paths, Vec::new());
}

/// A cut from the joint on the hole back to the same hole, around the
/// outside of it (a hole-anchored split): its undo merge keeps the joint.
#[test]
fn undoing_a_split_from_a_joint_on_a_hole_keeps_the_joint() {
    let (mut w, joint) = wall_with_a_jointed_hole();
    let wall = w.face;
    let path = vec![
        joint,
        mm(&w, 6.0, 1.0),
        mm(&w, 9.0, 1.0),
        mm(&w, 9.0, 3.0),
        mm(&w, 8.0, 3.0),
    ];
    draw_line(&mut w, wall, path, true);
}

/// The same line on a wall tilted about two axes, where no coordinate is
/// exact.
#[test]
fn undoing_a_line_from_a_joint_on_a_tilted_wall_keeps_the_joint() {
    line_from_a_collinear_joint_on(tilted_wall(), 0.5, 10.0, false);
    line_from_a_collinear_joint_on(tilted_wall(), 0.3, 7.0, true);
}

/// Both ends on vertices already there: the joint, and the wall's top-left
/// corner. Undo keeps both.
fn joint_to_a_corner(split: bool) {
    let mut w = front_wall();
    let joint = diagonal_with_joint(&mut w, 0.5);
    let upper = face_at(&w, 1.0 * MM, 9.0 * MM);
    let corner = mm(&w, 0.0, 10.0);
    draw_line(&mut w, upper, vec![joint, corner], split);
}

#[test]
fn undoing_a_line_from_a_joint_to_a_corner_keeps_both() {
    joint_to_a_corner(false);
}

#[test]
fn undoing_a_split_from_a_joint_to_a_corner_keeps_both() {
    joint_to_a_corner(true);
}

/// One re-trace that removes a line ending at the joint and inserts
/// another from the same joint: undo puts the first line back and keeps
/// the joint.
#[test]
fn undoing_a_retrace_that_swaps_lines_at_a_joint_keeps_it() {
    let mut w = front_wall();
    let joint = diagonal_with_joint(&mut w, 0.5);
    let upper = face_at(&w, 1.0 * MM, 9.0 * MM);
    let top = mm(&w, 10.0, 10.0);
    draw_line(&mut w, upper, vec![joint, top], false);
    let left = face_at(&w, 1.0 * MM, 9.0 * MM);
    let corner = mm(&w, 0.0, 10.0);
    retrace_and_check(
        &mut w,
        left,
        vec![vec![joint, corner]],
        vec![vec![joint, top]],
    );
}

/// A line drawn from a vertex that already sat mid-way along a straight
/// run of two collinear edges (a diagonal split at a point other geometry
/// met) undoes exactly: the removal heals away only the point the line's
/// other end split into the boundary, never the vertex it started from,
/// so the diagonal comes back as the two edges it was. Redo draws it again.
#[test]
fn undoing_a_line_from_a_collinear_vertex_keeps_that_vertex() {
    line_from_a_collinear_joint(0.5, 10.0, false);
}

/// The same line drawn as a face split: its undo merge keeps the joint the
/// cut started from, and only heals the point the cut split into the top
/// edge.
#[test]
fn undoing_a_split_from_a_collinear_vertex_keeps_that_vertex() {
    line_from_a_collinear_joint(0.5, 10.0, true);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    /// Rule 3 for the same: wherever the joint sits on the diagonal and
    /// wherever the line meets the top edge, undo gives the joint back.
    #[test]
    fn undoing_a_line_from_any_collinear_joint_keeps_it(
        t in 0.1f64..0.9,
        x in 1.0f64..19.0,
    ) {
        line_from_a_collinear_joint(t, x, false);
    }

    /// Rule 3 for the face split: wherever the joint and the far end sit,
    /// the undo merge gives the joint back.
    #[test]
    fn undoing_a_split_from_any_collinear_joint_keeps_it(
        t in 0.1f64..0.9,
        x in 1.0f64..19.0,
    ) {
        line_from_a_collinear_joint(t, x, true);
    }
}
