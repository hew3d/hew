//! Conformance coverage for `hew.sketch.*` and `hew.solid.*`
//! (docs/design/api-implementation-conventions.md's testing bar):
//! success + refusal paths per command, the §6.1 `$ref`/`$face`
//! chaining shape, and the one-envelope-one-undo/byte-identical-undo
//! property for every mutating envelope.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};

// ----------------------------------------------------------------- fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello(conn: &mut Connection, doc: &mut Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
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

/// A plain (non-`transact`) model-mutating request dispatches as exactly
/// a one-command transaction (docs/HEW_API.md §6.1); its result is the
/// wrapper's `results[0]`, not the bare command result.
fn single(result: Value) -> Value {
    result["results"][0].clone()
}

fn ground_rect(corner_a: [f64; 3], corner_b: [f64; 3]) -> Value {
    json!({ "plane": { "ground": true }, "corner_a": corner_a, "corner_b": corner_b })
}

/// Draws a rectangle then extrudes it into a solid, in one transaction —
/// the §6.1 example's exact chaining shape (`$ref` from the draw result
/// into the extrude params). Returns the new object's public id.
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

// ============================================================= hew.sketch

#[test]
fn draw_rect_then_extrude_chains_through_ref_like_the_spec_6_1_example() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "label": "Leg",
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect([0.0, 0.0, 0.0], [0.1, 0.1, 0.0]) },
                { "method": "hew.solid.extrude", "as": "leg", "params": {
                    "region": { "$ref": "profile#/region_id" },
                    "distance": 0.45
                }}
            ]
        }),
    );
    let profile_result = &result["results"][0];
    assert!(
        profile_result["sketch"]
            .as_str()
            .unwrap()
            .starts_with("skt_")
    );
    assert!(
        profile_result["region_id"]
            .as_str()
            .unwrap()
            .starts_with("rgn_")
    );
    let object_id = result["results"][1]["object_id"].as_str().unwrap();
    assert!(object_id.starts_with("obj_"));
}

#[test]
fn draw_rect_off_plane_corner_refuses_point_off_plane() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let before = doc.save();

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        ground_rect([0.0, 0.0, 0.0], [0.1, 0.1, 0.5]),
    );
    assert_eq!(data["refusal"], "point_off_plane");
    assert_eq!(
        doc.save(),
        before,
        "a refused envelope leaves the document untouched"
    );
}

#[test]
fn draw_circle_extrudes_into_a_cylinder_and_carries_a_curve_id() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_circle", "as": "c", "params": {
                    "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05
                }},
                { "method": "hew.solid.extrude", "as": "cyl", "params": {
                    "region": { "$ref": "c#/region_id" }, "distance": 0.1
                }}
            ]
        }),
    );
    assert!(
        result["results"][0]["curve_id"]
            .as_str()
            .unwrap()
            .starts_with("crv_")
    );
    assert!(
        result["results"][1]["object_id"]
            .as_str()
            .unwrap()
            .starts_with("obj_")
    );
}

#[test]
fn draw_circle_segments_below_floor_refuses_typed() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05, "segments": 3 }),
    );
    assert_eq!(data["refusal"], "segments_below_floor");
}

#[test]
fn draw_circle_segments_above_cap_refuses_typed() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05, "segments": 2000 }),
    );
    assert_eq!(data["refusal"], "segments_above_cap");
}

#[test]
fn draw_polygon_creates_exactly_one_region_and_a_curve() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_polygon",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05, "sides": 6 }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
    assert!(result["region_id"].as_str().unwrap().starts_with("rgn_"));
    assert!(result["curve_id"].as_str().unwrap().starts_with("crv_"));
}

#[test]
fn draw_polygon_needs_at_least_3_sides() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_polygon",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05, "sides": 2 }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn draw_arc_creates_an_open_chain_with_no_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": PI
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 0);
    assert!(result.get("region_id").is_none());
    assert!(result["curve_id"].as_str().unwrap().starts_with("crv_"));
}

/// The exact area of the CHORDED n-gon wedge a `close: "pie"` arc actually
/// draws (n triangles fanning out from the center, each with two sides of
/// length `radius` and included angle `sweep / n`) — NOT the smooth
/// analytic sector area `(sweep / 2) * radius^2`, which the faceted shape
/// only approaches as `n` grows. Comparing against this exact formula
/// (rather than the analytic one loosened by some chord-count-dependent
/// tolerance) is what lets the assertion below use a tight epsilon.
fn wedge_area(radius: f64, sweep: f64, n: usize) -> f64 {
    n as f64 * 0.5 * radius * radius * (sweep / n as f64).sin()
}

#[test]
fn draw_arc_pie_closes_exactly_one_region_with_the_chorded_wedge_area() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let sweep = PI / 3.0;
    let radius = 0.05;
    let n = 8;
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": radius,
            "start_angle": 0.0, "end_angle": sweep, "segments": n, "close": "pie"
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
    let region_id = result["region_id"].as_str().unwrap();
    assert!(result["curve_id"].as_str().unwrap().starts_with("crv_"));

    let sketch = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": result["sketch"].as_str().unwrap() }),
    );
    let regions = sketch["regions"].as_array().unwrap();
    let region = regions
        .iter()
        .find(|r| r["id"] == region_id)
        .expect("the pie's region is in its sketch's summary");
    let area = region["area"].as_f64().unwrap();
    let expected = wedge_area(radius, sweep, n);
    assert!(
        (area - expected).abs() < 1e-12,
        "pie area {area} should equal the exact chorded wedge area {expected}"
    );
}

#[test]
fn draw_arc_segment_closes_exactly_one_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": PI / 3.0, "segments": 8, "close": "segment"
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
    assert!(result["region_id"].as_str().unwrap().starts_with("rgn_"));
    assert!(result["curve_id"].as_str().unwrap().starts_with("crv_"));
}

/// Regression pin: `close: "pie"` with a single chord (`segments: 1`) and
/// a half-turn sweep used to commit a collinear, essentially zero-area
/// "wedge" — the loop's three points (arc-start, arc-end, center) sit on
/// one line exactly when the sweep is π, since the two arc points are
/// then diametrically opposite and the center is their midpoint.
/// `polygon_is_simple` is vacuous for a 3-point loop (no non-adjacent
/// edge pair exists to test), and nothing else checked loop area, so
/// this used to succeed. Two chords is the floor now, for every close
/// mode and every sweep — refused as a static param defect (pure
/// geometry of the request, no document state needed).
#[test]
fn draw_arc_pie_with_a_single_chord_and_a_half_turn_sweep_refuses_too_few_segments() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let before = doc.save();

    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": PI, "segments": 1, "close": "pie"
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert_eq!(
        doc.save(),
        before,
        "the rejected call left the document untouched"
    );
}

/// Regression pin: `close: "segment"` with a single chord used to
/// silently produce NO region at all — the implicit closing edge
/// (arc-end → arc-start) exactly retraces the one chord already drawn
/// (arc-start → arc-end), so nothing new closes, breaking the documented
/// promise that `close != "open"` closes a region. This held for EVERY
/// sweep with `segments: 1` (not just a special angle, unlike the pie
/// case above), so any sweep demonstrates it.
#[test]
fn draw_arc_segment_with_a_single_chord_refuses_too_few_segments() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let before = doc.save();

    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": PI / 3.0, "segments": 1, "close": "segment"
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert_eq!(
        doc.save(),
        before,
        "the rejected call left the document untouched"
    );
}

/// Regression pin: the same no-region defect above, reached WITHOUT an
/// explicit `segments` — any sweep under ~11° rounds `48 * sweep / TAU`
/// down to a single chord by the ordinary proportional-density default,
/// so a caller drawing a small `close: "segment"` slice never even had
/// to ask for `segments: 1` to hit it. The default is now floored at the
/// same 2-chord minimum an explicit count would need, so it always
/// closes a real region.
#[test]
fn draw_arc_segment_default_density_under_the_closed_floor_still_closes_a_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    // 5° — well under the ~11° threshold where `round(48 * sweep / TAU)`
    // first reaches 2, so the OLD default would have picked 1 chord.
    let sweep = PI / 36.0;
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": sweep, "close": "segment"
        }),
    ));
    assert_eq!(
        result["region_ids"].as_array().unwrap().len(),
        1,
        "a closed arc must always close a region, even at a tiny default-density sweep"
    );
}

// -------------------------------------------------- adjacent-edge coverage
// (checked alongside the three defects above, not defects themselves —
// each is confirmed here to behave correctly rather than merely assumed.)

/// A negative sweep (`end_angle < start_angle`) just traces the arc the
/// other way around; `close: "pie"` should still close exactly one
/// region with the same chorded-wedge area as the equivalent positive
/// sweep (`SplitFaceInner`/the sketch region-builder normalize winding
/// from the loop's own signed area, so direction alone never wins or
/// loses a valid closure).
#[test]
fn draw_arc_pie_with_a_negative_sweep_closes_one_region_with_the_same_wedge_area() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let radius = 0.05;
    let n = 8;
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": radius,
            // end before start: sweep = -PI/3.
            "start_angle": PI / 3.0, "end_angle": 0.0, "segments": n, "close": "pie"
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
    let region_id = result["region_id"].as_str().unwrap();
    let sketch = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": result["sketch"].as_str().unwrap() }),
    );
    let area = sketch["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == region_id)
        .unwrap()["area"]
        .as_f64()
        .unwrap();
    let expected = wedge_area(radius, PI / 3.0, n);
    assert!(
        (area - expected).abs() < 1e-12,
        "a negative sweep should close the same wedge area as its positive mirror: \
         got {area}, expected {expected}"
    );
}

/// A `close: "segment"` sweep past a half turn (here 3π/2, a 270° major
/// segment) still closes exactly one simple region — the arc points are
/// in monotonic angular order and the single closing chord connects only
/// the two ends, so the loop is star-shaped about the center and can
/// never self-cross regardless of how much of the circle it spans (short
/// of a full turn, which is refused separately as a param conflict).
#[test]
fn draw_arc_segment_past_a_half_turn_still_closes_a_simple_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let sweep = 3.0 * PI / 2.0;
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": sweep, "segments": 16, "close": "segment"
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
}

/// A `close: "pie"` sweep just short of a full turn (the center's two
/// closing spokes nearly touching the arc's two ends) is still a valid,
/// if thin, wedge — not the already-closed full-turn special case (which
/// stays refused, tested above) and not degenerate (the spoke separation
/// shrinks continuously with the gap to TAU; it only reaches zero AT the
/// full turn, which is a different, already-refused shape). Confirmed
/// via the same exact chorded-wedge-area formula the ordinary pie test
/// uses.
#[test]
fn draw_arc_pie_with_sweep_almost_a_full_turn_still_closes_a_valid_thin_wedge() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let radius = 0.05;
    let n = 48;
    let sweep = TAU - 0.01;
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": radius,
            "start_angle": 0.0, "end_angle": sweep, "segments": n, "close": "pie"
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
    let region_id = result["region_id"].as_str().unwrap();
    let sketch = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": result["sketch"].as_str().unwrap() }),
    );
    let area = sketch["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == region_id)
        .unwrap()["area"]
        .as_f64()
        .unwrap();
    let expected = wedge_area(radius, sweep, n);
    assert!(
        (area - expected).abs() < 1e-9,
        "a near-full-turn pie should still equal the exact chorded wedge area: \
         got {area}, expected {expected}"
    );
}

#[test]
fn draw_arc_full_turn_with_a_close_refuses_static_param_conflict() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let before = doc.save();

    for close in ["pie", "segment"] {
        let r = call(
            &mut conn,
            &mut doc,
            2,
            "hew.sketch.draw_arc",
            json!({
                "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
                "start_angle": 0.0, "end_angle": TAU, "close": close
            }),
        );
        assert_eq!(
            r.error.unwrap().code,
            codes::INVALID_PARAMS,
            "close: \"{close}\" on a full-turn sweep is a static param conflict, not a refusal"
        );
    }
    assert_eq!(
        doc.save(),
        before,
        "the rejected calls left the document untouched"
    );
}

#[test]
fn draw_arc_unknown_close_value_refuses_static_param_conflict() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.05,
            "start_angle": 0.0, "end_angle": PI, "close": "wedge"
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn draw_line_needs_at_least_2_points() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({ "plane": { "ground": true }, "points": [[0.0, 0.0, 0.0]] }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn draw_line_closing_a_triangle_creates_one_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({
            "plane": { "ground": true },
            "points": [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.05, 0.1, 0.0], [0.0, 0.0, 0.0]]
        }),
    ));
    assert_eq!(result["region_ids"].as_array().unwrap().len(), 1);
}

#[test]
fn offset_grows_a_region_boundary_into_a_new_region() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let drawn = single(call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        ground_rect([0.0, 0.0, 0.0], [0.2, 0.2, 0.0]),
    ));
    let region_id = drawn["region_id"].as_str().unwrap().to_string();

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.offset",
        json!({ "region": region_id.clone(), "distance": 0.02 }),
    ));
    let new_region = result["region_id"].as_str().unwrap();
    assert_ne!(new_region, region_id);
}

#[test]
fn offset_unknown_region_refuses_unknown_entity() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.offset",
        json!({ "region": "rgn_deadbeef_1", "distance": 0.02 }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

// Face-imprint drawing (plane spec `{"face": <locator>}`) is covered in
// commands_face_imprint.rs — draw_on_a_face_is_unimplemented_at_v0 lived
// here while the face path refused typed; it's superseded now that the
// path is implemented (docs/HEW_API.md §9: a documented refusal may
// become a success in a later release).

// =============================================================== hew.solid

#[test]
fn extrude_mints_base_top_and_side_face_tokens_usable_by_a_later_push_pull() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect([0.0, 0.0, 0.0], [0.2, 0.2, 0.0]) },
                { "method": "hew.solid.extrude", "as": "leg", "params": {
                    "region": { "$ref": "profile#/region_id" }, "distance": 0.2
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "leg#top" }, "distance": 0.05
                }}
            ]
        }),
    );
    let pushed = &result["results"][2];
    let object_id = pushed["object_id"].as_str().unwrap();
    assert_eq!(
        object_id,
        result["results"][1]["object_id"].as_str().unwrap()
    );
}

#[test]
fn push_pull_a_side_face_by_point_locator_moves_the_object_in_place() {
    let mut conn = Connection::new(Profile::Core, "test");
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

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.push_pull",
        json!({
            "face": { "object": object_id.clone(), "at": [0.0, 0.1, 0.1] },
            "distance": 0.02
        }),
    ));
    assert_eq!(result["object_id"].as_str().unwrap(), object_id);
}

#[test]
fn push_pull_zero_distance_refuses_distance_too_small() {
    let mut conn = Connection::new(Profile::Core, "test");
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
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.push_pull",
        json!({
            "face": { "object": object_id, "at": [0.0, 0.1, 0.1] },
            "distance": 0.0
        }),
    );
    assert_eq!(
        data["refusal"], "distance_too_small",
        "the push_pull -> KernelOp::PushPull path's refusal must NOT collapse to the generic 'push_pull' code"
    );
}

#[test]
fn push_pull_face_locator_unknown_object_refuses() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.solid.push_pull",
        json!({ "face": { "object": "obj_deadbeef", "at": [0.0, 0.0, 0.0] }, "distance": 0.02 }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn extrude_unknown_region_refuses_unknown_entity() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.solid.extrude",
        json!({ "region": "rgn_deadbeef_1", "distance": 0.1 }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn extrude_distance_too_small_refuses_typed() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let drawn = single(call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        ground_rect([0.0, 0.0, 0.0], [0.1, 0.1, 0.0]),
    ));
    let region_id = drawn["region_id"].as_str().unwrap().to_string();
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.extrude",
        json!({ "region": region_id, "distance": 0.0 }),
    );
    assert_eq!(data["refusal"], "distance_too_small");
}

fn two_overlapping_boxes(conn: &mut Connection, doc: &mut Document) -> (String, String) {
    let a = build_box(conn, doc, 2, [0.0, 0.0, 0.0], [0.2, 0.2, 0.0], 0.2);
    let b = build_box(conn, doc, 3, [0.1, 0.1, 0.0], [0.3, 0.3, 0.0], 0.2);
    (a, b)
}

#[test]
fn union_combines_two_overlapping_boxes() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let (a, b) = two_overlapping_boxes(&mut conn, &mut doc);
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.union",
        json!({ "a": a, "b": b }),
    ));
    let result_id = result["result"].as_str().unwrap();
    assert!(result_id.starts_with("obj_") || result_id.starts_with("grp_"));
}

#[test]
fn subtract_combines_two_overlapping_boxes() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let (a, b) = two_overlapping_boxes(&mut conn, &mut doc);
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.subtract",
        json!({ "a": a, "b": b }),
    ));
    let result_id = result["result"].as_str().unwrap();
    assert!(result_id.starts_with("obj_") || result_id.starts_with("grp_"));
}

#[test]
fn intersect_combines_two_overlapping_boxes() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let (a, b) = two_overlapping_boxes(&mut conn, &mut doc);
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.intersect",
        json!({ "a": a, "b": b }),
    ));
    let result_id = result["result"].as_str().unwrap();
    assert!(result_id.starts_with("obj_") || result_id.starts_with("grp_"));
}

#[test]
fn boolean_same_operand_twice_refuses_degenerate_contact() {
    let mut conn = Connection::new(Profile::Core, "test");
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
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.union",
        json!({ "a": object_id.clone(), "b": object_id }),
    );
    assert_eq!(data["refusal"], "degenerate_contact");
}

#[test]
fn slice_cuts_a_box_into_two_solids() {
    let mut conn = Connection::new(Profile::Core, "test");
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

    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.slice",
        json!({
            "object": object_id,
            "plane": { "origin": [0.0, 0.0, 0.1], "normal": [0.0, 0.0, 1.0] }
        }),
    ));
    let positive = result["positive"].as_str().unwrap();
    let negative = result["negative"].as_str().unwrap();
    assert!(positive.starts_with("obj_"));
    assert!(negative.starts_with("obj_"));
    assert_ne!(positive, negative);
}

#[test]
fn slice_plane_missing_the_solid_refuses_typed() {
    let mut conn = Connection::new(Profile::Core, "test");
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
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.slice",
        json!({
            "object": object_id,
            "plane": { "origin": [0.0, 0.0, 5.0], "normal": [0.0, 0.0, 1.0] }
        }),
    );
    assert_eq!(data["refusal"], "plane_misses_solid");
}

#[test]
fn follow_me_edges_path_is_unimplemented_at_v0() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let drawn = single(call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        ground_rect([0.0, 0.0, 0.0], [0.05, 0.05, 0.0]),
    ));
    let region_id = drawn["region_id"].as_str().unwrap().to_string();
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.follow_me",
        json!({
            "profile": region_id,
            "path": { "edges": [{ "object": "obj_deadbeef", "at": [0.0, 0.0, 0.0] }] }
        }),
    );
    assert_eq!(data["refusal"], "unimplemented");
}

#[test]
fn follow_me_sweeps_a_square_profile_around_a_circular_path() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    // The path: a circle on the ground plane, radius 0.1 — its facet ring
    // starts at (radius, 0, 0) with a tangent of +Y there (draw_circle's
    // own basis: u = +X, v = +Y for the ground plane).
    let path_result = single(call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.1 }),
    ));
    let curve_id = path_result["curve_id"].as_str().unwrap().to_string();

    // The profile: a small square on the XZ plane (normal +Y), centered
    // on the path's start point (0.1, 0, 0) — perpendicular to the path
    // tangent there, as Follow Me requires.
    let profile_result = single(call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "origin": [0.1, 0.0, 0.0], "normal": [0.0, 1.0, 0.0] },
            "corner_a": [0.08, 0.0, -0.02],
            "corner_b": [0.12, 0.0, 0.02]
        }),
    ));
    let region_id = profile_result["region_id"].as_str().unwrap().to_string();

    // follow_me consumes the profile's scaffolding exactly like extrude
    // (docs/HEW_API.md §7 semantics notes) and, like extrude, now undoes
    // byte-identically: `Sketch::remove_edges` frees vertex/edge slots in
    // descending id order so `Sketch::restore_edges`'s LIFO slot reuse
    // lands every re-inserted vertex/edge back in its ORIGINAL slot instead
    // of the mirror-reversed one (see
    // `crates/kernel/tests/region_consume_undo_reversal_specs.rs`'s module
    // doc for the mechanism and fix, and
    // `region_consume_undo_reversal_property_specs.rs` for the property
    // coverage). `call_ok_one_undo` asserts exactly that.
    let result = single(call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.follow_me",
        json!({ "profile": region_id, "path": { "curve": curve_id } }),
    ));
    assert!(result["object_id"].as_str().unwrap().starts_with("obj_"));
}

#[test]
fn a_refused_mid_transaction_envelope_leaves_the_document_byte_identical() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let before = doc.save();

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": ground_rect([0.0, 0.0, 0.0], [0.1, 0.1, 0.0]) },
                { "method": "hew.solid.extrude", "as": "leg", "params": {
                    "region": { "$ref": "profile#/region_id" }, "distance": 0.0
                }}
            ]
        }),
    );
    assert_eq!(data["refusal"], "distance_too_small");
    assert_eq!(data["failed_index"], 1);
    assert_eq!(
        doc.save(),
        before,
        "an aborted transaction leaves the document untouched"
    );
    assert_eq!(doc.undo_depth(), 0);
}
