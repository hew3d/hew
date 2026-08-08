//! Conformance coverage for `hew.query.*` and `hew.meta.documents`
//! (docs/design/api-implementation-conventions.md's testing bar): a
//! success path and a failure path per command, and proof that every
//! read-only dispatch adds no undo entry.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::{CurveGeom, Document, Material, NodeId, ObjectId, Plane, Point3, Rgba8, SketchId};
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

/// Sketches a unit-square rectangle at `(x0, 0)` and extrudes it 0.5m
/// tall. The rectangle's four edges are wholly consumed by the extrusion
/// (Model D — DEVELOPMENT.md / `Document::extrude_region`), so the sketch
/// does not survive; only the new watertight Object does.
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

/// A sketch that draws a closed circle and is never extruded — survives
/// as a `hew.query.scene` / `hew.query.entity` fixture (regions + a
/// circle curve).
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

/// A document exercising every entity kind `hew.query.scene` /
/// `hew.query.entity` describe: an object inside a group, an object
/// folded into a component + instance, a loose (unextruded) sketch, a
/// guide, a material, and a tag.
fn rich_doc() -> Document {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    doc.make_component(&[NodeId::Object(b)])
        .expect("fold into a component + instance");
    build_circle_sketch(&mut doc, 10.0, 10.0, 2.0);
    doc.add_guide_point(Point3::new(9.0, 9.0, 0.0))
        .expect("guide point");
    doc.add_material(Material::solid("Oak", Rgba8::rgb(180, 140, 90)));
    doc.set_tag_hidden(vec!["Structure".to_string()], false);
    doc
}

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

/// Runs the connection through `hello` + `attach` — every query command
/// needs an attached document.
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

// ------------------------------------------------------------- hew.meta.documents

#[test]
fn documents_lists_the_hosts_open_documents_and_adds_no_undo_entry() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let result = call_ok(&mut conn, &mut doc, 2, "hew.meta.documents", json!({}));
    assert_eq!(
        result["documents"],
        json!([]),
        "NoHost advertises no open documents"
    );
    assert_eq!(doc.undo_depth(), depth_before);
}

#[test]
fn documents_rejects_unknown_params() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.meta.documents",
        json!({ "bogus": true }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

// --------------------------------------------------------------- hew.query.scene

#[test]
fn scene_reports_the_tree_and_the_top_level_lists() {
    let mut doc = rich_doc();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));

    let tree = scene["tree"].as_array().expect("tree array");
    assert_eq!(tree.len(), doc.top_level_nodes().len());
    assert_eq!(
        tree.len(),
        2,
        "the grouped object and the folded-in instance"
    );
    let kinds: Vec<&str> = tree.iter().map(|n| n["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"group"));
    assert!(kinds.contains(&"instance"));

    let instance_node = tree.iter().find(|n| n["kind"] == "instance").unwrap();
    assert_eq!(
        instance_node["watertight"], true,
        "an instance's watertightness resolves through its definition's members"
    );

    let group_node = tree.iter().find(|n| n["kind"] == "group").unwrap();
    let members = group_node["members"].as_array().expect("group members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["kind"], "object");
    assert_eq!(members[0]["watertight"], true);
    assert!(members[0]["bbox"].is_object());

    let sketches = scene["sketches"].as_array().expect("sketches array");
    assert_eq!(
        sketches.len(),
        1,
        "both boxes' sketches were wholly consumed by extrusion (Model D); only the loose circle sketch remains"
    );
    assert_eq!(sketches[0]["regions"].as_array().unwrap().len(), 1);
    assert_eq!(sketches[0]["curves"].as_array().unwrap().len(), 1);
    assert_eq!(sketches[0]["curves"][0]["kind"], "circle");

    assert_eq!(scene["guides"].as_array().unwrap().len(), 1);
    assert_eq!(scene["materials"].as_array().unwrap().len(), 1);
    assert_eq!(scene["materials"][0]["name"], "Oak");
    assert_eq!(scene["tags"].as_array().unwrap().len(), 1);
    assert_eq!(scene["tags"][0]["path"], json!(["Structure"]));
    assert_eq!(scene["components"].as_array().unwrap().len(), 1);
    assert_eq!(scene["components"][0]["instance_count"], 1);

    let summary = &scene["document"];
    assert_eq!(
        summary["objects"], 1,
        "only the grouped object is a visible world object"
    );
    assert_eq!(summary["groups"], 1);
    assert_eq!(summary["instances"], 1);
    assert_eq!(summary["components"], 1);
    assert_eq!(summary["sketches"], 1);
    assert_eq!(summary["guides"], 1);
    assert_eq!(summary["materials"], 1);

    assert_eq!(
        doc.undo_depth(),
        depth_before,
        "a read-only query adds no undo entry"
    );
}

#[test]
fn scene_rejects_unknown_params() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.query.scene",
        json!({ "bogus": 1 }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

// -------------------------------------------------------------- hew.query.entity

#[test]
fn entity_describes_every_entity_kind_and_refuses_an_unknown_id() {
    let mut doc = rich_doc();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let tree = scene["tree"].as_array().unwrap();
    let group_node = tree.iter().find(|n| n["kind"] == "group").unwrap();
    let group_id = group_node["id"].as_str().unwrap().to_string();
    let object_id = group_node["members"][0]["id"].as_str().unwrap().to_string();
    let instance_id = tree.iter().find(|n| n["kind"] == "instance").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let component_id = scene["components"][0]["id"].as_str().unwrap().to_string();
    let sketch_id = scene["sketches"][0]["id"].as_str().unwrap().to_string();
    let guide_id = scene["guides"][0]["id"].as_str().unwrap().to_string();
    let material_id = scene["materials"][0]["id"].as_str().unwrap().to_string();
    let tag_id = scene["tags"][0]["id"].as_str().unwrap().to_string();

    let obj = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": object_id }),
    );
    assert_eq!(obj["kind"], "object");
    assert_eq!(obj["watertight"], true);
    assert_eq!(obj["face_count"], 6);
    assert!(obj["bbox"].is_object());
    assert!(obj["material_default"].is_null());

    let sketch = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.entity",
        json!({ "id": sketch_id }),
    );
    assert_eq!(sketch["kind"], "sketch");
    assert_eq!(sketch["regions"].as_array().unwrap().len(), 1);
    assert_eq!(sketch["curves"].as_array().unwrap().len(), 1);
    assert!(sketch["plane"]["normal"].is_array());

    let group = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.entity",
        json!({ "id": group_id }),
    );
    assert_eq!(group["kind"], "group");
    assert_eq!(group["members"].as_array().unwrap().len(), 1);

    let component = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.entity",
        json!({ "id": component_id }),
    );
    assert_eq!(component["kind"], "component");
    assert_eq!(component["members"].as_array().unwrap().len(), 1);
    assert_eq!(component["instances"].as_array().unwrap().len(), 1);

    let instance = call_ok(
        &mut conn,
        &mut doc,
        7,
        "hew.query.entity",
        json!({ "id": instance_id }),
    );
    assert_eq!(instance["kind"], "instance");
    assert_eq!(instance["def"], component_id);
    assert_eq!(instance["pose"].as_array().unwrap().len(), 12);
    assert_eq!(
        instance["watertight"], true,
        "resolved through the definition's members, same as the scene tree"
    );

    let guide = call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.query.entity",
        json!({ "id": guide_id }),
    );
    assert_eq!(guide["kind"], "guide");
    assert_eq!(guide["guide_kind"], "point");
    assert_eq!(guide["position"], json!([9.0, 9.0, 0.0]));

    let material = call_ok(
        &mut conn,
        &mut doc,
        9,
        "hew.query.entity",
        json!({ "id": material_id }),
    );
    assert_eq!(material["kind"], "material");
    assert_eq!(material["name"], "Oak");
    assert_eq!(material["has_texture"], false);

    let tag = call_ok(
        &mut conn,
        &mut doc,
        10,
        "hew.query.entity",
        json!({ "id": tag_id }),
    );
    assert_eq!(tag["kind"], "tag");
    assert_eq!(tag["path"], json!(["Structure"]));
    assert_eq!(tag["hidden"], false);

    let depth_before = doc.undo_depth();
    let refusal = call_err(
        &mut conn,
        &mut doc,
        11,
        "hew.query.entity",
        json!({ "id": "obj_deadbeef" }),
    );
    assert_eq!(refusal["refusal"], "unknown_entity");
    assert_eq!(refusal["failed_method"], "hew.query.entity");
    assert_eq!(doc.undo_depth(), depth_before);
}

/// A mesh recipe shaped exactly like `build_box`'s solid, minus its top
/// face — an open shell, never watertight, and not producible through the
/// sketch/extrude path (`Document::extrude_region` always closes), so this
/// goes in through `Document::ingest` instead (a legitimate public
/// construction path — see docs/design/api-kernel-map.md §6.1).
fn leaky_box_mesh(name: &str) -> kernel::MeshRecipe {
    let positions = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        // The top face (`vec![4, 5, 6, 7]` in a closed box) is
        // deliberately omitted — the whole point of this fixture.
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let face_count = faces.len();
    kernel::MeshRecipe {
        name: name.to_string(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
        face_uv_frames: vec![None; face_count],
        face_holes: vec![Vec::new(); face_count],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
    }
}

/// A document holding exactly one component instance whose sole definition
/// member is the leaky (open) box above — built directly as a definition +
/// instance through `Document::ingest`, rather than folding a world object
/// after the fact, since that is the shape `.skp`/glTF imports themselves
/// produce.
fn leaky_instance_doc() -> Document {
    let mut doc = Document::new();
    let scene = kernel::ImportScene {
        materials: Vec::new(),
        defs: vec![kernel::DefRecipe::from_meshes(
            Some("LeakyDef".to_string()),
            vec![leaky_box_mesh("Leaky")],
        )],
        roots: vec![kernel::ImportNode::Instance {
            def: 0,
            pose: kernel::Transform::IDENTITY,
            name: None,
            tags: Vec::new(),
            hidden: false,
        }],
        guides: Vec::new(),
        tags: Vec::new(),
    };
    doc.ingest(scene, Vec::new())
        .expect("ingest a component instance with a leaky sole member");
    doc
}

#[test]
fn an_instance_is_not_watertight_when_its_definitions_sole_member_is_leaky() {
    let mut doc = leaky_instance_doc();
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let instance_node = scene["tree"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "instance")
        .expect("the ingested instance is a top-level tree node");
    assert_eq!(
        instance_node["watertight"], false,
        "any leaky definition member makes the instance leaky, in the scene tree"
    );

    let instance_id = instance_node["id"].as_str().unwrap().to_string();
    let entity = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": instance_id }),
    );
    assert_eq!(
        entity["watertight"], false,
        "same resolution through hew.query.entity"
    );
}

/// A definition member tombstoned by `delete_def_member` (hide-not-delete:
/// it stays listed in `ComponentDef.members`, hidden) must NOT count as
/// leaky — the instance renders only visible members, and every visible
/// member here is watertight. Found by adversarial review: the first
/// resolution folded `object_solid` over ALL listed members, and a hidden
/// member's `object_solid` is `false` by design.
#[test]
fn a_tombstoned_definition_member_does_not_make_the_instance_leaky() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let b = build_box(&mut doc, 3.0);
    doc.make_component(&[NodeId::Object(a), NodeId::Object(b)])
        .expect("fold both boxes into one component");
    let component = *doc
        .component_ids()
        .first()
        .expect("the fold created a definition");
    let member = doc.def_members(component).expect("definition is live")[1];
    doc.delete_def_member(component, member)
        .expect("tombstone the second member");

    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let instance_node = scene["tree"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "instance")
        .expect("the instance is a top-level tree node");
    assert_eq!(
        instance_node["watertight"], true,
        "a hidden (tombstoned) member is excluded from the fold, not counted leaky"
    );

    let instance_id = instance_node["id"].as_str().unwrap().to_string();
    let entity = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.entity",
        json!({ "id": instance_id }),
    );
    assert_eq!(
        entity["watertight"], true,
        "same resolution through hew.query.entity"
    );

    // The definition's entity view must not publish the tombstone's id
    // either — handing out an id this same command refuses as
    // unknown_entity would strand any client that walks `members`.
    let component_id = entity["definition"].as_str().map(String::from);
    let component_id = component_id.unwrap_or_else(|| {
        scene["tree"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "component")
            .and_then(|n| n["id"].as_str())
            .map(String::from)
            .unwrap_or_default()
    });
    if !component_id.is_empty() {
        let comp = call_ok(
            &mut conn,
            &mut doc,
            4,
            "hew.query.entity",
            json!({ "id": component_id }),
        );
        let members = comp["members"].as_array().expect("members array");
        assert_eq!(
            members.len(),
            1,
            "a tombstoned member's id must not be published: {members:?}"
        );
    }
}

// -------------------------------------------------------------- hew.query.faces

#[test]
fn faces_lists_every_face_with_plane_area_and_centroid_and_refuses_an_unknown_object() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();

    let result = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.faces",
        json!({ "object": object_id }),
    );
    let faces = result["faces"].as_array().expect("faces array");
    assert_eq!(faces.len(), 6, "a box has six faces");
    let total_area: f64 = faces.iter().map(|f| f["area"].as_f64().unwrap()).sum();
    assert!(
        (total_area - 4.0).abs() < 1e-9,
        "a 1x1x0.5 box's surface area is 4 m^2, got {total_area}"
    );
    for f in faces {
        assert!(f["plane"]["normal"].is_array());
        assert!(f["plane"]["point"].is_array());
        assert!(f["outer"].as_array().unwrap().len() >= 3);
        assert!(f["holes"].as_array().unwrap().is_empty());
        assert!(
            f["material"].is_null(),
            "a freshly extruded box has no paint"
        );
        assert!(
            f["surface"].is_null(),
            "a straight-walled box has no analytic surface"
        );
    }

    let refusal = call_err(
        &mut conn,
        &mut doc,
        4,
        "hew.query.faces",
        json!({ "object": "obj_deadbeef" }),
    );
    assert_eq!(refusal["refusal"], "unknown_entity");

    assert_eq!(doc.undo_depth(), depth_before);
}

// ------------------------------------------------------------ hew.query.raycast

#[test]
fn raycast_hits_the_nearest_face_and_refuses_a_miss() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let hit = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert!((hit["distance"].as_f64().unwrap() - 9.5).abs() < 1e-9);
    assert_eq!(hit["point"], json!([0.5, 0.5, 0.5]));
    assert_eq!(hit["normal"], json!([0.0, 0.0, 1.0]));

    let miss = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.query.raycast",
        json!({ "origin": [100.0, 100.0, 100.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(miss["refusal"], "locator_missed");

    assert_eq!(doc.undo_depth(), depth_before);
}

#[test]
fn raycast_refuses_an_exact_tie_between_two_coincident_faces() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0);
    build_box(&mut doc, 0.0); // an exact duplicate: its top face coincides with the first's
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let ambiguous = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(ambiguous["refusal"], "ambiguous_locator");
}

/// Places a second box's definition as an instance directly above the
/// first (same x/y footprint, raised 5m in z so the two never overlap),
/// returning the instance id — shared setup for the instance-hit tests
/// below.
fn build_raised_instance(doc: &mut Document) -> kernel::InstanceId {
    let member = build_box(doc, 0.0);
    let (_component, instance, _change) = doc
        .make_component(&[NodeId::Object(member)])
        .expect("fold into a component + instance");
    doc.transform_instance(
        instance,
        &kernel::Transform::translation(kernel::Vec3::new(0.0, 0.0, 5.0)),
    )
    .expect("raise the instance well clear of the world box beneath it");
    instance
}

#[test]
fn raycast_prefers_a_nearer_instance_hit_over_a_farther_world_object() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0); // world box: top face at z=0.5, distance 9.5 — farther
    build_raised_instance(&mut doc); // instance: top face at z=5.5, distance 4.5 — nearer
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let instance_pub = scene["tree"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "instance")
        .expect("the folded instance is a top-level tree node")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let hit = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(hit["kind"], "instance");
    assert_eq!(
        hit["object"], instance_pub,
        "the nearer instance hit reports the INSTANCE's public id"
    );
    assert!((hit["distance"].as_f64().unwrap() - 4.5).abs() < 1e-9);
    assert_eq!(hit["point"], json!([0.5, 0.5, 5.5]));
    assert_eq!(hit["normal"], json!([0.0, 0.0, 1.0]));
}

#[test]
fn raycast_skips_a_user_hidden_instance_and_falls_through_to_the_world_object_beneath_it() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0); // world box: top face at z=0.5 — the only hit once the instance is hidden
    let instance = build_raised_instance(&mut doc);
    doc.set_node_user_hidden(NodeId::Instance(instance), true);

    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let hit = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(hit["kind"], "object");
    assert!((hit["distance"].as_f64().unwrap() - 9.5).abs() < 1e-9);
    assert_eq!(hit["point"], json!([0.5, 0.5, 0.5]));
}

// ------------------------------------------------------------ hew.query.measure

#[test]
fn measure_reports_distance_and_delta_and_refuses_an_unknown_locator() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let m = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.query.measure",
        json!({ "from": [0.0, 0.0, 0.0], "to": [3.0, 4.0, 0.0] }),
    );
    assert!((m["distance"].as_f64().unwrap() - 5.0).abs() < 1e-12);
    assert_eq!(m["delta"], json!([3.0, 4.0, 0.0]));

    let refusal = call_err(
        &mut conn,
        &mut doc,
        3,
        "hew.query.measure",
        json!({
            "from": [0.0, 0.0, 0.0],
            "to": { "point": "bbox", "of": "obj_deadbeef", "anchor": "center" },
        }),
    );
    assert_eq!(refusal["refusal"], "unknown_entity");

    assert_eq!(doc.undo_depth(), depth_before);
}

// ------------------------------------------------------------ hew.query.resolve

#[test]
fn resolve_answers_point_face_and_edge_locators_and_refuses_a_miss() {
    let mut doc = Document::new();
    build_box(&mut doc, 0.0);
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);
    let depth_before = doc.undo_depth();

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();

    let point = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.resolve",
        json!({ "point": [1.0, 2.0, 3.0] }),
    );
    assert_eq!(point["point"], json!([1.0, 2.0, 3.0]));

    let face = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.resolve",
        json!({ "face": { "object": object_id, "at": [0.5, 0.5, 0.5] } }),
    );
    assert!(
        (face["face"]["area"].as_f64().unwrap() - 1.0).abs() < 1e-9,
        "the top face is 1x1 m"
    );

    let edge = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.query.resolve",
        json!({ "edge": { "object": object_id, "at": [0.5, 0.0, 0.5] } }),
    );
    assert!(edge["edge"]["from"].is_array());
    assert!(edge["edge"]["to"].is_array());

    let refusal = call_err(
        &mut conn,
        &mut doc,
        6,
        "hew.query.resolve",
        json!({ "face": { "object": object_id, "at": [50.0, 50.0, 50.0] } }),
    );
    assert_eq!(refusal["refusal"], "locator_missed");

    let malformed = call(&mut conn, &mut doc, 7, "hew.query.resolve", json!({}));
    assert_eq!(
        malformed.error.unwrap().code,
        codes::INVALID_PARAMS,
        "exactly one of point/face/edge is required"
    );

    assert_eq!(doc.undo_depth(), depth_before);
}

// ------------------------------------------------------------ hew.query.context

#[test]
fn context_reports_the_open_session_stack_and_direct_members() {
    let mut doc = Document::new();
    let a = build_box(&mut doc, 0.0);
    let (group, _) = doc.group_nodes(&[NodeId::Object(a)]).expect("group");
    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let empty = call_ok(&mut conn, &mut doc, 2, "hew.query.context", json!({}));
    assert_eq!(empty["stack"], json!([]));
    assert!(empty["direct_members"].is_null());

    doc.open_group_session(group).expect("open session");
    let depth_after_open = doc.undo_depth();

    let opened = call_ok(&mut conn, &mut doc, 3, "hew.query.context", json!({}));
    let stack = opened["stack"].as_array().expect("stack array");
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0]["kind"], "group");
    assert!(stack[0]["id"].is_string());
    assert!(!opened["direct_members"].as_array().unwrap().is_empty());

    assert_eq!(
        doc.undo_depth(),
        depth_after_open,
        "hew.query.context adds no undo entry"
    );
    doc.close_group_session().expect("close session");
}

#[test]
fn context_rejects_unknown_params() {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let r = call(
        &mut conn,
        &mut doc,
        2,
        "hew.query.context",
        json!({ "bogus": 1 }),
    );
    assert_eq!(r.error.unwrap().code, codes::INVALID_PARAMS);
}

/// Symmetric to the instance case: a user-hidden WORLD object never
/// raycasts — with nothing else in the scene, the ray misses.
#[test]
fn raycast_skips_a_user_hidden_world_object() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc, 0.0);
    doc.set_node_user_hidden(NodeId::Object(obj), true);

    let mut conn = Connection::new(Profile::Core, "test");
    hello_attach(&mut conn, &mut doc);

    let refusal = call_err(
        &mut conn,
        &mut doc,
        2,
        "hew.query.raycast",
        json!({ "origin": [0.5, 0.5, 10.0], "dir": [0.0, 0.0, -1.0] }),
    );
    assert_eq!(refusal["refusal"], "locator_missed");
}
