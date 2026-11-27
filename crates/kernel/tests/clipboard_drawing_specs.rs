//! Executable specs for **copy and paste of drawing** (v1.5 decisions D3,
//! KB-46): a clipboard item carries the copied drawing as its world
//! sketches ([`Document::extract_clipboard_item_with_drawing`]), and a paste
//! of drawing is a draw at the destination
//! ([`InsertOptions::drawing`] = [`DrawingInsert::Draw`]): it goes through
//! the drafting engine on the plane it lands on, so a shape pasted onto a
//! solid's face imprints it, a shape pasted over other drawing arranges
//! with it, and a shape pasted in space is a free sketch. The test of the
//! rule is that paste gives what Move with the copy modifier gives for the
//! same selection and the same displacement.
//!
//! Coordinates below are dyadic (exact in binary), so the item's re-origin
//! and the paste's pose cancel exactly and geometry keys compare bit for
//! bit with a Move+copy of the same displacement.

use kernel::trig;
use std::collections::BTreeSet;

use kernel::{
    CurveGeom, Document, DocumentError, DrawingInsert, DrawnSource, InsertOptions, KernelOp,
    NodeId, ObjectId, Plane, Point3, SketchCurveKind, SketchId, Transform, Vec3,
};
use proptest::prelude::*;

// ----------------------------------------------------------------- helpers

const ORIGIN: &str = "window-a#0";

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane_z(z: f64) -> Plane {
    Plane::from_point_normal(p(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).expect("plane")
}

fn ground() -> Plane {
    plane_z(0.0)
}

fn shift(x: f64, y: f64, z: f64) -> Transform {
    Transform::translation(Vec3::new(x, y, z))
}

fn rect(
    w: &mut kernel::Sketch,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    z: f64,
) -> Result<(), kernel::SketchError> {
    let c = [p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)];
    for i in 0..4 {
        w.add_segment(c[i], c[(i + 1) % 4])?;
    }
    Ok(())
}

/// Draws a free rectangle on `plane_z(z)` through the drafting engine;
/// returns the sketch it lowered into.
fn draw_rect(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> SketchId {
    let plane = plane_z(z);
    let parts = doc.draft_participants(plane, None);
    let (report, _) = doc
        .draft(plane, None, &parts, "Rectangle", |w| {
            rect(w, x0, y0, x1, y1, z)
        })
        .expect("draw a rectangle");
    *report.sketches.last().expect("a sketch was written")
}

/// An axis-aligned box `[x0,x1] x [y0,y1] x [0,h]`.
fn build_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).expect("sketch is live");
    rect(sk, x0, y0, x1, y1, 0.0).expect("rectangle");
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    doc.extrude_region(s, region, h).expect("extrude").0
}

/// A plane's drawing as compared across documents: every unit's edge keys
/// and curve-run keys and kinds, and every region's key and hole count.
type Listing = (Vec<Vec<u64>>, Vec<(u64, String)>, Vec<(u64, usize)>);

fn listing(doc: &Document, plane: Plane) -> Listing {
    let objects = doc.draft_participants(plane, None);
    let units = doc.plane_units(plane, None, &objects).expect("listing");
    let mut edges: Vec<Vec<u64>> = units
        .units
        .iter()
        .map(|u| {
            let mut k: Vec<u64> = u.edges.iter().map(|e| e.key).collect();
            k.sort_unstable();
            k
        })
        .collect();
    edges.sort();
    let mut curves: Vec<(u64, String)> = units
        .units
        .iter()
        .flat_map(|u| u.curves.iter())
        .map(|c| (c.key, format!("{:?}", c.kind)))
        .collect();
    curves.sort();
    let mut regions: Vec<(u64, usize)> = units
        .regions
        .iter()
        .map(|r| (r.key, r.holes.len()))
        .collect();
    regions.sort();
    (edges, curves, regions)
}

/// The listing keys of every drawing edge on `plane` (all units).
fn all_keys(doc: &Document, plane: Plane) -> Vec<u64> {
    let objects = doc.draft_participants(plane, None);
    doc.plane_units(plane, None, &objects)
        .expect("listing")
        .units
        .iter()
        .flat_map(|u| u.edges.iter().map(|e| e.key))
        .collect()
}

/// Copy the drawing `drawing` (and the nodes `roots`) to the clipboard and
/// paste it back displaced by `d` from where it was copied.
fn copy_paste(
    doc: &mut Document,
    roots: &[NodeId],
    drawing: &[DrawnSource],
    d: Vec3,
) -> Result<(kernel::InsertReport, kernel::DocChange), DocumentError> {
    let item = doc.extract_clipboard_item_with_drawing(roots, drawing, ORIGIN)?;
    let placement = doc
        .extract_placement_with_drawing(roots, drawing)
        .expect("a placement");
    doc.insert_document(
        &item,
        &InsertOptions {
            pose: placement.then(&Transform::translation(d)),
            provenance: None,
            origin: Some(ORIGIN.to_string()),
            drawing: DrawingInsert::Draw,
            hidden_objects: BTreeSet::new(),
        },
    )
}

/// The plane-keys source naming every drawing edge on `plane`.
fn whole_plane(doc: &Document, plane: Plane) -> DrawnSource {
    DrawnSource::PlaneKeys {
        plane,
        frame: None,
        objects: doc.draft_participants(plane, None),
        keys: all_keys(doc, plane),
    }
}

/// Move+copy of every drawing edge on `plane` by `t` (the app's
/// `draft_copy_edges`): one draft, the working sketch's drawing edges
/// copied through `t`.
fn move_copy(doc: &mut Document, plane: Plane, t: Transform) {
    let objects = doc.draft_participants(plane, None);
    let keys = all_keys(doc, plane);
    doc.begin_draft(plane, None, &objects, "Copy")
        .expect("open");
    let edges = doc.draft_edges_by_key(&keys);
    doc.draft_sketch_mut()
        .expect("open draft")
        .copy_edges_transformed(&edges, &t)
        .expect("copy edges");
    doc.end_draft().expect("lower");
}

/// The live free sketches lying on the horizontal plane `plane`.
fn free_sketches_on(doc: &Document, plane: Plane) -> Vec<SketchId> {
    let d = plane.signed_distance(Point3::ORIGIN);
    doc.sketch_ids()
        .into_iter()
        .filter(|&s| {
            doc.sketch_owner_object(s).is_none()
                && doc.sketch(s).is_some_and(|sk| {
                    (sk.plane().signed_distance(Point3::ORIGIN) - d).abs() < 1e-9
                        && sk.plane().normal().z > 0.5
                })
        })
        .collect()
}

// --------------------------------------------------------------- spec 1

/// Copy a free rectangle and paste it translated in its plane: the
/// document's drawing equals the one `draft_copy_edges` makes for the same
/// displacement (same edges, regions, curve identity).
#[test]
fn a_pasted_rectangle_equals_move_copy_in_plane() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 0.5, 0.5, 1.5, 1.25, 0.0);
    let mut expected = doc.clone();
    move_copy(&mut expected, ground(), shift(3.0, 0.5, 0.0));

    let src = whole_plane(&doc, ground());
    let (report, _) = copy_paste(&mut doc, &[], &[src], Vec3::new(3.0, 0.5, 0.0)).expect("paste");

    assert_eq!(listing(&doc, ground()), listing(&expected, ground()));
    assert_eq!(report.drawn.len(), 1, "one pasted plane");
    assert_eq!(report.drawn[0].detached, None);
    assert!(!report.drawing_unchanged);
    assert!(report.roots.is_empty());
}

// --------------------------------------------------------------- spec 2

/// Paste a rectangle onto a box's top face (coplanar): the face is split,
/// and the new sub-face push/pulls.
#[test]
fn a_rectangle_pasted_on_a_face_imprints_it() {
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0, 4.0, 4.0, 1.0);
    // A free rectangle at the box's top height, off the box.
    draw_rect(&mut doc, 8.0, 0.0, 9.0, 1.0, 1.0);
    let faces_before = doc.object(b).unwrap().faces().len();
    let src = DrawnSource::PlaneKeys {
        plane: plane_z(1.0),
        frame: None,
        objects: Vec::new(),
        keys: all_keys(&doc, plane_z(1.0)),
    };
    // Onto the top face, at x 1..2.
    copy_paste(&mut doc, &[], &[src], Vec3::new(-7.0, 1.0, 0.0)).expect("paste");

    let obj = doc.object(b).unwrap();
    assert_eq!(obj.faces().len(), faces_before + 1, "the face is split");
    let sub = obj
        .faces()
        .iter()
        .find(|(f, face)| {
            face.plane.normal().z > 0.5 && {
                let ring: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
                let _ = f;
                ring.len() == 4 && ring.iter().all(|q| q.x >= 1.0 - 1e-9 && q.x <= 2.0 + 1e-9)
            }
        })
        .map(|(f, _)| f)
        .expect("the pasted sub-face");
    doc.apply_object_op(
        b,
        KernelOp::PushPull {
            face: sub,
            distance: 0.5,
        },
    )
    .expect("the sub-face push/pulls");
}

// --------------------------------------------------------------- spec 3

/// Paste off any face: one new free sketch on the translated plane, with
/// one region.
#[test]
fn a_rectangle_pasted_in_space_is_a_new_free_sketch() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0);
    let src = whole_plane(&doc, ground());
    let before: BTreeSet<SketchId> = doc.sketch_ids().into_iter().collect();
    let (report, _) = copy_paste(&mut doc, &[], &[src], Vec3::new(0.0, 0.0, 3.0)).expect("paste");
    let new: Vec<SketchId> = doc
        .sketch_ids()
        .into_iter()
        .filter(|s| !before.contains(s))
        .collect();
    assert_eq!(new.len(), 1, "one new sketch");
    let sk = doc.sketch(new[0]).unwrap();
    assert!((sk.plane().signed_distance(Point3::ORIGIN) + 3.0).abs() < 1e-9);
    assert_eq!(sk.regions().len(), 1);
    assert!(!sk.detached());
    assert_eq!(doc.sketch_owner_object(new[0]), None);
    assert!(
        (report.drawn[0].plane.signed_distance(Point3::ORIGIN) + 3.0).abs() < 1e-9,
        "the report names the posed plane"
    );
    assert_eq!(report.drawn[0].anchors.len(), 4);
}

// --------------------------------------------------------------- spec 4

/// A circle and a polygon keep their analytic identity through copy and
/// paste.
#[test]
fn circles_and_polygons_keep_their_identity() {
    let mut doc = Document::new();
    let n = 24usize;
    let ring = |cx: f64, cy: f64, r: f64, n: usize| -> Vec<Point3> {
        (0..=n)
            .map(|i| {
                let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                p(cx + r * trig::cos(a), cy + r * trig::sin(a), 0.0)
            })
            .collect()
    };
    let parts = doc.draft_participants(ground(), None);
    doc.draft(ground(), None, &parts, "Shapes", |w| {
        w.begin_curve_with_kind(
            CurveGeom {
                center: p(1.0, 1.0, 0.0),
                radius: 0.5,
            },
            SketchCurveKind::Circle,
        )?;
        for s in ring(1.0, 1.0, 0.5, n).windows(2) {
            w.add_segment(s[0], s[1])?;
        }
        w.end_curve();
        w.begin_curve_with_kind(
            CurveGeom {
                center: p(4.0, 1.0, 0.0),
                radius: 0.5,
            },
            SketchCurveKind::Polygon,
        )?;
        for s in ring(4.0, 1.0, 0.5, 6).windows(2) {
            w.add_segment(s[0], s[1])?;
        }
        w.end_curve();
        Ok(())
    })
    .expect("draw");
    let src = whole_plane(&doc, ground());
    copy_paste(&mut doc, &[], &[src], Vec3::new(0.0, 0.0, 2.0)).expect("paste");

    let sketches = free_sketches_on(&doc, plane_z(2.0));
    assert_eq!(sketches.len(), 1);
    let sk = doc.sketch(sketches[0]).unwrap();
    let mut found: Vec<(SketchCurveKind, Point3, f64)> = sk
        .edges()
        .values()
        .filter_map(|e| e.curve)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|c| sk.curve_analytic(c))
        .map(|a| (a.kind, a.geom.center, a.geom.radius))
        .collect();
    found.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    assert_eq!(found.len(), 2, "two analytic chains");
    assert_eq!(found[0].0, SketchCurveKind::Circle);
    assert!(found[0].1.approx_eq(p(1.0, 1.0, 2.0), 1e-9));
    assert!((found[0].2 - 0.5).abs() < 1e-12);
    assert_eq!(found[1].0, SketchCurveKind::Polygon);
    assert!(found[1].1.approx_eq(p(4.0, 1.0, 2.0), 1e-9));
}

// --------------------------------------------------------------- spec 5

/// A ring (a region with a hole) pastes as a ring, whether the copy names
/// the whole sketch or its plane's keys.
#[test]
fn a_ring_pastes_as_a_ring() {
    for whole in [true, false] {
        let mut doc = Document::new();
        let parts = doc.draft_participants(ground(), None);
        let (report, _) = doc
            .draft(ground(), None, &parts, "Ring", |w| {
                rect(w, 0.0, 0.0, 4.0, 4.0, 0.0)?;
                rect(w, 1.0, 1.0, 3.0, 3.0, 0.0)
            })
            .expect("draw");
        let s = report.sketches[0];
        let src = if whole {
            DrawnSource::Sketch(s)
        } else {
            whole_plane(&doc, ground())
        };
        copy_paste(&mut doc, &[], &[src], Vec3::new(0.0, 0.0, 2.0)).expect("paste");
        let (_, _, regions) = listing(&doc, plane_z(2.0));
        assert!(
            regions.iter().any(|&(_, holes)| holes == 1),
            "whole={whole}: a ring: {regions:?}"
        );
    }
}

// --------------------------------------------------------------- spec 6

/// A detached shape pastes as one new detached sketch and does not arrange
/// with a line it crosses.
#[test]
fn a_detached_shape_pastes_detached() {
    let mut doc = Document::new();
    let (report, _) = doc
        .draft_detached_new(ground(), None, "Detached", |w| {
            rect(w, 0.0, 0.0, 1.0, 1.0, 0.0)
        })
        .expect("draw detached");
    let detached = report.sketches[0];
    // A free line the pasted copy (at x 4..5) will cross.
    let parts = doc.draft_participants(ground(), None);
    doc.draft(ground(), None, &parts, "Line", |w| {
        w.add_segment(p(3.0, 0.5, 0.0), p(6.0, 0.5, 0.0))
            .map(|_| ())
    })
    .expect("line");
    let line_before = listing(&doc, ground());
    let islands: Vec<_> = doc.sketch(detached).unwrap().islands().keys().collect();
    let (report, _) = copy_paste(
        &mut doc,
        &[],
        &[DrawnSource::Islands(detached, islands)],
        Vec3::new(4.0, 0.0, 0.0),
    )
    .expect("paste");
    let copy = report.drawn[0].detached.expect("a new detached sketch");
    assert_ne!(copy, detached);
    let sk = doc.sketch(copy).unwrap();
    assert!(sk.detached());
    assert_eq!(sk.edges().len(), 4, "not split by the line");
    assert_eq!(sk.regions().len(), 1);
    assert_eq!(
        listing(&doc, ground()),
        line_before,
        "the line is not split"
    );
}

// --------------------------------------------------------------- spec 7

/// A mixed copy (a box and a rectangle beside it) pastes as ONE undo step:
/// undo restores the document, redo restores the paste.
#[test]
fn a_mixed_paste_is_one_undo_step() {
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    draw_rect(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0);
    let src = DrawnSource::PlaneKeys {
        plane: ground(),
        frame: None,
        objects: doc.draft_participants(ground(), None),
        keys: all_keys(&doc, ground()),
    };
    let before = doc.state_hash();
    let (report, _) = copy_paste(
        &mut doc,
        &[NodeId::Object(b)],
        &[src],
        Vec3::new(0.0, 4.0, 0.0),
    )
    .expect("paste");
    assert_eq!(report.roots.len(), 1);
    assert_eq!(report.drawn.len(), 1);
    let after = doc.state_hash();
    let entries = doc.history_entries();
    assert_eq!(entries.undo.last().expect("an entry").label, "Paste");
    doc.undo().expect("undo");
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
    doc.redo().expect("redo");
    assert_eq!(doc.state_hash(), after, "redo restores the paste");
}

// --------------------------------------------------------------- spec 8

/// A refusal in the drawing part (a pose that collapses a short line under
/// tolerance while the box survives it) leaves the document exactly as it
/// was, the pasted box included.
#[test]
fn a_refused_drawing_part_refuses_the_whole_paste() {
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let parts = doc.draft_participants(ground(), None);
    doc.draft(ground(), None, &parts, "Short line", |w| {
        w.add_segment(p(2.0, 0.0, 0.0), p(2.0 + 3e-9, 0.0, 0.0))
            .map(|_| ())
    })
    .expect("a short line");
    let src = whole_plane(&doc, ground());
    let item = doc
        .extract_clipboard_item_with_drawing(&[NodeId::Object(b)], &[src], ORIGIN)
        .expect("copy");
    let before = doc.state_hash();
    let undo_depth = doc.history_entries().undo.len();
    let squash = Transform::from_affine(&[
        0.25, 0.0, 0.0, 0.0, //
        0.0, 0.25, 0.0, 0.0, //
        0.0, 0.0, 0.25, 0.0,
    ]);
    let refused = doc.insert_document(
        &item,
        &InsertOptions {
            pose: squash,
            provenance: None,
            origin: Some(ORIGIN.to_string()),
            drawing: DrawingInsert::Draw,
            hidden_objects: BTreeSet::new(),
        },
    );
    assert!(
        matches!(refused, Err(DocumentError::Sketch(_))),
        "refused by the drawing part: {refused:?}"
    );
    assert_eq!(doc.state_hash(), before, "untouched, nodes included");
    assert_eq!(doc.history_entries().undo.len(), undo_depth);
}

// --------------------------------------------------------------- spec 9

/// A Library insert of an item holding world sketches still skips them and
/// counts them.
#[test]
fn a_library_insert_still_skips_world_sketches() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0);
    draw_rect(&mut doc, 0.0, 0.0, 1.0, 1.0, 2.0);
    let item = doc
        .extract_clipboard_item_with_drawing(
            &[],
            &[whole_plane(&doc, ground()), whole_plane(&doc, plane_z(2.0))],
            ORIGIN,
        )
        .expect("copy");
    let sketches_before = doc.sketch_ids().len();
    let (report, _) = doc
        .insert_document(
            &item,
            &InsertOptions {
                pose: Transform::IDENTITY,
                provenance: None,
                origin: None,
                drawing: DrawingInsert::Skip,
                hidden_objects: BTreeSet::new(),
            },
        )
        .expect("insert");
    assert_eq!(report.world_sketches_skipped, 2);
    assert!(report.drawn.is_empty());
    assert_eq!(doc.sketch_ids().len(), sketches_before);
}

// -------------------------------------------------------------- spec 10

/// Paste inside a group session: the pasted drawing is the group's after
/// the close. Inside a component session, copy and paste refuse.
#[test]
fn paste_in_a_group_session_folds_in_and_a_component_session_refuses() {
    // Group session.
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let s = draw_rect(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0);
    let item = doc
        .extract_clipboard_item_with_drawing(&[], &[DrawnSource::Sketch(s)], ORIGIN)
        .expect("copy");
    let placement = doc
        .extract_placement_with_drawing(&[], &[DrawnSource::Sketch(s)])
        .expect("placement");
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    doc.open_group_session(g).expect("open");
    let (report, _) = doc
        .insert_document(
            &item,
            &InsertOptions {
                pose: placement.then(&shift(0.0, 0.0, 3.0)),
                provenance: None,
                origin: Some(ORIGIN.to_string()),
                drawing: DrawingInsert::Draw,
                hidden_objects: BTreeSet::new(),
            },
        )
        .expect("paste mid-session");
    assert_eq!(report.drawn.len(), 1);
    let pasted = free_sketches_on(&doc, plane_z(3.0));
    assert_eq!(pasted.len(), 1);
    doc.close_innermost_session().expect("close");
    assert_eq!(
        doc.sketch_owner_group(pasted[0]),
        Some(g),
        "the pasted drawing is the group's"
    );

    // Component session.
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let s = draw_rect(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0);
    let item = doc
        .extract_clipboard_item_with_drawing(&[], &[DrawnSource::Sketch(s)], ORIGIN)
        .expect("copy");
    let (_, placed, _) = doc.make_component(&[NodeId::Object(a)]).expect("component");
    doc.open_explode_session(placed).expect("open");
    assert_eq!(
        doc.extract_clipboard_item_with_drawing(&[], &[DrawnSource::Sketch(s)], ORIGIN)
            .err(),
        Some(DocumentError::ExplodeSessionScope)
    );
    let before = doc.state_hash();
    let refused = doc.insert_document(
        &item,
        &InsertOptions {
            pose: Transform::IDENTITY,
            provenance: None,
            origin: Some(ORIGIN.to_string()),
            drawing: DrawingInsert::Draw,
            hidden_objects: BTreeSet::new(),
        },
    );
    assert_eq!(refused.err(), Some(DocumentError::ExplodeSessionScope));
    assert_eq!(doc.state_hash(), before);
}

// ----------------------------------------------- Paste In Place, no change

/// Paste In Place right after a copy draws nothing new: the report says so
/// and no undo step is recorded.
#[test]
fn paste_in_place_over_the_original_changes_nothing() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 0.5, 0.5, 1.5, 1.5, 0.0);
    let src = whole_plane(&doc, ground());
    let before = doc.state_hash();
    let depth = doc.history_entries().undo.len();
    let (report, _) = copy_paste(&mut doc, &[], &[src], Vec3::new(0.0, 0.0, 0.0)).expect("paste");
    assert!(report.drawing_unchanged);
    assert_eq!(doc.history_entries().undo.len(), depth, "no undo step");
    assert_eq!(doc.state_hash(), before);
}

/// A copy that names nothing is refused as an empty selection, and a stale
/// key is refused rather than dropped.
#[test]
fn empty_and_stale_copies_refuse() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0);
    assert_eq!(
        doc.extract_clipboard_item_with_drawing(&[], &[], ORIGIN)
            .err(),
        Some(DocumentError::EmptyComponent)
    );
    let stale = DrawnSource::PlaneKeys {
        plane: ground(),
        frame: None,
        objects: Vec::new(),
        keys: vec![0xdead_beef],
    };
    assert!(
        doc.extract_clipboard_item_with_drawing(&[], &[stale], ORIGIN)
            .is_err()
    );
}

/// Paste In Place's placement covers drawing: a drawing-only copy pasted
/// at its placement lands exactly over the original.
#[test]
fn the_placement_of_a_drawing_copy_is_its_bottom_center() {
    let mut doc = Document::new();
    draw_rect(&mut doc, 2.0, 4.0, 4.0, 6.0, 1.0);
    let src = whole_plane(&doc, plane_z(1.0));
    let placement = doc
        .extract_placement_with_drawing(&[], std::slice::from_ref(&src))
        .expect("placement");
    let a = placement.to_affine();
    assert_eq!((a[3], a[7], a[11]), (3.0, 5.0, 1.0));
    let item = doc
        .extract_clipboard_item_with_drawing(&[], &[src], ORIGIN)
        .expect("copy");
    let ids = item.sketch_ids();
    assert_eq!(ids.len(), 1);
    let sk = item.sketch(ids[0]).unwrap();
    assert!(
        sk.plane().signed_distance(Point3::ORIGIN).abs() < 1e-12,
        "re-origined to the bottom center"
    );
}

// -------------------------------------------------------------- spec 11

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// For a random free drawing and a random in-plane translation, copy
    /// then paste equals Move+copy.
    #[test]
    fn copy_paste_equals_move_copy(
        rects in prop::collection::vec((0i32..32, 0i32..32, 1i32..16, 1i32..16), 1..4),
        dx in -32i32..32,
        dy in -32i32..32,
    ) {
        let g = |v: i32| f64::from(v) / 8.0;
        let mut doc = Document::new();
        for &(x, y, w, h) in &rects {
            draw_rect(&mut doc, g(x), g(y), g(x + w), g(y + h), 0.0);
        }
        let mut expected = doc.clone();
        let moved = move_copy_ok(&mut expected, ground(), shift(g(dx), g(dy), 0.0));
        let src = whole_plane(&doc, ground());
        let pasted = copy_paste(&mut doc, &[], &[src], Vec3::new(g(dx), g(dy), 0.0));
        prop_assert_eq!(moved, pasted.is_ok());
        if moved {
            prop_assert_eq!(listing(&doc, ground()), listing(&expected, ground()));
        }
    }
}

/// [`move_copy`], answering whether the copy landed.
fn move_copy_ok(doc: &mut Document, plane: Plane, t: Transform) -> bool {
    let objects = doc.draft_participants(plane, None);
    let keys = all_keys(doc, plane);
    if doc.begin_draft(plane, None, &objects, "Copy").is_err() {
        return false;
    }
    let edges = doc.draft_edges_by_key(&keys);
    let copied = doc
        .draft_sketch_mut()
        .expect("open draft")
        .copy_edges_transformed(&edges, &t);
    if copied.is_err() {
        doc.cancel_draft();
        return false;
    }
    if doc.end_draft().is_err() {
        doc.cancel_draft();
        return false;
    }
    true
}
