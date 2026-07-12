//! Regression repro distilled from `document_fuzz.rs`: a boolean union
//! whose operand carries a concave (staple) imprint drove the coplanar
//! seam-dissolution merges into shared-chain configurations do_merge_faces
//! was never taught to represent — a chain covering a face's entire outer
//! boundary, chain endpoints whose neighbor side is not one consecutive
//! loop, and outer loops the chain/non-shared walks do not fully cover.
//! The surgery indexed dead slotmap keys and PANICKED mid-merge instead of
//! refusing typed; the corruption could also slip past surgery into the
//! debug invariant check.
//!
//! Each unsupported configuration is now detected and refused typed
//! (rule 4) — SharedChainCoversBoundary for whole-boundary chains, a
//! pre-mutation coverage check and a checked boundary re-walk for the
//! rest — so the boolean simply leaves those seams undissolved and returns
//! a valid solid.
use kernel::{BooleanOp, Document, KernelOp, ObjectId, Plane, Point3, Vec3};

fn add_box(doc: &mut Document, x: f64, y: f64, dx: f64, dy: f64, h: f64) -> ObjectId {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let s = doc.add_sketch(plane);
    doc.begin_sketch_gesture(s).unwrap();
    {
        let sk = doc.sketch_mut(s).unwrap();
        let p = [
            Point3::new(x, y, 0.0),
            Point3::new(x + dx, y, 0.0),
            Point3::new(x + dx, y + dy, 0.0),
            Point3::new(x, y + dy, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(p[k], p[(k + 1) % 4]).unwrap();
        }
    }
    doc.end_sketch_gesture(s).unwrap();
    let region = doc.sketch(s).unwrap().regions().keys().next().unwrap();
    let (oid, _) = doc.extrude_region(s, region, h).unwrap();
    oid
}

#[test]
fn union_with_concave_imprint_never_panics() {
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    add_box(
        &mut doc,
        -5.939857644774745,
        3.0947633010945186,
        2.5953712334649306,
        5.0746482401587265,
        0.5,
    );
    add_box(
        &mut doc,
        -5.044345096402345,
        -0.7636032013793194,
        1.0,
        5.802521990658308,
        0.5,
    );
    add_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 0.5);

    // Staple imprint (as the fuzz resolves it).
    let ids = doc.visible_object_ids();
    let oid = ids[6699577644586091478usize % ids.len()];
    let obj = doc.object(oid).unwrap();
    let face = obj
        .faces()
        .keys()
        .nth(1003484089025972364usize % obj.faces().len())
        .unwrap();
    let boundary: Vec<Point3> = obj.loop_positions(obj.faces()[face].outer_loop).collect();
    assert_eq!(boundary.len(), 4);
    let (o, ua, vb) = (boundary[0], boundary[1], boundary[3]);
    let at = |a: f64, b: f64| o + (ua - o) * a + (vb - o) * b;
    let loop_path = vec![
        at(0.2, 0.6),
        at(0.4, 0.6),
        at(0.4, 0.8),
        at(0.6, 0.8),
        at(0.6, 0.6),
        at(0.8, 0.6),
        at(0.8, 0.9),
        at(0.2, 0.9),
    ];
    doc.apply_object_op(
        oid,
        KernelOp::SplitFaceInner {
            face,
            loop_path,
            restore: None,
            curve: None,
        },
    )
    .expect("staple imprints");

    // The union must complete (or refuse typed) — never panic — and every
    // visible object must validate afterwards.
    let ids = doc.visible_object_ids();
    let a = ids[15763676422592703991usize % ids.len()];
    let b = ids[10578442261613101314usize % ids.len()];
    let _ = doc.boolean(BooleanOp::Union, a, b);
    for oid in doc.visible_object_ids() {
        doc.object(oid)
            .expect("visible id resolves")
            .validate()
            .expect("valid after union");
    }
}
