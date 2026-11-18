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
                    "profile" => "Profiles",
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

// ======================================================== profile items
//
// Saving a profile (`profile_chain`) belongs to a Preview feature
// (`fillet-profiles` in preview-features.json): these run in a preview
// build only; a production build refuses the parameter `preview_only`
// (preview_wall.rs).

/// Draws a three-point open chain in the ground plane and returns the
/// sketch id with the two edge locators in drawing order.
#[cfg(hew_preview)]
fn draw_profile_chain(conn: &mut Connection, doc: &mut Document) -> (String, Vec<Value>) {
    let r = call_ok(
        conn,
        doc,
        &mut NoHost,
        20,
        "hew.sketch.draw_line",
        json!({
            "plane": { "ground": true },
            "points": [[0.0, 0.05, 0.0], [-0.02, 0.02, 0.0], [-0.05, 0.0, 0.0]]
        }),
    );
    let sketch = r["results"][0]["sketch"].as_str().unwrap().to_string();
    let edges = vec![
        json!({ "from": [0.0, 0.05, 0.0], "to": [-0.02, 0.02, 0.0] }),
        json!({ "from": [-0.02, 0.02, 0.0], "to": [-0.05, 0.0, 0.0] }),
    ];
    (sketch, edges)
}

#[test]
#[cfg(hew_preview)]
fn save_a_profile_chain_then_fillet_with_it_and_replace_it() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let (sketch, edges) = draw_profile_chain(&mut conn, &mut doc);
    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        21,
        "hew.library.save",
        json!({ "name": "My Edge", "profile_chain": { "sketch": sketch, "edges": edges } }),
    );
    let path = saved["path"].as_str().unwrap().to_string();
    let id = saved["id"].as_str().unwrap().to_string();
    assert!(path.starts_with("Profiles/"), "{path}");
    let bytes = host.items.get(&path).unwrap().clone();
    let summary = item_summary(&bytes);
    assert!(summary.profile_chain_only);
    assert_eq!(summary.doc_attrs["hew.library"]["category"], "profile");

    // Fillet a box edge with the saved profile, by item id.
    let b = build_box(&mut conn, &mut doc);
    let object = api::IdResolver::new(&doc)
        .public_of(&doc, &EntityRef::Object(b))
        .unwrap();
    let r = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        22,
        "hew.solid.fillet",
        json!({
            "edges": [{ "object": object, "from": [0.0, 0.0, 1.0], "to": [1.0, 0.0, 1.0] }],
            "profile": id,
            "propagate": "none"
        }),
    );
    assert!((r["results"][0]["size"].as_f64().unwrap() - 0.05).abs() < 1e-9);
    // An unknown profile still refuses typed.
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        23,
        "hew.solid.fillet",
        json!({
            "edges": [{ "object": object, "from": [0.0, 1.0, 1.0], "to": [1.0, 1.0, 1.0] }],
            "profile": "no-such-item",
            "width": 0.1
        }),
    );
    assert_eq!(d["refusal"], "unknown_profile");

    // Replace keeps the id and overwrites the same file.
    let replaced = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        24,
        "hew.library.save",
        json!({
            "name": "My Edge v2",
            "replace": id,
            "profile_chain": { "sketch": sketch, "edges": edges }
        }),
    );
    assert_eq!(replaced["path"], json!(path));
    assert_eq!(replaced["id"], json!(id));
    assert_eq!(host.items.len(), 1);
}

#[test]
#[cfg(hew_preview)]
fn a_profile_item_cannot_be_inserted_and_save_refuses_a_bad_chain() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let (sketch, edges) = draw_profile_chain(&mut conn, &mut doc);
    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        21,
        "hew.library.save",
        json!({ "name": "Edge", "profile_chain": { "sketch": sketch, "edges": edges } }),
    );
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        22,
        "hew.library.insert",
        json!({ "item": saved["id"] }),
    );
    assert_eq!(d["refusal"], "profile_not_insertable");
    // One edge of a two-edge chain is still a chain; two unconnected are not.
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        23,
        "hew.library.save",
        json!({ "name": "Bad", "profile_chain": { "sketch": sketch, "edges": [] } }),
    );
    assert_eq!(d["refusal"], "invalid_profile");
    // Two edges that do not meet are not a chain either.
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        24,
        "hew.library.save",
        json!({
            "name": "Bad",
            "profile_chain": { "sketch": sketch, "edges": [edges[0].clone(), edges[0].clone()] }
        }),
    );
    assert_eq!(d["refusal"], "invalid_profile");
}

// ============================================ profile items, both builds
//
// A Library folder can hold profile items (a preview build wrote them, or
// a file was copied in). A production build tolerates them: it lists none,
// refuses to describe, edit or write over one (`preview_only`, naming the
// feature), and refuses to place one with copy that names no preview
// feature. A preview build serves them like any other item.

/// A Library holding one component item and one profile item.
fn library_with_a_profile() -> FakeHost {
    let mut host = FakeHost::default();
    host.seed(
        "Components/a.hew",
        bare_item_bytes("id-a", "A", "component"),
    );
    host.seed(
        "Profiles/edge.hew",
        bare_item_bytes("id-p", "Edge", "profile"),
    );
    host
}

#[test]
#[cfg(not(hew_preview))]
fn a_production_build_tolerates_profile_items_and_serves_none() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = library_with_a_profile();
    hello_attach(&mut conn, &mut doc, &mut host);
    let wall = |d: &Value, what: &str| {
        assert_eq!(d["refusal"], "preview_only", "{what}: {d}");
        assert_eq!(d["detail"]["feature"], "fillet", "{what}: {d}");
        assert!(
            d["explanation"]
                .as_str()
                .unwrap()
                .contains("Fillet and Chamfer"),
            "{what}: {d}"
        );
    };

    // Listing leaves the profile out, filtered or not.
    let listed = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.list",
        json!({}),
    );
    let paths: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["Components/a.hew"]);
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.list",
        json!({ "category": "profile" }),
    );
    wall(&d, "list by category");

    // Describe and update_meta refuse it; the component still answers.
    for (id, method, params) in [
        (4, "hew.library.describe", json!({ "item": "id-p" })),
        (
            5,
            "hew.library.describe",
            json!({ "item": "Profiles/edge.hew" }),
        ),
        (
            6,
            "hew.library.update_meta",
            json!({ "item": "id-p", "name": "X" }),
        ),
    ] {
        let d = call_err(&mut conn, &mut doc, &mut host, id, method, params);
        wall(&d, method);
    }
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        7,
        "hew.library.describe",
        json!({ "item": "id-a" }),
    );

    // Saving as a profile refuses, and so does writing over one, whatever
    // the category asked for.
    for (id, params) in [
        (8, json!({ "name": "P", "category": "profile" })),
        (9, json!({ "name": "P", "replace": "id-p" })),
        (
            10,
            json!({ "name": "P", "replace": "id-p", "category": "model" }),
        ),
        (
            11,
            json!({ "name": "P", "replace": "id-p", "category": "profile" }),
        ),
    ] {
        let d = call_err(
            &mut conn,
            &mut doc,
            &mut host,
            id,
            "hew.library.save",
            params.clone(),
        );
        wall(&d, &format!("save {params}"));
    }
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        12,
        "hew.doc.transact",
        json!({ "commands": [{ "method": "hew.library.save", "params": { "name": "P", "replace": "id-p" } }] }),
    );
    wall(&d, "save replace in a transaction");
    assert!(host.write_log.is_empty(), "nothing was written");

    // A typo is told the categories this build takes.
    let d = call(
        &mut conn,
        &mut doc,
        &mut host,
        13,
        "hew.library.save",
        json!({ "name": "P", "category": "profiles" }),
    );
    let message = d.error.unwrap().message;
    assert!(message.contains("\"model\""), "{message}");
    assert!(!message.contains("\"profile\""), "{message}");

    // Placing it refuses, in words that name no preview feature.
    for (id, params) in [
        (14, json!({ "item": "id-p" })),
        (
            15,
            json!({ "bytes_base64": base64_encode(&bare_item_bytes("id-q", "Q", "profile")) }),
        ),
    ] {
        let d = call_err(
            &mut conn,
            &mut doc,
            &mut host,
            id,
            "hew.library.insert",
            params,
        );
        assert_eq!(d["refusal"], "profile_not_insertable");
        let copy = d["explanation"].as_str().unwrap();
        assert_eq!(copy, api::preview::GATED_API_COPY[0].3);
        for f in api::preview::features() {
            assert!(!copy.contains(&f.name), "{copy} names {}", f.name);
            for t in &f.tools {
                assert!(!copy.contains(t.as_str()), "{copy} names {t}");
            }
        }
    }
    assert_eq!(host.items.len(), 2, "the profile file is still there");
}

#[test]
#[cfg(hew_preview)]
fn a_preview_build_serves_profile_items() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = library_with_a_profile();
    hello_attach(&mut conn, &mut doc, &mut host);
    let listed = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.library.list",
        json!({ "category": "profile" }),
    );
    assert_eq!(listed["items"][0]["path"], "Profiles/edge.hew");
    let d = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.describe",
        json!({ "item": "id-p" }),
    );
    assert_eq!(d["category"], "profile");
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.library.update_meta",
        json!({ "item": "id-p", "name": "Edge 2" }),
    );
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        5,
        "hew.library.insert",
        json!({ "item": "id-p" }),
    );
    assert_eq!(d["refusal"], "profile_not_insertable");
    assert_eq!(d["explanation"], api::preview::GATED_API_COPY[0].2);
}

// ============================================ Library updates (OW-2)

/// The public id of `entity` in `doc`.
fn public(doc: &Document, entity: EntityRef) -> String {
    api::IdResolver::new(doc)
        .public_of(doc, &entity)
        .expect("a live entity")
}

/// A library with one component item ("box-id"), saved the way
/// `hew.library.save` saves one: through the command, from a box.
fn library_with_a_box() -> FakeHost {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let root = build_box(&mut conn, &mut doc);
    let id = public(&doc, EntityRef::Object(root));
    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.library.save",
        json!({ "selection": [id], "name": "Box" }),
    );
    // Rename the item's id to a fixed one for the tests (a replace keeps
    // whatever id the item has).
    let path = saved["path"].as_str().unwrap().to_string();
    let mut item = Document::load(&host.items[&path]).unwrap();
    item.attr_set(
        AttrTarget::Document,
        "hew.library",
        "id",
        kernel::AttrValue::Text("box-id".into()),
    )
    .unwrap();
    host.items.insert(path, item.save());
    host
}

/// A document holding two inserted copies of "box-id": the connection,
/// the document, and the linked definition's public id.
fn model_with_two_boxes(host: &mut FakeHost) -> (Connection, Document, String) {
    let mut conn = new_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, host);
    for (n, x) in [(2, 0.0), (3, 3.0)] {
        call_ok(
            &mut conn,
            &mut doc,
            host,
            n,
            "hew.library.insert",
            json!({ "item": "box-id", "at": [x, 0.0, 0.0] }),
        );
    }
    assert_eq!(doc.component_ids().len(), 1);
    let def = public(&doc, EntityRef::Component(doc.component_ids()[0]));
    (conn, doc, def)
}

/// Edits "box-id" in the Library from a second document: insert it, raise
/// the box, and save it back over the item with `replace`.
fn edit_the_box_in_the_library(host: &mut FakeHost, rise: f64) {
    let mut conn = new_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, host);
    call_ok(
        &mut conn,
        &mut doc,
        host,
        2,
        "hew.library.insert",
        json!({ "item": "box-id" }),
    );
    let c = doc.component_ids()[0];
    let o = doc.def_members(c).unwrap()[0];
    let top = doc
        .object(o)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .unwrap();
    doc.apply_def_op(
        c,
        o,
        kernel::KernelOp::PushPull {
            face: top,
            distance: rise,
        },
    )
    .unwrap();
    let inst = public(&doc, EntityRef::Instance(doc.instance_ids()[0]));
    call_ok(
        &mut conn,
        &mut doc,
        host,
        3,
        "hew.library.save",
        json!({ "selection": [inst], "name": "Box", "replace": "box-id" }),
    );
}

fn states(status: &Value) -> Vec<String> {
    status["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["state"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn every_save_writes_prints_and_the_source_is_in_step_with_its_item() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let root = build_box(&mut conn, &mut doc);
    // An instance selection, so the source definition is stamped.
    let (_, inst, _) = doc.make_component(&[NodeId::Object(root)]).unwrap();
    let inst_id = public(&doc, EntityRef::Instance(inst));
    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.library.save",
        json!({ "selection": [inst_id], "name": "Box" }),
    );
    let bytes = &host.items[saved["path"].as_str().unwrap()];
    let summary = item_summary(bytes);
    let prints = summary.doc_attrs["hew.library"]["prints"]
        .as_object()
        .expect("the item carries its prints");
    assert_eq!(prints.len(), 1);
    let sid = summary.first_component_sid.clone().unwrap();
    let print = prints[&sid].as_str().unwrap();
    assert!(print.starts_with("p1:"), "{print}");
    // The source definition's base is that print.
    let c = doc.component_ids()[0];
    let stamp = doc.library_stamp(c).unwrap();
    assert_eq!(stamp.print.as_deref(), Some(print));
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        5,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step"]);
    assert_eq!(status["links"][0]["copies"], 1);
    assert_eq!(status["links"][0]["locked"], false);
    assert_eq!(status["links"][0]["item"], saved["id"]);
}

#[test]
fn saving_two_parts_links_each_to_its_own_definition_and_both_read_in_step() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    // Two different parts: a 1 m box and a box twice as wide.
    let a = build_box(&mut conn, &mut doc);
    let wide = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect([4.0, 0.0, 0.0], [6.0, 1.0, 0.0]) },
                { "method": "hew.solid.extrude", "params": {
                    "region": { "$ref": "profile#/region_id" }, "distance": 1.0
                }}
            ]
        }),
    );
    let Some(EntityRef::Object(b)) =
        api::IdResolver::new(&doc).resolve(wide["results"][1]["object_id"].as_str().unwrap())
    else {
        panic!("expected an object");
    };
    let (_, ai, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    let (_, bi, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let sel = [
        public(&doc, EntityRef::Instance(ai)),
        public(&doc, EntityRef::Instance(bi)),
    ];
    let saved = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        4,
        "hew.library.save",
        json!({ "selection": sel, "name": "Pair" }),
    );
    let bytes = &host.items[saved["path"].as_str().unwrap()];
    let prints = item_summary(bytes).doc_attrs["hew.library"]["prints"].clone();
    // Each definition's base is the item's print of its own copy.
    let mut sids = Vec::new();
    for c in doc.component_ids() {
        let stamp = doc.library_stamp(c).unwrap();
        let sid = stamp.def_sid.unwrap().to_string();
        assert_eq!(stamp.print.as_deref(), prints[&sid].as_str());
        sids.push(sid);
    }
    assert_ne!(sids[0], sids[1], "two definitions, two ids");
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        5,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step", "in_step"]);
}

#[test]
fn a_newer_item_reads_library_newer_and_update_all_brings_it_in_step() {
    let mut host = library_with_a_box();
    let (mut conn, mut doc, def) = model_with_two_boxes(&mut host);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        10,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step"]);
    assert_eq!(status["links"][0]["definition"], def);
    assert_eq!(status["links"][0]["copies"], 2);

    edit_the_box_in_the_library(&mut host, 0.5);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        11,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["library_newer"]);

    let poses: Vec<_> = doc
        .instance_ids()
        .iter()
        .map(|&i| doc.instance_pose(i).unwrap().to_affine())
        .collect();
    let before = doc.state_hash();
    let out = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        12,
        "hew.library.update",
        json!({ "all": true }),
    );
    let out = &out["results"][0];
    assert_eq!(out["updated"], json!([def]));
    assert_eq!(out["skipped"], json!([]));
    assert_eq!(out["kept"], json!([]));
    let after_poses: Vec<_> = doc
        .instance_ids()
        .iter()
        .map(|&i| doc.instance_pose(i).unwrap().to_affine())
        .collect();
    assert_eq!(poses, after_poses, "nothing moves");
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        13,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step"]);

    // One undo entry takes it all back.
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        14,
        "hew.history.undo",
        json!({}),
    );
    assert_eq!(doc.state_hash(), before);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        15,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["library_newer"]);
}

#[test]
fn renaming_an_item_announces_nothing() {
    let mut host = library_with_a_box();
    let (mut conn, mut doc, _) = model_with_two_boxes(&mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        10,
        "hew.library.update_meta",
        json!({ "item": "box-id", "name": "Oak box" }),
    );
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        11,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step"]);
    // And a third insert still reuses the definition.
    let third = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        12,
        "hew.library.insert",
        json!({ "item": "box-id", "at": [9.0, 0.0, 0.0] }),
    );
    assert_eq!(third["results"][0]["definitions_reused"], 1);
}

#[test]
fn an_item_saved_before_prints_is_printed_from_its_file() {
    let mut host = library_with_a_box();
    // Strip the prints, as an item saved by 1.4 has none.
    let path = host.items.keys().next().unwrap().clone();
    let mut item = Document::load(&host.items[&path]).unwrap();
    item.attr_delete(AttrTarget::Document, "hew.library", Some("prints"))
        .unwrap();
    host.items.insert(path, item.save());
    let (mut conn, mut doc, _) = model_with_two_boxes(&mut host);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        10,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["in_step"]);
}

#[test]
fn a_part_changed_here_reads_edited_here_and_update_all_leaves_it() {
    let mut host = library_with_a_box();
    let (mut conn, mut doc, _) = model_with_two_boxes(&mut host);
    let c = doc.component_ids()[0];
    let o = doc.def_members(c).unwrap()[0];
    let top = doc
        .object(o)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.9)
        .map(|(id, _)| id)
        .unwrap();
    doc.apply_def_op(
        c,
        o,
        kernel::KernelOp::PushPull {
            face: top,
            distance: 0.3,
        },
    )
    .unwrap();
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        10,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["edited_here"]);
    let out = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        11,
        "hew.library.update",
        json!({ "all": true }),
    );
    assert_eq!(out["results"][0]["updated"], json!([]));
    edit_the_box_in_the_library(&mut host, 0.5);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        12,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["both_changed"]);
}

#[test]
fn update_refusals_are_typed_and_change_nothing() {
    let mut host = library_with_a_box();
    let (mut conn, mut doc, def) = model_with_two_boxes(&mut host);
    edit_the_box_in_the_library(&mut host, 0.5);

    // Neither or both of definitions/all.
    for (n, params) in [
        (10, json!({})),
        (11, json!({ "all": true, "definitions": [def] })),
    ] {
        let r = call(
            &mut conn,
            &mut doc,
            &mut host,
            n,
            "hew.library.update",
            params,
        );
        assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    }
    // An id that is not a definition.
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        12,
        "hew.library.update",
        json!({ "definitions": ["obj_999999"] }),
    );
    assert_eq!(d["refusal"], "unknown_entity");

    // A locked part, named alone: a typed refusal naming the lock.
    let inst = public(&doc, EntityRef::Instance(doc.instance_ids()[0]));
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        13,
        "hew.entity.lock",
        json!({ "targets": [inst] }),
    );
    let hash_locked = doc.state_hash();
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        14,
        "hew.library.update",
        json!({ "definitions": [def] }),
    );
    assert_eq!(d["refusal"], "locked_node");
    assert!(d["detail"]["node"].is_string(), "{d}");
    assert_eq!(doc.state_hash(), hash_locked);
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        15,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(status["links"][0]["locked"], true);
    // With all: true the locked part is skipped and named.
    let out = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        16,
        "hew.library.update",
        json!({ "all": true }),
    );
    let out = &out["results"][0];
    assert_eq!(out["updated"], json!([]));
    assert_eq!(out["skipped"][0]["definition"], def);
    assert_eq!(out["skipped"][0]["refusal"], "locked_node");
    assert_eq!(doc.state_hash(), hash_locked);

    // A definition with no Library link.
    let root = build_box(&mut conn, &mut doc);
    let (plain, _, _) = doc.make_component(&[NodeId::Object(root)]).unwrap();
    let plain_id = public(&doc, EntityRef::Component(plain));
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        17,
        "hew.library.update",
        json!({ "definitions": [plain_id] }),
    );
    assert_eq!(d["refusal"], "unknown_library_item");

    // The item gone from the Library.
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        18,
        "hew.entity.unlock",
        json!({ "all": true }),
    );
    let path = host.items.keys().next().unwrap().clone();
    host.items.remove(&path);
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        19,
        "hew.library.update",
        json!({ "definitions": [def] }),
    );
    assert_eq!(d["refusal"], "unknown_library_item");
    let status = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        20,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(states(&status), vec!["item_missing"]);
}

#[test]
fn status_and_update_need_a_library_host() {
    let mut conn = new_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.library.status",
        json!({}),
    );
    assert_eq!(d["refusal"], "host_capability_missing");
    let d = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        3,
        "hew.library.update",
        json!({ "all": true }),
    );
    assert_eq!(d["refusal"], "host_capability_missing");
}

mod update_fuzz {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Step {
        Insert(f64),
        EditInLibrary(f64),
        EditHere(f64),
        Lock(usize),
        UnlockAll,
        Update(Vec<usize>),
        UpdateAll,
        Undo,
        Redo,
    }

    fn arb_step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (-5.0f64..5.0).prop_map(Step::Insert),
            (0.05f64..0.6).prop_map(Step::EditInLibrary),
            (0.05f64..0.6).prop_map(Step::EditHere),
            any::<usize>().prop_map(Step::Lock),
            Just(Step::UnlockAll),
            proptest::collection::vec(any::<usize>(), 0..3).prop_map(Step::Update),
            Just(Step::UpdateAll),
            Just(Step::Undo),
            Just(Step::Redo),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Arbitrary status reads and update calls, among inserts, edits on
        /// both sides, locks, undo and redo, never corrupt the document: it
        /// validates after every step (debug builds), reopens, and a refused
        /// update changes nothing.
        #[test]
        fn arbitrary_updates_never_corrupt(steps in proptest::collection::vec(arb_step(), 1..14)) {
            let mut host = library_with_a_box();
            let (mut conn, mut doc, _) = model_with_two_boxes(&mut host);
            for (k, step) in steps.iter().enumerate() {
                let n = 100 + k as i64;
                match step {
                    Step::Insert(x) => {
                        call(&mut conn, &mut doc, &mut host, n, "hew.library.insert", json!({ "item": "box-id", "at": [x, 0.0, 0.0] }));
                    }
                    Step::EditInLibrary(rise) => edit_the_box_in_the_library(&mut host, *rise),
                    Step::EditHere(rise) => {
                        if let Some(&c) = doc.component_ids().first()
                            && let Some(o) = doc.def_members(c).and_then(|m| m.first().copied())
                            && let Some(top) = doc.object(o).and_then(|obj| obj.faces().iter().find(|(_, f)| f.plane.normal().z > 0.9).map(|(id, _)| id))
                        {
                            let _ = doc.apply_def_op(c, o, kernel::KernelOp::PushPull { face: top, distance: *rise });
                        }
                    }
                    Step::Lock(sel) => {
                        let insts = doc.instance_ids();
                        if !insts.is_empty() {
                            let id = public(&doc, EntityRef::Instance(insts[sel % insts.len()]));
                            call(&mut conn, &mut doc, &mut host, n, "hew.entity.lock", json!({ "targets": [id] }));
                        }
                    }
                    Step::UnlockAll => {
                        call(&mut conn, &mut doc, &mut host, n, "hew.entity.unlock", json!({ "all": true }));
                    }
                    Step::Update(sels) => {
                        let defs = doc.component_ids();
                        let ids: Vec<String> = sels
                            .iter()
                            .filter(|_| !defs.is_empty())
                            .map(|s| public(&doc, EntityRef::Component(defs[s % defs.len()])))
                            .collect();
                        let before = doc.state_hash();
                        let r = call(&mut conn, &mut doc, &mut host, n, "hew.library.update", json!({ "definitions": ids }));
                        if r.error.is_some() {
                            prop_assert_eq!(doc.state_hash(), before, "a refused update changed the document");
                        }
                    }
                    Step::UpdateAll => {
                        let r = call(&mut conn, &mut doc, &mut host, n, "hew.library.update", json!({ "all": true }));
                        prop_assert!(r.error.is_none(), "{:?}", r.error);
                    }
                    Step::Undo => {
                        let _ = doc.undo();
                    }
                    Step::Redo => {
                        let _ = doc.redo();
                    }
                }
                let status = call(&mut conn, &mut doc, &mut host, n + 1000, "hew.library.status", json!({}));
                prop_assert!(status.error.is_none(), "{:?}", status.error);
                let back = Document::load(&doc.save());
                prop_assert!(back.is_ok(), "step {k} ({step:?}): the document does not reopen");
            }
        }
    }
}
