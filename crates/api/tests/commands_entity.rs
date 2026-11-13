//! Conformance coverage for `hew.entity.*` (docs/design/
//! api-implementation-conventions.md's testing bar): a success path and a
//! failure path per command, a rotate-90° correctness assertion, a
//! move-with-copy array, and the one-undo-entry / byte-identical-undo
//! property for every mutating envelope in this namespace.

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response, codes};
use kernel::{CurveGeom, Document, ObjectId, Plane, Point3, SketchId};
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

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
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

fn hello_attach(conn: &mut Connection, doc: &mut Document) {
    let DispatchOutcome::Reply(r) = conn.dispatch(
        &REGISTRY,
        doc,
        &mut NoHost,
        req(0, "hew.meta.hello", json!({ "protocol": 1 })),
    ) else {
        panic!("hello replies")
    };
    assert!(r.error.is_none(), "hello failed: {:?}", r.error);
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
/// (docs/agents/HEW_API.md §6.1, §14).
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
    let mut conn = Connection::new(Grant::core(), "test");
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
fn rename_renames_a_material_and_refuses_a_null_name() {
    let mut doc = Document::new();
    let mat = doc.add_material(kernel::Material::solid(
        "Oak",
        kernel::Rgba8::rgb(180, 140, 90),
    ));
    let public = public_of(&doc, &kernel::EntityRef::Material(mat));
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": public, "name": "Brick" }),
    );
    assert_eq!(doc.material(mat).unwrap().name, "Brick");
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    // A material's name is a required kernel `String`, never `Option` —
    // there is no "unnamed swatch" state to clear to, so `name: null` is
    // a params error rather than a kernel refusal.
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.entity.rename",
        json!({ "id": public }),
    );
    assert_eq!(r.error.as_ref().unwrap().code, codes::INVALID_PARAMS);
}

#[test]
fn rename_refuses_kinds_the_kernel_cannot_rename() {
    let mut doc = Document::new();
    let sketch = build_circle_sketch(&mut doc, 5.0, 5.0, 1.0);
    let public = public_of(&doc, &kernel::EntityRef::Sketch(sketch));
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
fn delete_deletes_a_material_unpainting_a_face_and_refuses_a_deleted_handle() {
    let mut doc = Document::new();
    let mat = doc.add_material(kernel::Material::solid(
        "Oak",
        kernel::Rgba8::rgb(180, 140, 90),
    ));
    let obj = build_box(&mut doc, 0.0);
    doc.set_object_material(obj, Some(mat)).expect("base");
    let public = public_of(&doc, &kernel::EntityRef::Material(mat));
    let mut conn = Connection::new(Grant::core(), "test");
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
    assert!(doc.material(mat).is_none(), "tombstoned");
    assert_eq!(
        doc.object(obj).unwrap().default_material(),
        None,
        "the object's base material was unpainted"
    );

    // A second delete of the now-deleted handle refuses (its public id
    // still resolves — the sid is a tombstone, not erased — but the
    // kernel's own `UnknownMaterial` surfaces). Checked BEFORE the
    // undo-restores-byte-identically assertion below, which undoes the
    // first delete and would make a second one succeed again.
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.entity.delete",
        json!({ "id": public }),
    );
    assert_eq!(data["refusal"], "unknown_material");

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn delete_deletes_a_component_definition_and_its_world_instance() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let (component, instance, _) = doc
        .make_component(&[kernel::NodeId::Object(obj)])
        .expect("fold");
    let public = public_of(&doc, &kernel::EntityRef::Component(component));
    let mut conn = Connection::new(Grant::core(), "test");
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
    assert!(!doc.component_ids().contains(&component));
    assert!(!doc.instance_ids().contains(&instance));
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn delete_refuses_a_definition_nested_inside_another_live_definition() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let (inner, inner_inst, _) = doc
        .make_component(&[kernel::NodeId::Object(obj)])
        .expect("fold inner");
    doc.make_component(&[kernel::NodeId::Instance(inner_inst)])
        .expect("fold outer, nesting the inner instance");
    let public = public_of(&doc, &kernel::EntityRef::Component(inner));
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.delete",
        json!({ "id": public }),
    );
    assert_eq!(data["refusal"], "definition_nested_in_definition");
    assert!(doc.component_ids().contains(&inner), "untouched");
    assert_eq!(doc.undo_depth(), depth_before, "document untouched");
}

#[test]
fn delete_adds_one_undo_entry_and_undoes_cleanly() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let public = public_of(&doc, &kernel::EntityRef::Object(obj));
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
            "copy": { "count": 3, "linked": false }
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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
            "copy": { "linked": false }
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
    let mut conn = Connection::new(Grant::core(), "test");
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
    let mut conn = Connection::new(Grant::core(), "test");
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

// ------------------------------------------- transforms through a path

/// A two-box component (`a` at x ∈ [0,1], `b` at x ∈ [2,3]) placed plainly
/// (`i1`) and mirrored across x into x ∈ [17, 20] (`i2`), a second
/// one-box component (`j1`), and a world box `w` at x = 30.
struct PathFixture {
    doc: Document,
    a: ObjectId,
    b: ObjectId,
    i1: kernel::InstanceId,
    i2: kernel::InstanceId,
    j1: kernel::InstanceId,
    j_member: ObjectId,
    w: ObjectId,
}

fn path_fixture() -> PathFixture {
    use kernel::{NodeId, Transform, Vec3};
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 2.0);
    let (comp, i1, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("component");
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(20.0, 0.0, 0.0)));
    let (i2, _) = doc
        .place_instance(comp, mirror)
        .expect("mirrored placement");
    let j_member = build_box(&mut doc, 10.0);
    let (_, j1, _) = doc
        .make_component(&[NodeId::Object(j_member)])
        .expect("second component");
    let w = build_box(&mut doc, 30.0);
    PathFixture {
        doc,
        a,
        b,
        i1,
        i2,
        j1,
        j_member,
        w,
    }
}

fn obj_id(doc: &Document, o: ObjectId) -> String {
    public_of(doc, &kernel::EntityRef::Object(o))
}

fn ins_id(doc: &Document, i: kernel::InstanceId) -> String {
    public_of(doc, &kernel::EntityRef::Instance(i))
}

/// The world minimum corner of member `o` under the placement `path`.
fn world_min(doc: &Document, path: &[kernel::InstanceId], o: ObjectId) -> Point3 {
    let pose = doc.path_pose(path).expect("path resolves");
    let mut lo = Point3::new(f64::MAX, f64::MAX, f64::MAX);
    for v in vertex_positions(doc, o) {
        let q = pose.apply_point(v);
        lo = Point3::new(lo.x.min(q.x), lo.y.min(q.y), lo.z.min(q.z));
    }
    lo
}

fn call_params_err(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) {
    let r = call(conn, doc, 90, method, params);
    let err = r.error.expect("a params error");
    assert_eq!(err.code, codes::INVALID_PARAMS, "{method}: {}", err.message);
}

#[test]
fn move_by_path_moves_the_member_in_world_terms_under_every_placement() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let (i1, i2) = (f.i1, f.i2);
    let mirrored = world_min(&f.doc, &[i2], f.a);
    let plain = world_min(&f.doc, &[i1], f.a);
    let depth = f.doc.undo_depth();
    let bytes = f.doc.save();
    let path = json!({ "path": [ins_id(&f.doc, i2), obj_id(&f.doc, f.a)] });
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.entity.move",
        json!({ "ids": [path], "translation": [0.1, 0.0, 0.0] }),
    );
    assert_eq!(result, json!({}));
    let moved = world_min(&f.doc, &[i2], f.a);
    assert!(moved.approx_eq(Point3::new(mirrored.x + 0.1, 0.0, 0.0), 1e-9));
    // The plain placement shows the mirror image of the move.
    let other = world_min(&f.doc, &[i1], f.a);
    assert!(other.approx_eq(Point3::new(plain.x - 0.1, 0.0, 0.0), 1e-9));
    assert_one_undo_and_clean_undo(&mut f.doc, depth, &bytes);
}

#[test]
fn transform_ids_take_one_scope_per_call() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let (i1, i2, j1) = (
        ins_id(&f.doc, f.i1),
        ins_id(&f.doc, f.i2),
        ins_id(&f.doc, f.j1),
    );
    let (a, b, jm, w) = (
        obj_id(&f.doc, f.a),
        obj_id(&f.doc, f.b),
        obj_id(&f.doc, f.j_member),
        obj_id(&f.doc, f.w),
    );
    let bytes = f.doc.save();
    let mv = |ids: Value| json!({ "ids": ids, "translation": [1.0, 0.0, 0.0] });
    // Different definitions; different paths to one definition; a mix.
    call_params_err(
        &mut conn,
        &mut f.doc,
        "hew.entity.move",
        mv(json!([{ "path": [i1, a] }, { "path": [j1, jm] }])),
    );
    call_params_err(
        &mut conn,
        &mut f.doc,
        "hew.entity.rotate",
        json!({ "ids": [{ "path": [i1, a] }, { "path": [i2, b] }], "pivot": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": 0.5 }),
    );
    call_params_err(
        &mut conn,
        &mut f.doc,
        "hew.entity.scale",
        json!({ "ids": [w, { "path": [i1, a] }], "anchor": [0.0, 0.0, 0.0], "factors": [2.0, 1.0, 1.0] }),
    );
    // A path that leads nowhere.
    let data = call_err(
        &mut conn,
        &mut f.doc,
        3,
        "hew.entity.move",
        mv(json!([{ "path": [j1, a] }])),
    );
    assert_eq!(data["refusal"], "unknown_occurrence");
    assert_eq!(f.doc.save(), bytes, "refusals leave the document untouched");
    // Two members by the same path: one call, one entry.
    let depth = f.doc.undo_depth();
    call_ok(
        &mut conn,
        &mut f.doc,
        4,
        "hew.entity.move",
        mv(json!([{ "path": [i1, a] }, { "path": [i1, b] }])),
    );
    assert_eq!(f.doc.undo_depth(), depth + 1);
    assert!(has_vertex_near(
        &vertex_positions(&f.doc, f.b),
        Point3::new(3.0, 0.0, 0.0)
    ));
    // A path whose leaf is a world node is that node's id, and mixes with ids.
    call_ok(
        &mut conn,
        &mut f.doc,
        5,
        "hew.entity.move",
        mv(json!([{ "path": [w.clone()] }])),
    );
    assert!(has_vertex_near(
        &vertex_positions(&f.doc, f.w),
        Point3::new(31.0, 0.0, 0.0)
    ));
}

#[test]
fn rotate_and_scale_by_path_conjugate_through_the_placement() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let path = json!({ "path": [ins_id(&f.doc, f.i2), obj_id(&f.doc, f.a)] });
    // Under the mirror, `a` spans world x ∈ [19, 20], y ∈ [0, 1]. A world
    // quarter turn about z through world (20, 0, 0) swings it to
    // x ∈ [19, 20], y ∈ [-1, 0]...
    call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.entity.rotate",
        json!({ "ids": [path.clone()], "pivot": [20.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": std::f64::consts::FRAC_PI_2 }),
    );
    let lo = world_min(&f.doc, &[f.i2], f.a);
    assert!(lo.approx_eq(Point3::new(19.0, -1.0, 0.0), 1e-9), "{lo:?}");
    // ...and a world scale of 2 along x about world (20, 0, 0) doubles it
    // along x away from there: x ∈ [18, 20].
    call_ok(
        &mut conn,
        &mut f.doc,
        3,
        "hew.entity.scale",
        json!({ "ids": [path], "anchor": [20.0, 0.0, 0.0], "factors": [2.0, 1.0, 1.0] }),
    );
    let mut hi = Point3::new(f64::MIN, f64::MIN, f64::MIN);
    let pose = f.doc.path_pose(&[f.i2]).unwrap();
    for v in vertex_positions(&f.doc, f.a) {
        let q = pose.apply_point(v);
        hi = Point3::new(hi.x.max(q.x), hi.y.max(q.y), hi.z.max(q.z));
    }
    assert!(hi.approx_eq(Point3::new(20.0, 0.0, 0.5), 1e-9), "{hi:?}");
    let lo = world_min(&f.doc, &[f.i2], f.a);
    assert!(lo.approx_eq(Point3::new(18.0, -1.0, 0.0), 1e-9), "{lo:?}");
}

#[test]
fn copy_by_path_lands_in_the_definition_and_refuses_while_it_is_open() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let path = json!({ "path": [ins_id(&f.doc, f.i2), obj_id(&f.doc, f.a)] });
    let comp = f.doc.instance_def(f.i1).unwrap();
    let before = f.doc.def_member_nodes(comp).unwrap().len();
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.entity.move",
        json!({ "ids": [path.clone()], "translation": [0.0, 2.0, 0.0], "copy": { "count": 2, "linked": false } }),
    );
    let ids = result["ids"].as_array().expect("copies listed");
    assert_eq!(ids.len(), 2);
    assert_eq!(f.doc.def_member_nodes(comp).unwrap().len(), before + 2);
    // A grid through the path, too.
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        3,
        "hew.entity.move",
        json!({ "ids": [path.clone()], "copy": { "grid": [{ "count": 2, "step": [0.0, 0.0, 2.0] }], "linked": false } }),
    );
    assert_eq!(result["ids"].as_array().unwrap().len(), 1);

    f.doc
        .open_explode_session(f.i1)
        .expect("open the definition");
    let data = call_err(
        &mut conn,
        &mut f.doc,
        4,
        "hew.entity.move",
        json!({ "ids": [path], "translation": [1.0, 0.0, 0.0] }),
    );
    assert_eq!(data["refusal"], "explode_session_scope");
}

// ------------------------------------------------ hew.solid.* by path

#[test]
fn subtract_by_path_cuts_every_placement_and_union_with_a_world_b_refuses() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    // A world cutter over the mirrored image of `a` (world x ∈ [19, 20]):
    // x ∈ [19.5, 20.5].
    let tool = build_box(&mut f.doc, 19.5);
    let tool_id = obj_id(&f.doc, tool);
    let a_path = json!({ "path": [ins_id(&f.doc, f.i2), obj_id(&f.doc, f.a)] });
    // Joining a member with world material would split it from its other
    // placements: the bus asks first (`unlink: true`).
    let data = call_err(
        &mut conn,
        &mut f.doc,
        80,
        "hew.solid.union",
        json!({ "a": a_path.clone(), "b": tool_id.clone() }),
    );
    assert_eq!(data["refusal"], "would_unlink");
    call_params_err(
        &mut conn,
        &mut f.doc,
        "hew.solid.subtract",
        json!({ "a": tool_id.clone(), "b": a_path.clone() }),
    );
    let depth = f.doc.undo_depth();
    let bytes = f.doc.save();
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.solid.subtract",
        json!({ "a": a_path, "b": tool_id }),
    );
    let comp = f.doc.instance_def(f.i1).unwrap();
    let cut = f
        .doc
        .def_members(comp)
        .unwrap()
        .into_iter()
        .find(|&o| obj_id(&f.doc, o) == result["result"].as_str().unwrap())
        .expect("the result is a member");
    // In the definition's frame the cutter removed local x ∈ [0, 0.5].
    assert!(has_vertex_near(
        &vertex_positions(&f.doc, cut),
        Point3::new(0.5, 0.0, 0.0)
    ));
    assert!(!f.doc.is_world_object(tool), "the cutter is consumed");
    assert_one_undo_and_clean_undo(&mut f.doc, depth, &bytes);

    // Two members by the same path combine inside the definition.
    let i1 = ins_id(&f.doc, f.i1);
    let (a, b) = (obj_id(&f.doc, f.a), obj_id(&f.doc, f.b));
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        3,
        "hew.solid.union",
        json!({ "a": { "path": [i1.clone(), a] }, "b": { "path": [i1, b] } }),
    );
    assert!(result["result"].is_string());
}

#[test]
fn slice_and_push_pull_by_path_edit_the_definition() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let i2 = ins_id(&f.doc, f.i2);
    let a = obj_id(&f.doc, f.a);
    // World plane x = 19.5 under the mirror is local x = 0.5.
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.solid.slice",
        json!({ "object": { "path": [i2.clone(), a] }, "plane": { "origin": [19.5, 0.0, 0.0], "normal": [1.0, 0.0, 0.0] } }),
    );
    assert!(result["positive"].is_string() && result["negative"].is_string());
    let comp = f.doc.instance_def(f.i1).unwrap();
    let live = f
        .doc
        .def_members(comp)
        .unwrap()
        .into_iter()
        .filter(|&o| f.doc.object(o).is_some())
        .count();
    assert_eq!(live, 3, "b and two pieces");

    // Push the top of `b` up through the path; `at` is in b's own frame.
    let b = obj_id(&f.doc, f.b);
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        3,
        "hew.solid.push_pull",
        json!({ "face": { "object": { "path": [i2, b] }, "at": [2.5, 0.5, 0.5] }, "distance": 0.25 }),
    );
    assert_eq!(result["object_id"], obj_id(&f.doc, f.b));
    assert!(has_vertex_near(
        &vertex_positions(&f.doc, f.b),
        Point3::new(2.0, 0.0, 0.75)
    ));
}

// ------------------------- paths whose pose is not its own inverse

/// A two-box component (`a` at x ∈ [0,1], `b` at x ∈ [2,3]) placed once
/// with a quarter turn about z plus a shift (`t1`), and once (`t2`, another
/// quarter turn and shift) inside an outer component whose own placement
/// `o1` shifts it again, so the nested path `[o1, t2]` composes two poses.
/// Neither path's pose is its own inverse (unlike a mirror, or two quarter
/// turns), so a conjugation taken in the wrong order, or a pose used where
/// its inverse belongs, lands the edit somewhere else.
struct TurnedFixture {
    doc: Document,
    comp: kernel::ComponentId,
    a: ObjectId,
    t1: kernel::InstanceId,
    t2: kernel::InstanceId,
    o1: kernel::InstanceId,
}

fn turned_fixture() -> TurnedFixture {
    use kernel::{NodeId, Transform, Vec3};
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 2.0);
    let (comp, base, _) = doc
        .make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("component");
    let turn = |dx: f64, dy: f64| {
        Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
            .expect("rotation")
            .then(&Transform::translation(Vec3::new(dx, dy, 0.0)))
    };
    doc.transform_instance(base, &turn(0.0, 10.0))
        .expect("turn the first placement");
    let (t2, _) = doc.place_instance(comp, turn(3.0, 0.0)).expect("second");
    let (_outer, o1, _) = doc
        .make_component(&[NodeId::Instance(t2)])
        .expect("outer component");
    doc.transform_instance(o1, &Transform::translation(Vec3::new(40.0, 5.0, 0.0)))
        .expect("shift the outer placement");
    TurnedFixture {
        doc,
        comp,
        a,
        t1: base,
        t2,
        o1,
    }
}

/// Member `o`'s vertices in world terms under the placement `path`.
fn world_vertices(doc: &Document, path: &[kernel::InstanceId], o: ObjectId) -> Vec<Point3> {
    let pose = doc.path_pose(path).expect("path resolves");
    vertex_positions(doc, o)
        .into_iter()
        .map(|v| pose.apply_point(v))
        .collect()
}

fn assert_same_points(got: &[Point3], want: &[Point3], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for &w in want {
        assert!(
            has_vertex_near(got, w),
            "{what}: no vertex near {w:?} in {got:?}"
        );
    }
}

#[test]
fn transforms_through_a_turned_or_nested_path_land_where_asked_in_world_terms() {
    use kernel::{Transform, Vec3};
    let mut f = turned_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let paths: [Vec<kernel::InstanceId>; 2] = [vec![f.t1], vec![f.o1, f.t2]];
    let mut id = 10;
    for path in &paths {
        let mut locator: Vec<Value> = path.iter().map(|&i| json!(ins_id(&f.doc, i))).collect();
        locator.push(json!(obj_id(&f.doc, f.a)));
        let loc = json!({ "path": locator });

        // A world move of +0.25 along x.
        let before = world_vertices(&f.doc, path, f.a);
        id += 1;
        call_ok(
            &mut conn,
            &mut f.doc,
            id,
            "hew.entity.move",
            json!({ "ids": [loc.clone()], "translation": [0.25, 0.0, 0.0] }),
        );
        let shift = Transform::translation(Vec3::new(0.25, 0.0, 0.0));
        let want: Vec<Point3> = before.iter().map(|&p| shift.apply_point(p)).collect();
        assert_same_points(&world_vertices(&f.doc, path, f.a), &want, "move");

        // A world quarter turn about z through world (1, 2, 0).
        let before = world_vertices(&f.doc, path, f.a);
        id += 1;
        call_ok(
            &mut conn,
            &mut f.doc,
            id,
            "hew.entity.rotate",
            json!({ "ids": [loc.clone()], "pivot": [1.0, 2.0, 0.0], "axis": [0.0, 0.0, 1.0], "angle": std::f64::consts::FRAC_PI_2 }),
        );
        let pivot = Vec3::new(1.0, 2.0, 0.0);
        let spin = Transform::translation(-pivot)
            .then(
                &Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
                    .unwrap(),
            )
            .then(&Transform::translation(pivot));
        let want: Vec<Point3> = before.iter().map(|&p| spin.apply_point(p)).collect();
        assert_same_points(&world_vertices(&f.doc, path, f.a), &want, "rotate");

        // A world stretch of 2 along x about world (0, 0, 0).
        let before = world_vertices(&f.doc, path, f.a);
        id += 1;
        call_ok(
            &mut conn,
            &mut f.doc,
            id,
            "hew.entity.scale",
            json!({ "ids": [loc.clone()], "anchor": [0.0, 0.0, 0.0], "factors": [2.0, 1.0, 1.0] }),
        );
        let stretch = Transform::scale(Vec3::new(2.0, 1.0, 1.0));
        let want: Vec<Point3> = before.iter().map(|&p| stretch.apply_point(p)).collect();
        assert_same_points(&world_vertices(&f.doc, path, f.a), &want, "scale");

        // A grid copy one world step of +3 along x: the copy is a new
        // member whose world image is the original's, shifted.
        let before = world_vertices(&f.doc, path, f.a);
        let members = f.doc.def_members(f.comp).unwrap();
        id += 1;
        call_ok(
            &mut conn,
            &mut f.doc,
            id,
            "hew.entity.move",
            json!({ "ids": [loc], "copy": { "grid": [{ "count": 2, "step": [3.0, 0.0, 0.0] }], "linked": false } }),
        );
        let fresh: Vec<ObjectId> = f
            .doc
            .def_members(f.comp)
            .unwrap()
            .into_iter()
            .filter(|o| !members.contains(o))
            .collect();
        assert_eq!(fresh.len(), 1, "one grid copy");
        let step = Transform::translation(Vec3::new(3.0, 0.0, 0.0));
        let want: Vec<Point3> = before.iter().map(|&p| step.apply_point(p)).collect();
        assert_same_points(&world_vertices(&f.doc, path, fresh[0]), &want, "grid copy");
    }
}

#[test]
fn subtract_through_a_nested_turned_path_cuts_where_the_cutter_sits() {
    use kernel::{NodeId, Transform, Vec3};
    let mut f = turned_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    let path = [f.o1, f.t2];
    // The path maps local (x, y) to world (43 - y, 5 + x), so `a` (local
    // x, y ∈ [0, 1]) shows at world x ∈ [42, 43], y ∈ [5, 6].
    let pose = f.doc.path_pose(&path).unwrap();
    assert!(
        pose.apply_point(Point3::new(1.0, 0.0, 0.0))
            .approx_eq(Point3::new(43.0, 6.0, 0.0), 1e-9)
    );
    // A world cutter at x ∈ [42.25, 43.25], y ∈ [4.5, 5.5] covers local
    // x ∈ [0, 0.5], y ∈ [0, 0.75] of `a`.
    let tool = build_box(&mut f.doc, 42.25);
    f.doc
        .transform_selection(
            &[NodeId::Object(tool)],
            &[],
            &Transform::translation(Vec3::new(0.0, 4.5, 0.0)),
        )
        .expect("place the cutter");
    let mut locator: Vec<Value> = path.iter().map(|&i| json!(ins_id(&f.doc, i))).collect();
    locator.push(json!(obj_id(&f.doc, f.a)));
    let tool_id = obj_id(&f.doc, tool);
    let depth = f.doc.undo_depth();
    let bytes = f.doc.save();
    let result = call_ok(
        &mut conn,
        &mut f.doc,
        2,
        "hew.solid.subtract",
        json!({ "a": { "path": locator }, "b": tool_id }),
    );
    let cut = f
        .doc
        .def_members(f.comp)
        .unwrap()
        .into_iter()
        .find(|&o| obj_id(&f.doc, o) == result["result"].as_str().unwrap())
        .expect("the result is a member");
    let verts = vertex_positions(&f.doc, cut);
    assert!(
        has_vertex_near(&verts, Point3::new(0.5, 0.75, 0.0)),
        "the notch corner sits at local (0.5, 0.75): {verts:?}"
    );
    assert!(has_vertex_near(&verts, Point3::new(1.0, 0.0, 0.0)));
    assert!(!has_vertex_near(&verts, Point3::new(0.0, 0.0, 0.0)));
    assert_one_undo_and_clean_undo(&mut f.doc, depth, &bytes);
}

// --------------------------- member-b booleans through a path: gates

#[test]
fn a_member_boolean_by_path_refuses_while_any_session_is_open() {
    let mut f = path_fixture();
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut f.doc);
    // The second component, not the one the path reaches, is open.
    f.doc
        .open_explode_session(f.j1)
        .expect("open another definition");
    let i1 = ins_id(&f.doc, f.i1);
    let (a, b) = (obj_id(&f.doc, f.a), obj_id(&f.doc, f.b));
    let bytes = f.doc.save();
    for (id, method) in [
        (2, "hew.solid.union"),
        (3, "hew.solid.subtract"),
        (4, "hew.solid.intersect"),
    ] {
        let data = call_err(
            &mut conn,
            &mut f.doc,
            id,
            method,
            json!({ "a": { "path": [i1.clone(), a.clone()] }, "b": { "path": [i1.clone(), b.clone()] } }),
        );
        assert_eq!(data["refusal"], "explode_session_scope", "{method}");
    }
    assert_eq!(f.doc.save(), bytes);
}

#[test]
fn a_member_boolean_by_path_refuses_a_grouped_operand() {
    use kernel::NodeId;
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let c = build_box(&mut doc, 5.0);
    let b = build_box(&mut doc, 0.5);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(c)])
        .expect("group");
    let (_comp, inst, _) = doc
        .make_component(&[NodeId::Group(g), NodeId::Object(b)])
        .expect("component");
    let mut conn = Connection::new(Grant::app(), "test");
    hello_attach(&mut conn, &mut doc);
    let i = ins_id(&doc, inst);
    let (a_id, b_id) = (obj_id(&doc, a), obj_id(&doc, b));
    let bytes = doc.save();
    // A path names instances and then the leaf; `a` sits in the member
    // group, which the path does not list.
    let grouped = json!({ "path": [i.clone(), a_id] });
    let plain = json!({ "path": [i, b_id] });
    for (id, (x, y)) in [(2, (&grouped, &plain)), (3, (&plain, &grouped))] {
        let data = call_err(
            &mut conn,
            &mut doc,
            id,
            "hew.solid.subtract",
            json!({ "a": x, "b": y }),
        );
        assert_eq!(data["refusal"], "grouped_operand");
    }
    assert_eq!(doc.save(), bytes);
}
