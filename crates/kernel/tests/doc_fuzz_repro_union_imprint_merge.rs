//! Acceptance spec distilled from `document_fuzz.rs` for a defect in the
//! boolean's seam cleanup, present on main independently of any push/pull
//! work (it reproduces at the branch base): a union of two overlapping
//! boxes, one reshaped by push/pull and both carrying `split_face_inner`
//! imprints, drives `merge_coplanar_faces` → `merge_faces` into committing
//! topology where a half-edge still references a dead loop — the debug
//! validator panics with "dangling handle: half-edge loop" inside the
//! mutation (torture mode reproduces it in release too).
//!
//! The sequence below is the proptest-shrunk minimal input, with the
//! harness's exact selector-resolution logic inlined so the case stays a
//! faithful, permanent reproducer. Un-ignore when the merge fix lands
//! (docs/ROADMAP.md, deferred list).

use kernel::{BooleanOp, Document, KernelOp, ObjectId, Plane, Point3, Vec3};

fn add_box(doc: &mut Document, x: f64, y: f64, dx: f64, dy: f64, h: f64) -> ObjectId {
    let plane = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("unit normal");
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).expect("gesture opens");
    {
        let sk = doc.sketch_mut(s).expect("sketch exists");
        let p = [
            Point3::new(x, y, 0.0),
            Point3::new(x + dx, y, 0.0),
            Point3::new(x + dx, y + dy, 0.0),
            Point3::new(x, y + dy, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(p[k], p[(k + 1) % 4]).expect("segment adds");
        }
    }
    doc.end_sketch_gesture(s).expect("gesture closes");
    let region = doc
        .sketch(s)
        .expect("sketch exists")
        .regions()
        .keys()
        .next()
        .expect("rectangle closes one region");
    let (oid, _) = doc.extrude_region(s, region, h).expect("box extrudes");
    oid
}

fn nth(items: &[ObjectId], sel: usize) -> ObjectId {
    items[sel % items.len()]
}

fn nth_face(doc: &Document, oid: ObjectId, face_sel: usize) -> kernel::FaceId {
    let obj = doc.object(oid).expect("visible id resolves");
    obj.faces()
        .keys()
        .nth(face_sel % obj.faces().len())
        .expect("faces are non-empty")
}

fn imprint(doc: &mut Document, obj_sel: usize, face_sel: usize, shrink: f64) {
    let oid = nth(&doc.visible_object_ids(), obj_sel);
    let face = nth_face(doc, oid, face_sel);
    let obj = doc.object(oid).expect("visible id resolves");
    let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
    let inv = 1.0 / boundary.len() as f64;
    let c = boundary.iter().fold(Point3::new(0.0, 0.0, 0.0), |acc, p| {
        Point3::new(acc.x + p.x * inv, acc.y + p.y * inv, acc.z + p.z * inv)
    });
    let loop_path: Vec<Point3> = boundary.iter().map(|&p| c + (p - c) * shrink).collect();
    let _ = doc.apply_object_op(
        oid,
        KernelOp::SplitFaceInner {
            face,
            loop_path,
            restore: None,
            curve: None,
        },
    );
}

// Resolved by the annular-seam merge_faces fix (SharedChainCoversBoundary /
// WouldCorrupt refusal) landed with the history-soundness work: the union
// seam cleanup no longer corrupts a loop handle on imprinted operands.
#[test]
fn union_of_imprinted_boxes_survives_seam_cleanup() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    add_box(
        &mut doc,
        0.0,
        4.7208714913252985,
        1.0,
        4.537596905826662,
        0.5,
    );
    add_box(
        &mut doc,
        0.0,
        5.966528039953601,
        3.621990804344813,
        1.0,
        0.5,
    );
    add_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.5);

    // Reshape one operand.
    {
        let oid = nth(&doc.visible_object_ids(), 12879252687764150959);
        let face = nth_face(&doc, oid, 12538251939287504099);
        let _ = doc.apply_object_op(
            oid,
            KernelOp::PushPull {
                face,
                distance: 2.1234617941358462,
            },
        );
    }
    // Imprint both operands.
    imprint(
        &mut doc,
        221596429870244547,
        6093196552748698386,
        0.467587104047185,
    );
    imprint(&mut doc, 15752604327855391015, 6914297881834180050, 0.3);

    // The union's seam cleanup must either produce valid topology or refuse
    // typed; today it panics inside `merge_faces` (debug) / trips the
    // torture-mode validator (release).
    let ids = doc.visible_object_ids();
    let a = nth(&ids, 8358807034271824536);
    let b = nth(&ids, 14659627709849515495);
    let _ = doc.boolean(BooleanOp::Union, a, b);

    for oid in doc.visible_object_ids() {
        doc.object(oid)
            .expect("visible id resolves")
            .validate()
            .expect("every visible object validates after the union");
    }
}
