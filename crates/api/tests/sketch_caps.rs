//! Bounds on sketch commands whose cost runs away with a count. An unbounded
//! `sides` once wedged a live window for good (and hung headless), so the
//! API caps it where the Polygon tool does.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, codes};
use kernel::Document;
use serde_json::json;

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn call(method: &str, params: serde_json::Value) -> api::Response {
    let mut conn = Connection::new(Grant::core(), "t");
    let mut doc = Document::new();
    let mut host = NoHost;
    for (id, m, p) in [
        (0, "hew.meta.hello", json!({"protocol": 1})),
        (1, "hew.doc.attach", json!({})),
        (2, method, params),
    ] {
        let r = Request {
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: m.to_string(),
            params: Some(p),
            document: None,
        };
        let DispatchOutcome::Reply(resp) = conn.dispatch(&REGISTRY, &mut doc, &mut host, r) else {
            panic!("answered");
        };
        if id == 2 {
            return resp;
        }
    }
    unreachable!()
}

fn polygon(sides: u64) -> serde_json::Value {
    json!({"plane": {"ground": true}, "center": [0, 0, 0], "radius": 1.0, "sides": sides})
}

#[test]
fn a_polygon_over_the_cap_refuses_typed_and_instantly() {
    let r = call("hew.sketch.draw_polygon", polygon(100_000));
    let err = r.error.expect("refused");
    assert_eq!(err.code, codes::REFUSED);
    let data = err.data.expect("data");
    assert_eq!(data["refusal"], "sides_above_cap");
    assert_eq!(data["detail"]["max_sides"], 120);
}

#[test]
fn a_polygon_at_the_cap_is_accepted_and_below_three_is_invalid() {
    assert!(
        call("hew.sketch.draw_polygon", polygon(120))
            .error
            .is_none()
    );
    let r = call("hew.sketch.draw_polygon", polygon(2));
    assert_eq!(r.error.expect("invalid").code, codes::INVALID_PARAMS);
}
