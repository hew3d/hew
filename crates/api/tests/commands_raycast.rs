//! `hew.query.raycast` through occurrences (docs/agents/HEW_API.md §7.4):
//! the occurrence path, leaf, and face locator of a hit, `all`,
//! `max_distance`, `exclude`, and `include_hidden`.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId};
use kernel::{Document, EntityRef, InstanceId, NodeId, ObjectId, Plane, Point3};
use kernel::{Transform, Vec3};
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

/// One dispatch on a fresh attached connection; the whole response.
fn call(doc: &mut Document, method: &str, params: Value) -> Value {
    let mut conn = Connection::new(Grant::core(), "raycast");
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
    r["result"].clone()
}

/// The refusal name, or `invalid_params` for -32602.
fn refusal(doc: &mut Document, method: &str, params: Value) -> String {
    let r = call(doc, method, params);
    let e = r
        .get("error")
        .unwrap_or_else(|| panic!("{method} should refuse: {r}"));
    if e["code"] == json!(-32602) {
        return "invalid_params".into();
    }
    e["data"]["refusal"].as_str().unwrap().to_string()
}

/// A wide floor (z 0 to 0.05); a board (0.4 x 0.2 x 0.1) inside an inner
/// component, inside an outer component, placed twice: plain, 1 m up, and
/// mirrored in x, 3 m over and 1 m up.
struct Fixture {
    doc: Document,
    floor: ObjectId,
    board: ObjectId,
    inner: InstanceId,
    outer1: InstanceId,
    outer2: InstanceId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let floor = extrude_box(&mut doc, -1.0, 4.0, 2.0, 0.05);
    let board = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.1);
    let (_, inner, _) = doc.make_component(&[NodeId::Object(board)]).unwrap();
    let (outer_def, outer1, _) = doc.make_component(&[NodeId::Instance(inner)]).unwrap();
    doc.transform_instance(outer1, &Transform::translation(Vec3::new(0.0, 0.0, 1.0)))
        .unwrap();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(3.0, 0.0, 1.0)));
    let (outer2, _) = doc.place_instance(outer_def, mirror).unwrap();
    Fixture {
        doc,
        floor,
        board,
        inner,
        outer1,
        outer2,
    }
}

fn near(a: &Value, b: f64) {
    assert!((a.as_f64().unwrap() - b).abs() < 1e-9, "{a} vs {b}");
}

#[test]
fn first_hit_reports_path_leaf_face() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0] }),
    );
    let o1 = public(&f.doc, EntityRef::Instance(f.outer1));
    let inner = public(&f.doc, EntityRef::Instance(f.inner));
    let board = public(&f.doc, EntityRef::Object(f.board));
    assert_eq!(r["path"], json!([o1, inner]));
    assert_eq!(r["leaf"], json!(board));
    assert_eq!(r["object"], json!(o1));
    assert_eq!(r["kind"], json!("instance"));
    assert_eq!(r["hidden"], json!(false));
    near(&r["distance"], 3.9);
    assert_eq!(r["face"]["object"], json!(board));
    let at = &r["face"]["at"];
    near(&at[0], 0.2);
    near(&at[1], 0.1);
    near(&at[2], 0.1);
}

#[test]
fn world_object_hit_has_empty_path() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [3.5, 1.5, 5.0], "dir": [0.0, 0.0, -1.0] }),
    );
    let floor = public(&f.doc, EntityRef::Object(f.floor));
    assert_eq!(r["path"], json!([]));
    assert_eq!(r["leaf"], json!(floor));
    assert_eq!(r["object"], json!(floor));
    assert_eq!(r["kind"], json!("object"));
    assert_eq!(r["normal"], json!([0.0, 0.0, 1.0]));
}

#[test]
fn face_feeds_resolve_and_paint() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0] }),
    );
    let resolved = ok(
        &mut f.doc,
        "hew.query.resolve",
        json!({ "face": r["face"] }),
    );
    let normal = &resolved["face"]["plane"]["normal"];
    near(&normal[0], 0.0);
    near(&normal[1], 0.0);
    near(&normal[2], 1.0);
    let red = f.doc.add_material(kernel::Material::solid(
        "Red",
        kernel::Rgba8::rgb(200, 0, 0),
    ));
    let red_id = public(&f.doc, EntityRef::Material(red));
    ok(
        &mut f.doc,
        "hew.material.paint",
        json!({ "face": r["face"], "material": red_id }),
    );
    let painted = f
        .doc
        .object(f.board)
        .unwrap()
        .faces()
        .values()
        .filter(|face| face.material == Some(red))
        .count();
    assert_eq!(painted, 1, "exactly the hit face is painted");
}

#[test]
fn all_lists_every_crossing_in_order() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "all": true }),
    );
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 4, "{r}");
    let d: Vec<f64> = hits
        .iter()
        .map(|h| h["distance"].as_f64().unwrap())
        .collect();
    assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
    near(&json!(d[0]), 3.9);
    near(&json!(d[1]), 4.0);
    near(&json!(d[2]), 4.95);
    near(&json!(d[3]), 5.0);
    assert!(r.get("raycast_hits_capped").is_none());
}

#[test]
fn all_empty_is_success() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [100.0, 100.0, 5.0], "dir": [0.0, 0.0, -1.0], "all": true }),
    );
    assert_eq!(r["hits"], json!([]));
}

#[test]
fn all_is_capped() {
    let mut doc = Document::new();
    let slab = extrude_box(&mut doc, 0.0, 1.0, 1.0, 0.005);
    let (def, _, _) = doc.make_component(&[NodeId::Object(slab)]).unwrap();
    for i in 1..600 {
        doc.place_instance(
            def,
            Transform::translation(Vec3::new(0.0, 0.0, 0.01 * f64::from(i))),
        )
        .unwrap();
    }
    let r = ok(
        &mut doc,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 50.0], "dir": [0.0, 0.0, -1.0], "all": true }),
    );
    assert_eq!(r["hits"].as_array().unwrap().len(), 1000);
    assert_eq!(r["raycast_hits_capped"], json!(true));
}

#[test]
fn max_distance_drops_far_hits() {
    let mut f = fixture();
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "all": true, "max_distance": 4.5 }),
    );
    assert_eq!(r["hits"].as_array().unwrap().len(), 2);
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.raycast",
            json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "max_distance": 0.001 })
        ),
        "locator_missed"
    );
    for bad in [0.0, -1.0] {
        assert_eq!(
            refusal(
                &mut f.doc,
                "hew.query.raycast",
                json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "max_distance": bad })
            ),
            "invalid_params"
        );
    }
}

#[test]
fn exclude_by_id_drops_subtree() {
    let mut f = fixture();
    let o1 = public(&f.doc, EntityRef::Instance(f.outer1));
    let floor = public(&f.doc, EntityRef::Object(f.floor));
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": [o1] }),
    );
    assert_eq!(r["leaf"], json!(floor));
    assert_eq!(r["kind"], json!("object"));
}

#[test]
fn exclude_a_group_drops_its_members() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 0.5);
    let b = extrude_box(&mut doc, 2.0, 3.0, 1.0, 0.5);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let gid = public(&doc, EntityRef::Group(g));
    let hit = ok(
        &mut doc,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 5.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(hit["leaf"], json!(public(&doc, EntityRef::Object(a))));
    assert_eq!(
        refusal(
            &mut doc,
            "hew.query.raycast",
            json!({ "origin": [0.5, 0.5, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": [gid] })
        ),
        "locator_missed"
    );
}

#[test]
fn exclude_by_path_drops_one_placement_not_the_other() {
    let mut f = fixture();
    let o1 = public(&f.doc, EntityRef::Instance(f.outer1));
    let o2 = public(&f.doc, EntityRef::Instance(f.outer2));
    let inner = public(&f.doc, EntityRef::Instance(f.inner));
    let board = public(&f.doc, EntityRef::Object(f.board));
    // The mirrored placement's board spans x 2.6 to 3.0.
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [2.8, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": [{ "path": [o1] }] }),
    );
    assert_eq!(r["path"], json!([o2, inner]));
    assert_eq!(r["leaf"], json!(board));
    // Excluding it by its leaf path leaves the floor.
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [2.8, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": [{ "path": [o2, inner, board] }] }),
    );
    assert_eq!(r["path"], json!([]));
}

#[test]
fn exclude_unknown_is_unknown_entity() {
    let mut f = fixture();
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.raycast",
            json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": ["obj_nope"] })
        ),
        "unknown_entity"
    );
    let board = public(&f.doc, EntityRef::Object(f.board));
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.raycast",
            json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": [board] })
        ),
        "unknown_occurrence",
        "a definition member needs a path"
    );
}

fn hide_outer_by_tag(f: &mut Fixture) {
    f.doc
        .add_node_tag(NodeId::Instance(f.outer1), vec!["Hid".into()])
        .unwrap();
    f.doc.set_tag_hidden(vec!["Hid".into()], true);
}

#[test]
fn hidden_tag_falls_through() {
    let mut f = fixture();
    hide_outer_by_tag(&mut f);
    let floor = public(&f.doc, EntityRef::Object(f.floor));
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(r["leaf"], json!(floor));
    assert_eq!(r["hidden"], json!(false));
}

#[test]
fn hidden_tag_reports_hidden_true() {
    let mut f = fixture();
    hide_outer_by_tag(&mut f);
    let board = public(&f.doc, EntityRef::Object(f.board));
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "include_hidden": true }),
    );
    assert_eq!(r["leaf"], json!(board));
    assert_eq!(r["hidden"], json!(true));
}

#[test]
fn tie_in_first_hit_is_ambiguous_but_all_returns_both() {
    let mut doc = Document::new();
    extrude_box(&mut doc, 0.0, 1.0, 1.0, 0.5);
    extrude_box(&mut doc, 0.0, 1.0, 1.0, 0.5);
    let p = json!({ "origin": [0.5, 0.5, 5.0], "dir": [0.0, 0.0, -1.0] });
    assert_eq!(
        refusal(&mut doc, "hew.query.raycast", p.clone()),
        "ambiguous_locator"
    );
    let mut all = p;
    all["all"] = json!(true);
    let r = ok(&mut doc, "hew.query.raycast", all);
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 4);
    near(&hits[0]["distance"], 4.5);
    near(&hits[1]["distance"], 4.5);
}

#[test]
fn mirrored_placement_normal_is_world_normal() {
    let mut f = fixture();
    let o2 = public(&f.doc, EntityRef::Instance(f.outer2));
    // From +x toward the mirrored board: its x=3.0 face (local x=0, local
    // normal -x) must report +x.
    let r = ok(
        &mut f.doc,
        "hew.query.raycast",
        json!({ "origin": [5.0, 0.1, 1.05], "dir": [-1.0, 0.0, 0.0] }),
    );
    assert_eq!(r["object"], json!(o2));
    near(&r["distance"], 2.0);
    near(&r["normal"][0], 1.0);
    near(&r["normal"][1], 0.0);
    near(&r["normal"][2], 0.0);
    // The locator is in the board's own frame: local x = 0.
    near(&r["face"]["at"][0], 0.0);
    let resolved = ok(
        &mut f.doc,
        "hew.query.resolve",
        json!({ "face": r["face"] }),
    );
    assert!(resolved["face"].is_object());
}

#[test]
fn invalid_params_are_refused() {
    let mut f = fixture();
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.raycast",
            json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "bogus": 1 })
        ),
        "invalid_params"
    );
    let many: Vec<Value> = (0..65).map(|_| json!("obj_x")).collect();
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.raycast",
            json!({ "origin": [0.2, 0.1, 5.0], "dir": [0.0, 0.0, -1.0], "exclude": many })
        ),
        "invalid_params"
    );
}

#[test]
fn past_the_cap_raycast_refuses_too_many_occurrences() {
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
    assert_eq!(
        refusal(
            &mut doc,
            "hew.query.raycast",
            json!({ "origin": [0.5, 0.5, 5.0], "dir": [0.0, 0.0, -1.0] })
        ),
        "too_many_occurrences"
    );
}
