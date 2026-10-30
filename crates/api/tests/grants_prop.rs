//! Grant properties (docs/agents/HEW_API.md §10, PLUGINS.md invariants 3
//! and 9), plus the enforcement paths the golden transcript does not
//! reach: transact-wrapped `attach`, the sole-solitary transact path,
//! `hew.meta.documents`, and the capabilities shape.

use std::collections::BTreeSet;

use api::{
    Connection, DispatchOutcome, Grant, Host, NoHost, Permission, Registry, Request, RequestId,
    Response, within_ceiling,
};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn holdable() -> Vec<Permission> {
    vec![
        Permission::ModelRead,
        Permission::ModelWrite,
        Permission::AttributesWrite,
        Permission::AttributesWriteNs("com.other.app".into()),
        Permission::View,
        Permission::DocumentsSave,
        Permission::DocumentsDiscard,
        Permission::DocumentsOther,
        Permission::Files,
        Permission::Clipboard,
        Permission::Library,
        Permission::Network("api.example.com".into()),
        Permission::All,
    ]
}

fn arb_permissions() -> impl Strategy<Value = BTreeSet<Permission>> {
    proptest::sample::subsequence(holdable(), 0..=13).prop_map(|v| v.into_iter().collect())
}

fn send(
    conn: &mut Connection,
    doc: &mut kernel::Document,
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
        DispatchOutcome::Reply(r) => r,
        DispatchOutcome::Dropped => panic!("a request with an id is never dropped"),
        DispatchOutcome::Forward(f) => {
            panic!("no plugin is registered, yet {} forwarded", f.method)
        }
    }
}

fn capabilities_after(grant: &Grant, reads: usize) -> String {
    let mut conn = Connection::new(grant.clone(), "prop");
    let mut doc = kernel::Document::new();
    let host = &mut NoHost;
    assert!(
        send(
            &mut conn,
            &mut doc,
            host,
            0,
            "hew.meta.hello",
            json!({"protocol": 1}),
            None
        )
        .error
        .is_none()
    );
    send(
        &mut conn,
        &mut doc,
        host,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    for i in 0..reads {
        let r = send(
            &mut conn,
            &mut doc,
            host,
            10 + i as i64,
            "hew.query.scene",
            json!({}),
            None,
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }
    let r = send(
        &mut conn,
        &mut doc,
        host,
        99,
        "hew.meta.capabilities",
        json!({}),
        None,
    );
    serde_json::to_string(&r.result.expect("capabilities succeeds")).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// A plugin or connected-app grant never reaches a command outside the
    /// ceiling its permissions derive.
    #[test]
    fn grant_never_exceeds_ceiling(perms in arb_permissions(), app in any::<bool>()) {
        let registry = Registry::protocol_1();
        let ceiling = Grant::derive_ceiling(&perms);
        let grant = if app {
            Grant::connected_app("pair_1", perms)
        } else {
            Grant::plugin("com.example.p", Some("doc_1"), perms)
        };
        prop_assert_eq!(grant.ceiling, ceiling);
        for c in registry.commands() {
            if grant.grants(c) {
                prop_assert!(within_ceiling(ceiling, c), "{} exceeds {:?}", c.name, ceiling);
            }
        }
    }

    /// `hew.meta.capabilities` is a pure function of (registry, grant):
    /// equal grants give byte-identical results, whatever came before.
    #[test]
    fn capabilities_is_pure(perms in arb_permissions()) {
        let mut perms = perms;
        perms.insert(Permission::ModelRead);
        let a = Grant::plugin("com.example.p", Some("doc_1"), perms.clone());
        let b = Grant::plugin("com.example.p", Some("doc_1"), perms);
        let fresh = capabilities_after(&a, 0);
        prop_assert_eq!(&fresh, &capabilities_after(&b, 0));
        prop_assert_eq!(&fresh, &capabilities_after(&a, 5));
    }
}

#[test]
fn first_party_capabilities_carry_no_grant_fields() {
    for grant in [Grant::core(), Grant::app()] {
        let caps: Value = serde_json::from_str(&capabilities_after(&grant, 0)).unwrap();
        assert!(caps.get("grant").is_none());
        assert!(
            caps["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c.get("permission").is_none())
        );
    }
}

#[test]
fn plugin_capabilities_list_only_granted_commands_with_their_permission() {
    let grant = Grant::plugin(
        "com.example.p",
        Some("doc_1"),
        [Permission::ModelRead].into_iter().collect(),
    );
    let caps: Value = serde_json::from_str(&capabilities_after(&grant, 0)).unwrap();
    assert_eq!(caps["grant"], serde_json::to_value(&grant).unwrap());
    let commands = caps["commands"].as_array().unwrap();
    for c in commands {
        let p = c["permission"].as_str().unwrap();
        assert!(p == "model.read" || p == "always", "{c}");
    }
    assert!(commands.iter().any(|c| c["name"] == "hew.query.scene"));
    assert!(!commands.iter().any(|c| c["name"] == "hew.sketch.draw_rect"));
}

/// A host with three documents.
struct ManyDocs;

impl Host for ManyDocs {
    fn documents(&self, _doc: &kernel::Document) -> Vec<Value> {
        vec![
            json!({"id":"doc_1","title":"A","path":"/m/a.hew","dirty":false}),
            json!({"id":"doc_2","title":"B","path":null,"dirty":true}),
            json!({"id":"doc_3","title":"C","path":"/m/c.hew","dirty":false}),
        ]
    }
}

fn plugin_conn(perms: &[Permission]) -> (Connection, kernel::Document) {
    let mut conn = Connection::new(
        Grant::plugin(
            "com.example.p",
            Some("doc_2"),
            perms.iter().cloned().collect(),
        ),
        "prop",
    );
    let mut doc = kernel::Document::new();
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        0,
        "hew.meta.hello",
        json!({"protocol": 1}),
        None,
    );
    assert!(r.error.is_none());
    (conn, doc)
}

#[test]
fn hello_and_documents_show_only_the_bound_document() {
    let (mut conn, mut doc) = plugin_conn(&[Permission::ModelRead]);
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.meta.hello",
        json!({"protocol": 1}),
        None,
    );
    let hello = r.result.unwrap();
    assert_eq!(hello["profile"], "plugin");
    assert_eq!(
        hello["documents"],
        json!([{"id":"doc_2","title":"B","dirty":true}])
    );
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.meta.documents",
        json!({}),
        None,
    );
    assert_eq!(
        r.result.unwrap()["documents"],
        json!([{"id":"doc_2","title":"B","dirty":true}])
    );

    let (mut conn, mut doc) = plugin_conn(&[Permission::ModelRead, Permission::DocumentsOther]);
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.meta.documents",
        json!({}),
        None,
    );
    let docs = r.result.unwrap()["documents"].clone();
    assert_eq!(docs.as_array().unwrap().len(), 3);
    assert!(
        docs.as_array()
            .unwrap()
            .iter()
            .all(|d| d.get("path").is_none())
    );
}

#[test]
fn attach_by_name_is_refused_on_every_path_without_documents_other() {
    let (mut conn, mut doc) = plugin_conn(&[Permission::ModelRead]);
    let plain = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({"document": "doc_2"}),
        None,
    );
    let err = plain.error.expect("refused");
    assert_eq!(err.code, api::codes::REFUSED);
    assert_eq!(err.data.unwrap()["refusal"], "document_not_granted");
    let wrapped = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.doc.transact",
        json!({"commands": [{"method": "hew.doc.attach", "params": {"document": "doc_3"}}]}),
        None,
    );
    assert_eq!(
        wrapped.error.expect("refused").data.unwrap()["refusal"],
        "document_not_granted"
    );
    // With documents.other, attaching by name resolves as for first party.
    let (mut conn, mut doc) = plugin_conn(&[Permission::ModelRead, Permission::DocumentsOther]);
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({"document": "doc_3"}),
        None,
    );
    assert_eq!(r.result.unwrap()["document"]["id"], "doc_3");
}

#[test]
fn path_parameters_are_refused_inside_transactions_too() {
    let (mut conn, mut doc) = plugin_conn(&[
        Permission::ModelRead,
        Permission::ModelWrite,
        Permission::DocumentsSave,
    ]);
    send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    // The sole-solitary path.
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.doc.transact",
        json!({"commands": [{"method": "hew.doc.save", "params": {"path": "/tmp/x.hew"}}]}),
        None,
    );
    let err = r.error.expect("refused");
    assert_eq!(err.code, api::codes::NOT_PERMITTED);
    assert_eq!(
        err.data.unwrap(),
        json!({"refusal": "path_not_granted", "index": 0, "pointer": "/path"})
    );
    // The static pass, at a later index, before anything runs.
    let before = doc.save();
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        3,
        "hew.doc.transact",
        json!({"commands": [
            {"method": "hew.tag.create", "params": {"name": "T"}},
            {"method": "hew.material.create", "params": {"name": "M", "texture": {"path": "/t.png", "world_size": [1.0, 1.0]}}}
        ]}),
        None,
    );
    let err = r.error.expect("refused");
    assert_eq!(err.code, api::codes::NOT_PERMITTED);
    assert_eq!(
        err.data.unwrap(),
        json!({"refusal": "path_not_granted", "index": 1, "pointer": "/texture/path"})
    );
    assert_eq!(doc.save(), before, "nothing ran");
    // First-party connections keep their path parameters.
    let mut first = Connection::new(Grant::app(), "t");
    let mut doc = kernel::Document::new();
    send(
        &mut first,
        &mut doc,
        &mut NoHost,
        0,
        "hew.meta.hello",
        json!({"protocol": 1}),
        None,
    );
    send(
        &mut first,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let r = send(
        &mut first,
        &mut doc,
        &mut NoHost,
        2,
        "hew.doc.export",
        json!({"format": "stl", "path": "/tmp/x.stl"}),
        None,
    );
    assert_ne!(r.error.map(|e| e.code), Some(api::codes::NOT_PERMITTED));
}

#[test]
fn transact_wrapped_attach_passes_the_same_gate_as_plain_attach() {
    // First party: unchanged.
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = kernel::Document::new();
    send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        0,
        "hew.meta.hello",
        json!({"protocol": 1}),
        None,
    );
    let r = send(
        &mut conn,
        &mut doc,
        &mut NoHost,
        1,
        "hew.doc.transact",
        json!({"commands": [{"method": "hew.doc.attach", "params": {}}]}),
        None,
    );
    assert_eq!(
        r.result.unwrap(),
        json!({"results": [{}], "label": "hew.doc.attach"})
    );
}

/// The `document` parameter of the window commands is guarded like the
/// envelope member: without `documents.other`, naming a window is
/// refused on the plain path and inside a transaction, before the host
/// sees it.
#[test]
fn window_command_document_params_need_documents_other() {
    for method in ["hew.doc.close", "hew.doc.revert", "hew.doc.focus"] {
        let (mut conn, mut doc) = plugin_conn(&[
            Permission::ModelRead,
            Permission::DocumentsDiscard,
            Permission::View,
        ]);
        send(
            &mut conn,
            &mut doc,
            &mut ManyDocs,
            1,
            "hew.doc.attach",
            json!({}),
            None,
        );
        let r = send(
            &mut conn,
            &mut doc,
            &mut ManyDocs,
            2,
            method,
            json!({"document": "doc_3"}),
            None,
        );
        let err = r.error.expect("refused");
        assert_eq!(err.code, api::codes::NOT_PERMITTED, "{method}");
        // focus is withheld outright (it needs documents.other itself);
        // close and revert reach the parameter check.
        if method != "hew.doc.focus" {
            assert_eq!(
                err.data.unwrap(),
                json!({"refusal": "document_not_granted", "document": "doc_3"}),
                "{method}"
            );
            let r = send(
                &mut conn,
                &mut doc,
                &mut ManyDocs,
                3,
                "hew.doc.transact",
                json!({"commands": [{"method": method, "params": {"document": "doc_3"}}]}),
                None,
            );
            assert_eq!(
                r.error.expect("refused").data.unwrap(),
                json!({"refusal": "document_not_granted", "index": 0, "document": "doc_3"}),
                "{method} in a transaction"
            );
            // Leaving it out, or null, acts on the connection's own document
            // and gets past the grant checks.
            for params in [json!({}), json!({"document": null})] {
                let r = send(&mut conn, &mut doc, &mut ManyDocs, 4, method, params, None);
                assert_ne!(
                    r.error.map(|e| e.code),
                    Some(api::codes::NOT_PERMITTED),
                    "{method} on its own document"
                );
            }
        }
    }
    // With documents.other the parameter is the host's to resolve.
    let (mut conn, mut doc) =
        plugin_conn(&[Permission::DocumentsDiscard, Permission::DocumentsOther]);
    send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        1,
        "hew.doc.attach",
        json!({}),
        None,
    );
    let r = send(
        &mut conn,
        &mut doc,
        &mut ManyDocs,
        2,
        "hew.doc.close",
        json!({"document": "doc_3"}),
        None,
    );
    assert_ne!(r.error.map(|e| e.code), Some(api::codes::NOT_PERMITTED));
}

/// First-party capabilities do not list the refusal names only a
/// restricted grant can be answered with; a plugin's do.
#[test]
fn restricted_only_refusals_are_hidden_from_first_party_capabilities() {
    let refusals_of = |caps: &Value, name: &str| -> Vec<String> {
        caps["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("{name} listed"))["refusals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect()
    };
    for grant in [Grant::core(), Grant::app()] {
        let caps: Value = serde_json::from_str(&capabilities_after(&grant, 0)).unwrap();
        for name in ["hew.attr.set", "hew.attr.delete", "hew.doc.attach"] {
            let refusals = refusals_of(&caps, name);
            for hidden in api::dispatch::RESTRICTED_ONLY_REFUSALS {
                assert!(!refusals.iter().any(|r| r == hidden), "{name}: {hidden}");
            }
        }
    }
    let plugin = Grant::plugin(
        "com.example.p",
        Some("doc_1"),
        [Permission::ModelRead, Permission::AttributesWrite]
            .into_iter()
            .collect(),
    );
    let caps: Value = serde_json::from_str(&capabilities_after(&plugin, 0)).unwrap();
    assert!(
        refusals_of(&caps, "hew.attr.set")
            .iter()
            .any(|r| r == "attr_namespace_not_granted")
    );
    assert!(
        refusals_of(&caps, "hew.doc.attach")
            .iter()
            .any(|r| r == "document_not_granted")
    );
}
