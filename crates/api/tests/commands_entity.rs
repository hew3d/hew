//! Conformance coverage for `hew.entity.*` (docs/design/
//! api-implementation-conventions.md's testing bar): a success path and a
//! failure path per command, a rotate-90° correctness assertion, a
//! move-with-copy array, and the one-undo-entry / byte-identical-undo
//! property for every mutating envelope in this namespace.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::{CurveGeom, Document, ObjectId, Plane, Point3, SketchId};
use serde_json::{Value, json};

// --------------------------------------------------------------- fixtures

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A unit-square box `[x0, x0+1] x [0, 1] x [0, 0.5]`.
fn build_box(doc: &mut Document, x0: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let (y0, x1, y1) = (0.0, x0 + 1.0, 1.0);
        let corners = [
            (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
            (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
            (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
            (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
        ];
        for (a, b) in corners {
            sk.add_segment(a, b).expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("live");
    doc.extrude_region(s, regions[0], 0.5)
        .expect("extrude box")
        .0
}

/// A loose (never-extruded) circle sketch — survives as a `move`/`rotate`
/// target so the sketch-selection rules can be exercised.
fn build_circle_sketch(doc: &mut Document, cx: f64, cy: f64, r: f64) -> SketchId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        sk.begin_curve_with(CurveGeom {
            center: Point3::new(cx, cy, 0.0),
            radius: r,
        })
        .expect("circle center is on-plane and non-degenerate");
        let segments = 12usize;
        let pts: Vec<Point3> = (0..segments)
            .map(|i| {
                let theta = std::f64::consts::TAU * (i as f64) / (segments as f64);
                Point3::new(cx + r * theta.cos(), cy + r * theta.sin(), 0.0)
            })
            .collect();
        for i in 0..segments {
            sk.add_segment(pts[i], pts[(i + 1) % segments])
                .expect("circle segment");
        }
        sk.end_curve();
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    s
}

fn public_of(doc: &Document, entity: &kernel::EntityRef) -> String {
    let sid = doc.sid_of(entity).expect("entity carries a stable id");
    api::ids::public_id(entity, sid)
}

// ------------------------------------------------------------- dispatch harness

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

/// Dispatches a command expected to succeed. Every `hew.entity.*` command
/// is model-mutating, so a plain request auto-wraps as a one-command
/// `hew.doc.transact` (dispatch.rs): the reply's `result` is therefore
/// `{"results": [<the command's own result>], "label": ...}`, not the
/// command's result directly — unwrap to the single entry.
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

/// Asserts the envelope just dispatched added exactly ONE undo entry, and
/// that undoing it restores `doc` to `bytes_before` byte-identically
/// (docs/HEW_API.md §6.1, §14).
fn assert_one_undo_and_clean_undo(doc: &mut Document, depth_before: usize, bytes_before: &[u8]) {
    assert_eq!(
        doc.undo_depth(),
        depth_before + 1,
        "envelope should add exactly one undo entry"
    );
    doc.undo().expect("undo restores the compound entry");
    assert_eq!(
        doc.save(),
        bytes_before,
        "undo did not restore byte-identical state"
    );
}

fn vertex_positions(doc: &Document, object: ObjectId) -> Vec<Point3> {
    doc.object(object)
        .expect("object is live")
        .vertices()
        .values()
        .map(|v| v.position)
        .collect()
}

fn has_vertex_near(positions: &[Point3], p: Point3) -> bool {
    positions.iter().any(|v| v.approx_eq(p, 1e-9))
}

// ------------------------------------------------------------- hew.entity.rename

#[test]
fn rename_renames_an_object_and_refuses_an_unknown_id() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": public, "name": "Leg" }),
    );
    assert_eq!(doc.object_name(obj), Some("Leg"));

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.entity.rename",
        json!({ "id": "obj_ffffff", "name": "X" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn rename_refuses_kinds_the_kernel_cannot_rename() {
    let mut doc = Document::new();
    let sketch = build_circle_sketch(&mut doc, 5.0, 5.0, 1.0);
    let public = public_of(&doc, &kernel::EntityRef::Sketch(sketch));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": public, "name": "X" }),
    );
    assert_eq!(data["refusal"], "rename_unsupported");
}

#[test]
fn rename_adds_one_undo_entry_and_undoes_cleanly() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": public, "name": "Leg" }),
    );
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

// ------------------------------------------------------------- hew.entity.delete

#[test]
fn delete_deletes_an_object_and_refuses_an_unknown_id() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.delete",
        json!({ "id": public }),
    );
    assert!(doc.object(obj).is_none() || !doc.visible_object_ids().contains(&obj));

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.entity.delete",
        json!({ "id": "obj_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn delete_refuses_materials_and_components_as_unsupported() {
    let mut doc = Document::new();
    let mat = doc.add_material(kernel::Material::solid(
        "Oak",
        kernel::Rgba8::rgb(180, 140, 90),
    ));
    let public = public_of(&doc, &kernel::EntityRef::Material(mat));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.delete",
        json!({ "id": public }),
    );
    assert_eq!(data["refusal"], "delete_unsupported");
}

#[test]
fn delete_adds_one_undo_entry_and_undoes_cleanly() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.delete",
        json!({ "id": public }),
    );
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

// --------------------------------------------------------------- hew.entity.move

#[test]
fn move_translates_by_a_vector_with_one_undo_entry() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.move",
        json!({ "ids": [public], "translation": [2.0, 0.0, 0.0] }),
    );
    let positions = vertex_positions(&doc, obj);
    assert!(has_vertex_near(&positions, Point3::new(2.0, 0.0, 0.0)));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn move_translates_by_from_to_points() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.move",
        json!({
            "ids": [public],
            "from": [0.0, 0.0, 0.0],
            "to": [1.0, 1.0, 0.0]
        }),
    );
    let positions = vertex_positions(&doc, obj);
    assert!(has_vertex_near(&positions, Point3::new(1.0, 1.0, 0.0)));
}

#[test]
fn move_with_copy_count_three_creates_an_array_in_one_undo_entry() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.move",
        json!({
            "ids": [public],
            "translation": [1.0, 0.0, 0.0],
            "copy": { "count": 3 }
        }),
    );
    let ids = result["ids"].as_array().expect("ids array");
    assert_eq!(ids.len(), 3, "count=3 makes three copies");
    // The original is untouched by a copy; each copy sits at k * step.
    let original_positions = vertex_positions(&doc, obj);
    assert!(has_vertex_near(
        &original_positions,
        Point3::new(0.0, 0.0, 0.0)
    ));

    for (k, id) in ids.iter().enumerate() {
        let public_copy = id.as_str().expect("id is a string");
        let entity = api::ids::IdResolver::new(&doc)
            .resolve(public_copy)
            .expect("copy resolves");
        let kernel::EntityRef::Object(copy_obj) = entity else {
            panic!("copy is an object")
        };
        let step = (k + 1) as f64;
        let positions = vertex_positions(&doc, copy_obj);
        assert!(
            has_vertex_near(&positions, Point3::new(step, 0.0, 0.0)),
            "copy {k} should sit at step {step}"
        );
    }

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn move_refuses_a_mixed_sketch_and_node_selection() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let sketch = build_circle_sketch(&mut doc, 5.0, 5.0, 1.0);
    let obj_public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let sketch_public = public_of(&doc, &kernel::EntityRef::Sketch(sketch));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.move",
        json!({ "ids": [obj_public, sketch_public], "translation": [1.0, 0.0, 0.0] }),
    );
    assert_eq!(data["refusal"], "mixed_selection_unsupported");
}

#[test]
fn move_refuses_copying_a_sketch() {
    let mut doc = Document::new();
    let sketch = build_circle_sketch(&mut doc, 5.0, 5.0, 1.0);
    let sketch_public = public_of(&doc, &kernel::EntityRef::Sketch(sketch));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.move",
        json!({
            "ids": [sketch_public],
            "translation": [1.0, 0.0, 0.0],
            "copy": { "count": 1 }
        }),
    );
    assert_eq!(data["refusal"], "sketch_copy_unsupported");
}

// ------------------------------------------------------------- hew.entity.rotate

/// A 90° rotation of a known box about a pivot NOT at the box's own
/// center: distinguishes a correct rotation from a no-op or a
/// wrong-direction rotation (both of which would leave the corner SET
/// unchanged for a pivot at the box's center — DEVELOPMENT.md's "write a
/// test that would catch the bug" posture).
///
/// Pivot at the origin, axis +Z, angle +90°: `Transform::rotation`'s
/// right-handed Rodrigues formula maps `(x, y, z) -> (-y, x, z)` about
/// the origin, so the box corner `(1, 1, z)` (diagonal from the pivot)
/// must land at `(-1, 1, z)`, and the pivot-adjacent corner `(0, 0, z)`
/// stays fixed.
///
/// Checks BOTH halves of the §14 property: exactly one undo entry, and
/// undo restores `doc.save()` byte-for-byte. `Document::transform_selection`
/// (which `hew.entity.rotate` calls) used to bake a recomputed inverse
/// matrix into geometry for undo — not bit-exact for an irrational angle
/// like `FRAC_PI_2` — but now records the exact pre-rotation snapshot and
/// restores it verbatim (rule 9 posture; see
/// `crates/kernel/tests/transform_exact_specs.rs`), so this round-trips
/// byte-identically like every other transform.
#[test]
fn rotate_ninety_degrees_lands_where_expected() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rotate",
        json!({
            "ids": [public],
            "pivot": [0.0, 0.0, 0.0],
            "axis": [0.0, 0.0, 1.0],
            "angle": std::f64::consts::FRAC_PI_2
        }),
    );
    let positions = vertex_positions(&doc, obj);
    assert!(
        has_vertex_near(&positions, Point3::new(0.0, 0.0, 0.0)),
        "the pivot-adjacent corner is a fixed point of the rotation"
    );
    assert!(
        has_vertex_near(&positions, Point3::new(-1.0, 1.0, 0.0)),
        "the diagonal corner (1,1,0) must land at (-1,1,0)"
    );
    assert!(
        !has_vertex_near(&positions, Point3::new(1.0, 1.0, 0.0)),
        "the pre-rotation diagonal corner must have moved"
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn rotate_refuses_an_unknown_id() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rotate",
        json!({
            "ids": ["obj_ffffff"],
            "pivot": [0.0, 0.0, 0.0],
            "axis": [0.0, 0.0, 1.0],
            "angle": 1.0
        }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn rotate_with_copy_creates_a_rotated_duplicate() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rotate",
        json!({
            "ids": [public],
            "pivot": [0.0, 0.0, 0.0],
            "axis": [0.0, 0.0, 1.0],
            "angle": std::f64::consts::FRAC_PI_2,
            "copy": {}
        }),
    );
    let ids = result["ids"].as_array().expect("ids array");
    assert_eq!(
        ids.len(),
        1,
        "single id + default count=1 -> duplicate_node"
    );
    // The original stays where it was.
    let original_positions = vertex_positions(&doc, obj);
    assert!(has_vertex_near(
        &original_positions,
        Point3::new(1.0, 0.0, 0.0)
    ));
}

// -------------------------------------------------------------- hew.entity.scale

#[test]
fn scale_scales_about_an_anchor_with_one_undo_entry() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.scale",
        json!({
            "ids": [public],
            "anchor": [0.0, 0.0, 0.0],
            "factors": [2.0, 2.0, 1.0]
        }),
    );
    let positions = vertex_positions(&doc, obj);
    assert!(
        has_vertex_near(&positions, Point3::new(2.0, 2.0, 0.0)),
        "the corner farthest from the anchor doubles its distance"
    );
    assert!(
        has_vertex_near(&positions, Point3::new(0.0, 0.0, 0.0)),
        "the anchor-adjacent corner is a fixed point of the scale"
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn scale_refuses_non_positive_factors() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.scale",
        json!({
            "ids": [public],
            "anchor": [0.0, 0.0, 0.0],
            "factors": [0.0, 1.0, 1.0]
        }),
    );
    assert_eq!(
        r.error.unwrap().code,
        codes::INVALID_PARAMS,
        "a non-positive factor is a parameter defect, not a kernel refusal"
    );
}
