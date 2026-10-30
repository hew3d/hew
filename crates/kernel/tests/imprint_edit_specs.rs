//! Executable specs for editable imprints (docs/design/editable-face-sketches.md):
//! a shape drawn on a solid's face — a sub-face imprint or a boundary chord —
//! is recovered structurally from the face topology, slides/turns/scales on
//! its face in place ([`Object::transform_sub_face`], and as a drafting move),
//! and dissolves back into its face (a drafting erase), every edit undoable as
//! one step.

use kernel::topo::LoopKind;
use kernel::{
    CurveGeom, Document, DocumentError, FaceId, History, KernelOp, KernelOpReport, Object, Plane,
    Point3, StickyError, Transform, Vec3, WatertightState, tol,
};
use proptest::prelude::*;

mod face_arrangement_support;
use face_arrangement_support::{
    delete_unit_at, draft_on_face, draw_loop, face_plane, group_faces, move_unit_at,
};

// ---------------------------------------------------------------- helpers

fn box_object(min: Point3, max: Point3) -> Object {
    let (a, b) = (min, max);
    Object::from_polygons(
        &[
            Point3::new(a.x, a.y, a.z),
            Point3::new(b.x, a.y, a.z),
            Point3::new(b.x, b.y, a.z),
            Point3::new(a.x, b.y, a.z),
            Point3::new(a.x, a.y, b.z),
            Point3::new(b.x, a.y, b.z),
            Point3::new(b.x, b.y, b.z),
            Point3::new(a.x, b.y, b.z),
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
    .unwrap()
}

fn unit_cube() -> Object {
    box_object(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0))
}

fn face_with_normal(obj: &Object, dir: Vec3) -> FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION))
        .map(|(id, _)| id)
        .expect("object has a face with the requested normal")
}

fn top(obj: &Object) -> FaceId {
    face_with_normal(obj, Vec3::new(0.0, 0.0, 1.0))
}

/// An axis-aligned square on z = 1, CCW seen from +z.
fn square(x0: f64, y0: f64, side: f64) -> Vec<Point3> {
    vec![
        Point3::new(x0, y0, 1.0),
        Point3::new(x0 + side, y0, 1.0),
        Point3::new(x0 + side, y0 + side, 1.0),
        Point3::new(x0, y0 + side, 1.0),
    ]
}

fn circle(cx: f64, cy: f64, r: f64, n: usize) -> Vec<Point3> {
    (0..n)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            Point3::new(cx + r * a.cos(), cy + r * a.sin(), 1.0)
        })
        .collect()
}

fn polygons_of(obj: &Object) -> Vec<Vec<Point3>> {
    let (points, faces) = obj.to_polygons();
    faces
        .into_iter()
        .map(|poly| poly.into_iter().map(|i| points[i]).collect())
        .collect()
}

fn cyclic_match(a: &[Point3], b: &[Point3]) -> bool {
    a.len() == b.len()
        && (0..a.len()).any(|shift| {
            a.iter()
                .enumerate()
                .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], tol::POINT_MERGE))
        })
}

fn objects_equivalent(x: &Object, y: &Object) -> bool {
    let xs = polygons_of(x);
    let mut ys = polygons_of(y);
    if xs.len() != ys.len() {
        return false;
    }
    for poly in xs {
        match ys.iter().position(|cand| cyclic_match(&poly, cand)) {
            Some(i) => {
                ys.swap_remove(i);
            }
            None => return false,
        }
    }
    true
}

/// `(face, parent, loop, curve, nested)` of one imprint sub-face.
type SubFaceFeat = (FaceId, FaceId, Vec<Point3>, Option<CurveGeom>, Vec<FaceId>);

/// The parent of `face` when it is an imprint: its outer loop is wholly
/// twinned with ONE hole loop of one coplanar, same-facing face.
fn imprint_parent(obj: &Object, face: FaceId) -> Option<FaceId> {
    let f = &obj.faces()[face];
    let mut twin_loop = None;
    for h in obj.loop_half_edges(f.outer_loop) {
        let l = obj.half_edges()[obj.half_edges()[h].twin?].loop_id;
        match twin_loop {
            None => twin_loop = Some(l),
            Some(x) if x == l => {}
            _ => return None,
        }
    }
    let l = twin_loop?;
    let lp = &obj.loops()[l];
    if lp.kind != LoopKind::Inner || lp.face == face {
        return None;
    }
    if obj.loop_half_edges(l).count() != obj.loop_half_edges(f.outer_loop).count() {
        return None;
    }
    let parent = &obj.faces()[lp.face];
    let coplanar = (parent.plane.normal() - f.plane.normal()).length() < tol::NORMAL_DIRECTION
        && obj
            .loop_positions(f.outer_loop)
            .all(|p| parent.plane.signed_distance(p).abs() <= tol::PLANE_DIST);
    coplanar.then_some(lp.face)
}

/// The circle every edge of the loop claims, when they all claim one.
fn uniform_curve(obj: &Object, l: kernel::LoopId) -> Option<CurveGeom> {
    let mut it = obj
        .loop_half_edges(l)
        .map(|h| obj.edges()[obj.half_edges()[h].edge].curve);
    let first = it.next()??;
    it.all(|c| {
        c.is_some_and(|c| {
            c.center.approx_eq(first.center, tol::POINT_MERGE)
                && (c.radius - first.radius).abs() < tol::POINT_MERGE
        })
    })
    .then_some(first)
}

fn sub_faces(obj: &Object) -> Vec<SubFaceFeat> {
    obj.faces()
        .iter()
        .filter_map(|(fid, f)| {
            let parent = imprint_parent(obj, fid)?;
            let nested = f
                .inner_loops
                .iter()
                .filter_map(|&il| {
                    let h = obj.loop_half_edges(il).next()?;
                    let g = obj.loops()[obj.half_edges()[obj.half_edges()[h].twin?].loop_id].face;
                    (imprint_parent(obj, g) == Some(fid)).then_some(g)
                })
                .collect();
            Some((
                fid,
                parent,
                obj.loop_positions(f.outer_loop).collect(),
                uniform_curve(obj, f.outer_loop),
                nested,
            ))
        })
        .collect()
}

fn imprint(obj: &mut Object, face: FaceId, path: &[Point3]) -> FaceId {
    obj.split_face_inner(face, path).expect("imprint").sub_face
}

fn slide(dx: f64, dy: f64) -> Transform {
    Transform::translation(Vec3::new(dx, dy, 0.0))
}

/// Rotation about the +z axis through `(cx, cy, 1)`.
fn spin(cx: f64, cy: f64, angle: f64) -> Transform {
    let c = Vec3::new(cx, cy, 1.0);
    Transform::translation(c * -1.0)
        .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), angle).unwrap())
        .then(&Transform::translation(c))
}

/// Uniform scale about `(cx, cy, 1)`.
fn grow(cx: f64, cy: f64, s: f64) -> Transform {
    let c = Vec3::new(cx, cy, 1.0);
    Transform::translation(c * -1.0)
        .then(&Transform::uniform_scale(s))
        .then(&Transform::translation(c))
}

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

/// A unit cube built through the document (so every op records undo).
fn doc_cube() -> (Document, kernel::ObjectId) {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    let pts = square(0.0, 0.0, 1.0)
        .into_iter()
        .map(|p| Point3::new(p.x, p.y, 0.0))
        .collect::<Vec<_>>();
    {
        let sk = doc.sketch_mut(s).unwrap();
        for i in 0..4 {
            sk.add_segment(pts[i], pts[(i + 1) % 4]).unwrap();
        }
    }
    let regions = doc.extrudable_regions(s).unwrap();
    let (id, _) = doc.extrude_region(s, regions[0], 1.0).unwrap();
    (doc, id)
}

/// The plane z = 1 (the cube's top).
fn top_plane() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap()
}

/// Draws `ring` on the top plane through one draft and returns the face
/// carrying the drawn region.
fn draw(
    doc: &mut Document,
    id: kernel::ObjectId,
    ring: Vec<Point3>,
    curve: Option<CurveGeom>,
) -> FaceId {
    let t = top(doc.object(id).unwrap());
    draw_loop(doc, None, id, t, &ring, curve).expect("the loop draws");
    face_arrangement_support::region_face_in(doc, None, id, top_plane(), &ring)
        .expect("a face carries the drawn region")
}

/// The imprint whose loop is `ring`, as `(face, parent, ..)`.
fn imprint_with_ring(obj: &Object, ring: &[Point3]) -> SubFaceFeat {
    sub_faces(obj)
        .into_iter()
        .find(|x| cyclic_match(&x.2, ring))
        .expect("an imprint with that ring")
}

/// The face of the cube's top plane holding the point `(x, y, 1)`.
fn top_at(obj: &Object, x: f64, y: f64) -> FaceId {
    face_with_normal_at(obj, Vec3::new(0.0, 0.0, 1.0), Point3::new(x, y, 1.0))
}

// ------------------------------------------------------- feature query

#[test]
fn a_square_drawn_on_the_top_is_one_sub_face_feature() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let path = square(0.3, 0.3, 0.2);
    let sf = imprint(&mut cube, t, &path);
    let feats = sub_faces(&cube);
    assert_eq!(feats.len(), 1);
    let (face, parent, loop_path, curve, nested) = &feats[0];
    assert_eq!(*face, sf);
    assert_eq!(*parent, t);
    assert!(cyclic_match(loop_path, &path));
    assert!(curve.is_none());
    assert!(nested.is_empty());
    assert_eq!(
        sub_faces(&cube).len(),
        1,
        "a pristine cube carries no other imprints"
    );
}

#[test]
fn a_circle_feature_carries_its_analytic_claim() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let path = circle(0.5, 0.5, 0.2, 24);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.2,
    };
    cube.split_face_inner_with_curve(t, &path, Some(g)).unwrap();
    let feats = sub_faces(&cube);
    assert_eq!(feats.len(), 1);
    let curve = feats[0].3.expect("a full drawn circle reports its circle");
    assert!(curve.center.approx_eq(g.center, tol::POINT_MERGE));
    assert!((curve.radius - g.radius).abs() < tol::POINT_MERGE);
}

#[test]
fn a_boss_is_not_a_feature() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let sf = imprint(&mut cube, t, &square(0.3, 0.3, 0.2));
    cube.extrude_sub_face(sf, 0.1).unwrap();
    assert!(sub_faces(&cube).is_empty());
}

#[test]
fn a_line_across_a_face_is_one_topology_drawing_unit() {
    // Replaces the chord-feature query: a cut from edge to edge is ONE
    // drawing unit of the plane, every edge of it face topology.
    let (mut doc, id) = doc_cube();
    let t = top(doc.object(id).unwrap());
    let path = [Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)];
    draft_on_face(&mut doc, None, id, t, "Line", |w| {
        w.add_segment(path[0], path[1]).map(|_| ())
    })
    .unwrap();
    let plane = face_plane(&doc, id, t);
    let units = face_arrangement_support::units_on(&doc, None, id, plane);
    assert_eq!(units.units.len(), 1);
    assert!(
        units.units[0]
            .edges
            .iter()
            .all(|e| e.store == kernel::DraftStore::Topology(id))
    );
    assert_eq!(units.regions.len(), 2);
    assert!(sub_faces(doc.object(id).unwrap()).is_empty());
}

// ------------------------------------------------------- transform_sub_face

#[test]
fn slide_moves_the_loop_in_place_and_keeps_every_handle() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let sf = imprint(&mut cube, t, &square(0.3, 0.3, 0.2));
    let edges_before: Vec<_> = cube
        .loop_half_edges(cube.faces()[sf].outer_loop)
        .map(|h| cube.half_edges()[h].edge)
        .collect();
    let rep = cube.transform_sub_face(sf, &slide(0.2, -0.1)).unwrap();
    assert_eq!(rep.sub_face, sf);
    assert_eq!(rep.parent, t);
    assert!(
        cube.faces().contains_key(sf),
        "the sub-face handle survives"
    );
    let edges_after: Vec<_> = cube
        .loop_half_edges(cube.faces()[sf].outer_loop)
        .map(|h| cube.half_edges()[h].edge)
        .collect();
    assert_eq!(edges_before, edges_after, "edge handles survive too");
    let feats = sub_faces(&cube);
    assert!(cyclic_match(&feats[0].2, &square(0.5, 0.2, 0.2)));
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    cube.validate().unwrap();
}

#[test]
fn slide_then_inverse_is_the_identity() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let sf = imprint(&mut cube, t, &square(0.3, 0.3, 0.2));
    let before = cube.clone();
    let rep = cube.transform_sub_face(sf, &spin(0.4, 0.4, 0.7)).unwrap();
    assert!(!objects_equivalent(&before, &cube));
    cube.transform_sub_face(sf, &rep.inverse).unwrap();
    assert!(objects_equivalent(&before, &cube));
}

#[test]
fn scale_maps_the_circle_claim() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let path = circle(0.5, 0.5, 0.1, 24);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.1,
    };
    let sf = cube
        .split_face_inner_with_curve(t, &path, Some(g))
        .unwrap()
        .sub_face;
    cube.transform_sub_face(sf, &grow(0.5, 0.5, 2.0).then(&slide(0.1, 0.0)))
        .unwrap();
    let curve = sub_faces(&cube)[0].3.expect("still one circle");
    assert!(
        curve
            .center
            .approx_eq(Point3::new(0.6, 0.5, 1.0), tol::POINT_MERGE)
    );
    assert!((curve.radius - 0.2).abs() < tol::POINT_MERGE);
    cube.validate().unwrap();
    // The claim still describes the loop: a push-through stamps a cylinder.
    for h in cube.loop_half_edges(cube.faces()[sf].outer_loop) {
        let e = &cube.edges()[cube.half_edges()[h].edge];
        let c = e.curve.expect("every loop edge keeps its claim");
        let p = cube.vertices()[cube.half_edges()[h].origin].position;
        assert!(((p - c.center).length() - c.radius).abs() < tol::PLANE_DIST);
    }
}

#[test]
fn off_plane_mirrored_and_non_uniform_transforms_refuse_typed() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let sf = imprint(&mut cube, t, &square(0.3, 0.3, 0.2));
    let before = cube.clone();
    let lift = Transform::translation(Vec3::new(0.0, 0.0, 0.1));
    let tilt = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 0.3).unwrap();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0));
    let stretch = Transform::scale(Vec3::new(1.5, 1.0, 1.0));
    let flat = Transform::scale(Vec3::new(1.0, 1.0, 0.0));
    for xf in [lift, tilt, mirror, stretch, flat] {
        assert_eq!(
            cube.transform_sub_face(sf, &xf),
            Err(StickyError::NotInPlane)
        );
    }
    assert!(
        objects_equivalent(&before, &cube),
        "refusals leave the object untouched"
    );
}

#[test]
fn sliding_off_the_face_or_onto_a_neighbour_refuses_typed() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let a = imprint(&mut cube, t, &square(0.1, 0.1, 0.2));
    let _b = imprint(&mut cube, t, &square(0.6, 0.6, 0.2));
    let before = cube.clone();
    assert!(matches!(
        cube.transform_sub_face(a, &slide(0.8, 0.0)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    // Flush against the boundary is not strictly inside either.
    assert!(matches!(
        cube.transform_sub_face(a, &slide(-0.1, 0.0)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    // Overlapping (or merely touching) the other imprint.
    assert!(matches!(
        cube.transform_sub_face(a, &slide(0.4, 0.4)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    assert!(matches!(
        cube.transform_sub_face(a, &slide(0.5, 0.5)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    assert!(objects_equivalent(&before, &cube));
}

#[test]
fn not_an_imprint_refuses_typed() {
    let mut cube = unit_cube();
    let t = top(&cube);
    assert_eq!(
        cube.transform_sub_face(t, &slide(0.1, 0.0)),
        Err(StickyError::NotAnInnerFace)
    );
    let sf = imprint(&mut cube, t, &square(0.3, 0.3, 0.2));
    cube.extrude_sub_face(sf, 0.1).unwrap();
    assert_eq!(
        cube.transform_sub_face(sf, &slide(0.1, 0.0)),
        Err(StickyError::NotAnInnerFace),
        "a boss top is solid geometry, not an imprint"
    );
}

#[test]
fn a_nested_imprint_moves_with_its_outer_and_is_its_own_feature() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let outer = imprint(&mut cube, t, &square(0.2, 0.2, 0.5));
    let inner = imprint(&mut cube, outer, &square(0.3, 0.3, 0.1));
    let feats = sub_faces(&cube);
    assert_eq!(feats.len(), 2);
    let outer_feat = feats.iter().find(|f| f.0 == outer).unwrap();
    assert_eq!(outer_feat.1, t);
    assert_eq!(outer_feat.4, vec![inner]);
    let inner_feat = feats.iter().find(|f| f.0 == inner).unwrap();
    assert_eq!(inner_feat.1, outer);
    assert!(inner_feat.4.is_empty());

    cube.transform_sub_face(outer, &slide(0.2, 0.1)).unwrap();
    let feats = sub_faces(&cube);
    let inner_feat = feats.iter().find(|f| f.0 == inner).unwrap();
    assert!(cyclic_match(&inner_feat.2, &square(0.5, 0.4, 0.1)));
    cube.validate().unwrap();

    // The inner one moves alone too, held inside its own parent.
    cube.transform_sub_face(inner, &slide(0.1, 0.1)).unwrap();
    assert!(matches!(
        cube.transform_sub_face(inner, &slide(0.5, 0.0)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
}

#[test]
fn a_boss_inside_an_imprint_pins_it() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let outer = imprint(&mut cube, t, &square(0.2, 0.2, 0.5));
    let inner = imprint(&mut cube, outer, &square(0.3, 0.3, 0.1));
    cube.extrude_sub_face(inner, 0.1).unwrap();
    let before = cube.clone();
    assert_eq!(
        cube.transform_sub_face(outer, &slide(0.1, 0.0)),
        Err(StickyError::NestedNotFlat)
    );
    assert!(objects_equivalent(&before, &cube));
    let feats = sub_faces(&cube);
    let outer_feat = feats.iter().find(|f| f.0 == outer).unwrap();
    assert!(
        outer_feat.4.is_empty(),
        "a raised hole is not a nested imprint"
    );
}

#[test]
fn a_loop_spanning_a_concave_notch_refuses() {
    // An L-shaped top: a loop whose vertices are all inside but whose edge
    // crosses the notch is refused (the shared placement gate).
    let mut slab = box_object(Point3::ORIGIN, Point3::new(2.0, 2.0, 1.0));
    let t = top(&slab);
    // Cut the top's north-east quarter off with a chord, then recess it
    // through: the remaining top is an L.
    let chord = [
        Point3::new(1.0, 2.0, 1.0),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(2.0, 1.0, 1.0),
    ];
    let rep = slab.split_face(t, &chord).unwrap();
    let quarter = rep
        .new_faces
        .into_iter()
        .find(|&f| slab.face_contains_point(f, Point3::new(1.5, 1.5, 1.0)))
        .unwrap();
    slab.push_pull(quarter, -0.5).unwrap();
    let l_top = face_with_normal_at(&slab, Vec3::new(0.0, 0.0, 1.0), Point3::new(0.5, 0.5, 1.0));
    let spanning = vec![
        Point3::new(0.5, 1.5, 1.0),
        Point3::new(1.5, 0.5, 1.0),
        Point3::new(1.8, 0.8, 1.0),
        Point3::new(0.8, 1.8, 1.0),
    ];
    assert!(matches!(
        slab.split_face_inner(l_top, &spanning),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    let sf = imprint(&mut slab, l_top, &square(0.2, 0.2, 0.3));
    assert!(matches!(
        slab.transform_sub_face(sf, &slide(0.9, 0.9)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
}

fn face_with_normal_at(obj: &Object, dir: Vec3, p: Point3) -> FaceId {
    obj.faces()
        .iter()
        .find(|(id, f)| {
            f.plane.normal().approx_eq(dir, tol::NORMAL_DIRECTION)
                && obj.face_contains_point(*id, p)
        })
        .map(|(id, _)| id)
        .expect("face at point")
}

// ------------------------------------------------------- history

#[test]
fn move_imprint_undoes_and_redoes_through_history() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let mut history = History::new();
    let rep = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.3, 0.3, 0.2),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(r) = rep else {
        unreachable!()
    };
    let imprinted = cube.clone();
    let xf = spin(0.4, 0.4, 0.5).then(&slide(0.1, 0.05));
    history
        .apply(
            &mut cube,
            KernelOp::TransformSubFace {
                sub_face: r.sub_face,
                xf,
                pinned: vec![],
            },
        )
        .unwrap();
    let moved = cube.clone();
    assert!(!objects_equivalent(&imprinted, &moved));

    history.undo(&mut cube).unwrap();
    assert!(
        objects_equivalent(&imprinted, &cube),
        "undo puts the shape back"
    );
    history.redo(&mut cube).unwrap();
    assert!(objects_equivalent(&moved, &cube), "redo moves it again");
    // Undo past the imprint itself, then redo both.
    history.undo(&mut cube).unwrap();
    history.undo(&mut cube).unwrap();
    assert!(objects_equivalent(&unit_cube(), &cube));
    history.redo(&mut cube).unwrap();
    history.redo(&mut cube).unwrap();
    assert!(objects_equivalent(&moved, &cube));
}

#[test]
fn move_then_boss_then_undo_all_the_way() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let mut history = History::new();
    let KernelOpReport::FaceSplitInner(r) = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.3, 0.3, 0.2),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    history
        .apply(
            &mut cube,
            KernelOp::TransformSubFace {
                sub_face: r.sub_face,
                xf: slide(0.2, 0.2),
                pinned: vec![],
            },
        )
        .unwrap();
    let moved = cube.clone();
    history
        .apply(
            &mut cube,
            KernelOp::ExtrudeSubFace {
                sub_face: r.sub_face,
                distance: 0.2,
            },
        )
        .unwrap();
    assert!(sub_faces(&cube).is_empty());
    history.undo(&mut cube).unwrap();
    assert!(objects_equivalent(&moved, &cube));
    assert_eq!(
        sub_faces(&cube).len(),
        1,
        "the boss undone is an imprint again"
    );
    history.undo(&mut cube).unwrap();
    history.undo(&mut cube).unwrap();
    assert!(objects_equivalent(&unit_cube(), &cube));
}

// ------------------------------------------------------- document level

#[test]
fn deleting_an_outer_shape_keeps_the_shapes_inside_it() {
    let (mut doc, id) = doc_cube();
    let outer_ring = square(0.2, 0.2, 0.5);
    let inner_ring = square(0.3, 0.3, 0.1);
    draw(&mut doc, id, outer_ring.clone(), None);
    draw(&mut doc, id, inner_ring.clone(), None);
    let with_both = doc.object(id).unwrap().clone();
    let depth = doc.undo_depth();
    delete_unit_at(&mut doc, None, id, top_plane(), Point3::new(0.2, 0.45, 1.0)).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    let obj = doc.object(id).unwrap();
    let feats = sub_faces(obj);
    assert_eq!(feats.len(), 1, "only the outer shape is gone");
    assert!(
        cyclic_match(&feats[0].2, &inner_ring),
        "the inner shape survives"
    );
    assert_eq!(
        feats[0].1,
        top_at(obj, 0.05, 0.05),
        "and now sits on the top face itself"
    );
    obj.validate().unwrap();
    doc.undo().unwrap();
    assert!(objects_equivalent(&with_both, doc.object(id).unwrap()));
    let obj = doc.object(id).unwrap();
    assert_eq!(sub_faces(obj).len(), 2);
    let inner_feat = imprint_with_ring(obj, &inner_ring);
    let outer_feat = imprint_with_ring(obj, &outer_ring);
    assert_eq!(
        inner_feat.1, outer_feat.0,
        "undo nests the inner shape again"
    );
}

#[test]
fn deleting_a_drawn_circle_and_undoing_gives_the_circle_back() {
    let (mut doc, id) = doc_cube();
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.2,
    };
    let ring = circle(0.5, 0.5, 0.2, 24);
    draw(&mut doc, id, ring.clone(), Some(g));
    assert!(sub_faces(doc.object(id).unwrap())[0].3.is_some());
    delete_unit_at(&mut doc, None, id, top_plane(), ring[0]).unwrap();
    assert!(sub_faces(doc.object(id).unwrap()).is_empty());
    doc.undo().unwrap();
    let obj = doc.object(id).unwrap();
    let feats = sub_faces(obj);
    assert_eq!(feats.len(), 1);
    let curve = feats[0].3.expect("the restored imprint is still a circle");
    assert!(curve.center.approx_eq(g.center, tol::POINT_MERGE));
    assert!((curve.radius - g.radius).abs() < tol::POINT_MERGE);
    for h in obj.loop_half_edges(obj.faces()[feats[0].0].outer_loop) {
        assert!(obj.edges()[obj.half_edges()[h].edge].curve.is_some());
    }
    // Redo dissolves it again; undo again still brings the circle back.
    doc.redo().unwrap();
    doc.undo().unwrap();
    assert!(sub_faces(doc.object(id).unwrap())[0].3.is_some());
}

#[test]
fn erasing_an_outer_shape_hands_a_boss_inside_it_back_to_the_face() {
    let (mut doc, id) = doc_cube();
    // A pristine top has no drawing unit to erase.
    assert!(
        face_arrangement_support::units_on(&doc, None, id, top_plane())
            .units
            .is_empty()
    );
    draw(&mut doc, id, square(0.2, 0.2, 0.5), None);
    let inner = draw(&mut doc, id, square(0.3, 0.3, 0.1), None);
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: inner,
            distance: 0.1,
        },
    )
    .unwrap();
    let before = doc.object(id).unwrap().clone();
    delete_unit_at(&mut doc, None, id, top_plane(), Point3::new(0.2, 0.45, 1.0)).unwrap();
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert!(
        sub_faces(obj).is_empty(),
        "the boss is solid, the shape is gone"
    );
    assert_eq!(
        obj.faces()[top_at(obj, 0.05, 0.05)].inner_loops.len(),
        1,
        "the boss footprint is a hole of the top face again"
    );
    doc.undo().unwrap();
    assert!(objects_equivalent(&before, doc.object(id).unwrap()));
    assert_eq!(sub_faces(doc.object(id).unwrap()).len(), 1);
}

// ------------------------------------------------------- circle rims on a face

#[test]
fn a_circle_drawn_on_a_face_reports_its_rim_until_it_becomes_a_boss() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.2,
    };
    let sf = cube
        .split_face_inner_with_curve(t, &circle(0.5, 0.5, 0.2, 24), Some(g))
        .unwrap()
        .sub_face;
    let rims = cube.edge_curve_rims();
    assert_eq!(rims.len(), 1, "one drawn circle, one rim");
    let r = &rims[0];
    assert!(r.center.approx_eq(g.center, tol::POINT_MERGE));
    assert!((r.radius - 0.2).abs() < tol::POINT_MERGE);
    assert!(
        r.axis.cross(Vec3::new(0.0, 0.0, 1.0)).length() < tol::NORMAL_DIRECTION,
        "the rim lies in the face's plane"
    );
    assert!(
        r.coverage.is_none(),
        "a whole drawn circle covers the full turn"
    );
    let quads = r.quadrant_points();
    assert_eq!(quads.len(), 4);
    for q in quads {
        assert!(((q - g.center).length() - 0.2).abs() < 1e-9);
        assert!((q.z - 1.0).abs() < 1e-9);
    }
    // A polygon or rectangle carries no claim and offers no rim.
    imprint(&mut cube, t, &square(0.05, 0.05, 0.1));
    assert_eq!(cube.edge_curve_rims().len(), 1);
    // Moving the circle carries its rim along.
    cube.transform_sub_face(sf, &slide(0.1, 0.0)).unwrap();
    assert!(
        cube.edge_curve_rims()[0]
            .center
            .approx_eq(Point3::new(0.6, 0.5, 1.0), tol::POINT_MERGE)
    );
    // Bossed, the wall rims report the circle and the edge rim is not doubled.
    cube.extrude_sub_face(sf, 0.1).unwrap();
    assert!(!cube.analytic_rims().is_empty());
    assert!(
        cube.edge_curve_rims().is_empty(),
        "no duplicate center or quadrants"
    );
}

// ------------------------------------------------------- enclosing shapes

#[test]
fn a_shape_drawn_around_another_adopts_it() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let inner = imprint(&mut cube, t, &square(0.4, 0.4, 0.2));
    let rep = cube.split_face_inner(t, &square(0.2, 0.2, 0.6)).unwrap();
    assert_eq!(rep.adopted_holes.len(), 1);
    cube.validate().unwrap();
    assert_eq!(cube.watertight(), WatertightState::Watertight);
    let feats = sub_faces(&cube);
    let outer_feat = feats.iter().find(|f| f.0 == rep.sub_face).unwrap();
    assert_eq!(outer_feat.1, t);
    assert_eq!(
        outer_feat.4,
        vec![inner],
        "the inner shape is nested in the new one"
    );
    let inner_feat = feats.iter().find(|f| f.0 == inner).unwrap();
    assert_eq!(inner_feat.1, rep.sub_face);
    // Undoing the draw (a dissolve) hands it straight back.
    cube.merge_inner_face(rep.sub_face).unwrap();
    let feats = sub_faces(&cube);
    assert_eq!(feats.len(), 1);
    assert_eq!(feats[0].1, t);
}

#[test]
fn a_shape_drawn_around_a_boss_or_a_hole_adopts_it() {
    // A boss.
    let mut cube = unit_cube();
    let t = top(&cube);
    let b = imprint(&mut cube, t, &square(0.4, 0.4, 0.2));
    cube.extrude_sub_face(b, 0.2).unwrap();
    let ring = cube.split_face_inner(t, &square(0.2, 0.2, 0.6)).unwrap();
    assert_eq!(ring.adopted_holes.len(), 1);
    cube.validate().unwrap();
    assert!(!cube.is_flat_sub_face(ring.sub_face));
    let feats = sub_faces(&cube);
    assert_eq!(feats.len(), 1, "the boss top is not a shape");
    assert!(feats[0].4.is_empty(), "a boss is not a nested shape");
    assert_eq!(
        cube.faces()[feats[0].0].inner_loops.len(),
        1,
        "but its footprint is a hole of the ring"
    );
    assert_eq!(
        cube.transform_sub_face(ring.sub_face, &slide(0.05, 0.0)),
        Err(StickyError::NestedNotFlat),
        "a shape holding a boss cannot slide"
    );

    // A push-through hole.
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s).unwrap();
        let pts = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(pts[i], pts[(i + 1) % 4]).unwrap();
        }
    }
    let regions = doc.extrudable_regions(s).unwrap();
    let (id, _) = doc.extrude_region(s, regions[0], 1.0).unwrap();
    let hole = draw(&mut doc, id, square(0.4, 0.4, 0.2), None);
    let (objs, _) = doc.push_pull_through(id, hole, -1.5).unwrap();
    let id = objs[0];
    let ring = draw(&mut doc, id, square(0.2, 0.2, 0.6), None);
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert_eq!(
        obj.faces()[ring].inner_loops.len(),
        1,
        "the ring holds the hole"
    );
}

#[test]
fn drawing_across_or_inside_another_shape_still_refuses() {
    let mut cube = unit_cube();
    let t = top(&cube);
    imprint(&mut cube, t, &square(0.4, 0.4, 0.2));
    let before = cube.clone();
    // Crossing it.
    assert!(matches!(
        cube.split_face_inner(t, &square(0.3, 0.3, 0.2)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    // Touching it along an edge.
    assert!(matches!(
        cube.split_face_inner(t, &square(0.2, 0.4, 0.2)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    // Sharing its ring exactly.
    assert!(matches!(
        cube.split_face_inner(t, &square(0.4, 0.4, 0.2)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    assert!(objects_equivalent(&before, &cube));
}

#[test]
fn moving_a_shape_around_another_adopts_it_and_undo_gives_it_back() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let mut history = History::new();
    let KernelOpReport::FaceSplitInner(big) = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.05, 0.05, 0.5),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let KernelOpReport::FaceSplitInner(small) = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.7, 0.7, 0.1),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let before = cube.clone();
    let KernelOpReport::TransformSubFace(rep) = history
        .apply(
            &mut cube,
            KernelOp::TransformSubFace {
                sub_face: big.sub_face,
                xf: slide(0.3, 0.3),
                pinned: vec![],
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(rep.adopted.len(), 1);
    cube.validate().unwrap();
    let parent_of = |c: &Object, f: FaceId| sub_faces(c).into_iter().find(|x| x.0 == f).unwrap().1;
    assert_eq!(parent_of(&cube, small.sub_face), big.sub_face, "adopted");
    let small_ring = |c: &Object| {
        sub_faces(c)
            .into_iter()
            .find(|x| x.0 == small.sub_face)
            .unwrap()
            .2
    };
    assert!(
        cyclic_match(&small_ring(&cube), &square(0.7, 0.7, 0.1)),
        "it did not move"
    );
    let after = cube.clone();

    history.undo(&mut cube).unwrap();
    assert!(objects_equivalent(&before, &cube));
    let small_face = sub_faces(&cube)
        .into_iter()
        .find(|x| cyclic_match(&x.2, &square(0.7, 0.7, 0.1)))
        .unwrap();
    assert_eq!(
        small_face.1, t,
        "undo hands the small shape back to the face"
    );

    history.redo(&mut cube).unwrap();
    assert!(objects_equivalent(&after, &cube));
    let small_face = sub_faces(&cube)
        .into_iter()
        .find(|x| cyclic_match(&x.2, &square(0.7, 0.7, 0.1)))
        .unwrap();
    assert_ne!(small_face.1, t, "redo adopts it again");

    // Moving the big shape again now carries the adopted one along.
    let big_face = sub_faces(&cube)
        .into_iter()
        .find(|x| cyclic_match(&x.2, &square(0.35, 0.35, 0.5)))
        .unwrap()
        .0;
    cube.transform_sub_face(big_face, &slide(-0.05, -0.05))
        .unwrap();
    assert!(
        sub_faces(&cube)
            .iter()
            .any(|x| cyclic_match(&x.2, &square(0.65, 0.65, 0.1)))
    );
}

#[test]
fn moving_a_shape_around_a_boss_adopts_it_and_undo_releases_it() {
    let mut cube = unit_cube();
    let t = top(&cube);
    let mut history = History::new();
    let KernelOpReport::FaceSplitInner(boss) = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.7, 0.7, 0.1),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    history
        .apply(
            &mut cube,
            KernelOp::ExtrudeSubFace {
                sub_face: boss.sub_face,
                distance: 0.1,
            },
        )
        .unwrap();
    let KernelOpReport::FaceSplitInner(big) = history
        .apply(
            &mut cube,
            KernelOp::SplitFaceInner {
                face: t,
                loop_path: square(0.05, 0.05, 0.5),
                restore: None,
                curves: Vec::new(),
                edge_attrs: Vec::new(),
            },
        )
        .unwrap()
    else {
        unreachable!()
    };
    let before = cube.clone();
    history
        .apply(
            &mut cube,
            KernelOp::TransformSubFace {
                sub_face: big.sub_face,
                xf: slide(0.3, 0.3),
                pinned: vec![],
            },
        )
        .unwrap();
    cube.validate().unwrap();
    assert_eq!(
        cube.faces()[big.sub_face].inner_loops.len(),
        1,
        "the ring holds the boss"
    );
    assert_eq!(
        cube.transform_sub_face(big.sub_face, &slide(0.01, 0.0)),
        Err(StickyError::NestedNotFlat),
        "and can no longer slide on its own"
    );
    history.undo(&mut cube).unwrap();
    assert!(
        objects_equivalent(&before, &cube),
        "undo slides it back past the boss"
    );
    assert_eq!(
        cube.faces()[t].inner_loops.len(),
        2,
        "boss and shape both on the top again"
    );
    history.redo(&mut cube).unwrap();
    cube.validate().unwrap();
}

#[test]
fn moving_a_shape_inside_or_across_another_still_refuses() {
    let mut cube = unit_cube();
    let t = top(&cube);
    imprint(&mut cube, t, &square(0.5, 0.5, 0.4));
    let small = imprint(&mut cube, t, &square(0.1, 0.1, 0.1));
    let before = cube.clone();
    // Into the big shape's interior.
    assert!(matches!(
        cube.transform_sub_face(small, &slide(0.55, 0.55)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    // Across its edge.
    assert!(matches!(
        cube.transform_sub_face(small, &slide(0.35, 0.35)),
        Err(StickyError::LoopNotStrictlyInside { .. })
    ));
    assert!(objects_equivalent(&before, &cube));
}

#[test]
fn offsetting_a_face_that_carries_shapes_adopts_them() {
    let (mut doc, id) = doc_cube();
    let t = top(doc.object(id).unwrap());
    draw(&mut doc, id, square(0.2, 0.2, 0.2), None);
    let b = draw(&mut doc, id, square(0.6, 0.6, 0.2), None);
    doc.apply_object_op(
        id,
        KernelOp::ExtrudeSubFace {
            sub_face: b,
            distance: 0.1,
        },
    )
    .unwrap();
    let t = if doc.object(id).unwrap().faces().contains_key(t) {
        t
    } else {
        top_at(doc.object(id).unwrap(), 0.05, 0.05)
    };
    let loop_ = kernel::offset_face_boundary(doc.object(id).unwrap(), t, -0.05).unwrap();
    let ring = draw(&mut doc, id, loop_.points, None);
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    assert_eq!(
        obj.faces()[ring].inner_loops.len(),
        2,
        "the inset holds both"
    );
    doc.undo().unwrap();
    let obj = doc.object(id).unwrap();
    assert_eq!(obj.faces()[top_at(obj, 0.05, 0.05)].inner_loops.len(), 2);
}

#[test]
fn moving_an_imprint_records_one_undoable_step() {
    let (mut doc, id) = doc_cube();
    draw(&mut doc, id, square(0.3, 0.3, 0.2), None);
    let before = doc.object(id).unwrap().clone();
    let depth = doc.undo_depth();
    move_unit_at(
        &mut doc,
        None,
        id,
        top_plane(),
        Point3::new(0.3, 0.4, 1.0),
        &slide(0.2, 0.0),
    )
    .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1);
    let feats = sub_faces(doc.object(id).unwrap());
    assert!(cyclic_match(&feats[0].2, &square(0.5, 0.3, 0.2)));
    doc.undo().unwrap();
    assert!(objects_equivalent(&before, doc.object(id).unwrap()));
}

/// A rectangle drawn from the south edge: three chord sides.
fn south_rect() -> Vec<Point3> {
    vec![
        Point3::new(0.2, 0.0, 1.0),
        Point3::new(0.5, 0.0, 1.0),
        Point3::new(0.5, 0.3, 1.0),
        Point3::new(0.2, 0.3, 1.0),
    ]
}

/// The top-plane faces of `id` as rings.
fn top_rings(doc: &Document, id: kernel::ObjectId) -> Vec<face_arrangement_support::Rings> {
    let obj = doc.object(id).unwrap();
    group_faces(obj, &top_plane())
        .into_iter()
        .map(|f| face_arrangement_support::face_rings(obj, f))
        .collect()
}

fn has_ring(doc: &Document, id: kernel::ObjectId, want: &[Point3]) -> bool {
    let n = Vec3::new(0.0, 0.0, 1.0);
    let want = face_arrangement_support::without_straight(want, n);
    top_rings(doc, id).iter().any(|(o, _)| {
        face_arrangement_support::cyclic_match(
            &face_arrangement_support::without_straight(o, n),
            &want,
            true,
        )
    })
}

#[test]
fn a_chord_slides_along_its_edge_as_one_undo_entry() {
    let (mut doc, id) = doc_cube();
    draw(&mut doc, id, south_rect(), None);
    assert!(has_ring(&doc, id, &south_rect()));
    assert_eq!(top_rings(&doc, id).len(), 2);
    let before = doc.object(id).unwrap().clone();
    let depth = doc.undo_depth();
    // The unit is the three off-boundary sides; slide them 0.3 east.
    move_unit_at(
        &mut doc,
        None,
        id,
        top_plane(),
        Point3::new(0.5, 0.15, 1.0),
        &slide(0.3, 0.0),
    )
    .unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one entry");
    let moved: Vec<Point3> = south_rect()
        .iter()
        .map(|p| *p + Vec3::new(0.3, 0.0, 0.0))
        .collect();
    assert!(
        has_ring(&doc, id, &moved),
        "the rectangle slid 0.3 along the south edge"
    );
    assert_eq!(top_rings(&doc, id).len(), 2, "still one chord");
    assert!(doc.object_owned_sketches(id).is_empty());
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    doc.undo().unwrap();
    assert!(objects_equivalent(&before, doc.object(id).unwrap()));
    doc.redo().unwrap();
    assert!(has_ring(&doc, id, &moved));
}

#[test]
fn a_chord_moved_off_the_boundary_stays_as_drawing_instead_of_refusing() {
    let (mut doc, id) = doc_cube();
    draw(&mut doc, id, south_rect(), None);
    let faces_before = top_rings(&doc, id);
    assert_eq!(faces_before.len(), 2);
    let pivot = Point3::new(0.5, 0.15, 1.0);
    let sketch_edges = |doc: &Document, owned: bool| -> usize {
        doc.sketch_ids()
            .iter()
            .filter(|&&s| doc.sketch_owner_object(s).is_some() == owned)
            .map(|&s| doc.sketch(s).unwrap().edges().len())
            .sum()
    };

    // Into the interior: no endpoint touches the boundary any more, so the
    // face is whole again and the three sides are the object's owned drawing.
    let bytes0 = doc.save();
    let depth = doc.undo_depth();
    move_unit_at(&mut doc, None, id, top_plane(), pivot, &slide(0.0, 0.2)).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    let rings = top_rings(&doc, id);
    assert_eq!(rings.len(), 1, "the top is one face again");
    assert!(has_ring(&doc, id, &square(0.0, 0.0, 1.0)));
    assert_eq!(sketch_edges(&doc, true), 3, "the sides are owned drawing");
    assert_eq!(sketch_edges(&doc, false), 0);
    doc.object(id).unwrap().validate().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.save(), bytes0, "undo is byte-exact");

    // Past the east edge: the part on the face stays a chord (x 0.9..1.0),
    // the overhang beyond the edge is free drawing.
    move_unit_at(&mut doc, None, id, top_plane(), pivot, &slide(0.7, 0.0)).unwrap();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(top_rings(&doc, id).len(), 2);
    let strip = [
        Point3::new(0.9, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(1.0, 0.3, 1.0),
        Point3::new(0.9, 0.3, 1.0),
    ];
    assert!(
        has_ring(&doc, id, &strip),
        "the on-face part is a chord strip"
    );
    assert_eq!(sketch_edges(&doc, false), 2, "the overhang is free drawing");
    assert_eq!(sketch_edges(&doc, true), 0);
    doc.object(id).unwrap().validate().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.save(), bytes0, "undo is byte-exact");
}

#[test]
fn a_solid_s_own_edge_cannot_be_moved_or_erased_by_drafting() {
    let (mut doc, id) = doc_cube();
    draw(&mut doc, id, south_rect(), None);
    let t = top(doc.object(id).unwrap());
    let before = doc.object(id).unwrap().clone();
    let bytes0 = doc.save();
    let depth = doc.undo_depth();
    // The east edge of the cube's top is the solid's own.
    let err = draft_on_face(&mut doc, None, id, t, "Erase", |w| {
        let e = w
            .edge_at_positions(Point3::new(1.0, 0.0, 1.0), Point3::new(1.0, 1.0, 1.0))
            .expect("east edge in the working sketch");
        w.remove_edge(e).map(|_| ())
    })
    .expect_err("a solid's edge stays");
    assert!(
        matches!(err, DocumentError::DraftBoundaryEdgeTouched),
        "got {err:?}"
    );
    let err = draft_on_face(&mut doc, None, id, t, "Move", |w| {
        let e = w
            .edge_at_positions(Point3::new(1.0, 0.0, 1.0), Point3::new(1.0, 1.0, 1.0))
            .expect("east edge in the working sketch");
        w.apply_transform_edges(&[e], &slide(-0.1, 0.0))
    })
    .expect_err("a solid's edge stays");
    assert!(
        matches!(err, DocumentError::DraftBoundaryEdgeTouched),
        "got {err:?}"
    );
    assert!(objects_equivalent(&before, doc.object(id).unwrap()));
    assert_eq!(doc.save(), bytes0, "saved bytes untouched");
    assert_eq!(doc.undo_depth(), depth, "a refused edit records nothing");
}

// ------------------------------------------------------- playtest II

#[test]
fn a_circle_started_inside_a_shape_can_enclose_it() {
    let (mut doc, id) = doc_cube();
    let square_ring = square(0.45, 0.45, 0.1);
    draw(&mut doc, id, square_ring.clone(), None);
    // A circle's first click is its center, which lands inside the small
    // square; the loop only fits the top, and drafting is by plane, so the
    // circle lands around the square on the top face.
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.3,
    };
    let circle_ring = circle(0.5, 0.5, 0.3, 48);
    draw(&mut doc, id, circle_ring.clone(), Some(g));
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    let circle_feat = imprint_with_ring(obj, &circle_ring);
    assert_eq!(
        circle_feat.1,
        top_at(obj, 0.05, 0.05),
        "the circle sits on the top face"
    );
    assert!(circle_feat.3.is_some(), "and is still a circle");
    let square_feat = imprint_with_ring(obj, &square_ring);
    assert_eq!(
        square_feat.1, circle_feat.0,
        "the square is now inside the circle"
    );
    // A shape that fits inside the square nests inside it.
    let tiny_ring = square(0.47, 0.47, 0.03);
    draw(&mut doc, id, tiny_ring.clone(), None);
    let obj = doc.object(id).unwrap();
    let square_feat = imprint_with_ring(obj, &square_ring);
    assert_eq!(imprint_with_ring(obj, &tiny_ring).1, square_feat.0);
}

#[test]
fn pushing_a_shape_around_a_circle_raises_a_smooth_cylinder_that_stays_a_circle() {
    let (mut doc, id) = doc_cube();
    let g = CurveGeom {
        center: Point3::new(0.5, 0.5, 1.0),
        radius: 0.2,
    };
    draw(&mut doc, id, circle(0.5, 0.5, 0.2, 48), Some(g));
    let r = draw(&mut doc, id, square(0.15, 0.15, 0.7), None);
    let c = imprint_with_ring(doc.object(id).unwrap(), &circle(0.5, 0.5, 0.2, 48)).0;
    struct Reg {
        region: FaceId,
    }
    let (c, r) = (Reg { region: c }, Reg { region: r });
    let drawn = doc.object(id).unwrap().clone();
    let is_circle = |doc: &Document| {
        sub_faces(doc.object(id).unwrap())
            .iter()
            .any(|f| f.3.is_some() && cyclic_match(&f.2, &circle(0.5, 0.5, 0.2, 48)))
    };
    let claimed = |doc: &Document| {
        doc.object(id)
            .unwrap()
            .edges()
            .values()
            .filter(|e| e.curve.is_some())
            .count()
    };

    // Sink the ring: the circle stands as a cylinder whose 48 walls are all
    // stamped facets of it.
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: r.region,
            distance: -0.1,
        },
    )
    .unwrap();
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    let stamped = obj.faces().values().filter(|f| f.surface.is_some()).count();
    assert_eq!(
        stamped, 48,
        "every wall around the circle is a cylinder facet"
    );
    assert!(
        obj.analytic_rims().iter().any(|rim| rim.has_coverage()),
        "the cylinder offers center and quadrant snaps"
    );

    // Raise the circle's top: the cylinder stays smooth, and the moved rim
    // carries its circle claim up with it rather than dropping it.
    let before = claimed(&doc);
    assert!(before >= 48);
    doc.apply_object_op(
        id,
        KernelOp::PushPull {
            face: c.region,
            distance: 0.2,
        },
    )
    .unwrap();
    let obj = doc.object(id).unwrap();
    obj.validate().unwrap();
    assert_eq!(
        obj.faces().values().filter(|f| f.surface.is_some()).count(),
        48,
        "the raised cylinder is still smooth"
    );
    assert!(obj.analytic_rims().iter().any(|rim| rim.has_coverage()));
    assert_eq!(claimed(&doc), before, "the pushed rim keeps every claim");
    let claims_centered = |obj: &Object, z: f64| {
        obj.edges()
            .values()
            .filter_map(|e| e.curve)
            .filter(|g| {
                g.center
                    .approx_eq(Point3::new(0.5, 0.5, z), tol::POINT_MERGE)
            })
            .count()
    };
    assert_eq!(
        claims_centered(obj, 1.2),
        48,
        "the raised rim is the drawn circle moved up with it"
    );
    assert_eq!(
        claims_centered(obj, 0.9),
        48,
        "the sunk ring's lowered rim carried the circle down with it"
    );
    let pushed = claimed(&doc);

    // Undo both, redo both, undo both again: a circle every time. Undo
    // restores exactly the claims the drawing had, and redo exactly the ones
    // the pushes carried.
    let drawn_claims = drawn.edges().values().filter(|e| e.curve.is_some()).count();
    for _ in 0..3 {
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(objects_equivalent(&drawn, doc.object(id).unwrap()));
        assert!(is_circle(&doc), "undo gives the circle back");
        assert_eq!(claimed(&doc), drawn_claims, "every drawn claim is restored");
        doc.redo().unwrap();
        doc.redo().unwrap();
        doc.object(id).unwrap().validate().unwrap();
        assert_eq!(claimed(&doc), pushed, "redo carries the claims again");
    }
}

// ------------------------------------------------------- properties

fn area_on_plane(pts: &[Point3]) -> f64 {
    let mut a = 0.0;
    for i in 0..pts.len() {
        let p = pts[i];
        let q = pts[(i + 1) % pts.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a.abs() * 0.5
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any in-plane similarity that keeps the loop inside round-trips
    /// exactly through its inverse, keeps the solid watertight, and
    /// scales the imprint's area by s².
    #[test]
    fn in_plane_similarity_round_trips(
        dx in -0.25..0.25f64,
        dy in -0.25..0.25f64,
        angle in -3.0..3.0f64,
        scale in 0.5..1.6f64,
    ) {
        let mut cube = unit_cube();
        let t = top(&cube);
        let sf = imprint(&mut cube, t, &square(0.4, 0.4, 0.2));
        let before = cube.clone();
        let xf = spin(0.5, 0.5, angle)
            .then(&grow(0.5, 0.5, scale))
            .then(&slide(dx, dy));
        match cube.transform_sub_face(sf, &xf) {
            Ok(rep) => {
                prop_assert_eq!(cube.watertight(), WatertightState::Watertight);
                prop_assert!(cube.validate().is_ok());
                let moved = sub_faces(&cube)[0].2.clone();
                prop_assert!((area_on_plane(&moved) - 0.04 * scale * scale).abs() < 1e-9);
                cube.transform_sub_face(sf, &rep.inverse).unwrap();
                prop_assert!(objects_equivalent(&before, &cube));
            }
            Err(StickyError::LoopNotStrictlyInside { .. }) => {
                prop_assert!(objects_equivalent(&before, &cube));
            }
            Err(e) => prop_assert!(false, "unexpected refusal {e:?}"),
        }
    }
}
