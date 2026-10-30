//! `hew.query.measure` by id or occurrence path (docs/agents/HEW_API.md
//! §7.4): volume, area, bounding box, and extents of a part, group, or
//! instance.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId};
use kernel::{
    Document, EntityRef, ImportNode, ImportScene, MeshRecipe, NodeId, ObjectId, Plane, Point3,
    Transform, Vec3,
};
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

fn extrude_box(doc: &mut Document, x0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).unwrap();
    let c = [
        Point3::new(x0, 0.0, 0.0),
        Point3::new(x1, 0.0, 0.0),
        Point3::new(x1, y1, 0.0),
        Point3::new(x0, y1, 0.0),
    ];
    for k in 0..4 {
        sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], h).unwrap().0
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

fn call(doc: &mut Document, method: &str, params: Value) -> Value {
    let mut conn = Connection::new(Grant::core(), "measure");
    for (id, (m, p)) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
        (method, params),
    ]
    .into_iter()
    .enumerate()
    {
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(id as i64)),
            method: m.to_string(),
            params: Some(p),
        };
        let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, &mut NoHost, req) else {
            panic!("{m} replies");
        };
        if m == method {
            return serde_json::to_value(r).unwrap();
        }
    }
    unreachable!()
}

fn measure(doc: &mut Document, params: Value) -> Value {
    let r = call(doc, "hew.query.measure", params);
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

fn refusal(doc: &mut Document, params: Value) -> String {
    let r = call(doc, "hew.query.measure", params);
    let e = r
        .get("error")
        .unwrap_or_else(|| panic!("should refuse: {r}"));
    if e["code"] == json!(-32602) {
        return "invalid_params".into();
    }
    e["data"]["refusal"].as_str().unwrap().to_string()
}

fn near(a: &Value, b: f64) {
    assert!((a.as_f64().unwrap() - b).abs() < 1e-12, "{a} vs {b}");
}

fn near_vec(a: &Value, b: [f64; 3]) {
    for k in 0..3 {
        near(&a[k], b[k]);
    }
}

#[test]
fn unit_cube_by_id() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let id = public(&doc, EntityRef::Object(b));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 1.0);
    near(&m["area"], 6.0);
    near_vec(&m["bbox"]["min"], [0.0, 0.0, 0.0]);
    near_vec(&m["bbox"]["max"], [1.0, 1.0, 1.0]);
    near_vec(&m["extents"], [1.0, 1.0, 1.0]);
    assert_eq!(m["leaf_count"], json!(1));
    assert_eq!(m["open_leaves"], json!(0));
}

#[test]
fn rotated_board_volume_and_area_are_pose_invariant() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.018);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let rot = Transform::rotation(Vec3::new(1.0, 1.0, 0.3), 0.7).unwrap();
    doc.transform_instance(inst, &rot).unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 0.4 * 0.2 * 0.018);
    let area = 2.0 * (0.4 * 0.2 + 0.4 * 0.018 + 0.2 * 0.018);
    near(&m["area"], area);
}

#[test]
fn scaled_instance_scales_volume_by_determinant() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 3.0, 4.0)))
        .unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 24.0);
    near(&m["area"], 2.0 * (6.0 + 8.0 + 12.0));
    near_vec(&m["extents"], [2.0, 3.0, 4.0]);
}

#[test]
fn mirrored_instance_has_positive_volume() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let (def, _, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let (inst, _) = doc
        .place_instance(def, Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
        .unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 1.0);
    near(&m["area"], 6.0);
    near_vec(&m["bbox"]["min"], [-1.0, 0.0, 0.0]);
}

#[test]
fn group_sums_members() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 3.0, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let id = public(&doc, EntityRef::Group(g));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 2.0);
    near(&m["area"], 12.0);
    near_vec(&m["extents"], [3.0, 1.0, 1.0]);
    assert_eq!(m["leaf_count"], json!(2));
}

#[test]
fn instance_of_two_member_component_sums() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 3.0, 1.0, 1.0);
    let (_, inst, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 2.0);
    assert_eq!(m["leaf_count"], json!(2));
}

#[test]
fn nested_path_matches_world_composition() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let (_, inner, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    doc.transform_instance(inner, &Transform::translation(Vec3::new(1.0, 0.0, 0.0)))
        .unwrap();
    let (_, outer, _) = doc.make_component(&[NodeId::Instance(inner)]).unwrap();
    doc.transform_instance(outer, &Transform::translation(Vec3::new(0.0, 5.0, 0.0)))
        .unwrap();
    let path = json!([
        public(&doc, EntityRef::Instance(outer)),
        public(&doc, EntityRef::Instance(inner)),
    ]);
    let m = measure(&mut doc, json!({ "path": path }));
    near(&m["volume"], 1.0);
    near_vec(&m["bbox"]["min"], [1.0, 5.0, 0.0]);
    near_vec(&m["bbox"]["max"], [2.0, 6.0, 1.0]);
    near_vec(&m["extents"], [1.0, 1.0, 1.0]);
    // A member alone is reachable by path, not by id.
    let member = public(&doc, EntityRef::Object(b));
    assert_eq!(
        refusal(&mut doc, json!({ "id": member })),
        "unknown_occurrence"
    );
}

fn open_box_scene() -> ImportScene {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces.len();
    let recipe = MeshRecipe {
        name: "Open".into(),
        positions: vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ],
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(recipe)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    }
}

#[test]
fn open_object_reports_null_volume_and_counts_it() {
    let mut doc = Document::new();
    doc.ingest(open_box_scene(), Vec::new()).unwrap();
    let oid = doc.visible_object_ids()[0];
    let id = public(&doc, EntityRef::Object(oid));
    let m = measure(&mut doc, json!({ "id": id }));
    assert_eq!(m["volume"], json!(null));
    assert_eq!(m["open_leaves"], json!(1));
    near(&m["area"], 5.0);
}

#[test]
fn legacy_from_to_unchanged() {
    let mut doc = Document::new();
    let m = measure(
        &mut doc,
        json!({ "from": [0.0, 0.0, 0.0], "to": [3.0, 4.0, 0.0] }),
    );
    near(&m["distance"], 5.0);
    assert_eq!(m["delta"], json!([3.0, 4.0, 0.0]));
}

#[test]
fn invalid_combinations_are_invalid_params() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let id = public(&doc, EntityRef::Object(b));
    for p in [
        json!({}),
        json!({ "id": id, "path": [id] }),
        json!({ "id": id, "from": [0, 0, 0], "to": [1, 0, 0] }),
        json!({ "from": [0, 0, 0] }),
        json!({ "to": [0, 0, 0] }),
    ] {
        assert_eq!(refusal(&mut doc, p.clone()), "invalid_params", "{p}");
    }
}

#[test]
fn unknown_id_is_unknown_entity() {
    let mut doc = Document::new();
    assert_eq!(
        refusal(&mut doc, json!({ "id": "obj_nope" })),
        "unknown_entity"
    );
}

/// A 2 x 2 x 1 block with a 1 x 1 hole through it: the top and bottom
/// faces each carry an inner loop. Volume 3, area 18.
fn holed_block_scene() -> ImportScene {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(2.0, 0.0, 1.0),
        p(2.0, 2.0, 1.0),
        p(0.0, 2.0, 1.0),
        p(0.5, 0.5, 0.0),
        p(1.5, 0.5, 0.0),
        p(1.5, 1.5, 0.0),
        p(0.5, 1.5, 0.0),
        p(0.5, 0.5, 1.0),
        p(1.5, 0.5, 1.0),
        p(1.5, 1.5, 1.0),
        p(0.5, 1.5, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
        vec![8, 12, 13, 9],
        vec![9, 13, 14, 10],
        vec![10, 14, 15, 11],
        vec![11, 15, 12, 8],
    ];
    let n = faces.len();
    let mut face_holes = vec![Vec::new(); n];
    face_holes[0] = vec![vec![8, 9, 10, 11]];
    face_holes[1] = vec![vec![12, 15, 14, 13]];
    let recipe = MeshRecipe {
        name: "Holed".into(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes,
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    };
    ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: vec![ImportNode::Mesh(recipe)],
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    }
}

#[test]
fn holed_faces_subtract_their_holes_through_a_pose() {
    let mut doc = Document::new();
    doc.ingest(holed_block_scene(), Vec::new()).unwrap();
    let oid = doc.visible_object_ids()[0];
    let (_, inst, _) = doc.make_component(&[NodeId::Object(oid)]).unwrap();
    doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 1.0, 1.0)))
        .unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let m = measure(&mut doc, json!({ "id": id }));
    near(&m["volume"], 6.0);
    // Outer sides 2*(2*1) + 2*(4*1), caps 2*(8 - 2), tunnel 2*(2*1) + 2*(1*1).
    near(&m["area"], 4.0 + 8.0 + 12.0 + 4.0 + 2.0);
}

/// A doubling ladder past the occurrence cap.
fn ladder() -> (Document, String) {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    for _ in 0..17 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    let id = public(&doc, EntityRef::Instance(inst));
    (doc, id)
}

#[test]
fn past_the_cap_measure_refuses_too_many_occurrences() {
    let (mut doc, id) = ladder();
    assert_eq!(
        refusal(&mut doc, json!({ "id": id })),
        "too_many_occurrences"
    );
}
