//! Executable specs for [`kernel::Document`] — the document model backbone.
//!
//! These pin the behaviour the wasm-api shim depends on: many first-class
//! sketches and objects coexist; extrusion consumes exactly its region; the
//! document undo log is an identity on visible state and keeps `ObjectId`s
//! stable across undo/redo.

use kernel::{
    BooleanError, BooleanOp, Document, DocumentError, KernelOp, Object, Plane, Point3, Transform,
    TransformError, Vec3, WatertightState,
};

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

// ----------------------------------------------------------------- helper

/// The +Z (top) face of an extruded box.
fn top_face(obj: &Object) -> kernel::FaceId {
    obj.faces()
        .iter()
        .find(|(_, f)| f.plane.normal().approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9))
        .map(|(id, _)| id)
        .expect("a top face exists")
}
