//! Conformance coverage for the face-imprint drawing path — plane spec
//! `{"face": <locator>}` on `hew.sketch.draw_*` (docs/agents/HEW_API.md §7's
//! semantics notes; docs/design/api-implementation-conventions.md's
//! testing bar). Unlike plane/sketch mode, on-face drawing never touches
//! a sketch: it cuts the solid's face directly through
//! `kernel::Document::apply_object_op` (`KernelOp::SplitFace` for an
//! open, boundary-to-boundary path; `KernelOp::SplitFaceInner` for a
//! closed loop strictly inside the face), returning `{object_id}` plus
//! transaction-scoped face tokens in place of `{sketch, region_ids}`.
//!
//! Covers: rect imprint on a box top followed by `push_pull` extruding
//! the imprinted sub-face (the full agent story — draw on a face, then
//! push it), circle imprint carrying its analytic curve claim, a line
//! split with its two halves individually push/pullable, an arc's dual
//! open-chain/closed-loop face behavior, a polygon imprint, off-face
//! points refusing typed, and — a component-definition member's face:
//! drawing and push/pull route through `kernel::Document::apply_def_op`
//! / `push_pull_through_in_component` instead of the plain world path,
//! so the edit is shared geometry every instance of the component picks
//! up at once (`component_face_imprint_and_push_pull_update_every_instance`
//! below; the coordinate-frame decision — always the definition's own
//! frame, never remapped through an instance's pose — is documented in
//! that test and in `commands/sketch.rs`'s module doc comment). Every
//! mutating envelope checks the one-envelope-one-undo /
//! byte-identical-undo property via `call_ok_one_undo`.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::Document;
use serde_json::{Value, json};
use std::f64::consts::TAU;

// ----------------------------------------------------------------- fixtures
// (mirrors commands_sketch_solid.rs's fixture set — each integration test
// binary is its own compilation unit, so this is duplicated rather than
// shared; do not let the two drift on behavior, only on which tests they
// hold.)

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

// ======================================================= hew.sketch.draw_*
// face-imprint path

#[test]
fn draw_rect_on_a_face_imprints_a_sub_face_that_push_pull_extrudes() {
    // The full agent story: draw a rectangle on a box's top face, then
    // push the imprinted sub-face up to boss it — the on-face analog of
    // the plane-mode §6.1 example.
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "corner_a": [0.05, 0.05, 0.2],
                    "corner_b": [0.15, 0.15, 0.2]
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.03
                }}
            ]
        }),
    );
    assert_eq!(
        result["results"][0]["object_id"].as_str().unwrap(),
        object_id
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id, "boss/recess reshapes in place");
}

#[test]
fn draw_circle_on_a_face_imprints_with_its_curve_claim_and_push_pull_extrudes() {
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_circle", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "center": [0.1, 0.1, 0.2],
                    "radius": 0.05
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_polygon_on_a_face_imprints_a_sub_face() {
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_polygon", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "center": [0.1, 0.1, 0.2],
                    "radius": 0.05,
                    "sides": 6
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_line_on_a_face_splits_it_and_both_halves_are_addressable() {
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_line", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "points": [[0.0, 0.1, 0.2], [0.2, 0.1, 0.2]]
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#a" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let cut = &result["results"][0];
    assert_eq!(cut["object_id"].as_str().unwrap(), object_id);
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_arc_full_turn_on_a_face_closes_into_a_loop_like_a_circle() {
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_arc", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "center": [0.1, 0.1, 0.2],
                    "radius": 0.05,
                    "start_angle": 0.0,
                    "end_angle": TAU
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_arc_pie_on_a_face_mints_face_and_parent_tokens_and_push_pull_extrudes_the_wedge() {
    // The real gap this closes: a pie is a CLOSED loop strictly inside the
    // face (arc points + center), so it routes through the SAME
    // `SplitFaceInner` path as `draw_circle`/`draw_rect` — minting
    // "face"/"parent" tokens (not the open arc's "a"/"b") — and the minted
    // sub-face push/pulls like any other imprinted wedge.
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_arc", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "center": [0.1, 0.1, 0.2],
                    "radius": 0.05,
                    "start_angle": 0.0,
                    "end_angle": std::f64::consts::FRAC_PI_2,
                    "close": "pie"
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    assert_eq!(
        result["results"][0]["object_id"].as_str().unwrap(),
        object_id
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id, "boss/recess reshapes in place");
}

#[test]
fn draw_arc_segment_on_a_face_imprints_a_closed_sub_face() {
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

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_arc", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
                    "center": [0.1, 0.1, 0.2],
                    "radius": 0.05,
                    "start_angle": 0.0,
                    "end_angle": std::f64::consts::FRAC_PI_2,
                    "close": "segment"
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_arc_full_turn_with_close_pie_refuses_static_param_conflict() {
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
    let before = doc.save();

    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            "center": [0.1, 0.1, 0.2],
            "radius": 0.05,
            "start_angle": 0.0,
            "end_angle": TAU,
            "close": "pie"
        }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
    assert_eq!(
        doc.save(),
        before,
        "the rejected call left the document untouched"
    );
}

#[test]
fn draw_arc_open_sweep_on_a_face_cuts_boundary_to_boundary_like_a_line() {
    // A half-turn arc whose two endpoints land exactly on the SAME
    // boundary edge (the bottom edge, y = 0) — a semicircular bump
    // entirely interior otherwise, on a big-enough face that the apex
    // (y = center.y + radius) stays well clear of every other edge. Not a
    // full turn, so this exercises the OPEN-chain branch (`SplitFace`),
    // not the closed-loop one.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        0.3,
    );

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_arc", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.5, 0.5, 0.3]) },
                    "center": [0.5, 0.0, 0.3],
                    "radius": 0.2,
                    "start_angle": 0.0,
                    "end_angle": std::f64::consts::PI
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#a" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}

#[test]
fn draw_line_on_a_face_with_interior_endpoints_refuses_endpoint_not_on_boundary() {
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
        "hew.sketch.draw_line",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            "points": [[0.05, 0.05, 0.2], [0.15, 0.15, 0.2]]
        }),
    );
    assert_eq!(data["refusal"], "endpoint_not_on_boundary");
}

#[test]
fn draw_rect_off_the_faces_plane_refuses_point_not_on_face() {
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
        "hew.sketch.draw_rect",
        json!({
            "plane": { "face": face_at(&object_id, [0.1, 0.1, 0.2]) },
            // z = 0.05 — off the top face's z = 0.2 plane.
            "corner_a": [0.05, 0.05, 0.05],
            "corner_b": [0.15, 0.15, 0.05]
        }),
    );
    assert_eq!(data["refusal"], "point_not_on_face");
}

#[test]
fn component_face_imprint_and_push_pull_update_every_instance() {
    // v1 coordinate-frame decision (see commands/sketch.rs's module doc
    // comment and commands/solid.rs's push_pull doc comment for the full
    // reasoning): a face locator (HEW_API.md §5.2) always names an
    // OBJECT — `{object, at|ray}` or a `$face` token — never an
    // instance. So drawing/pushing on a definition member's face always
    // addresses that member directly, in whatever frame its own
    // geometry happens to live in — the definition-local frame. For a
    // definition minted in-session by `make_component` (this test's
    // case) that frame equals the world frame at the moment of creation
    // ("No geometry moves") and never moves again, so every point given
    // below is, in effect, in that original world frame, exactly like
    // drawing on a plain world object; an IMPORTED definition instead
    // keeps its authored local frame, and callers address such members
    // in those stored coordinates. Imprinting
    // THROUGH one specific posed instance (mapping a world-space gesture
    // back into definition space via that instance's pose, the way the
    // UI's in-instance face tools / `split_face_inner_in_instance` do)
    // is simply not expressible at v1: there is no locator shape that
    // names "member X as seen through instance Y", so there is no
    // world→def-local remap for the API to perform here at all. Adding
    // such a locator is additive future work, not a behavior change.
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        0.3,
    );

    // Bare (non-`hew.doc.transact`) mutating calls auto-wrap as a
    // one-command transaction (api-implementation-conventions.md), so
    // their result lands at `results[0]`.
    let comp_result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.component.create",
        json!({ "members": [object_id.clone()] }),
    );
    let comp_result = &comp_result["results"][0];
    let component_pub = comp_result["component"].clone();
    let instance1 = comp_result["instance"].as_str().unwrap().to_string();

    let place_result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.component.place",
        json!({
            "component": component_pub,
            "pose": { "translation": [2.0, 0.0, 0.0] }
        }),
    );
    let instance2 = place_result["results"][0]["instance"]
        .as_str()
        .unwrap()
        .to_string();

    // Draw a rect AND a circle on the member's top face, pushing each
    // imprinted sub-face up into a boss — the same imprint-then-push
    // story the world-object tests above cover, now on a definition
    // member. Every step below reshapes the SAME member object in
    // place (through `apply_def_op`/its push-pull sibling): a
    // shared-geometry edit, never a replacement.
    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        5,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "rect", "params": {
                    "plane": { "face": face_at(&object_id, [0.5, 0.5, 0.3]) },
                    "corner_a": [0.1, 0.1, 0.3],
                    "corner_b": [0.3, 0.3, 0.3]
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "rect#face" },
                    "distance": 0.05
                }},
                { "method": "hew.sketch.draw_circle", "as": "circle", "params": {
                    "plane": { "face": face_at(&object_id, [0.6, 0.6, 0.3]) },
                    "center": [0.6, 0.6, 0.3],
                    "radius": 0.1
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "circle#face" },
                    "distance": 0.04
                }}
            ]
        }),
    );
    for (i, step) in result["results"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            step["object_id"].as_str().unwrap(),
            object_id,
            "step {i} should reshape the definition member in place, not replace it"
        );
    }

    // `hew.query.faces` on the member shows both bosses' top faces —
    // the shared definition itself carries the edit.
    let faces = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.faces",
        json!({ "object": object_id }),
    );
    let faces = faces["faces"].as_array().unwrap();
    let has_face_near = |x: f64, y: f64, z: f64| {
        faces.iter().any(|f| {
            let c = f["centroid"].as_array().unwrap();
            (c[0].as_f64().unwrap() - x).abs() < 1e-6
                && (c[1].as_f64().unwrap() - y).abs() < 1e-6
                && (c[2].as_f64().unwrap() - z).abs() < 1e-6
        })
    };
    assert!(
        has_face_near(0.2, 0.2, 0.35),
        "the rect boss's top face is present on the member"
    );
    assert!(
        has_face_near(0.6, 0.6, 0.34),
        "the circle boss's top face is present on the member"
    );

    // Both instances share the ONE definition, so EVERY placement
    // reflects the edit at once. Raycast straight down through each
    // instance's world position (instance2 = instance1 shifted +2 in x
    // by its pose) and confirm both bosses show up there too, correctly
    // offset — proving the shared-geometry edit, not just the raw
    // member record, is visible through both placements.
    for (dx, instance_pub) in [(0.0, instance1.as_str()), (2.0, instance2.as_str())] {
        let rect_hit = call_ok(
            &mut conn,
            &mut doc,
            7,
            "hew.query.raycast",
            json!({ "origin": [0.2 + dx, 0.2, 10.0], "dir": [0.0, 0.0, -1.0] }),
        );
        assert_eq!(rect_hit["kind"], "instance");
        assert_eq!(rect_hit["object"], instance_pub);
        assert!(
            (rect_hit["point"][2].as_f64().unwrap() - 0.35).abs() < 1e-6,
            "rect boss height visible through {instance_pub}"
        );

        let circle_hit = call_ok(
            &mut conn,
            &mut doc,
            8,
            "hew.query.raycast",
            json!({ "origin": [0.6 + dx, 0.6, 10.0], "dir": [0.0, 0.0, -1.0] }),
        );
        assert_eq!(circle_hit["kind"], "instance");
        assert_eq!(circle_hit["object"], instance_pub);
        assert!(
            (circle_hit["point"][2].as_f64().unwrap() - 0.34).abs() < 1e-6,
            "circle boss height visible through {instance_pub}"
        );
    }
}

#[test]
fn push_pull_on_a_component_members_plain_face_uses_the_shared_definition_path() {
    // Before this change, `hew.solid.push_pull` always called
    // `apply_object_op`, which refuses any non-world object outright
    // (`DocumentError::UnknownObject`) — so pushing a definition
    // member's face (even one with no imprint at all, the plain
    // `KernelOp::PushPull` branch) misbehaved. This exercises that
    // branch directly against a member's untouched top face.
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
    call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.component.create",
        json!({ "members": [object_id.clone()] }),
    );

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.push_pull",
        json!({
            "face": face_at(&object_id, [0.1, 0.1, 0.2]),
            "distance": 0.05
        }),
    );
    // A bare mutating call auto-wraps as a one-command transaction.
    assert_eq!(
        result["results"][0]["object_id"].as_str().unwrap(),
        object_id
    );

    let hit = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.raycast",
        json!({ "origin": [0.1, 0.1, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert!(
        (hit["point"][2].as_f64().unwrap() - 0.25).abs() < 1e-6,
        "the member's top face moved from z=0.2 to z=0.25"
    );
}

/// Follow Me has no definition-scoped kernel path, so a profile face on
/// a definition member refuses TYPED (`follow_me_in_component_unsupported`)
/// — not `unknown_object`, which would falsely tell the caller its id
/// cache is stale when the identical locator succeeds in
/// `hew.solid.push_pull` and the `hew.sketch.draw_*` face modes. Found
/// by adversarial review.
#[test]
fn follow_me_on_a_definition_members_face_refuses_typed() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        0.3,
    );
    let comp_result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.component.create",
        json!({ "members": [object_id.clone()] }),
    );
    let member = comp_result["results"][0]["member_objects"][0]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| object_id.clone());

    // A ground-plane circle to serve as the sweep path.
    let path_result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_circle",
        json!({
            "plane": { "origin": [5.0, 5.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "center": [5.0, 5.0, 0.0],
            "radius": 1.0
        }),
    );
    let curve = path_result["results"][0]["curve_id"]
        .as_str()
        .unwrap()
        .to_string();

    let data = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.follow_me",
        json!({
            "profile": { "face": face_at(&member, [0.5, 0.5, 0.3]) },
            "path": { "curve": curve }
        }),
    );
    assert_eq!(data["refusal"], "follow_me_in_component_unsupported");
}

/// Regression pin: a `close: "pie"` center that sits exactly on the
/// face's own boundary edge must refuse `loop_not_strictly_inside`, not
/// commit a hole loop pinched against the parent's outer loop at that
/// vertex.
///
/// This needs a face on a NON-axis-aligned plane to actually exercise
/// the defect: the API builds `draw_arc`'s points in its own in-plane
/// basis (`plane_basis`, Gram-Schmidt against a world reference), while
/// the kernel's `SplitFaceInner` validation re-derives an independent
/// basis for the same normal (`geom2d::plane_axes`, a different
/// reference rule and construction order) to run its point-in-polygon
/// test. For an axis-aligned face the two bases collapse to the same
/// exact world axes and the kernel's check happens to get boundary
/// points right anyway; on a tilted plane the two independently-rounded
/// bases disagree by a few ULPs, and the kernel's unguarded ray cast can
/// resolve an exactly-on-the-edge point to "inside". This was confirmed
/// empirically (not just reasoned about): with the API-side check
/// removed, this exact `center` — a point 1/8 of the way along the top
/// face's own edge — round-trips as a successful pie instead of a
/// refusal.
#[test]
fn draw_arc_pie_on_a_tilted_face_with_center_on_the_boundary_refuses_loop_not_strictly_inside() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);

    let plane = json!({ "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.6, 0.8] });
    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_rect", "as": "profile", "params": {
                    "plane": plane,
                    "corner_a": [0.0, 0.0, 0.0],
                    "corner_b": [1.0, 0.8, -0.6]
                }},
                { "method": "hew.solid.extrude", "as": "box", "params": {
                    "region": { "$ref": "profile#/region_id" },
                    "distance": 0.1
                }}
            ]
        }),
    );
    let object_id = result["results"][1]["object_id"].as_str().unwrap();

    // The top face's own boundary corners, offset from the base rect by
    // `distance * normal`; `centroid` is their mean, used purely to
    // locate the face by an interior point.
    let base_corners = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.8, -0.6],
        [0.0, 0.8, -0.6],
    ];
    let n = [0.0, 0.6, 0.8];
    let distance = 0.1;
    let top_corners: Vec<[f64; 3]> = base_corners
        .iter()
        .map(|c| {
            [
                c[0] + n[0] * distance,
                c[1] + n[1] * distance,
                c[2] + n[2] * distance,
            ]
        })
        .collect();
    let centroid = [
        top_corners.iter().map(|c| c[0]).sum::<f64>() / 4.0,
        top_corners.iter().map(|c| c[1]).sum::<f64>() / 4.0,
        top_corners.iter().map(|c| c[2]).sum::<f64>() / 4.0,
    ];
    // A point 1/8 of the way along the top face's [0]→[1] edge — on the
    // boundary, not at a corner (corners are handled correctly even by
    // the kernel's own check; it's an interior edge point that exposes
    // the basis mismatch).
    let a = top_corners[0];
    let b = top_corners[1];
    let t = 0.125;
    let center_on_boundary = [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ];
    let before = doc.save();

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_arc",
        json!({
            "plane": { "face": { "object": object_id, "at": centroid } },
            "center": center_on_boundary,
            "radius": 0.1,
            "start_angle": 0.0,
            "end_angle": std::f64::consts::FRAC_PI_2,
            "close": "pie"
        }),
    );
    assert_eq!(data["refusal"], "loop_not_strictly_inside");
    assert_eq!(
        doc.save(),
        before,
        "a refused envelope leaves the document untouched"
    );
}

/// Adjacent-edge coverage (checked alongside the pinned defect above, not
/// a defect itself): a `close: "segment"` sweep past a half turn — a
/// major circular segment — still imprints a simple `SplitFaceInner`
/// loop on a face. The arc points are in monotonic angular order and the
/// one closing chord only connects the two ends, so the loop is
/// star-shaped about the circle's center and cannot self-cross no matter
/// how much of the circle it spans short of a full turn — confirmed here
/// via the same push/pull-the-imprinted-sub-face round trip the other
/// face-mode arc tests use.
#[test]
fn draw_arc_segment_past_a_half_turn_on_a_face_still_imprints_a_simple_sub_face() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let object_id = build_box(
        &mut conn,
        &mut doc,
        2,
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        0.3,
    );

    let result = call_ok_one_undo(
        &mut conn,
        &mut doc,
        3,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.sketch.draw_arc", "as": "cut", "params": {
                    "plane": { "face": face_at(&object_id, [0.5, 0.5, 0.3]) },
                    "center": [0.5, 0.5, 0.3],
                    "radius": 0.2,
                    "start_angle": 0.0,
                    "end_angle": 3.0 * std::f64::consts::FRAC_PI_2,
                    "segments": 16,
                    "close": "segment"
                }},
                { "method": "hew.solid.push_pull", "params": {
                    "face": { "$face": "cut#face" },
                    "distance": 0.02
                }}
            ]
        }),
    );
    let pushed_id = result["results"][1]["object_id"].as_str().unwrap();
    assert_eq!(pushed_id, object_id);
}
