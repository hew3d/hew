// Test-only: the sets below are order-independent equality assertions on node /
// object id collections, not kernel output, so HashSet is fine here. Suppress
// the workspace clippy.toml ban for this integration-test crate.
#![allow(clippy::disallowed_types)]

//! Executable specs for [`kernel::Document`] — the document model backbone.
//!
//! These pin the behaviour the wasm-api shim depends on: many first-class
//! sketches and objects coexist; extrusion consumes exactly its region; the
//! document undo log is an identity on visible state and keeps `ObjectId`s
//! stable across undo/redo.

use kernel::{
    BooleanError, BooleanOp, Document, DocumentError, FaceId, GroupId, Guide, ImageFormat,
    KernelOp, KernelOpReport, Material, MaterialId, NodeId, Object, ObjectId, Plane, Point3, Rgba8,
    SketchError, SketchId, SketchVertexId, Texture, Transform, TransformError, Vec3,
    WatertightState,
};
use proptest::prelude::*;
use std::collections::HashSet;

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// Draw an axis-aligned rectangle into `doc`'s sketch `s`.
fn draw_rect(doc: &mut Document, s: kernel::SketchId, x0: f64, y0: f64, x1: f64, y1: f64) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
}

/// The single region of a sketch (panics if there isn't exactly one).
fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// Extrude an axis-aligned box whose base rectangle sits on the plane `z =
/// z_base`, swept up by `height`. Lets tests place two boxes in *general
/// position* (no coplanar faces) for booleans.
fn extrude_box(
    doc: &mut Document,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    z_base: f64,
    height: f64,
) -> kernel::ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z_base),
        Point3::new(1.0, 0.0, z_base),
        Point3::new(0.0, 1.0, z_base),
    ])
    .expect("offset plane is well-defined");
    let s = doc.add_sketch(plane);
    let corners = [
        (Point3::new(x0, y0, z_base), Point3::new(x1, y0, z_base)),
        (Point3::new(x1, y0, z_base), Point3::new(x1, y1, z_base)),
        (Point3::new(x1, y1, z_base), Point3::new(x0, y1, z_base)),
        (Point3::new(x0, y1, z_base), Point3::new(x0, y0, z_base)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
    let r = only_region(doc, s);
    doc.extrude_region(s, r, height).expect("extrude box").0
}

/// Multiset-equal polygon soup, up to vertex re-indexing and cyclic rotation —
/// the same notion of "geometrically identical" the History tests use.
fn objects_equivalent(x: &Object, y: &Object) -> bool {
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
                    .all(|(i, p)| p.approx_eq(b[(i + shift) % b.len()], 1e-9))
            })
    }
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

// ------------------------------------------- the multi-sketch capability

/// Two independent sketches on the *same* plane each extrude into their own
/// Object — the capability the single ephemeral global sketch could not express.
#[test]
fn two_independent_coplanar_sketches_extrude_into_separate_objects() {
    let mut doc = Document::new();
    let s1 = doc.add_sketch(ground());
    let s2 = doc.add_sketch(ground());
    assert_eq!(doc.sketch_ids().len(), 2, "both sketches persist");

    draw_rect(&mut doc, s1, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s2, 5.0, 5.0, 6.0, 6.0);

    let r1 = only_region(&doc, s1);
    let r2 = only_region(&doc, s2);

    let (o1, _) = doc.extrude_region(s1, r1, 1.0).expect("extrude s1");
    let (o2, _) = doc.extrude_region(s2, r2, 2.0).expect("extrude s2");

    assert_ne!(o1, o2, "distinct objects");
    let visible = doc.visible_object_ids();
    assert_eq!(visible.len(), 2, "two independent solids coexist");
    assert_eq!(
        doc.object(o1).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert_eq!(
        doc.object(o2).unwrap().watertight(),
        WatertightState::Watertight
    );
}

// ----------------------------------------------- region consumption semantics

/// Extruding a region consumes exactly that region; a sibling region of the
/// same sketch stays extrudable.
#[test]
fn extrude_consumes_exactly_its_region() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    // Two side-by-side rectangles sharing no edge → two regions in one sketch.
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 2.0, 0.0, 3.0, 1.0);

    let regions = doc.extrudable_regions(s).expect("live");
    assert_eq!(regions.len(), 2, "two independent regions in one sketch");
    let (first, second) = (regions[0], regions[1]);

    doc.extrude_region(s, first, 1.0).expect("extrude first");

    assert!(doc.is_region_consumed(s, first));
    assert!(!doc.is_region_consumed(s, second));
    let remaining = doc.extrudable_regions(s).expect("live");
    assert_eq!(
        remaining,
        vec![second],
        "only the sibling remains extrudable"
    );
}

// ------------------------------------------------------------- undo / redo

/// Undoing a creation hides the Object and restores the region's extrudability;
/// redo reverses both. The `ObjectId` is stable across the cycle.
#[test]
fn undo_creation_hides_object_and_restores_region() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);

    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    assert_eq!(doc.visible_object_ids(), vec![id]);
    assert!(doc.is_region_consumed(s, r));

    doc.undo().expect("undo creation");
    assert!(doc.visible_object_ids().is_empty(), "creation hidden");
    assert!(!doc.is_region_consumed(s, r), "region extrudable again");

    doc.redo().expect("redo creation");
    assert_eq!(
        doc.visible_object_ids(),
        vec![id],
        "same ObjectId after redo"
    );
    assert!(doc.is_region_consumed(s, r));
}

/// A full document session (two creations + a per-Object op) round-trips
/// through undo/redo: visible ids are stable and geometry is identical.
#[test]
fn undo_redo_is_identity_on_visible_state() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 2.0, 0.0, 3.0, 1.0);
    let regions = doc.extrudable_regions(s).expect("live");
    let (ra, rb) = (regions[0], regions[1]);

    let (a, _) = doc.extrude_region(s, ra, 1.0).expect("extrude a");
    let (b, _) = doc.extrude_region(s, rb, 1.0).expect("extrude b");

    // A per-object op on `a`: push its top face up.
    let top = top_face(doc.object(a).unwrap());
    doc.apply_object_op(
        a,
        KernelOp::PushPull {
            face: top,
            distance: 0.5,
        },
    )
    .expect("push/pull a");

    let final_a = doc.object(a).unwrap().clone();
    let final_b = doc.object(b).unwrap().clone();
    let final_ids = doc.visible_object_ids();

    // Undo everything (op, creation b, creation a), then redo everything.
    doc.undo().expect("undo op");
    doc.undo().expect("undo b");
    doc.undo().expect("undo a");
    assert!(doc.visible_object_ids().is_empty());

    doc.redo().expect("redo a");
    doc.redo().expect("redo b");
    doc.redo().expect("redo op");

    assert_eq!(doc.visible_object_ids(), final_ids, "ObjectIds stable");
    assert!(objects_equivalent(doc.object(a).unwrap(), &final_a));
    assert!(objects_equivalent(doc.object(b).unwrap(), &final_b));
}

// ------------------------------------------------------------------ booleans

/// A union consumes both operands into one visible watertight result; undo
/// restores exactly the two operands (stable ids), redo re-combines.
#[test]
fn boolean_union_consumes_operands_and_round_trips() {
    let mut doc = Document::new();
    // Two boxes overlapping in general position (offset in x/y and z so no
    // faces are coplanar — a ground-coplanar pair would be DegenerateContact).
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 0.5, 0.5, 1.5, 1.5, 0.5, 1.0);
    assert_eq!(doc.visible_object_ids().len(), 2);

    let (result, _) = doc.boolean(BooleanOp::Union, a, b).expect("union");
    assert_eq!(
        doc.visible_object_ids(),
        vec![result],
        "only the result is visible"
    );
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(
        doc.object(a).is_none() && doc.object(b).is_none(),
        "operands hidden"
    );

    doc.undo().expect("undo combine");
    let after = doc.visible_object_ids();
    assert_eq!(after.len(), 2, "operands restored");
    assert!(after.contains(&a) && after.contains(&b), "same ObjectIds");
    assert!(doc.object(result).is_none(), "result hidden");

    doc.redo().expect("redo combine");
    assert_eq!(doc.visible_object_ids(), vec![result]);
}

#[test]
fn boolean_with_self_is_refused() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    assert_eq!(
        doc.boolean(BooleanOp::Union, a, a),
        Err(DocumentError::Boolean(BooleanError::DegenerateContact))
    );
}

#[test]
fn boolean_with_hidden_operand_errors() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 0.5, 0.5, 1.5, 1.5, 0.5, 1.0);
    doc.undo().expect("undo b's creation → b hidden");
    assert_eq!(
        doc.boolean(BooleanOp::Union, a, b).map(|_| ()),
        Err(DocumentError::UnknownObject)
    );
}

// ------------------------------------------------------- move / rotate / scale

/// Average of an object's unique vertex positions.
fn centroid(obj: &Object) -> Point3 {
    let (pts, _) = obj.to_polygons();
    let mut acc = Vec3::ZERO;
    for p in &pts {
        acc = acc + p.to_vec();
    }
    Point3::ORIGIN + acc * (1.0 / pts.len() as f64)
}

/// Signed volume via a tetrahedron fan from the origin (hole-free objects).
fn signed_volume(obj: &Object) -> f64 {
    let (pts, faces) = obj.to_polygons();
    let mut six_v = 0.0;
    for face in faces {
        for i in 1..face.len() - 1 {
            let (a, b, c) = (pts[face[0]], pts[face[i]], pts[face[i + 1]]);
            six_v += a.to_vec().dot(b.to_vec().cross(c.to_vec()));
        }
    }
    six_v / 6.0
}

fn approx_pt(a: Point3, b: Point3) -> bool {
    a.approx_eq(b, 1e-9)
}

/// A move translates every point by the offset and round-trips through
/// undo/redo, keeping the object's handle.
#[test]
fn translate_moves_centroid_and_round_trips() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let original = doc.object(id).unwrap().clone();
    let c0 = centroid(&original);
    let offset = Vec3::new(5.0, 3.0, 2.0);

    doc.transform_object(id, &Transform::translation(offset))
        .expect("translate");
    assert!(
        approx_pt(centroid(doc.object(id).unwrap()), c0 + offset),
        "centroid moved by the offset"
    );

    doc.undo().expect("undo move");
    assert_eq!(doc.visible_object_ids(), vec![id], "same ObjectId");
    assert!(
        objects_equivalent(doc.object(id).unwrap(), &original),
        "undo restores the original geometry"
    );

    doc.redo().expect("redo move");
    assert!(approx_pt(centroid(doc.object(id).unwrap()), c0 + offset));
}

/// Rotation preserves volume and watertightness.
#[test]
fn rotate_preserves_volume() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 2.0, 1.0, 0.0, 1.0);
    let v0 = signed_volume(doc.object(id).unwrap());

    let rot = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_4).unwrap();
    doc.transform_object(id, &rot).expect("rotate");

    assert!((signed_volume(doc.object(id).unwrap()) - v0).abs() < 1e-6);
    assert_eq!(
        doc.object(id).unwrap().watertight(),
        WatertightState::Watertight
    );
}

/// Uniform scale multiplies volume by factor³.
#[test]
fn uniform_scale_scales_volume() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let v0 = signed_volume(doc.object(id).unwrap());

    doc.transform_object(id, &Transform::uniform_scale(2.0))
        .expect("scale");
    assert!((signed_volume(doc.object(id).unwrap()) - v0 * 8.0).abs() < 1e-6);
}

/// Orientation-flipping (negative scale) and singular transforms are refused
/// without mutating the object.
#[test]
fn reflection_and_singular_transforms_are_refused() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let c0 = centroid(doc.object(id).unwrap());

    assert_eq!(
        doc.transform_object(id, &Transform::uniform_scale(-1.0)),
        Err(DocumentError::Transform(TransformError::Reflection))
    );
    assert_eq!(
        doc.transform_object(id, &Transform::uniform_scale(0.0)),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert!(
        approx_pt(centroid(doc.object(id).unwrap()), c0),
        "object untouched after refused transforms"
    );
}

#[test]
fn transform_unknown_object_errors() {
    let mut doc = Document::new();
    let bogus = kernel::ObjectId::default();
    assert_eq!(
        doc.transform_object(bogus, &Transform::translation(Vec3::new(1.0, 0.0, 0.0))),
        Err(DocumentError::UnknownObject)
    );
}

// ------------------------------------------------- transform a whole sketch

/// Centroid of a sketch's vertex positions (insertion-order independent).
fn sketch_centroid(doc: &Document, s: SketchId) -> Point3 {
    let verts = doc.sketch(s).expect("sketch is live").vertices();
    let mut acc = Vec3::ZERO;
    let mut n = 0usize;
    for (_, v) in verts {
        acc = acc + v.position.to_vec();
        n += 1;
    }
    Point3::ORIGIN + acc * (1.0 / n as f64)
}

/// Transforming a free-standing sketch moves every vertex by the affine and
/// round-trips exactly through undo/redo, keeping the `SketchId` and the
/// sketch's drawn topology (region count) intact.
#[test]
fn transform_sketch_translates_and_round_trips() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 2.0, 1.0);
    let c0 = sketch_centroid(&doc, s);
    let regions0 = doc.extrudable_regions(s).unwrap().len();
    let offset = Vec3::new(5.0, 3.0, 0.0);

    let change = doc
        .transform_sketch(s, &Transform::translation(offset))
        .expect("translate sketch");
    assert_eq!(
        change.sketches_touched,
        vec![s],
        "reports the touched sketch"
    );
    assert!(
        approx_pt(sketch_centroid(&doc, s), c0 + offset),
        "every vertex moved by the offset"
    );
    assert_eq!(
        doc.extrudable_regions(s).unwrap().len(),
        regions0,
        "topology preserved — still extrudable"
    );

    doc.undo().expect("undo sketch transform");
    assert!(
        approx_pt(sketch_centroid(&doc, s), c0),
        "undo restores the original vertex positions"
    );
    assert!(
        doc.sketch_ids().contains(&s),
        "the SketchId stays valid and visible across undo"
    );

    doc.redo().expect("redo sketch transform");
    assert!(approx_pt(sketch_centroid(&doc, s), c0 + offset));
}

/// A sketch translated off the z=0 plane keeps its vertices and plane in sync,
/// so it remains a valid, extrudable sketch on its new plane.
#[test]
fn transform_sketch_remaps_plane_to_stay_coplanar() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);

    // Lift straight up: vertices move to z=4, and the plane must follow.
    doc.transform_sketch(s, &Transform::translation(Vec3::new(0.0, 0.0, 4.0)))
        .expect("lift sketch");

    let sk = doc.sketch(s).expect("sketch is live");
    let plane = sk.plane();
    for (_, v) in sk.vertices() {
        assert!(
            plane.signed_distance(v.position).abs() < 1e-9,
            "vertices stay on the remapped plane"
        );
    }
    // Still a single closed region the user can push/pull.
    assert_eq!(doc.extrudable_regions(s).unwrap().len(), 1);
}

/// Uniform scale about the origin scales the sketch's extent by the factor.
#[test]
fn transform_sketch_scales_extent() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 1.0, 1.0, 3.0, 2.0);
    let c0 = sketch_centroid(&doc, s);

    doc.transform_sketch(s, &Transform::uniform_scale(2.0))
        .expect("scale sketch");

    // Each vertex's offset from the (scaled) centroid doubled; equivalently the
    // centroid itself scaled about the origin by 2.
    assert!(approx_pt(
        sketch_centroid(&doc, s),
        Point3::ORIGIN + c0.to_vec() * 2.0
    ));
}

/// Orientation-flipping (negative scale) and singular transforms are refused
/// without mutating the sketch — the same transactional guarantee as objects.
#[test]
fn transform_sketch_reflection_and_singular_refused() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let c0 = sketch_centroid(&doc, s);

    assert_eq!(
        doc.transform_sketch(s, &Transform::uniform_scale(-1.0)),
        Err(DocumentError::Transform(TransformError::Reflection))
    );
    assert_eq!(
        doc.transform_sketch(s, &Transform::uniform_scale(0.0)),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert!(
        approx_pt(sketch_centroid(&doc, s), c0),
        "sketch untouched after refused transforms"
    );
}

#[test]
fn transform_unknown_sketch_errors() {
    let mut doc = Document::new();
    let bogus = SketchId::default();
    assert_eq!(
        doc.transform_sketch(bogus, &Transform::translation(Vec3::new(1.0, 0.0, 0.0))),
        Err(DocumentError::UnknownSketch)
    );
}

/// A hidden (deleted) sketch is not a transform target.
#[test]
fn transform_hidden_sketch_errors() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.delete_sketch(s).expect("delete sketch");
    assert_eq!(
        doc.transform_sketch(s, &Transform::translation(Vec3::new(1.0, 0.0, 0.0))),
        Err(DocumentError::UnknownSketch)
    );
}

// ----------------------------------------- move_sketch_vertex (Phase D slice 3)

/// The id of the vertex of sketch `s` sitting at `p` (panics if none).
fn sketch_vertex_at(doc: &Document, s: SketchId, p: Point3) -> SketchVertexId {
    let sk = doc.sketch(s).expect("sketch is live");
    sk.vertices()
        .iter()
        .find(|(_, v)| approx_pt(v.position, p))
        .map(|(id, _)| id)
        .expect("a vertex at the given position")
}

/// Dragging one sketch corner repositions just that vertex, keeps the drawn
/// topology, and round-trips exactly through undo/redo with a stable `SketchId`.
#[test]
fn move_sketch_vertex_moves_and_round_trips() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 2.0, 2.0);
    let v = sketch_vertex_at(&doc, s, Point3::new(2.0, 2.0, 0.0));
    let regions0 = doc.extrudable_regions(s).unwrap().len();

    let change = doc
        .move_sketch_vertex(s, v, Point3::new(2.5, 1.7, 0.0))
        .expect("move corner");
    assert_eq!(
        change.sketches_touched,
        vec![s],
        "reports the touched sketch"
    );
    assert!(approx_pt(
        doc.sketch(s).unwrap().vertices()[v].position,
        Point3::new(2.5, 1.7, 0.0)
    ));
    assert_eq!(
        doc.extrudable_regions(s).unwrap().len(),
        regions0,
        "topology preserved"
    );

    doc.undo().expect("undo vertex move");
    assert!(
        approx_pt(
            doc.sketch(s).unwrap().vertices()[v].position,
            Point3::new(2.0, 2.0, 0.0)
        ),
        "undo restores the original corner"
    );
    assert!(doc.sketch_ids().contains(&s), "SketchId stable across undo");

    doc.redo().expect("redo vertex move");
    assert!(approx_pt(
        doc.sketch(s).unwrap().vertices()[v].position,
        Point3::new(2.5, 1.7, 0.0)
    ));
}

/// A drag that would re-topologize the sketch (corner swept across the far
/// side) is refused as a typed `Sketch` error, leaving the document untouched.
#[test]
fn move_sketch_vertex_rejects_a_retopologizing_drag() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 2.0, 2.0);
    let v = sketch_vertex_at(&doc, s, Point3::new(0.0, 0.0, 0.0));

    assert_eq!(
        doc.move_sketch_vertex(s, v, Point3::new(3.0, 1.0, 0.0)),
        Err(DocumentError::Sketch(SketchError::WouldRetopologize))
    );
    assert!(
        approx_pt(
            doc.sketch(s).unwrap().vertices()[v].position,
            Point3::new(0.0, 0.0, 0.0)
        ),
        "sketch untouched after a refused drag"
    );
}

#[test]
fn move_unknown_sketch_vertex_errors() {
    let mut doc = Document::new();
    let bogus = SketchId::default();
    assert_eq!(
        doc.move_sketch_vertex(bogus, SketchVertexId::default(), Point3::ORIGIN),
        Err(DocumentError::UnknownSketch)
    );
}

/// A hidden (deleted) sketch is not a vertex-move target.
#[test]
fn move_vertex_in_hidden_sketch_errors() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let v = sketch_vertex_at(&doc, s, Point3::new(0.0, 0.0, 0.0));
    doc.delete_sketch(s).expect("delete sketch");
    assert_eq!(
        doc.move_sketch_vertex(s, v, Point3::new(0.5, 0.5, 0.0)),
        Err(DocumentError::UnknownSketch)
    );
}

// --------------------------------------------------------------- error paths

#[test]
fn extrude_with_stale_sketch_errors() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);

    // The null handle is never returned by `add_sketch`, so it is always stale.
    let bogus = kernel::SketchId::default();
    assert_eq!(
        doc.extrude_region(bogus, r, 1.0),
        Err(kernel::DocumentError::UnknownSketch)
    );
}

#[test]
fn apply_op_on_hidden_object_errors() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);
    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    let top = top_face(doc.object(id).unwrap());

    doc.undo().expect("hide via undo");
    assert_eq!(
        doc.apply_object_op(
            id,
            KernelOp::PushPull {
                face: top,
                distance: 0.5
            }
        )
        .map(|_| ()),
        Err(kernel::DocumentError::UnknownObject)
    );
}

// ------------------------------------------------------------ merge groups

/// The set of visible top-level nodes, order-insensitive.
fn top_set(doc: &Document) -> HashSet<NodeId> {
    doc.top_level_nodes().into_iter().collect()
}

/// Grouping is non-destructive: both members stay visible, watertight, and
/// keep their handles; the group replaces them at the top level.
#[test]
fn group_is_non_destructive_and_contains_its_members() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);

    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group two objects");

    // Geometry untouched: both objects still visible and solid (unlike union).
    let visible: HashSet<_> = doc.visible_object_ids().into_iter().collect();
    assert_eq!(visible, HashSet::from([a, b]), "both members stay visible");
    assert_eq!(
        doc.object(a).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert_eq!(
        doc.object(b).unwrap().watertight(),
        WatertightState::Watertight
    );

    // The group is the only top-level node and lists exactly its members.
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Group(g)]));
    assert_eq!(
        doc.group_members(g)
            .unwrap()
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([NodeId::Object(a), NodeId::Object(b)])
    );
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(g));
    assert_eq!(doc.node_parent(NodeId::Object(b)), Some(g));
}

/// Groups nest: a group may contain another group plus an object, and
/// `leaf_objects_under` flattens the whole subtree.
#[test]
fn groups_nest_and_flatten_to_leaves() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);

    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("inner group");
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .expect("outer group nests a group and an object");

    assert_eq!(top_set(&doc), HashSet::from([NodeId::Group(outer)]));
    assert_eq!(doc.node_parent(NodeId::Group(inner)), Some(outer));
    assert_eq!(
        doc.leaf_objects_under(NodeId::Group(outer))
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([a, b, c]),
        "leaves flatten the whole subtree"
    );
}

/// Transforming a group bakes into every leaf beneath it (recursively) and
/// round-trips through undo/redo with stable handles.
#[test]
fn transform_group_moves_all_leaves_and_round_trips() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let orig_a = doc.object(a).unwrap().clone();
    let orig_b = doc.object(b).unwrap().clone();
    let orig_c = doc.object(c).unwrap().clone();
    let (ca, cb, cc) = (centroid(&orig_a), centroid(&orig_b), centroid(&orig_c));

    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .unwrap();

    let offset = Vec3::new(5.0, 3.0, 2.0);
    doc.transform_group(outer, &Transform::translation(offset))
        .expect("transform the outer group");

    assert!(approx_pt(centroid(doc.object(a).unwrap()), ca + offset));
    assert!(approx_pt(centroid(doc.object(b).unwrap()), cb + offset));
    assert!(approx_pt(centroid(doc.object(c).unwrap()), cc + offset));

    doc.undo().expect("undo group transform");
    assert!(objects_equivalent(doc.object(a).unwrap(), &orig_a));
    assert!(objects_equivalent(doc.object(b).unwrap(), &orig_b));
    assert!(objects_equivalent(doc.object(c).unwrap(), &orig_c));

    doc.redo().expect("redo group transform");
    assert!(approx_pt(centroid(doc.object(a).unwrap()), ca + offset));
    assert!(approx_pt(centroid(doc.object(c).unwrap()), cc + offset));
}

// ------------------------------------------- transform a whole mixed selection

/// `transform_selection` moves a mixed selection — a bare object, a group, a
/// component instance, and a free-standing sketch — and the entire act is
/// **one** undo step: a single undo restores every target exactly, a single
/// redo re-applies.
#[test]
fn transform_selection_moves_mixed_selection_in_one_undo_step() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let d = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 0.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(b), NodeId::Object(c)])
        .unwrap();
    let (_comp, inst, _) = doc.make_component(&[NodeId::Object(d)]).unwrap();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 10.0, 10.0, 11.0, 11.0);

    let orig_a = doc.object(a).unwrap().clone();
    let (ca, cb, cc) = (
        centroid(doc.object(a).unwrap()),
        centroid(doc.object(b).unwrap()),
        centroid(doc.object(c).unwrap()),
    );
    let sc = sketch_centroid(&doc, s);
    let probe = Point3::new(0.3, 0.7, 0.2);

    let offset = Vec3::new(5.0, -2.0, 3.0);
    let t = Transform::translation(offset);
    let change = doc
        .transform_selection(
            &[NodeId::Object(a), NodeId::Group(g), NodeId::Instance(inst)],
            &[s],
            &t,
        )
        .expect("transform the mixed selection");

    // Every target moved: baked objects, group leaves, sketch, instance pose.
    assert!(approx_pt(centroid(doc.object(a).unwrap()), ca + offset));
    assert!(approx_pt(centroid(doc.object(b).unwrap()), cb + offset));
    assert!(approx_pt(centroid(doc.object(c).unwrap()), cc + offset));
    assert!(approx_pt(sketch_centroid(&doc, s), sc + offset));
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(t.apply_point(probe), 1e-9),
        "instance pose composed with t"
    );

    // The change reports everything it touched.
    assert_eq!(
        HashSet::from_iter(change.objects_touched.iter().copied()),
        HashSet::from([a, b, c])
    );
    assert_eq!(change.sketches_touched, vec![s]);
    assert_eq!(change.groups_touched, vec![g]);
    assert_eq!(change.instances_touched, vec![inst]);

    // ONE undo restores the whole selection.
    doc.undo().expect("undo the selection transform");
    assert!(objects_equivalent(doc.object(a).unwrap(), &orig_a));
    assert!(approx_pt(centroid(doc.object(b).unwrap()), cb));
    assert!(approx_pt(centroid(doc.object(c).unwrap()), cc));
    assert!(approx_pt(sketch_centroid(&doc, s), sc));
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(probe, 1e-9),
        "undo restores the exact prior (identity) pose"
    );
    // ONE redo re-applies it all.
    doc.redo().expect("redo the selection transform");
    assert!(approx_pt(centroid(doc.object(a).unwrap()), ca + offset));
    assert!(approx_pt(centroid(doc.object(c).unwrap()), cc + offset));
    assert!(approx_pt(sketch_centroid(&doc, s), sc + offset));
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(t.apply_point(probe), 1e-9)
    );
}

/// A node listed alongside its ancestor group transforms once, not twice.
#[test]
fn transform_selection_dedups_a_node_listed_with_its_ancestor_group() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let ca = centroid(doc.object(a).unwrap());

    let offset = Vec3::new(1.0, 2.0, 3.0);
    doc.transform_selection(
        &[NodeId::Group(g), NodeId::Object(a)],
        &[],
        &Transform::translation(offset),
    )
    .expect("group and member listed together");

    assert!(
        approx_pt(centroid(doc.object(a).unwrap()), ca + offset),
        "the doubly-listed leaf moved exactly once"
    );
}

proptest! {
    /// Property (DEVELOPMENT.md rule 3): for an arbitrary orientation-
    /// preserving similarity (scale → rotate → translate), transforming a
    /// mixed selection (bare object + group + free sketch) moves every
    /// baked centroid by exactly the map, one undo restores every target
    /// exactly, and re-listing a group member alongside its ancestor group
    /// is the identity on the outcome (dedup: one bake per leaf).
    #[test]
    fn transform_selection_round_trips_and_dedups_under_random_maps(
        dx in -10.0..10.0f64,
        dy in -10.0..10.0f64,
        dz in -10.0..10.0f64,
        angle in -3.0..3.0f64,
        scale in 0.25..4.0f64,
    ) {
        fn build(doc: &mut Document) -> (ObjectId, ObjectId, ObjectId, GroupId, SketchId) {
            let a = extrude_box(doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
            let b = extrude_box(doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
            let c = extrude_box(doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
            let (g, _) = doc
                .group_nodes(&[NodeId::Object(b), NodeId::Object(c)])
                .unwrap();
            let s = doc.add_sketch(ground());
            draw_rect(doc, s, 8.0, 8.0, 9.0, 9.0);
            (a, b, c, g, s)
        }
        let t = Transform::uniform_scale(scale)
            .then(&Transform::rotation(Vec3::new(0.0, 0.0, 1.0), angle).unwrap())
            .then(&Transform::translation(Vec3::new(dx, dy, dz)));

        let mut doc = Document::new();
        let (a, b, c, g, s) = build(&mut doc);
        let originals = [
            doc.object(a).unwrap().clone(),
            doc.object(b).unwrap().clone(),
            doc.object(c).unwrap().clone(),
        ];
        let centroids = [
            centroid(doc.object(a).unwrap()),
            centroid(doc.object(b).unwrap()),
            centroid(doc.object(c).unwrap()),
        ];
        let sc = sketch_centroid(&doc, s);

        doc.transform_selection(&[NodeId::Object(a), NodeId::Group(g)], &[s], &t)
            .expect("transform the mixed selection");

        // An affine map commutes with centroids, so each baked centroid
        // lands exactly on the mapped original.
        for (&obj, &c0) in [a, b, c].iter().zip(&centroids) {
            prop_assert!(approx_pt(centroid(doc.object(obj).unwrap()), t.apply_point(c0)));
        }
        prop_assert!(approx_pt(sketch_centroid(&doc, s), t.apply_point(sc)));

        // One undo restores every target exactly.
        doc.undo().expect("undo the selection transform");
        for (&obj, orig) in [a, b, c].iter().zip(&originals) {
            prop_assert!(objects_equivalent(doc.object(obj).unwrap(), orig));
        }
        prop_assert!(approx_pt(sketch_centroid(&doc, s), sc));

        // Dedup: listing a member with its ancestor group changes nothing.
        let mut doc2 = Document::new();
        let (a2, b2, c2, g2, s2) = build(&mut doc2);
        doc2.transform_selection(
            &[NodeId::Object(a2), NodeId::Group(g2), NodeId::Object(b2)],
            &[s2],
            &t,
        )
        .expect("transform with a doubly-listed member");
        for (&obj, &c0) in [a2, b2, c2].iter().zip(&centroids) {
            prop_assert!(approx_pt(centroid(doc2.object(obj).unwrap()), t.apply_point(c0)));
        }
    }
}

/// Empty, stale, and degenerate inputs are refused loudly, leaving the
/// document untouched (the strong guarantee).
#[test]
fn transform_selection_refuses_empty_stale_and_degenerate() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let ca = centroid(doc.object(a).unwrap());
    let t = Transform::translation(Vec3::new(1.0, 0.0, 0.0));

    assert_eq!(
        doc.transform_selection(&[], &[], &t),
        Err(DocumentError::EmptySelection)
    );
    assert_eq!(
        doc.transform_selection(&[NodeId::Object(kernel::ObjectId::default())], &[], &t),
        Err(DocumentError::UnknownObject)
    );
    assert_eq!(
        doc.transform_selection(&[], &[SketchId::default()], &t),
        Err(DocumentError::UnknownSketch)
    );
    // Reflection is refused for a selection with baked targets; the object is
    // untouched afterwards.
    assert_eq!(
        doc.transform_selection(&[NodeId::Object(a)], &[], &Transform::uniform_scale(-1.0)),
        Err(DocumentError::Transform(TransformError::Reflection))
    );
    assert_eq!(
        doc.transform_selection(&[NodeId::Object(a)], &[], &Transform::uniform_scale(0.0)),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert!(
        approx_pt(centroid(doc.object(a).unwrap()), ca),
        "object untouched after refused transforms"
    );
}

/// Group then ungroup restores the original top-level shape; the members keep
/// their handles and geometry. Undo/redo of each step is an identity.
#[test]
fn group_then_ungroup_round_trips() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let before = top_set(&doc);
    assert_eq!(
        before,
        HashSet::from([NodeId::Object(a), NodeId::Object(b)])
    );

    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.ungroup(g).expect("ungroup");

    assert_eq!(top_set(&doc), before, "top-level shape restored");
    assert!(doc.group_ids().is_empty(), "the group is gone (hidden)");
    assert_eq!(doc.node_parent(NodeId::Object(a)), None);

    // Undo the ungroup → group is back; undo the group → flat again.
    doc.undo().expect("undo ungroup");
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Group(g)]));
    doc.undo().expect("undo group");
    assert_eq!(top_set(&doc), before);

    // Redo both → grouped then ungrouped again.
    doc.redo().expect("redo group");
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Group(g)]));
    doc.redo().expect("redo ungroup");
    assert_eq!(top_set(&doc), before);
}

/// Ungroup returns members to the *group's own parent*, not the top level,
/// when the group is nested.
#[test]
fn ungroup_returns_members_to_the_groups_parent() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .unwrap();

    doc.ungroup(inner).expect("ungroup the inner group");

    // a and b now sit directly under `outer`, alongside c.
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(outer));
    assert_eq!(doc.node_parent(NodeId::Object(b)), Some(outer));
    assert_eq!(
        doc.group_members(outer)
            .unwrap()
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([NodeId::Object(a), NodeId::Object(b), NodeId::Object(c)])
    );
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Group(outer)]));
}

/// Grouping refuses degenerate selections without mutating the document.
#[test]
fn group_refuses_bad_selections() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);

    assert_eq!(
        doc.group_nodes(&[]).map(|_| ()),
        Err(DocumentError::EmptyGroup)
    );
    assert_eq!(
        doc.group_nodes(&[NodeId::Object(a), NodeId::Object(a)])
            .map(|_| ()),
        Err(DocumentError::DuplicateMember)
    );

    // Group a and b; now they are siblings under a group, so grouping a with
    // the still-top-level c mixes parents.
    let (_, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    assert_eq!(
        doc.group_nodes(&[NodeId::Object(a), NodeId::Object(c)])
            .map(|_| ()),
        Err(DocumentError::MixedParents)
    );

    // A stale object handle is refused as unknown.
    let bogus = kernel::ObjectId::default();
    assert_eq!(
        doc.group_nodes(&[NodeId::Object(bogus)]).map(|_| ()),
        Err(DocumentError::UnknownObject)
    );
}

/// A single-member group is allowed (a degenerate but valid container).
#[test]
fn single_member_group_is_allowed() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group of one");
    assert_eq!(doc.group_members(g).unwrap(), vec![NodeId::Object(a)]);
}

/// Ungrouping an unknown/hidden group is refused.
#[test]
fn ungroup_unknown_group_errors() {
    let mut doc = Document::new();
    assert_eq!(
        doc.ungroup(GroupId::default()),
        Err(DocumentError::UnknownGroup)
    );
}

/// Transforming an unknown group is refused without mutation.
#[test]
fn transform_unknown_group_errors() {
    let mut doc = Document::new();
    assert_eq!(
        doc.transform_group(
            GroupId::default(),
            &Transform::translation(Vec3::new(1.0, 0.0, 0.0))
        ),
        Err(DocumentError::UnknownGroup)
    );
}

// --------------------------------------------------------- whole-node delete

/// Deleting an object hides it (it disappears from `top_level_nodes`); undo
/// restores the identical top-level tree (same ids, same order); redo removes
/// it again.
#[test]
fn delete_object_removes_then_undo_restores_top_level_order() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let before = doc.top_level_nodes();
    assert_eq!(before, vec![NodeId::Object(a), NodeId::Object(b)]);

    let change = doc.delete_node(NodeId::Object(a)).expect("delete a");
    assert_eq!(doc.top_level_nodes(), vec![NodeId::Object(b)]);
    assert!(change.objects_touched.contains(&a));

    doc.undo().expect("undo delete");
    assert_eq!(
        doc.top_level_nodes(),
        before,
        "undo restores the identical top-level tree"
    );

    doc.redo().expect("redo delete");
    assert_eq!(doc.top_level_nodes(), vec![NodeId::Object(b)]);
}

/// Deleting a group hides the group and every leaf object beneath it in one
/// undoable step; undo restores the whole subtree (group + members, same
/// parent/child relationships).
#[test]
fn delete_group_hides_whole_subtree_and_undo_restores_it() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let before = top_set(&doc);
    assert_eq!(before, HashSet::from([NodeId::Group(g), NodeId::Object(c)]));

    let change = doc.delete_node(NodeId::Group(g)).expect("delete group");
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Object(c)]));
    assert!(doc.group_ids().is_empty(), "the group is hidden");
    assert!(
        change.objects_touched.contains(&a) && change.objects_touched.contains(&b),
        "deleting a group touches its leaf objects too"
    );
    assert!(change.groups_touched.contains(&g));

    doc.undo().expect("undo delete group");
    assert_eq!(top_set(&doc), before, "the whole subtree reappears");
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(g));
    assert_eq!(doc.node_parent(NodeId::Object(b)), Some(g));
    assert_eq!(
        doc.group_members(g)
            .unwrap()
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([NodeId::Object(a), NodeId::Object(b)]),
        "member order/membership is restored exactly"
    );

    doc.redo().expect("redo delete group");
    assert_eq!(top_set(&doc), HashSet::from([NodeId::Object(c)]));
    assert!(doc.group_ids().is_empty());
}

/// Deleting a component instance hides only that instance; its definition and
/// sibling instances are completely untouched. Undo restores it.
#[test]
fn delete_instance_leaves_definition_and_siblings_untouched() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, i1, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();

    let change = doc.delete_node(NodeId::Instance(i1)).expect("delete i1");
    assert_eq!(
        doc.instance_ids().into_iter().collect::<HashSet<_>>(),
        HashSet::from([i2]),
        "only i1 is hidden"
    );
    assert!(
        doc.component_ids().contains(&comp),
        "the shared definition survives"
    );
    assert_eq!(
        doc.instance_def(i2),
        Some(comp),
        "the sibling instance is untouched"
    );
    assert!(change.instances_touched.contains(&i1));
    assert!(
        !change.components_touched.contains(&comp),
        "deleting an instance must not touch the shared ComponentDef"
    );

    doc.undo().expect("undo delete instance");
    assert_eq!(
        doc.instance_ids().into_iter().collect::<HashSet<_>>(),
        HashSet::from([i1, i2]),
        "both instances are back"
    );
    assert_eq!(doc.instance_def(i1), Some(comp));
}

/// delete → undo round-trips to a byte-identical document (undo history is not
/// persisted, so `save()` before the delete must equal `save()` after
/// delete-then-undo).
#[test]
fn delete_then_undo_round_trips_to_byte_identical_save() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let before = doc.save();

    doc.delete_node(NodeId::Object(a)).expect("delete a");
    doc.undo().expect("undo delete");

    assert_eq!(
        doc.save(),
        before,
        "delete-then-undo must be byte-identical to the pre-delete document"
    );
}

/// Deleting an unknown/stale/hidden node is refused without mutating the
/// document (the strong guarantee); a deleted node cannot be deleted again.
#[test]
fn delete_unknown_or_already_deleted_node_errors() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);

    assert_eq!(
        doc.delete_node(NodeId::Object(ObjectId::default())),
        Err(DocumentError::UnknownObject)
    );
    assert_eq!(
        doc.delete_node(NodeId::Group(GroupId::default())),
        Err(DocumentError::UnknownGroup)
    );

    doc.delete_node(NodeId::Object(a)).expect("delete a");
    assert_eq!(
        doc.delete_node(NodeId::Object(a)),
        Err(DocumentError::UnknownObject),
        "deleting an already-hidden node is refused, not a no-op success"
    );
}

// ----------------------------------------------------------------- helper

/// The +Z (top) face of an extruded box.
fn top_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("a top face exists")
}

// ------------------------------------------------- components
//
// Acceptance specs for the Components slice. Each is `#[ignore]`d because its
// op is a `todo!()` stub ( stub-first); un-ignore it in the PR that
// implements the op. The assertions are the contract — never weaken them.

/// Make Component folds a selection into a flat definition plus one
/// identity-posed instance, **without moving geometry** (def-local frame =
/// world-at-creation): the folded object keeps its handle and shape, drops
/// out of the world-object set, and a stand-in instance takes its place.
#[test]
fn make_component_folds_selection_into_a_definition_at_identity() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let before = doc.object(o).expect("object live").clone();

    let (comp, inst, _change) = doc
        .make_component(&[NodeId::Object(o)])
        .expect("make component");

    // The object is now a definition member: same handle, unchanged geometry,
    // no longer a world object.
    assert!(
        !doc.visible_object_ids().contains(&o),
        "a definition member is not a world object"
    );
    assert_eq!(doc.def_members(comp), Some(vec![o]));
    assert!(
        objects_equivalent(&before, doc.object(o).expect("member still live")),
        "make_component moves no geometry"
    );

    // One identity-posed instance stands in its place at the top level.
    assert_eq!(doc.instance_def(inst), Some(comp));
    assert_eq!(doc.instance_pose(inst), Some(Transform::IDENTITY));
    assert!(doc.top_level_nodes().contains(&NodeId::Instance(inst)));
    assert_eq!(doc.instances_of(comp), vec![inst]);
}

/// The defining property of components: instances share one definition, so an
/// `apply_def_op` edit to the shared geometry is seen by every instance — and
/// the change names the component and all its instances for the shim to refresh.
#[test]
fn editing_a_definition_changes_every_instance() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, i1, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();

    assert_eq!(doc.instance_def(i1), doc.instance_def(i2));
    let member = doc.def_members(comp).unwrap()[0];
    let vol_before = signed_volume(doc.object(member).unwrap());

    // Edit the shared geometry: push the member's top face up.
    let face = top_face(doc.object(member).unwrap());
    let (_report, change) = doc
        .apply_def_op(
            comp,
            member,
            KernelOp::PushPull {
                face,
                distance: 1.0,
            },
        )
        .unwrap();

    assert!(
        signed_volume(doc.object(member).unwrap()) > vol_before,
        "the shared geometry grew"
    );
    assert!(change.components_touched.contains(&comp));
    assert!(
        change.instances_touched.contains(&i1) && change.instances_touched.contains(&i2),
        "a shared-geometry edit touches every instance"
    );
}

/// A move composes into the pose (never baked), accepts mirror and non-uniform
/// scale, refuses a singular transform, and undoes to the exact prior pose.
#[test]
fn transform_instance_composes_pose_and_undo_is_exact() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (_comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let probe = Point3::new(0.3, 0.7, 0.2);

    let t1 = Transform::translation(Vec3::new(2.0, 0.0, 0.0));
    doc.transform_instance(inst, &t1).unwrap();
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(t1.apply_point(probe), 1e-9),
        "identity then t1 == t1"
    );

    let t2 = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.5).unwrap();
    doc.transform_instance(inst, &t2).unwrap();
    let composed = t1.then(&t2);
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(composed.apply_point(probe), 1e-9),
        "poses compose: pose' = pose.then(t)"
    );

    doc.undo().unwrap();
    assert!(
        doc.instance_pose(inst)
            .unwrap()
            .apply_point(probe)
            .approx_eq(t1.apply_point(probe), 1e-9),
        "undo restores the exact prior pose"
    );

    // Mirror and non-uniform scale are allowed; only singular is refused.
    assert!(
        doc.transform_instance(inst, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
            .is_ok(),
        "mirroring an instance is allowed"
    );
    assert!(
        doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 3.0, 0.5)))
            .is_ok(),
        "non-uniform scale on an instance is allowed"
    );
    assert_eq!(
        doc.transform_instance(inst, &Transform::uniform_scale(0.0)),
        Err(DocumentError::Transform(TransformError::Singular))
    );
}

/// Explode bakes an orientation-preserving pose into independent world objects
/// (equal to the posed instance), leaving the definition intact; a mirrored
/// instance refuses (baking a reflection would invert winding).
#[test]
fn explode_bakes_pose_into_world_objects_and_refuses_mirror() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let pose = Transform::translation(Vec3::new(4.0, 0.0, 0.0));
    doc.transform_instance(inst, &pose).unwrap();

    // Expected geometry: the shared member baked by the instance pose.
    let member = doc.def_members(comp).unwrap()[0];
    let mut expected = doc.object(member).unwrap().clone();
    expected.apply_transform(&pose).unwrap();

    let (created, _change) = doc.explode_instance(inst).unwrap();
    assert_eq!(created.len(), 1);
    assert!(
        objects_equivalent(
            &expected,
            doc.object(created[0]).expect("exploded object live")
        ),
        "exploded geometry equals the posed instance"
    );
    assert!(
        doc.visible_object_ids().contains(&created[0]),
        "the exploded result is an independent world object"
    );
    assert!(doc.instance_pose(inst).is_none(), "the instance is gone");
    assert!(
        doc.object(member).is_some(),
        "the definition (and its member) is untouched by explode"
    );

    // A mirrored instance refuses to explode.
    let m = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (_c2, inst2, _) = doc.make_component(&[NodeId::Object(m)]).unwrap();
    doc.transform_instance(inst2, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .unwrap();
    assert_eq!(
        doc.explode_instance(inst2),
        Err(DocumentError::CannotExplodeReflected)
    );
}

/// Make Unique gives one instance its own private copy of the definition, so a
/// later edit to it no longer affects its former siblings.
#[test]
fn make_unique_detaches_an_instance_from_its_siblings() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, i1, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .unwrap();

    let (new_comp, _change) = doc.make_unique(i2).unwrap();
    assert_ne!(new_comp, comp);
    assert_eq!(doc.instance_def(i1), Some(comp));
    assert_eq!(doc.instance_def(i2), Some(new_comp));

    let m1 = doc.def_members(comp).unwrap()[0];
    let v1_before = signed_volume(doc.object(m1).unwrap());
    let m2 = doc.def_members(new_comp).unwrap()[0];
    let face = top_face(doc.object(m2).unwrap());
    doc.apply_def_op(
        new_comp,
        m2,
        KernelOp::PushPull {
            face,
            distance: 1.0,
        },
    )
    .unwrap();

    assert_eq!(
        signed_volume(doc.object(m1).unwrap()),
        v1_before,
        "the former sibling's definition is untouched"
    );
    assert!(
        signed_volume(doc.object(m2).unwrap()) > v1_before,
        "the unique copy grew"
    );
}

/// make_component then place_instance round-trips through document undo/redo,
/// restoring the original world object on undo and the *same* node handles on
/// redo (hide-not-delete).
#[test]
fn component_actions_round_trip_through_undo_redo() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .unwrap();
    let top_before = top_set(&doc);

    doc.undo().unwrap(); // undo place_instance
    assert!(doc.instance_pose(i2).is_none());
    doc.undo().unwrap(); // undo make_component
    assert!(
        doc.visible_object_ids().contains(&o),
        "the folded object is a world solid again"
    );
    assert!(doc.instance_pose(inst).is_none());
    assert!(doc.component_ids().is_empty());

    doc.redo().unwrap(); // redo make_component
    doc.redo().unwrap(); // redo place_instance
    assert_eq!(
        top_set(&doc),
        top_before,
        "redo restores the same top-level node set (stable handles)"
    );
    assert_eq!(doc.instance_def(inst), Some(comp));
    assert_eq!(doc.instance_def(i2), Some(comp));
}

// ------------------------------------------------------- materials

/// The face of `obj` whose plane normal matches `n` (within a tight tolerance).
fn face_with_normal(doc: &Document, obj: ObjectId, n: Vec3) -> FaceId {
    let object = doc.object(obj).expect("live object");
    object
        .faces()
        .iter()
        .find(|(_, f)| {
            let fn_ = f.plane.normal();
            (fn_.x - n.x).abs() < 1e-9 && (fn_.y - n.y).abs() < 1e-9 && (fn_.z - n.z).abs() < 1e-9
        })
        .map(|(id, _)| id)
        .expect("a face with that normal exists")
}

/// How many of `obj`'s faces currently carry `mat`.
fn faces_painted(doc: &Document, obj: ObjectId, mat: MaterialId) -> usize {
    doc.object(obj)
        .expect("live object")
        .faces()
        .values()
        .filter(|f| f.material == Some(mat))
        .count()
}

#[test]
fn paint_face_sets_and_clears_material() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let top = face_with_normal(&doc, o, Vec3::new(0.0, 0.0, 1.0));

    assert_eq!(doc.face_material(o, top), None, "unpainted = default");
    doc.paint_face(o, top, Some(red)).expect("paint");
    assert_eq!(doc.face_material(o, top), Some(red));

    // Painting None resets to the default material.
    doc.paint_face(o, top, None).expect("unpaint");
    assert_eq!(doc.face_material(o, top), None);
}

#[test]
fn paint_face_rejects_unknown_inputs() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let top = face_with_normal(&doc, o, Vec3::new(0.0, 0.0, 1.0));

    // Unknown material handle (from a different, empty document) is refused.
    let mut other = Document::new();
    let stray = other.add_material(Material::solid("X", Rgba8::rgb(0, 0, 0)));
    assert_eq!(
        doc.paint_face(o, top, Some(stray)),
        Err(DocumentError::UnknownMaterial)
    );

    // Unknown face handle is refused.
    let stray_face = FaceId::default();
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    assert_eq!(
        doc.paint_face(o, stray_face, Some(red)),
        Err(DocumentError::UnknownFace)
    );
}

#[test]
fn paint_face_undo_redo_is_exact() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220)));
    let top = face_with_normal(&doc, o, Vec3::new(0.0, 0.0, 1.0));

    doc.paint_face(o, top, Some(red)).unwrap();
    doc.paint_face(o, top, Some(blue)).unwrap();
    assert_eq!(doc.face_material(o, top), Some(blue));

    doc.undo().unwrap();
    assert_eq!(doc.face_material(o, top), Some(red), "undo restores prev");
    doc.undo().unwrap();
    assert_eq!(doc.face_material(o, top), None, "undo restores default");

    doc.redo().unwrap();
    assert_eq!(doc.face_material(o, top), Some(red), "redo re-applies");
    doc.redo().unwrap();
    assert_eq!(doc.face_material(o, top), Some(blue));
}

/// Splitting a painted top face of an *extruded* box propagates the material to
/// both halves and produces valid topology. This previously exposed a
/// seed-dependent `split_face` crash (dangling `vertex.outgoing` on extruded-box
/// faces — DESIGN risk #1, fixed); it is restored here as a
/// Document-level guard alongside the in-crate
/// `ops::tests::split_face_propagates_material_to_both_halves` (on `unit_cube`).
#[test]
fn split_painted_extruded_box_face_propagates_material() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));

    let top = face_with_normal(&doc, o, Vec3::new(0.0, 0.0, 1.0));
    doc.paint_face(o, top, Some(red)).expect("paint top");

    let (report, _) = doc
        .apply_object_op(
            o,
            KernelOp::SplitFace {
                face: top,
                path: vec![Point3::new(0.5, 0.0, 1.0), Point3::new(0.5, 1.0, 1.0)],
            },
        )
        .expect("split the painted extruded top");

    let new_faces = match report {
        KernelOpReport::FaceSplit(r) => r.new_faces,
        other => panic!("expected a FaceSplit report, got {other:?}"),
    };
    for fid in new_faces {
        assert_eq!(
            doc.face_material(o, fid),
            Some(red),
            "both halves of the split inherit the painted material"
        );
    }
}

#[test]
fn boolean_preserves_operand_face_materials() {
    let mut doc = Document::new();
    // Two overlapping boxes in general position (offset in z so faces aren't
    // coplanar), so union merges them into one solid.
    let a = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let b = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 1.0, 3.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220)));

    // Paint a face on each operand that survives onto the union boundary.
    let a_bottom = face_with_normal(&doc, a, Vec3::new(0.0, 0.0, -1.0));
    let b_top = face_with_normal(&doc, b, Vec3::new(0.0, 0.0, 1.0));
    doc.paint_face(a, a_bottom, Some(red)).unwrap();
    doc.paint_face(b, b_top, Some(blue)).unwrap();

    let (result, _) = doc.boolean(BooleanOp::Union, a, b).expect("union");

    assert!(
        faces_painted(&doc, result, red) >= 1,
        "operand A's material survives the boolean onto its source faces"
    );
    assert!(
        faces_painted(&doc, result, blue) >= 1,
        "operand B's material survives the boolean onto its source faces"
    );
}

// ----------------------------------------- object base material ( follow-up)

#[test]
fn set_object_material_sets_clears_and_undo_redo() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));

    assert_eq!(doc.object(o).unwrap().default_material(), None);
    doc.set_object_material(o, Some(red)).expect("set base");
    assert_eq!(doc.object(o).unwrap().default_material(), Some(red));

    doc.undo().unwrap();
    assert_eq!(
        doc.object(o).unwrap().default_material(),
        None,
        "undo clears base"
    );
    doc.redo().unwrap();
    assert_eq!(
        doc.object(o).unwrap().default_material(),
        Some(red),
        "redo restores"
    );

    doc.set_object_material(o, None).expect("clear base");
    assert_eq!(doc.object(o).unwrap().default_material(), None);
}

#[test]
fn set_object_material_rejects_unknown_material() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let mut other = Document::new();
    let stray = other.add_material(Material::solid("X", Rgba8::rgb(0, 0, 0)));
    assert_eq!(
        doc.set_object_material(o, Some(stray)),
        Err(DocumentError::UnknownMaterial)
    );
}

// --------------------------------------------------- material palette opacity

#[test]
fn set_material_alpha_sets_and_undo_redo() {
    let mut doc = Document::new();
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    assert_eq!(doc.material(red).unwrap().color.a, 255);

    doc.set_material_alpha(red, 128).expect("set alpha");
    assert_eq!(doc.material(red).unwrap().color.a, 128);

    doc.undo().unwrap();
    assert_eq!(
        doc.material(red).unwrap().color.a,
        255,
        "undo restores prev alpha"
    );
    doc.redo().unwrap();
    assert_eq!(doc.material(red).unwrap().color.a, 128, "redo re-applies");
}

#[test]
fn set_material_alpha_rejects_unknown_material() {
    let mut doc = Document::new();
    let mut other = Document::new();
    let stray = other.add_material(Material::solid("X", Rgba8::rgb(0, 0, 0)));
    assert_eq!(
        doc.set_material_alpha(stray, 100),
        Err(DocumentError::UnknownMaterial)
    );
}

#[test]
fn set_material_alpha_noop_does_not_record_undo() {
    let mut doc = Document::new();
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    doc.set_material_alpha(red, 255)
        .expect("setting to the already-current value is a no-op, not an error");
    assert_eq!(doc.material(red).unwrap().color.a, 255);
    assert_eq!(doc.undo(), Err(DocumentError::NothingToUndo));
}

#[test]
fn set_material_alpha_applies_uniformly_to_a_textured_material() {
    // `color`'s alpha modulates a texture too, so opacity must not be
    // restricted to flat-color materials.
    let mut doc = Document::new();
    let tex = Texture {
        image: vec![0u8; 4],
        format: ImageFormat::Png,
        world_size: [1.0, 1.0],
    };
    let glass = doc.add_material(Material::textured("Glass", Rgba8::rgb(200, 220, 255), tex));

    doc.set_material_alpha(glass, 96).expect("set alpha");
    assert_eq!(doc.material(glass).unwrap().color.a, 96);
    assert!(doc.material(glass).unwrap().has_texture());
}

#[test]
fn explicit_face_paint_overrides_object_base() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220)));
    let top = face_with_normal(&doc, o, Vec3::new(0.0, 0.0, 1.0));

    doc.set_object_material(o, Some(red)).unwrap();
    doc.paint_face(o, top, Some(blue)).unwrap();

    // The painted face keeps its own material; the base stays red for the rest
    // (non-destructive — face overrides win, base covers the unpainted faces).
    assert_eq!(doc.face_material(o, top), Some(blue));
    assert_eq!(doc.object(o).unwrap().default_material(), Some(red));
}

#[test]
fn boolean_result_inherits_operand_a_base_material() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let b = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 1.0, 3.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    doc.set_object_material(a, Some(red)).unwrap();

    let (result, _) = doc.boolean(BooleanOp::Subtract, a, b).expect("subtract");
    assert_eq!(
        doc.object(result).unwrap().default_material(),
        Some(red),
        "the subtract result inherits operand A's base material, so carved \
         walls from an unpainted cutter resolve to A's color"
    );
}

// ---------------------------------------- consumed-edge tombstone index

/// Helper: count visible (non-consumed) edges in a sketch.
fn visible_edge_count(doc: &Document, sid: SketchId) -> usize {
    let sk = doc.sketch(sid).expect("sketch is live");
    sk.edges()
        .keys()
        .filter(|&eid| !doc.is_sketch_edge_consumed(sid, eid))
        .count()
}

/// After extruding the sole rectangle on a ground sketch, all 4 of its
/// boundary edges are tombstoned (no longer visible). The count survives a
/// save → load round-trip (tombstones are rebuilt from the consumed-region
/// set on load).
#[test]
fn extruded_sketch_edges_are_tombstoned_and_survive_round_trip() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);

    // Before extrusion: all 4 edges are visible.
    assert_eq!(visible_edge_count(&doc, s), 4);

    // Before extrusion the sketch is an actionable entity.
    assert!(doc.sketch_ids().contains(&s));

    doc.extrude_region(s, r, 1.0).expect("extrude");

    // After extrusion: 0 visible edges (the outline is consumed) AND the sketch
    // itself no longer exists as an actionable entity — once wholly subsumed
    // into the solid it drops out of `sketch_ids` entirely (the user can't do
    // anything with a fully-consumed sketch, so it must not linger).
    assert_eq!(
        visible_edge_count(&doc, s),
        0,
        "all 4 boundary edges should be tombstoned after extrusion"
    );
    assert!(
        !doc.sketch_ids().contains(&s),
        "a fully-consumed sketch must vanish from sketch_ids"
    );

    // Undo: edges reappear and the sketch is actionable again.
    doc.undo().expect("undo");
    assert_eq!(
        visible_edge_count(&doc, s),
        4,
        "edges must reappear after undoing the extrusion"
    );
    assert!(
        doc.sketch_ids().contains(&s),
        "the sketch must come back after undoing the extrusion"
    );

    // Redo: edges hidden again and the sketch vanishes again.
    doc.redo().expect("redo");
    assert_eq!(
        visible_edge_count(&doc, s),
        0,
        "edges hidden again after redo"
    );
    assert!(
        !doc.sketch_ids().contains(&s),
        "sketch gone again after redo"
    );

    // Save → load: the consumed state is rebuilt from the consumed-region set,
    // so the fully-consumed sketch stays gone (not resurrected as actionable)
    // and only the solid survives.
    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    assert!(
        doc2.sketch_ids().is_empty(),
        "the fully-consumed sketch must not reappear after save/load"
    );
    assert_eq!(
        doc2.visible_object_ids().len(),
        1,
        "the extruded solid survives the round-trip"
    );
}

// ──────────────────────────────────── node metadata ops (WS3) ───────────────

/// `set_node_name` undo/redo: after rename+undo, redo re-applies the name.
#[test]
fn set_node_name_undo_restores_prior_name() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);

    // Rename then undo — verifiable via another rename/tag after undo.
    doc.set_node_name(node, Some("Widget".to_string())).unwrap();
    // Undo the rename; the node's tags should still be empty (cross-check via
    // node_tags which we can observe — name is internal, but tag state is
    // independent and also reset by undo).
    doc.add_node_tag(node, vec!["T".to_string()]).unwrap();
    doc.undo().expect("undo add_node_tag");
    doc.undo().expect("undo rename");
    // Now redo the rename → name should be Widget again.
    doc.redo().expect("redo rename");
    // Redo the tag add → tag back.
    doc.redo().expect("redo add_node_tag");
    assert_eq!(
        doc.node_tags(node),
        &[vec!["T".to_string()]],
        "redo restored the tag"
    );
}

/// `add_node_tag` / `remove_node_tag` are inverse operations (identity on tags).
#[test]
fn add_then_remove_tag_is_identity() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);
    let tag = vec!["Structure".to_string(), "Roof".to_string()];

    assert_eq!(doc.node_tags(node), &[] as &[Vec<String>]);

    doc.add_node_tag(node, tag.clone()).unwrap();
    assert_eq!(doc.node_tags(node), std::slice::from_ref(&tag));

    doc.remove_node_tag(node, &tag).unwrap();
    assert_eq!(doc.node_tags(node), &[] as &[Vec<String>]);
}

/// After `add_node_tag`, undo removes the tag; redo re-adds it.
#[test]
fn add_node_tag_undo_redo_roundtrip() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);
    let tag = vec!["Mechanical".to_string()];

    doc.add_node_tag(node, tag.clone()).unwrap();
    assert_eq!(doc.node_tags(node), std::slice::from_ref(&tag));

    doc.undo().expect("undo add_node_tag");
    assert_eq!(
        doc.node_tags(node),
        &[] as &[Vec<String>],
        "undo should remove the tag"
    );

    doc.redo().expect("redo add_node_tag");
    assert_eq!(
        doc.node_tags(node),
        std::slice::from_ref(&tag),
        "redo should re-add the tag"
    );
}

/// `add_node_tag` is idempotent: adding a duplicate does NOT push an undo entry.
#[test]
fn add_duplicate_tag_is_no_op_no_undo_entry() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);
    let tag = vec!["Structural".to_string()];

    doc.add_node_tag(node, tag.clone()).unwrap();
    let undo_depth_before = doc.can_undo(); // true
    // Adding the exact same tag again should be a no-op.
    doc.add_node_tag(node, tag.clone()).unwrap();
    // Tags are still just one entry.
    assert_eq!(doc.node_tags(node).len(), 1);
    // Only one undo entry (the first add). Undo the first add, then there
    // should be nothing left on the undo stack (beyond the extrude).
    doc.undo().unwrap(); // undo first add_node_tag
    // The duplicate add did NOT push an undo entry, so we're now at extrude.
    // The undo/redo stacks's depth can be probed by trying to undo the extrude.
    assert!(doc.can_undo(), "extrude is still on the undo stack");
    // But the tag is gone now.
    assert_eq!(doc.node_tags(node), &[] as &[Vec<String>]);
    let _ = undo_depth_before;
}

/// `set_node_name` to the current name is a no-op: it pushes no undo entry, so a
/// UI focus-blur that re-commits the same name never pollutes the undo stack.
#[test]
fn rename_to_same_name_is_no_op_no_undo_entry() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);

    doc.set_node_name(node, Some("Widget".to_string())).unwrap();
    // Re-commit the identical name — must not push another undo entry.
    doc.set_node_name(node, Some("Widget".to_string())).unwrap();
    // Undo once → back to the pre-rename (unnamed) state; only the first rename
    // and the extrude were ever recorded.
    doc.undo().expect("undo the single rename");
    assert!(doc.can_undo(), "only the extrude remains on the undo stack");
    // Re-commit None when already None is likewise a no-op (no panic, no entry).
    doc.set_node_name(node, None).unwrap();
    assert!(doc.can_undo(), "no-op clear pushed nothing");
}

/// `remove_node_tag` of an absent path is a no-op (no undo entry pushed).
#[test]
fn remove_absent_tag_is_no_op() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let node = NodeId::Object(id);

    // No tags yet — remove should be a no-op.
    doc.remove_node_tag(node, &["Ghost".to_string()]).unwrap();
    // Only the extrude is on the undo stack (remove pushed nothing).
    doc.undo().unwrap(); // undo extrude
    assert!(!doc.can_undo(), "only the extrude was on the undo stack");
}

/// `object_solid` returns `true` for a watertight box, `false` for stale ids.
#[test]
fn object_solid_reflects_watertight_state() {
    let mut doc = Document::new();
    let id = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    assert!(doc.object_solid(id), "extruded box is solid (watertight)");

    // Undo the creation → object is hidden (stale from the caller's POV).
    doc.undo().unwrap();
    assert!(
        !doc.object_solid(id),
        "hidden/undone object is not solid (hidden)"
    );
}

/// Tags and name survive on group and instance nodes as well.
#[test]
fn tag_and_name_ops_work_on_group_and_instance_nodes() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    doc.group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let gid = *doc.group_ids().first().unwrap();

    // Tag the group.
    let tag = vec!["Ground Floor".to_string()];
    doc.add_node_tag(NodeId::Group(gid), tag.clone()).unwrap();
    assert_eq!(
        doc.node_tags(NodeId::Group(gid)),
        std::slice::from_ref(&tag)
    );

    // Rename the group.
    doc.set_node_name(NodeId::Group(gid), Some("Floor1".to_string()))
        .unwrap();

    // Make a component instance and tag it.
    let c = extrude_box(&mut doc, 5.0, 0.0, 6.0, 1.0, 0.0, 1.0);
    let (comp, iid, _) = doc.make_component(&[NodeId::Object(c)]).unwrap();
    let inst_tag = vec!["Furniture".to_string()];
    doc.add_node_tag(NodeId::Instance(iid), inst_tag.clone())
        .unwrap();
    assert_eq!(
        doc.node_tags(NodeId::Instance(iid)),
        std::slice::from_ref(&inst_tag)
    );
    let _ = comp;

    // Undo the instance tag add → tag should be gone.
    doc.undo().unwrap();
    assert_eq!(doc.node_tags(NodeId::Instance(iid)), &[] as &[Vec<String>]);
}

// ------------------------------------------------------------------- guides

#[test]
fn add_guide_line_normalizes_direction_and_is_queryable() {
    let mut doc = Document::new();
    let id = doc
        .add_guide_line(Point3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 0.0, 0.0))
        .expect("add guide line");

    assert_eq!(doc.guide_ids(), vec![id]);
    match doc.guide(id).expect("guide is live") {
        Guide::Line { origin, direction } => {
            assert!(origin.approx_eq(Point3::new(1.0, 2.0, 3.0), 1e-12));
            // Stored direction is normalized, not the raw (2,0,0) input.
            assert!(direction.approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-12));
            assert!((direction.length() - 1.0).abs() < 1e-12);
        }
        Guide::Point { .. } => panic!("expected a Line guide"),
    }
}

#[test]
fn add_guide_point_is_queryable() {
    let mut doc = Document::new();
    let id = doc
        .add_guide_point(Point3::new(4.0, 5.0, 6.0))
        .expect("add guide point");

    assert_eq!(doc.guide_ids(), vec![id]);
    match doc.guide(id).expect("guide is live") {
        Guide::Point { position } => {
            assert!(position.approx_eq(Point3::new(4.0, 5.0, 6.0), 1e-12));
        }
        Guide::Line { .. } => panic!("expected a Point guide"),
    }
}

#[test]
fn add_guide_line_rejects_degenerate_direction() {
    let mut doc = Document::new();
    assert_eq!(
        doc.add_guide_line(Point3::ORIGIN, Vec3::ZERO),
        Err(DocumentError::DegenerateGuide)
    );
    assert!(doc.guide_ids().is_empty(), "document untouched on Err");
    assert!(!doc.can_undo(), "no undo entry pushed on Err");
}

#[test]
fn add_guide_line_rejects_non_finite_coordinates() {
    let mut doc = Document::new();
    let nan = f64::NAN;
    let inf = f64::INFINITY;

    // Non-finite origin, otherwise-valid direction.
    assert_eq!(
        doc.add_guide_line(Point3::new(nan, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        Err(DocumentError::DegenerateGuide)
    );
    // Non-finite direction.
    assert_eq!(
        doc.add_guide_line(Point3::ORIGIN, Vec3::new(inf, 0.0, 0.0)),
        Err(DocumentError::DegenerateGuide)
    );
    assert!(doc.guide_ids().is_empty());
}

#[test]
fn add_guide_point_rejects_non_finite_coordinates() {
    let mut doc = Document::new();
    assert_eq!(
        doc.add_guide_point(Point3::new(0.0, f64::NAN, 0.0)),
        Err(DocumentError::DegenerateGuide)
    );
    assert_eq!(
        doc.add_guide_point(Point3::new(0.0, 0.0, f64::INFINITY)),
        Err(DocumentError::DegenerateGuide)
    );
    assert!(doc.guide_ids().is_empty());
}

#[test]
fn add_guide_undo_redo_preserves_id() {
    let mut doc = Document::new();
    let id = doc
        .add_guide_line(Point3::ORIGIN, Vec3::new(0.0, 1.0, 0.0))
        .unwrap();
    assert_eq!(doc.guide_ids(), vec![id]);

    doc.undo().unwrap();
    assert!(doc.guide_ids().is_empty(), "undo hides the created guide");
    assert!(doc.guide(id).is_none(), "hidden guide is not queryable");

    doc.redo().unwrap();
    assert_eq!(
        doc.guide_ids(),
        vec![id],
        "redo unhides with the SAME GuideId"
    );
    assert!(doc.guide(id).is_some());
}

#[test]
fn delete_guide_undo_redo_round_trips() {
    let mut doc = Document::new();
    let id = doc.add_guide_point(Point3::new(1.0, 1.0, 1.0)).unwrap();

    let change = doc.delete_guide(id).expect("delete");
    assert_eq!(change.guides_touched, vec![id]);
    assert!(doc.guide_ids().is_empty(), "deleted guide is hidden");

    doc.undo().unwrap();
    assert_eq!(doc.guide_ids(), vec![id], "undo unhides it");

    doc.redo().unwrap();
    assert!(doc.guide_ids().is_empty(), "redo re-hides it");
}

#[test]
fn delete_guide_rejects_unknown_or_already_hidden() {
    let mut doc = Document::new();
    let id = doc.add_guide_point(Point3::ORIGIN).unwrap();
    doc.delete_guide(id).unwrap();
    // Already hidden — deleting again is refused.
    assert_eq!(doc.delete_guide(id), Err(DocumentError::UnknownGuide));

    // Stale handle from another document.
    let mut other = Document::new();
    let stray = other.add_guide_point(Point3::ORIGIN).unwrap();
    assert_eq!(doc.delete_guide(stray), Err(DocumentError::UnknownGuide));
}

#[test]
fn delete_sketch_undo_redo_round_trips() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    assert_eq!(doc.sketch_ids(), vec![s]);

    let change = doc.delete_sketch(s).expect("delete");
    assert_eq!(change.sketches_touched, vec![s]);
    assert!(doc.sketch_ids().is_empty(), "deleted sketch is hidden");
    assert!(doc.sketch(s).is_none(), "hidden sketch is not queryable");

    doc.undo().unwrap();
    assert_eq!(doc.sketch_ids(), vec![s], "undo unhides it, same SketchId");
    assert!(doc.sketch(s).is_some());

    doc.redo().unwrap();
    assert!(doc.sketch_ids().is_empty(), "redo re-hides it");
}

#[test]
fn delete_sketch_rejects_unknown_or_already_hidden() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.delete_sketch(s).unwrap();
    // Already hidden — deleting again is refused.
    assert_eq!(doc.delete_sketch(s), Err(DocumentError::UnknownSketch));

    // Stale handle from another document.
    let mut other = Document::new();
    let stray = other.add_sketch(ground());
    assert_eq!(doc.delete_sketch(stray), Err(DocumentError::UnknownSketch));
}

#[test]
fn delete_all_guides_is_one_undo_step_and_restores_all() {
    let mut doc = Document::new();
    let l = doc
        .add_guide_line(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    let p = doc.add_guide_point(Point3::new(1.0, 1.0, 1.0)).unwrap();
    assert_eq!(doc.guide_ids().len(), 2);

    let change = doc.delete_all_guides().expect("delete all");
    let mut touched = change.guides_touched.clone();
    touched.sort_by_key(|g| format!("{g:?}"));
    let mut expected = vec![l, p];
    expected.sort_by_key(|g| format!("{g:?}"));
    assert_eq!(touched, expected);
    assert!(doc.guide_ids().is_empty());

    // One undo step restores both.
    doc.undo().unwrap();
    let mut restored = doc.guide_ids();
    restored.sort_by_key(|g| format!("{g:?}"));
    assert_eq!(restored, expected, "undo restores exactly these guides");

    doc.redo().unwrap();
    assert!(doc.guide_ids().is_empty(), "redo re-hides them");
}

#[test]
fn delete_all_guides_on_empty_document_is_a_no_op() {
    let mut doc = Document::new();
    let change = doc.delete_all_guides().expect("no-op delete");
    assert_eq!(change, kernel::DocChange::default());
    assert!(!doc.can_undo(), "an empty delete-all pushes NO undo entry");
}

// ------------------------------------------------------------------ slice

#[test]
fn slice_node_splits_solid_and_round_trips() {
    let mut doc = Document::new();
    let src = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    assert_eq!(doc.visible_object_ids(), vec![src]);

    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let ((a, b), _) = doc.slice_node(src, &plane).expect("slice");

    let visible = doc.visible_object_ids();
    assert_eq!(visible.len(), 2, "two pieces visible");
    assert!(visible.contains(&a) && visible.contains(&b));
    assert!(doc.object(src).is_none(), "source hidden");
    for id in [a, b] {
        assert_eq!(
            doc.object(id).unwrap().watertight(),
            WatertightState::Watertight
        );
    }

    doc.undo().expect("undo slice");
    assert_eq!(doc.visible_object_ids(), vec![src], "source restored");
    assert!(
        doc.object(a).is_none() && doc.object(b).is_none(),
        "pieces hidden"
    );

    doc.redo().expect("redo slice");
    let v = doc.visible_object_ids();
    assert_eq!(v.len(), 2);
    assert!(v.contains(&a) && v.contains(&b), "stable piece ids");
}

#[test]
fn slice_node_missing_plane_refused_and_untouched() {
    use kernel::SliceError;
    let mut doc = Document::new();
    let src = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let outside =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    assert!(matches!(
        doc.slice_node(src, &outside),
        Err(DocumentError::Slice(SliceError::PlaneMissesSolid))
    ));
    assert_eq!(
        doc.visible_object_ids(),
        vec![src],
        "refused slice leaves the document untouched"
    );
}

/// Replacing world-context ops (boolean / slice / push-through) consume their
/// operand(s) and emit fresh top-level solids. Applied to a **grouped** leaf
/// they would hide the operand but leave the parent group still listing the
/// consumed id — a tree-consistency violation (`debug_validate_tree`) found by
/// the determinism guard. They must be refused loudly (rule 4) and leave
/// the document untouched, so the group is never left pointing at a stale id.
#[test]
fn replacing_ops_refuse_a_grouped_leaf_and_leave_the_document_untouched() {
    let mut doc = Document::new();
    // A 4×4×1 slab with a centred 1×1 sub-face imprinted (so push-through has a
    // real sub-face to drive), plus a second box to combine / group with.
    let a = extrude_box(&mut doc, 0.0, 0.0, 4.0, 4.0, 0.0, 1.0);
    let top = top_face(doc.object(a).unwrap());
    let sub = match doc
        .apply_object_op(
            a,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path: vec![
                    Point3::new(1.5, 1.5, 1.0),
                    Point3::new(2.5, 1.5, 1.0),
                    Point3::new(2.5, 2.5, 1.0),
                    Point3::new(1.5, 2.5, 1.0),
                ],
            },
        )
        .expect("imprint sub-face")
        .0
    {
        KernelOpReport::FaceSplitInner(r) => r.sub_face,
        other => panic!("unexpected report {other:?}"),
    };
    let b = extrude_box(&mut doc, 6.0, 6.0, 7.0, 7.0, 0.0, 1.0);

    // Group both leaves: now `a` and `b` are members of `g`, not top-level.
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("group the two boxes");
    assert_eq!(doc.node_parent(NodeId::Object(a)), Some(g));

    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, 1.0)).unwrap();

    // All three replacing ops refuse a grouped operand with GroupedOperand…
    assert_eq!(
        doc.boolean(BooleanOp::Union, a, b).map(|_| ()),
        Err(DocumentError::GroupedOperand),
        "boolean refuses a grouped operand"
    );
    assert_eq!(
        doc.slice_node(a, &plane).map(|_| ()),
        Err(DocumentError::GroupedOperand),
        "slice refuses a grouped source"
    );
    assert_eq!(
        doc.push_pull_through(a, sub, -2.0).map(|_| ()),
        Err(DocumentError::GroupedOperand),
        "push-through refuses a grouped source"
    );

    // …and each leaves the document untouched: both leaves still visible, the
    // group still lists exactly them (an invariant the debug validator that runs
    // after every mutation would have caught had any op partially applied).
    let visible = doc.visible_object_ids();
    assert_eq!(visible.len(), 2, "both leaves still visible");
    assert!(visible.contains(&a) && visible.contains(&b));
    assert_eq!(
        doc.group_members(g),
        Some(vec![NodeId::Object(a), NodeId::Object(b)]),
        "group membership intact — no stale/consumed id"
    );

    // The guard is precise: once ungrouped, the same slice succeeds.
    doc.ungroup(g).expect("ungroup");
    assert!(
        doc.slice_node(a, &plane).is_ok(),
        "slice works on the now-top-level object"
    );
}

// -------------------------------------------------- push-through subtract

#[test]
fn push_through_sub_face_punches_hole_and_round_trips() {
    let mut doc = Document::new();
    // 4×4×1 slab.
    let src = extrude_box(&mut doc, 0.0, 0.0, 4.0, 4.0, 0.0, 1.0);
    let top = top_face(doc.object(src).unwrap());

    // Imprint a centred 1×1 sub-face on top.
    let report = match doc
        .apply_object_op(
            src,
            KernelOp::SplitFaceInner {
                face: top,
                loop_path: vec![
                    Point3::new(1.5, 1.5, 1.0),
                    Point3::new(2.5, 1.5, 1.0),
                    Point3::new(2.5, 2.5, 1.0),
                    Point3::new(1.5, 2.5, 1.0),
                ],
            },
        )
        .expect("imprint sub-face")
        .0
    {
        KernelOpReport::FaceSplitInner(r) => r,
        other => panic!("unexpected report {other:?}"),
    };
    let sub = report.sub_face;
    assert!(doc.object(src).unwrap().push_pull_overshoots(sub, -2.0));

    let (results, _) = doc.push_pull_through(src, sub, -2.0).expect("through");
    assert_eq!(results.len(), 1, "a centred hole leaves one solid");
    let holed = results[0];
    assert!(doc.object(src).is_none(), "source consumed");
    assert_eq!(doc.visible_object_ids(), vec![holed]);
    assert_eq!(
        doc.object(holed).unwrap().watertight(),
        WatertightState::Watertight
    );

    // Undo restores exactly the imprinted source (still a valid solid); redo
    // re-punches.
    doc.undo().expect("undo through");
    assert_eq!(doc.visible_object_ids(), vec![src], "source restored");
    assert!(doc.object(holed).is_none());

    doc.redo().expect("redo through");
    assert_eq!(doc.visible_object_ids(), vec![holed]);
}

// -----------------------------------------------  duplicate_node (Move+copy)

/// Duplicating an Object deep-clones its geometry at the placement offset: the
/// copy is a distinct, independent world object; the source is untouched; and
/// the action is one handle-stable undo step.
#[test]
fn duplicate_object_clones_geometry_at_offset_and_round_trips() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let original = doc.object(a).unwrap().clone();
    let c0 = centroid(&original);
    let offset = Vec3::new(3.0, 0.0, 0.0);

    let (root, change) = doc
        .duplicate_node(NodeId::Object(a), &Transform::translation(offset))
        .expect("duplicate object");
    let b = match root {
        NodeId::Object(id) => id,
        _ => panic!("duplicating an object yields an object"),
    };

    assert_ne!(a, b, "the copy is a distinct handle");
    assert!(change.objects_touched.contains(&b));
    let visible = doc.visible_object_ids();
    assert!(visible.contains(&a) && visible.contains(&b), "both visible");
    assert!(
        doc.is_world_object(b),
        "the copy is an independent world object"
    );
    assert!(
        objects_equivalent(doc.object(a).unwrap(), &original),
        "the source is untouched"
    );
    assert!(
        approx_pt(centroid(doc.object(b).unwrap()), c0 + offset),
        "the copy is the source translated by the placement"
    );

    // Independence: editing the copy leaves the source put.
    doc.transform_object(b, &Transform::translation(Vec3::new(0.0, 0.0, 9.0)))
        .unwrap();
    assert!(
        approx_pt(centroid(doc.object(a).unwrap()), c0),
        "the source has its own geometry"
    );
    doc.undo().unwrap(); // undo the independence-probe transform

    // Undo the duplication: only the copy disappears; redo restores it.
    doc.undo().expect("undo duplicate");
    assert_eq!(
        doc.visible_object_ids(),
        vec![a],
        "undo removes the copy, keeps the source"
    );
    doc.redo().expect("redo duplicate");
    let visible = doc.visible_object_ids();
    assert!(visible.contains(&a) && visible.contains(&b));
    assert!(approx_pt(centroid(doc.object(b).unwrap()), c0 + offset));
}

/// Duplicating a Group clones the whole subtree into a new top-level group with
/// its own fresh leaves (none shared with the source); undo removes it wholesale.
#[test]
fn duplicate_group_clones_the_whole_subtree() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let orig_leaves: HashSet<ObjectId> = doc
        .leaf_objects_under(NodeId::Group(g))
        .into_iter()
        .collect();

    let (root, _) = doc
        .duplicate_node(
            NodeId::Group(g),
            &Transform::translation(Vec3::new(0.0, 5.0, 0.0)),
        )
        .unwrap();
    let g2 = match root {
        NodeId::Group(id) => id,
        _ => panic!("a group copy is a group"),
    };
    assert_ne!(g, g2);
    assert!(doc.top_level_nodes().contains(&NodeId::Group(g2)));

    let new_leaves = doc.leaf_objects_under(NodeId::Group(g2));
    assert_eq!(new_leaves.len(), orig_leaves.len(), "same leaf count");
    assert!(
        new_leaves.iter().all(|o| !orig_leaves.contains(o)),
        "the copy has its own distinct leaves"
    );

    doc.undo().unwrap();
    assert!(!doc.top_level_nodes().contains(&NodeId::Group(g2)));
    for o in &new_leaves {
        assert!(
            !doc.visible_object_ids().contains(o),
            "copied leaves are hidden on undo"
        );
    }
}

/// Duplicating an Instance places another instance of the **same definition**
/// (geometry stays shared — unlike make_unique) at `pose.then(placement)`.
#[test]
fn duplicate_instance_shares_the_definition_at_offset_pose() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let offset = Vec3::new(4.0, 0.0, 0.0);

    let (root, _) = doc
        .duplicate_node(NodeId::Instance(inst), &Transform::translation(offset))
        .unwrap();
    let inst2 = match root {
        NodeId::Instance(id) => id,
        _ => panic!("an instance copy is an instance"),
    };
    assert_ne!(inst, inst2);
    assert_eq!(
        doc.instance_def(inst2),
        Some(comp),
        "the copy shares the source's definition (not a fresh make_unique def)"
    );
    assert!(doc.instances_of(comp).contains(&inst) && doc.instances_of(comp).contains(&inst2));

    let probe = Point3::new(0.3, 0.7, 0.2);
    let expected = Transform::IDENTITY.then(&Transform::translation(offset));
    assert!(
        doc.instance_pose(inst2)
            .unwrap()
            .apply_point(probe)
            .approx_eq(expected.apply_point(probe), 1e-9),
        "the copy's pose is the source pose composed with the placement"
    );

    doc.undo().unwrap();
    assert!(
        !doc.top_level_nodes().contains(&NodeId::Instance(inst2)),
        "undo removes the copied instance"
    );
}

/// A singular or reflecting placement, and a stale source, are refused — the
/// document is left exactly as it was (a partial clone is rolled back).
#[test]
fn duplicate_refuses_bad_placement_and_stale_node() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);

    assert_eq!(
        doc.duplicate_node(NodeId::Object(a), &Transform::uniform_scale(0.0))
            .map(|_| ()),
        Err(DocumentError::Transform(TransformError::Singular))
    );
    assert_eq!(
        doc.duplicate_node(NodeId::Object(a), &Transform::uniform_scale(-1.0))
            .map(|_| ()),
        Err(DocumentError::Transform(TransformError::Reflection))
    );
    assert_eq!(
        doc.visible_object_ids(),
        vec![a],
        "a refused duplicate leaves the document untouched"
    );
    let bogus = ObjectId::default();
    assert_eq!(
        doc.duplicate_node(NodeId::Object(bogus), &Transform::IDENTITY)
            .map(|_| ()),
        Err(DocumentError::UnknownObject)
    );

    // No refused call left a stray action behind: the only thing on the undo
    // stack is the original extrude, so one undo empties the document.
    doc.undo().expect("undo the extrude");
    assert!(
        doc.visible_object_ids().is_empty(),
        "refused duplicates pushed no undo entry"
    );
    assert!(!doc.can_undo());
}

// ----------------------------------------------------------- torture mode

#[test]
fn torture_mode_toggles_and_passes_a_real_op_sequence() {
    let mut doc = Document::new();
    assert!(!doc.torture_mode(), "torture mode is off by default");
    doc.set_torture_mode(true);
    assert!(doc.torture_mode());

    // Under torture mode the always-on topology validator runs after *every*
    // op (release included). A representative build → boolean → transform →
    // duplicate → delete → slice sequence must complete cleanly: each op's
    // result passes the validator, so none of these `expect`s — nor the post-op
    // torture validation — panics. (Slice goes last because it consumes `u`.)
    let a = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let b = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 0.0, 3.0);
    let (u, _) = doc
        .boolean(BooleanOp::Union, a, b)
        .expect("union under torture");
    doc.transform_object(u, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .expect("translate under torture");
    let dup = doc
        .duplicate_node(
            NodeId::Object(u),
            &Transform::translation(Vec3::new(6.0, 0.0, 0.0)),
        )
        .expect("duplicate under torture")
        .0;
    doc.delete_node(dup).expect("delete under torture");
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("slice plane");
    let _ = doc.slice_node(u, &plane); // exercises the slice path; tolerate refusal

    doc.set_torture_mode(false);
    assert!(!doc.torture_mode());
}

// ------------------------------------------------- sketch drawing gestures
//
// One drawing gesture (a whole rectangle/circle, or one committed Line
// segment) = one undo step. The first gesture on a freshly-added sketch folds
// the sketch's creation into that step, so undoing it removes the sketch from
// view entirely — no empty ghost in the sketch list.

#[test]
fn sketch_gesture_groups_a_rectangle_into_one_undo_step() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());

    doc.begin_sketch_gesture(s).expect("begin gesture");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end gesture");

    assert!(doc.can_undo(), "the gesture is one undoable step");
    let region = only_region(&doc, s);

    // ONE undo removes the whole rectangle — and, because this gesture drew
    // the first geometry into a fresh sketch, the sketch itself vanishes.
    doc.undo().expect("undo the gesture");
    assert!(
        doc.sketch(s).is_none(),
        "created-by-gesture sketch is hidden"
    );
    assert!(doc.sketch_ids().is_empty(), "no empty ghost in the list");
    assert!(!doc.can_undo(), "nothing left to undo");

    // Redo brings back the sketch, its rectangle, and the same region handle.
    doc.redo().expect("redo the gesture");
    let sk = doc.sketch(s).expect("sketch is visible again");
    assert_eq!(sk.edges().len(), 4);
    assert_eq!(only_region(&doc, s), region, "region handle is stable");
}

#[test]
fn second_gesture_on_same_sketch_undoes_independently() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());

    doc.begin_sketch_gesture(s).expect("begin first");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end first");

    doc.begin_sketch_gesture(s).expect("begin second");
    draw_rect(&mut doc, s, 2.0, 0.0, 3.0, 1.0);
    doc.end_sketch_gesture(s).expect("end second");
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 8);

    // Undo #1 removes only the second rectangle; the sketch stays visible.
    doc.undo().expect("undo second gesture");
    let sk = doc.sketch(s).expect("sketch still visible");
    assert_eq!(sk.edges().len(), 4, "first rectangle survives");

    // Undo #2 removes the first rectangle AND the sketch (creation folded in).
    doc.undo().expect("undo first gesture");
    assert!(doc.sketch(s).is_none());

    // Redo both restores everything.
    doc.redo().expect("redo first");
    doc.redo().expect("redo second");
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 8);
}

#[test]
fn unchanged_gesture_records_nothing() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("begin");
    doc.end_sketch_gesture(s).expect("end");
    assert!(!doc.can_undo(), "an empty gesture is undo-invisible");

    // ... and the NEXT gesture still counts as the creating one.
    doc.begin_sketch_gesture(s).expect("begin again");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end again");
    doc.undo().expect("undo");
    assert!(
        doc.sketch(s).is_none(),
        "creation folded into the real gesture"
    );
}

#[test]
fn gesture_undo_interleaved_with_extrude_keeps_handles_stable() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("begin");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end");

    let r = only_region(&doc, s);
    let (obj, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    assert!(doc.visible_object_ids().contains(&obj));

    // LIFO: undo the extrude first, then the drawing gesture.
    doc.undo().expect("undo extrude");
    assert!(!doc.visible_object_ids().contains(&obj));
    assert_eq!(only_region(&doc, s), r, "region extrudable again");

    doc.undo().expect("undo gesture");
    assert!(doc.sketch(s).is_none());

    // Redo the full history: sketch, then solid.
    doc.redo().expect("redo gesture");
    doc.redo().expect("redo extrude");
    assert!(doc.visible_object_ids().contains(&obj));
    assert!(
        doc.is_region_consumed(s, r),
        "redone extrude re-consumed the same region handle"
    );
}

#[test]
fn gesture_bracket_misuse_errors_loudly() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());

    // end without begin
    assert!(matches!(
        doc.end_sketch_gesture(s),
        Err(DocumentError::SketchGestureNotOpen)
    ));

    // begin while open
    doc.begin_sketch_gesture(s).expect("begin");
    assert!(matches!(
        doc.begin_sketch_gesture(s),
        Err(DocumentError::SketchGestureAlreadyOpen)
    ));

    // end for a different sketch than the open one
    let other = doc.add_sketch(ground());
    assert!(matches!(
        doc.end_sketch_gesture(other),
        Err(DocumentError::SketchGestureNotOpen)
    ));

    // cancel drops the bracket without recording
    assert!(doc.cancel_sketch_gesture());
    assert!(!doc.cancel_sketch_gesture(), "nothing left to cancel");
    assert!(matches!(
        doc.end_sketch_gesture(s),
        Err(DocumentError::SketchGestureNotOpen)
    ));
    assert!(!doc.can_undo());

    // begin on a deleted (hidden) sketch
    doc.delete_sketch(s).expect("delete");
    assert!(matches!(
        doc.begin_sketch_gesture(s),
        Err(DocumentError::UnknownSketch)
    ));
}
