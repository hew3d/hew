//! Multi-document addressing (docs/agents/HEW_API.md §4.2): `hew.meta.documents`,
//! `hew.doc.attach` by id / path / label / title, the envelope's per-request
//! `document` member, and the app-only window commands.
//!
//! The desktop shell routes between windows itself and strips these before
//! its per-window dispatchers see them, so what `crates/api` owns — and
//! what these tests pin — is the contract a host with several documents
//! (and the headless host, with exactly one) answers: resolution rules,
//! the typed refusals and their shapes, and the profile split.

use api::{Connection, DispatchOutcome, Grant, Host, NoHost, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// A host with three documents — two of them sharing a title, which is
/// what makes title resolution ambiguous.
struct ManyDocs;

impl Host for ManyDocs {
    fn documents(&self, _doc: &Document) -> Vec<Value> {
        vec![
            json!({"id":"doc_1","title":"Untitled","path":null,"dirty":false,"label":"scratch"}),
            json!({"id":"doc_2","title":"Untitled","path":"/m/a.hew","dirty":true}),
            json!({"id":"doc_3","title":"House","path":"/m/house.hew","dirty":false}),
        ]
    }
}

fn send(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    id: i64,
    method: &str,
    params: Value,
    document: Option<&str>,
) -> Response {
    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
        document: document.map(str::to_string),
    };
    match conn.dispatch(&REGISTRY, doc, host, request) {
        DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => r,
        DispatchOutcome::Dropped => panic!("a request with an id is never dropped"),
        DispatchOutcome::Forward(f) => {
            panic!("no plugin is registered, yet {} forwarded", f.method)
        }
    }
}

fn hello(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) -> Response {
    send(
        conn,
        doc,
        host,
        0,
        "hew.meta.hello",
        json!({"protocol": 1}),
        None,
    )
}

#[test]
fn hello_reports_the_hosts_documents() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    let r = hello(&mut conn, &mut doc, &mut ManyDocs);
    let documents = r.result.expect("hello succeeds")["documents"].clone();
    assert_eq!(documents.as_array().expect("array").len(), 3);
    assert_eq!(documents[2]["title"], "House");
}

#[test]
fn attach_resolves_id_path_label_and_unambiguous_title() {
    for (reference, expect) in [
        ("doc_3", "doc_3"),
        ("/m/a.hew", "doc_2"),
        ("scratch", "doc_1"),
        ("House", "doc_3"),
    ] {
        let mut conn = Connection::new(Grant::app(), "t");
        let mut doc = Document::new();
        hello(&mut conn, &mut doc, &mut ManyDocs);
        let r = send(
            &mut conn,
            &mut doc,
            &mut ManyDocs,
            1,
            "hew.doc.attach",
            json!({ "document": reference }),
            None,
        );
        let result = r
            .result
            .unwrap_or_else(|| panic!("{reference}: {:?}", r.error));
        assert_eq!(result["document"]["id"], expect, "{reference}");
    }
}

#[test]
fn attach_refuses_an_unknown_document_with_the_open_ones_as_detail() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut ManyDocs);
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({ "document": "no-such-document-xyz" }),
        None,
    );
    let err = r.error.expect("refused, not silently accepted");
    assert_eq!(err.code, codes::REFUSED);
    let data = err.data.expect("canonical refusal data");
    assert_eq!(data["refusal"], "unknown_document");
    assert_eq!(data["failed_method"], "hew.doc.attach");
    assert_eq!(
        data["detail"]["documents"].as_array().expect("list").len(),
        3
    );

    // The refused attach left the connection unattached.
    let after = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.query.scene",
        json!({}),
        None,
    );
    assert_eq!(
        after.error.expect("still unattached").code,
        codes::NO_DOCUMENT
    );
}

#[test]
fn an_ambiguous_title_refuses_and_names_the_candidates() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut ManyDocs);
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({ "document": "Untitled" }),
        None,
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "ambiguous_document");
    assert_eq!(data["detail"]["matches"], json!(["doc_1", "doc_2"]));
}

#[test]
fn attach_works_as_the_sole_command_of_a_transaction_too() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut ManyDocs);
    let ok = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.transact",
        json!({"commands":[{"method":"hew.doc.attach","params":{"document":"doc_2"}}]}),
        None,
    );
    let result = ok.result.expect("attached");
    assert_eq!(result["results"][0]["document"]["id"], "doc_2");

    let mut conn = Connection::new(Grant::app(), "t");
    hello(&mut conn, &mut doc, &mut ManyDocs);
    let bad = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.transact",
        json!({"commands":[{"method":"hew.doc.attach","params":{"document":"nope"}}]}),
        None,
    );
    assert_eq!(
        bad.error.expect("refused").data.expect("data")["refusal"],
        "unknown_document"
    );
}

#[test]
fn attach_with_no_document_still_binds_a_single_document_host() {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    assert_eq!(r.result.expect("attached"), json!({}));
}

#[test]
fn attach_rejects_unknown_fields_and_a_non_string_document() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut ManyDocs);
    for params in [json!({"bogus": 1}), json!({"document": 7})] {
        let r = send(
            &mut conn,
            &mut doc,
            &mut ManyDocs,
            1,
            "hew.doc.attach",
            params,
            None,
        );
        assert_eq!(r.error.expect("invalid").code, codes::INVALID_PARAMS);
    }
}

#[test]
fn the_envelope_document_member_addresses_one_request_without_an_attach() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut ManyDocs);
    // Unattached: a bare read answers -32002 …
    let bare = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.query.scene",
        json!({}),
        None,
    );
    assert_eq!(bare.error.expect("unattached").code, codes::NO_DOCUMENT);
    // … a request that names a real document runs …
    let named = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.query.scene",
        json!({}),
        Some("doc_3"),
    );
    assert!(named.error.is_none(), "{:?}", named.error);
    // … one naming a document that is not there is refused typed …
    let wrong = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        3,
        "hew.query.scene",
        json!({}),
        Some("doc_99"),
    );
    assert_eq!(
        wrong.error.expect("refused").data.expect("data")["refusal"],
        "unknown_document"
    );
    // … and none of it attached the connection.
    let still = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        4,
        "hew.query.scene",
        json!({}),
        None,
    );
    assert_eq!(
        still.error.expect("still unattached").code,
        codes::NO_DOCUMENT
    );
}

#[test]
fn the_window_commands_are_app_only_and_refuse_where_there_is_no_window() {
    // Core never sees them.
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    for method in ["hew.doc.close", "hew.doc.focus"] {
        let r = send(&mut conn, &mut doc, &mut NoHost, 1, method, json!({}), None);
        assert_eq!(
            r.error.expect("not granted").code,
            codes::NOT_PERMITTED,
            "{method}"
        );
    }
    // App grants them; a host with no windows refuses typed.
    let mut conn = Connection::new(Grant::app(), "t");
    hello(&mut conn, &mut doc, &mut NoHost);
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    for method in ["hew.doc.close", "hew.doc.focus"] {
        let r = send(&mut conn, &mut doc, &mut NoHost, 2, method, json!({}), None);
        let data = r.error.expect("refused").data.expect("data");
        assert_eq!(data["refusal"], "host_capability_missing", "{method}");
    }
}

#[test]
fn close_validates_its_params() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.close",
        json!({"discard": "yes"}),
        None,
    );
    assert_eq!(r.error.expect("invalid").code, codes::INVALID_PARAMS);
}

#[test]
fn new_and_open_accept_placement_params() {
    let mut conn = Connection::new(Grant::app(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    // NoHost refuses the effect, but the params must parse first: a typo'd
    // `focus` is an invalid-params error, a valid one reaches the host.
    let bad = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.new",
        json!({"focus": "sideways"}),
        None,
    );
    assert_eq!(bad.error.expect("invalid").code, codes::INVALID_PARAMS);
    let good = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.new",
        json!({"focus": "background", "label": "scratch"}),
        None,
    );
    assert_eq!(
        good.error.expect("refused").data.expect("data")["refusal"],
        "host_capability_missing"
    );
}

#[test]
fn save_copy_requires_a_path_and_is_refused_by_a_host_that_cannot() {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let no_path = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.save",
        json!({"copy": true}),
        None,
    );
    assert_eq!(no_path.error.expect("invalid").code, codes::INVALID_PARAMS);
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        3,
        "hew.doc.save",
        json!({"copy": true, "path": "/tmp/x.hew"}),
        None,
    );
    assert_eq!(
        r.error.expect("refused").data.expect("data")["refusal"],
        "host_capability_missing"
    );
}

#[test]
fn import_accepts_client_supplied_bytes_and_rejects_malformed_base64() {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let bad = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.import",
        json!({"path": "a.stl", "bytes_base64": "!!not base64!!"}),
        None,
    );
    assert_eq!(bad.error.expect("invalid").code, codes::INVALID_PARAMS);
    // Well-formed bytes reach the host (NoHost then refuses the effect).
    let ok = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        3,
        "hew.doc.import",
        json!({"path": "a.stl", "bytes_base64": "Zm9v", "images": {"w.png": {"bytes_base64": "Zm9v", "format": "png"}}}),
        None,
    );
    assert_eq!(
        ok.error.expect("refused").data.expect("data")["refusal"],
        "host_capability_missing"
    );
}

#[test]
fn save_return_bytes_serializes_without_writing_binding_or_marking_saved() {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut NoHost);
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.save",
        json!({"return_bytes": true}),
        None,
    );
    let bytes = r
        .result
        .expect("served by the dispatcher itself, on any host")["bytes_base64"]
        .as_str()
        .expect("base64")
        .to_string();
    assert!(!bytes.is_empty());
    // It takes neither a path nor copy.
    for params in [
        json!({"return_bytes": true, "path": "/x.hew"}),
        json!({"return_bytes": true, "copy": true}),
    ] {
        let r = send(
            &mut conn,
            &mut doc,
            &mut NoHost,
            3,
            "hew.doc.save",
            params,
            None,
        );
        assert_eq!(r.error.expect("invalid").code, codes::INVALID_PARAMS);
    }
}

/// A host whose randomness is fixed, so the minted id is predictable.
struct FixedRandom;

impl Host for FixedRandom {
    fn random_16(&self) -> [u8; 16] {
        [0xab; 16]
    }
}

#[test]
fn save_mints_the_document_id_once() {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    hello(&mut conn, &mut doc, &mut FixedRandom);
    send(
        &mut conn,
        &mut doc,
        &mut FixedRandom,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    assert_eq!(doc.document_id(), None);
    // A save the host refuses (this host has no filesystem) keeps no id.
    let refused = send(
        &mut conn,
        &mut doc,
        &mut FixedRandom,
        5,
        "hew.doc.save",
        json!({"path": "/x.hew"}),
        None,
    );
    assert!(refused.error.is_some());
    assert_eq!(doc.document_id(), None);
    // Invalid params mint nothing.
    let bad = send(
        &mut conn,
        &mut doc,
        &mut FixedRandom,
        2,
        "hew.doc.save",
        json!({"return_bytes": true, "copy": true}),
        None,
    );
    assert!(bad.error.is_some());
    assert_eq!(doc.document_id(), None);

    let r = send(
        &mut conn,
        &mut doc,
        &mut FixedRandom,
        3,
        "hew.doc.save",
        json!({"return_bytes": true}),
        None,
    );
    let id = doc.document_id().expect("the save minted an id");
    assert_eq!(id, "abababab-abab-4bab-abab-abababababab");
    let b64 = r.result.expect("saved")["bytes_base64"]
        .as_str()
        .unwrap()
        .to_string();
    let bytes = api::commands::doc::decode_base64(&b64).expect("base64");
    let written = Document::load(&bytes).expect("loads");
    assert_eq!(
        written.document_id(),
        Some(id.clone()),
        "the bytes carry it"
    );

    // A second save keeps the same id.
    send(
        &mut conn,
        &mut doc,
        &mut FixedRandom,
        4,
        "hew.doc.save",
        json!({"return_bytes": true}),
        None,
    );
    assert_eq!(doc.document_id(), Some(id));
}
