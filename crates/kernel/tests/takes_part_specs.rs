//! Executable specs for ruling R11 of wireframe faces and solids (§16):
//! tag-hidden and group-held drawing.
//!
//! The contract under test:
//! - everywhere the design gathers (foreign edges, the close pass's seeds,
//!   partners, facing and candidate lifts, the faces of a cell) a free
//!   sketch takes part only when shown and not a closed group's: a line on
//!   a hidden tag, or held by a closed group, closes nothing and is never
//!   taken; a line held by the group an open session is editing does take
//!   part, and a solid it closes folds into that group at the close;
//! - an Object takes part only when it is not view-hidden (nor tag-hidden,
//!   nor a closed group's member): a view-hidden box's edge closes no gable;
//! - the ordinary on-plane lift of a draft is unchanged: a draft still
//!   arranges with tag-hidden and closed-group sketches on its own plane.

use kernel::{
    Document, DraftReport, ImportScene, ImportSketch, ImportTag, NodeId, ObjectId, Plane, Point3,
    SketchId, Vec3,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane(at: Point3, n: Vec3) -> Plane {
    Plane::from_point_normal(at, n).unwrap()
}

fn ground() -> Plane {
    plane(p(0., 0., 0.), Vec3::new(0., 0., 1.))
}

const A: Point3 = Point3::new(0., 0., 0.);
const B: Point3 = Point3::new(1., 0., 0.);
const C: Point3 = Point3::new(1., 1., 0.);
const D: Point3 = Point3::new(0., 1., 0.);
const APEX: Point3 = Point3::new(0.5, 0.5, 1.);

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

/// The vertical plane through `a`–`b`.
fn home_of(a: Point3, b: Point3) -> Plane {
    plane(a, (b - a).cross(Vec3::new(0., 0., 1.)))
}

fn line(doc: &mut Document, a: Point3, b: Point3) -> DraftReport {
    draw(doc, home_of(a, b), &[(a, b)])
}

fn ring(pts: &[Point3]) -> Vec<(Point3, Point3)> {
    (0..pts.len())
        .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
        .collect()
}

/// Whether some live sketch holds a faced region with exactly `corners`.
fn has_face(doc: &Document, corners: &[Point3]) -> bool {
    doc.sketch_ids().into_iter().any(|s| {
        let sk = doc.sketch(s).unwrap();
        sk.regions().values().filter(|r| !r.faceless).any(|r| {
            r.outer.len() == corners.len()
                && r.outer.iter().all(|&v| {
                    let q = sk.vertices()[v].position;
                    corners.iter().any(|c| c.approx_eq(q, 1e-9))
                })
        })
    })
}

/// Whether sketch `s` still holds the edge `a`–`b`.
fn holds(doc: &Document, s: SketchId, a: Point3, b: Point3) -> bool {
    let sk = doc.sketch(s).expect("the sketch is live");
    sk.edges().values().any(|e| {
        let (c, d) = (sk.vertices()[e.from].position, sk.vertices()[e.to].position);
        (c.approx_eq(a, 1e-9) && d.approx_eq(b, 1e-9))
            || (c.approx_eq(b, 1e-9) && d.approx_eq(a, 1e-9))
    })
}

/// One line imported on `on` (no close pass), tagged `tag` (hidden when
/// `hidden`). Returns its sketch.
fn import_line(
    doc: &mut Document,
    on: Plane,
    a: Point3,
    b: Point3,
    tag: &str,
    hidden: bool,
) -> SketchId {
    let mut scene = ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: Vec::new(),
        tags: vec![ImportTag {
            path: vec![tag.into()],
            hidden,
        }],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    };
    scene.extra.sketches.push(ImportSketch {
        plane: on,
        def: None,
        group: None,
        segments: vec![[a, b]],
        curves: Vec::new(),
        tags: vec![vec![tag.into()]],
    });
    let (report, _) = doc.ingest(scene, Vec::new()).unwrap();
    report.sketches[0].expect("the line imported")
}

/// A unit box far off the drawing, grouped: a group to hold drawing.
fn far_group(doc: &mut Document) -> (kernel::GroupId, ObjectId) {
    let s = draw(
        doc,
        ground(),
        &ring(&[
            p(10., 0., 0.),
            p(11., 0., 0.),
            p(11., 1., 0.),
            p(10., 1., 0.),
        ]),
    )
    .sketches[0];
    let region = doc.extrudable_regions(s).unwrap()[0];
    let bx = doc.extrude_region(s, region, 1.0).unwrap().0;
    let (g, _) = doc.group_nodes(&[NodeId::Object(bx)]).unwrap();
    (g, bx)
}

/// Draws the ground rectangle and the slanted lines from B, C and D.
fn rest_of_pyramid(doc: &mut Document) -> DraftReport {
    draw(doc, ground(), &ring(&[A, B, C, D]));
    let mut last = DraftReport::default();
    for q in [B, C, D] {
        last = line(doc, q, APEX);
    }
    last
}

/// A line on a hidden tag on another plane closes nothing and is not taken.
#[test]
fn a_tag_hidden_line_closes_nothing_and_is_not_taken() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let hidden = import_line(&mut doc, home_of(A, APEX), A, APEX, "construction", true);
    assert!(doc.sketch_tags_hidden(hidden));
    let last = rest_of_pyramid(&mut doc);
    assert!(last.cells.is_empty(), "{:?}", last.cells);
    assert!(doc.visible_object_ids().is_empty(), "no solid");
    assert!(
        !has_face(&doc, &[A, B, APEX]),
        "the hidden line closes no face"
    );
    assert!(!has_face(&doc, &[D, A, APEX]));
    assert!(
        has_face(&doc, &[B, C, APEX]),
        "shown lines still close faces"
    );
    assert!(holds(&doc, hidden, A, APEX), "the hidden line is not taken");

    // Control: the same line shown closes the pyramid.
    let mut doc = Document::new();
    import_line(&mut doc, home_of(A, APEX), A, APEX, "construction", false);
    let last = rest_of_pyramid(&mut doc);
    assert_eq!(last.cells.len(), 1, "{:?}", last.cells);
    assert_eq!(doc.visible_object_ids().len(), 1);
}

/// A closed group's line closes nothing; the same line while its group is
/// open for editing closes the pyramid, which folds into the group at the
/// close.
#[test]
fn a_closed_groups_line_closes_nothing_and_an_open_groups_line_does() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    let (g, _) = far_group(&mut doc);
    let s = line(&mut doc, A, APEX).sketches[0];
    doc.set_sketch_group(s, Some(g)).unwrap();
    let before = doc.state_hash();
    let last = rest_of_pyramid(&mut doc);
    assert!(last.cells.is_empty(), "{:?}", last.cells);
    assert!(!has_face(&doc, &[A, B, APEX]));
    assert!(holds(&doc, s, A, APEX), "the group's line is not taken");
    let objects_closed = doc.visible_object_ids().len();
    assert_eq!(objects_closed, 1, "only the group's box");

    // Back to the group's line alone, then open the group and draw again.
    while doc.state_hash() != before {
        doc.undo().unwrap();
    }
    doc.open_group_session(g).unwrap();
    let last = rest_of_pyramid(&mut doc);
    assert_eq!(last.cells.len(), 1, "{:?}", last.cells);
    let kernel::CellOutcome::Created(solid) = last.cells[0] else {
        panic!("{:?}", last.cells)
    };
    doc.close_group_session().unwrap();
    let members = doc.group_members(g).unwrap();
    assert!(
        members.contains(&NodeId::Object(solid)),
        "the solid folds into the group: {members:?}"
    );
    assert_eq!(doc.node_parent(NodeId::Object(solid)), Some(g));
}

/// A view-hidden box's edge closes no gable; shown, it does.
#[test]
fn a_view_hidden_boxs_edge_closes_no_gable() {
    for hidden in [true, false] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let s = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
        let region = doc.extrudable_regions(s).unwrap()[0];
        let bx = doc.extrude_region(s, region, 1.0).unwrap().0;
        if hidden {
            doc.set_view_hidden_objects([bx]);
        }
        // A gable on the box's x = 0 end, its two rafters each on a plane
        // that is no face of the box.
        let (l, r, top) = (p(0., 0., 1.), p(0., 1., 1.), p(0., 0.5, 1.5));
        let mut on = |a: Point3, b: Point3, n: Vec3| {
            let objects: Vec<ObjectId> = doc
                .drawable_objects_on_plane(plane(a, n), None)
                .into_iter()
                .filter(|&o| !(hidden && o == bx))
                .collect();
            doc.draft(plane(a, n), None, &objects, "Line", move |w| {
                w.add_segment(a, b)?;
                Ok(())
            })
            .unwrap()
            .0
        };
        on(l, top, Vec3::new(1., 1., -1.));
        let last = on(top, r, Vec3::new(1., 1., 1.));
        if hidden {
            assert!(last.faces_closed.is_empty(), "{:?}", last.faces_closed);
            assert!(!has_face(&doc, &[l, r, top]));
        } else {
            assert_eq!(last.faces_closed.len(), 1, "control: the gable closes");
            assert!(has_face(&doc, &[l, r, top]));
        }
    }
}

/// Pins the unchanged on-plane lift: a draft still arranges with a
/// tag-hidden or closed group's sketch on its own plane (a rectangle drawn
/// across such a line splits in two).
#[test]
fn a_draft_still_arranges_with_hidden_and_closed_group_drawing_on_its_plane() {
    let cross = (p(0.5, -1., 0.), p(0.5, 2., 0.));
    for case in ["tag", "group"] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        match case {
            "tag" => {
                import_line(&mut doc, ground(), cross.0, cross.1, "hidden", true);
            }
            _ => {
                let (g, _) = far_group(&mut doc);
                let s = draw(&mut doc, ground(), &[cross]).sketches[0];
                doc.set_sketch_group(s, Some(g)).unwrap();
            }
        }
        draw(&mut doc, ground(), &ring(&[A, B, C, D]));
        let units = doc.plane_units(ground(), None, &[]).unwrap();
        assert_eq!(
            units.regions.len(),
            2,
            "{case}: the line splits the rectangle"
        );
    }
}

/// Review #9/#30: a box hidden through the document's own persisted Hide
/// (as headless, API, CLI and a replayed recording see it) takes no part
/// exactly as one the app's view also hides: the same roof gives the same
/// document either way, and the box never grows.
#[test]
fn a_user_hidden_box_takes_no_part_as_a_view_hidden_one_does() {
    let mut runs = Vec::new();
    for view_too in [false, true] {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let s = draw(&mut doc, ground(), &ring(&[A, B, C, D])).sketches[0];
        let region = doc.extrudable_regions(s).unwrap()[0];
        let bx = doc.extrude_region(s, region, 1.0).unwrap().0;
        doc.set_node_user_hidden(NodeId::Object(bx), true);
        if view_too {
            doc.set_view_hidden_objects([bx]);
        }
        let ridge = p(0.5, 0.5, 1.5);
        let mut cells = Vec::new();
        let mut closed = Vec::new();
        for q in [A, B, C, D] {
            let r = line(&mut doc, ridge, p(q.x, q.y, 1.));
            cells.extend(r.cells);
            closed.extend(r.faces_closed);
        }
        assert!(cells.is_empty(), "view {view_too}: {cells:?}");
        assert!(closed.is_empty(), "view {view_too}: {closed:?}");
        assert!(doc.object(bx).is_some(), "view {view_too}: the box stays");
        // Make Solid sees the same.
        let all = doc.sketch_ids();
        let (out, _) = doc.make_solid_from_sketches(&all).unwrap();
        assert!(
            !out.iter().any(|c| matches!(
                c,
                kernel::CellOutcome::Grew { .. } | kernel::CellOutcome::Created(_)
            )),
            "view {view_too}: {out:?}"
        );
        runs.push(doc.state_hash());
    }
    assert_eq!(
        runs[0], runs[1],
        "the persisted Hide alone gives the same document"
    );
}
