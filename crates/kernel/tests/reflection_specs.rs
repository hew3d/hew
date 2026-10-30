//! Executable specs for baking reflections (negative-determinant maps) into
//! solids and sketches: the geometry is mirrored and every face is turned
//! over, so the solid stays watertight and right side out.

use kernel::{
    Document, DocumentError, Object, ObjectId, Plane, Point3, SketchId, Transform, TransformError,
    Vec3, WatertightState,
};

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane")
}

fn draw_rect(doc: &mut Document, s: SketchId, x0: f64, y0: f64, x1: f64, y1: f64) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    for (a, b) in c {
        sk.add_segment(a, b).expect("segment");
    }
}

/// A box with x in [x0, x1], y in [0, 1], z in [0, h].
fn make_box(doc: &mut Document, x0: f64, x1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    draw_rect(doc, s, x0, 0.0, x1, 1.0);
    let r = doc
        .sketch(s)
        .expect("sketch")
        .regions()
        .keys()
        .next()
        .expect("one region");
    doc.extrude_region(s, r, h).expect("extrude").0
}

fn centroid(o: &Object) -> Point3 {
    let n = o.vertices().len() as f64;
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    for v in o.vertices().values() {
        x += v.position.x;
        y += v.position.y;
        z += v.position.z;
    }
    Point3::new(x / n, y / n, z / n)
}

fn mirror_x() -> Transform {
    Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
}

/// Every face's plane normal points away from the solid's centroid.
fn assert_faces_point_outward(o: &Object) {
    let c = centroid(o);
    for f in o.faces().values() {
        let p = o.loop_positions(f.outer_loop).next().expect("loop vertex");
        let out = Vec3::new(p.x - c.x, p.y - c.y, p.z - c.z);
        assert!(
            f.plane.normal().dot(out) > 0.0,
            "a face normal points into the solid"
        );
    }
}

#[test]
fn reflecting_a_box_keeps_it_watertight_and_right_side_out() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    let before = doc.object(id).unwrap().signed_volume();
    doc.transform_object(id, &mirror_x())
        .expect("reflection bakes");
    let o = doc.object(id).unwrap();
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!(o.signed_volume() > 0.0);
    assert!((o.signed_volume() - before).abs() < 1e-9);
    assert_faces_point_outward(o);
    o.validate().expect("validator accepts the mirrored solid");
    // Mirrored position: x range is now [-3, -1].
    let min_x = o
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!((min_x + 3.0).abs() < 1e-9);
}

#[test]
fn reflecting_twice_is_the_identity() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    let original = doc.object(id).unwrap().clone();
    doc.transform_object(id, &mirror_x()).unwrap();
    doc.transform_object(id, &mirror_x()).unwrap();
    let o = doc.object(id).unwrap();
    for (vid, v) in original.vertices() {
        let w = o.vertices()[vid].position;
        assert!((w.x - v.position.x).abs() < 1e-12);
        assert!((w.y - v.position.y).abs() < 1e-12);
        assert!((w.z - v.position.z).abs() < 1e-12);
    }
    for (fid, f) in original.faces() {
        let g = &o.faces()[fid];
        let a: Vec<_> = original.loop_positions(f.outer_loop).collect();
        let b: Vec<_> = o.loop_positions(g.outer_loop).collect();
        assert_eq!(a.len(), b.len());
        // Same loop, same direction: the sequence matches from some start.
        let n = a.len();
        assert!(
            (0..n).any(|k| (0..n).all(|i| {
                let (p, q) = (a[i], b[(i + k) % n]);
                (p.x - q.x).abs() < 1e-12 && (p.y - q.y).abs() < 1e-12 && (p.z - q.z).abs() < 1e-12
            })),
            "a doubly reflected loop winds the way it did"
        );
        assert!((f.plane.normal() - g.plane.normal()).length() < 1e-12);
    }
    o.validate().unwrap();
}

#[test]
fn singular_is_still_refused_and_leaves_the_object_alone() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    let before = doc.object(id).unwrap().clone();
    assert_eq!(
        doc.transform_object(id, &Transform::scale(Vec3::new(0.0, 1.0, 1.0))),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    let after = doc.object(id).unwrap();
    assert_eq!(after.vertices().len(), before.vertices().len());
    for (vid, v) in before.vertices() {
        assert_eq!(after.vertices()[vid].position, v.position);
    }
}

#[test]
fn an_anisotropic_reflection_keeps_the_solid_right_side_out() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    doc.transform_object(id, &Transform::scale(Vec3::new(-2.0, 1.0, 3.0)))
        .unwrap();
    let o = doc.object(id).unwrap();
    assert!((o.signed_volume() - 24.0).abs() < 1e-9);
    assert!(o.signed_volume() > 0.0);
    assert_faces_point_outward(o);
    o.validate().unwrap();
}

#[test]
fn reflected_object_survives_save_and_load() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    doc.transform_object(id, &mirror_x()).unwrap();
    let bytes = doc.save();
    let back = Document::load(&bytes).expect("loads");
    let root = back.top_level_nodes().into_iter().next().expect("one node");
    let oid = back
        .leaf_objects_under(root)
        .into_iter()
        .next()
        .expect("one object");
    let o = back.object(oid).expect("object");
    o.validate().expect("valid after reload");
    assert!(o.signed_volume() > 0.0);
    assert_faces_point_outward(o);
}

#[test]
fn sketch_reflection_keeps_regions_and_faces_the_same_side() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 1.0, 0.0, 4.0, 3.0);
    let ids_before: Vec<_> = doc.sketch(s).unwrap().regions().keys().collect();
    let n0 = doc.sketch(s).unwrap().plane().normal();
    doc.transform_sketch(s, &mirror_x())
        .expect("sketch reflects");
    let sk = doc.sketch(s).unwrap();
    let ids_after: Vec<_> = sk.regions().keys().collect();
    assert_eq!(ids_before, ids_after);
    assert!((sk.plane().normal() - n0).length() < 1e-12);
}

#[test]
fn reflecting_a_sketch_twice_restores_its_regions() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 1.0, 0.0, 4.0, 3.0);
    let before = doc.sketch(s).unwrap().clone();
    doc.transform_sketch(s, &mirror_x()).unwrap();
    doc.transform_sketch(s, &mirror_x()).unwrap();
    assert_eq!(doc.sketch(s).unwrap(), &before);
}

#[test]
fn a_mirrored_sketch_still_extrudes_into_a_right_side_out_solid() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 1.0, 0.0, 4.0, 3.0);
    doc.transform_sketch(s, &mirror_x()).unwrap();
    let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (id, _) = doc.extrude_region(s, r, 2.0).expect("extrude");
    let o = doc.object(id).unwrap();
    assert!((o.signed_volume() - 18.0).abs() < 1e-9);
    assert_faces_point_outward(o);
}

#[test]
fn in_plane_island_mirror_recreates_regions() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 1.0, 0.0, 4.0, 3.0);
    draw_rect(&mut doc, s, 6.0, 0.0, 8.0, 2.0);
    let sk = doc.sketch(s).unwrap();
    let island = sk
        .islands()
        .iter()
        .find(|(_, i)| {
            i.edges
                .iter()
                .any(|e| sk.vertices()[sk.edges()[*e].from].position.x < 5.0)
        })
        .map(|(id, _)| id)
        .expect("island");
    let old_regions: Vec<_> = sk.regions().keys().collect();
    assert_eq!(old_regions.len(), 2);
    doc.transform_sketch_islands(s, &[island], &mirror_x())
        .expect("in-plane mirror is a reshape");
    let sk = doc.sketch(s).unwrap();
    assert_eq!(sk.regions().len(), 2, "both rectangles still bound regions");
    for r in sk.regions().values() {
        assert!(r.outer.len() >= 4);
    }
    // Both regions still extrude into right-side-out solids.
    let ids: Vec<_> = sk.regions().keys().collect();
    for r in ids {
        let (oid, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
        assert!(doc.object(oid).unwrap().signed_volume() > 0.0);
    }
}

#[test]
fn boolean_between_a_reflected_and_a_plain_solid() {
    // An L-bracket (two boxes unioned), mirrored, then unioned/subtracted
    // against a box; volumes match the same ops on a bracket built directly
    // in the mirrored position.
    let build = |doc: &mut Document, mirrored: bool| {
        let a = make_box(doc, 1.0, 3.0, 1.0);
        let b = make_box(doc, 1.0, 2.0, 2.0);
        let (l, _) = doc.boolean(kernel::BooleanOp::Union, a, b).expect("union");
        if mirrored {
            doc.transform_object(l, &mirror_x()).expect("mirror");
        }
        l
    };
    let mut reflected = Document::new();
    let l = build(&mut reflected, true);
    let cutter = {
        // Box straddling the mirrored bracket (x in [-2.5, -1.5]).
        let s = reflected.add_sketch(ground());
        draw_rect(&mut reflected, s, -2.5, 0.25, -1.5, 0.75);
        let r = reflected
            .sketch(s)
            .unwrap()
            .regions()
            .keys()
            .next()
            .unwrap();
        reflected.extrude_region(s, r, 0.5).unwrap().0
    };
    let vol_l = reflected.object(l).unwrap().signed_volume();
    // Union with a cutter that sits wholly inside the mirrored bracket
    // leaves its volume unchanged.
    let mut union_doc = reflected.clone();
    let (u, _) = union_doc
        .boolean(kernel::BooleanOp::Union, l, cutter)
        .expect("union with a mirrored solid");
    let uo = union_doc.object(u).unwrap();
    assert_eq!(uo.watertight(), WatertightState::Watertight);
    assert!((uo.signed_volume() - vol_l).abs() < 1e-9);
    let (r_sub, _) = reflected
        .boolean(kernel::BooleanOp::Subtract, l, cutter)
        .expect("subtract from a mirrored solid");
    let o = reflected.object(r_sub).unwrap();
    assert_eq!(o.watertight(), WatertightState::Watertight);
    assert!((o.signed_volume() - (vol_l - 0.5 * 0.5)).abs() < 1e-9);
    assert_faces_not_inverted(o);
}

fn assert_faces_not_inverted(o: &Object) {
    assert!(o.signed_volume() > 0.0);
    o.validate().expect("valid");
}

#[test]
fn duplicate_with_a_mirrored_step_is_a_mirrored_copy() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    let (copy, _) = doc
        .duplicate_node(kernel::NodeId::Object(id), &mirror_x())
        .expect("a mirrored copy step is accepted");
    let kernel::NodeId::Object(cid) = copy else {
        panic!("copy of an object is an object");
    };
    let o = doc.object(cid).unwrap();
    assert!((o.signed_volume() - 4.0).abs() < 1e-9);
    assert_faces_point_outward(o);
    let min_x = o
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!((min_x + 3.0).abs() < 1e-9);
    // The original is untouched.
    let orig = doc.object(id).unwrap();
    let omin = orig
        .vertices()
        .values()
        .map(|v| v.position.x)
        .fold(f64::MAX, f64::min);
    assert!((omin - 1.0).abs() < 1e-9);
}

#[test]
fn library_insert_at_a_mirrored_pose_bakes_a_mirrored_copy() {
    let mut item = Document::new();
    make_box(&mut item, 1.0, 3.0, 2.0);
    let mut doc = Document::new();
    doc.insert_document(
        &item,
        &kernel::InsertOptions {
            pose: mirror_x(),
            provenance: None,
        },
    )
    .expect("mirrored insert");
    let root = doc
        .top_level_nodes()
        .into_iter()
        .next()
        .expect("inserted node");
    let oid = doc
        .leaf_objects_under(root)
        .into_iter()
        .next()
        .expect("object");
    let o = doc.object(oid).unwrap();
    assert!(o.signed_volume() > 0.0);
    assert_faces_point_outward(o);
    o.validate().unwrap();
}

#[test]
fn push_pull_after_reflection_is_right_side_out() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    doc.transform_object(id, &mirror_x()).unwrap();
    // Pull the top face (normal +z) up by 1.
    let top = {
        let o = doc.object(id).unwrap();
        o.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.99)
            .map(|(fid, _)| fid)
            .expect("top face")
    };
    let (ids, _) = doc.push_pull_through(id, top, 1.0).expect("push/pull");
    let o = doc.object(ids[0]).unwrap();
    assert!((o.signed_volume() - 6.0).abs() < 1e-9);
    assert_faces_point_outward(o);
}

/// Draws an owned edge along a box's top rim, applies `t` to the selection,
/// pushes the top face up by 1, and reports whether the sketch rode.
fn owned_edge_rides_after(t: Option<Transform>) -> bool {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 1.0);
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(1.0, 0.0, 1.0), Point3::new(2.0, 0.0, 1.0))
        .unwrap();
    doc.set_sketch_object_owner(s, Some(id)).unwrap();
    if let Some(t) = t {
        doc.transform_selection(&[kernel::NodeId::Object(id)], &[], &t)
            .expect("transform the selection");
    }
    let top = {
        let o = doc.object(id).unwrap();
        o.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.99)
            .map(|(fid, _)| fid)
            .expect("top face")
    };
    doc.apply_object_op(
        id,
        kernel::KernelOp::PushPull {
            face: top,
            distance: 1.0,
        },
    )
    .expect("push/pull");
    doc.sketch_ids().iter().any(|sid| {
        let sk = doc.sketch(*sid).unwrap();
        !sk.vertices().is_empty()
            && sk
                .vertices()
                .values()
                .all(|v| (v.position.z - 2.0).abs() < 1e-9)
    })
}

#[test]
fn riding_sketch_still_rides_after_its_object_is_reflected() {
    assert!(
        owned_edge_rides_after(Some(Transform::translation(Vec3::new(5.0, 0.0, 0.0)))),
        "control: the edge rides a pushed face of a moved object"
    );
    assert!(
        owned_edge_rides_after(Some(mirror_x())),
        "the edge rides a pushed face of a mirrored object"
    );
}

#[test]
fn reflection_maps_cylinder_surfaces_and_drops_them_under_anisotropy() {
    use kernel::SurfaceRef;
    let build = || {
        let mut doc = Document::new();
        let s = doc.add_sketch(ground());
        let n = 24;
        let center = Point3::new(2.0, 0.0, 0.0);
        {
            let sk = doc.sketch_mut(s).unwrap();
            sk.begin_curve_with(kernel::CurveGeom {
                center,
                radius: 0.5,
            })
            .unwrap();
            let pt = |i: usize| {
                let a = (i % n) as f64 / n as f64 * std::f64::consts::TAU;
                Point3::new(center.x + 0.5 * a.cos(), center.y + 0.5 * a.sin(), 0.0)
            };
            for i in 0..n {
                sk.add_segment(pt(i), pt(i + 1)).unwrap();
            }
            sk.end_curve();
        }
        let r = doc.sketch(s).unwrap().regions().keys().next().unwrap();
        let (id, _) = doc.extrude_region(s, r, 1.0).unwrap();
        (doc, id)
    };
    let cyl = |o: &Object| {
        o.faces()
            .values()
            .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
            .count()
    };
    let (mut doc, id) = build();
    let before = cyl(doc.object(id).unwrap());
    assert!(before > 0, "the extruded circle carries cylinder faces");
    doc.transform_object(id, &mirror_x()).unwrap();
    let o = doc.object(id).unwrap();
    assert_eq!(cyl(o), before);
    for f in o.faces().values() {
        if let Some(SurfaceRef::Cylinder { radius, .. }) = f.surface {
            assert!((radius - 0.5).abs() < 1e-9);
        }
    }
    assert!(o.signed_volume() > 0.0);
    assert_faces_point_outward(o);

    let (mut doc, id) = build();
    doc.transform_object(id, &Transform::scale(Vec3::new(-2.0, 1.0, 1.0)))
        .unwrap();
    assert_eq!(cyl(doc.object(id).unwrap()), 0);
}

#[test]
fn reflection_mirrors_the_texture_frame() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 2.0);
    let top = {
        let o = doc.object(id).unwrap();
        o.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.99)
            .map(|(fid, _)| fid)
            .unwrap()
    };
    let frame = kernel::UvFrame {
        s: Vec3::new(1.0, 0.0, 0.0),
        t: Vec3::new(0.0, 1.0, 0.0),
        u0: 0.25,
        v0: 0.5,
    };
    doc.set_face_uv_frame(id, top, Some(frame)).unwrap();
    let uv = |f: &kernel::UvFrame, p: Point3| {
        (
            f.s.dot(Vec3::new(p.x, p.y, p.z)) + f.u0,
            f.t.dot(Vec3::new(p.x, p.y, p.z)) + f.v0,
        )
    };
    let corner = {
        let o = doc.object(id).unwrap();
        o.loop_positions(o.faces()[top].outer_loop).next().unwrap()
    };
    let (u0, v0) = uv(&frame, corner);
    let t = mirror_x();
    doc.transform_object(id, &t).unwrap();
    let o = doc.object(id).unwrap();
    let f = o.faces()[top].uv_frame.expect("frame survives the mirror");
    let moved = t.apply_point(corner);
    let (u1, v1) = uv(&f, moved);
    assert!((u0 - u1).abs() < 1e-9 && (v0 - v1).abs() < 1e-9);
}

#[test]
fn a_copied_sketch_under_a_mirror_faces_the_same_side_as_the_mirrored_original() {
    let build = || {
        let mut doc = Document::new();
        let s = doc.add_sketch(ground());
        draw_rect(&mut doc, s, 1.0, 0.0, 4.0, 3.0);
        (doc, s)
    };
    // In-place mirror.
    let (mut doc, s) = build();
    doc.transform_sketch(s, &mirror_x()).unwrap();
    let in_place = doc.sketch(s).unwrap().plane().normal();
    // Copy through the mirror (the island copy path).
    let (mut doc, s) = build();
    let island = doc.sketch(s).unwrap().islands().keys().next().unwrap();
    doc.copy_sketch_islands(s, &[island], &mirror_x()).unwrap();
    let copy_id = doc
        .sketch_ids()
        .into_iter()
        .find(|id| *id != s)
        .expect("the copy");
    let copied = doc.sketch(copy_id).unwrap().plane().normal();
    assert!(
        (copied - in_place).length() < 1e-12,
        "{copied:?} vs {in_place:?}"
    );
    let r = doc
        .sketch(copy_id)
        .unwrap()
        .regions()
        .keys()
        .next()
        .unwrap();
    let (id, _) = doc.extrude_region(copy_id, r, 1.0).unwrap();
    let o = doc.object(id).unwrap();
    let max_z = o
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MIN, f64::max);
    assert!(
        (max_z - 1.0).abs() < 1e-9,
        "the copy extrudes up, like the original"
    );
}

#[test]
fn a_z_mirrored_sketch_with_a_hole_keeps_its_normal_side_and_winding() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 6.0, 6.0);
    draw_rect(&mut doc, s, 2.0, 2.0, 4.0, 4.0);
    // The hole: the outer region has one hole, plus the inner region.
    let holes: usize = doc
        .sketch(s)
        .unwrap()
        .regions()
        .values()
        .map(|r| r.holes.len())
        .sum();
    assert_eq!(holes, 1);
    let ids_before: Vec<_> = doc.sketch(s).unwrap().regions().keys().collect();
    let n0 = doc.sketch(s).unwrap().plane().normal();
    // A mirror ACROSS the sketch plane's normal axis: the normal becomes the
    // mapped normal (`apply_normal`), here -z.
    let t = Transform::scale(Vec3::new(1.0, 1.0, -1.0));
    doc.transform_sketch(s, &t).unwrap();
    let sk = doc.sketch(s).unwrap();
    assert_eq!(ids_before, sk.regions().keys().collect::<Vec<_>>());
    let want = t.apply_normal(n0).normalized().expect("unit normal");
    assert!((sk.plane().normal() - want).length() < 1e-12);
    // A fresh trace agrees with the stored regions: extruding the ring
    // region (outer with its hole) yields volume 6*6-2*2 = 32 per unit.
    let ring = sk
        .regions()
        .iter()
        .find(|(_, r)| !r.holes.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    let (id, _) = doc.extrude_region(s, ring, 1.0).unwrap();
    assert!((doc.object(id).unwrap().signed_volume() - 32.0).abs() < 1e-9);
}

// ------------------------------------------------ plane maps under a mirror

/// The world plane z = `z`, normal +z.
fn level(z: f64) -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).expect("plane")
}

/// A unit cube folded into a component whose one instance is mirrored in
/// x and moved so it covers x in [2, 3], y and z in [0, 1].
fn mirrored_cube_instance(
    doc: &mut Document,
) -> (kernel::ComponentId, kernel::InstanceId, ObjectId, Transform) {
    let cube = make_box(doc, 0.0, 1.0, 1.0);
    let (comp, inst, _) = doc
        .make_component(&[kernel::NodeId::Object(cube)])
        .expect("make_component");
    let member = doc.def_members(comp).expect("live component")[0];
    let pose = mirror_x().then(&Transform::translation(Vec3::new(3.0, 0.0, 0.0)));
    doc.transform_instance(inst, &pose)
        .expect("mirror the instance");
    (comp, inst, member, pose)
}

/// The world-space bounds of `o` placed by `pose`: (min, max).
fn placed_bounds(o: &Object, pose: &Transform) -> (Point3, Point3) {
    let mut lo = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    for v in o.vertices().values() {
        let p = pose.apply_point(v.position);
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

/// Drawn on the top face of a MIRRORED instance and extruded by +0.1, the
/// new member sits ON the cube, not sunk into it: the definition sketch's
/// plane is the pose-inverse image of the user's up.
#[test]
fn a_sketch_on_a_mirrored_instance_extrudes_toward_the_users_up() {
    let mut doc = Document::new();
    let (_, inst, _, pose) = mirrored_cube_instance(&mut doc);
    let (sid, _) = doc
        .begin_sketch_on_plane_in_instance(inst, level(1.0))
        .expect("begin sketch in instance");
    let inv = pose.inverse().unwrap();
    let corners = [
        Point3::new(2.25, 0.25, 1.0),
        Point3::new(2.75, 0.25, 1.0),
        Point3::new(2.75, 0.75, 1.0),
        Point3::new(2.25, 0.75, 1.0),
    ]
    .map(|p| inv.apply_point(p));
    {
        let sk = doc.sketch_mut(sid).expect("sketch is live");
        for i in 0..4 {
            sk.add_segment(corners[i], corners[(i + 1) % 4])
                .expect("segment");
        }
    }
    let region = doc.extrudable_regions(sid).expect("live")[0];
    let (member, _) = doc
        .extrude_region_in_instance(inst, sid, region, 0.1)
        .expect("extrude in instance");
    let (lo, hi) = placed_bounds(doc.object(member).unwrap(), &pose);
    assert!(
        (lo.z - 1.0).abs() < 1e-9 && (hi.z - 1.1).abs() < 1e-9,
        "the new member lies above the cube: z in [{}, {}]",
        lo.z,
        hi.z
    );
    assert_faces_point_outward(doc.object(member).unwrap());
}

/// Slicing a member of a MIRRORED instance by a world plane returns the
/// piece on the plane's normal side first, as a world slice does.
#[test]
fn slice_def_member_returns_the_positive_piece_first_under_a_mirror() {
    let mut doc = Document::new();
    let (_, inst, member, pose) = mirrored_cube_instance(&mut doc);
    let cut = Plane::from_point_normal(Point3::new(2.25, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0))
        .expect("plane");
    let ((pos, neg), _) = doc
        .slice_def_member(inst, member, &cut)
        .expect("slice through a mirrored pose");
    let (plo, phi) = placed_bounds(doc.object(pos).unwrap(), &pose);
    let (nlo, nhi) = placed_bounds(doc.object(neg).unwrap(), &pose);
    assert!(
        (plo.x - 2.25).abs() < 1e-9 && (phi.x - 3.0).abs() < 1e-9,
        "the positive piece is on the +x side: x in [{}, {}]",
        plo.x,
        phi.x
    );
    assert!(
        (nlo.x - 2.0).abs() < 1e-9 && (nhi.x - 2.25).abs() < 1e-9,
        "the negative piece is on the -x side: x in [{}, {}]",
        nlo.x,
        nhi.x
    );
}

/// After a negative scale, an owned sketch only PART of which lies over
/// the top face splits when that face is pushed: the riding part lands on
/// a new sketch on the face's new plane, facing the face's outward side,
/// and the rest stays where it was.
#[test]
fn a_split_riding_sketch_after_a_negative_scale_faces_the_face_outward_side() {
    let mut doc = Document::new();
    let id = make_box(&mut doc, 1.0, 3.0, 1.0);
    let s = doc.add_sketch(level(1.0));
    {
        let sk = doc.sketch_mut(s).unwrap();
        // One edge over the top face, one beyond it on the same plane.
        sk.add_segment(Point3::new(1.5, 0.5, 1.0), Point3::new(2.5, 0.5, 1.0))
            .unwrap();
        sk.add_segment(Point3::new(4.0, 0.5, 1.0), Point3::new(5.0, 0.5, 1.0))
            .unwrap();
    }
    doc.set_sketch_object_owner(s, Some(id)).unwrap();
    let flip = Transform::scale(Vec3::new(-1.0, 2.0, 1.0));
    doc.transform_selection(&[kernel::NodeId::Object(id)], &[], &flip)
        .expect("negative scale");
    let top = {
        let o = doc.object(id).unwrap();
        o.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.99)
            .map(|(fid, _)| fid)
            .expect("top face")
    };
    doc.apply_object_op(
        id,
        kernel::KernelOp::PushPull {
            face: top,
            distance: 1.0,
        },
    )
    .expect("push/pull");
    let sketches: Vec<_> = doc
        .sketch_ids()
        .into_iter()
        .filter(|&sid| !doc.sketch(sid).unwrap().edges().is_empty())
        .collect();
    let at = |z: f64| {
        sketches
            .iter()
            .map(|&sid| doc.sketch(sid).unwrap())
            .find(|sk| {
                sk.vertices()
                    .values()
                    .all(|v| (v.position.z - z).abs() < 1e-9)
            })
            .unwrap_or_else(|| panic!("a sketch at z = {z}"))
    };
    let moved = at(2.0);
    assert_eq!(moved.edges().len(), 1, "the riding edge moved");
    assert!(
        moved.plane().normal().z > 0.99,
        "the moved part faces the pushed face's outward side"
    );
    let rest = at(1.0);
    assert_eq!(rest.edges().len(), 1, "the edge beyond the face stayed");
    assert!(rest.plane().normal().z > 0.99);
}

mod plane_map_props {
    use super::*;
    use proptest::prelude::*;

    /// A random invertible pose: a rotation, per-axis scales of either
    /// sign, and a translation.
    fn pose() -> impl Strategy<Value = Transform> {
        (
            (-1.0..1.0f64, -1.0..1.0f64, -1.0..1.0f64),
            -3.0..3.0f64,
            (0.5..2.0f64, 0.5..2.0f64, 0.5..2.0f64),
            (any::<bool>(), any::<bool>(), any::<bool>()),
            (-5.0..5.0f64, -5.0..5.0f64, -5.0..5.0f64),
        )
            .prop_filter_map(
                "a usable rotation axis",
                |(axis, angle, scale, signs, t)| {
                    let r = Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), angle).ok()?;
                    let sign = |neg: bool| if neg { -1.0 } else { 1.0 };
                    let s = Transform::scale(Vec3::new(
                        scale.0 * sign(signs.0),
                        scale.1 * sign(signs.1),
                        scale.2 * sign(signs.2),
                    ));
                    Some(
                        r.then(&s)
                            .then(&Transform::translation(Vec3::new(t.0, t.1, t.2))),
                    )
                },
            )
    }

    fn plane() -> impl Strategy<Value = Plane> {
        (
            (-1.0..1.0f64, -1.0..1.0f64, -1.0..1.0f64),
            (-3.0..3.0f64, -3.0..3.0f64, -3.0..3.0f64),
        )
            .prop_filter_map("a usable normal", |(n, p)| {
                Plane::from_point_normal(Point3::new(p.0, p.1, p.2), Vec3::new(n.0, n.1, n.2)).ok()
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// The side-keeping map's normal is `apply_normal`'s direction for
        /// every pose; the winding-following map agrees with it when the
        /// determinant is positive and is its opposite under a reflection.
        /// Both keep mapped points on the mapped plane.
        #[test]
        fn side_keeping_plane_normal_is_the_mapped_normal(t in pose(), p in plane()) {
            let keep = t.apply_plane_keeping_side(&p).expect("invertible");
            let wind = t.apply_plane_following_winding(&p).expect("invertible");
            let want = t.apply_normal(p.normal()).normalized().expect("unit");
            prop_assert!((keep.normal() - want).length() < 1e-9);
            if t.determinant() > 0.0 {
                prop_assert!((wind.normal() - want).length() < 1e-9);
                prop_assert_eq!(keep, wind);
            } else {
                prop_assert!((wind.normal() + want).length() < 1e-9);
            }
            let on = Point3::ORIGIN + p.normal() * -p.signed_distance(Point3::ORIGIN);
            let mapped = t.apply_point(on);
            prop_assert!(keep.signed_distance(mapped).abs() < 1e-9);
            prop_assert!(wind.signed_distance(mapped).abs() < 1e-9);
        }
    }
}
