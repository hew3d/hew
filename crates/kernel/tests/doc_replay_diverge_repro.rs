//! Acceptance spec distilled from `document_fuzz.rs` for a since-resolved
//! gap: replaying the same document log twice (unwind-all / replay-all,
//! twice) produced two semantically identical documents whose `save()` bytes
//! differed. `save()` emitted vertices and faces in slotmap order, and the
//! slot free-list state after a full undo/redo cycle differs from the state
//! before it, so identical replayed ops landed in different slots.
//!
//! Resolved by the canonical geometry writer (`Object::encode`,
//! HEW_FILE_FORMAT.md §3.1): loops are rotated to their lexicographically
//! smallest position, faces are sorted by their canonicalized rings, and
//! vertices are indexed by first appearance in that walk — a
//! format-compatible, topology-derived order that no longer depends on slot
//! allocation. Set DUMP_DIR to write both containers for diffing on failure.

use kernel::{Document, KernelOp, NodeId, ObjectId, Plane, Point3, Vec3};

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
        .expect("one region");
    let (oid, _) = doc.extrude_region(s, region, h).expect("box extrudes");
    oid
}

fn split_op(
    doc: &Document,
    oid: ObjectId,
    face_sel: usize,
    ea: usize,
    eb: usize,
    ta: f64,
    tb: f64,
) -> Option<KernelOp> {
    let obj = doc.object(oid)?;
    let face = obj.faces().keys().nth(face_sel % obj.faces().len())?;
    let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
    let sides = boundary.len();
    let (a, b) = (ea % sides, eb % sides);
    if a == b {
        return None;
    }
    let point_on = |i: usize, t: f64| {
        let p = boundary[i];
        let q = boundary[(i + 1) % sides];
        p + (q - p) * t
    };
    Some(KernelOp::SplitFace {
        face,
        path: vec![point_on(a, ta), point_on(b, tb)],
        restore: None,
    })
}

#[test]
fn document_replay_is_deterministic() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    add_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.5);
    add_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.5);

    let obj0 = |doc: &Document| doc.visible_object_ids()[0];

    // Op 1: split obj0 face 0.
    let oid = obj0(&doc);
    if let Some(op) = split_op(&doc, oid, 0, 237704701, 18629424587095398, 0.25, 0.25) {
        let _ = doc.apply_object_op(oid, op);
    }
    // Op 2: group the first two top-level nodes.
    let members: Vec<NodeId> = doc.top_level_nodes().into_iter().take(2).collect();
    let _ = doc.group_nodes(&members);
    // Op 3: push/pull obj0 face 0.
    let oid = obj0(&doc);
    let obj = doc.object(oid).unwrap();
    let face = obj.faces().keys().next().unwrap();
    let _ = doc.apply_object_op(
        oid,
        KernelOp::PushPull {
            face,
            distance: 1.9517919806020803,
        },
    );
    // Op 4: split obj0 again.
    let oid = obj0(&doc);
    if let Some(op) = split_op(
        &doc,
        oid,
        3576290684,
        2303387955956557570,
        5522761704381226509,
        0.6677184794123432,
        0.5800346651208398,
    ) {
        let _ = doc.apply_object_op(oid, op);
    }

    let cycle = |doc: &mut Document| -> Vec<u8> {
        while doc.can_undo() {
            doc.undo().expect("undo succeeds");
        }
        while doc.can_redo() {
            doc.redo().expect("redo succeeds");
        }
        doc.save()
    };

    let save1 = cycle(&mut doc);
    let save2 = cycle(&mut doc);
    if save1 != save2 {
        if let Ok(dir) = std::env::var("DUMP_DIR") {
            std::fs::write(format!("{dir}/replay1.hew"), &save1).unwrap();
            std::fs::write(format!("{dir}/replay2.hew"), &save2).unwrap();
        }
        panic!(
            "replay diverged: {} vs {} bytes (set DUMP_DIR to dump)",
            save1.len(),
            save2.len()
        );
    }
}
