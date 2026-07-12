//! Deterministic repro distilled from `document_fuzz.rs`: after a push/pull,
//! `load(save(doc))` no longer reproduces the document's `state_hash` —
//! save → load → save is not a fixed point. Set DUMP_DIR to write both
//! containers for diffing.

use kernel::{Document, KernelOp, ObjectId, Plane, Point3, Vec3};

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

#[test]
fn save_load_is_a_fixed_point_after_pushpull() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    // The boxes sit on the same plane but apart: the standing-solid gate
    // (docs/design/sketch-solid-model.md §4D) refuses the coincident
    // placement the original fuzz case used, and the overlap was always
    // incidental to what this repro pins (the push/pull save/load hash).
    add_box(&mut doc, 0.0, 0.0, 1.0, 1.2150904306205195, 0.5);
    add_box(&mut doc, 2.0, 0.0, 1.0, 1.0, 0.5);

    let oid = doc.visible_object_ids()[0];
    let obj = doc.object(oid).unwrap();
    let face = obj.faces().keys().next().unwrap();
    let result = doc.apply_object_op(
        oid,
        KernelOp::PushPull {
            face,
            distance: -2.543907517692129,
        },
    );
    eprintln!(
        "pushpull result: {:?}",
        result.as_ref().map(|_| "ok").map_err(|e| format!("{e}"))
    );
    for o in doc.visible_object_ids() {
        doc.object(o).unwrap().validate().expect("valid object");
    }

    let bytes = doc.save();
    let reloaded = Document::load(&bytes).expect("load succeeds");
    if reloaded.state_hash() != doc.state_hash() {
        if let Ok(dir) = std::env::var("DUMP_DIR") {
            std::fs::write(format!("{dir}/orig.hew"), &bytes).unwrap();
            std::fs::write(format!("{dir}/reloaded.hew"), reloaded.save()).unwrap();
        }
        panic!("state hash changed across save/load (set DUMP_DIR to dump)");
    }
}
