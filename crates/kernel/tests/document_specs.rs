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
    ImportNode, ImportScene, KernelOp, KernelOpReport, Material, MaterialId, MeshRecipe, NodeId,
    Object, ObjectId, Operand, Plane, Point3, Rgba8, SketchEdgeId, SketchError, SketchId,
    SketchRegionId, SketchVertexId, Texture, Transform, TransformError, Vec3, WatertightState,
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

/// Extruding a region consumes exactly that region — its scaffolding is
/// DELETED from the sketch (Model D: the outline became the solid's base
/// face) — while a sibling region of the same sketch keeps its edges and
/// stays extrudable.
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

    let sk = doc.sketch(s).expect("the sketch still has live content");
    assert!(
        !sk.regions().contains_key(first),
        "the extruded region's scaffolding is gone — the region with it"
    );
    assert!(
        sk.regions().contains_key(second),
        "the sibling is untouched"
    );
    assert_eq!(sk.edges().len(), 4, "exactly the sibling's edges remain");
    assert_eq!(
        doc.extrudable_regions(s).expect("live"),
        vec![second],
        "only the sibling remains extrudable"
    );
}

// ------------------------------------------------------------- undo / redo

/// Undoing a creation hides the Object AND restores the deleted sketch
/// scaffolding in one step; redo reverses both. The `ObjectId` and
/// `SketchId` are stable across the cycle; the restored scaffolding
/// carries fresh edge/region handles (re-insertion, not a snapshot — so
/// interleaved edits survive), and callers re-query as after any
/// reshaping mutation.
#[test]
fn undo_creation_hides_object_and_restores_region() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);

    let (id, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    assert_eq!(doc.visible_object_ids(), vec![id]);
    // The whole sketch became the solid: it ceased to exist.
    assert!(doc.sketch(s).is_none(), "emptied sketch is gone");

    doc.undo().expect("undo creation");
    assert!(doc.visible_object_ids().is_empty(), "creation hidden");
    let sk = doc.sketch(s).expect("sketch restored");
    assert_eq!(sk.edges().len(), 4, "the outline is back");
    let restored = only_region(&doc, s);
    assert_eq!(
        doc.sketch(s).expect("live").region_area(restored).unwrap(),
        1.0,
        "the restored region has the outline's exact geometry"
    );

    doc.redo().expect("redo creation");
    assert_eq!(
        doc.visible_object_ids(),
        vec![id],
        "same ObjectId after redo"
    );
    assert!(doc.sketch(s).is_none(), "sketch consumed again after redo");
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

/// Make Component inherits its display identity from a single-node
/// selection: the source's name becomes the definition name (the shared
/// label of every instance) and its tags copy onto the new instance (tags
/// attach to placements, never definitions). The source keeps both, so undo
/// restores it exactly.
#[test]
fn make_component_inherits_name_and_tags_from_a_single_source() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let tag = vec!["Objects".to_string(), "Boxes".to_string()];
    doc.add_node_tag(NodeId::Object(o), tag.clone()).unwrap();

    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    assert_eq!(doc.component_name(comp), Some("The Box"));
    assert_eq!(
        doc.instance_name(inst),
        None,
        "the inherited name is the definition's, not a per-instance override"
    );
    assert_eq!(
        doc.node_tags(NodeId::Instance(inst)),
        std::slice::from_ref(&tag)
    );

    // A second instance shares the same definition name automatically.
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .unwrap();
    assert_eq!(doc.instance_name(i2), None);
    assert_eq!(doc.instance_def(i2), Some(comp));

    // Undo the whole act: the source object is a world solid again with its
    // name and tags exactly as they were.
    doc.undo().unwrap(); // undo place_instance
    doc.undo().unwrap(); // undo make_component
    assert_eq!(doc.object_name(o), Some("The Box"));
    assert_eq!(doc.node_tags(NodeId::Object(o)), std::slice::from_ref(&tag));

    // Redo re-forms the component with the same identity (stable handles).
    doc.redo().unwrap();
    assert_eq!(doc.component_name(comp), Some("The Box"));
    assert_eq!(
        doc.node_tags(NodeId::Instance(inst)),
        std::slice::from_ref(&tag)
    );
}

/// A selection with no name to inherit — an unnamed node, or several
/// siblings — gets a generated definition name (`"Component N"`, lowest free
/// number over live definitions), so a definition always has a name and all
/// of its instances read identically.
#[test]
fn make_component_generates_a_definition_name_when_nothing_to_inherit() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (c1, _i1, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    assert_eq!(doc.component_name(c1), Some("Component 1"));

    // A multi-node selection has no single source: generated name, no tags.
    let b = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 5.0, 0.0, 6.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(b), Some("B".to_string()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), vec!["T".to_string()])
        .unwrap();
    let (c2, i2, _) = doc
        .make_component(&[NodeId::Object(b), NodeId::Object(c)])
        .unwrap();
    assert_eq!(doc.component_name(c2), Some("Component 2"));
    assert_eq!(doc.node_tags(NodeId::Instance(i2)), &[] as &[Vec<String>]);
}

/// Explode keeps each member's own name and tags on the baked world object —
/// the identity that rode into the definition at make_component rides back
/// out, instead of degrading to an anonymous positional label.
#[test]
fn explode_carries_the_member_name_and_tags_onto_the_world_object() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let tag = vec!["Objects".to_string(), "Boxes".to_string()];
    doc.add_node_tag(NodeId::Object(o), tag.clone()).unwrap();
    let (_comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();

    let (created, _) = doc.explode_instance(inst).unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(doc.object_name(created[0]), Some("The Box"));
    assert_eq!(
        doc.node_tags(NodeId::Object(created[0])),
        std::slice::from_ref(&tag)
    );
}

/// Explode prefers the instance's own name for a single-member definition —
/// the identity the user set on the placement survives the bake — while a
/// multi-member definition keeps each member's own name (stamping one
/// instance name onto several objects would mint duplicates). Undo restores
/// the instance with its name intact.
#[test]
fn explode_prefers_the_instance_name_on_a_single_member_definition() {
    let mut doc = Document::new();

    // Single member: the instance's set name wins over the member name.
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let (_comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.set_node_name(NodeId::Instance(inst), Some("Special".to_string()))
        .unwrap();
    let (created, _) = doc.explode_instance(inst).unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(doc.object_name(created[0]), Some("Special"));

    // Undo restores the instance, name intact.
    doc.undo().unwrap();
    assert_eq!(doc.instance_name(inst), Some("Special"));

    // Multi-member: each member keeps its own name; the instance name is
    // not stamped onto any of them.
    let a = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 5.0, 0.0, 6.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(a), Some("A".to_string()))
        .unwrap();
    doc.set_node_name(NodeId::Object(b), Some("B".to_string()))
        .unwrap();
    let (_c2, i2, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.set_node_name(NodeId::Instance(i2), Some("Combo".to_string()))
        .unwrap();
    let (created2, _) = doc.explode_instance(i2).unwrap();
    let names: Vec<Option<&str>> = created2.iter().map(|&c| doc.object_name(c)).collect();
    assert!(
        names.contains(&Some("A")) && names.contains(&Some("B")),
        "{names:?}"
    );
    assert!(!names.contains(&Some("Combo")), "{names:?}");
}

/// A single-member explode bakes the name the UI displays for the instance —
/// instance name, else the LIVE definition name, else the member's pre-fold
/// name. In particular a definition renamed after creation (set_component_name
/// touches only the definition, never the member record) must bake its
/// current name, not the member's stale one.
#[test]
fn explode_bakes_the_displayed_name_after_a_definition_rename() {
    let mut doc = Document::new();

    // Definition renamed after creation: the live definition name wins over
    // the member's pre-fold name.
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.set_component_name(comp, Some("Cabinet".to_string()))
        .unwrap();
    let (created, _) = doc.explode_instance(inst).unwrap();
    assert_eq!(doc.object_name(created[0]), Some("Cabinet"));

    // An unnamed member under a generated definition name bakes that name
    // (what every row displays), not a bare positional label.
    let u = extrude_box(&mut doc, 3.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    let (c2, i2, _) = doc.make_component(&[NodeId::Object(u)]).unwrap();
    let generated = doc.component_name(c2).expect("generated name").to_string();
    let (created2, _) = doc.explode_instance(i2).unwrap();
    assert_eq!(doc.object_name(created2[0]), Some(generated.as_str()));

    // The instance's own name still outranks the definition name.
    let p = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 0.0, 1.0);
    let (c3, i3, _) = doc.make_component(&[NodeId::Object(p)]).unwrap();
    doc.set_component_name(c3, Some("Cupboard".to_string()))
        .unwrap();
    doc.set_node_name(NodeId::Instance(i3), Some("Special".to_string()))
        .unwrap();
    let (created3, _) = doc.explode_instance(i3).unwrap();
    assert_eq!(doc.object_name(created3[0]), Some("Special"));
}

/// Make Unique names the new definition: a set instance name is promoted to
/// the definition name (and cleared off the instance); an unnamed instance
/// derives `"<def> Copy"`, disambiguated `"<def> Copy 2"`, … against live
/// definitions. Undo restores the promoted instance name exactly.
#[test]
fn make_unique_names_the_new_definition() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let (comp, _i1, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .unwrap();
    let (i3, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(6.0, 0.0, 0.0)))
        .unwrap();

    // Unnamed instance → "<def> Copy".
    let (u2, _) = doc.make_unique(i2).unwrap();
    assert_eq!(doc.component_name(u2), Some("The Box Copy"));

    // The next unnamed unique of the same def disambiguates.
    let (u3, _) = doc.make_unique(i3).unwrap();
    assert_eq!(doc.component_name(u3), Some("The Box Copy 2"));

    // A named instance promotes its name to the new definition and clears
    // its own — the row now reads as the new component, not "Name (Name)".
    let (i4, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(9.0, 0.0, 0.0)))
        .unwrap();
    doc.set_node_name(NodeId::Instance(i4), Some("Special".to_string()))
        .unwrap();
    let (u4, _) = doc.make_unique(i4).unwrap();
    assert_eq!(doc.component_name(u4), Some("Special"));
    assert_eq!(doc.instance_name(i4), None);

    // Undo restores the shared def AND the instance's own name; redo
    // re-promotes.
    doc.undo().unwrap();
    assert_eq!(doc.instance_def(i4), Some(comp));
    assert_eq!(doc.instance_name(i4), Some("Special"));
    doc.redo().unwrap();
    assert_eq!(doc.instance_def(i4), Some(u4));
    assert_eq!(doc.instance_name(i4), None);
    assert_eq!(doc.component_name(u4), Some("Special"));
}

/// `set_component_name` renames the shared definition label (undoable), is a
/// no-op on the current name (no undo entry), and touches every instance in
/// its change so the UI refreshes each row.
#[test]
fn set_component_name_is_undoable_and_touches_every_instance() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (comp, i1, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    let (i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(3.0, 0.0, 0.0)))
        .unwrap();

    let change = doc
        .set_component_name(comp, Some("Widget".to_string()))
        .unwrap();
    assert_eq!(doc.component_name(comp), Some("Widget"));
    assert!(change.components_touched.contains(&comp));
    assert!(
        change.instances_touched.contains(&i1) && change.instances_touched.contains(&i2),
        "a definition rename touches every instance"
    );

    // Renaming to the current name pushes no undo entry: the next undo steps
    // over it straight back to the real rename.
    doc.set_component_name(comp, Some("Widget".to_string()))
        .unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.component_name(comp), Some("Component 1"));
    doc.redo().unwrap();
    assert_eq!(doc.component_name(comp), Some("Widget"));

    // A stale handle fails typed.
    doc.undo().unwrap(); // undo rename
    doc.undo().unwrap(); // undo place_instance
    doc.undo().unwrap(); // undo make_component — the def is now hidden
    assert_eq!(
        doc.set_component_name(comp, Some("X".to_string())),
        Err(DocumentError::UnknownComponent)
    );
}

/// Component display identity — the definition name and the instance's tags
/// and own name — survives a save/load round trip.
#[test]
fn component_identity_round_trips_through_save_load() {
    let mut doc = Document::new();
    let o = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(o), Some("The Box".to_string()))
        .unwrap();
    let tag = vec!["Objects".to_string(), "Boxes".to_string()];
    doc.add_node_tag(NodeId::Object(o), tag.clone()).unwrap();
    let (_comp, inst, _) = doc.make_component(&[NodeId::Object(o)]).unwrap();
    doc.set_node_name(NodeId::Instance(inst), Some("First".to_string()))
        .unwrap();

    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("round trip loads");
    let comps = loaded.component_ids();
    assert_eq!(comps.len(), 1);
    assert_eq!(loaded.component_name(comps[0]), Some("The Box"));
    let insts = loaded.instance_ids();
    assert_eq!(insts.len(), 1);
    assert_eq!(loaded.instance_name(insts[0]), Some("First"));
    assert_eq!(
        loaded.node_tags(NodeId::Instance(insts[0])),
        std::slice::from_ref(&tag)
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
                restore: None,
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

// -------------------------------- extrusion deletes the scaffolding

/// After extruding the sole rectangle on a ground sketch, its 4 boundary
/// edges are DELETED (Model D: the outline became the solid's base face)
/// and the emptied sketch itself ceases to exist. Undo restores sketch and
/// outline while hiding the solid; redo reverses; the emptied sketch never
/// reaches a saved file.
#[test]
fn extrusion_deletes_the_scaffolding_and_the_emptied_sketch() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);

    assert_eq!(doc.sketch(s).expect("live").edges().len(), 4);
    assert!(doc.sketch_ids().contains(&s));

    doc.extrude_region(s, r, 1.0).expect("extrude");

    // Nothing hidden survives: the geometry is gone and so is the sketch.
    assert!(
        doc.sketch(s).is_none(),
        "the emptied sketch ceased to exist"
    );
    assert!(!doc.sketch_ids().contains(&s));

    // Undo: outline and sketch return; the solid hides.
    doc.undo().expect("undo");
    assert_eq!(doc.sketch(s).expect("restored").edges().len(), 4);
    assert!(doc.sketch_ids().contains(&s));
    assert!(doc.visible_object_ids().is_empty());

    // Redo: consumed again.
    doc.redo().expect("redo");
    assert!(doc.sketch(s).is_none());

    // Save → load: only the solid exists; no sketch and no stored claims.
    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    assert!(doc2.sketch_ids().is_empty(), "nothing hidden persists");
    assert_eq!(doc2.visible_object_ids().len(), 1);
}

/// Two regions sharing an edge (two rectangles sharing a wall): the shared
/// edge survives the FIRST extrude — it still bounds the live neighbor
/// region, which must stay closed — and dies with the SECOND, so no orphan
/// sketch line outlives the regions it bounded. Undo walks back one step,
/// and the fully-consumed sketch stays gone across save → load.
#[test]
fn shared_edge_deleted_with_last_region_not_first() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 1.0, 0.0, 2.0, 1.0); // shares the x=1 wall

    let regions: Vec<SketchRegionId> = doc.sketch(s).expect("live").regions().keys().collect();
    assert_eq!(regions.len(), 2);
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 7);

    // First extrude: only the left region's exclusive 3 edges go; the shared
    // wall stays because the right region still needs it.
    doc.extrude_region(s, regions[0], 1.0)
        .expect("extrude left");
    {
        let sk = doc.sketch(s).expect("live");
        assert_eq!(
            sk.edges().len(),
            4,
            "the shared wall and the live neighbor's edges must survive"
        );
        assert!(
            sk.regions().contains_key(regions[1]),
            "the neighbor region stays closed, same handle"
        );
    }

    // Second extrude: the shared wall no longer bounds anything live — the
    // sketch empties and ceases to exist.
    doc.extrude_region(s, regions[1], 1.0)
        .expect("extrude right");
    assert!(doc.sketch(s).is_none(), "the emptied sketch is gone");

    // Undo the second extrude: exactly its increment comes back.
    doc.undo().expect("undo");
    assert_eq!(
        doc.sketch(s).expect("restored").edges().len(),
        4,
        "undoing the second extrude restores the shared wall and neighbors"
    );

    // Redo, then round-trip.
    doc.redo().expect("redo");
    let bytes = doc.save();
    let doc2 = Document::load(&bytes).expect("round-trip");
    assert!(
        doc2.sketch_ids().is_empty(),
        "the consumed sketch must not reappear after save/load"
    );
    assert_eq!(
        doc2.visible_object_ids().len(),
        2,
        "both extruded solids survive the round-trip"
    );
}

/// A region with a leftover interior whisker extrudes: the spur is not
/// boundary, so the profile is clean, and after the extrude the spur —
/// an open chain — survives the scaffolding deletion and stays deletable
/// like any other line.
#[test]
fn region_with_interior_spur_extrudes_and_spur_survives() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    // Leftover interior line off the (1,1) corner — the stray-facet case.
    {
        let sk = doc.sketch_mut(s).expect("live");
        sk.add_segment(Point3::new(1.0, 1.0, 0.0), Point3::new(0.5, 0.5, 0.0))
            .expect("whisker");
    }

    let r = only_region(&doc, s);
    doc.extrude_region(s, r, 1.0)
        .expect("spur must not block the extrude");

    // The whisker is all that remains, still an ordinary deletable line.
    let sk = doc.sketch(s).expect("the sketch keeps its open chain");
    assert_eq!(sk.edges().len(), 1, "exactly the whisker remains");
    let whisker = sk.edges().keys().next().expect("whisker edge");
    doc.sketch_mut(s)
        .expect("live")
        .remove_edge(whisker)
        .expect("the leftover line deletes like any other");
}

/// Moving one island of a two-shape sketch is undoable and leaves the other
/// island untouched; landing on the neighbor refuses with a typed error and
/// records nothing.
#[test]
fn island_transform_is_undoable_and_scoped() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    draw_rect(&mut doc, s, 3.0, 0.0, 4.0, 1.0);

    let sk = doc.sketch(s).expect("live");
    assert_eq!(sk.islands().len(), 2);
    let left = sk
        .islands()
        .iter()
        .find(|(_, isl)| {
            let e = sk.edges()[isl.edges[0]];
            sk.vertices()[e.from].position.x < 2.0
        })
        .map(|(id, _)| id)
        .expect("left island");

    let up = Transform::translation(Vec3::new(0.0, 5.0, 0.0));
    doc.transform_sketch_island(s, left, &up).expect("move");
    let max_y = |doc: &Document, island| {
        let sk = doc.sketch(s).unwrap();
        sk.islands()[island]
            .edges
            .iter()
            .map(|&e| sk.vertices()[sk.edges()[e].from].position.y)
            .fold(f64::NEG_INFINITY, f64::max)
    };
    assert!(max_y(&doc, left) >= 5.0);

    doc.undo().expect("undo");
    assert!(max_y(&doc, left) <= 1.0 + 1e-9, "undo moved it back");
    doc.redo().expect("redo");
    assert!(max_y(&doc, left) >= 5.0, "redo re-applied");

    // Refusal records nothing: undo after a refused move undoes the redo.
    let onto = Transform::translation(Vec3::new(3.0, -5.0, 0.0));
    assert!(doc.transform_sketch_island(s, left, &onto).is_err());
    doc.undo().expect("undo the redo, not the refusal");
    assert!(max_y(&doc, left) <= 1.0 + 1e-9);
}

/// A curve bracket never outlives its gesture: ending (or cancelling) the
/// gesture force-closes it, so a tool that aborted mid-commit cannot leave
/// the sketch silently tagging later, unrelated edges into a dead curve.
#[test]
fn gesture_end_force_closes_an_open_curve_bracket() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());

    doc.begin_sketch_gesture(s).expect("gesture");
    let curve = doc.sketch_mut(s).unwrap().begin_curve();
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    // Tool dies here — no end_curve. The gesture close must clean up.
    doc.end_sketch_gesture(s).expect("end");

    doc.begin_sketch_gesture(s).expect("gesture 2");
    doc.sketch_mut(s)
        .unwrap()
        .add_segment(Point3::new(0.0, 2.0, 0.0), Point3::new(1.0, 2.0, 0.0))
        .unwrap();
    doc.end_sketch_gesture(s).expect("end 2");

    let sk = doc.sketch(s).unwrap();
    assert_eq!(
        sk.curve_edges(curve).len(),
        1,
        "only the bracketed edge is in the curve; the later line is plain"
    );
}

/// Re-extruding a consumed region is impossible by construction: the
/// region ceased to exist with its scaffolding (a stale handle, a typed
/// error — here the emptied sketch itself is gone). Undoing the extrude
/// re-inserts the scaffolding; the re-formed region extrudes again (a
/// fresh handle — re-insertion, not a snapshot).
#[test]
fn re_extruding_a_consumed_region_is_refused() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    let r = only_region(&doc, s);
    doc.extrude_region(s, r, 1.0).expect("first extrude");
    assert!(matches!(
        doc.extrude_region(s, r, 2.0).unwrap_err(),
        DocumentError::UnknownSketch
    ));
    // Undoing the extrude restores sketch and outline; re-query the
    // re-formed region and it extrudes again.
    doc.undo().expect("undo");
    let restored = only_region(&doc, s);
    doc.extrude_region(s, restored, 2.0)
        .expect("extrudable after undo");
}

/// The interior of an extruded hole stays live: extruding a holed region
/// deletes only the outer scaffolding — the hole's boundary still bounds
/// the surviving inner region, which remains extrudable (it lies in the
/// solid's hole, not under its material) and deletable like any geometry.
#[test]
fn hole_interior_survives_and_stays_editable() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 4.0, 4.0);
    draw_rect(&mut doc, s, 1.0, 1.0, 2.0, 2.0); // the "circle"

    let (outer, inner_region) = {
        let sk = doc.sketch(s).expect("live");
        let outer = sk
            .regions()
            .iter()
            .find(|(_, r)| !r.holes.is_empty())
            .map(|(id, _)| id)
            .expect("holed outer region");
        let inner = sk
            .regions()
            .keys()
            .find(|&id| id != outer)
            .expect("inner region");
        (outer, inner)
    };
    doc.extrude_region(s, outer, 1.0).expect("extrude");

    // The inner shape survives whole — its region included — and is still
    // extrudable: it sits in the solid's hole, not under its material.
    {
        let sk = doc.sketch(s).expect("live");
        assert_eq!(sk.edges().len(), 4, "only the inner shape remains");
        assert!(sk.regions().contains_key(inner_region));
    }
    assert_eq!(
        doc.extrudable_regions(s).expect("live"),
        vec![inner_region],
        "the hole interior is free ground"
    );

    // And its edges delete like any other line (one gesture, no refusal).
    let inner_edges: Vec<SketchEdgeId> = doc.sketch(s).expect("live").edges().keys().collect();
    doc.begin_sketch_gesture(s).expect("gesture");
    for &e in &inner_edges {
        doc.sketch_mut(s)
            .unwrap()
            .remove_edge(e)
            .expect("hole scaffolding deletes like any line");
    }
    doc.end_sketch_gesture(s).expect("end");
    assert!(doc.sketch(s).expect("live").edges().is_empty());

    // Undo restores the inner shape.
    doc.undo().expect("undo");
    assert_eq!(doc.sketch(s).expect("live").edges().len(), 4);
}

/// Sketch edits that bypass the gesture bracket entirely (direct
/// `sketch_mut` mutation — a scripting path) produce ordinary extrudable
/// regions: redrawing a standing solid's base extrudes into a coincident
/// second solid, like every other overlap (the standing-solid gate was
/// dropped — the sketch-solid-model design).
#[test]
fn unbracketed_redraw_over_a_solid_extrudes() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    draw_rect(&mut doc, s, 0.0, 0.0, 2.0, 2.0);
    let left = only_region(&doc, s);
    let (_obj, _) = doc.extrude_region(s, left, 1.0).expect("extrude left");

    // No gesture bracket: redraw the standing solid's base directly.
    let s2 = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(s2).expect("live");
        for (a, b) in [
            (Point3::new(0.0, 0.0, 0.0), Point3::new(2.0, 0.0, 0.0)),
            (Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 2.0, 0.0)),
            (Point3::new(2.0, 2.0, 0.0), Point3::new(0.0, 2.0, 0.0)),
            (Point3::new(0.0, 2.0, 0.0), Point3::new(0.0, 0.0, 0.0)),
        ] {
            sk.add_segment(a, b).expect("segment");
        }
    }
    let redrawn = doc
        .sketch(s2)
        .expect("live")
        .regions()
        .keys()
        .next()
        .expect("the redrawn base closes a region");
    doc.extrude_region(s2, redrawn, 1.0)
        .expect("a coincident redraw extrudes — interpenetration is allowed");
}

// ──────────────────────────────── boolean coplanar-seam cleanup ─────────────

/// A document-level union of two flush boxes dissolves the coplanar seams:
/// the result reads as one canonical box (6 faces), and undo still restores
/// both operands untouched (the cleanup runs before the result is inserted,
/// so the undo record is the ordinary boolean one).
#[test]
fn boolean_union_dissolves_seams_and_undoes_cleanly() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0);

    let (result, _) = doc.boolean(BooleanOp::Union, a, b).expect("union");
    assert_eq!(
        doc.object(result).expect("result live").faces().len(),
        6,
        "flush-union seams must dissolve to the canonical box"
    );

    doc.undo().expect("undo");
    assert_eq!(doc.visible_object_ids().len(), 2, "operands restored");
    assert_eq!(doc.object(a).expect("a live").faces().len(), 6);
    assert_eq!(doc.object(b).expect("b live").faces().len(), 6);
}

/// Differing face materials are a hard stop for the seam cleanup: painting
/// one operand's top face keeps the top seam (a painted face never bleeds
/// into its neighbor), while the unpainted faces still merge.
#[test]
fn boolean_union_keeps_seams_between_differently_painted_faces() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0);

    // Paint a's top face red.
    let red = doc.add_material(Material {
        name: "red".to_string(),
        color: Rgba8::rgb(200, 30, 30),
        texture: None,
    });
    let a_top = {
        let obj = doc.object(a).expect("a live");
        obj.faces()
            .iter()
            .find(|(_, f)| f.plane.normal().z > 0.9)
            .map(|(id, _)| id)
            .expect("a has a top face")
    };
    doc.paint_face(a, a_top, Some(red)).expect("paint");

    let (result, _) = doc.boolean(BooleanOp::Union, a, b).expect("union");
    let obj = doc.object(result).expect("result live");
    // Top stays split (painted vs unpainted); the other four seam pairs
    // merged: 2 top + 1 bottom + 1 north + 1 south + 1 east + 1 west = 7.
    assert_eq!(obj.faces().len(), 7);
    let top_materials: Vec<_> = obj
        .faces()
        .values()
        .filter(|f| f.plane.normal().z > 0.9)
        .map(|f| f.material)
        .collect();
    assert_eq!(top_materials.len(), 2);
    assert!(top_materials.contains(&Some(red)));
    assert!(top_materials.contains(&None));
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
                restore: None,
                curve: None,
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
                restore: None,
                curve: None,
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
    // A second box moved into overlap with `a` (interpenetration is allowed
    // everywhere in Hew), giving the union a real coplanar-seam job.
    let b = extrude_box(&mut doc, 4.0, 1.0, 6.0, 3.0, 0.0, 3.0);
    doc.transform_object(b, &Transform::translation(Vec3::new(-3.0, 0.0, 0.0)))
        .expect("move b into overlap");
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
fn gesture_undo_interleaved_with_extrude_keeps_entity_handles_stable() {
    let mut doc = Document::new();
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("begin");
    draw_rect(&mut doc, s, 0.0, 0.0, 1.0, 1.0);
    doc.end_sketch_gesture(s).expect("end");

    let r = only_region(&doc, s);
    let (obj, _) = doc.extrude_region(s, r, 1.0).expect("extrude");
    assert!(doc.visible_object_ids().contains(&obj));

    // LIFO: undo the extrude first, then the drawing gesture. The object
    // and sketch handles stay stable; the restored outline's edge/region
    // handles are fresh (re-insertion), so re-query.
    doc.undo().expect("undo extrude");
    assert!(!doc.visible_object_ids().contains(&obj));
    assert_eq!(
        doc.sketch(s).expect("live").edges().len(),
        4,
        "outline restored, extrudable again"
    );
    assert_eq!(doc.extrudable_regions(s).expect("live").len(), 1);

    doc.undo().expect("undo gesture");
    assert!(doc.sketch(s).is_none());

    // Redo the full history: sketch, then solid.
    doc.redo().expect("redo gesture");
    doc.redo().expect("redo extrude");
    assert!(doc.visible_object_ids().contains(&obj));
    assert!(
        doc.sketch(s).is_none(),
        "redone extrude re-consumed the sketch (emptied, ceased to exist)"
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

// ══════════════════════════════════ group ops (docs/design/group-ops.md) ═════

// -------------------------------------------- group duplication hardening

/// Duplicating a nested group deep-clones the whole subtree: nested groups
/// become new groups, instances become new instances of the SAME definition,
/// and names / tags / base materials / painted faces all carry over.
#[test]
fn duplicate_nested_group_clones_structure_instances_and_attributes() {
    let mut doc = Document::new();
    // Inner content: a painted, named, tagged object plus a component instance.
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220)));
    doc.set_object_material(a, Some(red)).unwrap();
    let a_top = face_with_normal(&doc, a, Vec3::new(0.0, 0.0, 1.0));
    doc.paint_face(a, a_top, Some(blue)).unwrap();
    doc.set_node_name(NodeId::Object(a), Some("Leg".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(a), vec!["Furniture".into()])
        .unwrap();

    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let (comp, inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);

    let (inner, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Instance(inst)])
        .unwrap();
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
        .unwrap();
    doc.set_node_name(NodeId::Group(outer), Some("Assembly".into()))
        .unwrap();

    let offset = Vec3::new(0.0, 10.0, 0.0);
    let hash_before = doc.state_hash();
    let (root, _) = doc
        .duplicate_node(NodeId::Group(outer), &Transform::translation(offset))
        .expect("duplicate nested group");
    let hash_after = doc.state_hash();
    let outer2 = match root {
        NodeId::Group(id) => id,
        _ => panic!("a group copy is a group"),
    };
    assert_ne!(outer, outer2);

    // Structure mirrored: [Group(inner2), Object(c2)] / [Object(a2), Instance(inst2)].
    let outer2_members = doc.group_members(outer2).unwrap();
    assert_eq!(outer2_members.len(), 2);
    let inner2 = match outer2_members[0] {
        NodeId::Group(id) => id,
        _ => panic!("first member of the copy mirrors the source (a group)"),
    };
    assert!(matches!(outer2_members[1], NodeId::Object(_)));
    let inner2_members = doc.group_members(inner2).unwrap();
    assert_eq!(inner2_members.len(), 2);
    let a2 = match inner2_members[0] {
        NodeId::Object(id) => id,
        _ => panic!("nested object clone"),
    };
    let inst2 = match inner2_members[1] {
        NodeId::Instance(id) => id,
        _ => panic!("nested instance clone"),
    };

    // The cloned instance shares the SAME definition (never an implicit
    // make-unique) at the composed pose.
    assert_eq!(doc.instance_def(inst2), Some(comp));
    assert_eq!(doc.component_ids().len(), 1, "no new definition was minted");
    let probe = Point3::new(0.2, 0.4, 0.6);
    assert!(
        doc.instance_pose(inst2)
            .unwrap()
            .apply_point(probe)
            .approx_eq(probe + offset, 1e-9),
        "instance pose composed with the placement"
    );

    // Attributes carried over; geometry offset; leaves distinct.
    assert_ne!(a2, a);
    assert_eq!(doc.object_name(a2), Some("Leg"));
    assert_eq!(
        doc.node_tags(NodeId::Object(a2)),
        &[vec![String::from("Furniture")]]
    );
    assert_eq!(doc.object(a2).unwrap().default_material(), Some(red));
    assert_eq!(faces_painted(&doc, a2, blue), 1, "painted face survives");
    assert_eq!(doc.group_name(outer2), Some("Assembly"));
    assert!(approx_pt(
        centroid(doc.object(a2).unwrap()),
        centroid(doc.object(a).unwrap()) + offset
    ));

    // Independence: moving the copy's leaf leaves the source put.
    let a_centroid = centroid(doc.object(a).unwrap());
    doc.transform_object(a2, &Transform::translation(Vec3::new(0.0, 0.0, 5.0)))
        .unwrap();
    assert!(approx_pt(centroid(doc.object(a).unwrap()), a_centroid));
    doc.undo().unwrap(); // drop the probe transform

    // One undo step removes the WHOLE copy, exactly; redo restores it exactly.
    doc.undo().expect("undo duplicate");
    assert_eq!(doc.state_hash(), hash_before, "undo is exact");
    assert!(!doc.top_level_nodes().contains(&NodeId::Group(outer2)));
    doc.redo().expect("redo duplicate");
    assert_eq!(doc.state_hash(), hash_after, "redo is exact");
}

proptest! {
    /// Property (docs/design/group-ops.md §1): duplicating a group undoes to
    /// the exact prior document, and redoes to the exact copied document,
    /// under arbitrary translations.
    #[test]
    fn duplicate_group_round_trips_exactly(
        dx in -10.0..10.0f64,
        dy in -10.0..10.0f64,
        dz in -10.0..10.0f64,
    ) {
        let mut doc = Document::new();
        let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
        let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
        let (inner, _) = doc
            .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
            .unwrap();
        let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
        let (outer, _) = doc
            .group_nodes(&[NodeId::Group(inner), NodeId::Object(c)])
            .unwrap();

        let hash_before = doc.state_hash();
        doc.duplicate_node(
            NodeId::Group(outer),
            &Transform::translation(Vec3::new(dx, dy, dz)),
        )
        .expect("duplicate nested group");
        let hash_after = doc.state_hash();

        doc.undo().expect("undo duplicate");
        prop_assert_eq!(doc.state_hash(), hash_before, "undo is exact");
        doc.redo().expect("redo duplicate");
        prop_assert_eq!(doc.state_hash(), hash_after, "redo is exact");
    }
}

/// Divergence-theorem signed volume, fan-triangulating every loop of every
/// face — hole loops wind opposite the outer, so their fans subtract
/// correctly. Boolean results can carry annulus faces (a seam strictly inside
/// a face), which the hole-free `signed_volume` above overcounts.
fn enclosed_volume(obj: &Object) -> f64 {
    let mut six_v = 0.0;
    for f in obj.faces().values() {
        for lid in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let p: Vec<Vec3> = obj.loop_positions(lid).map(|pt| pt.to_vec()).collect();
            for i in 1..p.len().saturating_sub(1) {
                six_v += p[0].dot(p[i].cross(p[i + 1]));
            }
        }
    }
    six_v / 6.0
}

// ------------------------------------------------ boolean_nodes (group booleans)

/// Union of two overlapping groups fuses every solid under both into ONE
/// watertight top-level Object; both operand subtrees are consumed; undo and
/// redo restore the exact document states.
#[test]
fn boolean_nodes_union_of_two_groups_yields_one_solid_and_round_trips() {
    let mut doc = Document::new();
    // Group A: two overlapping boxes; group B: two overlapping boxes, with
    // B's first box overlapping A's second. No coplanar faces anywhere.
    let a1 = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let a2 = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 0.5, 2.0);
    let b1 = extrude_box(&mut doc, 2.5, 1.5, 4.5, 2.75, 1.0, 1.0);
    let b2 = extrude_box(&mut doc, 4.0, 1.6, 5.0, 2.6, 1.2, 1.0);
    let (ga, _) = doc
        .group_nodes(&[NodeId::Object(a1), NodeId::Object(a2)])
        .unwrap();
    let (gb, _) = doc
        .group_nodes(&[NodeId::Object(b1), NodeId::Object(b2)])
        .unwrap();

    let hash_before = doc.state_hash();
    let (root, change) = doc
        .boolean_nodes(BooleanOp::Union, NodeId::Group(ga), NodeId::Group(gb))
        .expect("group union");
    let hash_after = doc.state_hash();

    let result = match root {
        NodeId::Object(id) => id,
        _ => panic!("a connected union is a single plain Object"),
    };
    assert!(change.objects_touched.contains(&result));
    assert_eq!(
        doc.visible_object_ids(),
        vec![result],
        "every operand leaf was consumed"
    );
    assert_eq!(
        doc.top_level_nodes(),
        vec![NodeId::Object(result)],
        "both operand groups were consumed"
    );
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    let expected_volume = {
        // Inclusion-exclusion over the four boxes (pairwise overlaps only:
        // a1∩a2, a2∩b1, b1∩b2 — no triple overlaps by construction).
        let v = |x0: f64, y0: f64, x1: f64, y1: f64, z0: f64, z1: f64| {
            (x1 - x0) * (y1 - y0) * (z1 - z0)
        };
        v(0.0, 0.0, 2.0, 2.0, 0.0, 2.0) + v(1.0, 1.0, 3.0, 3.0, 0.5, 2.5)
            + v(2.5, 1.5, 4.5, 2.75, 1.0, 2.0)
            + v(4.0, 1.6, 5.0, 2.6, 1.2, 2.2)
            - v(1.0, 1.0, 2.0, 2.0, 0.5, 2.0)   // a1∩a2
            - v(2.5, 1.5, 3.0, 2.75, 1.0, 2.0)  // a2∩b1
            - v(4.0, 1.6, 4.5, 2.6, 1.2, 2.0) // b1∩b2
    };
    assert!(
        (enclosed_volume(doc.object(result).unwrap()) - expected_volume).abs() < 1e-9,
        "union volume matches set algebra: got {}, expected {}",
        enclosed_volume(doc.object(result).unwrap()),
        expected_volume
    );

    doc.undo().expect("undo group union");
    assert_eq!(doc.state_hash(), hash_before, "undo is exact");
    let visible = doc.visible_object_ids();
    assert!(
        [a1, a2, b1, b2].iter().all(|o| visible.contains(o)),
        "operand leaves restored with stable handles"
    );
    assert!(
        doc.top_level_nodes().contains(&NodeId::Group(ga))
            && doc.top_level_nodes().contains(&NodeId::Group(gb)),
        "operand groups restored"
    );

    doc.redo().expect("redo group union");
    assert_eq!(doc.state_hash(), hash_after, "redo is exact");
}

/// Mixed operands: subtracting a GROUP of cutters from a plain solid removes
/// each cutter's volume (the group is composed first, then subtracted).
#[test]
fn boolean_nodes_subtracts_a_group_from_a_plain_object() {
    let mut doc = Document::new();
    let target = extrude_box(&mut doc, 0.0, 0.0, 4.0, 1.0, 0.0, 1.0);
    // Two disjoint cutters, each biting a corner-through notch out of the
    // target (they poke out of it on y and z — no coplanar faces).
    let c1 = extrude_box(&mut doc, 0.5, -0.25, 1.0, 1.25, 0.5, 1.0);
    let c2 = extrude_box(&mut doc, 2.5, -0.25, 3.0, 1.25, 0.5, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(c1), NodeId::Object(c2)])
        .unwrap();

    let (root, _) = doc
        .boolean_nodes(
            BooleanOp::Subtract,
            NodeId::Object(target),
            NodeId::Group(g),
        )
        .expect("subtract group from object");
    let result = match root {
        NodeId::Object(id) => id,
        _ => panic!("a connected subtract result is a single Object"),
    };
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    let expected = 4.0 - 2.0 * (0.5 * 1.0 * 0.5);
    assert!(
        (enclosed_volume(doc.object(result).unwrap()) - expected).abs() < 1e-9,
        "both cutters' overlap removed"
    );
}

/// A subtract that severs the target yields one Object per piece, housed in a
/// result group named from the operands; every piece is watertight.
#[test]
fn boolean_nodes_severing_subtract_yields_named_group_of_solids() {
    let mut doc = Document::new();
    let bar = extrude_box(&mut doc, 0.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    doc.set_node_name(NodeId::Object(bar), Some("Bar".into()))
        .unwrap();
    let cutter = extrude_box(&mut doc, 1.2, -0.3, 1.8, 1.3, -0.4, 1.8);
    let (g, _) = doc.group_nodes(&[NodeId::Object(cutter)]).unwrap();
    doc.set_node_name(NodeId::Group(g), Some("Cutter".into()))
        .unwrap();

    let hash_before = doc.state_hash();
    let (root, _) = doc
        .boolean_nodes(BooleanOp::Subtract, NodeId::Object(bar), NodeId::Group(g))
        .expect("severing subtract");
    let result_group = match root {
        NodeId::Group(id) => id,
        _ => panic!("a multi-solid result arrives in a result group"),
    };
    assert_eq!(
        doc.group_name(result_group),
        Some("Bar \u{2212} Cutter"),
        "the result group is named from the operands"
    );
    let members = doc.group_members(result_group).unwrap();
    assert_eq!(members.len(), 2, "the cut severed the bar into two");
    let mut volumes = Vec::new();
    for m in &members {
        let NodeId::Object(o) = *m else {
            panic!("result group members are plain Objects");
        };
        assert_eq!(
            doc.object(o).unwrap().watertight(),
            WatertightState::Watertight,
            "every piece is watertight"
        );
        let v = enclosed_volume(doc.object(o).unwrap());
        assert!(v > 0.0, "every piece is a genuine solid");
        volumes.push(v);
    }
    let total: f64 = volumes.iter().sum();
    assert!(
        (total - (3.0 - 0.6)).abs() < 1e-9,
        "pieces sum to bar minus the cut slab"
    );
    assert_eq!(
        doc.top_level_nodes(),
        vec![NodeId::Group(result_group)],
        "operands consumed; the result group is the only root"
    );

    doc.undo().expect("undo severing subtract");
    assert_eq!(doc.state_hash(), hash_before, "undo is exact");
    doc.redo().expect("redo severing subtract");
    assert_eq!(doc.group_members(result_group).unwrap().len(), 2);
}

/// A subtract that hollows the target (the cutter is strictly inside) keeps
/// the cavity with its solid — ONE multi-shell Object, never an inside-out
/// "solid" split out on its own.
#[test]
fn boolean_nodes_hollowing_subtract_stays_one_object() {
    let mut doc = Document::new();
    let block = extrude_box(&mut doc, 0.0, 0.0, 3.0, 3.0, 0.0, 3.0);
    let void = extrude_box(&mut doc, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(void)]).unwrap();

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Subtract, NodeId::Object(block), NodeId::Group(g))
        .expect("hollowing subtract");
    let result = match root {
        NodeId::Object(id) => id,
        _ => panic!("a hollowed solid stays one Object, cavity and all"),
    };
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(
        (enclosed_volume(doc.object(result).unwrap()) - (27.0 - 1.0)).abs() < 1e-9,
        "the cavity subtracts from the enclosed volume"
    );
}

/// A hollowing subtract must not fuse UNRELATED solids into the hollow: a
/// cavity attaches to the host that contains it, and every other solid stays
/// its own discrete Object (adversarial review, major — the first guard
/// collapsed everything whenever any shell was a cavity).
#[test]
fn boolean_nodes_hollowing_subtract_keeps_unrelated_solids_discrete() {
    let mut doc = Document::new();
    let block1 = extrude_box(&mut doc, 0.0, 0.0, 3.0, 3.0, 0.0, 3.0);
    let block2 = extrude_box(&mut doc, 10.0, 0.0, 11.0, 1.0, 0.0, 1.0);
    let void = extrude_box(&mut doc, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0);
    let (ga, _) = doc
        .group_nodes(&[NodeId::Object(block1), NodeId::Object(block2)])
        .unwrap();
    let (gb, _) = doc.group_nodes(&[NodeId::Object(void)]).unwrap();

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Subtract, NodeId::Group(ga), NodeId::Group(gb))
        .expect("hollowing subtract with a bystander");
    let NodeId::Group(rg) = root else {
        panic!("two discrete solids arrive in a result group, never one multi-shell blob");
    };
    let members = doc.group_members(rg).unwrap();
    assert_eq!(members.len(), 2, "hollow block1 and untouched block2");
    let mut volumes: Vec<f64> = members
        .iter()
        .map(|m| {
            let NodeId::Object(o) = *m else {
                panic!("result group members are Objects");
            };
            assert_eq!(
                doc.object(o).unwrap().watertight(),
                WatertightState::Watertight
            );
            enclosed_volume(doc.object(o).unwrap())
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!(
        (volumes[0] - 1.0).abs() < 1e-9,
        "the bystander block is untouched (got {})",
        volumes[0]
    );
    assert!(
        (volumes[1] - 26.0).abs() < 1e-9,
        "the hollowed block keeps its cavity (got {})",
        volumes[1]
    );
}

/// Two hosts, each hollowed by its own cutter, split into two discrete
/// hollow solids — each cavity assigned to the host that contains it.
#[test]
fn boolean_nodes_two_hollowed_hosts_split_apart() {
    let mut doc = Document::new();
    let block_a = extrude_box(&mut doc, 0.0, 0.0, 3.0, 3.0, 0.0, 3.0);
    let block_b = extrude_box(&mut doc, 10.0, 0.0, 13.0, 3.0, 0.0, 3.0);
    let void_a = extrude_box(&mut doc, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0);
    let void_b = extrude_box(&mut doc, 11.0, 1.0, 12.0, 2.0, 1.0, 1.0);
    let (ga, _) = doc
        .group_nodes(&[NodeId::Object(block_a), NodeId::Object(block_b)])
        .unwrap();
    let (gb, _) = doc
        .group_nodes(&[NodeId::Object(void_a), NodeId::Object(void_b)])
        .unwrap();

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Subtract, NodeId::Group(ga), NodeId::Group(gb))
        .expect("hollow both hosts");
    let NodeId::Group(rg) = root else {
        panic!("two hollow hosts arrive as two discrete Objects in a result group");
    };
    let members = doc.group_members(rg).unwrap();
    assert_eq!(members.len(), 2);
    for m in &members {
        let NodeId::Object(o) = *m else {
            panic!("result group members are Objects");
        };
        assert_eq!(
            doc.object(o).unwrap().watertight(),
            WatertightState::Watertight
        );
        assert!(
            (enclosed_volume(doc.object(o).unwrap()) - 26.0).abs() < 1e-9,
            "each host keeps exactly its own cavity"
        );
    }
}

/// MULTIPLE candidate hosts: a cavity whose sample point lies inside more
/// than one positive shell must attach to the SMALLEST containing one (the
/// immediate host). A pre-hollowed frame fully surrounds the host with
/// clearance, so the host's new cavity is geometrically inside both the host
/// and the frame — this pins the min-by-volume discrimination against
/// min/max swaps and index mixups (delta review).
#[test]
fn boolean_nodes_cavity_with_two_candidate_hosts_picks_the_smallest() {
    let mut doc = Document::new();
    // A hollow frame: 5-cube outer, 4-cube void — built through the same
    // document ops a user would take.
    let frame_outer = extrude_box(&mut doc, -1.0, -1.0, 4.0, 4.0, -1.0, 5.0);
    let frame_void = extrude_box(&mut doc, -0.5, -0.5, 3.5, 3.5, -0.5, 4.0);
    let (gfv, _) = doc.group_nodes(&[NodeId::Object(frame_void)]).unwrap();
    let (frame_root, _) = doc
        .boolean_nodes(
            BooleanOp::Subtract,
            NodeId::Object(frame_outer),
            NodeId::Group(gfv),
        )
        .expect("hollow the frame");
    let NodeId::Object(frame) = frame_root else {
        panic!("a hollowed frame is one Object");
    };

    // The host cube floats inside the frame's void with clearance; both go
    // in one operand group. Subtracting the cutter hollows the host — and
    // the new cavity's sample point is inside BOTH the host and the frame.
    let host = extrude_box(&mut doc, 0.0, 0.0, 3.0, 3.0, 0.0, 3.0);
    let (ga, _) = doc
        .group_nodes(&[NodeId::Object(host), NodeId::Object(frame)])
        .unwrap();
    let cutter = extrude_box(&mut doc, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0);
    let (gb, _) = doc.group_nodes(&[NodeId::Object(cutter)]).unwrap();

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Subtract, NodeId::Group(ga), NodeId::Group(gb))
        .expect("hollow the host inside the frame");
    let NodeId::Group(rg) = root else {
        panic!("host and frame are two discrete solids in a result group");
    };
    let members = doc.group_members(rg).unwrap();
    assert_eq!(
        members.len(),
        2,
        "the hollowed host and the untouched frame"
    );
    let mut volumes: Vec<f64> = members
        .iter()
        .map(|m| {
            let NodeId::Object(o) = *m else {
                panic!("result group members are Objects");
            };
            assert_eq!(
                doc.object(o).unwrap().watertight(),
                WatertightState::Watertight
            );
            enclosed_volume(doc.object(o).unwrap())
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!(
        (volumes[0] - 26.0).abs() < 1e-9,
        "the cavity landed in the SMALLEST containing host (got {})",
        volumes[0]
    );
    assert!(
        (volumes[1] - 61.0).abs() < 1e-9,
        "the surrounding frame is untouched (got {})",
        volumes[1]
    );
}

/// Nested shells: subtracting a HOLLOW cutter leaves a floating island where
/// the cutter's cavity was. The island is a discrete solid; the big cavity
/// stays with the host that contains it (smallest-containing-host rule).
#[test]
fn boolean_nodes_island_inside_cavity_assigns_shells_correctly() {
    let mut doc = Document::new();
    // Build the hollow cutter first: 3-cube shell minus 1-cube core, one
    // Object carrying its cavity.
    let cutter_outer = extrude_box(&mut doc, 1.0, 1.0, 4.0, 4.0, 1.0, 3.0);
    let cutter_void = extrude_box(&mut doc, 2.0, 2.0, 3.0, 3.0, 2.0, 1.0);
    let (gv, _) = doc.group_nodes(&[NodeId::Object(cutter_void)]).unwrap();
    let (hollow_root, _) = doc
        .boolean_nodes(
            BooleanOp::Subtract,
            NodeId::Object(cutter_outer),
            NodeId::Group(gv),
        )
        .expect("make the hollow cutter");
    let NodeId::Object(hollow_cutter) = hollow_root else {
        panic!("a hollowed solid is one Object, cavity and all");
    };

    // 5-cube block minus the hollow cutter: the cutter's material (26)
    // leaves; the cutter's core survives as a floating 1-cube island inside
    // the cavity.
    let block = extrude_box(&mut doc, 0.0, 0.0, 5.0, 5.0, 0.0, 5.0);
    let (root, _) = doc
        .boolean_nodes(
            BooleanOp::Subtract,
            NodeId::Object(block),
            NodeId::Object(hollow_cutter),
        )
        .expect("subtract the hollow cutter");
    let NodeId::Group(rg) = root else {
        panic!("host + island are two discrete solids in a result group");
    };
    let members = doc.group_members(rg).unwrap();
    assert_eq!(members.len(), 2, "the hollowed host and the island");
    let mut volumes: Vec<f64> = members
        .iter()
        .map(|m| {
            let NodeId::Object(o) = *m else {
                panic!("result group members are Objects");
            };
            assert_eq!(
                doc.object(o).unwrap().watertight(),
                WatertightState::Watertight
            );
            enclosed_volume(doc.object(o).unwrap())
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!(
        (volumes[0] - 1.0).abs() < 1e-9,
        "the island (got {})",
        volumes[0]
    );
    assert!(
        (volumes[1] - 98.0).abs() < 1e-9,
        "the host keeps the 3-cube cavity (got {})",
        volumes[1]
    );
}

/// Instance operands are refused typed — as the operand itself and nested
/// anywhere under a group operand — never implicitly made unique. The
/// document is untouched by every refusal.
#[test]
fn boolean_nodes_refuses_instance_operands() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let b = extrude_box(&mut doc, 0.5, 0.5, 1.5, 1.5, 0.5, 1.0);
    let c = extrude_box(&mut doc, 0.25, 0.25, 1.75, 0.75, 0.75, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(inst), NodeId::Object(b)])
        .unwrap();
    let hash = doc.state_hash();

    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Instance(inst), NodeId::Object(c))
            .map(|_| ()),
        Err(DocumentError::BooleanOperandHasInstance),
        "an instance operand is refused"
    );
    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Group(g), NodeId::Object(c))
            .map(|_| ()),
        Err(DocumentError::BooleanOperandHasInstance),
        "an instance nested under a group operand is refused"
    );
    assert_eq!(
        doc.state_hash(),
        hash,
        "refusals leave the document untouched"
    );
}

/// A leaky (non-watertight) leaf anywhere under an operand refuses the whole
/// op, naming the offending side; the document is untouched.
#[test]
fn boolean_nodes_refuses_leaky_leaves_naming_the_side() {
    let mut doc = Document::new();
    // Ingest an open shell (a box missing its top) — the one honest way a
    // leaky object exists in a document.
    let open = MeshRecipe {
        name: "open".into(),
        positions: vec![
            Point3::new(10.0, 10.0, 10.0),
            Point3::new(11.0, 10.0, 10.0),
            Point3::new(11.0, 11.0, 10.0),
            Point3::new(10.0, 11.0, 10.0),
            Point3::new(10.0, 10.0, 11.0),
            Point3::new(11.0, 10.0, 11.0),
            Point3::new(11.0, 11.0, 11.0),
            Point3::new(10.0, 11.0, 11.0),
        ],
        faces: vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
        face_materials: vec![kernel::NO_MATERIAL; 5],
        face_uv_frames: vec![None; 5],
        face_holes: vec![Vec::new(); 5],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
    };
    let scene = ImportScene {
        materials: vec![],
        defs: vec![],
        roots: vec![ImportNode::Mesh(open)],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    let (report, change) = doc.ingest(scene, vec![]).unwrap();
    assert_eq!(report.leaky, 1);
    let leaky = change.objects_touched[0];
    assert!(!doc.object_solid(leaky));

    let solid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let other = extrude_box(&mut doc, 0.5, 0.5, 1.5, 1.5, 0.5, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(leaky), NodeId::Object(solid)])
        .unwrap();
    let hash = doc.state_hash();

    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Group(g), NodeId::Object(other))
            .map(|_| ()),
        Err(DocumentError::BooleanOperandNotSolid { which: Operand::A })
    );
    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Object(other), NodeId::Group(g))
            .map(|_| ()),
        Err(DocumentError::BooleanOperandNotSolid { which: Operand::B })
    );
    assert_eq!(
        doc.state_hash(),
        hash,
        "refusals leave the document untouched"
    );
}

/// Structural refusals: an operand nested inside another group, self-combine,
/// and stale handles — all typed, all leaving the document untouched.
#[test]
fn boolean_nodes_refuses_nested_self_and_stale_operands() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 0.0, 1.0);
    let c = extrude_box(&mut doc, 4.0, 0.0, 5.0, 1.0, 0.0, 1.0);
    let (g1, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let (_g2, _) = doc
        .group_nodes(&[NodeId::Group(g1), NodeId::Object(c)])
        .unwrap();
    let d = extrude_box(&mut doc, 6.0, 0.0, 7.0, 1.0, 0.0, 1.0);
    let hash = doc.state_hash();

    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Group(g1), NodeId::Object(d))
            .map(|_| ()),
        Err(DocumentError::GroupedOperand),
        "an operand inside another group is refused, like every replacing op"
    );
    assert_eq!(
        doc.boolean_nodes(BooleanOp::Union, NodeId::Object(d), NodeId::Object(d))
            .map(|_| ()),
        Err(DocumentError::Boolean(BooleanError::DegenerateContact)),
        "self-combine is refused"
    );
    assert_eq!(
        doc.boolean_nodes(
            BooleanOp::Union,
            NodeId::Group(GroupId::default()),
            NodeId::Object(d)
        )
        .map(|_| ()),
        Err(DocumentError::UnknownGroup),
        "a stale operand is refused"
    );
    assert_eq!(
        doc.state_hash(),
        hash,
        "refusals leave the document untouched"
    );
    assert!(doc.can_undo(), "only the construction steps are on the log");
}

/// Materials follow the existing boolean rules through a group boolean:
/// painted faces survive onto the result, and the result's base material is
/// operand A's first leaf's base.
#[test]
fn boolean_nodes_preserves_materials() {
    let mut doc = Document::new();
    let a1 = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let a2 = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 0.5, 2.0);
    let b = extrude_box(&mut doc, 2.5, 1.5, 4.0, 2.5, 1.0, 2.0);
    let oak = doc.add_material(Material::solid("Oak", Rgba8::rgb(180, 140, 90)));
    let red = doc.add_material(Material::solid("Red", Rgba8::rgb(220, 30, 30)));
    let blue = doc.add_material(Material::solid("Blue", Rgba8::rgb(30, 30, 220)));
    doc.set_object_material(a1, Some(oak)).unwrap();
    let a1_bottom = face_with_normal(&doc, a1, Vec3::new(0.0, 0.0, -1.0));
    doc.paint_face(a1, a1_bottom, Some(red)).unwrap();
    let b_top = face_with_normal(&doc, b, Vec3::new(0.0, 0.0, 1.0));
    doc.paint_face(b, b_top, Some(blue)).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a1), NodeId::Object(a2)])
        .unwrap();

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Union, NodeId::Group(g), NodeId::Object(b))
        .expect("painted union");
    let NodeId::Object(result) = root else {
        panic!("connected union is one Object");
    };
    assert_eq!(
        doc.object(result).unwrap().default_material(),
        Some(oak),
        "result base material is operand A's first leaf's base"
    );
    assert!(faces_painted(&doc, result, red) >= 1, "A's paint survives");
    assert!(faces_painted(&doc, result, blue) >= 1, "B's paint survives");
}

/// A NESTED group operand composes its whole subtree: an outer group holding
/// an inner group plus its own solid fuses all three levels' leaves before
/// the op applies (nesting appears only in refusal tests otherwise).
#[test]
fn boolean_nodes_composes_nested_group_operands() {
    let mut doc = Document::new();
    // inner{box1} nested in outer{inner, box2}; box1 and box2 overlap each
    // other, and the operand `c` overlaps box2 — one connected union.
    let box1 = extrude_box(&mut doc, 0.0, 0.0, 2.0, 2.0, 0.0, 2.0);
    let box2 = extrude_box(&mut doc, 1.0, 1.0, 3.0, 3.0, 0.5, 2.0);
    let (inner, _) = doc.group_nodes(&[NodeId::Object(box1)]).unwrap();
    let (outer, _) = doc
        .group_nodes(&[NodeId::Group(inner), NodeId::Object(box2)])
        .unwrap();
    let c = extrude_box(&mut doc, 2.5, 1.5, 4.0, 2.5, 1.0, 1.0);

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Union, NodeId::Group(outer), NodeId::Object(c))
        .expect("nested-group union");
    let NodeId::Object(result) = root else {
        panic!("a connected union of a nested group is one Object");
    };
    assert_eq!(
        doc.object(result).unwrap().watertight(),
        WatertightState::Watertight
    );
    let expected = {
        let v = |x0: f64, y0: f64, x1: f64, y1: f64, z0: f64, z1: f64| {
            (x1 - x0) * (y1 - y0) * (z1 - z0)
        };
        v(0.0, 0.0, 2.0, 2.0, 0.0, 2.0) + v(1.0, 1.0, 3.0, 3.0, 0.5, 2.5)
            + v(2.5, 1.5, 4.0, 2.5, 1.0, 2.0)
            - v(1.0, 1.0, 2.0, 2.0, 0.5, 2.0) // box1∩box2
            - v(2.5, 1.5, 3.0, 2.5, 1.0, 2.0) // box2∩c
    };
    assert!(
        (enclosed_volume(doc.object(result).unwrap()) - expected).abs() < 1e-9,
        "every level of the nested operand contributed its volume"
    );
    assert_eq!(
        doc.top_level_nodes(),
        vec![NodeId::Object(result)],
        "the whole nested operand subtree was consumed"
    );
}

/// FLUSH group members weld during composition and their coplanar seams
/// dissolve on the result: a group of two side-by-side boxes unioned with an
/// interior solid reads as the canonical 2×1×1 box — six faces, no seams
/// (every other boolean_nodes fixture deliberately avoids coplanar faces).
#[test]
fn boolean_nodes_dissolves_seams_between_flush_group_members() {
    let mut doc = Document::new();
    let b1 = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let b2 = extrude_box(&mut doc, 1.0, 0.0, 2.0, 1.0, 0.0, 1.0); // flush at x=1
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(b1), NodeId::Object(b2)])
        .unwrap();
    // Strictly interior to the fused slab, so it changes nothing visible —
    // the result must be exactly the seamless slab.
    let c = extrude_box(&mut doc, 0.4, 0.4, 1.6, 0.6, 0.3, 0.4);

    let (root, _) = doc
        .boolean_nodes(BooleanOp::Union, NodeId::Group(g), NodeId::Object(c))
        .expect("flush-members union");
    let NodeId::Object(result) = root else {
        panic!("one connected slab");
    };
    assert_eq!(
        doc.object(result).unwrap().faces().len(),
        6,
        "composition seams between flush members dissolve to the canonical box"
    );
    assert!(
        (enclosed_volume(doc.object(result).unwrap()) - 2.0).abs() < 1e-9,
        "the interior operand adds nothing"
    );
}

proptest! {
    /// Property (docs/design/group-ops.md §3): unioning a group of disjoint
    /// boxes with a plain box that bridges them is the SAME volume algebra as
    /// unioning the same solids sequentially with the object-level boolean —
    /// same watertightness, same connectivity, same enclosed volume.
    #[test]
    fn boolean_nodes_group_union_matches_sequential_unions(
        dx in 0.05..0.9f64,
        w in 0.2..0.35f64,
    ) {
        // Two disjoint boxes bridged by a tube whose base is COPLANAR with
        // theirs (z = 0), so every case exercises exact coplanar-contact
        // resolution during composition, not just transversal crossings.
        let build = |doc: &mut Document| -> (ObjectId, ObjectId, ObjectId) {
            let a1 = extrude_box(doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
            let a2 = extrude_box(doc, 2.0 + dx, 0.0, 3.0 + dx, 1.0, 0.0, 1.0);
            let c = extrude_box(doc, -0.3, 0.5 - w, 2.3 + dx, 0.5 + w, 0.0, 0.5);
            (a1, a2, c)
        };

        // Compound path: group {a1, a2}, then one group union with c.
        let mut doc1 = Document::new();
        let (a1, a2, c) = build(&mut doc1);
        let (g, _) = doc1
            .group_nodes(&[NodeId::Object(a1), NodeId::Object(a2)])
            .unwrap();
        let (root, _) = doc1
            .boolean_nodes(BooleanOp::Union, NodeId::Group(g), NodeId::Object(c))
            .expect("group union");
        let NodeId::Object(r1) = root else {
            panic!("bridged union is connected — a single Object");
        };

        // Sequential path: (a1 ∪ c) ∪ a2 through the object-level boolean.
        let mut doc2 = Document::new();
        let (a1, a2, c) = build(&mut doc2);
        let (s1, _) = doc2.boolean(BooleanOp::Union, a1, c).expect("a1 ∪ c");
        let (r2, _) = doc2.boolean(BooleanOp::Union, s1, a2).expect("∪ a2");

        let o1 = doc1.object(r1).unwrap();
        let o2 = doc2.object(r2).unwrap();
        prop_assert_eq!(o1.watertight(), WatertightState::Watertight);
        prop_assert_eq!(o2.watertight(), WatertightState::Watertight);
        prop_assert_eq!(o1.split_connected_components().len(), 1);
        prop_assert_eq!(o2.split_connected_components().len(), 1);
        prop_assert!(
            (enclosed_volume(o1) - enclosed_volume(o2)).abs() < 1e-9,
            "both paths enclose the same volume: {} vs {}",
            enclosed_volume(o1),
            enclosed_volume(o2)
        );
    }

    /// Property (docs/design/group-ops.md §3): a severing group subtract
    /// always yields all-watertight, positive-volume pieces in a result
    /// group, and every boolean_nodes op undoes (and redoes) to the EXACT
    /// prior document state.
    #[test]
    fn boolean_nodes_severs_watertight_and_round_trips_exactly(
        u in 0.05..1.3f64,
        op_pick in 0..3usize,
    ) {
        let mut doc = Document::new();
        let bar = extrude_box(&mut doc, 0.0, 0.0, 3.0, 1.0, 0.0, 1.0);
        let cutter = extrude_box(&mut doc, 1.0 + u, -0.3, 1.5 + u, 1.3, -0.4, 1.8);
        // The cutter arrives NESTED (group in group), so every case also
        // exercises multi-level operand composition.
        let (inner, _) = doc.group_nodes(&[NodeId::Object(cutter)]).unwrap();
        let (g, _) = doc.group_nodes(&[NodeId::Group(inner)]).unwrap();

        let op = [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect][op_pick];
        let hash_before = doc.state_hash();
        let (root, _) = doc
            .boolean_nodes(op, NodeId::Object(bar), NodeId::Group(g))
            .expect("transversal operands combine cleanly");
        let hash_after = doc.state_hash();

        // Every result piece is watertight with positive volume; a severing
        // subtract arrives as a result group of two.
        let pieces: Vec<ObjectId> = match root {
            NodeId::Object(o) => vec![o],
            NodeId::Group(rg) => doc
                .group_members(rg)
                .unwrap()
                .into_iter()
                .map(|m| match m {
                    NodeId::Object(o) => o,
                    _ => panic!("result group members are Objects"),
                })
                .collect(),
            NodeId::Instance(_) => panic!("a boolean result is never an instance"),
        };
        if op == BooleanOp::Subtract {
            prop_assert_eq!(pieces.len(), 2, "the cut severs the bar");
        }
        for &p in &pieces {
            prop_assert_eq!(
                doc.object(p).unwrap().watertight(),
                WatertightState::Watertight
            );
            prop_assert!(enclosed_volume(doc.object(p).unwrap()) > 0.0);
        }

        doc.undo().expect("undo");
        prop_assert_eq!(doc.state_hash(), hash_before, "undo is exact");
        doc.redo().expect("redo");
        prop_assert_eq!(doc.state_hash(), hash_after, "redo is exact");
        doc.undo().expect("undo again");
        prop_assert_eq!(doc.state_hash(), hash_before, "undo stays exact");
    }
}

// ----------------------------------------------------------------- follow me

/// The x = 0 plane with normal +x (for Follow Me profiles perpendicular to
/// a ground path heading +x).
fn profile_plane_x(x: f64) -> Plane {
    Plane::from_polygon(&[
        Point3::new(x, 0.0, 0.0),
        Point3::new(x, 1.0, 0.0),
        Point3::new(x, 0.0, 1.0),
    ])
    .expect("vertical plane is well-defined")
}

/// Draw a square profile on the x = `x` plane spanning `[y0, y1] x [z0, z1]`.
fn draw_profile_rect(doc: &mut Document, s: SketchId, x: f64, y0: f64, z0: f64, y1: f64, z1: f64) {
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let corners = [
        (Point3::new(x, y0, z0), Point3::new(x, y1, z0)),
        (Point3::new(x, y1, z0), Point3::new(x, y1, z1)),
        (Point3::new(x, y1, z1), Point3::new(x, y0, z1)),
        (Point3::new(x, y0, z1), Point3::new(x, y0, z0)),
    ];
    for (a, b) in corners {
        sk.add_segment(a, b).expect("profile segment");
    }
}

#[test]
fn follow_me_commits_like_extrusion_and_undo_restores_the_profile() {
    let mut doc = Document::new();

    // Profile: a square on the x = 0 plane.
    let ps = doc.add_sketch(profile_plane_x(0.0));
    draw_profile_rect(&mut doc, ps, 0.0, -0.3, 0.5, 0.3, 1.1);
    let region = only_region(&doc, ps);

    // Path: an L of two sketch edges on the ground, starting on the
    // profile plane.
    let gs = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(gs).expect("sketch is live");
        sk.add_segment(Point3::ORIGIN, Point3::new(2.0, 0.0, 0.0))
            .expect("path segment");
        sk.add_segment(Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 2.0, 0.0))
            .expect("path segment");
    }
    let edges: Vec<SketchEdgeId> = doc.sketch(gs).expect("live").edges().keys().collect();

    let (id, change) = doc
        .follow_me(
            ps,
            region,
            &kernel::FollowMePath::SketchEdges { sketch: gs, edges },
        )
        .expect("follow me");
    assert_eq!(change.objects_touched, vec![id]);
    assert_eq!(change.sketches_touched, vec![ps]);
    assert_eq!(doc.visible_object_ids(), vec![id]);

    let solid = doc.object(id).expect("swept solid is live");
    assert_eq!(solid.watertight(), WatertightState::Watertight);
    assert_eq!(solid.faces().len(), 10, "2 caps + 2 segments x 4 walls");
    assert!(doc.object_solid(id), "export gating sees a solid");

    // The profile sketch was wholly consumed (Model D); the PATH sketch is
    // untouched — its spine stays drawn.
    assert!(doc.sketch(ps).is_none(), "profile sketch became the solid");
    assert_eq!(doc.sketch(gs).expect("path sketch lives").edges().len(), 2);

    // Undo: solid hidden, profile outline back, path still untouched.
    doc.undo().expect("undo follow me");
    assert!(doc.visible_object_ids().is_empty());
    assert_eq!(doc.sketch(ps).expect("profile restored").edges().len(), 4);
    assert_eq!(doc.sketch(gs).expect("path sketch lives").edges().len(), 2);

    // Redo: the same ObjectId returns, the profile is consumed again.
    doc.redo().expect("redo follow me");
    assert_eq!(doc.visible_object_ids(), vec![id]);
    assert!(doc.sketch(ps).is_none());
}

#[test]
fn follow_me_around_a_face_loop_leaves_the_solid_untouched() {
    let mut doc = Document::new();
    let cube = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let top: FaceId = doc
        .object(cube)
        .expect("cube is live")
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .expect("cube has a top face");

    // Profile on the x = 0.5 plane, straddling the top rim from outside
    // (y < 0), crossing the top face's y = 0 boundary edge mid-span.
    let ps = doc.add_sketch(profile_plane_x(0.5));
    draw_profile_rect(&mut doc, ps, 0.5, -0.3, 0.9, -0.05, 1.15);
    let region = only_region(&doc, ps);

    let (id, _) = doc
        .follow_me(
            ps,
            region,
            &kernel::FollowMePath::FaceLoop {
                object: cube,
                face: top,
            },
        )
        .expect("follow me around the face loop");

    let ring = doc.object(id).expect("molding ring is live");
    assert_eq!(ring.watertight(), WatertightState::Watertight);
    // Closed 4-corner loop, anchor split: 5 segments x 4 profile edges.
    assert_eq!(ring.faces().len(), 20);
    assert!(doc.object_solid(id));

    // The path solid is untouched — the ring is a separate Object the user
    // may union or subtract explicitly.
    assert_eq!(doc.object(cube).expect("cube lives").faces().len(), 6);
    assert!(doc.visible_object_ids().contains(&cube));

    // Save/load round-trips the swept solid byte-exactly (state hash).
    let bytes = doc.save();
    let loaded = Document::load(&bytes).expect("load");
    assert_eq!(loaded.state_hash(), doc.state_hash());
}

#[test]
fn follow_me_path_resolution_refuses_typed_and_touches_nothing() {
    let mut doc = Document::new();
    let ps = doc.add_sketch(profile_plane_x(0.0));
    draw_profile_rect(&mut doc, ps, 0.0, -0.3, 0.5, 0.3, 1.1);
    let region = only_region(&doc, ps);

    let gs = doc.add_sketch(ground());
    {
        let sk = doc.sketch_mut(gs).expect("sketch is live");
        // A branching fork: three edges meeting at (2, 0, 0)...
        sk.add_segment(Point3::ORIGIN, Point3::new(2.0, 0.0, 0.0))
            .expect("segment");
        sk.add_segment(Point3::new(2.0, 0.0, 0.0), Point3::new(4.0, 0.0, 0.0))
            .expect("segment");
        sk.add_segment(Point3::new(2.0, 0.0, 0.0), Point3::new(2.0, 2.0, 0.0))
            .expect("segment");
        // ...plus a disconnected stroke far away.
        sk.add_segment(Point3::new(9.0, 9.0, 0.0), Point3::new(9.5, 9.0, 0.0))
            .expect("segment");
    }
    let all: Vec<SketchEdgeId> = doc.sketch(gs).expect("live").edges().keys().collect();
    let hash_before = doc.state_hash();

    let path = |edges: Vec<SketchEdgeId>| kernel::FollowMePath::SketchEdges { sketch: gs, edges };

    // Branching fork.
    assert!(matches!(
        doc.follow_me(ps, region, &path(all.clone())).unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::PathBranches)
    ));
    // Two disconnected chains (drop one fork arm, keep the far stroke).
    assert!(matches!(
        doc.follow_me(ps, region, &path(vec![all[0], all[1], all[3]]))
            .unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::PathDisconnected)
    ));
    // No edges at all.
    assert!(matches!(
        doc.follow_me(ps, region, &path(Vec::new())).unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::EmptyPath)
    ));
    // A stale edge handle.
    let stale = all[3];
    doc.sketch_mut(gs)
        .expect("live")
        .remove_edge(stale)
        .expect("remove");
    assert!(matches!(
        doc.follow_me(ps, region, &path(vec![all[0], stale]))
            .unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::UnknownPathEdge)
    ));
    let hash_after_removal = doc.state_hash();

    // A sweep refusal from the kernel op surfaces through the same
    // wrapper: edge 1 runs perpendicular to the profile plane but starts
    // at x = 2, detached from it; edge 2 heads +y, never perpendicular.
    assert!(matches!(
        doc.follow_me(ps, region, &path(vec![all[1]])).unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::PathDetachedFromProfile)
    ));
    assert!(matches!(
        doc.follow_me(ps, region, &path(vec![all[2]])).unwrap_err(),
        DocumentError::FollowMe(kernel::FollowMeError::ProfileNotPerpendicular)
    ));

    // The failed calls touched nothing (strong guarantee) — only the
    // explicit remove_edge above changed the document.
    assert_ne!(hash_before, hash_after_removal, "remove_edge is real");
    assert_eq!(doc.state_hash(), hash_after_removal);
    assert!(doc.sketch(ps).is_some(), "profile sketch untouched");
    assert!(doc.visible_object_ids().is_empty());

    // A hidden path sketch is unknown.
    doc.delete_sketch(gs).expect("delete path sketch");
    assert!(matches!(
        doc.follow_me(ps, region, &path(vec![all[0]])).unwrap_err(),
        DocumentError::UnknownSketch
    ));
}
