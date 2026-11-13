//! Model writes for parts (docs/agents/HEW_API.md §7.4; docs/agents/PLUGINS.md
//! "Setting axes"): `hew.component.rebase`, `hew.component.flip_instance`,
//! `hew.entity.set_frame`, and `hew.entity.move`'s grid copy — their
//! params rules, refusals, and the one-undo-entry contract.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId};
use kernel::{Document, EntityRef, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
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
    let mut conn = Connection::new(Grant::core(), "writes");
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

fn ok(doc: &mut Document, method: &str, params: Value) -> Value {
    let r = call(doc, method, params);
    assert!(r.get("error").is_none(), "{method}: {r}");
    // A model-mutating command answers inside the transaction envelope.
    r["result"]["results"][0].clone()
}

fn refusal(doc: &mut Document, method: &str, params: Value) -> String {
    let r = call(doc, method, params);
    r["error"]["data"]["refusal"]
        .as_str()
        .unwrap_or_else(|| panic!("{method} refused: {r}"))
        .to_string()
}

fn params_error(doc: &mut Document, method: &str, params: Value) {
    let r = call(doc, method, params.clone());
    assert_eq!(r["error"]["code"], -32602, "{method} {params}: {r}");
}

fn lowest_vertex(doc: &Document, id: &str) -> Point3 {
    let resolver = api::ids::IdResolver::new(doc);
    let Some(EntityRef::Object(o)) = resolver.resolve(id) else {
        panic!("{id} is an object")
    };
    doc.object(o)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position)
        .fold(Point3::new(f64::MAX, f64::MAX, f64::MAX), |a, p| {
            Point3::new(a.x.min(p.x), a.y.min(p.y), a.z.min(p.z))
        })
}

#[test]
fn a_grid_copy_fills_every_slot_but_the_original_in_index_order() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.5, 0.5, 0.5);
    let id = public(&doc, EntityRef::Object(b));
    let depth = doc.undo_depth();
    let r = ok(
        &mut doc,
        "hew.entity.move",
        json!({ "ids": [id], "copy": { "grid": [
            { "count": 2, "step": [1.0, 0.0, 0.0] },
            { "count": 3, "step": [0.0, 1.0, 0.0] }
        ] } }),
    );
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    let ids: Vec<String> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let want = [(0.0, 1.0), (0.0, 2.0), (1.0, 0.0), (1.0, 1.0), (1.0, 2.0)];
    assert_eq!(ids.len(), want.len());
    for (id, (x, y)) in ids.iter().zip(want) {
        assert_eq!(lowest_vertex(&doc, id), Point3::new(x, y, 0.0), "{id}");
    }
    doc.undo().unwrap();
    assert_eq!(doc.visible_object_ids().len(), 1);
}

#[test]
fn a_grid_copy_runs_placement_major_over_several_ids() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.5, 0.5, 0.5);
    let b = extrude_box(&mut doc, 2.0, 2.5, 0.5, 0.5);
    let ids = [
        public(&doc, EntityRef::Object(a)),
        public(&doc, EntityRef::Object(b)),
    ];
    let r = ok(
        &mut doc,
        "hew.entity.move",
        json!({ "ids": ids, "copy": { "grid": [{ "count": 3, "step": [0.0, 0.0, 1.0] }] } }),
    );
    let got: Vec<Point3> = r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| lowest_vertex(&doc, v.as_str().unwrap()))
        .collect();
    assert_eq!(
        got,
        vec![
            Point3::new(0.0, 0.0, 1.0),
            Point3::new(2.0, 0.0, 1.0),
            Point3::new(0.0, 0.0, 2.0),
            Point3::new(2.0, 0.0, 2.0),
        ]
    );
}

/// `grid: [{count: 4, step}]` counts the original's slot; `copy.count: 3`
/// counts copies. Both make three copies at the same places.
#[test]
fn a_one_axis_grid_and_copy_count_make_the_same_copies() {
    let positions = |copy: Value, translation: Option<[f64; 3]>| -> Vec<Point3> {
        let mut doc = Document::new();
        let b = extrude_box(&mut doc, 0.0, 0.5, 0.5, 0.5);
        let mut params = json!({ "ids": [public(&doc, EntityRef::Object(b))], "copy": copy });
        if let Some(t) = translation {
            params["translation"] = json!(t);
        }
        let r = ok(&mut doc, "hew.entity.move", params);
        r["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| lowest_vertex(&doc, v.as_str().unwrap()))
            .collect()
    };
    let grid = positions(
        json!({ "grid": [{ "count": 4, "step": [1.0, 0.0, 0.0] }] }),
        None,
    );
    let count = positions(json!({ "count": 3 }), Some([1.0, 0.0, 0.0]));
    assert_eq!(grid.len(), 3);
    assert_eq!(grid, count);
}

#[test]
fn grid_params_rules() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.5, 0.5, 0.5);
    let id = public(&doc, EntityRef::Object(b));
    let axis = json!({ "count": 2, "step": [1.0, 0.0, 0.0] });
    let bad = [
        json!({ "ids": [id], "copy": { "count": 2, "grid": [axis] } }),
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": { "grid": [axis] } }),
        json!({ "ids": [id], "from": [0.0, 0.0, 0.0], "to": [1.0, 0.0, 0.0], "copy": { "grid": [axis] } }),
        json!({ "ids": [id], "copy": { "grid": [] } }),
        json!({ "ids": [id], "copy": { "grid": [axis, axis, axis, axis] } }),
        json!({ "ids": [id], "copy": { "grid": [{ "count": 0, "step": [1.0, 0.0, 0.0] }] } }),
        json!({ "ids": [id], "copy": { "grid": [{ "count": 1, "step": [1.0, 0.0, 0.0] }] } }),
        json!({ "ids": [id], "copy": { "grid": [{ "count": 2, "step": [1.0, 0.0] }] } }),
    ];
    let depth = doc.undo_depth();
    for params in bad {
        params_error(&mut doc, "hew.entity.move", params);
    }
    params_error(
        &mut doc,
        "hew.entity.rotate",
        json!({ "ids": [id], "pivot": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": 0.5, "copy": { "grid": [axis] } }),
    );
    // The original's slot counts toward the cap: 10 × 10 × 11 = 1100.
    let r = call(
        &mut doc,
        "hew.entity.move",
        json!({ "ids": [id], "copy": { "grid": [
            { "count": 10, "step": [1.0, 0.0, 0.0] },
            { "count": 10, "step": [0.0, 1.0, 0.0] },
            { "count": 11, "step": [0.0, 0.0, 1.0] }
        ] } }),
    );
    assert_eq!(r["error"]["data"]["refusal"], "array_count_too_large");
    assert_eq!(r["error"]["data"]["detail"]["max"], 1000);
    assert_eq!(doc.undo_depth(), depth, "nothing ran");
    // 10 × 10 × 10 is exactly the cap: 999 copies.
    let r = ok(
        &mut doc,
        "hew.entity.move",
        json!({ "ids": [id], "copy": { "grid": [
            { "count": 10, "step": [1.0, 0.0, 0.0] },
            { "count": 10, "step": [0.0, 1.0, 0.0] },
            { "count": 10, "step": [0.0, 0.0, 1.0] }
        ] } }),
    );
    assert_eq!(r["ids"].as_array().unwrap().len(), 999);
}

fn part_frame(doc: &mut Document, id: &str) -> Value {
    let r = call(doc, "hew.query.part_frame", json!({ "id": id }));
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

#[test]
fn set_frame_writes_and_clears_the_override() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.018);
    let id = public(&doc, EntityRef::Object(b));
    let face = part_frame(&mut doc, &id);
    assert_eq!(face["source"], "face");
    let depth = doc.undo_depth();
    // Grain across the width: x and y trade places in the face frame.
    let r = ok(
        &mut doc,
        "hew.entity.set_frame",
        json!({ "id": id, "origin": [0.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [1.0, 0.0, 0.0] }),
    );
    assert_eq!(r, json!({ "id": id, "source": "attribute" }));
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    let set = part_frame(&mut doc, &id);
    assert_eq!(set["source"], "attribute");
    assert_eq!(set["x"], face["y"]);
    assert_eq!(set["y"], face["x"]);
    let r = ok(
        &mut doc,
        "hew.entity.set_frame",
        json!({ "id": id, "clear": true }),
    );
    assert_eq!(r, json!({ "id": id, "source": "face" }));
    assert_eq!(part_frame(&mut doc, &id), {
        let mut f = face.clone();
        f["generation"] = json!(doc.history_generation());
        f["epoch"] = json!(doc.document_epoch());
        f
    });
    // Clearing an absent override is not an error and records nothing.
    let depth = doc.undo_depth();
    let r = ok(
        &mut doc,
        "hew.entity.set_frame",
        json!({ "id": id, "clear": true }),
    );
    assert_eq!(r["source"], "face");
    assert_eq!(doc.undo_depth(), depth);
}

/// Echoing the reported frame back changes nothing, even where the
/// reported frame is the face frame after rule 3's swap.
#[test]
fn set_frame_is_relative_to_the_reported_face_frame() {
    let mut doc = Document::new();
    // Longer along y than x: rule 3 swaps the face frame.
    let b = extrude_box(&mut doc, 0.0, 0.2, 0.6, 0.018);
    let id = public(&doc, EntityRef::Object(b));
    let face = part_frame(&mut doc, &id);
    ok(
        &mut doc,
        "hew.entity.set_frame",
        json!({ "id": id, "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
    );
    let set = part_frame(&mut doc, &id);
    assert_eq!(set["source"], "attribute");
    for k in ["origin", "x", "y", "z", "extents"] {
        let (a, b) = (set[k].as_array().unwrap(), face[k].as_array().unwrap());
        for (p, q) in a.iter().zip(b) {
            assert!(
                (p.as_f64().unwrap() - q.as_f64().unwrap()).abs() < 1e-12,
                "{k}"
            );
        }
    }
}

#[test]
fn set_frame_refusals() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.018);
    let id = public(&doc, EntityRef::Object(b));
    let (c, i, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let ins = public(&doc, EntityRef::Instance(i));
    let cmp = public(&doc, EntityRef::Component(c));
    let frame = json!({ "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] });
    for target in [&ins, &cmp] {
        let mut p = frame.clone();
        p["id"] = json!(target);
        assert_eq!(refusal(&mut doc, "hew.entity.set_frame", p), "not_a_part");
    }
    let mut p = frame.clone();
    p["id"] = json!("obj_ffffff");
    assert_eq!(
        refusal(&mut doc, "hew.entity.set_frame", p),
        "unknown_entity"
    );
    let g = extrude_box(&mut doc, 2.0, 2.4, 0.2, 0.018);
    let gid = public(&doc, EntityRef::Object(g));
    let depth = doc.undo_depth();
    assert_eq!(
        refusal(
            &mut doc,
            "hew.entity.set_frame",
            json!({ "id": gid, "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.5, 0.5, 0.0] }),
        ),
        "invalid_frame"
    );
    for p in [
        json!({ "id": gid }),
        json!({ "id": gid, "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
        json!({ "id": gid, "clear": true, "origin": [0.0, 0.0, 0.0] }),
        json!({ "id": gid, "clear": false }),
    ] {
        params_error(&mut doc, "hew.entity.set_frame", p);
    }
    assert_eq!(doc.undo_depth(), depth);
    let _ = id;
}

#[test]
fn set_frame_takes_a_group() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.018);
    let (g, _) = doc.group_nodes(&[NodeId::Object(a)]).unwrap();
    let id = public(&doc, EntityRef::Group(g));
    ok(
        &mut doc,
        "hew.entity.set_frame",
        json!({ "id": id, "origin": [0.01, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [1.0, 0.0, 0.0] }),
    );
    assert_eq!(part_frame(&mut doc, &id)["source"], "attribute");
}

/// One board placed twice, the second turned a quarter about z.
fn placed_twice() -> (
    Document,
    kernel::ComponentId,
    kernel::InstanceId,
    kernel::InstanceId,
) {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.018);
    let (c, i1, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(2.0, 0.0, 0.0)));
    let (i2, _) = doc.place_instance(c, turn).unwrap();
    (doc, c, i1, i2)
}

#[test]
fn rebase_by_instance_compensates_every_placement() {
    let (mut doc, c, i1, i2) = placed_twice();
    let poses: Vec<Transform> = [i1, i2]
        .iter()
        .map(|&i| doc.instance_pose(i).unwrap())
        .collect();
    let world = |doc: &Document| -> Vec<Point3> {
        doc.expanded_placements()
            .into_iter()
            .flat_map(|(o, p, _)| {
                doc.object(o)
                    .unwrap()
                    .vertices()
                    .values()
                    .map(move |v| p.apply_point(v.position))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let before = world(&doc);
    let ins = public(&doc, EntityRef::Instance(i2));
    let depth = doc.undo_depth();
    let r = ok(
        &mut doc,
        "hew.component.rebase",
        json!({ "id": ins, "origin": [0.0, 0.0, 0.0], "x": [0.0, 1.0, 0.0], "y": [-1.0, 0.0, 0.0] }),
    );
    assert_eq!(
        r,
        json!({ "component": public(&doc, EntityRef::Component(c)), "instances_updated": 2 })
    );
    assert_eq!(doc.undo_depth(), depth + 1);
    for (p, q) in before.iter().zip(world(&doc)) {
        assert!((*p - q).length() < 1e-9);
    }
    for (k, &i) in [i1, i2].iter().enumerate() {
        assert_ne!(doc.instance_pose(i).unwrap(), poses[k]);
    }
}

#[test]
fn rebase_refusals() {
    let (mut doc, c, i1, _) = placed_twice();
    let cmp = public(&doc, EntityRef::Component(c));
    assert_eq!(
        refusal(
            &mut doc,
            "hew.component.rebase",
            json!({ "id": cmp, "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [1.0, 0.0, 0.0] }),
        ),
        "invalid_frame"
    );
    assert_eq!(
        refusal(
            &mut doc,
            "hew.component.rebase",
            json!({ "id": "cmp_ffffff", "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
        ),
        "unknown_entity"
    );
    doc.open_explode_session(i1).unwrap();
    assert_eq!(
        refusal(
            &mut doc,
            "hew.component.rebase",
            json!({ "id": cmp, "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0] }),
        ),
        "rebase_during_session"
    );
}

#[test]
fn flip_instance_mirrors_one_world_placement() {
    let (mut doc, _c, i1, _) = placed_twice();
    let ins = public(&doc, EntityRef::Instance(i1));
    let depth = doc.undo_depth();
    let r = ok(
        &mut doc,
        "hew.component.flip_instance",
        json!({ "instance": ins, "axis": "x" }),
    );
    assert_eq!(r["instance"], json!(ins));
    assert_eq!(r["mirrored"], true);
    assert_eq!(
        r["pose"],
        json!([-1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(doc.undo_depth(), depth + 1);
    let r = ok(
        &mut doc,
        "hew.component.flip_instance",
        json!({ "instance": ins, "axis": "x" }),
    );
    assert_eq!(r["mirrored"], false);
    params_error(
        &mut doc,
        "hew.component.flip_instance",
        json!({ "instance": ins, "axis": "w" }),
    );
}

#[test]
fn flip_instance_refuses_a_nested_instance() {
    let (mut doc, _c, i1, _) = placed_twice();
    doc.make_component(&[NodeId::Instance(i1)]).unwrap();
    let ins = public(&doc, EntityRef::Instance(i1));
    assert_eq!(
        refusal(
            &mut doc,
            "hew.component.flip_instance",
            json!({ "instance": ins, "axis": "y" }),
        ),
        "unknown_entity"
    );
}
