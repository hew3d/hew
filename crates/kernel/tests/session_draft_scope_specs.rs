//! Executable specs for **drafts inside an editing session** (v1.5
//! decisions D1): while a component or group session is open, a draft in
//! the session frame arranges only with what is inside the innermost frame.
//! Geometry outside the session is something to snap to and nothing else —
//! a draft never splits an outside face, never gives a line to an outside
//! object, never merges into an outside sketch, and never closes a loop
//! with an outside edge. Drawing on an outside face still works: the face
//! gives the plane, and the result is the session's free drawing, which
//! folds into the component (or group) at close.
//!
//! Every spec runs under both frame kinds. The component frame is entered
//! through a TRANSLATED placement, so a fold through the session's inverse
//! pose visibly moves what was drawn.

use std::collections::BTreeSet;

use kernel::{
    ComponentId, Document, DocumentError, GroupId, NodeId, ObjectId, Plane, PlaneListing, Point3,
    SketchId, Transform, Vec3,
};
use proptest::prelude::*;

// ----------------------------------------------------------------- helpers

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane(at: Point3, n: Vec3) -> Plane {
    Plane::from_point_normal(at, n).expect("plane")
}

/// The z = 1 plane: the tops of every box below.
fn top() -> Plane {
    plane(p(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0))
}

fn ground() -> Plane {
    plane(p(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
}

/// An axis-aligned box `[x0,x1] x [y0,y1] x [0,1]`, a world object.
fn build_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        p(x0, y0, 0.0),
        p(x1, y0, 0.0),
        p(x1, y1, 0.0),
        p(x0, y1, 0.0),
    ];
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    let region = doc.extrudable_regions(s).expect("sketch is live")[0];
    doc.extrude_region(s, region, 1.0).expect("extrude").0
}

/// Which session frame a spec runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Component,
    Group,
}

/// An open session over two boxes (`members`, at `x` 0..2 and 4..6 in the
/// frame's own coordinates), with a box OUTSIDE it at `x` 10..12,
/// `y` 0..2. The component frame is entered through a placement translated
/// by `+10 y`, so its members sit at `y` 10..12 in the world mid-session.
struct Fixture {
    doc: Document,
    kind: Kind,
    component: Option<ComponentId>,
    group: Option<GroupId>,
    members: [ObjectId; 2],
    outside: ObjectId,
}

impl Fixture {
    fn open(kind: Kind) -> Self {
        Self::open_after(kind, |_| {})
    }

    /// [`Fixture::open`], running `before` on the document just before the
    /// session opens (to put down drawing outside it).
    fn open_after(kind: Kind, before: impl FnOnce(&mut Document)) -> Self {
        let mut doc = Document::new();
        doc.set_torture_mode(true);
        let a = build_box(&mut doc, 0.0, 0.0, 2.0, 2.0);
        let b = build_box(&mut doc, 4.0, 0.0, 6.0, 2.0);
        let outside = build_box(&mut doc, 10.0, 0.0, 12.0, 2.0);
        before(&mut doc);
        let (component, group) = match kind {
            Kind::Component => {
                let (c, _first, _) = doc
                    .make_component(&[NodeId::Object(a), NodeId::Object(b)])
                    .expect("component");
                let (placed, _) = doc
                    .place_instance(c, Transform::translation(Vec3::new(0.0, 10.0, 0.0)))
                    .expect("place");
                doc.open_explode_session(placed).expect("open");
                (Some(c), None)
            }
            Kind::Group => {
                let (g, _) = doc
                    .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
                    .expect("group");
                doc.open_group_session(g).expect("open");
                (None, Some(g))
            }
        };
        Fixture {
            doc,
            kind,
            component,
            group,
            members: [a, b],
            outside,
        }
    }

    /// The world offset of the frame's coordinates mid-session.
    fn dy(&self) -> f64 {
        match self.kind {
            Kind::Component => 10.0,
            Kind::Group => 0.0,
        }
    }

    /// The autosave (serialized as if every session were closed) reopens.
    fn autosave_reopens(&self, what: &str) {
        Document::load(&self.doc.save_for_persistence())
            .unwrap_or_else(|e| panic!("{:?}: the autosave after {what} reopens: {e}", self.kind));
    }

    fn close(&mut self) {
        self.doc.close_innermost_session().expect("close");
    }

    /// Whether sketch `s` is the frame's after the close: a definition
    /// sketch of the component, or the group's own drawing.
    fn folded_in(&self, s: SketchId) -> bool {
        match self.kind {
            Kind::Component => {
                let c = self.component.expect("component");
                self.doc.sketch_owner_component(s) == Some(c)
                    && self
                        .doc
                        .def_member_sketches(c)
                        .unwrap_or_default()
                        .contains(&s)
            }
            Kind::Group => {
                let g = self.group.expect("group");
                self.doc.sketch_owner_group(s) == Some(g)
                    && self.doc.group_member_sketches(g).contains(&s)
            }
        }
    }
}

fn points(doc: &Document, s: SketchId) -> Vec<(f64, f64, f64)> {
    let mut v: Vec<(f64, f64, f64)> = doc
        .sketch(s)
        .expect("sketch is live")
        .vertices()
        .values()
        .map(|v| (v.position.x, v.position.y, v.position.z))
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    v
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

const KINDS: [Kind; 2] = [Kind::Component, Kind::Group];

// --------------------------------------------------------------- spec 1

/// A closed rectangle drawn on an OUTSIDE box's top face mid-session: the
/// box keeps its faces and owns no sketch; the rectangle is one free
/// session sketch with one region, and after the close it is the frame's
/// (a definition sketch in definition-local coordinates, shown at every
/// placement; or the group's drawing). Undo and redo across the close
/// agree.
#[test]
fn a_rectangle_on_an_outside_face_is_session_drawing_and_folds_in() {
    for kind in KINDS {
        let mut f = Fixture::open(kind);
        let dy = f.dy();
        let participants = f.doc.draft_participants(top(), None);
        assert!(
            !participants.contains(&f.outside),
            "{kind:?}: the outside box takes no part"
        );
        assert!(
            f.members.iter().all(|m| participants.contains(m)),
            "{kind:?}: the members take part"
        );
        let faces_before = f.doc.object(f.outside).unwrap().faces().len();
        let (report, _) = f
            .doc
            .draft(top(), None, &participants, "Rectangle", |w| {
                rect(w, 10.5, 0.5, 11.5, 1.5, 1.0)
            })
            .expect("draw on the outside face");
        assert_eq!(
            report.objects,
            Vec::<ObjectId>::new(),
            "{kind:?}: no face split"
        );
        assert_eq!(report.sketches.len(), 1, "{kind:?}: {report:?}");
        let s = report.sketches[0];
        assert_eq!(
            f.doc.sketch_owner_object(s),
            None,
            "{kind:?}: a free sketch"
        );
        assert_eq!(f.doc.sketch(s).unwrap().regions().len(), 1);
        assert_eq!(f.doc.object(f.outside).unwrap().faces().len(), faces_before);
        assert!(f.doc.object_owned_sketches(f.outside).is_empty());
        assert!(
            f.doc.session_frame_sketches().contains(&s),
            "{kind:?}: session drawing"
        );
        f.autosave_reopens("the rectangle");
        let drawn = points(&f.doc, s);

        f.close();
        assert!(
            f.folded_in(s),
            "{kind:?}: the rectangle folds into the frame"
        );
        let local: Vec<(f64, f64, f64)> = drawn.iter().map(|&(x, y, z)| (x, y - dy, z)).collect();
        assert_eq!(
            points(&f.doc, s),
            local,
            "{kind:?}: in the frame's coordinates"
        );
        assert_eq!(f.doc.sketch(s).unwrap().regions().len(), 1);
        if let Some(c) = f.component {
            assert_eq!(f.doc.instances_of(c).len(), 2);
            let (shown, _) = f.doc.def_rendered_sketches(c).expect("definition is live");
            assert!(shown.contains(&s), "every placement shows it");
        }
        assert_eq!(f.doc.object(f.outside).unwrap().faces().len(), faces_before);
        assert!(f.doc.object_owned_sketches(f.outside).is_empty());
        Document::load(&f.doc.save()).expect("the closed document reopens");

        let closed = f.doc.state_hash();
        f.doc.undo().expect("undo the close");
        assert_eq!(points(&f.doc, s), drawn, "{kind:?}: back in the world");
        f.doc.redo().expect("redo the close");
        assert_eq!(f.doc.state_hash(), closed);
        assert!(f.folded_in(s));
    }
}

// --------------------------------------------------------------- spec 2

/// A world free sketch on the plane, outside the session, crossing the
/// new line: neither splits the other.
#[test]
fn an_outside_sketch_on_the_plane_is_not_split_and_does_not_split() {
    for kind in KINDS {
        let mut w = None;
        let mut f = Fixture::open_after(kind, |doc| {
            let (r, _) = doc
                .draft(top(), None, &[], "Line", |sk| {
                    sk.add_segment(p(8.0, 1.0, 1.0), p(14.0, 1.0, 1.0))
                        .map(|_| ())
                })
                .expect("the outside line");
            w = Some(r.sketches[0]);
        });
        let w = w.expect("drawn");
        let before = points(&f.doc, w);
        let edges_before = f.doc.sketch(w).unwrap().edges().len();
        assert_eq!(edges_before, 1);
        assert!(
            !f.doc.draft_scope_holds_sketch(w),
            "{kind:?}: the line is outside"
        );
        let participants = f.doc.draft_participants(top(), None);
        let (report, _) = f
            .doc
            .draft(top(), None, &participants, "Line", |sk| {
                sk.add_segment(p(11.0, -0.5, 1.0), p(11.0, 2.5, 1.0))
                    .map(|_| ())
            })
            .expect("draw across the outside line");
        assert_eq!(
            report.sketches.len(),
            1,
            "{kind:?}: only the new line is written"
        );
        let s = report.sketches[0];
        assert_ne!(s, w);
        assert_eq!(
            points(&f.doc, w),
            before,
            "{kind:?}: the outside line is unchanged"
        );
        assert_eq!(f.doc.sketch(w).unwrap().edges().len(), edges_before);
        assert_eq!(
            f.doc.sketch(s).unwrap().edges().len(),
            1,
            "{kind:?}: the new line is one unsplit edge"
        );
        assert!(f.doc.object_owned_sketches(f.outside).is_empty());
        f.autosave_reopens("the crossing line");
        f.close();
        assert!(f.folded_in(s));
        assert!(
            !f.folded_in(w),
            "{kind:?}: the outside line stays in the world"
        );
        assert_eq!(points(&f.doc, w), before);
        Document::load(&f.doc.save()).expect("reopens");
    }
}

// --------------------------------------------------------------- spec 3

/// Two lines whose free ends meet an outside box's vertical edge (lying in
/// their plane, bounding no face there): outside the session they close a
/// triangle with that edge; inside it the edge is not lent, and no region
/// is made.
#[test]
fn a_loop_that_closes_only_with_an_outside_edge_makes_no_region() {
    // The vertical plane x + y = 10 holds the outside box's edge
    // (10,0,0)-(10,0,1) and bounds none of its faces.
    let diag = plane(p(10.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0));
    let vee = |w: &mut kernel::Sketch| {
        w.add_segment(p(10.0, 0.0, 1.0), p(9.0, 1.0, 0.5))?;
        w.add_segment(p(9.0, 1.0, 0.5), p(10.0, 0.0, 0.0))?;
        Ok(())
    };
    // Control: with no session, the box's edge closes the triangle.
    {
        let mut doc = Document::new();
        let _ = build_box(&mut doc, 10.0, 0.0, 12.0, 2.0);
        let objects = doc.draft_participants(diag, None);
        let (report, _) = doc.draft(diag, None, &objects, "Line", vee).expect("draw");
        let regions: usize = report
            .sketches
            .iter()
            .map(|&s| doc.sketch(s).unwrap().regions().len())
            .sum();
        assert_eq!(regions, 1, "the control closes with the box's edge");
    }
    for kind in KINDS {
        let mut f = Fixture::open(kind);
        let objects = f.doc.draft_participants(diag, None);
        let (report, _) = f
            .doc
            .draft(diag, None, &objects, "Line", vee)
            .expect("draw");
        let regions: usize = report
            .sketches
            .iter()
            .map(|&s| f.doc.sketch(s).unwrap().regions().len())
            .sum();
        assert_eq!(regions, 0, "{kind:?}: no region with an outside edge");
        assert!(report.faces_closed.is_empty() && report.cells.is_empty());
        f.autosave_reopens("the vee");
    }
}

// --------------------------------------------------------------- spec 4

/// A rectangle on a MEMBER's face still splits it, as before the wall.
#[test]
fn a_rectangle_on_a_member_face_still_splits_it() {
    for kind in KINDS {
        let mut f = Fixture::open(kind);
        let dy = f.dy();
        let m = f.members[0];
        let faces_before = f.doc.object(m).unwrap().faces().len();
        let objects = f.doc.draft_participants(top(), None);
        let (report, _) = f
            .doc
            .draft(top(), None, &objects, "Rectangle", |w| {
                rect(w, 0.5, 0.5 + dy, 1.5, 1.5 + dy, 1.0)
            })
            .expect("draw on a member");
        assert_eq!(
            report.objects,
            vec![m],
            "{kind:?}: the member's face splits"
        );
        assert_eq!(f.doc.object(m).unwrap().faces().len(), faces_before + 1);
        f.autosave_reopens("the member rectangle");
        f.close();
        Document::load(&f.doc.save()).expect("reopens");
    }
}

// --------------------------------------------------------------- spec 7

/// Naming an outside object as a participant is refused
/// `ExplodeSessionScope` before anything is lifted; the document is
/// untouched and no draft is left open.
#[test]
fn an_outside_participant_is_refused_and_nothing_changes() {
    for kind in KINDS {
        let mut f = Fixture::open(kind);
        let mut objects = f.doc.draft_participants(top(), None);
        objects.push(f.outside);
        let before = f.doc.state_hash();
        let generation = f.doc.history_generation();
        let err = f
            .doc
            .draft(top(), None, &objects, "Line", |w| {
                w.add_segment(p(10.5, 0.5, 1.0), p(11.0, 1.5, 1.0))
                    .map(|_| ())
            })
            .unwrap_err();
        assert_eq!(err, DocumentError::ExplodeSessionScope, "{kind:?}");
        assert_eq!(
            f.doc
                .begin_draft(top(), None, &[f.outside], "Line")
                .unwrap_err(),
            DocumentError::ExplodeSessionScope
        );
        assert!(
            f.doc.draft_sketch().is_none(),
            "{kind:?}: no draft left open"
        );
        assert_eq!(f.doc.state_hash(), before);
        assert_eq!(f.doc.history_generation(), generation);
        f.autosave_reopens("the refusal");
    }
}

/// A detached sketch outside the session cannot be drafted on mid-session
/// (component and group frames alike); one drawn inside can.
#[test]
fn an_outside_detached_sketch_is_refused_mid_session() {
    for kind in KINDS {
        let mut outside_shape = None;
        let mut f = Fixture::open_after(kind, |doc| {
            let (r, _) = doc
                .draft_detached_new(top(), None, "Line", |w| {
                    w.add_segment(p(20.0, 0.0, 1.0), p(21.0, 0.0, 1.0))
                        .map(|_| ())
                })
                .expect("a detached line outside");
            outside_shape = Some(r.sketches[0]);
        });
        let outside_shape = outside_shape.expect("drawn");
        assert!(!f.doc.draft_scope_holds_sketch(outside_shape));
        let before = f.doc.state_hash();
        assert_eq!(
            f.doc
                .draft_detached(outside_shape, "Line", |w| {
                    w.add_segment(p(21.0, 0.0, 1.0), p(21.0, 1.0, 1.0))
                        .map(|_| ())
                })
                .unwrap_err(),
            DocumentError::ExplodeSessionScope,
            "{kind:?}"
        );
        assert_eq!(f.doc.state_hash(), before);
        assert!(f.doc.draft_sketch().is_none());
        let (r, _) = f
            .doc
            .draft_detached_new(top(), None, "Line", |w| {
                w.add_segment(p(20.0, 5.0, 1.0), p(21.0, 5.0, 1.0))
                    .map(|_| ())
            })
            .expect("a detached line inside");
        let inside_shape = r.sketches[0];
        assert!(f.doc.draft_scope_holds_sketch(inside_shape));
        f.doc
            .draft_detached(inside_shape, "Line", |w| {
                w.add_segment(p(21.0, 5.0, 1.0), p(21.0, 6.0, 1.0))
                    .map(|_| ())
            })
            .expect("the session's own detached shape continues");
        f.autosave_reopens("the detached lines");
        // The listing marks the outside shape as context, the session's not.
        let all = f.doc.free_plane_units(None, &BTreeSet::new());
        for (l, _) in &all {
            if let PlaneListing::Detached {
                sketch, context, ..
            } = *l
            {
                assert_eq!(context, sketch == outside_shape, "{kind:?}: {sketch:?}");
            }
        }
    }
}

// --------------------------------------------------------------- spec 6

/// A plane rebuilt from its numbers rounded to 1e-12, as the app's context
/// string rebuilds it (`draft_specs.rs`'s `rounded_rebuild`).
fn rounded_rebuild(plane: Plane) -> Plane {
    let r = |x: f64| (x * 1e12).round() / 1e12 + 0.0;
    let n = plane.normal();
    let (nx, ny, nz) = (r(n.x), r(n.y), r(n.z));
    let o = Point3::ORIGIN + n * (-plane.signed_distance(Point3::ORIGIN));
    let d = o.x * nx + o.y * ny + o.z * nz;
    Plane::from_point_normal(p(r(nx * d), r(ny * d), r(nz * d)), Vec3::new(nx, ny, nz))
        .expect("a rounded unit normal is not degenerate")
}

/// The listing invariant: with drawing on both sides of the wall on one
/// plane, the plane lists twice — the outside drawing first, as context,
/// then the session's — and every key of the session entry names an edge
/// of the draft opened on that plane.
fn listing_splits_at_the_wall(kind: Kind, pl: Plane, at: impl Fn(f64, f64) -> Point3) {
    // Outside drawing on the plane, before the session.
    let mut f = Fixture::open_after(kind, |doc| {
        doc.draft(pl, None, &[], "Line", |sk| {
            sk.add_segment(at(-1.0, 0.3), at(1.7, 0.9)).map(|_| ())
        })
        .expect("the outside line");
    });
    // Session drawing crossing it: a triangle (a region).
    let objects = f.doc.draft_participants(pl, None);
    f.doc
        .draft(pl, None, &objects, "Polygon", |sk| {
            for (a, b) in [
                ((0.0, 0.0), (1.0, 0.0)),
                ((1.0, 0.0), (0.2, 1.3)),
                ((0.2, 1.3), (0.0, 0.0)),
            ] {
                sk.add_segment(at(a.0, a.1), at(b.0, b.1))?;
            }
            Ok(())
        })
        .expect("session drawing");

    let all = f
        .doc
        .free_plane_units_on(None, &BTreeSet::new(), rounded_rebuild);
    let on_plane: Vec<&(PlaneListing, kernel::PlaneUnits)> = all
        .iter()
        .filter(|(l, _)| {
            let q = l.plane();
            (q.normal() - pl.normal()).length() < 1e-9
                && q.signed_distance(at(0.0, 0.0)).abs() < 1e-9
        })
        .collect();
    assert_eq!(on_plane.len(), 2, "{kind:?}: one entry each side: {all:?}");
    assert!(
        matches!(on_plane[0].0, PlaneListing::Context(_)),
        "{kind:?}: the outside first"
    );
    assert!(on_plane[0].0.is_context());
    assert!(
        matches!(on_plane[1].0, PlaneListing::Plane(_)),
        "{kind:?}: then the session's"
    );
    // Each side lifted among itself: the outside line is one edge, the
    // triangle three, and neither is split by the other.
    let edges = |u: &kernel::PlaneUnits| u.units.iter().map(|u| u.edges.len()).sum::<usize>();
    assert_eq!(edges(&on_plane[0].1), 1, "{kind:?}");
    assert_eq!(edges(&on_plane[1].1), 3, "{kind:?}");
    assert_eq!(on_plane[1].1.regions.len(), 1);

    let on = rounded_rebuild(on_plane[1].0.plane());
    let objects = f.doc.draft_participants(on, None);
    // The single-plane listing on the rebuilt plane agrees with the batch.
    let one = f.doc.plane_units(on, None, &objects).unwrap();
    assert_eq!(&one, &on_plane[1].1, "{kind:?}");
    let keys: Vec<u64> = on_plane[1]
        .1
        .units
        .iter()
        .flat_map(|u| u.edges.iter().map(|e| e.key))
        .collect();
    f.doc.begin_draft(on, None, &objects, "Erase").unwrap();
    let (_, missing) = f.doc.draft_edges_for_keys(&keys);
    assert!(
        missing.is_empty(),
        "{kind:?}: keys the draft does not hold: {missing:?}"
    );
    // The outside entry's line is not in the draft.
    let outside_keys: Vec<u64> = on_plane[0]
        .1
        .units
        .iter()
        .flat_map(|u| u.edges.iter().map(|e| e.key))
        .collect();
    let (found, _) = f.doc.draft_edges_for_keys(&outside_keys);
    assert!(found.is_empty(), "{kind:?}: the outside line is context");
    f.doc.cancel_draft();
    f.autosave_reopens("the listing");
}

#[test]
fn the_listing_splits_at_the_wall_on_a_horizontal_plane() {
    for kind in KINDS {
        let o = p(30.0, 0.0, 3.0);
        listing_splits_at_the_wall(kind, plane(o, Vec3::new(0.0, 0.0, 1.0)), |a, b| {
            o + Vec3::new(a, b, 0.0)
        });
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// The listing invariant on oblique planes too (#103's posture).
    #[test]
    fn the_listing_splits_at_the_wall_on_an_oblique_plane(
        nx in -1.0f64..1.0, ny in -1.0f64..1.0, nz in -1.0f64..1.0,
        ox in 20.0f64..25.0, oy in -5.0f64..5.0, oz in 3.0f64..5.0,
        group in any::<bool>(),
    ) {
        let n = Vec3::new(nx, ny, nz);
        prop_assume!(n.length() > 0.2);
        let n = n * (1.0 / n.length());
        let o = p(ox, oy, oz);
        let helper = if n.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
        let u = n.cross(helper);
        let u = u * (1.0 / u.length());
        let v = n.cross(u);
        // Keep the plane clear of the fixture's boxes, so no object takes
        // part on either side: only the drawing splits.
        let pl = plane(o, n);
        let corners = [p(0.0, 0.0, 0.0), p(12.0, 12.0, 1.0)];
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for x in [corners[0].x, corners[1].x] {
            for y in [-2.0, 12.0] {
                for z in [corners[0].z, corners[1].z] {
                    let d = pl.signed_distance(p(x, y, z));
                    lo = lo.min(d);
                    hi = hi.max(d);
                }
            }
        }
        prop_assume!(lo > 0.01 || hi < -0.01);
        let kind = if group { Kind::Group } else { Kind::Component };
        listing_splits_at_the_wall(kind, pl, |a, b| o + u * a + v * b);
    }
}

// --------------------------------------------------------------- the API

/// `draft_scope_holds_object` answers the begin-draft refusal's question.
#[test]
fn scope_predicates_name_inside_and_outside() {
    for kind in KINDS {
        let f = Fixture::open(kind);
        assert!(f.members.iter().all(|&m| f.doc.draft_scope_holds_object(m)));
        assert!(!f.doc.draft_scope_holds_object(f.outside));
    }
    let mut doc = Document::new();
    let b = build_box(&mut doc, 0.0, 0.0, 1.0, 1.0);
    assert!(doc.draft_scope_holds_object(b), "no session, no wall");
    assert_eq!(doc.draft_participants(top(), None), vec![b]);
}
