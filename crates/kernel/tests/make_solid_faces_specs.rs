//! Executable specs for Make Solid on drawn faces and its overlay query
//! (wireframe faces and solids §6, §7), and for API-style sketch gestures
//! that close faces and solids (§7: the automatic rule applies to API
//! drawing as to pointer drawing).
//!
//! The contract under test:
//! - [`Document::make_solid_from_sketches`] runs the cells search seeded by
//!   every faced region of the named sketches; what closes is ONE undo step
//!   labeled "Make Solid"; a set that closes nothing declines with its open
//!   edges and non-planar loops and leaves the document untouched (nothing
//!   recorded); a stale sketch refuses typed.
//! - [`Document::faces_open_edges`] is read-only and lists exactly what such
//!   a decline is missing: open edges, and closed loops of open edges off
//!   one plane (an edge of such a loop is listed with the loop only).
//! - [`Document::end_sketch_gesture_closing_faces`] ends a raw gesture and
//!   runs the close pass (its own plane included) and the cells search in
//!   the gesture's undo step; plain [`Document::end_sketch_gesture`] does
//!   not.

use kernel::{
    CellDeclineReason, CellOutcome, Document, DocumentError, Object, Plane, Point3, SketchId, Vec3,
    WatertightState,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane(at: Point3, n: Vec3) -> Plane {
    Plane::from_point_normal(at, n).unwrap()
}

fn up(q: Point3, z: f64) -> Point3 {
    p(q.x, q.y, z)
}

const A: Point3 = Point3::new(0., 0., 0.);
const B: Point3 = Point3::new(1., 0., 0.);
const C: Point3 = Point3::new(1., 1., 0.);
const D: Point3 = Point3::new(0., 1., 0.);
const APEX: Point3 = Point3::new(0.5, 0.5, 1.);

/// A closed polygon as one raw sketch gesture (no close pass, no cells
/// search): how faces that would close on their own are set up open.
fn raw_face(doc: &mut Document, corners: &[Point3]) -> SketchId {
    let on = Plane::from_polygon(corners).unwrap();
    let s = doc.add_sketch(on);
    doc.begin_sketch_gesture(s).unwrap();
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..corners.len() {
        sk.add_segment(corners[i], corners[(i + 1) % corners.len()])
            .unwrap();
    }
    doc.end_sketch_gesture(s).unwrap();
    s
}

/// One line on its own new sketch on `on`, ended as the Hew API ends its
/// drawing gestures. Returns what it closed.
fn api_line(
    doc: &mut Document,
    on: Plane,
    a: Point3,
    b: Point3,
) -> (Vec<SketchId>, Vec<CellOutcome>) {
    let s = doc.add_sketch(on);
    doc.begin_sketch_gesture(s).unwrap();
    doc.sketch_mut(s).unwrap().add_segment(a, b).unwrap();
    let (_, closed, cells) = doc.end_sketch_gesture_closing_faces(s).unwrap();
    (closed, cells)
}

/// The ground rectangle A–B–C–D as one API-style gesture.
fn api_ground_rect(doc: &mut Document) -> SketchId {
    let s = doc.add_sketch(plane(A, Vec3::new(0., 0., 1.)));
    doc.begin_sketch_gesture(s).unwrap();
    let sk = doc.sketch_mut(s).unwrap();
    for (a, b) in [(A, B), (B, C), (C, D), (D, A)] {
        sk.add_segment(a, b).unwrap();
    }
    let (_, closed, cells) = doc.end_sketch_gesture_closing_faces(s).unwrap();
    assert!(closed.is_empty() && cells.is_empty());
    s
}

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

fn solid(doc: &Document, cells: &[CellOutcome], vol: f64) {
    let [CellOutcome::Created(id)] = cells else {
        panic!("expected one new solid, got {cells:?}")
    };
    let obj = doc.object(*id).expect("a visible object");
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj.validate().expect("the solid validates");
    assert!(
        (volume(obj) - vol).abs() <= 1e-9,
        "volume {} expected {vol}",
        volume(obj)
    );
}

fn reopens(doc: &Document) {
    let back = Document::load(&doc.save()).expect("the saved bytes reopen");
    assert_eq!(back.state_hash(), doc.state_hash(), "reopens unchanged");
}

fn one_step(doc: &mut Document, before: u64) {
    let after = doc.state_hash();
    doc.undo().expect("undo");
    assert_eq!(doc.state_hash(), before, "one undo restores the document");
    doc.redo().expect("redo");
    assert_eq!(doc.state_hash(), after, "redo returns the edit's state");
}

/// The floor and four walls of the unit cube, each its own raw sketch,
/// with the walls' top corners at `tops` (A', B', C', D').
fn open_box(doc: &mut Document, tops: [Point3; 4]) -> Vec<SketchId> {
    let [a1, b1, c1, d1] = tops;
    vec![
        raw_face(doc, &[A, B, C, D]),
        raw_face(doc, &[A, B, b1, a1]),
        raw_face(doc, &[B, C, c1, b1]),
        raw_face(doc, &[C, D, d1, c1]),
        raw_face(doc, &[D, A, a1, d1]),
    ]
}

// ------------------------------------------------------------ Make Solid

/// §11 stage 5 / §15 step 7, as §16 R26/R27 amend it: Make Solid first
/// fills the faces the selected lines can close. Five faces of a cube
/// (floor and four walls, raw: no pass ran) close their lid on the way and
/// make the cube as one "Make Solid" step, every face's sketch consumed. A
/// floor and three walls close nothing: they decline untouched and
/// unrecorded, the gap's rim one loop of six corners on no one plane; with
/// the fourth wall drawn raw, Make Solid from one wall closes the cube.
#[test]
fn five_faces_of_a_cube_close_their_lid_and_four_decline() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let top = [up(A, 1.), up(B, 1.), up(C, 1.), up(D, 1.)];
    let five = open_box(&mut doc, top);
    let before = doc.state_hash();
    let (out, change) = doc.make_solid_from_sketches(&five).unwrap();
    solid(&doc, &out, 1.0);
    assert!(!change.objects_touched.is_empty());
    assert_eq!(
        doc.history_entries().undo.last().unwrap().label,
        "Make Solid"
    );
    assert!(
        doc.sketch_ids().is_empty(),
        "every face's sketch is consumed, the filled lid's too"
    );
    reopens(&doc);
    one_step(&mut doc, before);

    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let [a1, b1, c1, d1] = top;
    let four = vec![
        raw_face(&mut doc, &[A, B, C, D]),
        raw_face(&mut doc, &[A, B, b1, a1]),
        raw_face(&mut doc, &[B, C, c1, b1]),
        raw_face(&mut doc, &[C, D, d1, c1]),
    ];
    let before = doc.state_hash();
    let depth = doc.history_entries().undo.len();

    let (out, change) = doc.make_solid_from_sketches(&four).unwrap();
    let [CellOutcome::Declined(d)] = &out[..] else {
        panic!("{out:?}")
    };
    assert_eq!(d.reason, CellDeclineReason::NotClosed);
    assert_eq!(d.non_planar_loops.len(), 1, "{:?}", d.non_planar_loops);
    assert_eq!(d.non_planar_loops[0].len(), 6);
    assert!(change.objects_touched.is_empty() && change.sketches_touched.is_empty());
    assert_eq!(doc.state_hash(), before, "a decline changes nothing");
    assert_eq!(doc.history_entries().undo.len(), depth, "nothing recorded");

    // The overlay query names the rim as that loop, from any one of the
    // faces (the search gathers the rest), and no open edge besides it.
    for seeds in [four.clone(), vec![four[0]]] {
        let missing = doc.faces_open_edges(&seeds);
        assert!(missing.open_edges.is_empty(), "{:?}", missing.open_edges);
        assert_eq!(missing.non_planar_loops.len(), 1);
        let rim = &missing.non_planar_loops[0];
        for q in [D, A, top[0], top[1], top[2], top[3]] {
            assert!(
                rim.iter().any(|p| p.approx_eq(q, 1e-12)),
                "{q:?} on the rim"
            );
        }
    }
    assert_eq!(doc.state_hash(), before, "the query is read-only");

    // The fourth wall, still raw: Make Solid from one wall closes the cube
    // (its top edge seeds the lid).
    raw_face(&mut doc, &[D, A, top[0], top[3]]);
    assert!(
        doc.visible_object_ids().is_empty(),
        "raw faces close nothing"
    );
    let before = doc.state_hash();
    let (out, change) = doc.make_solid_from_sketches(&[four[1]]).unwrap();
    solid(&doc, &out, 1.0);
    assert!(!change.objects_touched.is_empty());
    assert_eq!(
        doc.history_entries().undo.last().unwrap().label,
        "Make Solid"
    );
    assert!(
        doc.sketch_ids().is_empty(),
        "every face's sketch is consumed"
    );
    reopens(&doc);
    one_step(&mut doc, before);
}

/// §6: a gap whose rim is a closed loop off one plane is reported as that
/// loop (warn color), its edges not repeated as open edges.
#[test]
fn a_non_planar_rim_is_reported_as_a_loop() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let tops = [up(A, 1.), up(B, 1.), up(C, 1.), up(D, 2.)];
    let five = open_box(&mut doc, tops);
    let before = doc.state_hash();
    let (out, _) = doc.make_solid_from_sketches(&five).unwrap();
    let [CellOutcome::Declined(d)] = &out[..] else {
        panic!("{out:?}")
    };
    assert_eq!(d.reason, CellDeclineReason::NotClosed);
    assert_eq!(d.non_planar_loops.len(), 1);
    assert_eq!(doc.state_hash(), before);
    let missing = doc.faces_open_edges(&five);
    assert_eq!(missing.non_planar_loops.len(), 1);
    let ring = &missing.non_planar_loops[0];
    assert_eq!(ring.len(), 4);
    assert!(
        tops.iter()
            .all(|t| ring.iter().any(|q| q.approx_eq(*t, 1e-12)))
    );
    assert_eq!(missing.count(), 0, "the loop's edges are not repeated");
}

/// A stale sketch refuses typed and changes nothing; the query ignores it.
#[test]
fn a_stale_sketch_refuses_make_solid() {
    let mut doc = Document::new();
    let s = raw_face(&mut doc, &[A, B, C, D]);
    let gone = raw_face(&mut doc, &[A, B, up(B, 1.), up(A, 1.)]);
    doc.delete_sketch(gone).unwrap();
    let before = doc.state_hash();
    assert!(matches!(
        doc.make_solid_from_sketches(&[s, gone]),
        Err(DocumentError::UnknownSketch)
    ));
    assert_eq!(doc.state_hash(), before);
    let missing = doc.faces_open_edges(&[gone]);
    assert!(missing.is_empty());
}

// ------------------------------------------------------------ API gestures

/// §7, §11 stage 5: a rectangle and four lines drawn as API gestures make a
/// pyramid on the closing line, each line on the plane of a face it
/// bounds (the gesture's own plane is a candidate: a raw gesture arranges
/// nothing there). One undo returns the drawing before the last line.
#[test]
fn api_gestures_on_the_faces_own_planes_make_a_pyramid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    api_ground_rect(&mut doc);
    // Each line from a corner to the apex, on the triangle through it and
    // the next corner.
    let tri = |q: Point3, r: Point3| Plane::from_polygon(&[q, r, APEX]).unwrap();
    let (closed, cells) = api_line(&mut doc, tri(A, B), A, APEX);
    assert!(closed.is_empty() && cells.is_empty());
    let (closed, cells) = api_line(&mut doc, tri(B, C), B, APEX);
    assert_eq!(closed.len(), 1, "A–B–apex fills");
    assert!(cells.is_empty());
    let (closed, cells) = api_line(&mut doc, tri(C, D), C, APEX);
    assert_eq!(closed.len(), 1, "B–C–apex fills");
    assert!(cells.is_empty());
    let before = doc.state_hash();
    let (closed, cells) = api_line(&mut doc, tri(D, A), D, APEX);
    assert!(
        !closed.is_empty(),
        "the last line closes the last two sides"
    );
    solid(&doc, &cells, 1.0 / 3.0);
    assert_eq!(doc.visible_object_ids().len(), 1);
    reopens(&doc);
    one_step(&mut doc, before);
}

/// The same pyramid with each line on a vertical plane through it (no
/// face's plane), as an agent might pick.
#[test]
fn api_gestures_on_vertical_planes_make_a_pyramid() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    api_ground_rect(&mut doc);
    let vertical = |q: Point3| plane(q, (APEX - q).cross(Vec3::new(0., 0., 1.)));
    let mut last = Vec::new();
    for q in [A, B, C, D] {
        last = api_line(&mut doc, vertical(q), q, APEX).1;
    }
    solid(&doc, &last, 1.0 / 3.0);
    reopens(&doc);
}

/// Plain `end_sketch_gesture` keeps its contract: a raw gesture closes
/// nothing (the app's text placement and old recordings rely on it).
#[test]
fn a_plain_gesture_closes_nothing() {
    let mut doc = Document::new();
    raw_face(&mut doc, &[A, B, C, D]);
    for q in [A, B, C, D] {
        let s = doc.add_sketch(plane(q, (APEX - q).cross(Vec3::new(0., 0., 1.))));
        doc.begin_sketch_gesture(s).unwrap();
        doc.sketch_mut(s).unwrap().add_segment(q, APEX).unwrap();
        doc.end_sketch_gesture(s).unwrap();
    }
    assert!(doc.visible_object_ids().is_empty());
    let faced: usize = doc
        .sketch_ids()
        .into_iter()
        .map(|s| {
            doc.sketch(s)
                .unwrap()
                .regions()
                .values()
                .filter(|r| !r.faceless)
                .count()
        })
        .sum();
    assert_eq!(faced, 1, "only the ground rectangle is a face");
}

/// An API gesture that changes nothing records nothing and closes nothing.
#[test]
fn an_unchanged_api_gesture_closes_nothing() {
    let mut doc = Document::new();
    let s = api_ground_rect(&mut doc);
    let depth = doc.history_entries().undo.len();
    doc.begin_sketch_gesture(s).unwrap();
    let (change, closed, cells) = doc.end_sketch_gesture_closing_faces(s).unwrap();
    assert!(change.sketches_touched.is_empty() && closed.is_empty() && cells.is_empty());
    assert_eq!(doc.history_entries().undo.len(), depth);
}

// ------------------------------------------------------------ nested rims

/// A unit box on the ground, by extrusion.
fn unit_box(doc: &mut Document) -> kernel::ObjectId {
    let g = plane(A, Vec3::new(0., 0., 1.));
    let objects = doc.drawable_objects_on_plane(g, None);
    let (r, _) = doc
        .draft(g, None, &objects, "Rectangle", |w| {
            for (a, b) in [(A, B), (B, C), (C, D), (D, A)] {
                w.add_segment(a, b)?;
            }
            Ok(())
        })
        .unwrap();
    let s = r.sketches[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    doc.extrude_region(s, region, 1.0).unwrap().0
}

/// The square `[lo, hi]²` at height `z`, counter-clockwise from above.
fn square(lo: f64, hi: f64, z: f64) -> [Point3; 4] {
    [p(lo, lo, z), p(hi, lo, z), p(hi, hi, z), p(lo, hi, z)]
}

/// A square tube standing on the unit box's top: outer walls on
/// `[0.1, 0.9]²` and inner walls on `[0.3, 0.7]²` from z = 1 to 2, the
/// four trapezoids of its top between them; with `plug`, the inner square
/// at z = 2 too. Every face its own raw sketch.
fn ring_tower(doc: &mut Document, plug: bool) -> Vec<SketchId> {
    let (o1, o2) = (square(0.1, 0.9, 1.), square(0.1, 0.9, 2.));
    let (i1, i2) = (square(0.3, 0.7, 1.), square(0.3, 0.7, 2.));
    let mut out = Vec::new();
    for k in 0..4 {
        let n = (k + 1) % 4;
        out.push(raw_face(doc, &[o1[k], o1[n], o2[n], o2[k]]));
        out.push(raw_face(doc, &[i1[k], i1[n], i2[n], i2[k]]));
        out.push(raw_face(doc, &[o2[k], o2[n], i2[n], i2[k]]));
    }
    if plug {
        out.push(raw_face(doc, &i2));
    }
    out
}

/// Review #24: a tube on a box grows the box by the tube (the inner square
/// stays open air). The same tube with its top plugged would leave the plug
/// a zero-thickness lid with air on both sides over a sealed pocket, so
/// the whole set declines, typed, and the document is untouched.
#[test]
fn a_plugged_tube_on_a_box_declines_and_an_open_one_grows_it() {
    // The open tube: the box grows by the tube alone.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let faces = ring_tower(&mut doc, false);
    let before = doc.state_hash();
    let (out, _) = doc.make_solid_from_sketches(&faces).unwrap();
    let [CellOutcome::Grew { from, to }] = out[..] else {
        panic!("expected the box to grow: {out:?}")
    };
    assert_eq!(from, bx);
    assert_eq!(doc.visible_object_ids(), vec![to]);
    let obj = doc.object(to).unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj.validate().expect("the grown box validates");
    assert!((volume(obj) - 1.48).abs() <= 1e-9, "volume {}", volume(obj));
    for s in &faces {
        assert!(
            doc.sketch(*s).is_none_or(|sk| sk.regions().is_empty()),
            "every face was consumed"
        );
    }
    reopens(&doc);
    one_step(&mut doc, before);

    // The plugged tube: a typed decline, nothing changed or recorded.
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let faces = ring_tower(&mut doc, true);
    let before = doc.state_hash();
    let depth = doc.history_entries().undo.len();
    let (out, change) = doc.make_solid_from_sketches(&faces).unwrap();
    let [CellOutcome::Declined(d)] = &out[..] else {
        panic!("expected one decline: {out:?}")
    };
    assert_eq!(d.reason, CellDeclineReason::SelfTouching);
    assert!(change.objects_touched.is_empty() && change.sketches_touched.is_empty());
    assert_eq!(doc.state_hash(), before, "the decline changes nothing");
    assert_eq!(doc.history_entries().undo.len(), depth, "nothing recorded");
    assert_eq!(doc.visible_object_ids(), vec![bx]);
    let obj = doc.object(bx).unwrap();
    assert!((volume(obj) - 1.0).abs() <= 1e-12);
}

/// The plugged tube's decline is its own: an unrelated cube of raw faces
/// in the same Make Solid call still becomes a solid, while the tube's
/// faces and the box stay exactly as they were.
#[test]
fn a_plugged_tube_declines_alone_beside_a_cube_that_closes() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let bx = unit_box(&mut doc);
    let box_before = doc.object(bx).unwrap().clone();
    let tube = ring_tower(&mut doc, true);
    let s = |q: Point3| p(q.x + 5., q.y, q.z);
    let (a, b, c, d) = (s(A), s(B), s(C), s(D));
    let (a1, b1, c1, d1) = (up(a, 1.), up(b, 1.), up(c, 1.), up(d, 1.));
    let mut faces = tube.clone();
    for f in [
        [a, b, c, d],
        [a1, b1, c1, d1],
        [a, b, b1, a1],
        [b, c, c1, b1],
        [c, d, d1, c1],
        [d, a, a1, d1],
    ] {
        faces.push(raw_face(&mut doc, &f));
    }
    let tube_regions: Vec<usize> = tube
        .iter()
        .map(|&t| doc.sketch(t).unwrap().regions().len())
        .collect();
    let (out, _) = doc.make_solid_from_sketches(&faces).unwrap();
    let created: Vec<&CellOutcome> = out
        .iter()
        .filter(|c| matches!(c, CellOutcome::Created(_)))
        .collect();
    assert_eq!(created.len(), 1, "{out:?}");
    solid(&doc, &[created[0].clone()], 1.0);
    assert!(
        out.iter().any(|c| matches!(
            c,
            CellOutcome::Declined(d) if d.reason == CellDeclineReason::SelfTouching
        )),
        "{out:?}"
    );
    assert_eq!(out.len(), 2, "{out:?}");
    let now = doc.object(bx).expect("the box stays");
    assert_eq!(now.faces().len(), box_before.faces().len());
    assert!((volume(now) - 1.0).abs() <= 1e-12);
    for (t, n) in tube.iter().zip(tube_regions) {
        assert_eq!(
            doc.sketch(*t).unwrap().regions().len(),
            n,
            "the tube's faces are untouched"
        );
    }
    reopens(&doc);
}
