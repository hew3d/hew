//! Conformance coverage for editable imprints: `hew.solid.imprints`,
//! `move_imprint`, `rotate_imprint`, `scale_imprint`, `delete_imprint`
//! (docs/agents/HEW_API.md §7's `hew.solid` semantics notes;
//! `crates/api/src/commands/solid.rs`'s module doc comment for the
//! shared imprint-locator resolution).
//!
//! Covers: a sub-face imprint (a rect drawn clear of the face boundary)
//! listed, moved, rotated, scaled, and deleted, checking `hew.solid.imprints`'s
//! reported shape along the way; a chord imprint (a rect drawn flush
//! against one edge) listed, moved along that edge, and deleted, via the
//! SAME `imprint` locator shape — a point on the chord's own line,
//! resolved as an edge once the face reading finds no sub-face there
//! (`resolve_imprint`'s fallback); and two refusals — an off-plane move
//! (`not_in_plane`) and a locator that names neither kind of imprint
//! (`not_an_imprint`).

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// ----------------------------------------------------------------- fixtures
// (mirrors commands_face_imprint.rs's fixture set — each integration test
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

/// An imprint locator by point, on `object` — the same shape as a face
/// locator (`commands/solid.rs`'s module doc comment): resolved as a face
/// first, falling back to an edge reading for a chord.
fn imprint_at(object: &str, at: [f64; 3]) -> Value {
    json!({ "object": object, "at": at })
}

/// A bare (non-`hew.doc.transact`) mutating call auto-wraps as a
/// one-command transaction (api-implementation-conventions.md), so its
/// result lands at `results[0]` rather than at the top level.
fn result_object_id(v: &Value) -> &str {
    v["results"][0]["object_id"]
        .as_str()
        .expect("a bare mutating call's result carries results[0].object_id")
}

fn approx_point(v: &Value, expected: [f64; 3], tol: f64) {
    let arr = v.as_array().expect("point is a 3-array");
    assert_eq!(arr.len(), 3, "point has 3 components");
    for (i, &e) in expected.iter().enumerate() {
        let got = arr[i].as_f64().expect("component is a number");
        assert!(
            (got - e).abs() <= tol,
            "component {i}: expected {e}, got {got} (tol {tol})"
        );
    }
}

// ============================================================= sub-face

#[test]
fn sub_face_imprint_is_listed_moved_rotated_scaled_and_deleted() {
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

    // Draw a 0.1x0.1 rect clear of the top face's boundary — a sub-face
    // imprint, centered at (0.1, 0.1, 0.2).
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            "corner_a": [0.05, 0.05, 0.2],
            "corner_b": [0.15, 0.15, 0.2]
        }),
    );

    // hew.solid.imprints reports it as one sub-face, no curve claim
    // (a rectangle, not a drawn circle), no nested imprints, and an `at`
    // point strictly inside its 0.05..0.15 square.
    let listed = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let imprints = listed["imprints"].as_array().unwrap();
    assert_eq!(imprints.len(), 1, "exactly one drawn imprint");
    let sub_face = &imprints[0];
    assert_eq!(sub_face["kind"], "sub_face");
    assert_eq!(sub_face["nested"], 0);
    assert!(sub_face["curve"].is_null());
    let loop_pts = sub_face["loop"].as_array().unwrap();
    assert_eq!(loop_pts.len(), 4, "a rectangle's outer loop has 4 vertices");
    let at = sub_face["at"].as_array().unwrap();
    let (ax, ay, az) = (
        at[0].as_f64().unwrap(),
        at[1].as_f64().unwrap(),
        at[2].as_f64().unwrap(),
    );
    assert!(
        (0.05..=0.15).contains(&ax) && (0.05..=0.15).contains(&ay),
        "`at` sits inside the drawn square: got ({ax}, {ay})"
    );
    assert!((az - 0.2).abs() < 1e-9, "`at` sits on the top face's plane");

    // Move it by [0.02, 0.02, 0.0] — the reported `at` is a valid locator
    // for the move.
    let moved = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.move_imprint",
        json!({
            "imprint": { "object": object_id, "at": sub_face["at"].clone() },
            "offset": [0.02, 0.02, 0.0]
        }),
    );
    assert_eq!(result_object_id(&moved), object_id);

    // The imprint's now centered at (0.12, 0.12, 0.2) — still a 0.1x0.1
    // square, so that point stays a valid interior locator through the
    // rotate and scale below.
    let center = [0.12, 0.12, 0.2];
    let rotated = call_ok_one_undo(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.rotate_imprint",
        json!({
            "imprint": imprint_at(&object_id, center),
            "angle": 0.3,
            "about": center
        }),
    );
    assert_eq!(result_object_id(&rotated), object_id);

    let scaled = call_ok_one_undo(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.scale_imprint",
        json!({
            "imprint": imprint_at(&object_id, center),
            "factor": 1.2,
            "about": center
        }),
    );
    assert_eq!(result_object_id(&scaled), object_id);

    // Re-list to confirm the scale actually grew the loop (half-width
    // 0.05 * 1.2 = 0.06 from the same center) before deleting it.
    let listed = call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let imprints = listed["imprints"].as_array().unwrap();
    assert_eq!(imprints.len(), 1);
    approx_point(&imprints[0]["at"], center, 1e-6);

    let deleted = call_ok_one_undo(
        &mut conn,
        &mut doc,
        9,
        "hew.solid.delete_imprint",
        json!({ "imprint": imprint_at(&object_id, center) }),
    );
    assert_eq!(result_object_id(&deleted), object_id);

    let after = call_ok(
        &mut conn,
        &mut doc,
        10,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    assert!(
        after["imprints"].as_array().unwrap().is_empty(),
        "the imprint is gone after delete_imprint"
    );
}

/// A negative factor is accepted: the shape turns 180 degrees about the
/// anchor and scales by |factor|, as one undo step; a zero factor is still
/// refused as a bad parameter.
#[test]
fn scale_imprint_by_a_negative_factor_turns_it_about_the_anchor() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [0.4, 0.4, 0.0],
        0.2,
    );
    // A 0.1x0.05 rect off the anchor, so the half turn shows.
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.2, 0.2, 0.2]) },
            "corner_a": [0.1, 0.1, 0.2],
            "corner_b": [0.2, 0.15, 0.2]
        }),
    );
    let about = [0.2, 0.2, 0.2];

    let zero = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.scale_imprint",
        json!({
            "imprint": imprint_at(&object_id, [0.15, 0.125, 0.2]),
            "factor": 0.0,
            "about": about
        }),
    );
    let err = zero.error.expect("a zero factor is refused");
    assert!(err.message.contains("nonzero"), "{}", err.message);

    let scaled = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.scale_imprint",
        json!({
            "imprint": imprint_at(&object_id, [0.15, 0.125, 0.2]),
            "factor": -1.5,
            "about": about
        }),
    );
    assert_eq!(result_object_id(&scaled), object_id);

    // Each corner p lands at about - 1.5 (p - about).
    let listed = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let imprints = listed["imprints"].as_array().unwrap();
    assert_eq!(imprints.len(), 1);
    let loop_pts = imprints[0]["loop"].as_array().unwrap();
    assert_eq!(loop_pts.len(), 4);
    for want in [
        [0.2, 0.275, 0.2],
        [0.35, 0.275, 0.2],
        [0.35, 0.35, 0.2],
        [0.2, 0.35, 0.2],
    ] {
        assert!(
            loop_pts.iter().any(|p| {
                p.as_array()
                    .unwrap()
                    .iter()
                    .zip(want)
                    .all(|(c, w)| (c.as_f64().unwrap() - w).abs() < 1e-9)
            }),
            "corner {want:?} in {loop_pts:?}"
        );
    }
}

// =================================================================== chord

#[test]
fn chord_imprint_is_listed_moved_along_its_edge_and_deleted() {
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

    // A rect flush against the top face's x=0 edge (one side runs along
    // the boundary from y=0.05 to y=0.15) — a chord, not a sub-face.
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.05, 0.1, 0.2]) },
            "corner_a": [0.0, 0.05, 0.2],
            "corner_b": [0.1, 0.15, 0.2]
        }),
    );

    let listed = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let imprints = listed["imprints"].as_array().unwrap();
    let chord = imprints
        .iter()
        .find(|f| f["kind"] == "chord")
        .expect("the flush rect imprints as a chord");
    let path = chord["path"].as_array().unwrap();
    assert!(path.len() >= 2, "a chord's path has at least two vertices");
    // `at` (the midpoint of the run's first segment — HEW_API.md's
    // imprints semantics note) is itself the round-trip locator: it sits
    // exactly on the shared boundary between the drawn rect and the
    // remaining L-shaped face, which is precisely where
    // `locate::resolve_face`'s strict inside/outside test is a coin flip
    // — `resolve_imprint`'s edge fallback is what this test exercises, so
    // no hand-picked coordinate is asserted here, only that it resolves.
    let at = chord["at"].clone();

    // Slide it along the edge it was drawn from (y += 0.02) — legal per
    // `Document::transform_chord`'s doc comment: both endpoints of the
    // run stay on the merged face's boundary (the box's full x=0 edge,
    // spanning y in [0, 0.2]).
    let moved = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.move_imprint",
        json!({
            "imprint": { "object": object_id, "at": at },
            "offset": [0.0, 0.02, 0.0]
        }),
    );
    assert_eq!(result_object_id(&moved), object_id);

    // Re-list to find the moved chord's new locator, then delete it.
    let listed = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let imprints = listed["imprints"].as_array().unwrap();
    let chord = imprints
        .iter()
        .find(|f| f["kind"] == "chord")
        .expect("still a chord after sliding along the edge");
    let at = chord["at"].clone();

    let deleted = call_ok_one_undo(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.delete_imprint",
        json!({ "imprint": { "object": object_id, "at": at } }),
    );
    assert_eq!(result_object_id(&deleted), object_id);

    let after = call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    assert!(
        after["imprints"].as_array().unwrap().is_empty(),
        "the chord is gone after delete_imprint"
    );
}

// =============================================================== refusals

#[test]
fn move_imprint_off_the_faces_plane_refuses_not_in_plane() {
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
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            "corner_a": [0.05, 0.05, 0.2],
            "corner_b": [0.15, 0.15, 0.2]
        }),
    );
    let before = doc.save();

    // A translation with a z component tilts the imprint off the top
    // face's plane — refused, not silently projected.
    let data = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.move_imprint",
        json!({
            "imprint": imprint_at(&object_id, [0.1, 0.1, 0.2]),
            "offset": [0.0, 0.0, 0.05]
        }),
    );
    assert_eq!(data["refusal"], "not_in_plane");
    assert_eq!(
        doc.save(),
        before,
        "a refused move leaves the document untouched"
    );
}

#[test]
fn move_imprint_on_a_plain_face_point_refuses_not_an_imprint() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    // A plain box, nothing drawn on any of its faces.
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [0.2, 0.2, 0.0],
        0.2,
    );
    let before = doc.save();

    // A point well inside the top face, far from every edge: resolves as
    // a face (the plain top face itself, not an imprint) and finds no
    // edge nearby either.
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.move_imprint",
        json!({
            "imprint": imprint_at(&object_id, [0.1, 0.1, 0.2]),
            "offset": [0.01, 0.0, 0.0]
        }),
    );
    assert_eq!(data["refusal"], "not_an_imprint");
    assert_eq!(
        doc.save(),
        before,
        "a refused move leaves the document untouched"
    );
}

// ========================================================= curve units

#[test]
fn imprints_lists_a_circle_cut_apart_by_a_rectangle_as_one_curve_unit() {
    // A circle drawn across the rectangle's right side lands as two runs,
    // each listed as a chord; `curve_units` names them as the one circle,
    // each run with the `at` locator its chord entry carries.
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
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.02, 0.02, 0.2]) },
            "corner_a": [0.05, 0.05, 0.2],
            "corner_b": [0.15, 0.15, 0.2]
        }),
    );
    // Before the circle: no curve units at all (the field is always there).
    let listed = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    assert_eq!(listed["curve_units"], json!([]));
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_circle",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            "center": [0.15, 0.1, 0.2],
            "radius": 0.02
        }),
    );
    let listed = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let units = listed["curve_units"].as_array().unwrap();
    assert_eq!(units.len(), 1);
    let unit = &units[0];
    assert_eq!(unit["closed"], true);
    approx_point(&unit["curve"]["center"], [0.15, 0.1, 0.2], 1e-12);
    assert!((unit["curve"]["radius"].as_f64().unwrap() - 0.02).abs() < 1e-12);
    let runs = unit["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    let chord_ats: Vec<&Value> = listed["imprints"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["kind"] == "chord")
        .map(|f| &f["at"])
        .collect();
    for run in runs {
        assert!(
            chord_ats.contains(&&run["at"]),
            "each run is a listed chord"
        );
        assert!(run["path"].as_array().unwrap().len() >= 2);
    }
}

/// A whole circle no line cuts lists as a one-run curve unit whose run
/// locator names that circle: passing it to move_imprint moves the circle.
#[test]
fn a_whole_circles_run_locator_moves_it() {
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
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_circle",
        json!({
            "plane": { "face": face_at(&object_id, [0.02, 0.02, 0.2]) },
            "center": [0.14, 0.1, 0.2],
            "radius": 0.03
        }),
    );
    let listed = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.imprints",
        json!({ "object": object_id }),
    );
    let at = listed["curve_units"][0]["runs"][0]["at"].clone();
    assert!(at.is_array(), "the run carries a locator: {listed}");
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.move_imprint",
        json!({ "imprint": { "object": object_id, "at": at }, "offset": [0.01, 0.0, 0.0] }),
    );
}
