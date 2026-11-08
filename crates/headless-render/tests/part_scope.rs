//! `scope: "own"` and `orient: "part"` (docs/agents/PLUGINS.md "Reading a
//! container's own Objects") through the real renderers, dispatched on the
//! bus the way `hew-cli` serves them.

use api::{
    Connection, DispatchOutcome, Grant, Host, LineDrawingParams, LineDrawingResult, Refusal,
    Request, RequestId, SnapshotParams, SnapshotResult,
};
use kernel::{Document, EntityRef, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// The software renderers, as a headless host serves them.
struct Render;

impl Host for Render {
    fn snapshot(&mut self, doc: &Document, p: &SnapshotParams) -> Result<SnapshotResult, Refusal> {
        headless_render::snapshot(doc, p)
    }
    fn line_drawing(
        &mut self,
        doc: &Document,
        p: &LineDrawingParams,
    ) -> Result<LineDrawingResult, Refusal> {
        headless_render::line_drawing(doc, p)
    }
}

fn cuboid(doc: &mut Document, at: [f64; 3], size: [f64; 3]) -> ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, at[2]),
        Point3::new(1.0, 0.0, at[2]),
        Point3::new(0.0, 1.0, at[2]),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    let sk = doc.sketch_mut(s).unwrap();
    let c = [
        Point3::new(at[0], at[1], at[2]),
        Point3::new(at[0] + size[0], at[1], at[2]),
        Point3::new(at[0] + size[0], at[1] + size[1], at[2]),
        Point3::new(at[0], at[1] + size[1], at[2]),
    ];
    for k in 0..4 {
        sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], size[2]).unwrap().0
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

fn ok(doc: &mut Document, method: &str, params: Value) -> Value {
    let mut conn = Connection::new(Grant::core(), "render");
    let mut host = Render;
    let mut last = Value::Null;
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
        let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, &mut host, req) else {
            panic!("{m} replies");
        };
        last = serde_json::to_value(r).unwrap();
    }
    assert!(last.get("error").is_none(), "{method}: {last}");
    last["result"].clone()
}

/// A Door group: a Panel object and a placement of a Plate + Pin hinge.
fn door() -> (Document, String, [ObjectId; 3]) {
    let mut doc = Document::new();
    let panel = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let plate = cuboid(&mut doc, [0.3, 0.05, 0.018], [0.04, 0.03, 0.002]);
    let pin = cuboid(&mut doc, [0.3, 0.05, 0.02], [0.01, 0.01, 0.03]);
    let (_, hinge, _) = doc
        .make_component(&[NodeId::Object(plate), NodeId::Object(pin)])
        .unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(panel), NodeId::Instance(hinge)])
        .unwrap();
    let id = public(&doc, EntityRef::Group(g));
    (doc, id, [panel, plate, pin])
}

#[test]
fn snapshot_scope_own_renders_only_own_leaves() {
    let (mut doc, door, [panel, ..]) = door();
    let own = ok(
        &mut doc,
        "hew.view.snapshot",
        json!({ "selection": [door], "scope": "own", "include_ids": true, "width": 96, "height": 96 }),
    );
    assert_eq!(
        own["id_palette"],
        json!([public(&doc, EntityRef::Object(panel))])
    );
    let all = ok(
        &mut doc,
        "hew.view.snapshot",
        json!({ "selection": [door], "include_ids": true, "width": 96, "height": 96 }),
    );
    assert_ne!(all["id_palette"], own["id_palette"]);
}

#[test]
fn line_drawing_scope_own_counts_only_own_edges() {
    let (mut doc, door, _) = door();
    let own = ok(
        &mut doc,
        "hew.view.line_drawing",
        json!({ "selection": [door], "scope": "own", "format": "segments", "view": "iso" }),
    );
    let all = ok(
        &mut doc,
        "hew.view.line_drawing",
        json!({ "selection": [door], "format": "segments", "view": "iso" }),
    );
    assert!(
        own["count"].as_u64().unwrap() < all["count"].as_u64().unwrap(),
        "{} vs {}",
        own["count"],
        all["count"]
    );
}

/// The covered pixels' bounding box.
fn pixel_box(r: &Value) -> [usize; 4] {
    let bytes = decode(r["id_buffer_base64"].as_str().unwrap());
    let w = r["width"].as_u64().unwrap() as usize;
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    for (k, px) in bytes.chunks_exact(2).enumerate() {
        if u16::from_le_bytes([px[0], px[1]]) != 0 {
            let (x, y) = (k % w, k / w);
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
    }
    b
}

fn decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        let n = chunk.iter().fold(0u32, |acc, &c| (acc << 6) | val(c));
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        out.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
    }
    out
}

#[test]
fn snapshot_orient_part_front_looks_along_the_frame_y() {
    let mut boxes = Vec::new();
    for yaw in [0.0_f64, 0.6, 1.9, 4.0] {
        let mut doc = Document::new();
        let b = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
        let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), yaw)
            .unwrap()
            .then(&Transform::translation(Vec3::new(1.0, 2.0, 0.0)));
        doc.transform_object(b, &turn).unwrap();
        let id = public(&doc, EntityRef::Object(b));
        let r = ok(
            &mut doc,
            "hew.view.snapshot",
            json!({ "selection": [id], "orient": "part", "view": "front",
                    "include_ids": true, "width": 128, "height": 128 }),
        );
        boxes.push(pixel_box(&r));
    }
    for b in &boxes {
        assert_drawn_and_centered(*b, 128);
    }
    for b in &boxes[1..] {
        for k in 0..4 {
            assert!(b[k].abs_diff(boxes[0][k]) <= 1, "{b:?} vs {:?}", boxes[0]);
        }
    }
}

/// The box holds covered pixels (the empty box is the `usize::MAX`
/// sentinel) and sits around the image center.
fn assert_drawn_and_centered(b: [usize; 4], size: usize) {
    assert!(b[0] <= b[2] && b[1] <= b[3], "nothing was drawn: {b:?}");
    let half = (size as f64 - 1.0) / 2.0;
    let (cx, cy) = (
        (b[0] + b[2]) as f64 / 2.0 - half,
        (b[1] + b[3]) as f64 / 2.0 - half,
    );
    let tol = size as f64 * 0.06;
    assert!(cx.abs() <= tol && cy.abs() <= tol, "off-center {b:?}");
}

fn part_box(doc: &mut Document, id: &str) -> [usize; 4] {
    let r = ok(
        doc,
        "hew.view.snapshot",
        json!({ "selection": [id], "orient": "part", "view": "front",
                "include_ids": true, "width": 128, "height": 128 }),
    );
    pixel_box(&r)
}

#[test]
fn orient_part_centers_a_placement_whose_definition_is_off_the_origin() {
    let mut doc = Document::new();
    let plate = cuboid(&mut doc, [0.3, 0.05, 0.018], [0.04, 0.03, 0.002]);
    let pin = cuboid(&mut doc, [0.3, 0.05, 0.02], [0.01, 0.01, 0.03]);
    let (_, hinge, _) = doc
        .make_component(&[NodeId::Object(plate), NodeId::Object(pin)])
        .unwrap();
    let id = public(&doc, EntityRef::Instance(hinge));
    assert_drawn_and_centered(part_box(&mut doc, &id), 128);
    // The same placement mirrored (a negative-determinant pose).
    ok(
        &mut doc,
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "x", "about": "origin" }),
    );
    assert_drawn_and_centered(part_box(&mut doc, &id), 128);
    // And turned and moved.
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(2.0, -1.0, 0.5)));
    doc.transform_instance(hinge, &turn).unwrap();
    assert_drawn_and_centered(part_box(&mut doc, &id), 128);
}

#[test]
fn orient_part_centers_an_override_frame_whose_origin_is_mid_box() {
    use kernel::{AttrTarget, AttrValue};
    let mut doc = Document::new();
    let b = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let list = |v: [f64; 3]| AttrValue::List(v.iter().map(|&c| AttrValue::Float(c)).collect());
    let mut m = std::collections::BTreeMap::new();
    m.insert("origin".to_string(), list([0.2, 0.1, 0.009]));
    m.insert("x".to_string(), list([1.0, 0.0, 0.0]));
    m.insert("y".to_string(), list([0.0, 1.0, 0.0]));
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(b)),
        api::part_frame::FRAME_NAMESPACE,
        api::part_frame::FRAME_KEY,
        AttrValue::Map(m),
    )
    .unwrap();
    let id = public(&doc, EntityRef::Object(b));
    assert_drawn_and_centered(part_box(&mut doc, &id), 128);
}

#[test]
fn orient_part_with_scope_own_and_nothing_own_is_a_params_error() {
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.1, 0.1, 0.1]);
    let (_, inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (g, _) = doc.group_nodes(&[NodeId::Instance(inst)]).unwrap();
    let id = public(&doc, EntityRef::Group(g));
    let mut conn = Connection::new(Grant::core(), "render");
    let mut host = Render;
    let mut reply = Value::Null;
    for (n, (m, p)) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
        (
            "hew.view.snapshot",
            json!({ "selection": [id], "scope": "own", "orient": "part" }),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(n as i64)),
            method: m.to_string(),
            params: Some(p),
        };
        let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, &mut doc, &mut host, req) else {
            panic!("replies");
        };
        reply = serde_json::to_value(r).unwrap();
    }
    assert_eq!(reply["error"]["code"], -32602, "{reply}");
}
