//! Conformance coverage for addressable sketch edges (docs/HEW_API.md
//! §5.2's compound `"edg_…"` id, `hew.query.entity`'s edge listing/direct
//! query, `hew.entity.delete`'s edge branch, and the sketch-edge locator
//! §5.3's derived points reuse) — the gap the "Getting started" tutorial
//! surfaced: trimming a sketch after drawing it (draw a rectangle, draw a
//! diagonal into it, erase two edges to leave a wedge) was not
//! expressible at all before this, since sketch edges carried no public
//! id and no locator named them.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};

// --------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello_attach(conn: &mut Connection, doc: &mut Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        doc,
        &mut NoHost,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
    let DispatchOutcome::Reply(r) =
        conn.dispatch(doc, &mut NoHost, req(1, "hew.doc.attach", json!({})))
    else {
        panic!("attach replies")
    };
    assert!(r.error.is_none(), "attach failed: {:?}", r.error);
}

fn call(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    method: &str,
    params: Value,
) -> Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, &mut NoHost, req(id, method, params)) else {
        panic!("{method} replies")
    };
    r
}

/// Dispatches a command expected to succeed. Model-mutating commands
/// auto-wrap as a one-command `hew.doc.transact` (dispatch.rs), so the
/// reply's `result` is `{"results": [<the command's own result>], …}`;
/// read-only commands (`hew.query.*`) are never wrapped. Unwrapping
/// `results[0]` when present, and falling back to the raw result
/// otherwise, handles both uniformly.
fn call_ok(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, id, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    let result = r.result.expect("a successful reply carries a result");
    result
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|r| r.first())
        .cloned()
        .unwrap_or(result)
}

fn call_err(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, id, method, params);
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

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Draws a rectangle on the ground plane, returning its sketch's public
/// id.
fn draw_rect(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    a: [f64; 3],
    b: [f64; 3],
) -> String {
    let result = call_ok(conn, doc, id, "hew.sketch.draw_rect", ground_rect(a, b));
    result["sketch"].as_str().unwrap().to_string()
}

/// The `edges` array of `hew.query.entity`'s answer for a sketch.
fn edges_of(conn: &mut Connection, doc: &mut Document, id: i64, sketch: &str) -> Vec<Value> {
    let entity = call_ok(conn, doc, id, "hew.query.entity", json!({ "id": sketch }));
    entity["edges"].as_array().unwrap().clone()
}

/// The `regions` array of `hew.query.entity`'s answer for a sketch.
fn regions_of(conn: &mut Connection, doc: &mut Document, id: i64, sketch: &str) -> Vec<Value> {
    let entity = call_ok(conn, doc, id, "hew.query.entity", json!({ "id": sketch }));
    entity["regions"].as_array().unwrap().clone()
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn point_matches(v: &Value, p: [f64; 3]) -> bool {
    let arr = v.as_array().expect("point is an array");
    approx(arr[0].as_f64().unwrap(), p[0])
        && approx(arr[1].as_f64().unwrap(), p[1])
        && approx(arr[2].as_f64().unwrap(), p[2])
}

/// An edge entry (from `edges_of`) whose endpoints coincide with `a`/`b`
/// in either order — the test's own stand-in for "click this edge",
/// mirroring how a client would scan a sketch's own listing for the edge
/// it wants.
fn edge_id_at(edges: &[Value], a: [f64; 3], b: [f64; 3]) -> String {
    edges
        .iter()
        .find(|e| {
            (point_matches(&e["from"], a) && point_matches(&e["to"], b))
                || (point_matches(&e["from"], b) && point_matches(&e["to"], a))
        })
        .unwrap_or_else(|| panic!("no edge with endpoints {a:?}/{b:?} in {edges:?}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn assert_one_undo_and_clean_undo(doc: &mut Document, depth_before: usize, bytes_before: &[u8]) {
    assert_eq!(
        doc.undo_depth(),
        depth_before + 1,
        "the delete should add exactly one undo entry"
    );
    doc.undo().expect("undo restores the compound entry");
    assert_eq!(
        doc.save(),
        bytes_before,
        "undo did not restore byte-identical state"
    );
}

// ---------------------------------------------------- hew.query listing/direct

#[test]
fn sketch_query_lists_edges_and_entity_answers_an_edge_id_directly() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);

    let edges = edges_of(&mut conn, &mut doc, 3, &sketch);
    assert_eq!(edges.len(), 4, "a rectangle has four edges");
    for e in &edges {
        assert!(e["id"].as_str().unwrap().starts_with("edg_"));
        assert!(e["curve"].is_null(), "a rectangle's sides are plain lines");
    }

    // Pick the bottom edge (0,0,0)-(2,0,0) and query it directly by id —
    // a client that just listed an edge will reasonably query it right
    // back (docs/HEW_API.md §5.2).
    let bottom_id = edge_id_at(&edges, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    let direct = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.entity",
        json!({ "id": bottom_id }),
    );
    assert_eq!(direct["kind"], "edge");
    assert_eq!(direct["id"], bottom_id);
    assert_eq!(direct["sketch"], sketch);
    assert!(direct["curve"].is_null());
    assert!(
        approx(direct["length"].as_f64().unwrap(), 2.0),
        "the bottom edge is 2m long"
    );
    assert!(
        (point_matches(&direct["from"], [0.0, 0.0, 0.0])
            && point_matches(&direct["to"], [2.0, 0.0, 0.0]))
            || (point_matches(&direct["from"], [2.0, 0.0, 0.0])
                && point_matches(&direct["to"], [0.0, 0.0, 0.0]))
    );

    let refusal = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.query.entity",
        json!({ "id": "edg_deadbeef_1" }),
    );
    assert_eq!(refusal["refusal"], "unknown_entity");
}

// --------------------------------------------------------- hew.query.resolve

#[test]
fn resolve_edge_locator_by_point_and_by_endpoints_matches_the_listed_id() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
    let edges = edges_of(&mut conn, &mut doc, 3, &sketch);
    let expected_id = edge_id_at(&edges, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);

    // By a point ON the edge (its midpoint).
    let by_point = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.resolve",
        json!({ "edge": { "sketch": sketch, "at": [1.0, 0.0, 0.0] } }),
    );
    assert_eq!(by_point["edge"]["kind"], "sketch");
    assert_eq!(by_point["edge"]["id"], expected_id);
    assert_eq!(by_point["edge"]["sketch"], sketch);
    assert!(by_point["edge"]["curve"].is_null());

    // By its two endpoints.
    let by_endpoints = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.resolve",
        json!({
            "edge": { "sketch": sketch, "from": [2.0, 0.0, 0.0], "to": [0.0, 0.0, 0.0] }
        }),
    );
    assert_eq!(by_endpoints["edge"]["id"], expected_id);

    // The bare public id is itself a legal "edge locator" (§5.2).
    let by_id = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.resolve",
        json!({ "edge": expected_id }),
    );
    assert_eq!(by_id["edge"]["id"], expected_id);

    // A solid edge locator still resolves through the same command,
    // tagged "kind": "solid" (unchanged pre-existing behavior).
    let object_id = {
        let profile = ground_rect([10.0, 0.0, 0.0], [11.0, 1.0, 0.0]);
        let r = call(
            &mut conn,
            &mut doc,
            7,
            "hew.doc.transact",
            json!({
                "commands": [
                    { "method": "hew.sketch.draw_rect", "as": "profile", "params": profile },
                    { "method": "hew.solid.extrude", "params": { "region": { "$ref": "profile#/region_id" }, "distance": 1.0 } },
                ]
            }),
        );
        assert!(r.error.is_none(), "transact refused: {:?}", r.error);
        let result = r.result.expect("a successful reply carries a result");
        result["results"][1]["object_id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let solid_edge = call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.query.resolve",
        json!({ "edge": { "object": object_id, "at": [10.5, 0.0, 0.0] } }),
    );
    assert_eq!(solid_edge["edge"]["kind"], "solid");
    assert_eq!(solid_edge["edge"]["object"], object_id);
}

#[test]
fn resolve_edge_locator_refuses_ambiguous_when_the_point_sits_on_a_shared_vertex() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);

    // A point exactly on a shared corner is equidistant (zero) from both
    // incident edges — refused typed, never guessed (docs/HEW_API.md
    // §5.2's ambiguity rule, API_AMBIGUITY_TOL).
    let refusal = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.query.resolve",
        json!({ "edge": { "sketch": sketch, "at": [0.0, 0.0, 0.0] } }),
    );
    assert_eq!(refusal["refusal"], "ambiguous_locator");
}

#[test]
fn resolve_edge_locator_refuses_a_miss_and_bad_params() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);

    let miss = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.query.resolve",
        json!({ "edge": { "sketch": sketch, "at": [10.0, 10.0, 0.0] } }),
    );
    assert_eq!(miss["refusal"], "locator_missed");

    let no_such_edge = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.query.resolve",
        json!({
            "edge": { "sketch": sketch, "from": [0.0, 0.0, 0.0], "to": [5.0, 5.0, 0.0] }
        }),
    );
    assert_eq!(no_such_edge["refusal"], "locator_missed");

    let unknown_sketch = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.query.resolve",
        json!({ "edge": { "sketch": "skt_deadbeef", "at": [0.0, 0.0, 0.0] } }),
    );
    assert_eq!(unknown_sketch["refusal"], "unknown_entity");
}

// ------------------------------------------------------------ hew.entity.delete

#[test]
fn delete_erases_one_sketch_edge_as_one_undo_entry_and_undoes_byte_identically() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
    let edges = edges_of(&mut conn, &mut doc, 3, &sketch);
    let bottom_id = edge_id_at(&edges, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    assert_eq!(
        regions_of(&mut conn, &mut doc, 4, &sketch).len(),
        1,
        "the closed rectangle is one region before any edge is erased"
    );

    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.entity.delete",
        json!({ "id": bottom_id }),
    );

    // Erasing one side of the rectangle opens its region: three edges
    // remain, no closed region.
    let edges_after = edges_of(&mut conn, &mut doc, 6, &sketch);
    assert_eq!(edges_after.len(), 3);
    assert!(
        edges_after.iter().all(|e| e["id"] != bottom_id),
        "the erased edge's id is gone from the listing"
    );
    assert_eq!(
        regions_of(&mut conn, &mut doc, 7, &sketch).len(),
        0,
        "an open rectangle bounds no region"
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn delete_refuses_a_stale_sketch_edge_id_and_touches_nothing() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
    let edges = edges_of(&mut conn, &mut doc, 3, &sketch);
    let bottom_id = edge_id_at(&edges, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);

    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.entity.delete",
        json!({ "id": bottom_id }),
    );

    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    // The id still parses (the sketch is live), but the edge key it
    // names is gone — the kernel's own typed refusal, not a silent
    // no-op and not a repair (DEVELOPMENT.md rule 4).
    let refusal = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.entity.delete",
        json!({ "id": bottom_id }),
    );
    assert_eq!(refusal["refusal"], "unknown_edge");
    assert_eq!(
        doc.undo_depth(),
        depth_before,
        "a refused delete adds no undo entry"
    );
    assert_eq!(
        doc.save(),
        bytes_before,
        "a refused delete leaves the document untouched"
    );

    // A wholly made-up edge id (well-formed, unknown sketch) refuses the
    // same way as any other dangling entity id.
    let unknown = call_err(
        &mut conn,
        &mut doc,
        6,
        "hew.entity.delete",
        json!({ "id": "edg_deadbeef_1" }),
    );
    assert_eq!(unknown["refusal"], "unknown_entity");
}

// --------------------------------------------------------------------- tutorial

/// The "Getting started" tutorial's own case, end to end: draw a
/// rectangle, draw a diagonal into the same sketch (sticky rules split
/// the edge it lands on — the tutorial's "short piece"), then delete the
/// two edges the tutorial calls out, leaving exactly the wedge region.
///
/// Rectangle A(0,0) B(2,0) C(2,1) D(0,1); the diagonal runs from A to a
/// point E(2, 0.5) partway up edge BC, splitting it into BE/EC and
/// carving the rectangle into a lower wedge (A,B,E) and an upper region
/// (A,E,C,D). Deleting the "top" edge (C-D) and the short upper fragment
/// of the split edge (E-C) leaves the wedge as the sketch's only region —
/// exactly what a client that has never queried a sketch edge before now
/// has the vocabulary to do (docs/HEW_API.md §5.2).
#[test]
fn tutorial_trim_leaves_a_single_wedge_region_of_the_expected_area() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let sketch = draw_rect(&mut conn, &mut doc, 2, [0.0, 0.0, 0.0], [2.0, 1.0, 0.0]);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({
            "plane": { "sketch": sketch },
            "points": [[0.0, 0.0, 0.0], [2.0, 0.5, 0.0]],
        }),
    );

    assert_eq!(
        regions_of(&mut conn, &mut doc, 4, &sketch).len(),
        2,
        "the diagonal splits the rectangle into two regions"
    );

    let edges = edges_of(&mut conn, &mut doc, 5, &sketch);
    assert_eq!(
        edges.len(),
        6,
        "4 rectangle sides, one split into two fragments, plus the diagonal"
    );
    let top_id = edge_id_at(&edges, [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]);
    let fragment_id = edge_id_at(&edges, [2.0, 0.5, 0.0], [2.0, 1.0, 0.0]);

    call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.entity.delete",
        json!({ "id": top_id }),
    );
    call_ok(
        &mut conn,
        &mut doc,
        7,
        "hew.entity.delete",
        json!({ "id": fragment_id }),
    );

    let regions = regions_of(&mut conn, &mut doc, 8, &sketch);
    assert_eq!(
        regions.len(),
        1,
        "only the wedge remains once the top and the fragment are erased"
    );
    let area = regions[0]["area"].as_f64().expect("the wedge has an area");
    assert!(
        (area - 0.5).abs() < 1e-9,
        "the wedge A(0,0) B(2,0) E(2,0.5) has area 0.5 m², got {area}"
    );
}
