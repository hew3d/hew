//! Linked copies on the bus (docs/agents/PLUGINS.md "Linked copies";
//! docs/agents/HEW_API.md §7): copies link by default, `copy.linked:
//! false` copies independently, `hew.component.make_unique` by path with
//! its remap, `hew.component.promote`, face and edge locators by path,
//! `would_unlink` on union and intersect, and the change sets' `linked`
//! and `copied_from`.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId};
use kernel::{Document, EntityRef, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn cuboid(doc: &mut Document, at: [f64; 3], size: [f64; 3]) -> ObjectId {
    let [x0, y0, z0] = at;
    let [dx, dy, dz] = size;
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z0),
        Point3::new(1.0, 0.0, z0),
        Point3::new(0.0, 1.0, z0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    let sk = doc.sketch_mut(s).unwrap();
    let c = [
        Point3::new(x0, y0, z0),
        Point3::new(x0 + dx, y0, z0),
        Point3::new(x0 + dx, y0 + dy, z0),
        Point3::new(x0, y0 + dy, z0),
    ];
    for k in 0..4 {
        sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], dz).unwrap().0
}

fn board(doc: &mut Document, x: f64) -> ObjectId {
    cuboid(doc, [x, 0.0, 0.0], [0.4, 0.2, 0.018])
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

fn obj(doc: &Document, o: ObjectId) -> String {
    public(doc, EntityRef::Object(o))
}

fn ins(doc: &Document, i: kernel::InstanceId) -> String {
    public(doc, EntityRef::Instance(i))
}

/// One connection over one document, as a client holds them.
struct Session {
    doc: Document,
    conn: Connection,
    next: i64,
}

impl Session {
    fn new(doc: Document) -> Session {
        let mut s = Session {
            doc,
            conn: Connection::new(Grant::app(), "linked"),
            next: 0,
        };
        s.raw("hew.meta.hello", json!({ "protocol": 1 }));
        s.raw("hew.doc.attach", json!({}));
        s
    }

    fn raw(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(self.next)),
            method: method.into(),
            params: Some(params),
        };
        let DispatchOutcome::Reply(r) =
            self.conn
                .dispatch(&REGISTRY, &mut self.doc, &mut NoHost, req)
        else {
            panic!("{method} replies")
        };
        serde_json::to_value(r).unwrap()
    }

    /// A successful command's own result (a mutating command's sole
    /// transaction entry unwrapped).
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.raw(method, params.clone());
        assert!(r.get("error").is_none(), "{method} {params}: {r}");
        let result = r["result"].clone();
        match result.get("results") {
            Some(list) => list[0].clone(),
            None => result,
        }
    }

    fn refusal(&mut self, method: &str, params: Value) -> Value {
        let r = self.raw(method, params.clone());
        assert_eq!(r["error"]["code"], -32000, "{method} {params}: {r}");
        r["error"]["data"].clone()
    }

    fn params_error(&mut self, method: &str, params: Value) {
        let r = self.raw(method, params.clone());
        assert_eq!(r["error"]["code"], -32602, "{method} {params}: {r}");
    }

    fn rows(&mut self) -> Vec<Value> {
        self.ok("hew.query.occurrences", json!({}))["occurrences"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn resolve(&self, id: &str) -> EntityRef {
        api::ids::IdResolver::new(&self.doc)
            .resolve(id)
            .unwrap_or_else(|| panic!("{id} resolves"))
    }

    fn instance(&self, id: &str) -> kernel::InstanceId {
        match self.resolve(id) {
            EntityRef::Instance(i) => i,
            other => panic!("{id} is {other:?}"),
        }
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

// --------------------------------------------------------------- copies

#[test]
fn move_copy_is_linked_by_default() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    doc.set_node_name(NodeId::Object(b), Some("Leg".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), vec!["Legs".into()])
        .unwrap();
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": {} }),
    );
    let ids = r["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    assert!(s(&ids[0]).starts_with("ins_"));
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["source"], json!(id));
    assert!(s(&linked[0]["definition"]).starts_with("cmp_"));
    let rows = ses.rows();
    let placements: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "instance").collect();
    assert_eq!(placements.len(), 2);
    for p in &placements {
        assert_eq!(p["definition"], linked[0]["definition"]);
        assert_eq!(p["linked"], true);
    }
    assert_eq!(ses.doc.object_name(b), Some("Leg"));
    assert_eq!(
        ses.doc.node_tags(NodeId::Object(b)),
        vec![vec!["Legs".to_string()]]
    );
}

#[test]
fn move_copy_linked_false_is_independent() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": { "linked": false } }),
    );
    assert!(s(&r["ids"][0]).starts_with("obj_"));
    assert_eq!(r["linked"], json!([]));
    assert!(ses.rows().iter().all(|r| r["kind"] == "object"));
    // A plain move carries no `linked`.
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [0.0, 1.0, 0.0] }),
    );
    assert!(r.get("linked").is_none());
}

#[test]
fn move_copy_count_and_grid_link_each_object_separately() {
    let mut doc = Document::new();
    let ids: Vec<String> = (0..4)
        .map(|k| {
            let b = board(&mut doc, k as f64);
            obj(&doc, b)
        })
        .collect();
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": ids, "translation": [0.0, 1.0, 0.0], "copy": { "count": 2 } }),
    );
    assert_eq!(r["ids"].as_array().unwrap().len(), 8);
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 4, "four boards make four sets");
    let defs: std::collections::BTreeSet<&str> =
        linked.iter().map(|w| s(&w["definition"])).collect();
    assert_eq!(defs.len(), 4);
    // A grid copy of a placement adds placements and wraps nothing.
    let first = s(&r["ids"][0]).to_string();
    let g = ses.ok(
        "hew.entity.move",
        json!({ "ids": [first], "copy": { "grid": [{ "count": 3, "step": [0.0, 0.0, 1.0] }] } }),
    );
    assert_eq!(g["ids"].as_array().unwrap().len(), 2);
    assert_eq!(g["linked"], json!([]));
}

#[test]
fn rotate_copy_links_and_refuses_grid() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.rotate",
        json!({ "ids": [id], "pivot": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": 1.0, "copy": {} }),
    );
    assert!(s(&r["ids"][0]).starts_with("ins_"));
    assert_eq!(r["linked"].as_array().unwrap().len(), 1);
    ses.params_error(
        "hew.entity.rotate",
        json!({ "ids": [id], "pivot": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": 1.0,
                "copy": { "grid": [{ "count": 2, "step": [1.0, 0.0, 0.0] }] } }),
    );
}

#[test]
fn a_group_copy_wraps_as_one_set() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let b = board(&mut doc, 1.0);
    let (_, ia, _) = doc.wrap_linked(NodeId::Object(a)).unwrap();
    let (_, ib, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    let (g, _) = doc
        .group_nodes(&[NodeId::Instance(ia), NodeId::Instance(ib)])
        .unwrap();
    let gid = public(&doc, EntityRef::Group(g));
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [gid], "translation": [0.0, 2.0, 0.0], "copy": {} }),
    );
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 1, "a group is one set");
    assert_eq!(linked[0]["source"], json!(gid));
    // The set's member is the group, still holding its two placements.
    let rows = ses.rows();
    let wrapped = rows
        .iter()
        .find(|r| r["kind"] == "instance" && r["definition"] == linked[0]["definition"])
        .unwrap();
    assert_eq!(wrapped["members"]["groups"], json!([gid]));
    let group_row = rows.iter().find(|r| r["id"] == json!(gid)).unwrap();
    assert_eq!(
        group_row["members"]["instances"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn the_copy_and_the_wrap_are_one_undo_entry_and_undo_restores_the_loose_board() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let bytes = doc.save();
    let depth = doc.undo_depth();
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": { "count": 3 } }),
    );
    assert_eq!(ses.doc.undo_depth(), depth + 1);
    ses.doc.undo().unwrap();
    assert_eq!(ses.doc.save(), bytes);
    let rows = ses.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], json!(id));
}

#[test]
fn sketch_copy_still_refuses() {
    let mut doc = Document::new();
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let sk = doc.add_sketch(plane);
    doc.sketch_mut(sk)
        .unwrap()
        .add_segment(Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0))
        .unwrap();
    let id = public(&doc, EntityRef::Sketch(sk));
    let mut ses = Session::new(doc);
    let data = ses.refusal(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": {} }),
    );
    assert_eq!(data["refusal"], "sketch_copy_unsupported");
}

#[test]
fn a_member_copy_through_a_path_links_inside_the_definition() {
    let mut doc = Document::new();
    let a = board(&mut doc, 0.0);
    let (def, inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let path = json!({ "path": [ins(&doc, inst), obj(&doc, a)] });
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [path], "translation": [0.0, 1.0, 0.0], "copy": {} }),
    );
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 1);
    let inner = linked[0]["definition"].clone();
    // The outer definition now holds two placements of one inner set.
    let members = ses.doc.def_member_nodes(def).unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|m| matches!(m, NodeId::Instance(_))));
    let rows = ses.rows();
    let inner_rows: Vec<&Value> = rows.iter().filter(|r| r["definition"] == inner).collect();
    assert_eq!(inner_rows.len(), 2);
    assert!(inner_rows.iter().all(|r| r["linked"] == true));
}

// ----------------------------------------------------------- make_unique

/// A world placement `outer` of a definition holding a placement `inner`
/// of a definition holding one board; a second world placement `twin` of
/// the outer definition.
struct Nested {
    doc: Document,
    board: ObjectId,
    inner_def: kernel::ComponentId,
    inner: kernel::InstanceId,
    outer_def: kernel::ComponentId,
    outer: kernel::InstanceId,
    twin: kernel::InstanceId,
}

fn nested() -> Nested {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    doc.set_node_name(NodeId::Object(b), Some("Leg".into()))
        .unwrap();
    doc.add_node_tag(NodeId::Object(b), vec!["Legs".into()])
        .unwrap();
    let (inner_def, inner, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let (outer_def, outer, _) = doc.make_component(&[NodeId::Instance(inner)]).unwrap();
    let (twin, _) = doc
        .place_instance(outer_def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    Nested {
        doc,
        board: b,
        inner_def,
        inner,
        outer_def,
        outer,
        twin,
    }
}

#[test]
fn make_unique_by_path_returns_component_instance_and_remap() {
    let n = nested();
    let path = json!([ins(&n.doc, n.outer), ins(&n.doc, n.inner)]);
    let (old_inner_def, old_outer_def) = (
        public(&n.doc, EntityRef::Component(n.inner_def)),
        public(&n.doc, EntityRef::Component(n.outer_def)),
    );
    let mut ses = Session::new(n.doc);
    let r = ses.ok("hew.component.make_unique", json!({ "path": path }));
    assert!(s(&r["component"]).starts_with("cmp_"));
    assert_ne!(r["component"], json!(old_inner_def));
    let remap = r["remap"].as_object().unwrap();
    assert_eq!(remap[&old_inner_def], r["component"]);
    assert!(
        remap.contains_key(&old_outer_def),
        "the cascade forked the outer definition"
    );
    assert!(remap.values().all(|v| v.is_string()));
}

#[test]
fn make_unique_instance_is_the_leaf_for_one_element_and_the_twin_for_a_deeper_path() {
    let n = nested();
    let (outer, inner) = (ins(&n.doc, n.outer), ins(&n.doc, n.inner));
    let mut ses = Session::new(n.doc);
    let r = ses.ok(
        "hew.component.make_unique",
        json!({ "path": [outer.clone(), inner.clone()] }),
    );
    assert_ne!(r["instance"], json!(inner));
    assert!(
        r["remap"]
            .as_object()
            .unwrap()
            .values()
            .any(|v| *v == r["instance"])
    );
    let one = ses.ok(
        "hew.component.make_unique",
        json!({ "path": [outer.clone()] }),
    );
    assert_eq!(one["instance"], json!(outer));
}

#[test]
fn make_unique_instance_form_still_works() {
    let n = nested();
    let twin = ins(&n.doc, n.twin);
    let mut ses = Session::new(n.doc);
    let r = ses.ok("hew.component.make_unique", json!({ "instance": twin }));
    assert!(s(&r["component"]).starts_with("cmp_"));
    assert_eq!(r["instance"], json!(twin));
    assert!(r["remap"].is_object());
}

#[test]
fn make_unique_refuses_both_or_neither_of_instance_and_path() {
    let n = nested();
    let twin = ins(&n.doc, n.twin);
    let mut ses = Session::new(n.doc);
    ses.params_error("hew.component.make_unique", json!({}));
    ses.params_error(
        "hew.component.make_unique",
        json!({ "instance": twin, "path": [twin] }),
    );
    ses.params_error("hew.component.make_unique", json!({ "path": [] }));
}

#[test]
fn make_unique_keeps_the_objects_name_and_tags() {
    let n = nested();
    let (outer, inner) = (ins(&n.doc, n.outer), ins(&n.doc, n.inner));
    let board = obj(&n.doc, n.board);
    let mut ses = Session::new(n.doc);
    let r = ses.ok(
        "hew.component.make_unique",
        json!({ "path": [outer, inner] }),
    );
    let copy = s(&r["remap"][&board]).to_string();
    let EntityRef::Object(o) = ses.resolve(&copy) else {
        panic!("an object")
    };
    assert_eq!(ses.doc.object_name(o), Some("Leg"));
    assert_eq!(
        ses.doc.node_tags(NodeId::Object(o)),
        vec![vec!["Legs".to_string()]]
    );
}

#[test]
fn make_unique_cascade_false_forks_only_the_last_definition() {
    let n = nested();
    let (outer, inner) = (ins(&n.doc, n.outer), ins(&n.doc, n.inner));
    let old_outer = public(&n.doc, EntityRef::Component(n.outer_def));
    let mut ses = Session::new(n.doc);
    let r = ses.ok(
        "hew.component.make_unique",
        json!({ "path": [outer, inner], "cascade": false }),
    );
    assert!(!r["remap"].as_object().unwrap().contains_key(&old_outer));
    // Both world placements still share the outer definition.
    assert_eq!(
        ses.doc.instance_def(n.twin),
        ses.doc.instance_def(ses.instance(&ins(&ses.doc, n.outer)))
    );
}

#[test]
fn make_unique_deep_remaps_nested_definitions() {
    let n = nested();
    let twin = ins(&n.doc, n.twin);
    let inner_def = public(&n.doc, EntityRef::Component(n.inner_def));
    let mut ses = Session::new(n.doc);
    let shallow = ses.ok(
        "hew.component.make_unique",
        json!({ "instance": twin.clone() }),
    );
    assert!(
        !shallow["remap"]
            .as_object()
            .unwrap()
            .contains_key(&inner_def)
    );
    let n = nested();
    let twin = ins(&n.doc, n.twin);
    let inner_def = public(&n.doc, EntityRef::Component(n.inner_def));
    let mut ses = Session::new(n.doc);
    let deep = ses.ok(
        "hew.component.make_unique",
        json!({ "instance": twin, "deep": true }),
    );
    assert!(deep["remap"].as_object().unwrap().contains_key(&inner_def));
}

#[test]
fn make_unique_of_a_set_of_one_stays_linked() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (_, inst, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    let id = ins(&doc, inst);
    let mut ses = Session::new(doc);
    let r = ses.ok("hew.component.make_unique", json!({ "instance": id }));
    let row = ses
        .rows()
        .into_iter()
        .find(|row| row["id"] == json!(id))
        .unwrap();
    assert_eq!(row["definition"], r["component"]);
    assert_eq!(row["linked"], true);
}

#[test]
fn make_unique_is_one_undo_entry() {
    let n = nested();
    let (outer, inner) = (ins(&n.doc, n.outer), ins(&n.doc, n.inner));
    let bytes = n.doc.save();
    let depth = n.doc.undo_depth();
    let mut ses = Session::new(n.doc);
    ses.ok(
        "hew.component.make_unique",
        json!({ "path": [outer, inner], "deep": true }),
    );
    assert_eq!(ses.doc.undo_depth(), depth + 1);
    ses.doc.undo().unwrap();
    assert_eq!(ses.doc.save(), bytes);
}

// --------------------------------------------------------------- promote

#[test]
fn promote_removes_the_attribute_and_names_the_definition() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (def, inst, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    let cmp = public(&doc, EntityRef::Component(def));
    let id = ins(&doc, inst);
    let depth = doc.undo_depth();
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.component.promote",
        json!({ "component": cmp, "name": "  Shelf  " }),
    );
    assert_eq!(r, json!({ "component": cmp }));
    assert_eq!(ses.doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(ses.doc.component_name(def), Some("Shelf"));
    let row = ses
        .rows()
        .into_iter()
        .find(|row| row["id"] == json!(id))
        .unwrap();
    assert_eq!(row["linked"], false);
    assert_eq!(row["definition_name"], "Shelf");
}

#[test]
fn promote_refuses_not_linked() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (def, _, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let cmp = public(&doc, EntityRef::Component(def));
    let mut ses = Session::new(doc);
    let data = ses.refusal(
        "hew.component.promote",
        json!({ "component": cmp, "name": "Shelf" }),
    );
    assert_eq!(data["refusal"], "not_linked");
}

#[test]
fn promote_rejects_a_blank_name() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (def, _, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    let cmp = public(&doc, EntityRef::Component(def));
    let mut ses = Session::new(doc);
    for name in ["", "   ", "a\u{7}b"] {
        ses.params_error(
            "hew.component.promote",
            json!({ "component": cmp, "name": name }),
        );
    }
    ses.params_error(
        "hew.component.promote",
        json!({ "component": cmp, "name": "x".repeat(129) }),
    );
}

// -------------------------------------------------------------- locators

/// A linked set of two placements of one board, the second at x = 2.
fn pair() -> (Document, ObjectId, kernel::InstanceId, kernel::InstanceId) {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (def, first, _) = doc.wrap_linked(NodeId::Object(b)).unwrap();
    let (second, _) = doc
        .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
        .unwrap();
    (doc, b, first, second)
}

/// The top face's z: the board's highest vertex.
fn top(doc: &Document, o: ObjectId) -> f64 {
    doc.object(o)
        .unwrap()
        .vertices()
        .values()
        .map(|v| v.position.z)
        .fold(f64::MIN, f64::max)
}

#[test]
fn face_locator_path_reaches_the_definition_face() {
    let (doc, b, _, second) = pair();
    let path = json!([ins(&doc, second), obj(&doc, b)]);
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.query.resolve",
        json!({ "face": { "path": path, "at": [0.2, 0.1, 0.018] } }),
    );
    let by_object = ses.ok(
        "hew.query.resolve",
        json!({ "face": { "object": obj(&ses.doc, b), "at": [0.2, 0.1, 0.018] } }),
    );
    assert_eq!(r, by_object);
}

#[test]
fn face_locator_object_form_still_works() {
    let (doc, b, _, _) = pair();
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.solid.push_pull",
        json!({ "face": { "object": id, "at": [0.2, 0.1, 0.018] }, "distance": 0.01 }),
    );
    assert!((top(&ses.doc, b) - 0.028).abs() < 1e-9);
}

#[test]
fn face_locator_path_to_a_group_refuses_not_an_object() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let (g, _) = doc.group_nodes(&[NodeId::Object(b)]).unwrap();
    let gid = public(&doc, EntityRef::Group(g));
    let mut ses = Session::new(doc);
    let data = ses.refusal(
        "hew.material.paint",
        json!({ "face": { "path": [gid], "at": [0.2, 0.1, 0.018] }, "material": null }),
    );
    assert_eq!(data["refusal"], "not_an_object");
}

#[test]
fn face_locator_refuses_object_and_path_together() {
    let (doc, b, first, _) = pair();
    let (id, path) = (obj(&doc, b), json!([ins(&doc, first), obj(&doc, b)]));
    let mut ses = Session::new(doc);
    ses.params_error(
        "hew.query.resolve",
        json!({ "face": { "object": id, "path": path, "at": [0.2, 0.1, 0.018] } }),
    );
}

#[test]
fn edge_locator_path_reaches_the_definition_edge() {
    let (doc, b, _, second) = pair();
    let path = json!([ins(&doc, second), obj(&doc, b)]);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let by_path = ses.ok(
        "hew.query.resolve",
        json!({ "edge": { "path": path, "at": [0.2, 0.0, 0.0] } }),
    );
    let by_object = ses.ok(
        "hew.query.resolve",
        json!({ "edge": { "object": id, "at": [0.2, 0.0, 0.0] } }),
    );
    assert_eq!(by_path, by_object);
}

#[test]
fn paint_by_path_shows_on_every_copy() {
    let (doc, b, _, second) = pair();
    let path = json!([ins(&doc, second), obj(&doc, b)]);
    let mut ses = Session::new(doc);
    let red = ses.ok(
        "hew.material.create",
        json!({ "name": "Red", "color": [255, 0, 0] }),
    );
    let red = red["material"]
        .as_str()
        .or(red["id"].as_str())
        .unwrap()
        .to_string();
    ses.ok(
        "hew.material.paint",
        json!({ "face": { "path": path, "at": [0.2, 0.1, 0.018] }, "material": red }),
    );
    // One face of the shared board is painted; both placements show it.
    let painted = ses
        .doc
        .object(b)
        .unwrap()
        .faces()
        .values()
        .filter(|f| f.material.is_some())
        .count();
    assert_eq!(painted, 1);
}

#[test]
fn push_pull_by_path_changes_every_copy() {
    let (doc, b, first, second) = pair();
    let path = json!([ins(&doc, second), obj(&doc, b)]);
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.solid.push_pull",
        json!({ "face": { "path": path, "at": [0.2, 0.1, 0.018] }, "distance": 0.02 }),
    );
    assert!((top(&ses.doc, b) - 0.038).abs() < 1e-9);
    assert_eq!(ses.doc.instance_def(first), ses.doc.instance_def(second));
}

// -------------------------------------------------------------- booleans

#[test]
fn union_with_a_placement_refuses_would_unlink() {
    let (mut doc, _, first, _) = pair();
    let tool = cuboid(&mut doc, [0.3, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let (a, b) = (ins(&doc, first), obj(&doc, tool));
    let bytes = doc.save();
    let mut ses = Session::new(doc);
    let data = ses.refusal("hew.solid.union", json!({ "a": a, "b": b }));
    assert_eq!(data["refusal"], "would_unlink");
    assert_eq!(data["detail"]["operands"], json!([a]));
    assert_eq!(ses.doc.save(), bytes);
    // A group holding a placement asks too.
    let (g, _) = ses.doc.group_nodes(&[NodeId::Instance(first)]).unwrap();
    let gid = public(&ses.doc, EntityRef::Group(g));
    let data = ses.refusal("hew.solid.union", json!({ "a": gid, "b": b }));
    assert_eq!(data["refusal"], "would_unlink");
}

#[test]
fn union_with_unlink_true_unlinks_and_joins() {
    let (mut doc, b, first, second) = pair();
    let tool = cuboid(&mut doc, [0.3, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let (a, t) = (ins(&doc, first), obj(&doc, tool));
    let depth = doc.undo_depth();
    let mut ses = Session::new(doc);
    let r = ses.ok("hew.solid.union", json!({ "a": a, "b": t, "unlink": true }));
    assert!(s(&r["result"]).starts_with("obj_"));
    assert_eq!(r["unlinked"], json!([a]));
    assert_eq!(ses.doc.undo_depth(), depth + 1, "one undo entry");
    // The other copy keeps the shared board.
    assert!(ses.doc.instance_def(second).is_some());
    assert!(ses.doc.object(b).is_some());
    assert!(
        ses.doc.instance_def(first).is_none(),
        "the placement was exploded"
    );
    // A world-only pair with unlink: true is accepted and ignored.
    let mut doc = Document::new();
    let x = cuboid(&mut doc, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let y = cuboid(&mut doc, [0.5, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let (x, y) = (obj(&doc, x), obj(&doc, y));
    let mut ses = Session::new(doc);
    let r = ses.ok("hew.solid.union", json!({ "a": x, "b": y, "unlink": true }));
    assert!(r["result"].is_string());
    assert!(r.get("unlinked").is_none());
}

#[test]
fn union_with_unlink_true_takes_a_member_by_path() {
    let (mut doc, b, first, second) = pair();
    let tool = cuboid(&mut doc, [0.3, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let a = json!({ "path": [ins(&doc, first), obj(&doc, b)] });
    let t = obj(&doc, tool);
    let mut ses = Session::new(doc);
    let data = ses.refusal("hew.solid.union", json!({ "a": a, "b": t }));
    assert_eq!(data["refusal"], "would_unlink");
    let r = ses.ok("hew.solid.union", json!({ "a": a, "b": t, "unlink": true }));
    assert!(s(&r["result"]).starts_with("obj_"));
    assert!(ses.doc.instance_def(second).is_some());
}

#[test]
fn intersect_with_a_placement_refuses_would_unlink() {
    let (mut doc, _, first, _) = pair();
    let tool = cuboid(&mut doc, [0.3, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let (a, b) = (ins(&doc, first), obj(&doc, tool));
    let mut ses = Session::new(doc);
    let data = ses.refusal("hew.solid.intersect", json!({ "a": b, "b": a }));
    assert_eq!(data["refusal"], "would_unlink");
    ses.params_error(
        "hew.solid.subtract",
        json!({ "a": b, "b": a, "unlink": true }),
    );
}

#[test]
fn subtract_with_a_placement_host_never_refuses_would_unlink() {
    let (mut doc, b, first, second) = pair();
    let tool = cuboid(&mut doc, [0.1, 0.05, -0.01], [0.05, 0.05, 0.05]);
    let a = json!({ "path": [ins(&doc, first), obj(&doc, b)] });
    let t = obj(&doc, tool);
    let mut ses = Session::new(doc);
    let r = ses.ok("hew.solid.subtract", json!({ "a": a, "b": t }));
    assert!(r["result"].is_string());
    assert_eq!(ses.doc.instance_def(first), ses.doc.instance_def(second));
}

// ----------------------------------------------------------- change sets

#[test]
fn change_set_carries_linked_and_copied_from() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let epoch = ses.doc.document_epoch();
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": {} }),
    );
    let records = ses.doc.change_records_since_epoch(epoch).unwrap().to_vec();
    let last = records.last().unwrap();
    let event = api::events::document_changed(&ses.doc, "doc_1", last);
    let changes = &event["changes"];
    assert_eq!(changes["linked"], r["linked"]);
    let wrap_instance = r["linked"][0]["instance"].clone();
    assert!(
        changes["copied_from"]
            .as_array()
            .unwrap()
            .contains(&json!([r["ids"][0], wrap_instance])),
        "{changes}"
    );
}

#[test]
fn changes_since_carries_linked_and_copied_from() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let generation = ses.doc.history_generation();
    let r = ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": {} }),
    );
    let since = ses.ok(
        "hew.query.changes_since",
        json!({ "generation": generation }),
    );
    assert_eq!(since["changes"]["linked"], r["linked"]);
    assert!(
        !since["changes"]["copied_from"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn undo_change_set_carries_no_linked_or_copied_from() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0], "copy": {} }),
    );
    let epoch = ses.doc.document_epoch();
    ses.doc.undo().unwrap();
    let records = ses.doc.change_records_since_epoch(epoch).unwrap().to_vec();
    let event = api::events::document_changed(&ses.doc, "doc_1", records.last().unwrap());
    assert_eq!(event["kind"], "undo");
    assert_eq!(event["changes"]["linked"], json!([]));
    assert_eq!(event["changes"]["copied_from"], json!([]));
}

#[test]
fn a_plain_commit_has_empty_linked_and_copied_from() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let generation = ses.doc.history_generation();
    ses.ok(
        "hew.entity.move",
        json!({ "ids": [id], "translation": [1.0, 0.0, 0.0] }),
    );
    let since = ses.ok(
        "hew.query.changes_since",
        json!({ "generation": generation }),
    );
    assert_eq!(since["changes"]["linked"], json!([]));
    assert_eq!(since["changes"]["copied_from"], json!([]));
}

#[test]
fn full_change_sets_keep_empty_linked_and_copied_from() {
    let full = api::events::full_change_set_json();
    assert_eq!(full["full"], true);
    assert_eq!(full["linked"], json!([]));
    assert_eq!(full["copied_from"], json!([]));
}

// --------------------------------------------------------------- registry

#[test]
fn registry_declares_promote_and_signature() {
    let reg = api::Registry::protocol_1();
    let p = reg.get("hew.component.promote").unwrap();
    assert_eq!(p.class, api::CommandClass::ModelMutating);
    assert_eq!(p.permission, api::Permission::ModelWrite);
    assert!(p.implemented && p.mutates_document);
    assert!(p.refusals.iter().any(|r| r == "not_linked"));
    assert!(reg.get("hew.query.signature").unwrap().implemented);
    for name in ["hew.solid.union", "hew.solid.intersect"] {
        assert!(
            reg.get(name)
                .unwrap()
                .refusals
                .iter()
                .any(|r| r == "would_unlink")
        );
    }
    assert!(
        !reg.get("hew.solid.subtract")
            .unwrap()
            .refusals
            .iter()
            .any(|r| r == "would_unlink")
    );
}

#[test]
fn copy_schema_accepts_linked_on_move_and_rotate_only_where_declared() {
    let reg = api::Registry::protocol_1();
    for name in ["hew.entity.move", "hew.entity.rotate"] {
        let c = reg.get(name).unwrap();
        assert_eq!(
            c.params_schema["properties"]["copy"]["properties"]["linked"]["type"], "boolean",
            "{name}"
        );
        assert!(
            c.result_schema["properties"]["linked"].is_object(),
            "{name}"
        );
    }
    let scale = reg.get("hew.entity.scale").unwrap();
    assert!(scale.params_schema["properties"].get("copy").is_none());
}

// ------------------------------------------------------------------ flip

/// The world bounding box of an object's vertices.
fn bounds(doc: &Document, o: ObjectId) -> (Point3, Point3) {
    let mut lo = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    for v in doc.object(o).unwrap().vertices().values() {
        let p = v.position;
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

#[test]
fn flip_axis_mirrors_about_the_bbox_center() {
    let mut doc = Document::new();
    let b = cuboid(&mut doc, [1.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let r = ses.ok("hew.entity.flip", json!({ "ids": [id], "axis": "x" }));
    assert_eq!(r, json!({}));
    let (lo, hi) = bounds(&ses.doc, b);
    assert!(
        (lo.x - 1.0).abs() < 1e-9 && (hi.x - 1.4).abs() < 1e-9,
        "flipped in place"
    );
    ses.ok(
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "x", "about": "origin" }),
    );
    let (lo, hi) = bounds(&ses.doc, b);
    assert!(
        (lo.x + 1.4).abs() < 1e-9 && (hi.x + 1.0).abs() < 1e-9,
        "across x = 0"
    );
    ses.ok(
        "hew.entity.flip",
        json!({ "ids": [id], "plane": { "point": [0.0, 1.0, 0.0], "normal": [0.0, 2.0, 0.0] } }),
    );
    let (lo, hi) = bounds(&ses.doc, b);
    assert!(
        (lo.y - 1.8).abs() < 1e-9 && (hi.y - 2.0).abs() < 1e-9,
        "across y = 1"
    );
    assert!(
        ses.doc.object(b).unwrap().volume().unwrap() > 0.0,
        "right side out"
    );
}

#[test]
fn flip_copy_makes_a_mirrored_linked_placement() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let r = ses.ok(
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "x", "about": [1.0, 0.0, 0.0], "copy": {} }),
    );
    let copy = s(&r["ids"][0]).to_string();
    assert!(copy.starts_with("ins_"));
    assert_eq!(r["linked"].as_array().unwrap().len(), 1);
    let row = ses
        .rows()
        .into_iter()
        .find(|row| row["id"] == json!(copy))
        .unwrap();
    assert_eq!(row["mirrored"], true);
    assert_eq!(row["linked"], true);
    // An independent mirrored copy bakes.
    let r = ses.ok(
        "hew.entity.flip",
        json!({ "ids": [copy], "axis": "y", "copy": { "linked": false } }),
    );
    assert!(
        s(&r["ids"][0]).starts_with("ins_"),
        "a placement copies as a placement"
    );
}

#[test]
fn flip_refuses_axis_and_plane_together() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    let plane = json!({ "point": [0.0, 0.0, 0.0], "normal": [1.0, 0.0, 0.0] });
    for params in [
        json!({ "ids": [id], "axis": "x", "plane": plane }),
        json!({ "ids": [id] }),
        json!({ "ids": [id], "plane": plane, "about": "origin" }),
        json!({ "ids": [id], "axis": "w" }),
        json!({ "ids": [id], "axis": "x", "about": "middle" }),
        json!({ "ids": [id], "plane": { "point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 0.0] } }),
    ] {
        ses.params_error("hew.entity.flip", params);
    }
}

#[test]
fn flip_refuses_grid_copy() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    ses.params_error(
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "x", "copy": { "grid": [{ "count": 2, "step": [1.0, 0.0, 0.0] }] } }),
    );
    ses.params_error(
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "x", "copy": { "count": 2 } }),
    );
}

#[test]
fn flip_is_one_undo_entry() {
    let mut doc = Document::new();
    let b = board(&mut doc, 0.0);
    let id = obj(&doc, b);
    let bytes = doc.save();
    let depth = doc.undo_depth();
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.entity.flip",
        json!({ "ids": [id], "axis": "z", "copy": {} }),
    );
    assert_eq!(ses.doc.undo_depth(), depth + 1);
    ses.doc.undo().unwrap();
    assert_eq!(ses.doc.save(), bytes);
    ses.ok("hew.entity.flip", json!({ "ids": [id], "axis": "z" }));
    assert_eq!(ses.doc.undo_depth(), depth + 1);
}

/// The member's world bounds through one placement of its definition.
fn bounds_at(doc: &Document, path: &[kernel::InstanceId], o: ObjectId) -> (Point3, Point3) {
    let occ = doc.occurrence_at(path, NodeId::Object(o)).unwrap();
    let mut lo = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    for v in doc.object(o).unwrap().vertices().values() {
        let p = occ.pose.apply_point(v.position);
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

/// A definition with `a` and `pin` and three placements: as made, moved
/// along x, and mirrored in x then moved along y.
fn three_placements() -> (Document, [kernel::InstanceId; 3], ObjectId, ObjectId) {
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [1.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let pin = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.1, 0.1, 0.1]);
    let (def, i1, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(pin)])
        .unwrap();
    let (i2, _) = doc
        .place_instance(def, Transform::translation(Vec3::new(10.0, 0.0, 0.0)))
        .unwrap();
    let mirror_x = Transform::reflection(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let (i3, _) = doc
        .place_instance(
            def,
            mirror_x.then(&Transform::translation(Vec3::new(0.0, 5.0, 0.0))),
        )
        .unwrap();
    (doc, [i1, i2, i3], a, pin)
}

#[test]
fn flip_through_a_path_mirrors_the_member_in_every_placement() {
    let (doc, [i1, i2, i3], a, _) = three_placements();
    let path = json!({ "path": [ins(&doc, i1), obj(&doc, a)] });
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.entity.flip",
        json!({ "ids": [path], "axis": "x", "about": "origin" }),
    );
    // The definition-local x range [1, 1.4] became [-1.4, -1] and every
    // placement shows its own pose of that.
    let near = |p: (Point3, Point3), lo: f64, hi: f64| {
        assert!(
            (p.0.x - lo).abs() < 1e-9 && (p.1.x - hi).abs() < 1e-9,
            "{p:?}"
        );
    };
    near(bounds_at(&ses.doc, &[i1], a), -1.4, -1.0);
    near(bounds_at(&ses.doc, &[i2], a), 8.6, 9.0);
    near(bounds_at(&ses.doc, &[i3], a), 1.0, 1.4);
    assert!(ses.doc.object(a).unwrap().signed_volume() > 0.0);
}

#[test]
fn flip_through_a_path_defaults_to_the_members_own_center() {
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [1.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let pin = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.1, 0.1, 0.1]);
    let rest = cuboid(&mut doc, [0.0, 4.0, 0.0], [0.1, 0.1, 0.1]);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(pin)])
        .unwrap();
    let (def, i1, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(rest)])
        .unwrap();
    let mirror_x = Transform::reflection(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let (i3, _) = doc.place_instance(def, mirror_x).unwrap();
    let path = json!({ "path": [ins(&doc, i1), public(&doc, EntityRef::Group(g))] });
    let mut ses = Session::new(doc);
    ses.ok("hew.entity.flip", json!({ "ids": [path], "axis": "x" }));
    // The group spans x 0 to 1.4, center 0.7: x becomes 1.4 - x, so `a`
    // moves to [0, 0.4]; the mirrored placement shows it at [-0.4, 0].
    let x = |p: (Point3, Point3)| (p.0.x, p.1.x);
    let (lo, hi) = x(bounds_at(&ses.doc, &[i1], a));
    assert!(lo.abs() < 1e-9 && (hi - 0.4).abs() < 1e-9, "{lo} {hi}");
    let (lo, hi) = x(bounds_at(&ses.doc, &[i3], a));
    assert!((lo + 0.4).abs() < 1e-9 && hi.abs() < 1e-9, "{lo} {hi}");
    // The pin took the far end.
    let (lo, hi) = x(bounds_at(&ses.doc, &[i1], pin));
    assert!(
        (lo - 1.3).abs() < 1e-9 && (hi - 1.4).abs() < 1e-9,
        "{lo} {hi}"
    );
    for o in [a, pin] {
        assert!(ses.doc.object(o).unwrap().signed_volume() > 0.0);
    }
}

#[test]
fn flip_accepts_a_tiny_plane_normal() {
    let mut doc = Document::new();
    let b = cuboid(&mut doc, [1.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let id = obj(&doc, b);
    let mut ses = Session::new(doc);
    ses.ok(
        "hew.entity.flip",
        json!({ "ids": [id], "plane": { "point": [0.0, 0.0, 0.0], "normal": [1e-13, 0.0, 0.0] } }),
    );
    let (lo, hi) = bounds(&ses.doc, b);
    assert!((lo.x + 1.4).abs() < 1e-9 && (hi.x + 1.0).abs() < 1e-9);
    ses.params_error(
        "hew.entity.flip",
        json!({ "ids": [id], "plane": { "point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 0.0] } }),
    );
}

#[test]
fn registry_declares_flip() {
    let reg = api::Registry::protocol_1();
    let f = reg.get("hew.entity.flip").unwrap();
    assert_eq!(f.class, api::CommandClass::ModelMutating);
    assert_eq!(f.permission, api::Permission::ModelWrite);
    assert!(f.implemented && f.mutates_document);
    assert!(
        f.params_schema["properties"]["copy"]["properties"]
            .get("grid")
            .is_none()
    );
}
