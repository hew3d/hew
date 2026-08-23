//! Conformance coverage for Wave C's remaining namespaces
//! (docs/design/api-implementation-conventions.md's testing bar):
//! `hew.group.*`, `hew.component.*`, `hew.context.*`, `hew.material.*`,
//! `hew.tag.*`, `hew.guide.*`, `hew.attr.*`, `hew.history.*`. A success
//! path and a failure path per command, one-undo-entry + byte-identical
//! undo per mutating envelope (except the not-undoable trio:
//! `hew.material.create`, `hew.tag.create`, `hew.tag.set_visible`), and
//! the context-balance / history-status pins the task calls out
//! specifically.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::{Document, EntityRef, Material, NodeId, Plane, Point3, Rgba8};
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

fn build_box(doc: &mut Document, x0: f64) -> kernel::ObjectId {
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

fn public_of(doc: &Document, entity: &EntityRef) -> String {
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

/// Dispatches a command expected to succeed. Model-mutating commands
/// auto-wrap a plain request as a one-command transaction (dispatch.rs),
/// so the reply's `result` is `{"results": [<own result>], "label": ...}`
/// — unwrap to the single entry. Read-only and solitary commands (
/// `hew.attr.get`, `hew.history.*`) run bare and need no unwrapping; the
/// `results`-array probe is a no-op for those.
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

// ================================================================ hew.group

#[test]
fn group_create_groups_nodes_and_refuses_an_unknown_member() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    let a_pub = public_of(&doc, &EntityRef::Object(a));
    let b_pub = public_of(&doc, &EntityRef::Object(b));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.group.create",
        json!({ "members": [a_pub, b_pub] }),
    );
    let group_public = result["group"].as_str().expect("group id").to_string();
    let entity = api::IdResolver::new(&doc)
        .resolve(&group_public)
        .expect("group resolves");
    assert!(matches!(entity, EntityRef::Group(_)));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.group.create",
        json!({ "members": [a_pub, "obj_ffffff"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn group_explode_dissolves_a_group_and_refuses_an_unknown_id() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (group, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let group_pub = public_of(&doc, &EntityRef::Group(group));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.group.explode",
        json!({ "id": group_pub }),
    );
    assert!(doc.node_parent(NodeId::Object(a)).is_none());

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.group.explode",
        json!({ "id": "grp_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

// ============================================================ hew.component

#[test]
fn component_create_folds_members_into_a_definition_plus_instance() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let a_pub = public_of(&doc, &EntityRef::Object(a));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.component.create",
        json!({ "members": [a_pub] }),
    );
    let component_pub = result["component"].as_str().expect("component id");
    let instance_pub = result["instance"].as_str().expect("instance id");
    let resolver = api::IdResolver::new(&doc);
    assert!(matches!(
        resolver.resolve(component_pub),
        Some(EntityRef::Component(_))
    ));
    assert!(matches!(
        resolver.resolve(instance_pub),
        Some(EntityRef::Instance(_))
    ));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.component.create",
        json!({ "members": ["obj_ffffff"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn component_place_places_an_instance_at_a_translation_pose() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (component, _, _) = doc.make_component(&[NodeId::Object(a)]).expect("fold");
    let component_pub = public_of(&doc, &EntityRef::Component(component));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.component.place",
        json!({ "component": component_pub, "pose": { "translation": [5.0, 0.0, 0.0] } }),
    );
    let instance_pub = result["instance"]
        .as_str()
        .expect("instance id")
        .to_string();
    let entity = api::IdResolver::new(&doc)
        .resolve(&instance_pub)
        .expect("instance resolves");
    let EntityRef::Instance(instance) = entity else {
        panic!("expected an instance")
    };
    let pose = doc.instance_pose(instance).expect("live instance");
    assert_eq!(
        pose.to_affine()[3],
        5.0,
        "translation x baked into the pose"
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.component.place",
        json!({ "component": "cmp_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn component_make_unique_deep_copies_the_definition() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (component, instance, _) = doc.make_component(&[NodeId::Object(a)]).expect("fold");
    let (instance2, _) = doc
        .place_instance(component, kernel::Transform::IDENTITY)
        .expect("second placement");
    let instance2_pub = public_of(&doc, &EntityRef::Instance(instance2));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.component.make_unique",
        json!({ "instance": instance2_pub }),
    );
    let new_component_pub = result["component"].as_str().expect("component id");
    let entity = api::IdResolver::new(&doc)
        .resolve(new_component_pub)
        .expect("resolves");
    let EntityRef::Component(new_component) = entity else {
        panic!("expected a component")
    };
    assert_ne!(
        new_component, component,
        "make_unique mints a private definition"
    );
    // The original instance still shares the original definition.
    assert_eq!(doc.instance_def(instance), Some(component));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.component.make_unique",
        json!({ "instance": "ins_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn component_explode_bakes_an_instance_into_world_objects() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (_, instance, _) = doc.make_component(&[NodeId::Object(a)]).expect("fold");
    let instance_pub = public_of(&doc, &EntityRef::Instance(instance));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.component.explode",
        json!({ "instance": instance_pub }),
    );
    let objects = result["objects"].as_array().expect("objects array");
    assert_eq!(objects.len(), 1);

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.component.explode",
        json!({ "instance": "ins_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

// ============================================================== hew.context

#[test]
fn context_enter_and_exit_balanced_inside_one_transaction_commits() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (group, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let group_pub = public_of(&doc, &EntityRef::Group(group));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    assert!(doc.session_stack().is_empty());

    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "commands": [
                { "method": "hew.context.enter", "params": { "id": group_pub } },
                { "method": "hew.context.exit", "params": {} }
            ]
        }),
    );
    assert!(
        r.error.is_none(),
        "balanced enter+exit commits: {:?}",
        r.error
    );
    assert!(
        doc.session_stack().is_empty(),
        "the transaction closed exactly the frame it opened"
    );
    assert_eq!(
        doc.undo_depth(),
        depth_before + 1,
        "the balanced enter+exit is still one envelope, one undo entry"
    );
}

/// docs/agents/HEW_API.md §6.3: context commands are legal only inside a
/// transaction. A BARE `hew.context.enter` is a model-mutating plain
/// request, which dispatch.rs auto-wraps as a one-command transaction —
/// which is then unbalanced (one `enter`, no `exit`) and rejected
/// `-32602` before anything runs. Pinning that the auto-wrap does NOT
/// accidentally let a bare context command through.
#[test]
fn bare_context_enter_is_rejected_as_a_protocol_error() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (group, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let group_pub = public_of(&doc, &EntityRef::Group(group));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.context.enter",
        json!({ "id": group_pub }),
    );
    assert_eq!(r.error.as_ref().unwrap().code, codes::INVALID_PARAMS);
    assert!(doc.session_stack().is_empty(), "nothing ran");
    assert_eq!(doc.undo_depth(), depth_before, "document untouched");
}

// ============================================================= hew.material

#[test]
fn material_create_succeeds_and_adds_no_undo_entry() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.material.create",
        json!({ "name": "Oak", "color": [180, 140, 90] }),
    );
    let material_pub = result["material"].as_str().expect("material id");
    let entity = api::IdResolver::new(&doc)
        .resolve(material_pub)
        .expect("resolves");
    assert!(matches!(entity, EntityRef::Material(_)));
    // `Document::add_material` is deliberately not undoable (docs/design/
    // api-kernel-map.md §1.9) — a palette addition is registry state, not
    // a modeled edit, so the envelope adds NO undo entry.
    assert_eq!(
        doc.undo_depth(),
        depth_before,
        "material.create must not add an undo entry"
    );
}

#[test]
fn material_paint_paints_a_face_and_refuses_an_unknown_material() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let face = doc
        .object(obj)
        .expect("live")
        .faces()
        .keys()
        .next()
        .expect("at least one face");
    let point_on_face: Point3 = {
        let object = doc.object(obj).expect("live");
        let f = &object.faces()[face];
        let ring: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
        // Centroid of the ring, a robust interior point for any convex face.
        let n = ring.len() as f64;
        let sum = ring.iter().fold(Point3::new(0.0, 0.0, 0.0), |s, p| {
            Point3::new(s.x + p.x, s.y + p.y, s.z + p.z)
        });
        Point3::new(sum.x / n, sum.y / n, sum.z / n)
    };
    let material = doc.add_material(Material::solid("Oak", Rgba8::rgb(180, 140, 90)));
    let material_pub = public_of(&doc, &EntityRef::Material(material));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.material.paint",
        json!({
            "face": { "object": obj_pub, "at": [point_on_face.x, point_on_face.y, point_on_face.z] },
            "material": material_pub
        }),
    );
    assert_eq!(doc.face_material(obj, face), Some(material));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.material.paint",
        json!({ "id": obj_pub, "material": "mat_ffffff" }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn material_set_default_sets_the_objects_base_material() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let material = doc.add_material(Material::solid("Oak", Rgba8::rgb(180, 140, 90)));
    let material_pub = public_of(&doc, &EntityRef::Material(material));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.material.set_default",
        json!({ "id": obj_pub, "material": material_pub }),
    );
    assert_eq!(doc.object(obj).unwrap().default_material(), Some(material));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.material.set_default",
        json!({ "id": "obj_ffffff", "material": null }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn material_set_opacity_sets_alpha_and_refuses_an_unknown_material() {
    let mut doc = Document::new();
    let material = doc.add_material(Material::solid("Oak", Rgba8::rgb(180, 140, 90)));
    let material_pub = public_of(&doc, &EntityRef::Material(material));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.material.set_opacity",
        json!({ "material": material_pub, "alpha": 128 }),
    );
    assert_eq!(doc.material(material).unwrap().color.a, 128);

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.material.set_opacity",
        json!({ "material": "mat_ffffff", "alpha": 128 }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

// ==================================================================== hew.tag

#[test]
fn tag_create_registers_a_path_and_adds_no_undo_entry() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.tag.create",
        json!({ "path": ["Structure"] }),
    );
    let tag_pub = result["tag"].as_str().expect("tag id");
    let entity = api::IdResolver::new(&doc)
        .resolve(tag_pub)
        .expect("resolves");
    assert!(matches!(entity, EntityRef::Tag(_)));
    assert!(doc.tag_meta().any(|(p, _)| p == ["Structure"]));
    // set_tag_hidden (§1.10) is deliberately not undoable — registry
    // state, not a modeled edit.
    assert_eq!(
        doc.undo_depth(),
        depth_before,
        "tag.create must not add an undo entry"
    );
}

#[test]
fn tag_assign_assigns_and_removes_a_tag_from_a_node() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.tag.assign",
        json!({ "id": obj_pub, "path": ["Structure"] }),
    );
    assert!(
        doc.node_tags(NodeId::Object(obj))
            .contains(&vec!["Structure".to_string()])
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    // Re-assign then remove, exercising `remove: true`.
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.tag.assign",
        json!({ "id": obj_pub, "path": ["Structure"] }),
    );
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.tag.assign",
        json!({ "id": obj_pub, "path": ["Structure"], "remove": true }),
    );
    assert!(
        !doc.node_tags(NodeId::Object(obj))
            .contains(&vec!["Structure".to_string()])
    );

    let data = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.tag.assign",
        json!({ "id": "obj_ffffff", "path": ["Structure"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn tag_set_visible_toggles_visibility_and_adds_no_undo_entry() {
    let mut doc = Document::new();
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.tag.set_visible",
        json!({ "path": ["Structure"], "visible": false }),
    );
    assert!(doc.tag_hidden(&["Structure".to_string()]));
    assert_eq!(
        doc.undo_depth(),
        depth_before,
        "tag.set_visible must not add an undo entry"
    );
}

#[test]
fn tag_delete_deletes_a_registered_tag_and_refuses_an_unknown_path() {
    let mut doc = Document::new();
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.tag.delete",
        json!({ "path": ["Structure"] }),
    );
    assert!(!doc.tag_meta().any(|(p, _)| p == ["Structure"]));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.tag.delete",
        json!({ "path": ["NoSuchTag"] }),
    );
    assert_eq!(data["refusal"], "unknown_tag");
}

#[test]
fn tag_rename_moves_the_tag_keeping_identity_and_refuses_unknown_duplicate_and_invalid() {
    let mut doc = Document::new();
    doc.set_tag_hidden(vec!["Structure".to_string()], true);
    doc.set_tag_hidden(vec!["Structure".to_string(), "Roof".to_string()], false);
    doc.set_tag_hidden(vec!["Oak".to_string()], false);
    let sid = doc
        .sid_of(&kernel::EntityRef::Tag(vec![
            "Structure".to_string(),
            "Roof".to_string(),
        ]))
        .unwrap();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.tag.rename",
        json!({ "path": ["Structure"], "new_path": ["Frame"] }),
    );
    assert!(!doc.tag_meta().any(|(p, _)| p == ["Structure"]));
    assert!(doc.tag_hidden(&["Frame".to_string()]));
    assert_eq!(
        doc.sid_of(&kernel::EntityRef::Tag(vec![
            "Frame".to_string(),
            "Roof".to_string()
        ])),
        Some(sid),
        "nested tag moved with its identity"
    );
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.tag.rename",
        json!({ "path": ["NoSuchTag"], "new_path": ["X"] }),
    );
    assert_eq!(data["refusal"], "unknown_tag");
    let data = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.tag.rename",
        json!({ "path": ["Structure"], "new_path": ["Oak"] }),
    );
    assert_eq!(data["refusal"], "duplicate_tag");
    let data = call_err(
        &mut conn,
        &mut doc,
        5,
        "hew.tag.rename",
        json!({ "path": ["Structure"], "new_path": ["Structure", "Inner"] }),
    );
    assert_eq!(data["refusal"], "invalid_tag_path");
}

// ================================================================== hew.guide

#[test]
fn guide_line_adds_a_guide_and_refuses_a_bad_derived_origin() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.guide.line",
        json!({ "origin": [0.0, 0.0, 0.0], "direction": [1.0, 0.0, 0.0] }),
    );
    let guide_pub = result["guide"].as_str().expect("guide id");
    assert!(matches!(
        api::IdResolver::new(&doc).resolve(guide_pub),
        Some(EntityRef::Guide(_))
    ));

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.guide.line",
        json!({
            "origin": { "point": "position", "of": "gde_ffffff" },
            "direction": [1.0, 0.0, 0.0]
        }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn guide_point_adds_a_guide_point() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.guide.point",
        json!({ "position": [1.0, 2.0, 3.0] }),
    );
    let guide_pub = result["guide"].as_str().expect("guide id").to_string();
    let entity = api::IdResolver::new(&doc)
        .resolve(&guide_pub)
        .expect("resolves");
    let EntityRef::Guide(gid) = entity else {
        panic!("expected a guide")
    };
    assert_eq!(
        doc.guide(gid),
        Some(&kernel::Guide::Point {
            position: Point3::new(1.0, 2.0, 3.0)
        })
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn guide_clear_deletes_every_guide() {
    let mut doc = Document::new();
    doc.add_guide_point(Point3::new(1.0, 1.0, 1.0)).unwrap();
    doc.add_guide_line(Point3::new(0.0, 0.0, 0.0), kernel::Vec3::new(1.0, 0.0, 0.0))
        .unwrap();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(&mut conn, &mut doc, 2, "hew.guide.clear", json!({}));
    assert!(doc.guide_ids().is_empty());

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn guide_angular_rotates_base_dir_around_plane_normal_and_adds_one_undo_entry() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    // Hand-computed 90° case: rotating +X by 90° about +Z (right-hand
    // rule) lands on +Y — the protractor reading a caller would expect.
    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.guide.angular",
        json!({
            "origin": [2.0, 3.0, 4.0],
            "plane_normal": [0.0, 0.0, 1.0],
            "base_dir": [1.0, 0.0, 0.0],
            "angle": std::f64::consts::FRAC_PI_2,
        }),
    );
    let guide_pub = result["guide"].as_str().expect("guide id").to_string();
    let entity = api::IdResolver::new(&doc)
        .resolve(&guide_pub)
        .expect("resolves");
    let EntityRef::Guide(gid) = entity else {
        panic!("expected a guide")
    };
    let Some(kernel::Guide::Line { origin, direction }) = doc.guide(gid).copied() else {
        panic!("expected a guide line")
    };
    assert!(origin.approx_eq(Point3::new(2.0, 3.0, 4.0), 1e-12));
    assert!(
        direction.approx_eq(kernel::Vec3::new(0.0, 1.0, 0.0), 1e-9),
        "expected +X rotated 90° about +Z to land on +Y, got {direction:?}"
    );

    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);
}

#[test]
fn guide_angular_refuses_a_bad_derived_origin() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.guide.angular",
        json!({
            "origin": { "point": "position", "of": "gde_ffffff" },
            "plane_normal": [0.0, 0.0, 1.0],
            "base_dir": [1.0, 0.0, 0.0],
            "angle": 0.5,
        }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn guide_angular_rejects_a_base_dir_with_no_projection_into_the_plane() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    // base_dir parallel to plane_normal projects to zero — a malformed
    // parameter interaction, not a document-state refusal.
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.guide.angular",
        json!({
            "origin": [0.0, 0.0, 0.0],
            "plane_normal": [0.0, 0.0, 1.0],
            "base_dir": [0.0, 0.0, 2.0],
            "angle": 0.5,
        }),
    );
    assert_eq!(
        r.error.unwrap().code,
        codes::INVALID_PARAMS,
        "a base_dir with no in-plane component is a params error, not a typed refusal"
    );
}

// =================================================================== hew.attr

#[test]
fn attr_set_writes_a_key_and_get_reads_it_back() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.set",
        json!({ "target": obj_pub, "ns": "com.example", "key": "part_no", "value": "A-42" }),
    );
    assert_one_undo_and_clean_undo(&mut doc, depth_before, &bytes_before);

    // Redo the write (undo just reverted it) so `get` has something to read.
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.attr.set",
        json!({ "target": obj_pub, "ns": "com.example", "key": "part_no", "value": "A-42" }),
    );
    let all = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.attr.get",
        json!({ "target": obj_pub }),
    );
    assert_eq!(all["com.example"]["part_no"], json!("A-42"));

    let filtered = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.attr.get",
        json!({ "target": obj_pub, "ns": "com.example" }),
    );
    assert_eq!(filtered["com.example"]["part_no"], json!("A-42"));

    let missing_ns = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.attr.get",
        json!({ "target": obj_pub, "ns": "com.nobody" }),
    );
    assert_eq!(missing_ns, json!({}));
}

#[test]
fn attr_set_refuses_an_unrepresentable_number() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.set",
        json!({ "target": obj_pub, "ns": "ns", "key": "k", "value": u64::MAX }),
    );
    assert_eq!(data["refusal"], "unrepresentable_attr_value");
}

#[test]
fn attr_set_refuses_a_too_deep_value() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let mut value = json!(null);
    for _ in 0..(kernel::attr::MAX_ATTR_DEPTH + 1) {
        value = json!([value]);
    }
    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.set",
        json!({ "target": obj_pub, "ns": "ns", "key": "k", "value": value }),
    );
    assert_eq!(data["refusal"], "attr_value_too_deep");
}

/// docs/agents/HEW_API.md §8: the `hew` prefix is reserved for first-party use,
/// on writes through the API. Both spellings the spec names — bare `hew`
/// and anything under `hew.` — refuse, from `set` and from `delete`.
#[test]
fn attr_writes_refuse_the_reserved_hew_namespace() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    for ns in ["hew", "hew.bim", "hew.a.b"] {
        let data = call_err(
            &mut conn,
            &mut doc,
            2,
            "hew.attr.set",
            json!({ "target": obj_pub, "ns": ns, "key": "k", "value": 1 }),
        );
        assert_eq!(data["refusal"], "reserved_attr_namespace", "set ns={ns}");
        assert_eq!(data["detail"]["ns"], ns);

        let data = call_err(
            &mut conn,
            &mut doc,
            2,
            "hew.attr.delete",
            json!({ "target": obj_pub, "ns": ns }),
        );
        assert_eq!(data["refusal"], "reserved_attr_namespace", "delete ns={ns}");
    }

    // A refusal leaves the document untouched (docs/agents/HEW_API.md §4.4):
    // no undo entry, and the serialized bytes are the ones we started
    // with — not merely unreached code, but pinned.
    assert_eq!(doc.undo_depth(), depth_before, "a refusal added an entry");
    assert_eq!(doc.save(), bytes_before, "a refusal changed the document");
}

/// The reservation is a prefix of *namespace segments*, not of characters:
/// a reverse-DNS name that merely starts with the letters `hew` belongs to
/// whoever coined it and must go through.
#[test]
fn attr_set_accepts_a_namespace_that_merely_begins_with_hew() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    for ns in ["hewlett.example", "hews", "hew-tools.example", "HEW"] {
        call_ok(
            &mut conn,
            &mut doc,
            2,
            "hew.attr.set",
            json!({ "target": obj_pub, "ns": ns, "key": "k", "value": 1 }),
        );
    }
}

/// Reads stay unrestricted: the reservation is on claiming a namespace,
/// not on seeing what a document already carries. A first-party
/// dictionary written kernel-side reads back through `hew.attr.get`.
#[test]
fn attr_get_still_reads_the_reserved_namespace() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    doc.attr_set(
        kernel::AttrTarget::Entity(EntityRef::Object(obj)),
        "hew.internal",
        "k",
        kernel::AttrValue::Int(7),
    )
    .expect("first-party write goes through the kernel, unrestricted");
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let got = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.get",
        json!({ "target": obj_pub, "ns": "hew.internal" }),
    );
    assert_eq!(got, json!({ "hew.internal": { "k": 7 } }));
}

#[test]
fn attr_set_refuses_an_unknown_target() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.set",
        json!({ "target": "obj_ffffff", "ns": "ns", "key": "k", "value": 1 }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

#[test]
fn attr_delete_deletes_a_key_and_refuses_an_unknown_attr() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    doc.attr_set(
        kernel::AttrTarget::Entity(EntityRef::Object(obj)),
        "ns",
        "k",
        kernel::AttrValue::Int(1),
    )
    .expect("seed attr");
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    let bytes_before = doc.save();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.delete",
        json!({ "target": obj_pub, "ns": "ns", "key": "k" }),
    );
    assert!(
        doc.attr_get(&kernel::AttrTarget::Entity(EntityRef::Object(obj)))
            .unwrap()
            .is_none_or(|dict| !dict.contains_key("ns"))
    );
    // `assert_one_undo_and_clean_undo` would undo the delete, restoring
    // the key — check the refusal on the SAME now-deleted key first,
    // then verify the undo/byte-identity property separately below.
    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.attr.delete",
        json!({ "target": obj_pub, "ns": "ns", "key": "k" }),
    );
    assert_eq!(data["refusal"], "unknown_attr");

    assert_eq!(doc.undo_depth(), depth_before + 1);
    doc.undo().expect("undo restores the delete");
    assert_eq!(doc.save(), bytes_before);
}

#[test]
fn attr_get_addresses_the_document_target() {
    let mut doc = Document::new();
    doc.attr_set(
        kernel::AttrTarget::Document,
        "ns",
        "k",
        kernel::AttrValue::Bool(true),
    )
    .expect("seed");
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let result = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.attr.get",
        json!({ "target": "document" }),
    );
    assert_eq!(result["ns"]["k"], json!(true));
}

// ================================================================ hew.history

#[test]
fn status_reports_no_entries_before_anything_happens() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let result = call_ok(&mut conn, &mut doc, 2, "hew.history.status", json!({}));
    assert_eq!(result["undo_depth"], json!(0));
    assert_eq!(result["redo_depth"], json!(0));
    assert_eq!(result["top"], Value::Null);
}

#[test]
fn status_reports_the_transact_label_and_connection_origin_after_an_api_transaction() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({
            "label": "Rename the leg",
            "commands": [
                { "method": "hew.entity.rename", "params": { "id": obj_pub, "name": "Leg" } }
            ]
        }),
    );
    let result = call_ok(&mut conn, &mut doc, 3, "hew.history.status", json!({}));
    assert_eq!(result["undo_depth"], json!(depth_before + 1));
    assert_eq!(result["top"]["label"], json!("Rename the leg"));
    assert_eq!(result["top"]["origin"], json!({ "connection": "test" }));
}

#[test]
fn undo_with_a_wrong_expected_label_refuses() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": obj_pub, "name": "Leg" }),
    );

    let data = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.history.undo",
        json!({ "expected_label": "not the real label" }),
    );
    assert_eq!(data["refusal"], "expected_label_mismatch");
    assert_eq!(data["detail"]["expected"], json!("not the real label"));
    assert_eq!(data["detail"]["found"], json!("hew.entity.rename"));
    // A refused undo leaves the document untouched.
    assert_eq!(doc.undo_depth(), depth_before + 1);
}

#[test]
fn undo_pops_the_top_entry_and_redo_restores_it() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    let obj_pub = public_of(&doc, &EntityRef::Object(obj));
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();
    call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.entity.rename",
        json!({ "id": obj_pub, "name": "Leg" }),
    );
    assert_eq!(doc.undo_depth(), depth_before + 1);

    call_ok(&mut conn, &mut doc, 3, "hew.history.undo", json!({}));
    assert_eq!(doc.undo_depth(), depth_before);
    assert_eq!(doc.object_name(obj), None);

    call_ok(&mut conn, &mut doc, 4, "hew.history.redo", json!({}));
    assert_eq!(doc.undo_depth(), depth_before + 1);
    assert_eq!(doc.object_name(obj), Some("Leg"));
}

#[test]
fn undo_refuses_nothing_to_undo_and_redo_refuses_nothing_to_redo() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let data = call_err(&mut conn, &mut doc, 2, "hew.history.undo", json!({}));
    assert_eq!(data["refusal"], "nothing_to_undo");

    let data = call_err(&mut conn, &mut doc, 3, "hew.history.redo", json!({}));
    assert_eq!(data["refusal"], "nothing_to_redo");
}
