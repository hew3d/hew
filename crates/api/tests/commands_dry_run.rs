//! Dry runs (docs/agents/HEW_API.md §6.6): `hew.doc.transact`
//! with `dry_run: true` rehearses an envelope on a clone and answers what a
//! commit would, changing nothing — not the history generation, the epoch,
//! the undo stack, the state hash, the change ring, the selection, or the
//! hidden state.

use std::collections::BTreeSet;

use api::{Connection, DispatchOutcome, Grant, NoHost, Permission, Request, RequestId};
use kernel::Document;
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// One request on `conn`: the response as JSON, and whether the
/// dispatcher answered it as a dry run.
fn send_on(
    registry: &api::Registry,
    conn: &mut Connection,
    doc: &mut Document,
    method: &str,
    params: Value,
) -> (Value, bool) {
    let req = Request {
        document: None,
        jsonrpc: "2.0".into(),
        id: Some(RequestId::Number(7)),
        method: method.to_string(),
        params: Some(params),
    };
    match conn.dispatch(registry, doc, &mut NoHost, req) {
        DispatchOutcome::Reply(r) => (serde_json::to_value(r).unwrap(), false),
        DispatchOutcome::DryRun(r) => (serde_json::to_value(r).unwrap(), true),
        other => panic!("{method}: {other:?}"),
    }
}

fn ready(grant: Grant, doc: &mut Document) -> Connection {
    let mut conn = Connection::new(grant, "dry");
    for (m, p) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        let (r, _) = send_on(&REGISTRY, &mut conn, doc, m, p);
        assert!(r.get("error").is_none(), "{m}: {r}");
    }
    conn
}

fn tx(conn: &mut Connection, doc: &mut Document, params: Value) -> (Value, bool) {
    send_on(&REGISTRY, conn, doc, "hew.doc.transact", params)
}

fn rect_and_extrude() -> Vec<Value> {
    vec![
        json!({ "method": "hew.sketch.draw_rect", "as": "p", "params": {
            "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [1.0, 1.0, 0.0] } }),
        json!({ "method": "hew.solid.extrude", "as": "box", "params": {
            "region": { "$ref": "p#/region_id" }, "distance": 1.0 } }),
    ]
}

/// The clocks and contents a dry run must leave alone.
fn fingerprint(doc: &Document) -> (u64, u64, usize, u64, usize, Vec<u8>) {
    (
        doc.history_generation(),
        doc.document_epoch(),
        doc.undo_depth(),
        doc.state_hash(),
        doc.change_ring_len(),
        doc.save(),
    )
}

fn plugin_grant(perms: &[&str]) -> Grant {
    let set: BTreeSet<Permission> = perms
        .iter()
        .map(|p| Permission::parse(p).unwrap())
        .collect();
    Grant::plugin("com.example.p", None, set)
}

#[test]
fn a_dry_run_answers_what_the_commit_would_and_changes_nothing() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let before = fingerprint(&doc);
    let (dry, was_dry) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "label": "Box", "commands": rect_and_extrude() }),
    );
    assert!(was_dry, "answered as a dry run");
    assert!(dry.get("error").is_none(), "{dry}");
    assert_eq!(dry["result"]["dry_run"], true);
    assert_eq!(dry["result"]["label"], "Box");
    assert_eq!(dry["result"]["results"].as_array().unwrap().len(), 2);
    assert_eq!(fingerprint(&doc), before, "the live document is untouched");

    // The same envelope committed answers the same results, ids included.
    let (real, was_dry) = tx(
        &mut conn,
        &mut doc,
        json!({ "label": "Box", "commands": rect_and_extrude() }),
    );
    assert!(!was_dry);
    assert_eq!(real["result"]["results"], dry["result"]["results"]);
    assert!(real["result"].get("dry_run").is_none());
    assert_eq!(doc.undo_depth(), before.2 + 1);
}

#[test]
fn dry_run_with_coalesce_is_invalid_params() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let (r, was_dry) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "coalesce": "k", "commands": rect_and_extrude() }),
    );
    assert!(was_dry);
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

#[test]
fn dry_run_stale_expect_generation_refuses_generation_moved() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "expect_generation": 99, "commands": rect_and_extrude() }),
    );
    assert_eq!(r["error"]["data"]["refusal"], "generation_moved", "{r}");
    assert_eq!(r["error"]["data"]["detail"]["expected"], 99);
}

#[test]
fn dry_run_refuses_host_served_command() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": [
            { "method": "hew.doc.export", "params": { "path": "/tmp/x.stl", "format": "stl" } }
        ] }),
    );
    let data = &r["error"]["data"];
    assert_eq!(r["error"]["code"], -32000, "{r}");
    assert_eq!(data["refusal"], "dry_run_host_command");
    assert_eq!(data["failed_index"], 0);
    assert_eq!(data["failed_method"], "hew.doc.export");
    assert_eq!(data["detail"], json!({ "method": "hew.doc.export" }));
}

#[test]
fn dry_run_refuses_plugin_command() {
    let mut registry = api::Registry::protocol_1();
    let decls: Vec<plugin_manifest::CommandContribution> = serde_json::from_value(json!([
        { "id": "shelf.add", "class": "mutating", "summary": "Adds a shelf.", "label": "Add shelf" }
    ]))
    .unwrap();
    registry.register_plugin("com.example.lib", &decls).unwrap();
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "dry");
    for (m, p) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        send_on(&registry, &mut conn, &mut doc, m, p);
    }
    // Alone (the sole-command form a commit would forward) and beside a
    // Kernel-served command.
    for commands in [
        json!([{ "method": "com.example.lib.shelf.add" }]),
        json!([rect_and_extrude()[0].clone(), { "method": "com.example.lib.shelf.add" }]),
    ] {
        let n = commands.as_array().unwrap().len();
        let (r, was_dry) = send_on(
            &registry,
            &mut conn,
            &mut doc,
            "hew.doc.transact",
            json!({ "dry_run": true, "commands": commands }),
        );
        assert!(was_dry);
        assert_eq!(r["error"]["data"]["refusal"], "dry_run_host_command", "{r}");
        assert_eq!(r["error"]["data"]["failed_index"], n - 1);
    }
    assert_eq!(conn.pending_forward_count(), 0, "nothing was forwarded");
}

#[test]
fn dry_run_refuses_sole_solitary_kernel_command() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": [{ "method": "hew.history.undo" }] }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot be rehearsed")
    );
}

#[test]
fn dry_run_read_after_write_sees_the_rehearsed_geometry() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let mut commands = rect_and_extrude();
    commands.push(
        json!({ "method": "hew.query.mesh", "params": { "id": { "$ref": "box#/object_id" } } }),
    );
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": commands }),
    );
    assert_eq!(r["result"]["results"][2]["triangle_count"], 12, "{r}");
    assert!(doc.live_object_ids().is_empty());
}

#[test]
fn dry_run_needs_only_read_permission() {
    let mut doc = Document::new();
    let mut conn = ready(plugin_grant(&["model.read"]), &mut doc);
    let (dry, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": rect_and_extrude() }),
    );
    assert!(dry.get("error").is_none(), "{dry}");
    let (real, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "commands": rect_and_extrude() }),
    );
    assert_eq!(real["error"]["code"], -32001, "{real}");
}

#[test]
fn dry_run_attr_namespace_still_checked() {
    let mut doc = Document::new();
    let mut conn = ready(plugin_grant(&["model.read", "attributes.write"]), &mut doc);
    let set = |ns: &str| {
        json!({ "dry_run": true, "commands": [
            { "method": "hew.attr.set", "params": { "target": "document", "ns": ns, "key": "k", "value": 1 } }
        ] })
    };
    let before = fingerprint(&doc);
    let (r, _) = tx(&mut conn, &mut doc, set("com.example.other"));
    assert_eq!(
        r["error"]["data"]["refusal"], "attr_namespace_not_granted",
        "{r}"
    );
    let (r, _) = tx(&mut conn, &mut doc, set("com.example.p"));
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(
        fingerprint(&doc),
        before,
        "the rehearsed write did not land"
    );
}

#[test]
fn dry_run_while_a_compound_is_open_rehearses_and_leaves_it_open() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let token = doc.begin_compound(kernel::CompoundMeta {
        label: "Parts".into(),
        origin: kernel::HistoryOrigin::Connection("plugin:com.example.lib".into()),
        coalesce: None,
    });
    let before = fingerprint(&doc);
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": rect_and_extrude() }),
    );
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(fingerprint(&doc), before);
    assert_eq!(doc.compound_depth(), 1, "the compound is still open");
    doc.abort_compound(token).unwrap();
}

#[test]
fn dry_run_refused_command_rolls_back_inside_the_clone() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let mut commands = rect_and_extrude();
    commands[1]["params"]["distance"] = json!(0.0);
    let before = fingerprint(&doc);
    let (dry, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": commands.clone() }),
    );
    assert_eq!(fingerprint(&doc), before);
    let (real, _) = tx(&mut conn, &mut doc, json!({ "commands": commands }));
    assert_eq!(dry["error"]["data"]["failed_index"], 1, "{dry}");
    assert_eq!(dry["error"], real["error"], "the same error, byte for byte");
}

#[test]
fn dry_run_emits_no_events() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let (r, _) = send_on(
        &REGISTRY,
        &mut conn,
        &mut doc,
        "hew.event.subscribe",
        json!({ "events": ["document_changed"] }),
    );
    assert!(r.get("error").is_none(), "{r}");
    let epoch = doc.document_epoch();
    tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": rect_and_extrude() }),
    );
    // Hosts derive `document_changed` from the change ring; it holds
    // nothing newer than before.
    assert_eq!(
        doc.change_records_since_epoch(epoch).map(<[_]>::len),
        Some(0)
    );
    assert_eq!(doc.change_ring_len(), 0);
}

#[test]
fn dry_run_leaves_selection_and_hidden_state() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::app(), &mut doc);
    let (made, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "commands": rect_and_extrude() }),
    );
    let id = made["result"]["results"][1]["object_id"].clone();
    let before = fingerprint(&doc);
    for method in ["hew.view.hide", "hew.selection.set"] {
        let params = json!({ "ids": [id] });
        let (r, _) = tx(
            &mut conn,
            &mut doc,
            json!({ "dry_run": true, "commands": [{ "method": method, "params": params }] }),
        );
        assert_eq!(
            r["error"]["data"]["refusal"], "dry_run_host_command",
            "{method}: {r}"
        );
        assert_eq!(r["error"]["data"]["detail"]["method"], method);
    }
    assert_eq!(fingerprint(&doc), before, "nothing hidden");
}

#[test]
fn a_dry_run_with_a_compute_command_inside_is_gated_like_one() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let gate = conn.compute_gate();
    let held = gate.try_acquire().expect("free");
    let (r, was_dry) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": [
            { "method": "hew.compute.measure", "params": { "solid": { "box": { "min": [0, 0, 0], "max": [1, 1, 1] } } } }
        ] }),
    );
    assert_eq!(r["error"]["data"]["refusal"], "compute_busy", "{r}");
    assert!(
        was_dry,
        "a dry run refused before it ran is still a dry run"
    );
    drop(held);
    let (r, _) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": [
            { "method": "hew.compute.measure", "params": { "solid": { "box": { "min": [0, 0, 0], "max": [1, 1, 1] } } } }
        ] }),
    );
    assert_eq!(r["result"]["results"][0]["volume"], 1.0, "{r}");
}

/// A host with a Move drag armed: the read a dry run must forward.
struct ArmedHost;

impl api::Host for ArmedHost {
    fn gesture_armed(&self) -> bool {
        true
    }
}

#[test]
fn a_dry_run_reads_the_live_host_like_the_commit_does() {
    let mut doc = Document::new();
    let mut conn = ready(Grant::core(), &mut doc);
    let envelope =
        |dry: bool| json!({ "dry_run": dry, "commands": [{ "method": "hew.query.context" }] });
    let mut answer = |dry: bool| {
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(9)),
            method: "hew.doc.transact".into(),
            params: Some(envelope(dry)),
        };
        let out = conn.dispatch(&REGISTRY, &mut doc, &mut ArmedHost, req);
        serde_json::to_value(out.into_response().unwrap()).unwrap()
    };
    let real = answer(false);
    let dry = answer(true);
    assert_eq!(real["result"]["results"][0]["armed"], true, "{real}");
    assert_eq!(
        dry["result"]["results"], real["result"]["results"],
        "a rehearsal reads what the commit reads: {dry}"
    );
}

#[test]
fn an_unattached_dry_run_checks_the_envelope_first_like_the_commit() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "dry");
    let (r, _) = send_on(
        &REGISTRY,
        &mut conn,
        &mut doc,
        "hew.meta.hello",
        json!({ "protocol": 1 }),
    );
    assert!(r.get("error").is_none(), "{r}");
    for extra in [
        json!({ "expect_generation": 999_999 }),
        json!({ "coalesce": "" }),
    ] {
        let mut commit = json!({ "commands": rect_and_extrude() });
        commit
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let mut dry = commit.clone();
        dry["dry_run"] = json!(true);
        let (real, _) = tx(&mut conn, &mut doc, commit);
        let (rehearsed, was_dry) = tx(&mut conn, &mut doc, dry);
        assert!(was_dry);
        assert_ne!(real["error"]["code"], -32002, "{real}");
        assert_eq!(rehearsed["error"], real["error"], "{extra}: {rehearsed}");
    }
    // A sound envelope on an unattached connection is still NO_DOCUMENT.
    let (r, was_dry) = tx(
        &mut conn,
        &mut doc,
        json!({ "dry_run": true, "commands": rect_and_extrude() }),
    );
    assert!(was_dry);
    assert_eq!(r["error"]["code"], -32002, "{r}");
}
