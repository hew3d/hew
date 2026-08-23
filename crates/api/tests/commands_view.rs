//! Conformance coverage for `hew.view.camera`, `hew.view.zoom_extents`,
//! and `hew.view.units` (docs/agents/HEW_API.md §7's units/camera section) —
//! the live-viewport/app-display effects alongside `hew.view.snapshot`
//! (covered by `commands_doc.rs`). All three are `Served::Host`
//! (registry.rs): `crates/api` owns envelope validation, profile
//! enforcement, and result shaping, and delegates the actual effect to
//! `ctx.host`. Mirrors `commands_doc.rs`'s own two-layered bar:
//!
//! - **`NoHost` refusals** — every command answers `host_capability_missing`
//!   in the canonical §4.4 shape against a host implementing nothing.
//! - **A `FakeHost`** recording what it was called with, proving the
//!   params-parsing contract (mutual exclusion, exactly-one-required,
//!   unknown-name/projection/format rejection) independent of any real
//!   viewport or Settings window.
//!
//! All three are `Profile::App`-only (registry.rs's `Profile::grants`:
//! every `hew.view.*` command except `snapshot`) and `CommandClass::Solitary`
//! with `mutates_document = false` — asserted directly here rather than
//! only via the registry's own declarative tests, so a dispatch-level
//! regression (not just a registry typo) would be caught.

use api::{Connection, DispatchOutcome, Host, NoHost, Profile, Refusal, Request, RequestId, codes};
use kernel::Document;
use serde_json::{Value, json};

// ------------------------------------------------------------- test host

/// Records every `hew.view.*` effect call it receives — no real viewport,
/// just enough to prove `commands/view.rs` parses params and delegates
/// correctly.
#[derive(Default)]
struct FakeHost {
    cameras_set: Vec<api::ViewCameraSpec>,
    zoom_extents_calls: u32,
    units_set: Vec<String>,
    /// If set, `set_camera` returns this instead of `Ok(())`.
    set_camera_result: Option<Result<(), Refusal>>,
}

impl Host for FakeHost {
    fn set_camera(&mut self, spec: &api::ViewCameraSpec) -> Result<(), Refusal> {
        self.cameras_set.push(spec.clone());
        self.set_camera_result.take().unwrap_or(Ok(()))
    }

    fn zoom_extents(&mut self) -> Result<(), Refusal> {
        self.zoom_extents_calls += 1;
        Ok(())
    }

    fn set_display_units(&mut self, format: &str) -> Result<(), Refusal> {
        self.units_set.push(format.to_string());
        Ok(())
    }
}

// ----------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        host,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
}

fn hello_attach(conn: &mut Connection, doc: &mut Document, host: &mut dyn Host) {
    hello(conn, doc, host);
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, host, req(1, "hew.doc.attach", json!({})))
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
) -> api::Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, host, req(id, method, params)) else {
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

fn new_app_conn() -> Connection {
    Connection::new(Profile::App, "test")
}

// ------------------------------------------------------------------ registry

#[test]
fn the_three_view_effect_commands_never_mutate_the_document() {
    let reg = api::Registry::protocol_1();
    for name in ["hew.view.camera", "hew.view.zoom_extents", "hew.view.units"] {
        let cmd = reg.get(name).unwrap_or_else(|| panic!("{name} declared"));
        assert!(
            !cmd.mutates_document,
            "{name} must be mutates_document = false: a host effect on the \
             view/app-settings, never a modeled edit"
        );
        assert_eq!(cmd.class, api::CommandClass::Solitary);
        assert_eq!(cmd.served, api::Served::Host);
    }
}

#[test]
fn core_profile_withholds_all_three() {
    let reg = api::Registry::protocol_1();
    for name in ["hew.view.camera", "hew.view.zoom_extents", "hew.view.units"] {
        let cmd = reg.get(name).unwrap();
        assert!(!Profile::Core.grants(cmd), "{name} must be app-only");
        assert!(Profile::App.grants(cmd), "{name} must be app-granted");
    }
}

// ---------------------------------------------------- host_capability_missing

#[test]
fn camera_refuses_host_capability_missing_against_nohost() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.view.camera",
        json!({ "view": "iso" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn zoom_extents_refuses_host_capability_missing_against_nohost() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.view.zoom_extents",
        json!({}),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

#[test]
fn units_refuses_host_capability_missing_against_nohost() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc, &mut NoHost);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut NoHost,
        2,
        "hew.view.units",
        json!({ "format": "cm" }),
    );
    assert_eq!(data["refusal"], "host_capability_missing");
}

// --------------------------------------------------------------- hew.view.camera

#[test]
fn camera_dispatches_an_explicit_camera_to_the_host() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({ "camera": { "eye": [1.0, 2.0, 3.0], "target": [0.0, 0.0, 0.0] } }),
    );
    assert_eq!(result, json!({}));
    assert_eq!(host.cameras_set.len(), 1);
    match &host.cameras_set[0] {
        api::ViewCameraSpec::Explicit(cam) => {
            assert_eq!(cam.eye, [1.0, 2.0, 3.0]);
            assert_eq!(cam.target, [0.0, 0.0, 0.0]);
        }
        api::ViewCameraSpec::Standard(_) => panic!("expected an explicit camera"),
    }
}

#[test]
fn camera_dispatches_a_standard_view_to_the_host() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({ "view": "top" }),
    );
    assert_eq!(host.cameras_set.len(), 1);
    match &host.cameras_set[0] {
        api::ViewCameraSpec::Standard(v) => assert_eq!(*v, api::StandardView::Top),
        api::ViewCameraSpec::Explicit(_) => panic!("expected a standard view"),
    }
}

#[test]
fn camera_rejects_camera_and_view_together() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({
            "view": "iso",
            "camera": { "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0] }
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert!(host.cameras_set.is_empty(), "never reaches the host");
}

#[test]
fn camera_rejects_neither_camera_nor_view() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert!(host.cameras_set.is_empty());
}

#[test]
fn camera_rejects_an_unknown_view_name() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({ "view": "diagonal" }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn camera_rejects_an_unknown_projection() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({
            "camera": { "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "projection": "orthographic" }
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn camera_surfaces_the_hosts_refusal() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost {
        set_camera_result: Some(Err(Refusal::api("nothing_to_view", "test refusal"))),
        ..Default::default()
    };
    hello_attach(&mut conn, &mut doc, &mut host);
    let data = call_err(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.camera",
        json!({ "view": "iso" }),
    );
    assert_eq!(data["refusal"], "nothing_to_view");
}

// --------------------------------------------------------- hew.view.zoom_extents

#[test]
fn zoom_extents_takes_no_params_and_calls_the_host() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let result = call_ok(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.zoom_extents",
        json!({}),
    );
    assert_eq!(result, json!({}));
    assert_eq!(host.zoom_extents_calls, 1);
}

#[test]
fn zoom_extents_rejects_unknown_params() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.zoom_extents",
        json!({ "unexpected": true }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert_eq!(host.zoom_extents_calls, 0);
}

// -------------------------------------------------------------- hew.view.units

#[test]
fn units_dispatches_every_valid_format_to_the_host() {
    for format in ["m", "cm", "mm", "arch", "frac_in", "dec_in"] {
        let mut conn = new_app_conn();
        let mut doc = Document::new();
        let mut host = FakeHost::default();
        hello_attach(&mut conn, &mut doc, &mut host);
        let result = call_ok(
            &mut conn,
            &mut doc,
            &mut host,
            2,
            "hew.view.units",
            json!({ "format": format }),
        );
        assert_eq!(result, json!({}));
        assert_eq!(host.units_set, vec![format.to_string()]);
    }
}

#[test]
fn units_rejects_an_unknown_format() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.units",
        json!({ "format": "ft" }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert!(host.units_set.is_empty());
}

#[test]
fn units_requires_the_format_field() {
    let mut conn = new_app_conn();
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        2,
        "hew.view.units",
        json!({}),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

// ------------------------------------------------------------------- profile

#[test]
fn core_profile_refuses_all_three_as_not_permitted() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    let mut host = FakeHost::default();
    hello_attach(&mut conn, &mut doc, &mut host);
    for (method, params) in [
        ("hew.view.camera", json!({ "view": "iso" })),
        ("hew.view.zoom_extents", json!({})),
        ("hew.view.units", json!({ "format": "cm" })),
    ] {
        let r = call(&mut conn, &mut doc, &mut host, 3, method, params);
        let err = r
            .error
            .unwrap_or_else(|| panic!("{method} should refuse on core"));
        assert_eq!(err.code, codes::NOT_PERMITTED, "{method} on core");
    }
    assert!(
        host.cameras_set.is_empty() && host.zoom_extents_calls == 0 && host.units_set.is_empty(),
        "profile gate blocks these before the host is ever reached"
    );
}
