//! Conformance coverage for the drafting-engine API (unified drafting §7):
//! face-target `hew.sketch.draw_*` through the draft engine (loose lines,
//! overhanging shapes, the `units` result field), `hew.sketch.units`, the
//! unit verbs `move`/`rotate`/`scale`/`copy`/`delete`, `offset` of a face
//! region, and `hew.solid.extrude` of a plane region by unit key. Every
//! mutating call checks the one-call-one-undo / byte-identical-undo
//! property via `call_ok_one_undo`.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// ----------------------------------------------------------------- fixtures
// (mirrors commands_sketch_solid.rs's fixture set — each integration test
// binary is its own compilation unit, so this is duplicated rather than
// shared; do not let the two drift on behavior, only on which tests they
// hold.)

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
}

fn hello_attach(conn: &mut Connection, doc: &mut Document) {
    hello(conn, doc);
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(1, "hew.doc.attach", json!({})),
    ) else {
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
    let DispatchOutcome::Reply(r) =
        conn.dispatch(&REGISTRY, doc, &mut NoHost, req(id, method, params))
    else {
        panic!("{method} replies")
    };
    r
}

fn call_ok(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, id, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

/// Dispatches a command expected to refuse, returning the canonical §4.4
/// `error.data` payload.
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

/// Calls `method`, asserts it added EXACTLY one undo entry, asserts
/// undoing it restores `doc.save()` to `before`'s bytes byte-for-byte,
/// then redoes so the caller can keep building on the change — the
/// one-envelope-one-undo / byte-identical-undo property every mutating
/// envelope owes (docs/design/api-implementation-conventions.md).
fn call_ok_one_undo(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let before = doc.save();
    let depth_before = doc.undo_depth();
    let result = call_ok(conn, doc, id, method, params);
    assert_eq!(
        doc.undo_depth(),
        depth_before + 1,
        "{method} should add exactly one undo entry"
    );
    doc.undo().expect("undo succeeds");
    assert_eq!(
        doc.save(),
        before,
        "{method}'s undo should restore byte-identical bytes"
    );
    doc.redo().expect("redo succeeds");
    result
}

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Draws a rectangle then extrudes it into a solid, in one transaction.
/// Returns the new object's public id.
fn build_box(
    conn: &mut Connection,
    doc: &mut Document,
    id: i64,
    corner_a: [f64; 3],
    corner_b: [f64; 3],
    distance: f64,
) -> String {
    let result = call_ok_one_undo(
        conn,
        doc,
        id,
        "hew.doc.transact",
        json!({
            "label": "Box",
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect(corner_a, corner_b) },
                { "method": "hew.solid.extrude", "as": "box", "params": {
                    "region": { "$ref": "profile#/region_id" },
                    "distance": distance
                }}
            ]
        }),
    );
    result["results"][1]["object_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A face locator by point, on `object`.
fn face_at(object: &str, at: [f64; 3]) -> Value {
    json!({ "object": object, "at": at })
}

// ------------------------------------------------------------- helpers

/// A fresh attached connection with a 0.2 x 0.2 x 0.2 box on the ground.
fn setup() -> (Connection, Document, String) {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [0.2, 0.2, 0.0],
        0.2,
    );
    (conn, doc, object_id)
}

fn top_plane(object_id: &str) -> Value {
    // A point on the top face clear of anything the tests draw or raise.
    json!({ "face": face_at(object_id, [0.02, 0.02, 0.2]) })
}

fn ground() -> Value {
    json!({ "ground": true })
}

/// The unwrapped result of a standalone call (the dispatcher wraps a lone
/// command's result as `{label, results: [..]}`).
fn first(result: &Value) -> &Value {
    &result["results"][0]
}

fn units(conn: &mut Connection, doc: &mut Document, plane: Value) -> Value {
    // A standalone read-only call answers its result directly.
    call_ok(conn, doc, 80, "hew.sketch.units", json!({ "plane": plane }))
}

fn island_keys(u: &Value) -> Vec<String> {
    u["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["key"].as_str().unwrap().to_string())
        .collect()
}

fn all_edges(u: &Value) -> Vec<Value> {
    u["units"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|i| i["edges"].as_array().unwrap().clone())
        .collect()
}

fn xyz(v: &Value) -> [f64; 3] {
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}

/// The bounding box of every listed edge endpoint.
fn extent(u: &Value) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for e in all_edges(u) {
        for end in ["a", "b"] {
            let p = xyz(&e[end]);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    (lo, hi)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn rect_on(plane: Value, a: [f64; 3], b: [f64; 3]) -> Value {
    json!({ "plane": plane, "corner_a": a, "corner_b": b })
}

fn has_unit(entries: &Value, kind: &str, store: &str) -> bool {
    entries
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["kind"] == kind && u["store"] == store)
}

// ----------------------------------------------------- drawing on a face

#[test]
fn draw_rect_on_a_box_top_is_a_face_and_nothing_is_extrudable() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.05, 0.05, 0.2], [0.15, 0.15, 0.2]),
    );
    let drawn = first(&r);
    assert_eq!(drawn["object_id"].as_str().unwrap(), object_id);
    assert!(has_unit(&drawn["units"], "edge", "topology"));
    assert!(has_unit(&drawn["units"], "region", "topology"));
    assert!(!has_unit(&drawn["units"], "region", "free"));

    let u = units(&mut conn, &mut doc, top_plane(&object_id));
    let regions = u["regions"].as_array().unwrap();
    assert!(regions.len() >= 2, "the rectangle and the face around it");
    assert!(
        regions.iter().all(|r| r["extrudable"] == false),
        "every region on the box top is over the face: {regions:?}"
    );
    assert!(regions.iter().all(|r| r["face"].is_object()));
}

#[test]
fn a_loose_line_on_a_face_lands_in_the_objects_owned_sketch() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({
            "plane": top_plane(&object_id),
            "points": [[0.05, 0.05, 0.2], [0.15, 0.1, 0.2]]
        }),
    );
    let drawn = first(&r);
    assert!(has_unit(&drawn["units"], "edge", "owned"));
    let owned = drawn["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["store"] == "owned")
        .unwrap();
    assert_eq!(owned["object_id"].as_str().unwrap(), object_id);
    assert!(owned["sketch_id"].is_string());
    let u = units(&mut conn, &mut doc, top_plane(&object_id));
    assert_eq!(all_edges(&u)[0]["store"], "owned");
}

#[test]
fn a_rect_overhanging_the_face_edge_splits_into_a_face_and_a_free_region() {
    let (mut conn, mut doc, object_id) = setup();
    // x from 0.1 to 0.3: the part past x = 0.2 hangs over the void.
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.1, 0.05, 0.2], [0.3, 0.15, 0.2]),
    );
    assert!(has_unit(&first(&r)["units"], "region", "topology"));
    assert!(has_unit(&first(&r)["units"], "region", "free"));

    let u = units(&mut conn, &mut doc, top_plane(&object_id));
    let extrudable: Vec<&Value> = u["regions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["extrudable"] == true)
        .collect();
    assert_eq!(extrudable.len(), 1, "one free region: {u}");
    let on_face: Vec<&Value> = u["regions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["face"].is_object() && r["outer"].as_array().unwrap().len() == 4)
        .collect();
    assert!(!on_face.is_empty());
}

/// The two-order story: extrude the free region and push/pull the face
/// region of an overhanging rectangle, in either order; both give two
/// objects.
fn overhang_both_orders(free_first: bool) {
    let (mut conn, mut doc, object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.1, 0.05, 0.2], [0.3, 0.15, 0.2]),
    );
    let key_of = |u: &Value, extrudable: bool| -> String {
        u["regions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["extrudable"] == extrudable)
            // The rectangle's face part: the smaller face-backed region.
            .filter(|r| extrudable || r["outer"].as_array().unwrap().len() == 4)
            .map(|r| r["key"].as_str().unwrap().to_string())
            .next()
            .unwrap()
    };
    assert_eq!(doc.visible_object_ids().len(), 1);

    let extrude = |conn: &mut Connection, doc: &mut Document, id: i64, region: String, d: f64| {
        call_ok_one_undo(
            conn,
            doc,
            id,
            "hew.solid.extrude",
            json!({ "plane": top_plane(&object_id), "region": region, "distance": d }),
        )
    };
    let steps = if free_first {
        [true, false]
    } else {
        [false, true]
    };
    for (i, free) in steps.into_iter().enumerate() {
        let u = units(&mut conn, &mut doc, top_plane(&object_id));
        let region = key_of(&u, free);
        let r = extrude(&mut conn, &mut doc, 10 + i as i64, region, 0.05);
        if free {
            let made = first(&r)["object_id"].as_str().unwrap();
            assert_ne!(made, object_id, "the free region makes a new object");
        } else {
            let grown = first(&r)["object_id"].as_str().unwrap();
            assert_eq!(grown, object_id, "the face region grows the box");
        }
    }
    assert_eq!(
        doc.visible_object_ids().len(),
        2,
        "free_first = {free_first}"
    );
}

#[test]
fn overhang_extrude_free_region_then_push_pull_face_makes_two_objects() {
    overhang_both_orders(true);
}

#[test]
fn overhang_push_pull_face_then_extrude_free_region_makes_two_objects() {
    overhang_both_orders(false);
}

#[test]
fn face_draw_still_mints_face_tokens_from_the_faces_it_produced() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.sketch.draw_circle", "as": "c", "params": {
                "plane": top_plane(&object_id),
                "center": [0.1, 0.1, 0.2], "radius": 0.04
            }},
            { "method": "hew.solid.push_pull", "params": {
                "face": { "$face": "c#face" }, "distance": 0.03
            }}
        ]}),
    );
    assert_eq!(
        r["results"][1]["object_id"].as_str().unwrap(),
        object_id,
        "the circle's face boss reshapes the box in place"
    );
}

#[test]
fn a_loose_draw_mints_no_face_token() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.sketch.draw_line", "as": "l", "params": {
                "plane": top_plane(&object_id),
                "points": [[0.05, 0.05, 0.2], [0.15, 0.1, 0.2]]
            }},
            { "method": "hew.solid.push_pull", "params": {
                "face": { "$face": "l#a" }, "distance": 0.03
            }}
        ]}),
    );
    let data = r
        .error
        .expect("no face was produced, so no token")
        .data
        .unwrap();
    assert_eq!(data["refusal"], "face_token_unknown");
}

#[test]
fn drawing_off_the_faces_plane_still_refuses_point_not_on_face() {
    let (mut conn, mut doc, object_id) = setup();
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": top_plane(&object_id), "points": [[0.05, 0.05, 0.1], [0.15, 0.1, 0.1]] }),
    );
    assert_eq!(data["refusal"], "point_not_on_face");
}

// ------------------------------------------------------- hew.sketch.units

#[test]
fn units_lists_islands_edges_stores_curves_and_regions() {
    let (mut conn, mut doc, object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_circle",
        json!({ "plane": top_plane(&object_id), "center": [0.1, 0.1, 0.2], "radius": 0.04 }),
    );
    let u = units(&mut conn, &mut doc, top_plane(&object_id));
    let islands = u["units"].as_array().unwrap();
    assert_eq!(islands.len(), 1, "one circle, one island");
    assert!(islands[0]["key"].as_str().unwrap().starts_with("isl_"));
    let edges = islands[0]["edges"].as_array().unwrap();
    assert!(edges.len() >= 24);
    for e in edges {
        assert!(e["key"].as_str().unwrap().starts_with("edg_"));
        assert_eq!(e["store"], "topology");
        assert_eq!(e["object_id"].as_str().unwrap(), object_id);
        assert!(e["curve"]["key"].as_str().unwrap().starts_with("crv_"));
        assert!(close(e["curve"]["radius"].as_f64().unwrap(), 0.04));
    }
    assert!(
        u["regions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| { r["key"].as_str().unwrap().starts_with("rgn_") && r["holes"].is_array() })
    );

    // The same plane through a sketch id lists the same units: drawing on
    // the ground, the plane is addressable by its sketch.
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_rect",
        rect_on(ground(), [1.0, 1.0, 0.0], [1.5, 1.5, 0.0]),
    );
    let g = units(&mut conn, &mut doc, ground());
    assert_eq!(island_keys(&g).len(), 1);
    assert!(all_edges(&g).iter().all(|e| e["store"] == "free"));
    assert!(all_edges(&g)[0]["sketch_id"].is_string());
}

// ------------------------------------------------- unit verbs on the ground

fn ground_rect_island(conn: &mut Connection, doc: &mut Document) -> String {
    call_ok(
        conn,
        doc,
        5,
        "hew.sketch.draw_rect",
        rect_on(ground(), [1.0, 1.0, 0.0], [2.0, 2.0, 0.0]),
    );
    island_keys(&units(conn, doc, ground()))[0].clone()
}

#[test]
fn ground_unit_move_rotate_scale_copy_delete_are_one_undo_step_each() {
    let (mut conn, mut doc, _object_id) = setup();
    let isl = ground_rect_island(&mut conn, &mut doc);

    // move
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.move",
        json!({ "plane": ground(), "units": [isl], "offset": [0.5, 0.0, 0.0] }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let (lo, hi) = extent(&u);
    assert!(close(lo[0], 1.5) && close(hi[0], 2.5), "{lo:?} {hi:?}");
    let isl = island_keys(&u)[0].clone();

    // rotate a quarter turn about (1.5, 1, 0): x extent becomes [0.5, 1.5].
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        7,
        "hew.sketch.rotate",
        json!({
            "plane": ground(), "units": [isl],
            "axis_point": [1.5, 1.0, 0.0], "angle": std::f64::consts::FRAC_PI_2
        }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let (lo, hi) = extent(&u);
    assert!(
        (lo[0] - 0.5).abs() < 1e-6 && (hi[0] - 1.5).abs() < 1e-6,
        "{lo:?} {hi:?}"
    );
    let isl = island_keys(&u)[0].clone();

    // scale by 2 about the lower corner of the extent
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        8,
        "hew.sketch.scale",
        json!({ "plane": ground(), "units": [isl], "center": [lo[0], lo[1], 0.0], "factor": 2.0 }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let (lo2, hi2) = extent(&u);
    assert!((hi2[0] - lo2[0] - 2.0).abs() < 1e-6, "{lo2:?} {hi2:?}");
    let isl = island_keys(&u)[0].clone();

    // copy
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        9,
        "hew.sketch.copy",
        json!({ "plane": ground(), "units": [isl.clone()], "offset": [5.0, 0.0, 0.0] }),
    );
    let u = units(&mut conn, &mut doc, ground());
    assert_eq!(island_keys(&u).len(), 2);

    // delete (both islands)
    let keys = island_keys(&u);
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        10,
        "hew.sketch.delete",
        json!({ "plane": ground(), "units": keys }),
    );
    assert!(island_keys(&units(&mut conn, &mut doc, ground())).is_empty());
}

#[test]
fn ground_unit_verbs_take_edge_curve_and_region_keys() {
    let (mut conn, mut doc, _object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_circle",
        json!({ "plane": ground(), "center": [1.0, 1.0, 0.0], "radius": 0.3 }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let crv = all_edges(&u)[0]["curve"]["key"]
        .as_str()
        .unwrap()
        .to_string();
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.move",
        json!({ "plane": ground(), "units": [crv], "offset": [1.0, 0.0, 0.0] }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let (lo, hi) = extent(&u);
    assert!(close((lo[0] + hi[0]) / 2.0, 2.0), "{lo:?} {hi:?}");

    let rgn = u["regions"].as_array().unwrap()[0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        7,
        "hew.sketch.delete",
        json!({ "plane": ground(), "units": [rgn] }),
    );
    assert!(island_keys(&units(&mut conn, &mut doc, ground())).is_empty());

    // a single edge
    call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.sketch.draw_line",
        json!({ "plane": ground(), "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] }),
    );
    let edg = all_edges(&units(&mut conn, &mut doc, ground()))[0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        9,
        "hew.sketch.delete",
        json!({ "plane": ground(), "units": [edg] }),
    );
}

// --------------------------------------------------- unit verbs on a face

fn rect_island_on_top(conn: &mut Connection, doc: &mut Document, object_id: &str) -> String {
    call_ok(
        conn,
        doc,
        5,
        "hew.sketch.draw_rect",
        rect_on(top_plane(object_id), [0.05, 0.05, 0.2], [0.1, 0.1, 0.2]),
    );
    island_keys(&units(conn, doc, top_plane(object_id)))[0].clone()
}

#[test]
fn face_unit_move_rotate_scale_copy_delete_are_one_undo_step_each() {
    let (mut conn, mut doc, object_id) = setup();
    let plane = top_plane(&object_id);
    let isl = rect_island_on_top(&mut conn, &mut doc, &object_id);

    call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.move",
        json!({ "plane": plane.clone(), "units": [isl], "offset": [0.05, 0.0, 0.0] }),
    );
    let u = units(&mut conn, &mut doc, plane.clone());
    let (lo, hi) = extent(&u);
    assert!(close(lo[0], 0.1) && close(hi[0], 0.15), "{lo:?} {hi:?}");
    assert!(all_edges(&u).iter().all(|e| e["store"] == "topology"));
    let isl = island_keys(&u)[0].clone();

    call_ok_one_undo(
        &mut conn,
        &mut doc,
        7,
        "hew.sketch.rotate",
        json!({
            "plane": plane.clone(), "units": [isl],
            "axis_point": [0.1, 0.05, 0.2], "angle": std::f64::consts::FRAC_PI_2
        }),
    );
    let u = units(&mut conn, &mut doc, plane.clone());
    let isl = island_keys(&u)[0].clone();

    call_ok_one_undo(
        &mut conn,
        &mut doc,
        8,
        "hew.sketch.scale",
        json!({ "plane": plane.clone(), "units": [isl], "center": [0.125, 0.075, 0.2], "factor": 1.5 }),
    );
    let u = units(&mut conn, &mut doc, plane.clone());
    let isl = island_keys(&u)[0].clone();

    call_ok_one_undo(
        &mut conn,
        &mut doc,
        9,
        "hew.sketch.copy",
        json!({ "plane": plane.clone(), "units": [isl], "offset": [0.0, 0.08, 0.0] }),
    );
    let u = units(&mut conn, &mut doc, plane.clone());
    assert_eq!(island_keys(&u).len(), 2, "{u}");

    call_ok_one_undo(
        &mut conn,
        &mut doc,
        10,
        "hew.sketch.delete",
        json!({ "plane": plane.clone(), "units": island_keys(&u) }),
    );
    let u = units(&mut conn, &mut doc, plane);
    assert!(island_keys(&u).is_empty(), "{u}");
    assert_eq!(u["regions"].as_array().unwrap().len(), 1, "the bare face");
}

#[test]
fn offset_of_a_face_region_by_unit_key_is_one_undo_step() {
    let (mut conn, mut doc, object_id) = setup();
    let plane = top_plane(&object_id);
    call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_rect",
        rect_on(plane.clone(), [0.05, 0.05, 0.2], [0.15, 0.15, 0.2]),
    );
    let u = units(&mut conn, &mut doc, plane.clone());
    // The rectangle's own region: the face-backed one with four corners
    // and no holes.
    let rgn = u["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["outer"].as_array().unwrap().len() == 4 && r["holes"].as_array().unwrap().is_empty()
        })
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.offset",
        json!({ "plane": plane.clone(), "region": rgn, "distance": -0.02 }),
    );
    assert!(!first(&r)["units"].as_array().unwrap().is_empty());
    assert_eq!(island_keys(&units(&mut conn, &mut doc, plane)).len(), 2);
}

// ------------------------------------------------------------- refusals

#[test]
fn deleting_a_solids_own_edge_refuses_draft_boundary_edge_touched() {
    let (mut conn, mut doc, object_id) = setup();
    let plane = top_plane(&object_id);
    let before = doc.save();
    let p = kernel::Plane::from_point_normal(
        kernel::Point3::new(0.0, 0.0, 0.2),
        kernel::Vec3::new(0.0, 0.0, 1.0),
    )
    .unwrap();
    let key = kernel::edge_key(
        &p,
        kernel::Point3::new(0.0, 0.0, 0.2),
        kernel::Point3::new(0.2, 0.0, 0.2),
    );
    let data = call_err(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.delete",
        json!({ "plane": plane.clone(), "units": [format!("edg_{key:016x}")] }),
    );
    assert_eq!(data["refusal"], "draft_boundary_edge_touched");
    assert!(data["explanation"].as_str().unwrap().contains("solid"));
    assert_eq!(doc.save(), before, "a refusal changes nothing");

    // Moving the bare top face's region (every ring edge is the solid's
    // own) refuses the same way.
    let u = units(&mut conn, &mut doc, plane.clone());
    let rgn = u["regions"].as_array().unwrap()[0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    let data = call_err(
        &mut conn,
        &mut doc,
        7,
        "hew.sketch.move",
        json!({ "plane": plane, "units": [rgn], "offset": [0.01, 0.0, 0.0] }),
    );
    assert_eq!(data["refusal"], "draft_boundary_edge_touched");
    assert_eq!(doc.save(), before);
}

#[test]
fn unknown_and_malformed_unit_keys_refuse_unknown_unit() {
    let (mut conn, mut doc, object_id) = setup();
    for key in [
        "edg_0000000000000001",
        "isl_ffffffffffffffff",
        "crv_0000000000000002",
        "rgn_0000000000000003",
        "edg_xyz",
        "nonsense",
    ] {
        let before = doc.save();
        let data = call_err(
            &mut conn,
            &mut doc,
            6,
            "hew.sketch.delete",
            json!({ "plane": top_plane(&object_id), "units": [key] }),
        );
        assert_eq!(data["refusal"], "unknown_unit", "{key}");
        assert_eq!(doc.save(), before);
    }
}

#[test]
fn extruding_an_unknown_plane_region_refuses_unknown_unit() {
    // `region_not_pushable` (a sliver kept as drawing) needs a
    // shape the kernel keeps as owned drawing; the engine's own specs pin
    // that case. Here: a well-formed key naming no region refuses typed.
    let (mut conn, mut doc, object_id) = setup();
    let data = call_err(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.extrude",
        json!({ "plane": top_plane(&object_id), "region": "rgn_0000000000000009", "distance": 0.1 }),
    );
    assert_eq!(data["refusal"], "unknown_unit");
}

// ---------------------------------------------------- review regressions

/// Drawing on a grouped solid's face (addressed by the caller) imprints
/// that face, as it did before the drafting engine: the named object takes
/// part even though the UI only draws into a group once it is entered.
#[test]
fn drawing_on_a_grouped_solids_face_imprints_it() {
    let (mut conn, mut doc, object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.group.create",
        json!({ "members": [object_id] }),
    );
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.05, 0.05, 0.2], [0.1, 0.1, 0.2]),
    );
    assert!(
        has_unit(&first(&r)["units"], "region", "topology"),
        "the face was imprinted: {:?}",
        first(&r)["units"]
    );
    assert!(!has_unit(&first(&r)["units"], "edge", "free"));
}

/// A shape that touches a drawn shape at one point is a face (the
/// rectangle holds it as a hole sharing that point); extruding its region
/// by key pulls it, one undo step.
#[test]
fn extruding_a_region_touching_a_shape_at_one_point_pushes_it() {
    let (mut conn, mut doc, object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.04, 0.04, 0.2], [0.16, 0.16, 0.2]),
    );
    // A diamond touching the rectangle's left side at (0.04, 0.10).
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_line",
        json!({
            "plane": top_plane(&object_id),
            "points": [
                [0.04, 0.10, 0.2], [0.07, 0.07, 0.2], [0.10, 0.10, 0.2],
                [0.07, 0.13, 0.2], [0.04, 0.10, 0.2]
            ]
        }),
    );
    let u = units(&mut conn, &mut doc, top_plane(&object_id));
    let regions = u["regions"].as_array().unwrap();
    assert!(
        regions
            .iter()
            .all(|r| !(r["object_id"].is_string() && r["face"].is_null())),
        "every region over the solid is a face: {u}"
    );
    // The diamond: the region whose outer ring holds its far corner.
    let diamond = regions
        .iter()
        .find(|r| {
            r["outer"].as_array().is_some_and(|o| {
                o.len() == 4
                    && o.iter().any(|p| {
                        let p: Vec<f64> = p
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|c| c.as_f64().unwrap())
                            .collect();
                        (p[0] - 0.10).abs() < 1e-9 && (p[1] - 0.10).abs() < 1e-9
                    })
            })
        })
        .unwrap_or_else(|| panic!("the diamond's region: {u}"))
        .clone();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.extrude",
        json!({
            "plane": top_plane(&object_id),
            "region": diamond["key"],
            "distance": 0.05
        }),
    );
    // The diamond shares its touch point with the face around the
    // rectangle, which the pull leaves: the push/pull grows the solid as a
    // union (a pinned ring's route), one solid either way.
    let grown = &first(&r);
    let ids: Vec<&str> = match grown["object_id"].as_str() {
        Some(id) => vec![id],
        None => grown["object_ids"]
            .as_array()
            .unwrap_or_else(|| panic!("a push/pull result: {r}"))
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect(),
    };
    assert_eq!(ids.len(), 1, "one solid: {r}");
    assert_eq!(doc.visible_object_ids().len(), 1);
    for id in doc.visible_object_ids() {
        doc.object(id).unwrap().validate().expect("valid result");
    }
}

/// The live queries read an object whose face holds a hole touching its
/// outer loop (a diamond drawn touching a rectangle at one point): faces
/// listing, entity details and the scene all answer, no refusal.
#[test]
fn queries_answer_on_a_face_with_a_touching_hole() {
    let (mut conn, mut doc, object_id) = setup();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(top_plane(&object_id), [0.04, 0.04, 0.2], [0.16, 0.16, 0.2]),
    );
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_line",
        json!({
            "plane": top_plane(&object_id),
            "points": [
                [0.04, 0.10, 0.2], [0.07, 0.07, 0.2], [0.10, 0.10, 0.2],
                [0.07, 0.13, 0.2], [0.04, 0.10, 0.2]
            ]
        }),
    );
    let faces = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.faces",
        json!({ "object": object_id }),
    );
    let list = faces["faces"].as_array().expect("faces");
    assert_eq!(
        list.len(),
        8,
        "the box's six, the diamond, and the rectangle around it"
    );
    call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.entity",
        json!({ "id": object_id }),
    );
    call_ok(&mut conn, &mut doc, 7, "hew.query.scene", json!({}));
}

/// The ground's region around a circle drawn inside a rectangle with its
/// rim touching one side: `hew.sketch.units` lists it with the circle as a
/// hole sharing that point, and extruding it refuses `region_touches_hole`
/// with the UI's copy (never the tracing-failure refusal), the document
/// untouched. The disc extrudes.
#[test]
fn extruding_a_ground_region_with_a_touching_hole_refuses_region_touches_hole() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        rect_on(ground(), [0.0, 0.0, 0.0], [0.2, 0.2, 0.0]),
    );
    // A 24-gon about (0.1, 0.05), radius 0.05, its vertex 0 on the bottom
    // side at (0.1, 0).
    let mut pts: Vec<Value> = (0..24)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * f64::from(i) / 24.0;
            json!([0.1 + 0.05 * a.cos(), 0.05 + 0.05 * a.sin(), 0.0])
        })
        .collect();
    pts[0] = json!([0.1, 0.0, 0.0]);
    pts.push(json!([0.1, 0.0, 0.0]));
    // Into the rectangle's own sketch, so the two shapes are one drawing.
    let g = units(&mut conn, &mut doc, ground());
    let sketch = all_edges(&g)[0]["sketch_id"].clone();
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": { "sketch": sketch }, "points": pts }),
    );
    let u = units(&mut conn, &mut doc, ground());
    let regions = u["regions"].as_array().unwrap();
    assert_eq!(
        regions.len(),
        2,
        "the disc and the rectangle around it: {u}"
    );
    let around = regions
        .iter()
        .find(|r| r["holes"].as_array().is_some_and(|h| !h.is_empty()))
        .unwrap_or_else(|| panic!("the region holding the circle: {u}"))
        .clone();
    let disc = regions
        .iter()
        .find(|r| r["holes"].as_array().is_none_or(|h| h.is_empty()))
        .unwrap()
        .clone();
    let before = doc.save();
    let data = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.extrude",
        json!({ "plane": ground(), "region": around["key"], "distance": 0.05 }),
    );
    assert_eq!(data["refusal"], "region_touches_hole", "{data}");
    assert!(
        data["explanation"]
            .as_str()
            .unwrap()
            .contains("touches another shape at a point"),
        "{data}"
    );
    assert_eq!(
        doc.save(),
        before,
        "a refusal leaves the document untouched"
    );
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.extrude",
        json!({ "plane": ground(), "region": disc["key"], "distance": 0.05 }),
    );
}

// ------------------------------------------------------- detached shapes

/// The sketch entries of `hew.query.scene` that are marked detached.
fn detached_sketches(conn: &mut Connection, doc: &mut Document) -> Vec<Value> {
    let scene = call_ok(conn, doc, 90, "hew.query.scene", json!({}));
    scene["sketches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["detached"] == true)
        .cloned()
        .collect()
}

/// Detached shapes §5: a 2 cm circle drawn with `detached: true` centered on
/// the box top's corner leaves the face whole (nothing drawn on the top's
/// plane, the face its only region), answers the sketch result shape, and
/// is one detached sketch whose regions extrude into a new Object.
#[test]
fn a_detached_circle_on_a_face_leaves_the_face_whole() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_circle",
        json!({
            "plane": top_plane(&object_id),
            "center": [0.2, 0.2, 0.2], "radius": 0.02, "detached": true
        }),
    );
    let drawn = first(&r);
    assert!(drawn.get("object_id").is_none(), "no face changed: {drawn}");
    assert!(drawn["sketch"].is_string() && drawn["curve_id"].is_string());
    assert_eq!(drawn["region_ids"].as_array().unwrap().len(), 1, "{drawn}");

    let top = units(&mut conn, &mut doc, top_plane(&object_id));
    assert!(
        top["units"].as_array().unwrap().is_empty(),
        "the face's plane has no drawing: {top}"
    );
    assert_eq!(
        top["regions"].as_array().unwrap().len(),
        1,
        "the face alone"
    );

    let seps = detached_sketches(&mut conn, &mut doc);
    assert_eq!(seps.len(), 1);
    assert_eq!(seps[0]["id"], drawn["sketch"]);
    assert_eq!(seps[0]["edges"].as_array().unwrap().len(), 48, "whole");

    // The same draw without `detached` splits at the face's edge.
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_circle",
        json!({
            "plane": top_plane(&object_id),
            "center": [0.0, 0.0, 0.2], "radius": 0.02
        }),
    );
    assert!(has_unit(&first(&r)["units"], "region", "topology"));
    assert!(has_unit(&first(&r)["units"], "region", "free"));
    assert_eq!(detached_sketches(&mut conn, &mut doc).len(), 1);
}

/// With a `{sketch}` plane spec the sketch's own flag decides: `detached`
/// is refused as invalid params, the document untouched; `false` is
/// accepted as the default it is.
#[test]
fn detached_with_a_sketch_target_is_refused_as_invalid_params() {
    let (mut conn, mut doc, _) = setup();
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        rect_on(ground(), [1.0, 1.0, 0.0], [1.1, 1.1, 0.0]),
    );
    let sketch = first(&r)["sketch"].as_str().unwrap().to_string();
    let before = doc.save();
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_circle",
        json!({
            "plane": { "sketch": sketch },
            "center": [1.05, 1.05, 0.0], "radius": 0.02, "detached": true
        }),
    );
    let err = r.error.expect("a sketch target refuses detached");
    assert_eq!(err.code, codes::INVALID_PARAMS, "{err:?}");
    assert!(
        err.message.contains("detached is decided by the sketch"),
        "{}",
        err.message
    );
    assert_eq!(doc.save(), before, "nothing changed");
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_circle",
        json!({
            "plane": { "sketch": sketch },
            "center": [1.05, 1.05, 0.0], "radius": 0.02, "detached": false
        }),
    );
}

/// `hew.sketch.units` never lists a detached sketch on its plane; a
/// `{sketch}` spec on one lists exactly it, every free store marked
/// `detached`, and the unit verbs with that spec edit it alone, leaving
/// the plain drawing it overlaps untouched.
#[test]
fn units_of_a_detached_sketch_carry_the_flag_and_edit_it_alone() {
    let (mut conn, mut doc, _) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": ground(), "corner_a": [1.0, 1.0, 0.0], "corner_b": [1.2, 1.2, 0.0],
            "detached": true
        }),
    );
    let sep = first(&r)["sketch"].as_str().unwrap().to_string();
    // A plain rectangle through it.
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_rect",
        rect_on(ground(), [1.1, 1.1, 0.0], [1.3, 1.3, 0.0]),
    );
    let plain_before = units(&mut conn, &mut doc, ground());
    assert_eq!(all_edges(&plain_before).len(), 4, "the plain one alone");
    assert!(
        all_edges(&plain_before)
            .iter()
            .all(|e| e.get("detached").is_none())
    );

    let spec = json!({ "sketch": sep });
    let own = units(&mut conn, &mut doc, spec.clone());
    let edges = all_edges(&own);
    assert_eq!(edges.len(), 4, "its own rectangle, unsplit: {own}");
    assert!(
        edges.iter().all(|e| e["store"] == "free"
            && e["detached"] == true
            && e["sketch_id"] == sep.as_str())
    );
    assert_eq!(own["regions"].as_array().unwrap().len(), 1);
    assert_eq!(own["regions"][0]["extrudable"], true);

    // Move it onto the plain one with its own spec: still whole, still
    // detached, the plain drawing unchanged.
    let isl = island_keys(&own);
    let moved = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.move",
        json!({ "plane": spec.clone(), "units": isl, "offset": [0.05, 0.05, 0.0] }),
    );
    let made = first(&moved)["units"].as_array().unwrap().clone();
    assert!(!made.is_empty());
    assert!(made.iter().all(|u| u["detached"] == true), "{made:?}");
    let own = units(&mut conn, &mut doc, spec.clone());
    assert_eq!(all_edges(&own).len(), 4);
    let (lo, hi) = extent(&own);
    assert!(close(lo[0], 1.05) && close(hi[1], 1.25), "{lo:?} {hi:?}");
    assert_eq!(units(&mut conn, &mut doc, ground()), plain_before);
    assert_eq!(detached_sketches(&mut conn, &mut doc).len(), 1);

    // Deleting all of it empties and removes the sketch.
    let isl = island_keys(&own);
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.sketch.delete",
        json!({ "plane": spec, "units": isl }),
    );
    assert!(detached_sketches(&mut conn, &mut doc).is_empty());
    assert_eq!(units(&mut conn, &mut doc, ground()), plain_before);
}

/// Draw-then-extrude in one transaction, as on the ground: the detached
/// circle's `region_id` extrudes into a new Object beside the box; and a
/// detached region extrudes by unit key through its `{sketch}` spec.
#[test]
fn extruding_a_detached_region_makes_a_new_object() {
    let (mut conn, mut doc, object_id) = setup();
    let r = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.sketch.draw_circle", "as": "c", "params": {
                "plane": top_plane(&object_id),
                "center": [0.2, 0.2, 0.2], "radius": 0.02, "detached": true
            }},
            { "method": "hew.solid.extrude", "params": {
                "region": { "$ref": "c#/region_id" }, "distance": -0.07
            }}
        ]}),
    );
    let cutter = r["results"][1]["object_id"].as_str().unwrap();
    assert_ne!(cutter, object_id, "a new Object, not the box");
    assert_eq!(doc.visible_object_ids().len(), 2);
    let top = units(&mut conn, &mut doc, top_plane(&object_id));
    assert!(top["units"].as_array().unwrap().is_empty(), "{top}");
    assert!(
        detached_sketches(&mut conn, &mut doc).is_empty(),
        "the extruded sketch is consumed"
    );

    // By unit key through the sketch's own spec.
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_polygon",
        json!({
            "plane": top_plane(&object_id),
            "center": [0.1, 0.1, 0.2], "radius": 0.03, "sides": 6, "detached": true
        }),
    );
    let spec = json!({ "sketch": first(&r)["sketch"] });
    let own = units(&mut conn, &mut doc, spec.clone());
    let key = own["regions"][0]["key"].clone();
    let made = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.extrude",
        json!({ "plane": spec, "region": key, "distance": 0.05 }),
    );
    assert_ne!(first(&made)["object_id"].as_str().unwrap(), object_id);
    assert_eq!(doc.visible_object_ids().len(), 3);
}
