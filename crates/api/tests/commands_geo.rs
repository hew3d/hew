//! `hew.geo.get` / `hew.geo.set`: the document's geo-reference, one undo
//! step per change, typed refusals, and the round trip through a save.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn call(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, &mut NoHost, req(9, method, params)) else {
        panic!("{method} replies")
    };
    r
}

fn ok(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Value {
    let r = call(conn, doc, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    let result = r.result.expect("a successful reply carries a result");
    match result["results"].as_array() {
        Some(one) if result["label"] == method && one.len() == 1 => one[0].clone(),
        _ => result,
    }
}

fn attach(doc: &mut Document) -> Connection {
    let mut conn = Connection::new(Profile::Core, "test");
    ok(&mut conn, doc, "hew.meta.hello", json!({ "protocol": 1 }));
    ok(&mut conn, doc, "hew.doc.attach", json!({}));
    conn
}

fn record() -> Value {
    json!({
        "name": "NAD83 / Oregon North",
        "easting": 2500000.5,
        "northing": 250000.25,
        "height": 12.75,
        "x_axis": [0.0, 1.0],
        "scale": 0.9999,
        "geodetic_datum": "NAD83",
        "map_unit": "foot",
        "map_unit_metres": 0.3048
    })
}

#[test]
fn a_document_starts_without_a_geo_reference() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    assert_eq!(
        ok(&mut conn, &mut doc, "hew.geo.get", json!({})),
        json!({ "geo": null })
    );
}

#[test]
fn set_then_get_returns_the_record_and_it_is_one_undo_step() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    ok(
        &mut conn,
        &mut doc,
        "hew.geo.set",
        json!({ "geo": record() }),
    );
    let got = ok(&mut conn, &mut doc, "hew.geo.get", json!({}));
    assert_eq!(got["geo"]["name"], "NAD83 / Oregon North");
    assert_eq!(got["geo"]["easting"], 2500000.5);
    assert_eq!(got["geo"]["map_unit_metres"], 0.3048);
    // Text left out comes back left out.
    assert!(got["geo"].get("vertical_datum").is_none());
    // What get returns is what set accepts.
    ok(
        &mut conn,
        &mut doc,
        "hew.geo.set",
        json!({ "geo": got["geo"].clone() }),
    );

    ok(&mut conn, &mut doc, "hew.history.undo", json!({}));
    assert_eq!(doc.geo_reference(), None);
    ok(&mut conn, &mut doc, "hew.history.redo", json!({}));
    assert_eq!(doc.geo_reference().unwrap().easting, 2500000.5);
}

#[test]
fn null_clears_the_record() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    ok(
        &mut conn,
        &mut doc,
        "hew.geo.set",
        json!({ "geo": record() }),
    );
    ok(&mut conn, &mut doc, "hew.geo.set", json!({ "geo": null }));
    assert_eq!(doc.geo_reference(), None);
}

#[test]
fn a_non_positive_unit_factor_is_refused_and_changes_nothing() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    let mut bad = record();
    bad["map_unit_metres"] = json!(0.0);
    let r = call(&mut conn, &mut doc, "hew.geo.set", json!({ "geo": bad }));
    let err = r.error.expect("refused");
    assert_eq!(err.code, codes::REFUSED);
    assert_eq!(err.data.unwrap()["refusal"], "invalid_geo_reference");
    assert_eq!(doc.geo_reference(), None);
}

#[test]
fn unknown_fields_and_a_missing_geo_key_are_bad_params() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    for params in [json!({}), json!({ "geo": { "north": 1.0 } })] {
        let r = call(&mut conn, &mut doc, "hew.geo.set", params);
        let err = r.error.expect("rejected");
        assert_ne!(err.code, codes::REFUSED);
    }
}

#[test]
fn the_record_survives_a_save_and_load() {
    let mut doc = Document::new();
    let mut conn = attach(&mut doc);
    ok(
        &mut conn,
        &mut doc,
        "hew.geo.set",
        json!({ "geo": record() }),
    );
    let loaded = Document::load(&doc.save()).unwrap();
    let mut doc2 = loaded;
    let mut conn2 = attach(&mut doc2);
    let got = ok(&mut conn2, &mut doc2, "hew.geo.get", json!({}));
    assert_eq!(got["geo"]["geodetic_datum"], "NAD83");
}
