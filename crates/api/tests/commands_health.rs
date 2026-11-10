//! Conformance coverage for model health: `hew.doc.health`, the repairs
//! `hew.solid.cap` / `delete_faces` / `mark_surface` / `thicken`, and the
//! `health` field `hew.query` carries (the import-health design §7). A
//! success and a failure path per command, one undo entry per repair, and
//! one for a batch run as a transaction.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::{DefRecipe, Document, ImportNode, ImportScene, MeshRecipe, Point3, Transform};
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

/// The command's own result: a bare mutating command replies as a
/// one-command transaction (`{label, results: [..]}`), unwrapped here.
fn ok(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Value {
    let r = call(conn, doc, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    let result = r.result.expect("a successful reply carries a result");
    match result["results"].as_array() {
        Some(one) if result["label"] == method && one.len() == 1 => one[0].clone(),
        _ => result,
    }
}

fn refusal(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> String {
    let r = call(conn, doc, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(err.code, codes::REFUSED, "{method} should refuse typed");
    err.data.expect("refusal data")["refusal"]
        .as_str()
        .unwrap()
        .to_string()
}

fn attach(doc: &mut Document) -> Connection {
    let mut conn = Connection::new(Profile::Core, "test");
    ok(&mut conn, doc, "hew.meta.hello", json!({ "protocol": 1 }));
    ok(&mut conn, doc, "hew.doc.attach", json!({}));
    conn
}

fn mesh(name: &str, positions: Vec<Point3>, faces: Vec<Vec<usize>>) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::serialize::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::serialize::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

/// A unit box at `x0` without its lid.
fn open_box(name: &str, x0: f64) -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x0 + x, y, z);
    mesh(
        name,
        vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ],
        vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ],
    )
}

/// A 1 m square pane at `x0` on the ground.
fn pane(x0: f64) -> MeshRecipe {
    let p = |x: f64, y: f64| Point3::new(x0 + x, y, 0.0);
    mesh(
        "Pane",
        vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
        vec![vec![0, 1, 2, 3]],
    )
}

fn doc_with(roots: Vec<ImportNode>, defs: Vec<DefRecipe>) -> Document {
    let mut doc = Document::new();
    doc.ingest(
        ImportScene {
            materials: Vec::new(),
            defs,
            roots,
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            extra: Default::default(),
            geo: None,
        },
        Vec::new(),
    )
    .unwrap();
    doc
}

/// The health row named `name`.
fn row(health: &Value, name: &str) -> Value {
    health["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name || r["definition_name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn health_lists_a_shared_definitions_open_member_once_and_adds_no_undo_entry() {
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(kernel::Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = doc_with(
        vec![place(0.0), place(5.0)],
        vec![DefRecipe {
            name: Some("Bin".into()),
            children: vec![ImportNode::Mesh(open_box("Bin", 0.0))],
        }],
    );
    let mut conn = attach(&mut doc);
    let depth = doc.content_depth();
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    assert_eq!(doc.content_depth(), depth);
    let bin = row(&health, "Bin");
    assert_eq!(bin["verdict"]["kind"], "cappable");
    assert_eq!(bin["placements"], 2);
    assert!(bin["object"].as_str().unwrap().starts_with("obj_"));
    assert!(bin["definition"].as_str().unwrap().starts_with("cmp_"));
    let instances = bin["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .all(|i| i.as_str().unwrap().starts_with("ins_"))
    );
    assert_eq!(bin["rims"][0]["planar"], true);
    assert_eq!(bin["rims"][0]["sheet_edge"], false);
    assert_eq!(bin["rims"][0]["shared"], false, "no other piece meets it");
    assert!(bin["faces"].as_u64().unwrap() > 0);
    assert_eq!(health["counts"]["cappable"], 1);

    // The scene tree says the same per instance, beside `watertight`.
    let scene = ok(&mut conn, &mut doc, "hew.query.scene", json!({}));
    let first = &scene["tree"][0];
    assert_eq!(first["health"], "cappable");
    assert_eq!(first["watertight"], false);

    // Capping the member closes every instance, in one undo entry.
    let object = bin["object"].as_str().unwrap();
    let capped = ok(
        &mut conn,
        &mut doc,
        "hew.solid.cap",
        json!({ "object": object }),
    );
    assert_eq!(capped["closed"], true);
    assert_eq!(capped["faces"], 1);
    assert_eq!(doc.content_depth(), depth + 1);
    let scene = ok(&mut conn, &mut doc, "hew.query.scene", json!({}));
    assert_eq!(scene["tree"][1]["health"], "solid");
    assert_eq!(scene["tree"][1]["watertight"], true);
    let entity = ok(
        &mut conn,
        &mut doc,
        "hew.query.entity",
        json!({ "id": object }),
    );
    assert_eq!(entity["health"], "solid");
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    assert!(health["rows"].as_array().unwrap().is_empty());

    ok(&mut conn, &mut doc, "hew.history.undo", json!({}));
    let health = ok(
        &mut conn,
        &mut doc,
        "hew.doc.health",
        json!({ "objects": [object] }),
    );
    assert_eq!(row(&health, "Bin")["verdict"]["kind"], "cappable");
}

#[test]
fn health_refuses_an_unknown_id() {
    let mut doc = doc_with(vec![ImportNode::Mesh(pane(0.0))], Vec::new());
    let mut conn = attach(&mut doc);
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.doc.health",
        json!({ "objects": ["obj_999"] }),
    );
    assert_eq!(name, "unknown_entity");
}

#[test]
fn cap_mints_face_tokens_a_push_pull_can_name_in_one_transaction() {
    let mut doc = doc_with(vec![ImportNode::Mesh(open_box("Tank", 0.0))], Vec::new());
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let object = row(&health, "Tank")["object"].as_str().unwrap().to_string();
    let depth = doc.content_depth();
    ok(
        &mut conn,
        &mut doc,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.solid.cap", "params": { "object": object }, "as": "lid" },
            { "method": "hew.solid.push_pull", "params": { "face": { "$face": "lid#cap.1" }, "distance": 0.5 } }
        ]}),
    );
    assert_eq!(
        doc.content_depth(),
        depth + 1,
        "one transaction, one undo entry"
    );
    let entity = ok(
        &mut conn,
        &mut doc,
        "hew.query.entity",
        json!({ "id": object }),
    );
    assert_eq!(entity["health"], "solid");
    let bbox = &entity["bbox"];
    assert!(
        (bbox["max"][2].as_f64().unwrap() - 1.5).abs() < 1e-9,
        "{bbox}"
    );
}

#[test]
fn cap_refuses_a_flat_sheet_and_a_solid() {
    let mut doc = doc_with(vec![ImportNode::Mesh(pane(0.0))], Vec::new());
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let object = row(&health, "Pane")["object"].as_str().unwrap().to_string();
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.cap",
        json!({ "object": object }),
    );
    assert_eq!(name, "cap_would_be_coplanar");
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.cap",
        json!({ "object": object, "rims": [3] }),
    );
    assert_eq!(name, "unknown_rim");

    ok(
        &mut conn,
        &mut doc,
        "hew.solid.thicken",
        json!({ "object": object, "distance": 0.1 }),
    );
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.cap",
        json!({ "object": object }),
    );
    assert_eq!(name, "object_is_solid");
}

#[test]
fn thicken_pulls_a_surface_into_a_solid_centered_or_not_and_refuses_the_rest() {
    let mut doc = doc_with(
        vec![
            ImportNode::Mesh(pane(0.0)),
            ImportNode::Mesh(open_box("Tank", 3.0)),
        ],
        Vec::new(),
    );
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let pane_id = row(&health, "Pane")["object"].as_str().unwrap().to_string();
    let tank_id = row(&health, "Tank")["object"].as_str().unwrap().to_string();

    let out = ok(
        &mut conn,
        &mut doc,
        "hew.solid.thicken",
        json!({ "object": pane_id, "distance": 0.2, "centered": true }),
    );
    assert_eq!(out["object_ids"], json!([]));
    let entity = ok(
        &mut conn,
        &mut doc,
        "hew.query.entity",
        json!({ "id": pane_id }),
    );
    assert_eq!(entity["health"], "solid");
    assert!((entity["bbox"]["min"][2].as_f64().unwrap() + 0.1).abs() < 1e-9);
    assert!((entity["bbox"]["max"][2].as_f64().unwrap() - 0.1).abs() < 1e-9);

    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.thicken",
        json!({ "object": tank_id, "distance": 0.1 }),
    );
    assert_eq!(name, "not_a_flat_sheet");
    ok(&mut conn, &mut doc, "hew.history.undo", json!({}));
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.thicken",
        json!({ "object": pane_id, "distance": 0.0 }),
    );
    assert_eq!(name, "thickness_too_small");
}

#[test]
fn mark_surface_flags_the_row_and_clears_and_refuses_a_solid() {
    let mut doc = doc_with(vec![ImportNode::Mesh(pane(0.0))], Vec::new());
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let object = row(&health, "Pane")["object"].as_str().unwrap().to_string();
    ok(
        &mut conn,
        &mut doc,
        "hew.solid.mark_surface",
        json!({ "object": object }),
    );
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    assert_eq!(row(&health, "Pane")["accepted"], true);
    ok(
        &mut conn,
        &mut doc,
        "hew.solid.mark_surface",
        json!({ "object": object, "accepted": false }),
    );
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    assert_eq!(row(&health, "Pane")["accepted"], false);

    ok(
        &mut conn,
        &mut doc,
        "hew.solid.thicken",
        json!({ "object": object, "distance": 0.1 }),
    );
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.mark_surface",
        json!({ "object": object }),
    );
    assert_eq!(name, "object_is_solid");
}

#[test]
fn delete_faces_drops_a_stray_sheet_and_refuses_a_face_of_another_object() {
    // A closed box with a loose sheet beside it, in one object.
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let mut positions = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    positions.extend([
        p(3.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
        p(4.0, 1.0, 0.0),
        p(3.0, 1.0, 0.0),
    ]);
    let crate_mesh = mesh(
        "Crate",
        positions,
        vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
            vec![8, 9, 10, 11],
        ],
    );
    let mut doc = doc_with(
        vec![ImportNode::Mesh(crate_mesh), ImportNode::Mesh(pane(10.0))],
        Vec::new(),
    );
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let crate_id = row(&health, "Crate")["object"]
        .as_str()
        .unwrap()
        .to_string();
    let pane_id = row(&health, "Pane")["object"].as_str().unwrap().to_string();

    let r = call(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": crate_id, "faces": [{ "object": pane_id, "at": [10.5, 0.5, 0.0] }] }),
    );
    assert!(
        r.error.is_some(),
        "a face of another object is a params error"
    );

    let out = ok(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": crate_id, "faces": [{ "object": crate_id, "at": [3.5, 0.5, 0.0] }] }),
    );
    assert_eq!(out["deleted"], false);
    assert_eq!(out["object_ids"], json!([]));
    let entity = ok(
        &mut conn,
        &mut doc,
        "hew.query.entity",
        json!({ "id": crate_id }),
    );
    assert_eq!(entity["health"], "solid");
    assert_eq!(entity["watertight"], true);

    // Every face of the pane: the object goes.
    let out = ok(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": pane_id, "faces": [{ "object": pane_id, "at": [10.5, 0.5, 0.0] }] }),
    );
    assert_eq!(out["deleted"], true);
    ok(&mut conn, &mut doc, "hew.history.undo", json!({}));
    // No faces named: every face.
    let out = ok(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": pane_id }),
    );
    assert_eq!(out["deleted"], true);
    let r = call(
        &mut conn,
        &mut doc,
        "hew.query.entity",
        json!({ "id": pane_id }),
    );
    assert!(r.error.is_some(), "the pane is gone");
    // A solid refuses.
    let name = refusal(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": crate_id, "faces": [{ "object": crate_id, "at": [0.5, 0.5, 1.0] }] }),
    );
    assert_eq!(name, "object_is_solid");
}

#[test]
fn a_deleted_object_answers_unknown_entity() {
    let mut doc = doc_with(vec![ImportNode::Mesh(open_box("Tank", 0.0))], Vec::new());
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let object = row(&health, "Tank")["object"].as_str().unwrap().to_string();
    ok(
        &mut conn,
        &mut doc,
        "hew.entity.delete",
        json!({ "id": object }),
    );
    for (method, params) in [
        ("hew.solid.cap", json!({ "object": object })),
        ("hew.solid.mark_surface", json!({ "object": object })),
        (
            "hew.solid.thicken",
            json!({ "object": object, "distance": 0.1 }),
        ),
        ("hew.solid.delete_faces", json!({ "object": object })),
        ("hew.doc.health", json!({ "objects": [object] })),
    ] {
        assert_eq!(
            refusal(&mut conn, &mut doc, method, params),
            "unknown_entity",
            "{method}"
        );
    }
}

#[test]
fn deleting_a_member_inside_a_group_within_its_definition_works() {
    let mut doc = doc_with(
        vec![ImportNode::Instance {
            def: 0,
            pose: Transform::IDENTITY,
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        }],
        vec![DefRecipe {
            name: Some("Sign".into()),
            children: vec![
                ImportNode::Mesh(open_box("Bin", 0.0)),
                ImportNode::Group {
                    name: "Inner".into(),
                    children: vec![ImportNode::Mesh(pane(3.0))],
                    tags: Vec::new(),
                    hidden: false,
                    key: None,
                },
            ],
        }],
    );
    let mut conn = attach(&mut doc);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    let pane_id = row(&health, "Pane")["object"].as_str().unwrap().to_string();
    let out = ok(
        &mut conn,
        &mut doc,
        "hew.solid.delete_faces",
        json!({ "object": pane_id }),
    );
    assert_eq!(out["deleted"], true);
    let health = ok(&mut conn, &mut doc, "hew.doc.health", json!({}));
    assert!(
        health["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["name"] != "Pane")
    );
}
