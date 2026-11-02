//! Executable specs for loops that touch at a single point (GitHub #43): a
//! circle drawn on a face tangent to the face's edge at exactly one vertex,
//! two circles touching inside a face, a circle touching an imprint's
//! corner.
//!
//! The representation rule (ARCHITECTURE §2.4): every loop is simple (no
//! vertex twice in one loop), and loops of one face may share vertices at
//! isolated points; two loops never share an edge. A closed walk the
//! re-trace (or the ground's region tracer) takes through a touch point is
//! split there into simple loops, so the circle tangent to the edge is a
//! disc face plus a hole in the face around it sharing that one vertex with
//! the face's outer loop, and two circles touching are two holes sharing a
//! vertex. The validator enforces it: loops simple, a face's loops sharing
//! vertices but never an edge.
//!
//! What must hold, each spec below:
//! 1. **Draft.** The tangent circle, built as the Circle tool builds it
//!    (its rim snapped On Edge along an axis from the center, so the rim is
//!    a polygon vertex on the edge), becomes a disc face plus the parent
//!    with a touching hole; so do two circles touching inside a face, a
//!    circle touching an imprint's corner, and a circle tangent to the edge
//!    of a sub-face. On the ground the same shapes give the same regions;
//!    the disc extrudes, the region with the touching hole refuses typed.
//!    Every region on a face's plane names a face (nothing says it cannot
//!    push).
//! 2. **Push/pull.** The disc pulls up, pushes down into a pocket (its
//!    wall's generatrix at the tangent lies in the side face's plane: the
//!    solid touches itself along that line), and pushes through to the
//!    bottom (the bottom face gets a touching hole too). The parent with
//!    the touching hole pulls and pushes. Each result validates, stays
//!    watertight, and undoes and redoes exactly.
//! 3. **Erase.** Deleting the circle merges the hole back and leaves the
//!    parent face as it was, with exact undo and redo.
//! 4. **Move.** Dragging the circle off the edge makes the touching hole an
//!    ordinary hole; dragging an inside circle onto the edge makes it
//!    touching; dragging it across the edge gives the lens split.
//! 5. **Persistence.** Save and load round-trip byte-identically, and the
//!    bytes do not depend on the order the drawing was made in.
//! 7. **Other ops.** Transform, slice, boolean union and subtract, Make
//!    Component, explode: each succeeds and validates or refuses typed.
//! 8. **Split.** A cut across the parent, or around a hole that another
//!    hole touches, gives each touching hole to the face it lies in,
//!    whichever way the cut runs.
//!
//! (6, tessellation, lives in `crates/tessellate`; `.skp` write and read in
//! `crates/skp-export`; STL export in `crates/mesh-export`.)

use std::f64::consts::PI;

use kernel::topo::WatertightState;
use kernel::{
    BooleanOp, CurveGeom, Document, FaceId, KernelOp, Object, ObjectId, Plane, Point3, Transform,
    Vec3, tol,
};

mod face_arrangement_support;
use face_arrangement_support::*;

// ---------------------------------------------------------------- setup

/// The Circle tool's segment count for a radius in metres
/// (`app/src/tools/arcMath.ts` `segmentsPerTurn`): the coarsest count whose
/// sagitta stays within half a millimetre, rounded up to a multiple of 4,
/// clamped to 24..=96.
fn segments_per_turn(radius: f64) -> usize {
    let ratio = 5e-4 / radius;
    if ratio >= 1.0 {
        return 24;
    }
    let exact = PI / (1.0 - ratio).acos();
    (((exact / 4.0).ceil() * 4.0) as usize).clamp(24, 96)
}

/// The Circle tool's polygon (`circlePolygonFace`): vertex 0 at the rim,
/// the rest counter-clockwise about `center`, in the wall's frame.
fn tool_circle(w: &Wall, cx: f64, cy: f64, rim_x: f64, rim_y: f64) -> (Vec<Point3>, CurveGeom) {
    let r = (rim_x - cx).hypot(rim_y - cy);
    let start = (rim_y - cy).atan2(rim_x - cx);
    let n = segments_per_turn(r);
    (w.ring(cx, cy, r, n, start), w.circle_geom(cx, cy, r))
}

/// The top of a 2 × 2 × 1 m box, framed at its corner.
fn top_2x2() -> Wall {
    wall_of_box(ground(), 2.0, 2.0, 1.0, |_| Vec3::new(0.0, 0.0, 1.0))
}

/// The issue's shape: on the 2 × 2 top, a circle about (1, 0.5) with its
/// rim snapped onto the bottom edge at (1, 0) — tangent there at one
/// vertex.
fn tangent_circle(w: &Wall) -> (Vec<Point3>, CurveGeom) {
    let (ring, g) = tool_circle(w, 1.0, 0.5, 1.0, 0.0);
    assert_eq!(ring.len(), 72, "the tool's count for a 0.5 m radius");
    assert!(
        (ring[0] - w.at(1.0, 0.0)).length() == 0.0,
        "the rim vertex is exactly on the edge"
    );
    (ring, g)
}

/// The top with the tangent circle drawn (checked), and the disc face.
fn top_with_tangent_circle() -> (Wall, Vec<Point3>, CurveGeom, FaceId) {
    let mut w = top_2x2();
    let (ring, g) = tangent_circle(&w);
    let seed = w.face;
    let disc = draw_and_check(&mut w, seed, ring.clone(), Some(g));
    (w, ring, g, disc)
}

/// The group face that is not `disc` and holds a hole.
fn parent_of(w: &Wall, disc: FaceId) -> FaceId {
    let obj = w.obj();
    group_faces(obj, &w.plane)
        .into_iter()
        .find(|&f| f != disc && !obj.faces()[f].inner_loops.is_empty())
        .expect("the face around the disc")
}

/// Whether some face (of any plane) holds two loops sharing a vertex.
fn any_touching_loops(obj: &Object) -> bool {
    obj.faces().values().any(|face| {
        let loops: Vec<Vec<kernel::VertexId>> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| {
                obj.loop_half_edges(l)
                    .map(|h| obj.half_edges()[h].origin)
                    .collect()
            })
            .collect();
        (0..loops.len())
            .any(|i| (i + 1..loops.len()).any(|j| loops[i].iter().any(|v| loops[j].contains(v))))
    })
}

/// Every region of the plane names a face of `w`'s object, none saying why
/// it cannot push.
fn assert_every_region_is_a_face(w: &Wall) {
    let units = units_on(&w.doc, None, w.id, w.plane);
    assert!(!units.regions.is_empty());
    for r in &units.regions {
        assert!(
            matches!(r.over, Some((o, Some(_))) if o == w.id) && r.why.is_none(),
            "a region on the plane is not a face: {:?} {:?}",
            r.over,
            r.why
        );
    }
}

/// Signed volume of a closed object (divergence theorem over its faces,
/// holes subtracted).
fn volume(obj: &Object) -> f64 {
    let mut v = 0.0;
    for face in obj.faces().values() {
        let n = face.plane.normal();
        let area: f64 = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .map(|l| signed_area(&obj.loop_positions(l).collect::<Vec<_>>(), n))
            .sum();
        let p = obj
            .loop_positions(face.outer_loop)
            .next()
            .expect("a vertex");
        v += area * n.dot(p.to_vec()) / 3.0;
    }
    v
}

/// Saves after a change: the document reopens to the same bytes, undo
/// restores `bytes0` and redo `bytes1` exactly, three times over.
fn assert_exact_history(doc: &mut Document, bytes0: &[u8]) {
    let bytes1 = doc.save();
    assert_reopens(&bytes1);
    for _ in 0..3 {
        doc.undo().expect("undo");
        assert_eq!(doc.save(), bytes0, "undo restores the saved bytes");
        doc.redo().expect("redo");
        assert_eq!(doc.save(), bytes1, "redo restores the saved bytes");
    }
}

/// Push/pull `face` the way the app does (`Scene::push_pull`): past the
/// opposite wall it is a push-through boolean; a flat imprinted sub-face
/// extrudes its own walls; any other face push/pulls. Returns the objects
/// holding the result.
fn push_like_the_app(doc: &mut Document, id: ObjectId, face: FaceId, d: f64) -> Vec<ObjectId> {
    let obj = doc.object(id).expect("object");
    if obj.push_pull_overshoots(face, d) {
        let (results, _) = doc
            .push_pull_through(id, face, d)
            .unwrap_or_else(|e| panic!("push through refused: {e:?}"));
        return results;
    }
    let op = if obj.is_flat_sub_face(face) {
        KernelOp::ExtrudeSubFace {
            sub_face: face,
            distance: d,
        }
    } else {
        KernelOp::PushPull { face, distance: d }
    };
    doc.apply_object_op(id, op)
        .unwrap_or_else(|e| panic!("push/pull refused: {e:?}"));
    vec![id]
}

/// Pushes `face` by `d` like the app and checks every result validates and
/// stays watertight, the volume changes by the swept prism (`swept` signed
/// m³), and the history is exact. Returns the result objects.
fn push_and_check(w: &mut Wall, face: FaceId, d: f64, swept: f64) -> Vec<ObjectId> {
    let id = w.id;
    let v0 = volume(w.obj());
    let bytes0 = w.doc.save();
    let results = push_like_the_app(&mut w.doc, id, face, d);
    let mut v1 = 0.0;
    for &r in &results {
        let obj = w.doc.object(r).expect("result");
        obj.validate().expect("the result validates");
        assert_eq!(obj.watertight(), WatertightState::Watertight);
        v1 += volume(obj);
    }
    assert!(
        (v1 - v0 - swept).abs() < 1e-9,
        "volume {v0} -> {v1}, expected a change of {swept}"
    );
    assert_exact_history(&mut w.doc, &bytes0);
    results
}

/// Net area (outer less holes) of one face.
fn net_area(obj: &Object, f: FaceId) -> f64 {
    let face = &obj.faces()[f];
    let n = face.plane.normal();
    std::iter::once(face.outer_loop)
        .chain(face.inner_loops.iter().copied())
        .map(|l| signed_area(&obj.loop_positions(l).collect::<Vec<_>>(), n))
        .sum()
}

// ------------------------------------------------------------- 1. draft

#[test]
fn the_tool_circle_tangent_to_the_edge_is_a_disc_in_a_touching_hole() {
    let (w, ring, g, disc) = top_with_tangent_circle();
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 2, "the disc and the top");
    assert!(obj.faces()[disc].inner_loops.is_empty());
    let parent = parent_of(&w, disc);
    let face = &obj.faces()[parent];
    assert_eq!(face.inner_loops.len(), 1, "one hole");
    let outer = ring_of(obj, face.outer_loop);
    let hole = ring_of(obj, face.inner_loops[0]);
    let touch = w.at(1.0, 0.0);
    let shared: Vec<&Point3> = hole.iter().filter(|p| outer.contains(p)).collect();
    assert_eq!(
        shared.len(),
        1,
        "the hole shares one vertex with the outer loop"
    );
    assert_eq!(*shared[0], touch, "at the rim");
    assert_eq!(hole.len(), ring.len(), "the hole is the circle");
    assert_eq!(outer.len(), 5, "the square, split at the rim");
    // Every facet keeps its circle; nothing stays drawing.
    assert_eq!(claims_on(obj, g), ring.len());
    assert!(w.doc.object_owned_sketches(w.id).is_empty());
    assert_every_region_is_a_face(&w);
    let area = net_area(obj, parent) + net_area(obj, disc);
    assert!((area - 4.0).abs() < 1e-12, "the top's area is conserved");
}

#[test]
fn a_tangent_circle_drawn_at_any_quadrant_rim_touches_as_a_face() {
    // Rim on each of the four edges, along an axis from the center.
    for (cx, cy, rx, ry) in [
        (1.0, 0.5, 1.0, 0.0),
        (1.5, 1.0, 2.0, 1.0),
        (1.0, 1.5, 1.0, 2.0),
        (0.5, 1.0, 0.0, 1.0),
    ] {
        let mut w = top_2x2();
        let (ring, g) = tool_circle(&w, cx, cy, rx, ry);
        let seed = w.face;
        draw_and_check(&mut w, seed, ring, Some(g));
        assert!(has_touching_loops(w.obj(), &w.plane));
        assert_every_region_is_a_face(&w);
    }
}

#[test]
fn two_circles_touching_inside_a_face_are_two_holes_sharing_a_vertex() {
    let mut w = top_2x2();
    let r = 0.3;
    let (a, ga) = (w.ring(0.6, 1.0, r, 24, 0.0), w.circle_geom(0.6, 1.0, r));
    // The second circle's vertex 12 (angle pi) on the first's vertex 0.
    let mut b = w.ring(1.2, 1.0, r, 24, 0.0);
    assert!((b[12] - a[0]).length() < 1e-15);
    b[12] = a[0];
    let gb = w.circle_geom(1.2, 1.0, r);
    let seed = w.face;
    draw_and_check(&mut w, seed, a.clone(), Some(ga));
    let seed = face_at(&w, 1.8, 1.8);
    draw_and_check(&mut w, seed, b.clone(), Some(gb));
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 3, "the top and two discs");
    let top = face_at(&w, 1.8, 1.8);
    let holes: Vec<Vec<Point3>> = obj.faces()[top]
        .inner_loops
        .iter()
        .map(|&l| ring_of(obj, l))
        .collect();
    assert_eq!(holes.len(), 2, "two holes");
    assert!(
        holes.iter().all(|h| h.contains(&a[0])),
        "sharing the touch point"
    );
    assert_eq!(claims_on(obj, ga) + claims_on(obj, gb), 48);
    assert_every_region_is_a_face(&w);
}

#[test]
fn a_circle_touching_an_imprint_corner_from_outside_is_a_face() {
    let mut w = top_2x2();
    let seed = w.face;
    let rect = w.rect(0.5, 0.5, 1.0, 1.0);
    draw_and_check(&mut w, seed, rect, None);
    // Vertex 0 (angle pi) on the rectangle's corner (1.5, 1.5), the rest
    // to its right: it touches the rectangle at that corner only.
    let mut ring = w.ring(1.7, 1.5, 0.2, 24, PI);
    assert!((ring[0] - w.at(1.5, 1.5)).length() < 1e-15);
    ring[0] = w.at(1.5, 1.5);
    let g = w.circle_geom(1.7, 1.5, 0.2);
    let seed = face_at(&w, 1.9, 0.2);
    draw_and_check(&mut w, seed, ring.clone(), Some(g));
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 3);
    let top = face_at(&w, 1.9, 0.2);
    assert_eq!(
        obj.faces()[top].inner_loops.len(),
        2,
        "the rectangle and the disc"
    );
    assert!(has_touching_loops(obj, &w.plane));
    assert_every_region_is_a_face(&w);
}

#[test]
fn a_circle_tangent_to_a_sub_faces_edge_touches_inside_it() {
    let mut w = top_2x2();
    let seed = w.face;
    let rect = w.rect(0.25, 0.25, 1.5, 1.0);
    let sub = draw_and_check(&mut w, seed, rect, None);
    // Tangent to the rectangle's bottom side at (1, 0.25), inside it.
    let (ring, g) = tool_circle(&w, 1.0, 0.5, 1.0, 0.25);
    draw_and_check(&mut w, sub, ring.clone(), Some(g));
    let obj = w.obj();
    assert_eq!(group_faces(obj, &w.plane).len(), 3);
    let rect_face = face_at(&w, 0.3, 1.2);
    let face = &obj.faces()[rect_face];
    assert_eq!(face.inner_loops.len(), 1);
    let outer = ring_of(obj, face.outer_loop);
    assert!(
        ring_of(obj, face.inner_loops[0])
            .iter()
            .any(|p| outer.contains(p)),
        "the disc's hole touches the rectangle's outer loop"
    );
    assert_eq!(claims_on(obj, g), ring.len());
    assert_every_region_is_a_face(&w);
}

#[test]
fn on_the_ground_the_same_shapes_give_the_same_regions() {
    // A ground sketch holding the 2 × 2 square and the tangent circle: the
    // disc and the square with a hole touching its outer ring at the rim.
    let w = top_2x2();
    let (ring, g) = tangent_circle(&w);
    let mut doc = Document::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground");
    let s = doc.add_sketch(plane);
    let flat = |p: Point3| Point3::new(p.x, p.y, 0.0);
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        let sq = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(0.0, 2.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(sq[i], sq[(i + 1) % 4]).expect("square");
        }
        sk.begin_curve_with(CurveGeom {
            center: flat(g.center),
            radius: g.radius,
        })
        .expect("curve");
        for i in 0..ring.len() {
            sk.add_segment(flat(ring[i]), flat(ring[(i + 1) % ring.len()]))
                .expect("circle");
        }
        sk.end_curve();
    }
    let sk = doc.sketch(s).expect("sketch");
    assert!(!ground_has_pinch(sk), "every ring is simple");
    let regions = ground_regions(sk);
    assert_eq!(regions.len(), 2, "the disc and the square around it");
    let touching = regions
        .iter()
        .find(|r| r.1.len() == 1)
        .expect("the square holds the circle");
    assert!(
        touching.1[0].iter().any(|p| touching.0.contains(p)),
        "its hole shares the rim with its outer ring"
    );
    // The disc extrudes; the square with the touching hole refuses typed,
    // `RegionTouchesHole` (no
    // single manifold solid can be swept from it), the document untouched.
    let ids = doc.extrudable_regions(s).expect("regions");
    let mut extruded = 0;
    for rid in ids {
        let r = &doc.sketch(s).expect("sketch").regions()[rid];
        let touches = r
            .holes
            .iter()
            .any(|h| h.iter().any(|v| r.outer.contains(v)));
        let bytes0 = doc.save();
        match doc.extrude_region(s, rid, 0.5) {
            Ok((id, _)) => {
                assert!(!touches, "the touching region extruded");
                let obj = doc.object(id).expect("disc solid");
                obj.validate().expect("valid");
                assert_eq!(obj.watertight(), WatertightState::Watertight);
                extruded += 1;
                doc.undo().expect("undo");
            }
            Err(e) => {
                assert!(touches, "a simple region refused: {e:?}");
                assert!(
                    matches!(
                        e,
                        kernel::DocumentError::Sketch(kernel::SketchError::RegionTouchesHole)
                    ),
                    "refused for what it is, not as a tracing failure: {e:?}"
                );
                assert_eq!(doc.save(), bytes0, "the refusal changed nothing");
                let depth = doc.undo_depth();
                assert!(
                    matches!(
                        doc.sketch(s).expect("sketch").profile(rid),
                        Err(kernel::SketchError::RegionTouchesHole)
                    ),
                    "the profile names it too"
                );
                assert_eq!(doc.undo_depth(), depth);
            }
        }
    }
    assert_eq!(extruded, 1, "the disc extrudes");
}

#[test]
fn two_circles_touching_on_the_ground_are_two_discs() {
    let mut sk = kernel::Sketch::on_plane(ground());
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let r = 0.3;
    let ring = |cx: f64| -> Vec<Point3> {
        (0..24)
            .map(|i| {
                let a = 2.0 * PI * f64::from(i) / 24.0;
                p(cx + r * a.cos(), 1.0 + r * a.sin())
            })
            .collect()
    };
    let a = ring(0.6);
    let mut b = ring(1.2);
    b[12] = a[0];
    for c in [&a, &b] {
        for i in 0..24 {
            sk.add_segment(c[i], c[(i + 1) % 24]).expect("circle");
        }
    }
    assert!(!ground_has_pinch(&sk));
    let regions = ground_regions(&sk);
    assert_eq!(regions.len(), 2, "two discs, no region around them");
    assert!(regions.iter().all(|r| r.1.is_empty() && r.0.len() == 24));
    for rid in sk.regions().keys() {
        sk.profile(rid).expect("each disc is a profile");
    }
}

// --------------------------------------------------------- 2. push/pull

#[test]
fn the_disc_pulls_up() {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let area = signed_area(&ring, w.plane.normal());
    push_and_check(&mut w, disc, 0.25, 0.25 * area);
}

#[test]
fn the_disc_pushes_down_into_a_pocket_touching_the_side_face_along_a_line() {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let area = signed_area(&ring, w.plane.normal());
    for d in [-0.25, -0.5, -0.9] {
        push_and_check(&mut w, disc, d, d * area);
        w.doc.undo().expect("undo");
    }
}

#[test]
fn the_disc_pushes_through_to_the_bottom() {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let area = signed_area(&ring, w.plane.normal());
    let results = push_and_check(&mut w, disc, -1.0, -area);
    assert_eq!(results.len(), 1, "one solid with a through-hole");
    let obj = w.doc.object(results[0]).expect("result");
    let bottom = obj
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z < -0.9)
        .map(|(id, _)| id)
        .expect("the bottom face");
    assert_eq!(
        obj.faces()[bottom].inner_loops.len(),
        1,
        "the bottom's hole"
    );
    assert!(
        any_touching_loops(obj),
        "it touches the bottom's outer loop"
    );
}

#[test]
fn the_parent_with_the_touching_hole_pulls_and_pushes() {
    let (mut w, _, _, disc) = top_with_tangent_circle();
    let parent = parent_of(&w, disc);
    let net = net_area(w.obj(), parent);
    for d in [0.25, -0.25, -0.75] {
        push_and_check(&mut w, parent, d, d * net);
        w.doc.undo().expect("undo");
    }
}

#[test]
fn pulling_the_disc_then_the_parent_stacks_and_undoes() {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let parent = parent_of(&w, disc);
    let net = net_area(w.obj(), parent);
    let area = signed_area(&ring, w.plane.normal());
    let bytes0 = w.doc.save();
    push_and_check(&mut w, disc, 0.5, 0.5 * area);
    push_and_check(&mut w, parent, 0.25, 0.25 * net);
    w.doc.undo().expect("undo");
    w.doc.undo().expect("undo");
    assert_eq!(w.doc.save(), bytes0);
}

// ------------------------------------------------------------- 3. erase

#[test]
fn erasing_the_circle_merges_the_hole_back() {
    let mut w = top_2x2();
    let before = w.obj().clone();
    let bytes_before = w.doc.save();
    let (ring, g) = tangent_circle(&w);
    let seed = w.face;
    draw_and_check(&mut w, seed, ring.clone(), Some(g));
    let bytes0 = w.doc.save();
    let depth0 = w.doc.undo_depth();
    let plane = w.plane;
    let id = w.id;
    // Delete the circle's run by a point on one of its facets.
    let on_facet = Point3::ORIGIN + (ring[10].to_vec() + ring[11].to_vec()) * 0.5;
    let (report, _) =
        delete_unit_at(&mut w.doc, None, id, plane, on_facet).expect("the circle erases");
    assert!(report.kept_as_sketch.is_empty());
    assert_eq!(w.doc.undo_depth(), depth0 + 1, "one undo step");
    assert!(
        objects_equivalent(w.obj(), &before),
        "the top is the plain square again (the rim vertex healed away)"
    );
    assert_eq!(w.doc.save(), bytes_before, "byte for byte the undrawn box");
    assert_exact_history(&mut w.doc, &bytes0);
}

/// The tangent circle drawn over a rectangle imprint that clips its right
/// side: the part of the disc left of the rectangle is a face in the
/// parent's hole, and the run it shares with that hole is the arc from one
/// clip point, around through the rim vertex where the hole touches the
/// parent's outer loop, to the other. Returns the wall, the clipped disc,
/// and the parent.
fn tangent_circle_clipped_by_an_imprint() -> (Wall, FaceId, FaceId) {
    let mut w = top_2x2();
    let seed = w.face;
    let rect = w.rect(1.25, 0.03, 0.55, 1.37);
    draw_region(&mut w, seed, rect, None);
    let (ring, g) = tangent_circle(&w);
    let seed = face_at(&w, 0.2, 1.8);
    w.draw_loop(seed, &ring, Some(g)).expect("the circle draws");
    let disc = face_at(&w, 0.7, 0.5);
    let parent = face_at(&w, 0.2, 1.8);
    let obj = w.obj();
    obj.validate().expect("valid");
    let rim = w.at(1.0, 0.0);
    let on = |l: kernel::LoopId| {
        obj.loop_positions(l)
            .any(|p| p.approx_eq(rim, tol::POINT_MERGE))
    };
    let pf = &obj.faces()[parent];
    assert!(
        on(pf.outer_loop),
        "the rim vertex is on the parent's outline"
    );
    assert!(
        pf.inner_loops.len() == 1 && on(pf.inner_loops[0]),
        "the parent's one hole touches its outline at the rim vertex"
    );
    assert!(
        on(obj.faces()[disc].outer_loop),
        "the clipped disc passes it"
    );
    (w, disc, parent)
}

/// An edge of the arc the clipped disc shares with the parent's hole.
fn arc_edge(obj: &Object, disc: FaceId, parent: FaceId) -> kernel::EdgeId {
    obj.loop_half_edges(obj.faces()[disc].outer_loop)
        .find(|&h| {
            obj.half_edges()[h]
                .twin
                .is_some_and(|t| obj.loops()[obj.half_edges()[t].loop_id].face == parent)
        })
        .map(|h| obj.half_edges()[h].edge)
        .expect("the disc shares its arc with the parent")
}

#[test]
fn merging_a_clipped_tangent_disc_across_its_arc_undoes_and_redoes() {
    // The run the merge dissolves passes through the rim vertex, which the
    // parent's outline keeps. Its undo used to replay a single cut whose
    // interior ran through that boundary vertex, which `split_face`
    // refuses ("path point is not on the face").
    let (mut w, disc, parent) = tangent_circle_clipped_by_an_imprint();
    let bytes0 = w.doc.save();
    let id = w.id;
    let edge = arc_edge(w.obj(), disc, parent);
    w.doc
        .apply_object_op(id, KernelOp::MergeFaces { edge })
        .expect("the disc merges into the parent");
    let obj = w.obj();
    obj.validate().expect("valid after the merge");
    assert!(
        !has_touching_loops(obj, &w.plane),
        "the hole no longer touches the outline"
    );
    let rim = w.at(1.0, 0.0);
    assert!(
        obj.vertices()
            .values()
            .all(|v| !v.position.approx_eq(rim, tol::POINT_MERGE)),
        "the rim vertex heals away, as erasing the whole circle heals it"
    );
    assert_exact_history(&mut w.doc, &bytes0);
}

#[test]
fn erasing_a_clipped_tangent_disc_arc_agrees_with_the_merge() {
    // The user's way to the same result: erase the arc's edges in one
    // draft. It lands on the geometry the recorded merge gives, so neither
    // leaves a scar the other heals.
    let (mut w, disc, parent) = tangent_circle_clipped_by_an_imprint();
    let obj = w.obj();
    let arc: Vec<(Point3, Point3)> = obj
        .loop_half_edges(obj.faces()[disc].outer_loop)
        .filter(|&h| {
            obj.half_edges()[h]
                .twin
                .is_some_and(|t| obj.loops()[obj.half_edges()[t].loop_id].face == parent)
        })
        .map(|h| obj.edge_endpoints(obj.half_edges()[h].edge).expect("live"))
        .collect();
    let mut merged = obj.clone();
    merged
        .merge_faces(arc_edge(obj, disc, parent))
        .expect("merges");
    let bytes0 = w.doc.save();
    let (plane, id) = (w.plane, w.id);
    let objs = plane_objects(&w.doc, plane, None, id);
    w.doc
        .draft(plane, None, &objs, "Erase", |sk| {
            for &(a, b) in &arc {
                let e = sk.edge_at_positions(a, b).expect("an arc edge in the lift");
                sk.remove_edge(e)?;
            }
            Ok(())
        })
        .expect("the arc erases");
    w.obj().validate().expect("valid after the erase");
    assert!(
        objects_equivalent(w.obj(), &merged),
        "the erase and the merge give the same face"
    );
    assert_exact_history(&mut w.doc, &bytes0);
}

// -------------------------------------------------------------- 4. move

#[test]
fn moving_the_circle_off_the_edge_makes_an_ordinary_hole() {
    let (mut w, ring, _, _) = top_with_tangent_circle();
    let bytes0 = w.doc.save();
    let (plane, id) = (w.plane, w.id);
    let on_facet = Point3::ORIGIN + (ring[10].to_vec() + ring[11].to_vec()) * 0.5;
    let up = Transform::translation(w.v * 0.2);
    move_unit_at(&mut w.doc, None, id, plane, on_facet, &up).expect("moves");
    let obj = w.obj();
    obj.validate().expect("valid");
    assert_eq!(group_faces(obj, &w.plane).len(), 2);
    assert!(!has_touching_loops(obj, &w.plane), "an ordinary hole now");
    assert_every_region_is_a_face(&w);
    assert_exact_history(&mut w.doc, &bytes0);
}

#[test]
fn moving_an_inside_circle_onto_the_edge_makes_it_touch() {
    let mut w = top_2x2();
    let (ring, g) = tool_circle(&w, 1.0, 0.7, 1.0, 0.2);
    let seed = w.face;
    draw_and_check(&mut w, seed, ring.clone(), Some(g));
    assert!(!has_touching_loops(w.obj(), &w.plane));
    let bytes0 = w.doc.save();
    let (plane, id) = (w.plane, w.id);
    let on_facet = Point3::ORIGIN + (ring[10].to_vec() + ring[11].to_vec()) * 0.5;
    let down = Transform::translation(w.v * -0.2);
    move_unit_at(&mut w.doc, None, id, plane, on_facet, &down).expect("moves");
    let obj = w.obj();
    obj.validate().expect("valid");
    assert_eq!(group_faces(obj, &w.plane).len(), 2);
    assert!(has_touching_loops(obj, &w.plane), "touching the edge now");
    assert_every_region_is_a_face(&w);
    assert_exact_history(&mut w.doc, &bytes0);
}

#[test]
fn moving_the_circle_across_the_edge_splits_a_lens() {
    let (mut w, ring, _, _) = top_with_tangent_circle();
    let bytes0 = w.doc.save();
    let (plane, id) = (w.plane, w.id);
    let on_facet = Point3::ORIGIN + (ring[10].to_vec() + ring[11].to_vec()) * 0.5;
    let down = Transform::translation(w.v * -0.2);
    move_unit_at(&mut w.doc, None, id, plane, on_facet, &down).expect("moves");
    let obj = w.obj();
    obj.validate().expect("valid");
    assert_eq!(group_faces(obj, &w.plane).len(), 2, "the lens and the rest");
    assert!(!has_touching_loops(obj, &w.plane));
    assert!(
        group_faces(obj, &w.plane)
            .iter()
            .all(|&f| obj.faces()[f].inner_loops.is_empty()),
        "the circle crosses the edge: no hole"
    );
    assert_exact_history(&mut w.doc, &bytes0);
}

// ------------------------------------------------------- 5. persistence

#[test]
fn a_touching_hole_saves_and_loads_byte_identically() {
    let (w, _, _, _) = top_with_tangent_circle();
    let bytes = w.doc.save();
    let reopened = Document::load(&bytes).expect("reopens");
    assert_eq!(reopened.save(), bytes, "save after load is identical");
    let obj = reopened.object(w.id).expect("the box");
    obj.validate().expect("valid");
    assert!(
        any_touching_loops(obj),
        "the touching hole survives the file"
    );
}

#[test]
fn the_bytes_do_not_depend_on_the_drawing_order() {
    // Two circles touching, drawn in either order, and the tangent circle
    // with them: one canonical file.
    let build = |first_a: bool| -> (Vec<u8>, ObjectId) {
        let mut w = top_2x2();
        let r = 0.3;
        let a = w.ring(0.6, 1.0, r, 24, 0.0);
        let mut b = w.ring(1.2, 1.0, r, 24, 0.0);
        b[12] = a[0];
        let (ga, gb) = (w.circle_geom(0.6, 1.0, r), w.circle_geom(1.2, 1.0, r));
        let shapes = if first_a {
            [(a, ga), (b, gb)]
        } else {
            [(b, gb), (a, ga)]
        };
        for (ring, g) in shapes {
            let seed = face_at(&w, 1.8, 1.8);
            w.draw_loop(seed, &ring, Some(g)).expect("draws");
        }
        let (ring, g) = tangent_circle(&w);
        let seed = face_at(&w, 1.8, 1.8);
        w.draw_loop(seed, &ring, Some(g)).expect("draws");
        (w.doc.save(), w.id)
    };
    let (x, ix) = build(true);
    let (y, iy) = build(false);
    // The object's geometry record is canonical: drawn in either order, it
    // encodes the same.
    let ox = Document::load(&x).expect("x");
    let oy = Document::load(&y).expect("y");
    let (gx, gy) = (
        ox.object(ix).expect("x obj").encode(&|_| 0),
        oy.object(iy).expect("y obj").encode(&|_| 0),
    );
    assert_eq!(gx, gy, "the geometry encodes the same either way");
}

// ------------------------------------------------------------- 8. split

/// Splits `face` along `path` through the document, as the Line tool's cut
/// does, and checks the result validates with exact history.
fn split_and_check(w: &mut Wall, face: FaceId, path: Vec<Point3>) {
    let bytes0 = w.doc.save();
    w.doc
        .apply_object_op(
            w.id,
            KernelOp::SplitFace {
                face,
                path,
                restore: None,
                curves: Vec::new(),
            },
        )
        .unwrap_or_else(|e| panic!("the cut refused: {e:?}"));
    w.obj().validate().expect("the split validates");
    assert_exact_history(&mut w.doc, &bytes0);
}

#[test]
fn a_cut_across_the_parent_keeps_the_touching_hole_whichever_way_it_runs() {
    // A circle tangent to each edge of the top, and a cut edge to edge
    // clear of it. The hole touches the outer loop of the result face that
    // holds it at the rim, and a strict inside test reads that one vertex
    // either way depending on the edge; the hole's side cannot depend on
    // the edge or on the cut's direction (a redo replays the cut reversed).
    // (center, rim, cut ends, a point beside the circle, a point across the
    // cut)
    type Case = (
        (f64, f64),
        (f64, f64),
        [(f64, f64); 2],
        (f64, f64),
        (f64, f64),
    );
    let cases: [Case; 4] = [
        (
            (1.0, 0.5),
            (1.0, 0.0),
            [(0.0, 1.5), (2.0, 1.5)],
            (0.2, 0.2),
            (0.2, 1.8),
        ),
        (
            (0.5, 1.0),
            (0.0, 1.0),
            [(1.5, 0.0), (1.5, 2.0)],
            (0.2, 0.2),
            (1.8, 1.8),
        ),
        (
            (1.0, 1.5),
            (1.0, 2.0),
            [(0.0, 0.5), (2.0, 0.5)],
            (0.2, 1.8),
            (0.2, 0.2),
        ),
        (
            (1.5, 1.0),
            (2.0, 1.0),
            [(0.5, 0.0), (0.5, 2.0)],
            (1.8, 0.2),
            (0.2, 0.2),
        ),
    ];
    for ((cx, cy), (rx, ry), [p, q], beside, across) in cases {
        for forward in [true, false] {
            let what = format!("rim ({rx}, {ry}), forward {forward}");
            let mut w = top_2x2();
            let (ring, g) = tool_circle(&w, cx, cy, rx, ry);
            let seed = w.face;
            let disc = draw_and_check(&mut w, seed, ring.clone(), Some(g));
            let parent = parent_of(&w, disc);
            let (a, b) = (w.at(p.0, p.1), w.at(q.0, q.1));
            split_and_check(
                &mut w,
                parent,
                if forward { vec![a, b] } else { vec![b, a] },
            );
            let obj = w.obj();
            let holder = face_at(&w, beside.0, beside.1);
            let other = face_at(&w, across.0, across.1);
            assert_ne!(holder, other, "the cut split the parent: {what}");
            let holes: Vec<Vec<Point3>> = obj.faces()[holder]
                .inner_loops
                .iter()
                .map(|&h| ring_of(obj, h))
                .collect();
            assert_eq!(
                holes.len(),
                1,
                "the face beside the circle keeps the hole: {what}"
            );
            assert!(
                cyclic_match(&holes[0], &ring, true),
                "the hole is the circle: {what}"
            );
            assert!(
                ring_of(obj, obj.faces()[holder].outer_loop).contains(&w.at(rx, ry)),
                "still touching at the rim: {what}"
            );
            assert!(obj.faces()[other].inner_loops.is_empty(), "{what}");
        }
    }
}

#[test]
fn a_cut_around_a_hole_takes_a_second_hole_touching_it() {
    // Two circles touching at a point, then a cut from the first circle
    // back to it around the second: the region between the cut and the
    // first circle takes the second hole, which touches that region's
    // outer loop at the shared vertex. The layout is turned through each
    // quarter about the top's center, so the shared vertex meets the cut
    // region's boundary from every side.
    let r = 0.3;
    let a2 = (-r + r * (PI / 6.0).cos(), r * (PI / 6.0).sin());
    for quarter in 0..4 {
        let turn = quarter as f64 * PI / 2.0;
        // Local frame: the first circle about (-r, 0), the second about
        // (r, 0), touching at the origin.
        let at = |x: f64, y: f64| {
            (
                1.0 + x * turn.cos() - y * turn.sin(),
                1.0 + x * turn.sin() + y * turn.cos(),
            )
        };
        for forward in [true, false] {
            let what = format!("quarter {quarter}, forward {forward}");
            let mut w = top_2x2();
            let (ca, cb) = (at(-r, 0.0), at(r, 0.0));
            let a = w.ring(ca.0, ca.1, r, 24, turn);
            let mut b = w.ring(cb.0, cb.1, r, 24, turn);
            assert!((b[12] - a[0]).length() < 1e-12, "{what}");
            b[12] = a[0];
            for (ring, g) in [
                (a.clone(), w.circle_geom(ca.0, ca.1, r)),
                (b.clone(), w.circle_geom(cb.0, cb.1, r)),
            ] {
                let (x, y) = at(-0.8, -0.8);
                let seed = face_at(&w, x, y);
                w.draw_loop(seed, &ring, Some(g)).expect("draws");
            }
            let (rx, ry) = at(-0.8, -0.8);
            let top = face_at(&w, rx, ry);
            assert_eq!(
                w.obj().faces()[top].inner_loops.len(),
                2,
                "two holes: {what}"
            );
            let mut path = vec![a[2]];
            for (x, y) in [(a2.0, 0.45), (0.65, 0.45), (0.65, -0.45), (a2.0, -0.45)] {
                let (u, v) = at(x, y);
                path.push(w.at(u, v));
            }
            path.push(a[22]);
            if !forward {
                path.reverse();
            }
            split_and_check(&mut w, top, path);
            let obj = w.obj();
            let (lx, ly) = at(r, 0.4);
            let lune = face_at(&w, lx, ly);
            let rest = face_at(&w, rx, ry);
            assert_ne!(lune, rest, "the cut split the top: {what}");
            let held: Vec<Vec<Point3>> = obj.faces()[lune]
                .inner_loops
                .iter()
                .map(|&h| ring_of(obj, h))
                .collect();
            assert_eq!(held.len(), 1, "the lune holds the second hole: {what}");
            assert!(cyclic_match(&held[0], &b, true), "{what}");
            // The rest keeps the first circle, rerouted along the cut.
            assert_eq!(obj.faces()[rest].inner_loops.len(), 1, "{what}");
        }
    }
}

/// A point strictly inside the region of `outer` less `holes`: the support
/// module's interior point of the outer ring, nudged off any hole by
/// scanning the outer ring's edge midpoints inward.
fn kernel_interior(outer: &[Point3], holes: &[Vec<Point3>], n: Vec3) -> Point3 {
    let inside_holes = |p: Point3| holes.iter().any(|h| winds(h, p, n));
    let c = interior_point(outer, n);
    if !inside_holes(c) {
        return c;
    }
    for i in 0..outer.len() {
        let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
        let inward = n.cross(b - a).normalized().expect("an edge");
        let mid = Point3::ORIGIN + (a.to_vec() + b.to_vec()) * 0.5;
        for k in 1..200 {
            let p = mid + inward * (1e-3 * f64::from(k));
            if winds(outer, p, n) && !inside_holes(p) {
                return p;
            }
        }
    }
    panic!("no interior point");
}

/// Whether `p` is inside the ring (crossing parity in the ring's plane).
fn winds(ring: &[Point3], p: Point3, n: Vec3) -> bool {
    let (u, v) = plane_basis(&Plane::from_polygon(ring).expect("a ring plane"));
    let _ = n;
    let xy = |q: Point3| ((q - p).dot(u), (q - p).dot(v));
    let mut inside = false;
    for i in 0..ring.len() {
        let (a, b) = (xy(ring[i]), xy(ring[(i + 1) % ring.len()]));
        if (a.1 > 0.0) != (b.1 > 0.0) && 0.0 < a.0 + (0.0 - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            inside = !inside;
        }
    }
    inside
}

// ---------------------------------------------------------- 7. other ops

/// An axis-aligned box from `lo` to `hi`.
fn cube(lo: [f64; 3], hi: [f64; 3]) -> Object {
    let p = |x: usize, y: usize, z: usize| {
        Point3::new([lo[0], hi[0]][x], [lo[1], hi[1]][y], [lo[2], hi[2]][z])
    };
    Object::from_polygons(
        &[
            p(0, 0, 0),
            p(1, 0, 0),
            p(1, 1, 0),
            p(0, 1, 0),
            p(0, 0, 1),
            p(1, 0, 1),
            p(1, 1, 1),
            p(0, 1, 1),
        ],
        &[
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
    .expect("a box")
}

/// Every piece valid and watertight.
fn assert_valid_solids(what: &str, objs: &[&Object]) {
    for o in objs {
        o.validate()
            .unwrap_or_else(|e| panic!("{what}: an invalid result: {e:?}"));
        assert_eq!(o.watertight(), WatertightState::Watertight, "{what}");
    }
}

/// A plane through `p` with normal `n`.
fn plane_at(p: Point3, n: Vec3) -> Plane {
    Plane::from_point_normal(p, n).expect("plane")
}

#[test]
fn transform_slice_and_booleans_on_a_touching_hole() {
    let (w, _, _, _) = top_with_tangent_circle();
    let obj = w.obj().clone();

    // Rigid moves carry the touching hole.
    let mut moved = obj.clone();
    moved
        .apply_transform(&Transform::translation(Vec3::new(0.3, -0.2, 0.1)))
        .expect("a rigid move applies");
    assert_valid_solids("move", &[&moved]);
    assert!(any_touching_loops(&moved));
    let mut turned = obj.clone();
    turned
        .apply_transform(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7).expect("a rotation"))
        .expect("a rotation applies");
    assert_valid_solids("rotate", &[&turned]);
    assert!(any_touching_loops(&turned));

    // Slices away from the touch point succeed.
    for (what, plane) in [
        (
            "slice beside the disc",
            plane_at(Point3::new(0.3, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        ),
        (
            "slice across the disc, clear of the touch",
            plane_at(Point3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 1.0, 0.0)),
        ),
        (
            "slice under the top",
            plane_at(Point3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, 1.0)),
        ),
    ] {
        let (a, b) = obj
            .slice(&plane)
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_valid_solids(what, &[&a, &b]);
    }
    // Through the touch point itself.
    let (a, b) = obj
        .slice(&plane_at(
            Point3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ))
        .expect("a slice through the touch point");
    assert_valid_solids("slice through the touch point", &[&a, &b]);

    // Booleans with a disjoint box, a box overlapping the disc, and a box
    // straddling the touch point all succeed.
    let disjoint = cube([3.0, 0.0, 0.0], [4.0, 1.0, 1.0]);
    let r = Object::boolean(BooleanOp::Union, &obj, &disjoint, &Transform::IDENTITY)
        .expect("a union with a disjoint box");
    assert_valid_solids("disjoint union", &[&r]);
    let over_disc = cube([0.8, 0.3, 0.5], [1.2, 0.7, 1.5]);
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let r = Object::boolean(op, &obj, &over_disc, &Transform::IDENTITY)
            .unwrap_or_else(|e| panic!("{op:?} with a box over the disc: {e:?}"));
        assert_valid_solids(&format!("{op:?} over the disc"), &[&r]);
    }
    let at_touch = cube([0.8, -0.2, 0.6], [1.2, 0.2, 1.4]);
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let r = Object::boolean(op, &obj, &at_touch, &Transform::IDENTITY)
            .unwrap_or_else(|e| panic!("{op:?} with a box at the touch: {e:?}"));
        assert_valid_solids(&format!("{op:?} at the touch"), &[&r]);
    }
}

#[test]
fn slice_and_booleans_on_the_pocketed_solid() {
    // The pocket's wall touches the front side face along the generatrix
    // under the touch point: a slice through that line, slices clear of it
    // and across it, a disjoint union, and every boolean with a box
    // straddling it all give valid solids.
    let w = pocketed();
    let obj = w.obj().clone();
    let (a, b) = obj
        .slice(&plane_at(
            Point3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ))
        .expect("a slice through the generatrix");
    assert_valid_solids("slice through the generatrix", &[&a, &b]);
    let (a, b) = obj
        .slice(&plane_at(
            Point3::new(0.3, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ))
        .expect("a slice clear of the pocket");
    assert_valid_solids("slice clear of the pocket", &[&a, &b]);
    let (a, b) = obj
        .slice(&plane_at(
            Point3::new(0.0, 0.0, 0.7),
            Vec3::new(0.0, 0.0, 1.0),
        ))
        .expect("a slice across the pocket, through the generatrix's middle");
    assert_valid_solids("slice across the pocket", &[&a, &b]);
    let r = Object::boolean(
        BooleanOp::Union,
        &obj,
        &cube([3.0, 0.0, 0.0], [4.0, 1.0, 1.0]),
        &Transform::IDENTITY,
    )
    .expect("a union with a disjoint box");
    assert_valid_solids("disjoint union", &[&r]);
    let straddle = cube([0.8, -0.2, 0.3], [1.2, 0.2, 0.8]);
    for op in [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect] {
        let r = Object::boolean(op, &obj, &straddle, &Transform::IDENTITY)
            .unwrap_or_else(|e| panic!("{op:?} with a box across the generatrix: {e:?}"));
        assert_valid_solids(&format!("{op:?} across the generatrix"), &[&r]);
    }
}

#[test]
fn make_component_and_explode_carry_the_touching_hole() {
    let (mut w, _, _, _) = top_with_tangent_circle();
    let id = w.id;
    let bytes0 = w.doc.save();
    let (component, instance, _) = w
        .doc
        .make_component(&[kernel::NodeId::Object(id)])
        .expect("make component");
    let members = w.doc.def_members(component).expect("members");
    for m in &members {
        let obj = w.doc.object(*m).expect("member");
        obj.validate().expect("valid member");
        assert!(
            any_touching_loops(obj),
            "the member keeps its touching hole"
        );
    }
    let (exploded, _) = w.doc.explode_instance(instance).expect("explode");
    assert!(!exploded.is_empty());
    for o in exploded {
        let obj = w.doc.object(o).expect("exploded object");
        obj.validate().expect("valid");
        assert!(any_touching_loops(obj));
    }
    let after = w.doc.save();
    assert_reopens(&after);
    w.doc.undo().expect("undo explode");
    w.doc.undo().expect("undo make component");
    assert_eq!(w.doc.save(), bytes0);
}

#[test]
fn the_tangent_lattice_never_corrupts() {
    // A circle about a lattice point with its rim on one of the four edges
    // along an axis (exact tangency) or off them, at several radii; drawn,
    // pushed by the app's routing, and undone.
    let mut drawn = 0;
    let mut touching = 0;
    for &(cx, cy) in &[(1.0, 0.5), (0.5, 0.5), (1.5, 0.3), (0.25, 1.0)] {
        for &(rx, ry) in &[(cx, 0.0), (0.0, cy), (cx + 0.1, cy), (cx, cy - 0.2)] {
            let mut w = top_2x2();
            let (ring, g) = tool_circle(&w, cx, cy, rx, ry);
            let r = g.radius;
            if cx - r < -tol::POINT_MERGE
                || cy - r < -tol::POINT_MERGE
                || cx + r > 2.0
                || cy + r > 2.0
            {
                continue;
            }
            let seed = w.face;
            draw_and_check(&mut w, seed, ring.clone(), Some(g));
            drawn += 1;
            let touches_once = has_touching_loops(w.obj(), &w.plane);
            if touches_once {
                touching += 1;
            }
            // Every face of the top (the disc, and the face or faces around
            // it), pulled, pushed, and undone. With a touching hole every
            // push succeeds; a circle tangent to two edges splits the top
            // instead, and a corner piece pushed in would meet the side
            // faces along lines from no touch point, which may refuse typed
            // with the document untouched (as before this rule).
            let faces: Vec<(Point3, f64)> = group_faces(w.obj(), &w.plane)
                .into_iter()
                .map(|f| {
                    let face = &w.obj().faces()[f];
                    let outer: Vec<Point3> = w.obj().loop_positions(face.outer_loop).collect();
                    let holes: Vec<Vec<Point3>> = face
                        .inner_loops
                        .iter()
                        .map(|&l| w.obj().loop_positions(l).collect())
                        .collect();
                    let inside = kernel_interior(&outer, &holes, w.plane.normal());
                    (inside, net_area(w.obj(), f))
                })
                .collect();
            for (inside, net) in faces {
                for d in [0.3, -0.4] {
                    let f = group_faces(w.obj(), &w.plane)
                        .into_iter()
                        .find(|&f| w.obj().face_contains_point(f, inside))
                        .expect("the face again");
                    if touches_once || d > 0.0 {
                        push_and_check(&mut w, f, d, d * net);
                        w.doc.undo().expect("undo");
                    } else {
                        let bytes0 = w.doc.save();
                        let id = w.id;
                        let op = if w.obj().is_flat_sub_face(f) {
                            KernelOp::ExtrudeSubFace {
                                sub_face: f,
                                distance: d,
                            }
                        } else {
                            KernelOp::PushPull {
                                face: f,
                                distance: d,
                            }
                        };
                        match w.doc.apply_object_op(id, op) {
                            Ok(_) => {
                                w.obj().validate().expect("valid");
                                w.doc.undo().expect("undo");
                            }
                            Err(e) => {
                                // Only the interference guard's refusal: a
                                // wall meeting a side face along a line from
                                // no touch point.
                                assert!(
                                    matches!(
                                        e,
                                        kernel::DocumentError::Op(kernel::KernelOpError::PushPull(
                                            kernel::PushPullError::NonManifoldResult
                                        ))
                                    ),
                                    "refused as {e:?}"
                                );
                                assert_eq!(w.doc.save(), bytes0, "refused untouched");
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        drawn >= 10 && touching >= 3,
        "{drawn} drawn, {touching} touching"
    );
}

#[test]
fn a_ground_region_with_a_touching_hole_refuses_extrusion_typed_but_drags_freely() {
    // The ordinary gesture: a ground rectangle, a circle inside it with its
    // rim on one side. Pulling the rectangle's region refuses
    // `RegionTouchesHole` (never the "couldn't be traced" tracing failure),
    // the document untouched; the disc pulls; and dragging a corner of the
    // rectangle in place is no refusal (the touch is a region the tracer
    // makes on purpose).
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let sq = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)];
    let mut ring: Vec<Point3> = (0..72)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 72.0;
            p(1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
        })
        .collect();
    ring[0] = p(1.0, 0.0);
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(sq[i], sq[(i + 1) % 4]).expect("square");
        }
        for i in 0..72 {
            sk.add_segment(ring[i], ring[(i + 1) % 72]).expect("circle");
        }
    }
    let sk = doc.sketch(s).expect("sketch");
    let around = sk
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("the region around the circle");
    let disc = sk
        .regions()
        .iter()
        .find(|(_, r)| r.holes.is_empty())
        .map(|(id, _)| id)
        .expect("the disc");
    let bytes0 = doc.save();
    let depth0 = doc.undo_depth();
    let err = doc.extrude_region(s, around, 0.5).expect_err("refuses");
    assert_eq!(
        err,
        kernel::DocumentError::Sketch(kernel::SketchError::RegionTouchesHole)
    );
    assert_eq!(doc.save(), bytes0, "the document is untouched");
    assert_eq!(doc.undo_depth(), depth0, "no undo entry");
    let (id, _) = doc.extrude_region(s, disc, 0.5).expect("the disc pulls");
    doc.object(id).expect("disc").validate().expect("valid");
    doc.undo().expect("undo");
    let corner = doc
        .sketch(s)
        .expect("sketch")
        .vertices()
        .iter()
        .find(|(_, v)| v.position == p(2.0, 2.0))
        .map(|(id, _)| id)
        .expect("the far corner");
    doc.sketch_mut(s)
        .expect("sketch")
        .move_vertex(corner, p(2.2, 2.1))
        .expect("an in-place drag of an unrelated corner");
}

/// What a face of the swept solid is, for the expected-outcome table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SweptKind {
    /// A plane face (no analytic surface) other than the tangent side face.
    Planar,
    /// A facet of a pocket's or tunnel's cylinder wall.
    Wall,
    /// The side face the wall is tangent to (normal `-v`).
    TangentSide,
}

/// One push/pull attempt of the sweep: the face's kind, the distance,
/// whether it was routed as the app routes it, and the refusal (`None`
/// when it committed).
type Attempt = (SweptKind, f64, bool, Option<kernel::DocumentError>);

/// Every face of `w`'s object (located again by an interior point after
/// each undo), pushed and pulled by 0.2 both as the raw kernel op and as the
/// app routes it (past an opposite wall, a push-through boolean). Each
/// attempt either commits a valid watertight solid whose volume grew by the
/// swept prism (a pull) or shrank by at most it (a push) and undoes
/// byte-exactly, or refuses with the document untouched. Never a panic.
/// Returns every attempt, for the caller's expected-outcome table.
fn sweep_every_face(w: &mut Wall) -> Vec<Attempt> {
    let id = w.id;
    let v = w.v;
    let probes: Vec<(Point3, Vec3, f64, SweptKind)> = {
        let obj = w.obj();
        obj.faces()
            .keys()
            .map(|f| {
                let face = &obj.faces()[f];
                let outer: Vec<Point3> = obj.loop_positions(face.outer_loop).collect();
                let holes: Vec<Vec<Point3>> = face
                    .inner_loops
                    .iter()
                    .map(|&l| obj.loop_positions(l).collect())
                    .collect();
                let n = face.plane.normal();
                let kind = if face.surface.is_some() {
                    SweptKind::Wall
                } else if (n + v).length() < 1e-9 {
                    SweptKind::TangentSide
                } else {
                    SweptKind::Planar
                };
                (
                    kernel_interior(&outer, &holes, n),
                    n,
                    net_area(obj, f),
                    kind,
                )
            })
            .collect()
    };
    let mut attempts = Vec::new();
    for (inside, n, net, kind) in probes {
        for d in [0.2, -0.2] {
            for routed in [false, true] {
                let obj = w.obj();
                let f = obj
                    .faces()
                    .keys()
                    .find(|&f| {
                        (obj.faces()[f].plane.normal() - n).length() < 1e-9
                            && obj.face_contains_point(f, inside)
                    })
                    .expect("the face again");
                let bytes0 = w.doc.save();
                let v0 = volume(obj);
                let result: Result<Vec<ObjectId>, kernel::DocumentError> =
                    if routed && obj.push_pull_overshoots(f, d) {
                        w.doc.push_pull_through(id, f, d).map(|(r, _)| r)
                    } else {
                        let op = if obj.is_flat_sub_face(f) {
                            KernelOp::ExtrudeSubFace {
                                sub_face: f,
                                distance: d,
                            }
                        } else {
                            KernelOp::PushPull {
                                face: f,
                                distance: d,
                            }
                        };
                        w.doc.apply_object_op(id, op).map(|_| vec![id])
                    };
                match result {
                    Ok(results) => {
                        let mut v1 = 0.0;
                        for &r in &results {
                            let o = w.doc.object(r).expect("result");
                            o.validate().expect("a committed result validates");
                            assert_eq!(o.watertight(), WatertightState::Watertight);
                            v1 += volume(o);
                        }
                        // A pull adds the swept prism; a push removes at
                        // most it (less where the prism crosses the pocket's
                        // or tunnel's void), never nothing.
                        let change = v1 - v0;
                        let ok = if d > 0.0 {
                            (change - d * net).abs() < 1e-9
                        } else {
                            change < -1e-9 && change >= d * net - 1e-9
                        };
                        assert!(ok, "volume {v0} -> {v1}, swept prism {}", d * net);
                        assert_exact_history(&mut w.doc, &bytes0);
                        w.doc.undo().expect("undo");
                        assert_eq!(w.doc.save(), bytes0);
                        attempts.push((kind, d, routed, None));
                    }
                    Err(e) => {
                        assert_eq!(w.doc.save(), bytes0, "a refusal changes nothing");
                        attempts.push((kind, d, routed, Some(e)));
                    }
                }
            }
        }
    }
    attempts
}

/// The expected outcome of every attempt on a solid whose cylinder wall is
/// tangent to its front side face: every plane face commits, raw and
/// routed, both ways; every wall facet refuses, pulled out
/// `WallNeighborNonPlanar` (the radial offset would bend the faces the
/// wall's touch vertex holds) and pushed in `NonManifoldResult`, raw and
/// routed; the tangent side face refuses raw `NonManifoldResult` both ways
/// (its in-place translation would bend the wall facets holding the touch
/// vertex) and commits routed (the boolean). Returns the attempt counts per
/// kind, for the caller to pin.
fn assert_tangent_sweep_table(
    attempts: &[Attempt],
) -> std::collections::BTreeMap<SweptKind, usize> {
    use kernel::{DocumentError::Op, KernelOpError::PushPull, PushPullError as P};
    let mut per_kind = std::collections::BTreeMap::new();
    for (kind, d, routed, refusal) in attempts {
        *per_kind.entry(*kind).or_insert(0) += 1;
        let what = format!("{kind:?} d={d} routed={routed}: {refusal:?}");
        match kind {
            SweptKind::Planar => assert!(refusal.is_none(), "{what}"),
            SweptKind::Wall => {
                let want = if *d > 0.0 {
                    P::WallNeighborNonPlanar
                } else {
                    P::NonManifoldResult
                };
                assert_eq!(*refusal, Some(Op(PushPull(want))), "{what}");
            }
            SweptKind::TangentSide => {
                if *routed {
                    assert!(refusal.is_none(), "{what}");
                } else {
                    assert_eq!(*refusal, Some(Op(PushPull(P::NonManifoldResult))), "{what}");
                }
            }
        }
    }
    per_kind
}

/// The pocketed box: the tangent disc pushed down 0.5.
fn pocketed() -> Wall {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let area = signed_area(&ring, w.plane.normal());
    push_and_check(&mut w, disc, -0.5, -0.5 * area);
    w
}

/// The side face the pocket's wall is tangent to (normal -v at v = 0).
fn tangent_side(w: &Wall) -> FaceId {
    let obj = w.obj();
    obj.faces()
        .iter()
        .find(|(_, f)| (f.plane.normal() + w.v).length() < 1e-9)
        .map(|(f, _)| f)
        .expect("the front side face")
}

#[test]
fn every_face_of_the_pocketed_solid_pushes_pulls_or_refuses_typed() {
    let mut w = pocketed();
    let attempts = sweep_every_face(&mut w);
    let per_kind = assert_tangent_sweep_table(&attempts);
    // Six plane faces (the top around the pocket, the floor, the bottom,
    // three side faces) plus the tangent side face, and the 72 wall facets,
    // four attempts each.
    assert_eq!(
        per_kind,
        std::collections::BTreeMap::from([
            (SweptKind::Planar, 6 * 4),
            (SweptKind::Wall, 72 * 4),
            (SweptKind::TangentSide, 4),
        ])
    );
}

#[test]
fn pulling_the_side_face_the_pocket_is_tangent_to_never_panics() {
    // The raw push/pull of the side face would translate the touch vertex,
    // which the pocket wall's facets hold without sharing an edge with the
    // side face: it refuses typed before any mutation, either way.
    let mut w = pocketed();
    let id = w.id;
    for d in [0.2, 0.5, -0.2, -0.6] {
        let side = tangent_side(&w);
        let bytes0 = w.doc.save();
        let depth0 = w.doc.undo_depth();
        let r = w.doc.apply_object_op(
            id,
            KernelOp::PushPull {
                face: side,
                distance: d,
            },
        );
        assert!(
            matches!(
                r,
                Err(kernel::DocumentError::Op(kernel::KernelOpError::PushPull(
                    kernel::PushPullError::NonManifoldResult
                )))
            ),
            "{d}: {:?}",
            r.map(|_| ())
        );
        assert_eq!(w.doc.save(), bytes0, "refused untouched");
        assert_eq!(w.doc.undo_depth(), depth0);
    }
    // As the app routes the outward pull, it succeeds: a ring pinned at the
    // touch vertex grows the solid as a union.
    let side = tangent_side(&w);
    let net = net_area(w.obj(), side);
    let results = push_and_check(&mut w, side, 0.2, 0.2 * net);
    assert_eq!(results.len(), 1);
}

#[test]
fn every_face_of_the_pushed_through_solid_pushes_pulls_or_refuses_typed() {
    let (mut w, ring, _, disc) = top_with_tangent_circle();
    let area = signed_area(&ring, w.plane.normal());
    let results = push_and_check(&mut w, disc, -1.0, -area);
    w.id = results[0];
    let attempts = sweep_every_face(&mut w);
    let per_kind = assert_tangent_sweep_table(&attempts);
    // Five plane faces (the top and bottom around the tunnel, three side
    // faces) plus the tangent side face, and the 72 tunnel wall facets,
    // four attempts each.
    assert_eq!(
        per_kind,
        std::collections::BTreeMap::from([
            (SweptKind::Planar, 5 * 4),
            (SweptKind::Wall, 72 * 4),
            (SweptKind::TangentSide, 4),
        ])
    );
}

/// Rewrites one entry of a `.hew` container, every other entry copied
/// byte-for-byte.
fn rewrite_entry(bytes: &[u8], name: &str, edit: impl FnOnce(Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a container");
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut edit = Some(edit);
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).expect("entry");
        let entry = f.name().expect("entry name").to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).expect("read");
        if entry == name {
            data = (edit.take().expect("one entry"))(data);
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        out.start_file(entry, options).expect("start");
        out.write_all(&data).expect("write");
    }
    out.finish().expect("finish").into_inner()
}

#[test]
fn an_older_file_with_a_ring_visiting_a_vertex_twice_loads_and_re_traces() {
    // Before rings were split at touch points, a ground circle tangent to a
    // rectangle's side was traced as ONE walk visiting the touch point
    // twice. Such a file still loads (the loader keeps regions as written),
    // and the sketch's next edit re-traces it into simple rings: the
    // rectangle's ring with the circle as a hole sharing that vertex.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let sq = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)];
    let mut ring: Vec<Point3> = (0..24)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 24.0;
            p(1.0 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
        })
        .collect();
    ring[0] = p(1.0, 0.0);
    {
        let sk = doc.sketch_mut(s).expect("sketch");
        for i in 0..4 {
            sk.add_segment(sq[i], sq[(i + 1) % 4]).expect("square");
        }
        for i in 0..24 {
            sk.add_segment(ring[i], ring[(i + 1) % 24]).expect("circle");
        }
    }
    // Rewrite the region holding the circle as the old single walk.
    let bytes = rewrite_entry(&doc.save(), "manifest.json", |m| {
        let mut v: serde_json::Value = serde_json::from_slice(&m).expect("json");
        let regions = v["sketches"][0]["regions"].as_array_mut().expect("regions");
        let r = regions
            .iter_mut()
            .find(|r| r["holes"].as_array().is_some_and(|h| !h.is_empty()))
            .expect("the region around the circle");
        let outer: Vec<u64> = r["outer"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        let hole: Vec<u64> = r["holes"][0]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        let t = *outer
            .iter()
            .find(|v| hole.contains(v))
            .expect("the touch vertex");
        let rot = |ring: &[u64]| -> Vec<u64> {
            let k = ring.iter().position(|&x| x == t).unwrap();
            ring[k..].iter().chain(&ring[..k]).copied().collect()
        };
        let (o, h) = (rot(&outer), rot(&hole));
        let walk: Vec<u64> = h.iter().chain(o.iter()).copied().collect();
        assert_eq!(walk.iter().filter(|&&x| x == t).count(), 2);
        r["outer"] = serde_json::json!(walk);
        r["holes"] = serde_json::json!([]);
        serde_json::to_vec(&v).expect("json")
    });
    let mut old = Document::load(&bytes).expect("an older file loads");
    let s = old.sketch_ids()[0];
    let pinched = |d: &Document| {
        d.sketch(s).unwrap().regions().values().any(|r| {
            let mut seen = std::collections::BTreeSet::new();
            !r.outer.iter().all(|v| seen.insert(*v))
        })
    };
    assert!(pinched(&old), "loaded as written");
    // The next edit: a loose line elsewhere.
    old.sketch_mut(s)
        .unwrap()
        .add_segment(p(3.0, 0.0), p(3.0, 1.0))
        .expect("an edit");
    assert!(!pinched(&old), "re-traced into simple rings");
    let sk = old.sketch(s).unwrap();
    assert!(
        sk.regions().values().any(|r| r
            .holes
            .iter()
            .any(|h| h.iter().any(|v| r.outer.contains(v)))),
        "the circle is a hole touching the rectangle's ring"
    );
}

#[test]
fn every_hew_fixture_in_the_repo_loads_under_the_stricter_validator() {
    // Every .hew file the repository carries (samples, fixtures, goldens)
    // loads, every object passes the validator that now requires simple
    // loops and no edge with one face on both sides, and saves back to the
    // bytes it reloads from.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs = [
        "crates/kernel/tests/fixtures",
        "crates/kernel/tests/golden",
        "crates/wasm-api/tests/fixtures",
        "crates/skp-export/tests/fixtures",
        "app/e2e/fixtures",
        "app/public/samples",
    ];
    let mut n = 0;
    for d in dirs {
        let entries = std::fs::read_dir(root.join(d))
            .unwrap_or_else(|e| panic!("fixture directory {d} is missing: {e}"));
        let mut paths: Vec<_> = entries
            .map(|e| e.expect("entry").path())
            .filter(|p| p.extension().is_some_and(|e| e == "hew"))
            .collect();
        paths.sort();
        for path in paths {
            let bytes = std::fs::read(&path).expect("read");
            let doc = Document::load(&bytes)
                .unwrap_or_else(|e| panic!("{}: does not load: {e:?}", path.display()));
            let mut ids = doc.visible_object_ids();
            for c in doc.component_ids() {
                ids.extend(doc.def_members(c).unwrap_or_default());
            }
            for id in ids {
                doc.object(id)
                    .expect("object")
                    .validate()
                    .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            }
            let again = doc.save();
            assert_eq!(
                Document::load(&again).expect("reloads").save(),
                again,
                "{}",
                path.display()
            );
            n += 1;
        }
    }
    // Every fixture the repository holds today; a new one raises the count,
    // a lost one fails here.
    assert!(n >= 10, "too few fixtures found: {n}");
}

#[test]
fn a_wall_meeting_a_side_face_along_a_line_from_no_touch_point_still_refuses() {
    // A chain from the top's front corners to its back edge's midpoint cuts
    // a triangle whose apex sits on that edge. Pushing the triangle in
    // drives its two slanted walls together along a line lying in the back
    // side face, from a vertex no face holds in two loops (not a touch
    // point): the interference guard still refuses that, typed, the
    // document untouched. (The same contract `op_specs` pins on a cube.)
    let mut w = top_2x2();
    let seed = w.face;
    let path = vec![w.at(0.0, 0.0), w.at(1.0, 2.0), w.at(2.0, 0.0)];
    let id = w.id;
    draw_path(&mut w.doc, None, id, seed, &path, &[]).expect("the chain cuts the top");
    assert_eq!(group_faces(w.obj(), &w.plane).len(), 3);
    assert!(!has_touching_loops(w.obj(), &w.plane));
    let middle = face_at(&w, 1.0, 0.5);
    let (bytes0, depth0) = (w.doc.save(), w.doc.undo_depth());
    for d in [-0.2, -0.6] {
        let r = w.doc.apply_object_op(
            id,
            KernelOp::PushPull {
                face: middle,
                distance: d,
            },
        );
        assert!(
            matches!(
                r,
                Err(kernel::DocumentError::Op(kernel::KernelOpError::PushPull(
                    kernel::PushPullError::NonManifoldResult
                )))
            ),
            "{d}: {:?}",
            r.map(|_| ())
        );
        assert_eq!(w.doc.save(), bytes0);
        assert_eq!(w.doc.undo_depth(), depth0);
    }
}

#[test]
fn the_pocket_touches_the_side_face_along_its_generatrix() {
    // The self-touching line is real topology: an edge of the pocket wall
    // (between its two facets at the touch point), from the touch point on
    // the side face's top edge down to the pocket floor, lying in the side
    // face's plane and inside it, but no edge of the side face.
    let w = pocketed();
    let obj = w.obj();
    let side = tangent_side(&w);
    let plane = obj.faces()[side].plane;
    let n = plane.normal();
    let ring: Vec<Point3> = obj.loop_positions(obj.faces()[side].outer_loop).collect();
    let side_edges: Vec<kernel::EdgeId> = obj
        .loop_half_edges(obj.faces()[side].outer_loop)
        .map(|h| obj.half_edges()[h].edge)
        .collect();
    let touch = w.at(1.0, 0.0);
    let lines: Vec<(Point3, Point3)> = obj
        .edges()
        .keys()
        .filter(|e| !side_edges.contains(e))
        .filter_map(|e| obj.edge_endpoints(e))
        .filter(|&(a, b)| {
            plane.signed_distance(a).abs() < 1e-12 && plane.signed_distance(b).abs() < 1e-12
        })
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "one wall edge lies in the side face's plane: {lines:?}"
    );
    let (a, b) = lines[0];
    let (top, bottom) = if a == touch { (a, b) } else { (b, a) };
    assert_eq!(top, touch, "it starts at the touch point");
    assert!(
        (bottom - (touch - Vec3::new(0.0, 0.0, 0.5))).length() < 1e-12,
        "and runs down to the pocket floor"
    );
    let mid = Point3::ORIGIN + (top.to_vec() + bottom.to_vec()) * 0.5;
    assert!(winds(&ring, mid, n), "inside the side face");
    // Both faces on that edge are the pocket wall's facets, on the solid's
    // side of the side face.
    let e = obj
        .edges()
        .keys()
        .find(|&e| obj.edge_endpoints(e) == Some((a, b)))
        .unwrap();
    let edge = obj.edges()[e];
    for h in [edge.half_edge, edge.twin_half_edge.expect("interior")] {
        let f = obj.loops()[obj.half_edges()[h].loop_id].face;
        assert!(obj.faces()[f].surface.is_some(), "a wall facet");
        assert!(
            obj.loop_positions(obj.faces()[f].outer_loop)
                .all(|p| plane.signed_distance(p) <= 1e-12),
            "behind the side face"
        );
    }
}

#[test]
fn a_wall_building_push_raises_a_touch_vertex_another_disc_holds_and_commits() {
    // Two circles touching at a point inside the top: each disc's ring
    // passes through the touch vertex, which the other disc holds without
    // sharing an edge. The raw push/pull of one disc builds walls along its
    // whole ring (every edge is shared with the hole around it), so the
    // touch vertex is RAISED, not translated in place: the other disc keeps
    // the original, nothing bends, and the push commits. (Were the pin
    // check applied to every moved vertex rather than the ones translated in
    // place, the other disc would read as bent and this would refuse.)
    let mut w = top_2x2();
    let r = 0.3;
    let a = w.ring(0.6, 1.0, r, 24, 0.0);
    let mut b = w.ring(1.2, 1.0, r, 24, 0.0);
    b[12] = a[0];
    let (ga, gb) = (w.circle_geom(0.6, 1.0, r), w.circle_geom(1.2, 1.0, r));
    let seed = w.face;
    draw_and_check(&mut w, seed, a, Some(ga));
    let seed = face_at(&w, 1.8, 1.8);
    draw_and_check(&mut w, seed, b, Some(gb));
    let id = w.id;
    for d in [0.2, -0.2] {
        let disc = face_at(&w, 0.6, 1.0);
        let other = face_at(&w, 1.2, 1.0);
        let shares_edge = {
            let obj = w.obj();
            obj.loop_half_edges(obj.faces()[disc].outer_loop).any(|h| {
                obj.half_edges()[h]
                    .twin
                    .is_some_and(|t| obj.loops()[obj.half_edges()[t].loop_id].face == other)
            })
        };
        assert!(!shares_edge, "the discs share only the touch vertex");
        let bytes0 = w.doc.save();
        let v0 = volume(w.obj());
        let area = net_area(w.obj(), disc);
        w.doc
            .apply_object_op(
                id,
                KernelOp::PushPull {
                    face: disc,
                    distance: d,
                },
            )
            .unwrap_or_else(|e| panic!("the raw push/pull {d}: {e:?}"));
        w.obj().validate().expect("valid");
        assert!((volume(w.obj()) - v0 - d * area).abs() < 1e-9);
        assert_exact_history(&mut w.doc, &bytes0);
        w.doc.undo().expect("undo");
        assert_eq!(w.doc.save(), bytes0);
    }
}
