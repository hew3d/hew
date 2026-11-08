//! `hew.component.import_definition` (docs/agents/HEW_API.md §7, document commands): bytes
//! become one unplaced component definition, reused by content hash.

use api::{
    Connection, DispatchOutcome, Grant, Host, ImportOptions, NoHost, Permission, Refusal, Request,
    RequestId,
};
use kernel::{Document, ImportNode, ImportScene, MeshRecipe, Point3};
use serde_json::{Value, json};
use std::collections::BTreeSet;

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// A host whose importer turns any bytes into a unit cube and remembers how
/// it was called.
#[derive(Default)]
struct CubeHost {
    calls: Vec<(String, bool)>,
}

fn cube_scene() -> ImportScene {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let n = faces.len();
    let recipe = MeshRecipe {
        name: "import".into(),
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

impl Host for CubeHost {
    fn import_document(
        &mut self,
        doc: &mut Document,
        path: &str,
        options: &ImportOptions,
    ) -> Result<Value, Refusal> {
        self.calls.push((path.to_string(), options.record));
        doc.ingest(cube_scene(), Vec::new()).expect("cube ingests");
        Ok(json!({}))
    }
}

fn cube_hew_bytes() -> Vec<u8> {
    let mut d = Document::new();
    d.ingest(cube_scene(), Vec::new()).unwrap();
    d.save()
}

fn b64(bytes: &[u8]) -> String {
    api::commands::doc::encode_base64(bytes)
}

fn connect(grant: Grant, doc: &mut Document, host: &mut dyn Host) -> Connection {
    let mut conn = Connection::new(grant, "t");
    for (id, m) in [(0, "hew.meta.hello"), (1, "hew.doc.attach")] {
        let params = if id == 0 {
            json!({"protocol": 1})
        } else {
            json!({})
        };
        send(&mut conn, doc, host, id, m, params);
    }
    conn
}

fn send(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let req = Request {
        document: None,
        jsonrpc: "2.0".into(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    };
    let (DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r)) =
        conn.dispatch(&REGISTRY, doc, host, req)
    else {
        panic!("{method} replies");
    };
    serde_json::to_value(r).unwrap()
}

fn ok(r: &Value) -> Value {
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

fn refusal(r: &Value) -> String {
    let e = r
        .get("error")
        .unwrap_or_else(|| panic!("should refuse: {r}"));
    if e["code"] == json!(-32602) {
        return "invalid_params".into();
    }
    if e["code"] == json!(-32001) {
        return "not_granted".into();
    }
    e["data"]["refusal"].as_str().unwrap().to_string()
}

fn import(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host, params: Value) -> Value {
    send(
        conn,
        doc,
        host,
        10,
        "hew.component.import_definition",
        params,
    )
}

#[test]
fn import_hew_bytes_makes_one_unplaced_definition_without_the_host() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew", "name": "Hinge" }),
    );
    let result = ok(&r);
    assert_eq!(result["reused"], json!(false));
    assert_eq!(result["report"], json!({ "objects": 1, "members": 1 }));
    assert!(result["definition"].as_str().unwrap().starts_with("cmp_"));
    assert!(doc.instance_ids().is_empty());
    assert_eq!(doc.component_ids().len(), 1);
    assert_eq!(doc.component_name(doc.component_ids()[0]), Some("Hinge"));
}

#[test]
fn import_stl_goes_through_the_host_into_a_scratch_document_unrecorded() {
    let mut doc = Document::new();
    let mut host = CubeHost::default();
    let mut conn = connect(Grant::core(), &mut doc, &mut host);
    let r = import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({ "bytes_base64": b64(b"solid"), "format": "STL", "units": "mm" }),
    );
    let result = ok(&r);
    assert_eq!(result["report"]["objects"], json!(1));
    assert_eq!(host.calls, vec![("import.stl".to_string(), false)]);
    assert!(
        doc.top_level_nodes().is_empty(),
        "nothing lands in the world"
    );
    assert_eq!(doc.component_name(doc.component_ids()[0]), Some("Imported"));
}

#[test]
fn import_same_bytes_twice_reuses_with_one_undo_entry() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let params = json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew" });
    let a = ok(&import(&mut conn, &mut doc, &mut NoHost, params.clone()));
    let depth = doc.undo_depth();
    let b = ok(&import(&mut conn, &mut doc, &mut NoHost, params));
    assert_eq!(b["reused"], json!(true));
    assert_eq!(a["definition"], b["definition"]);
    assert_eq!(doc.undo_depth(), depth);
    assert_eq!(doc.component_ids().len(), 1);
}

#[test]
fn import_same_bytes_as_a_different_format_does_not_reuse() {
    let mut doc = Document::new();
    let mut host = CubeHost::default();
    let mut conn = connect(Grant::core(), &mut doc, &mut host);
    let bytes = b64(b"same bytes");
    let a = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({ "bytes_base64": bytes, "format": "glb" }),
    ));
    let b = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({ "bytes_base64": bytes, "format": "gltf" }),
    ));
    assert_eq!(b["reused"], json!(false));
    assert_ne!(a["definition"], b["definition"]);
}

#[test]
fn a_hundred_placements_are_one_definition() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let params = json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew" });
    let def = ok(&import(&mut conn, &mut doc, &mut NoHost, params.clone()))["definition"].clone();
    for i in 0..100 {
        let r = send(
            &mut conn,
            &mut doc,
            &mut NoHost,
            20 + i,
            "hew.component.place",
            json!({ "component": def }),
        );
        assert!(r.get("error").is_none(), "{r}");
    }
    assert_eq!(doc.instance_ids().len(), 100);
    assert_eq!(doc.component_ids().len(), 1);
    // Importing again still finds it.
    let again = ok(&import(&mut conn, &mut doc, &mut NoHost, params));
    assert_eq!(again["reused"], json!(true));
}

#[test]
fn import_undo_is_one_step_and_a_reuse_made_no_entry() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let params = json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew" });
    let before = doc.undo_depth();
    import(&mut conn, &mut doc, &mut NoHost, params.clone());
    import(&mut conn, &mut doc, &mut NoHost, params);
    assert_eq!(doc.undo_depth(), before + 1);
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        30,
        "hew.history.undo",
        json!({}),
    );
    assert!(r.get("error").is_none(), "{r}");
    assert!(doc.component_ids().is_empty());
}

#[test]
fn parameter_errors_are_invalid_params() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let good = b64(&cube_hew_bytes());
    for params in [
        json!({ "format": "hew" }),
        json!({ "bytes_base64": good }),
        json!({ "bytes_base64": good, "format": "obj" }),
        json!({ "bytes_base64": "!!", "format": "hew" }),
        json!({ "bytes_base64": good, "format": "hew", "path": "/x.hew" }),
        json!({ "bytes_base64": good, "format": "hew", "name": "a".repeat(129) }),
        json!({ "bytes_base64": good, "format": "hew", "name": "bad\nname" }),
    ] {
        let r = import(&mut conn, &mut doc, &mut NoHost, params.clone());
        assert_eq!(refusal(&r), "invalid_params", "{params}");
    }
    assert!(doc.component_ids().is_empty());
}

#[test]
fn a_picture_refuses_unsupported_for_definition() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(b"png"), "format": "png" }),
    );
    assert_eq!(refusal(&r), "unsupported_for_definition");
}

#[test]
fn bad_hew_bytes_refuse_load_failed_and_empty_items_refuse() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(b"nope"), "format": "hew" }),
    );
    assert_eq!(refusal(&r), "load_failed");
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(&Document::new().save()), "format": "hew" }),
    );
    assert_eq!(refusal(&r), "empty_group");
}

#[test]
fn stl_without_a_capable_host_refuses_host_capability_missing() {
    let mut doc = Document::new();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(b"x"), "format": "stl", "units": "m" }),
    );
    assert_eq!(refusal(&r), "host_capability_missing");
}

#[test]
fn import_in_a_component_session_refuses_and_changes_nothing() {
    let mut doc = Document::new();
    doc.ingest(cube_scene(), Vec::new()).unwrap();
    let o = doc.live_object_ids()[0];
    let (_, inst, _) = doc.make_component(&[kernel::NodeId::Object(o)]).unwrap();
    doc.open_explode_session(inst).unwrap();
    let mut conn = connect(Grant::core(), &mut doc, &mut NoHost);
    let hash = doc.state_hash();
    let r = import(
        &mut conn,
        &mut doc,
        &mut NoHost,
        json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew" }),
    );
    assert_eq!(refusal(&r), "explode_session_scope");
    assert_eq!(doc.state_hash(), hash);
}

fn plugin(perms: &[Permission]) -> Grant {
    let set: BTreeSet<Permission> = perms.iter().cloned().collect();
    Grant::plugin("com.example.p", None, set)
}

#[test]
fn a_model_write_grant_can_import_and_a_read_only_grant_cannot() {
    let params = json!({ "bytes_base64": b64(&cube_hew_bytes()), "format": "hew" });
    let mut doc = Document::new();
    let mut conn = connect(plugin(&[Permission::ModelWrite]), &mut doc, &mut NoHost);
    ok(&import(&mut conn, &mut doc, &mut NoHost, params.clone()));
    let mut doc = Document::new();
    let mut conn = connect(plugin(&[Permission::ModelRead]), &mut doc, &mut NoHost);
    let r = import(&mut conn, &mut doc, &mut NoHost, params.clone());
    assert_eq!(refusal(&r), "not_granted");
    // documents.save alone is the old import permission: it does not help.
    let mut conn = connect(plugin(&[Permission::DocumentsSave]), &mut doc, &mut NoHost);
    let r = import(&mut conn, &mut doc, &mut NoHost, params);
    assert_eq!(refusal(&r), "not_granted");
}

#[test]
fn the_registry_declares_it_a_document_mutation() {
    let d = REGISTRY.get("hew.component.import_definition").unwrap();
    assert!(d.mutates_document);
    assert!(d.implemented);
}

#[test]
fn different_units_or_images_are_a_different_definition() {
    let mut doc = Document::new();
    let mut host = CubeHost::default();
    let mut conn = connect(Grant::core(), &mut doc, &mut host);
    let bytes = b64(b"solid x");
    let first = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "stl", "units": "mm"}),
    ));
    let same = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "stl", "units": "mm"}),
    ));
    assert_eq!(same["reused"], json!(true));
    let other_units = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "stl", "units": "m"}),
    ));
    assert_eq!(other_units["reused"], json!(false));
    assert_ne!(other_units["definition"], first["definition"]);
    let img = |data: &[u8]| json!({"a.png": {"bytes_base64": b64(data), "format": "png"}});
    let with_a = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "dae", "images": img(b"one")}),
    ));
    let with_b = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "dae", "images": img(b"two")}),
    ));
    let with_a_again = ok(&import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": bytes, "format": "dae", "images": img(b"one")}),
    ));
    assert_eq!(with_b["reused"], json!(false));
    assert_ne!(with_a["definition"], with_b["definition"]);
    assert_eq!(with_a_again["definition"], with_a["definition"]);
    assert_eq!(with_a_again["reused"], json!(true));
}

#[test]
fn the_cap_counts_images_and_is_checked_before_decoding() {
    let mut doc = Document::new();
    let mut host = CubeHost::default();
    let mut conn = connect(Grant::core(), &mut doc, &mut host);
    // 90 MiB of base64 that is not even valid base64: refused by size, so
    // it never reaches the decoder (which would say invalid params).
    let huge = "!".repeat(90 << 20);
    let r = import(
        &mut conn,
        &mut doc,
        &mut host,
        json!({"bytes_base64": b64(b"x"), "format": "dae",
               "images": {"a.png": {"bytes_base64": huge, "format": "png"}}}),
    );
    assert_eq!(refusal(&r), "import_too_large");
}
