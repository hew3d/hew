//! Conformance coverage for the user's editing context from the bus
//! (docs/agents/HEW_API.md §6.3): `hew.view.enter`, `hew.view.exit`,
//! `hew.view.exit_all`, and `hew.query.context`'s `fallback` and `armed`.
//!
//! Two layers: the `NoHost` refusals (a headless host cannot act on a
//! window), and a `FakeHost` that records what the handler asked of it and
//! holds a fallback list and an armed flag the way the live app's host does.

use api::{
    Connection, ContextEntry, ContextKind, DispatchOutcome, Grant, Host, IdResolver, NoHost,
    Refusal, Request, RequestId, codes,
};
use kernel::{Document, EntityRef, InstanceId, NodeId, Plane, Point3, Transform, Vec3};
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// What the host was asked to do, and the state it reports.
#[derive(Default)]
struct FakeHost {
    armed: bool,
    fallback: Vec<ContextEntry>,
    entered: Vec<(Vec<InstanceId>, NodeId)>,
    exits: u32,
    exit_alls: u32,
}

impl Host for FakeHost {
    fn fallback_context(&self) -> Vec<ContextEntry> {
        self.fallback.clone()
    }

    fn gesture_armed(&self) -> bool {
        self.armed
    }

    fn enter_user_context(&mut self, path: &[InstanceId], leaf: NodeId) -> Result<(), Refusal> {
        self.entered.push((path.to_vec(), leaf));
        Ok(())
    }

    fn exit_user_context(&mut self) -> Result<(), Refusal> {
        self.exits += 1;
        Ok(())
    }

    fn exit_all_user_contexts(&mut self) -> Result<(), Refusal> {
        self.exit_alls += 1;
        Ok(())
    }
}

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

fn boxed(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude");
    *doc.live_object_ids().last().expect("an object")
}

fn public(doc: &Document, entity: EntityRef) -> String {
    IdResolver::new(doc)
        .public_of(doc, &entity)
        .expect("a live entity carries a stable id")
}

/// A two-box component placed twice (the second mirrored), a lone sketch,
/// and a world box.
struct Fixture {
    doc: Document,
    comp: kernel::ComponentId,
    i2: InstanceId,
    a: kernel::ObjectId,
    w: kernel::ObjectId,
    sketch: kernel::SketchId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let a = boxed(&mut doc, 0.0);
    let b = boxed(&mut doc, 2.0);
    let (comp, _, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("component");
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(20.0, 0.0, 0.0)));
    let (i2, _) = doc
        .place_instance(comp, mirror)
        .expect("mirrored placement");
    let w = boxed(&mut doc, 30.0);
    let sketch = doc.add_sketch(ground());
    Fixture {
        doc,
        comp,
        i2,
        a,
        w,
        sketch,
    }
}

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn call(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> api::Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, host, req(9, method, params))
    else {
        panic!("{method} replies")
    };
    r
}

fn connect(doc: &mut Document, host: &mut dyn Host) -> Connection {
    let mut conn = Connection::new(Grant::app(), "test");
    for (method, params) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        let r = call(&mut conn, doc, host, method, params);
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
    conn
}

fn ok(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

fn refused(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(err.code, codes::REFUSED, "{method}: {err:?}");
    err.data.expect("a refusal carries the canonical data")
}

fn params_error(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) {
    let r = call(conn, doc, host, method, params.clone());
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} {params} was expected to fail"));
    assert_eq!(
        err.code,
        codes::INVALID_PARAMS,
        "{method} {params}: {err:?}"
    );
}

#[test]
fn a_headless_host_cannot_act_on_a_window() {
    let mut f = fixture();
    let mut host = NoHost;
    let mut conn = connect(&mut f.doc, &mut host);
    let w = public(&f.doc, EntityRef::Object(f.w));
    for (method, params) in [
        ("hew.view.enter", json!({ "id": w })),
        ("hew.view.exit", json!({})),
        ("hew.view.exit_all", json!({})),
    ] {
        let data = refused(&mut conn, &mut f.doc, &mut host, method, params);
        assert_eq!(data["refusal"], "host_capability_missing", "{method}");
    }
    // The context read still answers, with nothing in front of the stack.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.query.context",
        json!({}),
    );
    assert_eq!(got["fallback"], json!([]));
    assert_eq!(got["armed"], json!(false));
}

#[test]
fn the_commands_are_solitary_view_commands_that_mutate() {
    for name in ["hew.view.enter", "hew.view.exit", "hew.view.exit_all"] {
        let decl = REGISTRY.get(name).expect("declared");
        assert_eq!(decl.class, api::CommandClass::Solitary, "{name}");
        assert_eq!(decl.served, api::Served::Host, "{name}");
        assert!(decl.mutates_document, "{name}");
        assert!(decl.implemented, "{name}");
        assert_eq!(decl.permission, api::Permission::View, "{name}");
    }
}

#[test]
fn a_transaction_refuses_them_as_a_params_error() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let w = public(&f.doc, EntityRef::Object(f.w));
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.view.enter", "params": { "id": w } },
            { "method": "hew.entity.rename", "params": { "id": w, "name": "x" } }
        ] }),
    );
    assert!(host.entered.is_empty(), "nothing reached the host");
}

#[test]
fn targets_that_cannot_be_entered_are_params_errors() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let cmp = public(&f.doc, EntityRef::Component(f.comp));
    let sk = public(&f.doc, EntityRef::Sketch(f.sketch));
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "id": cmp }),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "id": sk }),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "path": [sk] }),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({}),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "id": cmp, "path": [cmp] }),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "path": [] }),
    );
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "id": "obj_99999" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    assert!(host.entered.is_empty());
}

#[test]
fn a_path_that_leads_nowhere_is_unknown_occurrence() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let a = public(&f.doc, EntityRef::Object(f.a));
    let w = public(&f.doc, EntityRef::Object(f.w));
    // An object cannot sit above another element of a path.
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "path": [w, a] }),
    );
    assert_eq!(data["refusal"], "unknown_occurrence");
    // A definition member by bare id needs its path.
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "id": a }),
    );
    assert_eq!(data["refusal"], "unknown_occurrence");
    assert!(host.entered.is_empty());
}

#[test]
fn an_armed_gesture_refuses_all_three_before_the_host_hears() {
    let mut f = fixture();
    let mut host = FakeHost {
        armed: true,
        ..FakeHost::default()
    };
    let mut conn = connect(&mut f.doc, &mut host);
    let w = public(&f.doc, EntityRef::Object(f.w));
    for (method, params) in [
        ("hew.view.enter", json!({ "id": w })),
        ("hew.view.exit", json!({})),
        ("hew.view.exit_all", json!({})),
    ] {
        let data = refused(&mut conn, &mut f.doc, &mut host, method, params);
        assert_eq!(data["refusal"], "gesture_in_progress", "{method}");
        assert!(
            data["explanation"]
                .as_str()
                .is_some_and(|e| e.starts_with("A tool gesture is in progress")),
            "{data}"
        );
    }
    assert!(host.entered.is_empty());
    assert_eq!((host.exits, host.exit_alls), (0, 0));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.query.context",
        json!({}),
    );
    assert_eq!(got["armed"], json!(true));
}

#[test]
fn enter_hands_the_resolved_path_and_leaf_to_the_host() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let i2 = public(&f.doc, EntityRef::Instance(f.i2));
    let a = public(&f.doc, EntityRef::Object(f.a));
    assert_eq!(
        ok(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.view.enter",
            json!({ "id": i2 })
        ),
        json!({})
    );
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.enter",
        json!({ "path": [i2, a] }),
    );
    assert_eq!(
        host.entered,
        vec![
            (vec![], NodeId::Instance(f.i2)),
            (vec![f.i2], NodeId::Object(f.a)),
        ]
    );
    ok(&mut conn, &mut f.doc, &mut host, "hew.view.exit", json!({}));
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.exit_all",
        json!({}),
    );
    assert_eq!((host.exits, host.exit_alls), (1, 1));
}

#[test]
fn context_reports_the_hosts_fallback_entries() {
    let mut f = fixture();
    let mut host = FakeHost {
        fallback: vec![
            ContextEntry {
                kind: ContextKind::Instance,
                node: NodeId::Instance(f.i2),
            },
            ContextEntry {
                kind: ContextKind::Object,
                node: NodeId::Object(f.a),
            },
        ],
        ..FakeHost::default()
    };
    let mut conn = connect(&mut f.doc, &mut host);
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.query.context",
        json!({}),
    );
    let i2 = public(&f.doc, EntityRef::Instance(f.i2));
    let a = public(&f.doc, EntityRef::Object(f.a));
    assert_eq!(
        got["fallback"],
        json!([{ "kind": "instance", "id": i2 }, { "kind": "object", "id": a }])
    );
    assert_eq!(got["stack"], json!([]));
    assert_eq!(got["armed"], json!(false));
}
