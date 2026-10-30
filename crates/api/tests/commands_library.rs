//! Conformance coverage for `hew.library.*` (docs/agents/HEW_API.md §8.1;
//! docs/design/v1.1-cycle.md's Lane B): `list`/`describe`/`remove`/
//! `update_meta` against a `FakeHost` standing in for a real filesystem-
//! backed one, and `insert`/`save` — which touch the document through the
//! same kernel entry points the UI's own Library flows use
//! (`Document::insert_document`/`extract_item`/`stamp_library_source`) —
//! proving the geometry and provenance actually round-trip.

use api::{
    Connection, DispatchOutcome, Grant, Host, LibraryItemEntry, LibraryListing, LibraryReadResult,
    LibraryWriteResult, LibraryWriteTarget, NoHost, Refusal, Request, RequestId, Response, codes,
};
use kernel::{AttrTarget, Document, EntityRef, NodeId};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// ----------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello_attach(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        host,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
    let DispatchOutcome::Reply(r) =
        conn.dispatch(&REGISTRY, doc, host, req(1, "hew.doc.attach", json!({})))
    else {
        panic!("attach replies")
    };
    assert!(r.error.is_none(), "attach failed: {:?}", r.error);
}

fn call(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, host, req(id, method, params))
    else {
        panic!("{method} replies")
    };
    r
}

fn call_ok(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, id, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

fn call_err(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, id, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(
        err.code,
        codes::REFUSED,
        "{method}'s failure should be a typed refusal"
    );
    err.data.expect("a refusal carries the canonical §4.4 data")
}

fn new_conn() -> Connection {
    Connection::new(Grant::core(), "test")
}

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Draws a rectangle then extrudes it into a solid, in one transaction,
/// against a plain `NoHost` connection (`hew.sketch.*`/`hew.solid.*` are
/// `Served::Kernel` — no host effect involved) — the box `hew.library.save`
/// tests below extract.
fn build_box(conn: &mut Connection, doc: &mut Document) -> kernel::ObjectId {
    let result = call_ok(
        conn,
        doc,
        &mut NoHost,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]) },
                { "method": "hew.solid.extrude", "as": "box", "params": {
                    "region": { "$ref": "profile#/region_id" }, "distance": 1.0
                }}
            ]
        }),
    );
    let object_id = result["results"][1]["object_id"].as_str().unwrap();
    let EntityRef::Object(o) = api::IdResolver::new(doc)
        .resolve(object_id)
        .expect("just-created object resolves")
    else {
        panic!("expected an object");
    };
    o
}

/// A minimal, standalone `.hew` item — no geometry — with `hew.library`
/// meta stamped, for the read-side (list/describe/remove/update_meta)
/// tests that only need SOME bytes and don't insert them.
fn bare_item_bytes(id: &str, name: &str, category: &str) -> Vec<u8> {
    let mut doc = Document::new();
    doc.attr_set(
        AttrTarget::Document,
        "hew.library",
        "id",
        kernel::AttrValue::Text(id.to_string()),
    )
    .unwrap();
    doc.attr_set(
        AttrTarget::Document,
        "hew.library",
        "name",
        kernel::AttrValue::Text(name.to_string()),
    )
    .unwrap();
    doc.attr_set(
        AttrTarget::Document,
        "hew.library",
        "category",
        kernel::AttrValue::Text(category.to_string()),
    )
    .unwrap();
    doc.save()
}

fn item_summary(bytes: &[u8]) -> kernel::ItemSummary {
    kernel::read_item_summary(bytes).expect("valid item bytes")
}

// ------------------------------------------------------------- FakeHost

/// An in-memory stand-in for a filesystem-backed library — no real disk,
/// just enough to prove `commands/library.rs` parses params, resolves
/// `item` id-or-path correctly, and delegates to the `Host` trait's four
/// `library_*` methods faithfully.
#[derive(Default)]
struct FakeHost {
    items: BTreeMap<String, Vec<u8>>,
    write_log: Vec<(String, Vec<u8>)>,
    remove_log: Vec<String>,
    /// `hew.library.save`'s `source_doc` default — stands in for
    /// `CliHost::working_document_path`'s `--file`-opened path.
    working_path: Option<String>,
}

impl FakeHost {
    fn seed(&mut self, path: &str, bytes: Vec<u8>) {
        self.items.insert(path.to_string(), bytes);
    }

    fn entry_of(path: &str, bytes: &[u8]) -> LibraryItemEntry {
        let summary = item_summary(bytes);
        let meta = summary
            .doc_attrs
            .get("hew.library")
            .cloned()
            .unwrap_or(Value::Null);
        let obj = meta.as_object();
        let text = |k: &str| {
            obj.and_then(|o| o.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let category = text("category").unwrap_or_else(|| "model".to_string());
        let name = text("name").unwrap_or_else(|| path.to_string());
        LibraryItemEntry {
            path: path.to_string(),
            id: text("id"),
            name,
            category,
            keywords: Vec::new(),
            collection: text("collection"),
            saved_at: text("savedAt"),
            size: bytes.len() as u64,
            mtime_ms: 0,
            summary: Some(summary),
            error: None,
        }
    }
}

fn fake_hash(bytes: &[u8]) -> String {
    format!("hash-{:x}", bytes.len())
}

impl Host for FakeHost {
    fn library_list(&self) -> Result<LibraryListing, Refusal> {
        Ok(LibraryListing {
            folder: "/fake/library".to_string(),
            items: self
                .items
                .iter()
                .map(|(path, bytes)| FakeHost::entry_of(path, bytes))
                .collect(),
        })
    }

    fn library_read(&self, path: &str) -> Result<LibraryReadResult, Refusal> {
        let bytes = self
            .items
            .get(path)
            .ok_or_else(|| Refusal::api("load_failed", "no such fake item"))?;
        Ok(LibraryReadResult {
            path: path.to_string(),
            bytes: bytes.clone(),
            content_hash: fake_hash(bytes),
        })
    }

    fn library_write(
        &mut self,
        target: LibraryWriteTarget,
        bytes: &[u8],
    ) -> Result<LibraryWriteResult, Refusal> {
        let path = match target {
            LibraryWriteTarget::New { category, name, id } => {
                let dir = match category {
                    "component" => "Components",
                    "material" => "Materials",
                    _ => "Models",
                };
                let slug: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                format!(
                    "{dir}/{}-{}.hew",
                    slug.to_lowercase(),
                    &id[..id.len().min(6)]
                )
            }
            LibraryWriteTarget::Existing { path } => path.to_string(),
        };
        self.write_log.push((path.clone(), bytes.to_vec()));
        self.items.insert(path.clone(), bytes.to_vec());
        Ok(LibraryWriteResult {
            content_hash: fake_hash(bytes),
            path,
        })
    }

    fn library_remove(&mut self, path: &str) -> Result<(), Refusal> {
        self.remove_log.push(path.to_string());
        self.items.remove(path);
        Ok(())
    }

    fn working_document_path(&self) -> Option<&str> {
        self.working_path.as_deref()
    }
}

// ==================================================== hew.library.list

#[test]
fn list_reports_folder_and_every_seeded_item() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    host.seed(
        "Components/a.hew",
        bare_item_bytes("id-a", "A", "component"),
    );
    host.seed("Models/b.hew", bare_item_bytes("id-b", "B", "model"));
    hello_attach(&mut conn, &mut doc, &mut host);

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.list",
        json!({}),
    );
    assert_eq!(result["folder"], "/fake/library");
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[test]
fn list_filters_by_category_and_query() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    host.seed(
        "Components/chair.hew",
        bare_item_bytes("id-1", "Chair", "component"),
    );
    host.seed(
        "Models/house.hew",
        bare_item_bytes("id-2", "House", "model"),
    );
    hello_attach(&mut conn, &mut doc, &mut host);

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.list",
        json!({"category": "component"}),
    );
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Chair");

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.list",
        json!({"query": "hou"}),
    );
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "House");
}

#[test]
fn list_rejects_an_unknown_category_as_a_params_error() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.list",
        json!({"category": "sofa"}),
    );
    let err = r.error.expect("bad category refuses");
    assert_eq!(err.code, codes::INVALID_PARAMS);
}

#[test]
fn list_refuses_host_capability_missing_without_a_library_host() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.library.list",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

// ================================================== hew.library.describe

#[test]
fn describe_returns_meta_summary_and_attrs_by_id_or_path() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    host.seed(
        "Models/house.hew",
        bare_item_bytes("id-house", "House", "model"),
    );
    hello_attach(&mut conn, &mut doc, &mut host);

    let by_id = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.describe",
        json!({"item": "id-house"}),
    );
    assert_eq!(by_id["name"], "House");
    assert_eq!(by_id["attrs"]["hew.library"]["id"], "id-house");

    let by_path = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.describe",
        json!({"item": "Models/house.hew"}),
    );
    assert_eq!(by_path["id"], "id-house");
}

#[test]
fn describe_refuses_unknown_library_item() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.describe",
        json!({"item": "nope"}),
    );
    assert_eq!(data["refusal"], "unknown_library_item");
}

// ==================================================== hew.library.remove

#[test]
fn remove_deletes_by_id_and_reports_the_removed_path() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    host.seed(
        "Models/house.hew",
        bare_item_bytes("id-house", "House", "model"),
    );
    hello_attach(&mut conn, &mut doc, &mut host);

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.remove",
        json!({"item": "id-house"}),
    );
    assert_eq!(result["removed"], "Models/house.hew");
    assert_eq!(host.remove_log, vec!["Models/house.hew".to_string()]);
    assert!(host.items.is_empty());
}

// ================================================ hew.library.update_meta

#[test]
fn update_meta_edits_in_place_without_changing_the_path() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    host.seed(
        "Components/chair.hew",
        bare_item_bytes("id-chair", "Chair", "component"),
    );
    hello_attach(&mut conn, &mut doc, &mut host);

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.update_meta",
        json!({"item": "id-chair", "name": "Dining Chair", "keywords": ["oak"], "collection": "Furniture"}),
    );
    assert_eq!(result["name"], "Dining Chair");
    assert_eq!(result["keywords"], json!(["oak"]));
    assert_eq!(result["collection"], "Furniture");
    assert_eq!(result["id"], "id-chair", "the id is untouched");

    // The write landed at the SAME path — a display-name edit must not
    // mint a new file name.
    assert_eq!(host.write_log.len(), 1);
    assert_eq!(host.write_log[0].0, "Components/chair.hew");
    let rewritten = kernel::Document::load(&host.write_log[0].1).unwrap();
    let dict = rewritten.attr_get(&AttrTarget::Document).unwrap().unwrap();
    let lib = dict.get("hew.library").unwrap();
    assert_eq!(
        lib.get("name"),
        Some(&kernel::AttrValue::Text("Dining Chair".to_string()))
    );
}

// ===================================================== hew.library.insert

#[test]
fn insert_grafts_geometry_and_a_second_insert_reuses_the_definition() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let root = build_box(&mut conn, &mut doc);

    // Build the library item exactly like `hew.library.save` would: wrap
    // the box as a component and stamp `hew.library` meta.
    let mut item = doc.extract_item(&[NodeId::Object(root)], true).unwrap();
    item.attr_set(
        AttrTarget::Document,
        "hew.library",
        "id",
        kernel::AttrValue::Text("box-id".to_string()),
    )
    .unwrap();
    item.attr_set(
        AttrTarget::Document,
        "hew.library",
        "name",
        kernel::AttrValue::Text("Box".to_string()),
    )
    .unwrap();
    item.attr_set(
        AttrTarget::Document,
        "hew.library",
        "category",
        kernel::AttrValue::Text("component".to_string()),
    )
    .unwrap();
    host.seed("Components/box.hew", item.save());

    let mut fresh = Document::new();
    let mut conn2 = new_conn();
    hello_attach(&mut conn2, &mut fresh, &mut host);

    // `hew.library.insert` is `ModelMutating` class, so a plain (non-
    // `transact`) request dispatches as exactly a one-command transaction
    // (§6.1) — the result is `results[0]`, not the bare command result
    // (unlike the `ReadOnly`-class `list`/`describe`/`save`/`remove`/
    // `update_meta` commands elsewhere in this file, which run bare).
    let first = call_ok(
        &mut conn2,
        &mut fresh,
        &mut host,
        2,
        "hew.library.insert",
        json!({"item": "box-id", "at": [2.0, 0.0, 0.0]}),
    );
    let first = &first["results"][0];
    assert_eq!(first["objects_added"], 1);
    assert_eq!(first["definitions_added"], 1);
    assert_eq!(first["definitions_reused"], 0);

    let second = call_ok(
        &mut conn2,
        &mut fresh,
        &mut host,
        3,
        "hew.library.insert",
        json!({"item": "box-id", "at": [4.0, 0.0, 0.0]}),
    );
    let second = &second["results"][0];
    assert_eq!(
        second["definitions_reused"], 1,
        "the second insert of the same item version must reuse the definition, not copy it again"
    );
    assert_eq!(second["definitions_added"], 0);
}

#[test]
fn insert_requires_exactly_one_of_item_or_bytes_base64() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);

    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.insert",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);

    let bytes = bare_item_bytes("x", "X", "model");
    let b64 = base64_encode(&bytes);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.insert",
        json!({"item": "x", "bytes_base64": b64}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

// ======================================================= hew.library.save

#[test]
fn save_with_a_selection_extracts_wraps_as_component_and_stamps_provenance() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let root = build_box(&mut conn, &mut doc);
    let resolver = api::IdResolver::new(&doc);
    let public_id = resolver.public_of(&doc, &EntityRef::Object(root)).unwrap();

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.save",
        json!({"selection": [public_id], "name": "Box", "keywords": ["k1"]}),
    );
    let path = result["path"].as_str().unwrap();
    assert!(path.starts_with("Components/"), "{path}");
    assert!(result["id"].is_string());

    // The write actually reached the host with a real item.
    assert_eq!(host.write_log.len(), 1);
    let item = kernel::Document::load(&host.write_log[0].1).unwrap();
    assert_eq!(item.component_ids().len(), 1, "wrapped as a component item");

    // The ORIGINAL document's saved node now carries `hew.library`
    // provenance (`Document::stamp_library_source`) — the same "in this
    // model" bookkeeping the UI's Save-to-Library flow performs.
    let dict = doc
        .attr_get(&AttrTarget::Entity(EntityRef::Object(root)))
        .unwrap()
        .unwrap();
    assert!(
        dict.contains_key("hew.library"),
        "provenance must be stamped"
    );
}

/// The palette's Save to Library: one material, alone, as a material item
/// that inserts back with its texture intact.
#[test]
fn save_a_material_writes_a_material_item_that_inserts_back() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    build_box(&mut conn, &mut doc);
    let png = api::commands::doc::encode_base64(b"\x89PNG\r\n\x1a\nimage");
    let made = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.material.create",
        json!({"name": "Bamboo", "texture": {"bytes_base64": png, "world_size": [0.2, 1.0]}}),
    );
    let material = made["results"][0]["material"].as_str().unwrap().to_string();

    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.library.save",
        json!({"material": material, "name": "Bamboo", "return_bytes": true}),
    );
    let path = saved["path"].as_str().unwrap();
    assert!(path.starts_with("Materials/"), "{path}");
    let item = Document::load(&host.write_log[0].1).unwrap();
    assert!(
        item.visible_object_ids().is_empty(),
        "the material alone, no geometry"
    );
    let mats: Vec<_> = item.materials().values().collect();
    assert_eq!(mats.len(), 1);
    assert_eq!(mats[0].name, "Bamboo");
    assert_eq!(mats[0].texture.as_ref().unwrap().world_size, [0.2, 1.0]);
    assert_eq!(
        item_summary(&host.write_log[0].1).doc_attrs["hew.library"]["category"],
        "material"
    );

    // Into a fresh document it adds just the material, with no undo entry
    // (a palette addition, like hew.material.create).
    let mut fresh = Document::new();
    let mut conn2 = new_conn();
    hello_attach(&mut conn2, &mut fresh, &mut host);
    let depth = fresh.undo_depth();
    let inserted = call_ok(
        &mut conn2,
        &mut fresh,
        &mut host,
        5,
        "hew.library.insert",
        json!({"bytes_base64": saved["bytes_base64"]}),
    );
    assert_eq!(inserted["results"][0]["materials_added"], 1);
    assert!(
        fresh
            .materials()
            .values()
            .any(|m| m.name == "Bamboo" && m.texture.is_some())
    );
    assert_eq!(
        inserted["results"][0]["materials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Again: the identical material is reused, not duplicated.
    let again = call_ok(
        &mut conn2,
        &mut fresh,
        &mut host,
        6,
        "hew.library.insert",
        json!({"bytes_base64": saved["bytes_base64"]}),
    );
    assert_eq!(again["results"][0]["materials_reused"], 1);
    assert_eq!(
        again["results"][0]["materials"],
        inserted["results"][0]["materials"]
    );
    assert_eq!(fresh.materials().len(), 1);
    assert_eq!(
        fresh.undo_depth(),
        depth,
        "palette additions record no undo entry"
    );
    // Placement arguments don't apply to a material item.
    let r = call(
        &mut conn2,
        &mut fresh,
        &mut host,
        7,
        "hew.library.insert",
        json!({"bytes_base64": saved["bytes_base64"], "at": [1, 0, 0]}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);

    // A material with a selection, or under another category, is a params
    // error; an unknown material refuses typed.
    for params in [
        json!({"material": material, "selection": ["obj_1"], "name": "X"}),
        json!({"material": material, "category": "component", "name": "X"}),
    ] {
        let r = call(
            &mut conn,
            &mut doc,
            &mut host,
            6,
            "hew.library.save",
            params,
        );
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    }
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        7,
        "hew.library.save",
        json!({"material": "mat_ffffff", "name": "X"}),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    assert_eq!(host.write_log.len(), 1, "nothing else was written");
}

#[test]
fn save_with_no_selection_saves_the_whole_document_as_a_model_item() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    build_box(&mut conn, &mut doc);

    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.save",
        json!({"name": "Whole Doc"}),
    );
    let path = result["path"].as_str().unwrap();
    assert!(path.starts_with("Models/"), "{path}");
}

/// A model item is a separate document: it never carries the source
/// document's id, so documents opened from it do not share that identity.
#[test]
fn a_model_item_carries_no_document_id() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    build_box(&mut conn, &mut doc);
    doc.ensure_document_id([0x7e; 16]);

    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.save",
        json!({"name": "Whole Doc"}),
    );
    let (_, bytes) = host.write_log.last().expect("the item was written");
    let item = Document::load(bytes).expect("the item loads");
    assert_eq!(item.document_id(), None);
    assert!(doc.document_id().is_some(), "the source keeps its own");
}

#[test]
fn save_return_bytes_includes_the_item_bytes_only_when_asked() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);

    let without = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.save",
        json!({"name": "A"}),
    );
    assert!(without.get("bytes_base64").is_none());

    let with = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.save",
        json!({"name": "B", "return_bytes": true}),
    );
    assert!(with["bytes_base64"].is_string());
}

/// `hew.library.save`'s `source_doc`: an explicit value always wins;
/// absent, it defaults to `Host::working_document_path` (`CliHost`'s
/// `--file`-opened path, in real use); with neither, the key is left off
/// the item entirely rather than stamping an empty string.
#[test]
fn save_source_doc_defaults_from_the_host_and_an_explicit_value_wins() {
    let mut conn = new_conn();
    let mut doc = Document::new();

    // Neither an explicit source_doc nor a host that knows one: omitted.
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.save",
        json!({"name": "No Source"}),
    );
    let item = kernel::Document::load(&host.write_log[0].1).unwrap();
    let dict = item.attr_get(&AttrTarget::Document).unwrap().unwrap();
    assert!(
        !dict.get("hew.library").unwrap().contains_key("sourceDoc"),
        "no source_doc anywhere should mean no sourceDoc key at all"
    );

    // The host knows a working path: it becomes the default.
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost {
        working_path: Some("/tmp/from-host.hew".to_string()),
        ..FakeHost::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.save",
        json!({"name": "Host Default"}),
    );
    let item = kernel::Document::load(&host.write_log[0].1).unwrap();
    let dict = item.attr_get(&AttrTarget::Document).unwrap().unwrap();
    assert_eq!(
        dict.get("hew.library").unwrap().get("sourceDoc"),
        Some(&kernel::AttrValue::Text("/tmp/from-host.hew".to_string()))
    );

    // An explicit source_doc overrides the host's default.
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost {
        working_path: Some("/tmp/from-host.hew".to_string()),
        ..FakeHost::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.save",
        json!({"name": "Explicit Wins", "source_doc": "/tmp/explicit.hew"}),
    );
    let item = kernel::Document::load(&host.write_log[0].1).unwrap();
    let dict = item.attr_get(&AttrTarget::Document).unwrap().unwrap();
    assert_eq!(
        dict.get("hew.library").unwrap().get("sourceDoc"),
        Some(&kernel::AttrValue::Text("/tmp/explicit.hew".to_string()))
    );
}

/// `hew.library.insert` composes with `$ref` inside a multi-command
/// `hew.doc.transact` (§6.1/§6.2's chaining shape) exactly like any other
/// command: labeling the insert `"placed"` and referencing
/// `{"$ref": "placed#/roots/0"}` from a LATER command in the same
/// envelope must resolve to the first root's public id — proving
/// `substitute_refs` reaches into `roots`, a JSON ARRAY result field, not
/// just a bare object one (`crates/api/src/transact.rs`'s `Value::Array`
/// arm).
#[test]
fn insert_composes_with_a_later_ref_to_its_roots_array_in_one_transaction() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let root = build_box(&mut conn, &mut doc);
    let item = doc.extract_item(&[NodeId::Object(root)], true).unwrap();
    host.seed("Components/box.hew", item.save());

    let mut fresh = Document::new();
    let mut conn2 = new_conn();
    hello_attach(&mut conn2, &mut fresh, &mut host);

    let result = call_ok(
        &mut conn2,
        &mut fresh,
        &mut host,
        2,
        "hew.doc.transact",
        json!({
            "label": "insert then rename",
            "commands": [
                { "method": "hew.library.insert", "as": "placed", "params": {
                    "item": "Components/box.hew"
                } },
                { "method": "hew.entity.rename", "params": {
                    "id": { "$ref": "placed#/roots/0" },
                    "name": "Renamed Via Ref"
                } },
            ],
        }),
    );
    let rename_result = &result["results"][1];
    assert!(
        rename_result.get("error").is_none(),
        "the rename must not itself carry an error field: {rename_result}"
    );

    // Confirm the SAME entity the insert created actually got renamed —
    // not just that the rename call didn't refuse.
    let placed_id = result["results"][0]["roots"][0]
        .as_str()
        .expect("insert's roots[0] is a public id string");
    let entity = api::IdResolver::new(&fresh)
        .resolve(placed_id)
        .expect("the inserted root resolves");
    let EntityRef::Instance(instance_id) = entity else {
        panic!("a wrap-as-component insert's root is an instance: {entity:?}");
    };
    assert_eq!(fresh.instance_name(instance_id), Some("Renamed Via Ref"));
}

/// RFC 4648 base64 — a test-fixture-only encoder mirroring the one
/// `crates/api/src/commands/doc.rs` hand-rolls (private to that crate).
fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
