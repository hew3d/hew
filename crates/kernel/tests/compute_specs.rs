//! Executable specs for the kernel pieces behind the API's pure
//! computations and dry runs (docs/agents/HEW_API.md §6 "Rehearsal" and
//! §7 `hew.compute.*`): `Object::volume` and `Object::surface_area`,
//! `Object::boolean_product` (the geometry a committed boolean leaves),
//! `Document::rehearsal_disturbance` (a commit's refusals, checked against
//! the untouched original), and `Document::subtract_solid_through_placement`
//! (a transient cutter through a placement, no record behind it).

use kernel::{
    BooleanOp, Document, DocumentError, NodeId, Object, ObjectId, Plane, Point3, Transform, Vec3,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn plane_z(z: f64) -> Plane {
    Plane::from_polygon(&[p(0.0, 0.0, z), p(1.0, 0.0, z), p(0.0, 1.0, z)]).expect("plane")
}

/// Draws `rings` (outer first, then holes) on a sketch at height `z0` and
/// extrudes the region holding the first ring's material by `h`.
fn prism(doc: &mut Document, z0: f64, rings: &[Vec<Point3>], h: f64) -> ObjectId {
    let s = doc.add_sketch(plane_z(z0));
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        for ring in rings {
            for k in 0..ring.len() {
                sk.add_segment(ring[k], ring[(k + 1) % ring.len()])
                    .expect("segment");
            }
        }
    }
    let region = doc
        .extrudable_regions(s)
        .expect("live")
        .into_iter()
        .max_by(|&a, &b| {
            let area = |r| doc.sketch(s).unwrap().region_area(r).unwrap();
            area(a).total_cmp(&area(b))
        })
        .expect("a region");
    doc.extrude_region(s, region, h).expect("extrude").0
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> Vec<Point3> {
    vec![p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)]
}

fn make_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    prism(doc, 0.0, &[rect(x0, y0, x1, y1, 0.0)], h)
}

// ---------------------------------------------------- volume and area

#[test]
fn a_unit_cube_has_volume_one_and_area_six() {
    let mut doc = Document::new();
    let o = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let obj = doc.object(o).unwrap();
    assert!((obj.volume().expect("watertight") - 1.0).abs() < 1e-12);
    assert!((obj.surface_area() - 6.0).abs() < 1e-12);
}

#[test]
fn a_through_hole_is_subtracted_from_the_volume() {
    let mut doc = Document::new();
    let o = prism(
        &mut doc,
        0.0,
        &[rect(0.0, 0.0, 2.0, 2.0, 0.0), rect(0.5, 0.5, 1.0, 1.0, 0.0)],
        1.0,
    );
    let obj = doc.object(o).unwrap();
    assert!((obj.volume().unwrap() - (4.0 - 0.25)).abs() < 1e-12);
    // Two caps of 3.75, four outer walls of 2, four hole walls of 0.5.
    assert!((obj.surface_area() - (2.0 * 3.75 + 8.0 + 2.0)).abs() < 1e-12);
}

#[test]
fn an_open_object_has_no_volume() {
    let quad = Object::from_polygons(
        &[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ],
        &[vec![0, 1, 2, 3]],
    )
    .expect("one face");
    assert_eq!(quad.volume(), None);
    assert!((quad.surface_area() - 1.0).abs() < 1e-12);
}

// ---------------------------------------------------- boolean_product

#[test]
fn boolean_product_is_what_a_committed_boolean_leaves() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = prism(&mut doc, 0.5, &[rect(0.5, 0.25, 1.5, 0.75, 0.5)], 1.0);
    let product = Object::boolean_product(
        BooleanOp::Subtract,
        doc.object(a).unwrap(),
        doc.object(b).unwrap(),
        &Transform::IDENTITY,
    )
    .expect("pure product");
    let (result, _) = doc
        .boolean(BooleanOp::Subtract, a, b)
        .expect("committed subtract");
    let committed = doc.object(result).unwrap();
    assert_eq!(product.faces().len(), committed.faces().len());
    let points = |o: &Object| -> Vec<[u64; 3]> {
        o.vertices()
            .values()
            .map(|v| {
                [
                    v.position.x.to_bits(),
                    v.position.y.to_bits(),
                    v.position.z.to_bits(),
                ]
            })
            .collect()
    };
    assert_eq!(
        points(&product),
        points(committed),
        "same vertices, same order"
    );
}

// ---------------------------------------------------- rehearsal

#[test]
fn a_rehearsal_runs_on_one_clone_and_leaves_the_original() {
    let mut doc = Document::new();
    make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (hash, generation, epoch, depth) = (
        doc.state_hash(),
        doc.history_generation(),
        doc.document_epoch(),
        doc.undo_depth(),
    );
    let mut scratch = doc.clone();
    make_box(&mut scratch, 2.0, 0.0, 3.0, 1.0, 1.0);
    assert_eq!(scratch.rehearsal_disturbance(&doc), None);
    assert_ne!(scratch.state_hash(), hash);
    drop(scratch);
    assert_eq!(doc.state_hash(), hash);
    assert_eq!(doc.history_generation(), generation);
    assert_eq!(doc.document_epoch(), epoch);
    assert_eq!(doc.undo_depth(), depth);
}

#[test]
fn a_rehearsal_reports_what_the_commit_would_refuse() {
    let mut doc = Document::new();
    make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    // An undo inside the rehearsal disturbs the history, as inside a bracket.
    let mut scratch = doc.clone();
    scratch.undo().expect("undo");
    assert_eq!(
        scratch.rehearsal_disturbance(&doc),
        Some(DocumentError::TransactionHistoryDisturbed)
    );
    // A session opened and left open is unbalanced.
    let mut scratch = doc.clone();
    let o = scratch.live_object_ids()[0];
    let (g, _) = scratch.group_nodes(&[NodeId::Object(o)]).expect("group");
    let doc_with_group = scratch.clone();
    scratch.open_group_session(g).expect("open the group");
    assert_eq!(
        scratch.rehearsal_disturbance(&doc_with_group),
        Some(DocumentError::TransactionSessionUnbalanced)
    );
}

// --------------------------------------- a transient cutter through a placement

#[test]
fn a_transient_cutter_cuts_every_placement_and_undo_restores_only_the_host() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (comp, i1, _) = doc.make_component(&[NodeId::Object(a)]).expect("component");
    let (_i2, _) = doc
        .place_instance(comp, Transform::translation(Vec3::new(5.0, 0.0, 0.0)))
        .expect("second placement");
    // The cutter is world geometry over placement `i1` and never a record.
    let mut scratch = Document::new();
    let c = prism(&mut scratch, 0.5, &[rect(0.25, 0.25, 0.75, 0.75, 0.5)], 1.0);
    let cutter = scratch.object(c).unwrap().clone();
    let objects_before = doc.live_object_ids().len();
    let before = doc.save();
    let depth = doc.undo_depth();
    let (result, change) = doc
        .subtract_solid_through_placement(&[i1], a, &cutter)
        .expect("cut");
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(change.components_touched, vec![comp]);
    assert_eq!(doc.object_owner_component(result), Some(comp));
    let vol = doc.object(result).unwrap().volume().unwrap();
    assert!(
        (vol - (1.0 - 0.125)).abs() < 1e-12,
        "a pocket 0.5 x 0.5 x 0.5"
    );
    assert_eq!(
        doc.live_object_ids().len(),
        objects_before,
        "no record for the cutter, and the host replaced"
    );
    doc.undo().expect("undo");
    assert_eq!(doc.save(), before, "undo brings back the host only");
    doc.redo().expect("redo");
    assert!(doc.object(result).is_some());
}

#[test]
fn a_transient_cutter_refuses_like_a_record_tool() {
    let mut doc = Document::new();
    let a = make_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_comp, i1, _) = doc.make_component(&[NodeId::Object(a)]).expect("component");
    let mut scratch = Document::new();
    let far = make_box(&mut scratch, 10.0, 10.0, 11.0, 11.0, 1.0);
    let far = scratch.object(far).unwrap().clone();
    let before = doc.save();
    assert!(matches!(
        doc.subtract_solid_through_placement(&[i1], a, &far),
        Err(DocumentError::Boolean(_))
    ));
    assert!(matches!(
        doc.subtract_solid_through_placement(&[], a, &far),
        Err(DocumentError::UnknownInstance)
    ));
    assert_eq!(doc.save(), before, "untouched");
}
