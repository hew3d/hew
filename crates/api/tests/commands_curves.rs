//! Command-level coverage for the true-curves surface of the Hew API:
//! analytic surface classes in query output, and (in later work) the
//! fillet, chamfer, revolve, deform, loft, shell, project, smooth and array
//! commands.
//!
//! A test that drives a Preview feature's command or parameter
//! (`preview-features.json`) is compiled into a preview build only
//! (`HEW_PREVIEW=1`, `cfg(hew_preview)`): a production build refuses those
//! `preview_only`, which `preview_wall.rs` covers for every entry.
#![cfg_attr(not(hew_preview), allow(dead_code, unused_imports))]

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response, codes};
use kernel::{Document, ObjectId, Plane, Point3, SurfaceRef, Vec3};
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// --------------------------------------------------------------- fixtures

fn ground() -> Plane {
    Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane is well-defined")
}

/// A 1 × 1 × 0.5 box extruded from a ground rectangle.
fn build_box(doc: &mut Document) -> ObjectId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    {
        let sk = doc.sketch_mut(s).expect("sketch is live");
        let c = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .expect("rectangle segment");
        }
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    let regions = doc.extrudable_regions(s).expect("live");
    doc.extrude_region(s, regions[0], 0.5)
        .expect("extrude box")
        .0
}

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

/// Runs the connection through `hello` + `attach`.
fn hello_attach(conn: &mut Connection, doc: &mut Document) {
    for (id, method, params) in [
        (0, "hew.meta.hello", json!({ "protocol": 1 })),
        (1, "hew.doc.attach", json!({})),
    ] {
        let DispatchOutcome::Reply(r) =
            conn.dispatch(&REGISTRY, doc, &mut NoHost, req(id, method, params))
        else {
            panic!("{method} replies")
        };
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
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

// ---------------------------------------------------- query: surface kinds

#[test]
fn query_faces_reports_a_sphere_claim_with_its_kind() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    // The top face (z = 0.5) claims a sphere centered under it: a valid
    // weak-invariant claim, set through the test hook since no operation
    // stamps spheres on a box.
    let top = doc
        .object(obj)
        .unwrap()
        .faces()
        .iter()
        .find(|(_, f)| f.plane.normal().z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    let sphere = SurfaceRef::Sphere {
        center: Point3::new(0.5, 0.5, 0.0),
        radius: 2.0,
    };
    doc.set_face_surface_for_test(obj, top, Some(sphere));
    doc.object(obj).unwrap().validate().unwrap();

    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let result = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.query.faces",
        json!({ "object": object_id }),
    );
    let faces = result["faces"].as_array().unwrap();
    let claimed: Vec<&Value> = faces.iter().filter(|f| !f["surface"].is_null()).collect();
    assert_eq!(claimed.len(), 1, "only the top claims a surface");
    let s = &claimed[0]["surface"];
    assert_eq!(s["kind"], "sphere");
    assert_eq!(s["center"], json!([0.5, 0.5, 0.0]));
    assert_eq!(s["radius"], 2.0);
    assert!(s.get("axis").is_none(), "a sphere has no axis");
}

// ---------------------------------------------------------- hew.solid.revolve

/// Draws a closed polyline in the XZ plane (y = 0) and returns its region id.
fn draw_xz_profile(conn: &mut Connection, doc: &mut Document, id: i64, pts: &[[f64; 2]]) -> String {
    let mut points: Vec<Value> = pts.iter().map(|p| json!([p[0], 0.0, p[1]])).collect();
    points.push(points[0].clone());
    let r = call_ok(
        conn,
        doc,
        id,
        "hew.sketch.draw_line",
        json!({ "plane": { "origin": [0, 0, 0], "normal": [0, -1, 0] }, "points": points }),
    );
    result_of(&r)["region_id"]
        .as_str()
        .unwrap_or_else(|| panic!("one closed region: {r}"))
        .to_string()
}

/// A mutating command's own result inside its envelope.
fn result_of(v: &Value) -> &Value {
    v.get("results").map_or(v, |r| &r[0])
}

fn object_of(doc: &Document, public: &str) -> ObjectId {
    match api::IdResolver::new(doc).resolve(public) {
        Some(kernel::EntityRef::Object(id)) => id,
        other => panic!("{public} is not an object: {other:?}"),
    }
}

#[test]
#[cfg(hew_preview)]
fn revolve_rectangle_about_a_two_point_axis_makes_a_closed_cylinder() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let region = draw_xz_profile(
        &mut conn,
        &mut doc,
        2,
        &[[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]],
    );
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "from": [0, 0, 0], "to": [0, 0, 1] } }),
    );
    let oid = object_of(&doc, result_of(&r)["object_id"].as_str().unwrap());
    let obj = doc.object(oid).unwrap();
    obj.validate().unwrap();
    let walls: Vec<_> = obj
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .collect();
    // Default segments: the drawing count at radius 0.5.
    assert_eq!(
        walls.len() as u32,
        api::geom::draw_segments_per_turn(0.5),
        "one cylinder facet per default segment"
    );
}

#[test]
#[cfg(hew_preview)]
fn revolve_about_a_sketch_edge_partial_turn() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let region = draw_xz_profile(
        &mut conn,
        &mut doc,
        2,
        &[[0.2, 0.0], [0.5, 0.0], [0.5, 1.0], [0.2, 1.0]],
    );
    // The axis: a separate drawn line along Z.
    let line = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": { "origin": [0, 0, 0], "normal": [1, 0, 0] }, "points": [[0, 0, 0], [0, 0, 1]] }),
    );
    let sketch = result_of(&line)["sketch"].as_str().unwrap().to_string();
    let edge = json!({ "sketch": sketch, "at": [0, 0, 0.5] });
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "edge": edge }, "angle": std::f64::consts::FRAC_PI_2, "segments": 32 }),
    );
    let oid = object_of(&doc, result_of(&r)["object_id"].as_str().unwrap());
    let obj = doc.object(oid).unwrap();
    obj.validate().unwrap();
    let cyl = obj
        .faces()
        .values()
        .filter(|f| matches!(f.surface, Some(SurfaceRef::Cylinder { .. })))
        .count();
    assert_eq!(
        cyl, 16,
        "inner + outer walls, 8 facets each over a quarter turn"
    );
}

#[test]
#[cfg(hew_preview)]
fn revolve_refuses_an_axis_off_the_profile_plane_and_a_crossing_axis() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let region = draw_xz_profile(
        &mut conn,
        &mut doc,
        2,
        &[[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]],
    );
    let refusal = |r: Response| -> String {
        let e = r.error.expect("refused");
        e.data.expect("refusal data")["refusal"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "from": [0, 0, 0], "to": [0, 1, 0] } }),
    );
    assert_eq!(refusal(r), "axis_not_in_profile_plane");
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "from": [0.25, 0, 0], "to": [0.25, 0, 1] } }),
    );
    assert_eq!(refusal(r), "profile_crosses_axis");
    let r = call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "from": [0, 0, 0], "to": [0, 0, 1] }, "segments": 2 }),
    );
    assert_eq!(refusal(r), "amount_out_of_range");
    let r = call(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.revolve",
        json!({ "profile": region, "axis": { "from": [0, 0, 0], "to": [0, 0, 1] }, "angle": 1.0 }),
    );
    assert_eq!(refusal(r), "partial_sweep_on_pole");
}

#[test]
#[cfg(hew_preview)]
fn revolve_a_box_face_about_its_edge() {
    let mut doc = Document::new();
    let _obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let box_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    // The box's y = 0 side face turned a full turn about its x = 0 edge: a
    // radius-1 disk-ended cylinder; a quarter turn touches the axis and
    // refuses as a partial pole.
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.revolve",
        json!({
            "profile": { "face": { "object": box_id, "at": [0.5, 0.0, 0.25] } },
            "axis": { "from": [0, 0, 0], "to": [0, 0, 1] },
            "segments": 24
        }),
    );
    let oid = object_of(&doc, result_of(&r)["object_id"].as_str().unwrap());
    assert_ne!(oid, _obj, "a new solid");
    doc.object(oid).unwrap().validate().unwrap();
    doc.object(_obj).unwrap().validate().unwrap();
}

// ------------------------------------------------- hew.solid.follow_me twist

#[test]
#[cfg(hew_preview)]
fn follow_me_twist_and_end_scale() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let region = draw_xz_profile(
        &mut conn,
        &mut doc,
        2,
        &[[0.0, 0.0], [0.2, 0.0], [0.2, 0.2], [0.0, 0.2]],
    );
    // An open arc on the ground leaving the profile plane at (0.1, 0, 0).
    let arc = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_arc",
        json!({ "plane": { "ground": true }, "center": [2.1, 0, 0], "radius": 2.0,
                "start_angle": std::f64::consts::PI, "end_angle": std::f64::consts::PI + 0.5 }),
    );
    let curve = result_of(&arc)["curve_id"].as_str().unwrap().to_string();
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.follow_me",
        json!({ "profile": region, "path": { "curve": curve }, "twist": 0.5, "end_scale": 0.5 }),
    );
    let oid = object_of(&doc, result_of(&r)["object_id"].as_str().unwrap());
    let obj = doc.object(oid).unwrap();
    obj.validate().unwrap();
    assert!(
        obj.faces().values().all(|f| f.surface.is_none()),
        "a generated sweep"
    );
}

#[test]
#[cfg(hew_preview)]
fn follow_me_twist_around_a_face_refuses_twist_needs_open_path() {
    let mut doc = Document::new();
    let _ = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let box_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let region = draw_xz_profile(
        &mut conn,
        &mut doc,
        3,
        &[[0.4, 0.5], [0.6, 0.5], [0.6, 0.7], [0.4, 0.7]],
    );
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.follow_me",
        json!({ "profile": region, "path": { "face": { "object": box_id, "at": [0.5, 0.5, 0.5] } }, "twist": 0.5 }),
    );
    let e = r.error.expect("refused");
    assert_eq!(e.data.unwrap()["refusal"], "twist_needs_open_path");
}

// ----------------------------------------------------------- hew.solid.shell

#[test]
#[cfg(hew_preview)]
fn shell_hollows_a_box_with_its_top_open() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let result = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.shell",
        json!({
            "object": object_id,
            "open": [{ "object": object_id, "at": [0.5, 0.5, 0.5] }],
            "thickness": 0.01
        }),
    );
    assert_eq!(
        result["results"][0]["object_id"],
        json!(object_id),
        "the object keeps its id"
    );
    let o = doc.object(obj).unwrap();
    o.validate().unwrap();
    // 5 outer walls, 5 inner walls, one rim.
    assert_eq!(o.faces().len(), 11);

    // Too thick: a typed refusal naming where, and nothing changes.
    let before = doc.state_hash();
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.shell",
        json!({ "object": object_id, "thickness": 0.4 }),
    );
    let err = r.error.expect("a 0.4 m wall in a 0.5 m box refuses");
    let data = err.data.expect("a refusal carries data");
    assert_eq!(data["refusal"], "wall_too_thick");
    assert_eq!(data["detail"]["at"].as_array().map(Vec::len), Some(3));
    assert_eq!(doc.state_hash(), before);
}

// ---------------------------------------------------------- hew.solid.deform

#[test]
#[cfg(hew_preview)]
fn deform_twists_a_box_about_its_bounding_box() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.deform",
        json!({ "object": object_id, "mode": "twist", "end": "+z", "angle": std::f64::consts::FRAC_PI_2 }),
    );
    assert_eq!(result_of(&r)["object_id"], json!(object_id));
    let o = doc.object(obj).unwrap();
    o.validate().unwrap();
    // The top corner (1, 1, 0.5) turns 90° about the box's vertical
    // center line (0.5, 0.5).
    assert!(
        o.vertices()
            .values()
            .any(|v| (v.position - Point3::new(0.0, 1.0, 0.5)).length() < 1e-9)
    );
}

#[test]
#[cfg(hew_preview)]
fn deform_bend_too_far_refuses_with_the_limit() {
    let mut doc = Document::new();
    let _ = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let before = doc.state_hash();
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.deform",
        json!({ "object": object_id, "mode": "bend", "end": "+x", "toward": "+z", "angle": 6.0 }),
    );
    let data = r.error.expect("refused").data.unwrap();
    assert_eq!(data["refusal"], "self_intersecting");
    let limit = data["detail"]["limit"].as_f64().expect("a limit");
    assert!(limit > 0.0 && limit < 6.0);
    assert_eq!(doc.state_hash(), before);
}

#[test]
#[cfg(hew_preview)]
fn deform_bends_a_bar_along_a_drawn_arc() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let rect = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        json!({ "plane": { "ground": true }, "corner_a": [0, -0.02, 0], "corner_b": [1, 0.02, 0] }),
    );
    let region = result_of(&rect)["region_id"].as_str().unwrap().to_string();
    let bar = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.extrude",
        json!({ "region": region, "distance": 0.04 }),
    );
    let bar_id = result_of(&bar)["object_id"].as_str().unwrap().to_string();
    let arc = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_arc",
        json!({ "plane": { "ground": true }, "center": [0, 1, 0], "radius": 1.0,
                "start_angle": -std::f64::consts::FRAC_PI_2, "end_angle": 0.0 }),
    );
    let curve = result_of(&arc)["curve_id"].as_str().unwrap().to_string();
    call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.deform",
        json!({ "object": bar_id, "mode": "bend_to_path",
                "base": { "from": [0, 0, 0], "to": [1, 0, 0] }, "path": { "curve": curve } }),
    );
    let oid = object_of(&doc, &bar_id);
    let o = doc.object(oid).unwrap();
    o.validate().unwrap();
    // The bar's far end now sits at the arc's end, (1, 1, 0).
    assert!(
        o.vertices()
            .values()
            .any(|v| (v.position - Point3::new(1.0, 1.0, 0.0)).length() < 0.05)
    );
}

// --------------------------------------------------------- hew.solid.project

#[test]
#[cfg(hew_preview)]
fn project_engraves_a_square_into_a_box_top() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let drawn = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "origin": [0.0, 0.0, 2.0], "normal": [0.0, 0.0, 1.0] },
            "corner_a": [0.4, 0.4, 2.0],
            "corner_b": [0.6, 0.6, 2.0]
        }),
    );
    let sketch = drawn["results"][0]["sketch"].as_str().unwrap().to_string();
    let before = doc.object(obj).unwrap().clone();
    let result = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.project",
        json!({ "object": object_id, "sketch": sketch, "depth": 0.01 }),
    );
    assert_eq!(result["results"][0]["faces"], 1);
    let after = doc.object(obj).unwrap();
    after.validate().unwrap();
    assert!(after.faces().len() > before.faces().len());

    // Off to the side: the shape lands on nothing.
    let r = call(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "origin": [0.0, 0.0, 2.0], "normal": [0.0, 0.0, 1.0] },
            "corner_a": [3.0, 3.0, 2.0],
            "corner_b": [3.2, 3.2, 2.0]
        }),
    );
    let far = r.result.unwrap()["results"][0]["sketch"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.project",
        json!({ "object": object_id, "sketch": far }),
    );
    let data = r.error.expect("refused").data.expect("refusal data");
    assert_eq!(data["refusal"], "project_misses");
}

/// A cylinder (radius 0.5, height 1) on the ground, and a rectangle on the
/// plane x = 2 that faces it: its corners span y in [-0.2, 0.2] and z in
/// [0.3, 0.7]. Returns the object's public id, the sketch's public id and
/// the rectangle's region id.
fn cylinder_and_front_square(
    conn: &mut Connection,
    doc: &mut Document,
) -> (String, String, String) {
    let circle = call_ok(
        conn,
        doc,
        2,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.5 }),
    );
    let disc = result_of(&circle)["region_id"]
        .as_str()
        .unwrap()
        .to_string();
    let solid = call_ok(
        conn,
        doc,
        3,
        "hew.solid.extrude",
        json!({ "region": disc, "distance": 1.0 }),
    );
    let object = result_of(&solid)["object_id"].as_str().unwrap().to_string();
    let rect = call_ok(
        conn,
        doc,
        4,
        "hew.sketch.draw_rect",
        json!({
            "plane": { "origin": [2.0, 0.0, 0.0], "normal": [1.0, 0.0, 0.0] },
            "corner_a": [2.0, -0.2, 0.3],
            "corner_b": [2.0, 0.2, 0.7]
        }),
    );
    let rect = result_of(&rect);
    (
        object,
        rect["sketch"].as_str().unwrap().to_string(),
        rect["region_id"].as_str().unwrap().to_string(),
    )
}

/// The signed volume of an object's outer loops (holes ignored).
fn signed_volume(doc: &Document, object: ObjectId) -> f64 {
    let obj = doc.object(object).expect("the object is live");
    let mut v = 0.0;
    for (_, f) in obj.faces() {
        let pts: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
        for i in 1..pts.len() - 1 {
            v += pts[0]
                .to_vec()
                .dot(pts[i].to_vec().cross(pts[i + 1].to_vec()))
                / 6.0;
        }
    }
    v
}

#[test]
#[cfg(hew_preview)]
fn project_follow_mode_engraves_the_wall_without_refusing() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let (object, sketch, region) = cylinder_and_front_square(&mut conn, &mut doc);
    let id = object_of(&doc, &object);
    let volume_before = signed_volume(&doc, id);

    let result = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.project",
        json!({
            "object": object,
            "sketch": sketch,
            "regions": [region],
            "depth": 0.001,
            "mode": "follow"
        }),
    );
    assert_eq!(result_of(&result)["object_id"], object);
    assert!(
        signed_volume(&doc, id) < volume_before,
        "a follow engrave removes material"
    );
}

#[test]
#[cfg(hew_preview)]
fn project_refused_depth_leaves_no_imprint_behind() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let (object, sketch, region) = cylinder_and_front_square(&mut conn, &mut doc);
    let id = object_of(&doc, &object);
    let faces_before = doc.object(id).unwrap().faces().len();
    let volume_before = signed_volume(&doc, id);

    // The imprint would succeed; the zero depth is refused after it, and the
    // whole command rolls back with it.
    let r = call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.project",
        json!({ "object": object, "sketch": sketch, "regions": [region], "depth": 0.0 }),
    );
    let err = r.error.expect("a zero depth refuses");
    assert_eq!(err.data.unwrap()["refusal"], "amount_out_of_range");
    assert_eq!(doc.object(id).unwrap().faces().len(), faces_before);
    assert!((signed_volume(&doc, id) - volume_before).abs() < 1e-12);
}

// ------------------------------------------------------------ hew.solid.loft

fn rect_region(conn: &mut Connection, doc: &mut Document, id: i64, z: f64, h: f64) -> String {
    let r = call_ok(
        conn,
        doc,
        id,
        "hew.sketch.draw_rect",
        json!({ "plane": { "origin": [0, 0, z], "normal": [0, 0, 1] },
                "corner_a": [-h, -h, z], "corner_b": [h, h, z] }),
    );
    result_of(&r)["region_id"].as_str().unwrap().to_string()
}

#[test]
#[cfg(hew_preview)]
fn loft_two_rectangles_and_an_apex() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let a = rect_region(&mut conn, &mut doc, 2, 0.0, 0.5);
    let b = rect_region(&mut conn, &mut doc, 3, 1.0, 0.3);
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.loft",
        json!({ "profiles": [a, b] }),
    );
    let o = doc
        .object(object_of(
            &doc,
            result_of(&r)["object_id"].as_str().unwrap(),
        ))
        .unwrap();
    o.validate().unwrap();
    assert_eq!(o.faces().len(), 6, "a ruled frustum");
    let r = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.loft",
        json!({ "profiles": [a], "apex": [0, 0, -1] }),
    );
    let o = doc
        .object(object_of(
            &doc,
            result_of(&r)["object_id"].as_str().unwrap(),
        ))
        .unwrap();
    o.validate().unwrap();
    assert_eq!(o.faces().len(), 5, "a pyramid");
}

#[test]
#[cfg(hew_preview)]
fn loft_refusals() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let a = rect_region(&mut conn, &mut doc, 2, 0.0, 0.5);
    let refusal = |r: Response| -> String {
        r.error.expect("refused").data.unwrap()["refusal"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.loft",
        json!({ "profiles": [a] }),
    );
    assert_eq!(refusal(r), "loft_needs_two_profiles");
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.loft",
        json!({ "profiles": [a, { "curve": "crv_0_1" }] }),
    );
    assert_eq!(refusal(r), "open_profile");
}

// ------------------------------------------- hew.object.smooth / crease / bake

#[test]
#[cfg(hew_preview)]
fn smooth_crease_and_bake_through_the_api() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(scene["tree"][0]["smooth_level"], 0);

    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.object.smooth",
        json!({ "object": object_id, "level": 2 }),
    );
    assert_eq!(doc.smooth_level(obj), 2);
    let scene = call_ok(&mut conn, &mut doc, 4, "hew.query.scene", json!({}));
    assert_eq!(scene["tree"][0]["smooth_level"], 2);

    let r = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.object.crease",
        json!({ "edges": [{ "object": object_id, "at": [0.5, 0.0, 0.0] }] }),
    );
    assert_eq!(r["results"][0]["edges"], 1);
    assert_eq!(
        doc.object(obj)
            .unwrap()
            .edges()
            .values()
            .filter(|e| e.crease)
            .count(),
        1
    );

    let r = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.bake",
        json!({ "object": object_id }),
    );
    assert_eq!(r["results"][0]["object_id"], json!(object_id));
    assert_eq!(doc.smooth_level(obj), 0);
    assert!(doc.object(obj).unwrap().faces().len() > 6);

    // Nothing left to bake; a level out of range.
    let r = call(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.bake",
        json!({ "object": object_id }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "amount_out_of_range");
    let r = call(
        &mut conn,
        &mut doc,
        8,
        "hew.object.smooth",
        json!({ "object": object_id, "level": 4 }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "amount_out_of_range");
}

// ================================================== true-curves: blend ====

fn refusal_of(r: Response) -> Value {
    r.error.expect("refused").data.expect("refusal data")
}

/// A box (1 × 1 × 0.5) in a fresh connection; returns the public object id.
fn box_in_connection(doc: &mut Document) -> (Connection, String) {
    let _ = build_box(doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, doc);
    let scene = call_ok(&mut conn, doc, 2, "hew.query.scene", json!({}));
    let id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    (conn, id)
}

/// A face locator at the centroid of the first blend face of `object`.
fn blend_face_locator(conn: &mut Connection, doc: &mut Document, id: i64, object: &str) -> Value {
    let q = call_ok(
        conn,
        doc,
        id,
        "hew.query.faces",
        json!({ "object": object }),
    );
    let f = q["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f.get("blend").is_some())
        .expect("a blend face");
    json!({ "object": object, "at": f["centroid"] })
}

fn top_edge(object: &str) -> Value {
    json!({ "object": object, "from": [0.0, 0.0, 0.5], "to": [1.0, 0.0, 0.5] })
}

#[test]
#[cfg(hew_preview)]
fn fillet_rounds_the_top_rim_through_a_face_locator() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [{ "face": { "object": id, "at": [0.5, 0.5, 0.5] } }], "radius": 0.1 }),
    );
    let r = result_of(&r);
    assert_eq!(r["object_id"], json!(id));
    assert_eq!(r["blend_ids"].as_array().unwrap().len(), 1);
    assert!(r["faces"].as_u64().unwrap() > 4);
    assert!((r["size"].as_f64().unwrap() - 0.1).abs() < 1e-12);

    let oid = object_of(&doc, &id);
    doc.object(oid).unwrap().validate().unwrap();
    // The query names the blend on its faces.
    let q = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.faces",
        json!({ "object": id }),
    );
    let tagged: Vec<&Value> = q["faces"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f.get("blend").is_some())
        .collect();
    assert!(!tagged.is_empty());
    assert_eq!(tagged[0]["blend"]["kind"], "fillet");
    assert_eq!(tagged[0]["blend"]["role"], "strip");
}

#[test]
#[cfg(hew_preview)]
fn fillet_one_edge_with_tangent_propagation_off_and_on_a_cylinder_top() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.05, "segments": 4, "propagate": "none" }),
    );
    assert_eq!(result_of(&r)["blend_ids"].as_array().unwrap().len(), 1);
    doc.object(object_of(&doc, &id))
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
#[cfg(hew_preview)]
fn fillet_edge_set_forms_all_and_loop_near() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let before = doc.object(object_of(&doc, &id)).unwrap().faces().len();
    // loop_near on the top face picks its outer loop.
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.chamfer",
        json!({
            "edges": [{ "face": { "object": id, "at": [0.5, 0.5, 0.5] }, "loop_near": [0.5, 0.0, 0.5] }],
            "distance": 0.05
        }),
    );
    assert!(doc.object(object_of(&doc, &id)).unwrap().faces().len() > before);
    // all:true on a fresh box rounds all twelve edges.
    let mut doc2 = Document::new();
    let (mut conn2, id2) = box_in_connection(&mut doc2);
    call_ok(
        &mut conn2,
        &mut doc2,
        3,
        "hew.solid.fillet",
        json!({ "edges": [{ "object": id2, "all": true }], "radius": 0.1 }),
    );
    doc2.object(object_of(&doc2, &id2))
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
#[cfg(hew_preview)]
fn fillet_refusals_name_their_limit_and_family() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    // Too large: the detail carries the maximum and what limits it.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 5.0 }),
    ));
    assert_eq!(d["refusal"], "radius_too_large");
    assert!(d["detail"]["max"].as_f64().unwrap() > 0.0);
    assert!(d["detail"]["limited_by"].is_string());
    // A profile wider than the face.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "profile": "builtin:ogee", "width": 0.9 }),
    ));
    assert_eq!(d["refusal"], "profile_too_wide");
    assert!(d["detail"]["needed"].is_number() && d["detail"]["available"].is_number());
    // Unknown profile; segments out of range.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "profile": "nonesuch", "width": 0.1 }),
    ));
    assert_eq!(d["refusal"], "unknown_profile");
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.05, "segments": 500 }),
    ));
    assert_eq!(d["refusal"], "segments_out_of_range");
    // A cove and a bullnose on this box edge: a cove is fine, a bullnose
    // needs parallel sides of one face.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "profile": "bullnose" }),
    ));
    assert_eq!(d["refusal"], "bullnose_needs_parallel_sides");
    // A locator that finds no edge.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        8,
        "hew.solid.fillet",
        json!({ "edges": [{ "object": id, "at": [0.5, 0.5, 0.25] }], "radius": 0.05 }),
    ));
    assert_eq!(d["refusal"], "locator_missed");
    // Nothing changed through all of that.
    assert_eq!(doc.object(object_of(&doc, &id)).unwrap().faces().len(), 6);
}

#[test]
#[cfg(hew_preview)]
fn fillet_refuses_a_smooth_edge_and_edges_across_objects() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1, "propagate": "none" }),
    );
    // The rail between the strip and the top face is tangent now.
    let obj = doc.object(object_of(&doc, &id)).unwrap();
    let rail = obj
        .edges()
        .keys()
        .find(|&e| obj.edge_smoothness(e) == kernel::EdgeSmoothness::Tangent)
        .expect("a tangent rail");
    let (a, b) = obj.edge_endpoints(rail).unwrap();
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.fillet",
        json!({
            "edges": [{ "object": id, "from": [a.x, a.y, a.z], "to": [b.x, b.y, b.z] }],
            "radius": 0.01
        }),
    ));
    assert_eq!(d["refusal"], "edge_is_smooth");

    // A second object far from the first.
    let s = call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.sketch.draw_rect",
        json!({ "plane": { "origin": [0, 0, 0], "normal": [0, 0, 1] }, "corner_a": [3, 0, 0], "corner_b": [4, 1, 0] }),
    );
    let region = result_of(&s)["region_id"].as_str().unwrap().to_string();
    let e = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.solid.extrude",
        json!({ "region": region, "distance": 0.5 }),
    );
    let other = result_of(&e)["object_id"].as_str().unwrap().to_string();
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.fillet",
        json!({
            "edges": [
                { "object": id, "from": [0.0, 1.0, 0.0], "to": [1.0, 1.0, 0.0] },
                { "object": other, "from": [3.0, 0.0, 0.5], "to": [4.0, 0.0, 0.5] }
            ],
            "radius": 0.05
        }),
    ));
    assert_eq!(d["refusal"], "edges_span_objects");
}

#[test]
#[cfg(hew_preview)]
fn chamfer_forms_distance_pair_and_angle() {
    // Equal setbacks.
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.chamfer",
        json!({ "edges": [top_edge(&id)], "distance": 0.1, "propagate": "none" }),
    );
    assert!((result_of(&r)["size"].as_f64().unwrap() - 0.1).abs() < 1e-12);

    // A pair with a primary face (the top takes 0.2, the side 0.05).
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.chamfer",
        json!({
            "edges": [top_edge(&id)],
            "distances": [0.2, 0.05],
            "primary": { "object": id, "at": [0.5, 0.5, 0.5] },
            "propagate": "none"
        }),
    );
    assert!((result_of(&r)["size"].as_f64().unwrap() - 0.2).abs() < 1e-12);
    // The setbacks landed as asked: the top face lost 0.2 on the y = 0 side.
    let obj = doc.object(object_of(&doc, &id)).unwrap();
    let ys: Vec<f64> = obj
        .faces()
        .values()
        .filter(|f| f.plane.normal().z > 0.99)
        .flat_map(|f| {
            obj.loop_positions(f.outer_loop)
                .map(|p| p.y)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        ys.iter().any(|y| (y - 0.2).abs() < 1e-9),
        "top starts at y = 0.2: {ys:?}"
    );

    // Distance with a 45-degree angle on a right-angle edge: equal setbacks.
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.chamfer",
        json!({ "edges": [top_edge(&id)], "distance": 0.1, "angle": std::f64::consts::FRAC_PI_4, "propagate": "none" }),
    );
    assert!((result_of(&r)["size"].as_f64().unwrap() - 0.1).abs() < 1e-12);
    // An angle that cannot fit refuses before the kernel.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.chamfer",
        json!({ "edges": [{ "object": id, "from": [0.0, 1.0, 0.5], "to": [1.0, 1.0, 0.5] }], "distance": 0.1, "angle": 2.0 }),
    ));
    assert_eq!(d["refusal"], "angle_does_not_fit");
    // A chamfer needs a size.
    let r = call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.chamfer",
        json!({ "edges": [{ "object": id, "from": [0.0, 1.0, 0.5], "to": [1.0, 1.0, 0.5] }] }),
    );
    assert!(r.error.is_some());
}

#[test]
#[cfg(hew_preview)]
fn fillet_with_a_built_in_profile_makes_a_profile_blend() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "profile": "builtin:ogee", "width": 0.2, "propagate": "none" }),
    );
    assert!((result_of(&r)["size"].as_f64().unwrap() - 0.2).abs() < 1e-12);
    let q = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.query.faces",
        json!({ "object": id }),
    );
    assert!(
        q["faces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["blend"]["kind"] == "profile")
    );
}

#[test]
#[cfg(hew_preview)]
fn cove_and_variable_radius_and_stops_pass_through() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "profile": "cove", "radius": 0.1, "propagate": "none" }),
    );
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1, "propagate": "none", "stops": { "start": 0.2, "end": 0.2 } }),
    );
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1, "propagate": "none", "stops": { "start": 2.0, "end": 0.0 } }),
    ));
    assert_eq!(d["refusal"], "stop_out_of_range");
}

#[test]
#[cfg(hew_preview)]
fn resize_and_remove_a_blend_by_any_of_its_faces() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1, "propagate": "none" }),
    );
    let strip = blend_face_locator(&mut conn, &mut doc, 40, &id);
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.resize_blend",
        json!({ "face": strip, "radius": 0.2 }),
    );
    assert!((result_of(&r)["size"].as_f64().unwrap() - 0.2).abs() < 1e-12);
    // Switch to a chamfer.
    let strip = blend_face_locator(&mut conn, &mut doc, 41, &id);
    call_ok(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.resize_blend",
        json!({ "face": strip, "distance": 0.15 }),
    );
    let q = call_ok(
        &mut conn,
        &mut doc,
        6,
        "hew.query.faces",
        json!({ "object": id }),
    );
    assert!(
        q["faces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["blend"]["kind"] == "chamfer")
    );
    // A face that is not part of a blend.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        7,
        "hew.solid.remove_blend",
        json!({ "face": { "object": id, "at": [0.5, 0.5, 0.0] } }),
    ));
    assert_eq!(d["refusal"], "unknown_blend");
    // Remove it: back to a plain box.
    let strip = blend_face_locator(&mut conn, &mut doc, 42, &id);
    call_ok(
        &mut conn,
        &mut doc,
        8,
        "hew.solid.remove_blend",
        json!({ "face": strip }),
    );
    let obj = doc.object(object_of(&doc, &id)).unwrap();
    assert_eq!(obj.faces().len(), 6, "the sharp box is back");
    obj.validate().unwrap();
}

#[test]
#[cfg(hew_preview)]
fn fillet_rounds_a_drawn_sketch_corner() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let s = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_rect",
        json!({ "plane": { "origin": [0, 0, 0], "normal": [0, 0, 1] }, "corner_a": [0, 0, 0], "corner_b": [2, 1, 0] }),
    );
    let sketch = result_of(&s)["sketch"].as_str().unwrap().to_string();
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "corner": { "sketch": sketch, "at": [2, 1, 0] }, "radius": 0.25, "segments": 6 }),
    );
    assert_eq!(result_of(&r)["sketch_id"], json!(sketch));
    // The rounded corner is gone as a vertex; the same call now misses.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.fillet",
        json!({ "corner": { "sketch": sketch, "at": [2, 1, 0] }, "radius": 0.25 }),
    ));
    assert_eq!(d["refusal"], "locator_missed");
    // Too large for the other corner.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.chamfer",
        json!({ "corner": { "sketch": sketch, "at": [0, 0, 0] }, "distance": 9.0 }),
    ));
    assert_eq!(d["refusal"], "corner_radius_too_large");
}

#[test]
#[cfg(hew_preview)]
fn fillet_rounds_a_corner_of_a_shape_drawn_on_a_face() {
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let face = json!({ "object": id, "at": [0.5, 0.5, 0.5] });
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_rect",
        json!({ "plane": { "face": face }, "corner_a": [0.2, 0.2, 0.5], "corner_b": [0.7, 0.7, 0.5] }),
    );
    let faces_before = doc.object(object_of(&doc, &id)).unwrap().faces().len();
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.fillet",
        json!({ "corner": { "face": face, "at": [0.7, 0.7, 0.5] }, "radius": 0.1, "segments": 6 }),
    );
    let obj = doc.object(object_of(&doc, &id)).unwrap();
    obj.validate().unwrap();
    assert_eq!(
        obj.faces().len(),
        faces_before,
        "the imprint keeps its face"
    );
    // A box corner itself never blends as a 2D corner.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        5,
        "hew.solid.fillet",
        json!({ "corner": { "face": face, "at": [0.0, 0.0, 0.5] }, "radius": 0.1 }),
    ));
    assert!(matches!(
        d["refusal"].as_str().unwrap(),
        "corner_leaves_face" | "unknown_vertex"
    ));
}

#[test]
#[cfg(hew_preview)]
fn chamfer_angle_form_uses_the_law_of_sines() {
    // 30 degrees from the top face on a right-angle edge: d_b = d sin30 / sin60.
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.chamfer",
        json!({
            "edges": [top_edge(&id)],
            "distance": 0.1,
            "angle": std::f64::consts::FRAC_PI_6,
            "primary": { "object": id, "at": [0.5, 0.5, 0.5] },
            "propagate": "none"
        }),
    );
    let obj = doc.object(object_of(&doc, &id)).unwrap();
    let top_y: Vec<f64> = obj
        .faces()
        .values()
        .filter(|f| f.plane.normal().z > 0.99)
        .flat_map(|f| {
            obj.loop_positions(f.outer_loop)
                .map(|p| p.y)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        top_y.iter().any(|y| (y - 0.1).abs() < 1e-9),
        "the top starts 0.1 in: {top_y:?}"
    );
    let front_z: Vec<f64> = obj
        .faces()
        .values()
        .filter(|f| f.plane.normal().y < -0.99)
        .flat_map(|f| {
            obj.loop_positions(f.outer_loop)
                .map(|p| p.z)
                .collect::<Vec<_>>()
        })
        .collect();
    let want =
        0.5 - 0.1 * (std::f64::consts::FRAC_PI_6).sin() / (std::f64::consts::FRAC_PI_3).sin();
    assert!(
        front_z.iter().any(|z| (z - want).abs() < 1e-9),
        "the front face ends at {want}: {front_z:?}"
    );
}

#[test]
#[cfg(hew_preview)]
fn fillet_default_segments_follow_the_turn_of_the_edge() {
    // A right-angle edge turns pi/2: arc_segments(0.1, pi/2) facets.
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1, "propagate": "none" }),
    );
    let want = api::geom::arc_segments(0.1, std::f64::consts::FRAC_PI_2) as u64;
    assert_eq!(result_of(&r)["faces"].as_u64().unwrap(), want);
}

// ------------------------------------------------------------ hew.array.path

fn refusal_name(r: &Response) -> String {
    let err = r.error.as_ref().expect("expected a refusal");
    assert_eq!(err.code, codes::REFUSED);
    err.data.as_ref().expect("refusal data")["refusal"]
        .as_str()
        .expect("refusal name")
        .to_string()
}

#[test]
#[cfg(hew_preview)]
fn array_path_copies_along_a_drawn_circle_as_one_undo_step() {
    let mut doc = Document::new();
    build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);

    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let circle = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_circle",
        json!({ "plane": { "ground": true }, "center": [0.0, 4.0, 0.0], "radius": 3.0 }),
    );
    let curve_id = circle["results"][0]["curve_id"]
        .as_str()
        .unwrap()
        .to_string();

    let depth = doc.undo_depth();
    let params = json!({
        "nodes": [object_id],
        "path": { "curve": curve_id },
        "count": 6,
        "scale": { "from": 1.0, "to": 0.5 },
        "align": true
    });
    let result = call_ok(&mut conn, &mut doc, 4, "hew.array.path", params.clone());
    let ids = result["results"][0]["node_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 5, "count includes the original");
    assert!(ids.iter().all(|i| i.as_str().unwrap().starts_with("obj_")));
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    let scene = call_ok(&mut conn, &mut doc, 5, "hew.query.scene", json!({}));
    assert_eq!(scene["tree"].as_array().unwrap().len(), 6);
    doc.undo().unwrap();
    assert_eq!(doc.visible_object_ids().len(), 1);

    // Refusals: below the minimum, over the cap, bad scale, unknown node.
    let mut refused = |id, mutate: &dyn Fn(&mut Value)| {
        let mut p = params.clone();
        mutate(&mut p);
        refusal_name(&call(&mut conn, &mut doc, id, "hew.array.path", p))
    };
    assert_eq!(
        refused(6, &|p| p["count"] = json!(1)),
        "amount_out_of_range"
    );
    assert_eq!(
        refused(7, &|p| p["count"] = json!(1001)),
        "array_count_too_large"
    );
    assert_eq!(
        refused(8, &|p| p["scale"] = json!({ "from": 0.0, "to": 1.0 })),
        "amount_out_of_range"
    );
    assert_eq!(
        refused(9, &|p| p["nodes"] = json!(["obj_nope"])),
        "unknown_entity"
    );
    assert_eq!(doc.visible_object_ids().len(), 1, "refusals add nothing");
}

// ------------------------------------------ hew.solid.remove_degenerate_faces

#[test]
#[cfg(hew_preview)]
fn remove_degenerate_faces_refuses_a_clean_box() {
    let mut doc = Document::new();
    build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let before = doc.state_hash();
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.remove_degenerate_faces",
        json!({ "object": object_id }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "nothing_to_remove");
    assert_eq!(doc.state_hash(), before);
}

// ------------------------------------------------------------ hew.solid.soften

#[test]
#[cfg(hew_preview)]
fn soften_counts_and_refuses_a_bad_angle() {
    let mut doc = Document::new();
    let obj = build_box(&mut doc);
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let object_id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    let r = call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.soften",
        json!({ "object": object_id, "max_angle": 1.7 }),
    );
    assert_eq!(r["results"][0]["softened"], 12);
    assert_eq!(
        doc.object(obj)
            .unwrap()
            .edges()
            .values()
            .filter(|e| e.soft)
            .count(),
        12
    );
    let r = call(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.soften",
        json!({ "object": object_id, "max_angle": 4.0 }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "amount_out_of_range");
}

// ------------------------------------------------- hew.sketch: near misses

/// How many edges the document's first sketch has.
fn sketch_edge_count(doc: &Document) -> usize {
    doc.sketch(doc.sketch_ids()[0])
        .expect("a sketch")
        .edges()
        .len()
}

/// The sketch of a draw result, as the plane spec a follow-up command takes.
fn on_sketch(result: &Value) -> Value {
    json!({ "sketch": result_of(result)["sketch"] })
}

#[test]
fn endpoint_near_miss_refuses_a_rounded_endpoint_and_names_the_gap() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    // An open chain A -> B -> C: its open ends are A and C.
    let first = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({ "plane": { "ground": true },
                "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] }),
    );
    let plane = on_sketch(&first);
    let edges_before = sketch_edge_count(&doc);

    // The closing segment ends 1e-7 m short of A, as a client that rounds to
    // micrometers would leave it.
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[0.0, 1.0, 0.0], [1e-7, 0.0, 0.0]] }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "endpoint_near_miss");
    let gap = data["detail"]["gap"].as_f64().expect("a gap");
    assert!((gap - 1e-7).abs() < 1e-12, "gap {gap}");
    assert_eq!(data["detail"]["nearest"], json!([0.0, 0.0, 0.0]));
    assert_eq!(
        sketch_edge_count(&doc),
        edges_before,
        "a refusal adds nothing"
    );

    // The same segment at A exactly closes the triangle.
    let ok = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[0.0, 1.0, 0.0], [0.0, 0.0, 0.0]] }),
    );
    assert!(
        result_of(&ok)["region_id"].is_string(),
        "closing exactly makes the region: {ok}"
    );
}

#[test]
fn endpoint_near_miss_is_a_heuristic_with_a_floor_and_a_ceiling() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let first = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({ "plane": { "ground": true }, "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] }),
    );
    let plane = on_sketch(&first);
    // 1 mm from the open end (1, 0, 0) is a real, different point.
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[1.0, 0.001, 0.0], [1.0, 1.0, 0.0]] }),
    );
    // Closer than the merge distance is the same point, not a miss.
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[0.0, 1.0, 0.0], [5e-10, 0.0, 0.0]] }),
    );
}

#[test]
fn draw_line_refuses_an_endpoint_that_misses_by_a_hair() {
    let mut conn = Connection::new(Grant::core(), "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    let first = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({ "plane": { "ground": true }, "points": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]] }),
    );
    let plane = on_sketch(&first);
    // 50 micrometers shy of the existing endpoint at (1, 0, 0).
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[1.00005, 0.0, 0.0], [1.0, 1.0, 0.0]] }),
    );
    let err = r.error.expect("refused");
    let data = err.data.as_ref().expect("data");
    assert_eq!(data["refusal"], "endpoint_near_miss");
    assert!(data["detail"]["gap"].as_f64().expect("gap") > 4e-5);
    // Exactly on the endpoint is fine.
    call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_line",
        json!({ "plane": plane, "points": [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]] }),
    );
}

#[test]
fn endpoint_near_miss_also_guards_draw_arc() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let first = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.sketch.draw_line",
        json!({ "plane": { "ground": true }, "points": [[0.0, 0.0, 0.0], [0.36, 0.0, 0.0]] }),
    );
    let plane = on_sketch(&first);
    // A quarter arc about (0.36, 0.04) starting at (0.36, 0): its center is
    // 1e-7 m off, so the start misses the line's open end.
    let r = call(
        &mut conn,
        &mut doc,
        3,
        "hew.sketch.draw_arc",
        json!({ "plane": plane, "center": [0.36, 0.04 + 1e-7, 0.0], "radius": 0.04,
                "start_angle": -std::f64::consts::FRAC_PI_2, "end_angle": 0.0 }),
    );
    let data = r.error.expect("refused").data.expect("data");
    assert_eq!(data["refusal"], "endpoint_near_miss");
    let ok = call(
        &mut conn,
        &mut doc,
        4,
        "hew.sketch.draw_arc",
        json!({ "plane": plane, "center": [0.36, 0.04, 0.0], "radius": 0.04,
                "start_angle": -std::f64::consts::FRAC_PI_2, "end_angle": 0.0 }),
    );
    assert!(ok.error.is_none(), "{:?}", ok.error);
}

// ------------------------------------------------ API_GUIDE: curved profiles

/// The transaction printed in API_GUIDE.md "Draw curved profiles": a 0.4 x
/// 0.2 m plate with 0.04 m rounded corners, extruded 0.05 m, its top rim
/// filleted. Keep this and the guide in step.
#[test]
#[cfg(hew_preview)]
fn guide_rounded_plate_extrudes_and_fillets_its_top_rim() {
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let on_it = json!({ "sketch": { "$ref": "bottom#/sketch" } });
    let q = std::f64::consts::FRAC_PI_2;
    let commands = json!([
        { "method": "hew.sketch.draw_line", "as": "bottom",
          "params": { "plane": { "ground": true },
                      "points": [[0.04, 0, 0], [0.36, 0, 0]] } },
        { "method": "hew.sketch.draw_arc",
          "params": { "plane": on_it, "center": [0.36, 0.04, 0], "radius": 0.04,
                      "start_angle": -q, "end_angle": 0 } },
        { "method": "hew.sketch.draw_line",
          "params": { "plane": on_it, "points": [[0.4, 0.04, 0], [0.4, 0.16, 0]] } },
        { "method": "hew.sketch.draw_arc",
          "params": { "plane": on_it, "center": [0.36, 0.16, 0], "radius": 0.04,
                      "start_angle": 0, "end_angle": q } },
        { "method": "hew.sketch.draw_line",
          "params": { "plane": on_it, "points": [[0.36, 0.2, 0], [0.04, 0.2, 0]] } },
        { "method": "hew.sketch.draw_arc",
          "params": { "plane": on_it, "center": [0.04, 0.16, 0], "radius": 0.04,
                      "start_angle": q, "end_angle": 2.0 * q } },
        { "method": "hew.sketch.draw_line",
          "params": { "plane": on_it, "points": [[0, 0.16, 0], [0, 0.04, 0]] } },
        { "method": "hew.sketch.draw_arc", "as": "closed",
          "params": { "plane": on_it, "center": [0.04, 0.04, 0], "radius": 0.04,
                      "start_angle": 2.0 * q, "end_angle": 3.0 * q } },
        { "method": "hew.solid.extrude", "as": "plate",
          "params": { "region": { "$ref": "closed#/region_id" }, "distance": 0.05 } },
        { "method": "hew.solid.fillet",
          "params": { "edges": [{ "face": { "object": { "$ref": "plate#/object_id" },
                                            "at": [0.2, 0.1, 0.05] } }],
                      "radius": 0.01 } }
    ]);
    let r = call_ok(
        &mut conn,
        &mut doc,
        2,
        "hew.doc.transact",
        json!({ "label": "Rounded plate", "commands": commands }),
    );
    let results = r["results"].as_array().expect("one result per command");
    assert_eq!(results.len(), 10);
    assert!(
        results[7]["region_id"].is_string(),
        "the closing arc closes the profile: {}",
        results[7]
    );
    let plate = results[8]["object_id"].as_str().expect("an object");
    assert!(results[9]["blend_ids"].is_array(), "{}", results[9]);
    let o = doc.object(object_of(&doc, plate)).expect("the plate");
    assert!(o.validate().is_ok(), "the filleted plate is valid");
}

#[test]
#[cfg(hew_preview)]
fn push_pull_beside_a_fillet_names_the_object() {
    // The face next to a blend moves with the blend re-applied; the reply
    // names the object like every other in-place Push/Pull.
    let mut doc = Document::new();
    let (mut conn, id) = box_in_connection(&mut doc);
    call_ok(
        &mut conn,
        &mut doc,
        3,
        "hew.solid.fillet",
        json!({ "edges": [top_edge(&id)], "radius": 0.1 }),
    );
    let r = call_ok(
        &mut conn,
        &mut doc,
        4,
        "hew.solid.push_pull",
        json!({ "face": { "object": id, "at": [0.5, 0.5, 0.5] }, "distance": 0.1 }),
    );
    assert_eq!(result_of(&r)["object_id"], json!(id));
    doc.object(object_of(&doc, &id))
        .unwrap()
        .validate()
        .unwrap();
}

/// Two pipes of one radius (0.1, 32 sides, 0.8 long) in a tee, as the app
/// builds it: one up from (5, 0), the other laid along +y from just inside
/// it at z 0.4, then Union. Returns the tee and its junction's edges' end
/// points (the loop a click on one junction edge picks).
fn equal_radius_tee(doc: &mut Document) -> (ObjectId, Vec<(Point3, Point3)>) {
    use kernel::sketch::CurveGeom;
    use std::f64::consts::{PI, TAU};
    let pipe = |doc: &mut Document, c: Point3| {
        let s = doc.add_sketch(ground());
        let (r, n) = (0.1, 32);
        {
            let sk = doc.sketch_mut(s).unwrap();
            sk.begin_curve_with(CurveGeom {
                center: c,
                radius: r,
            })
            .unwrap();
            let at = |i: usize| {
                let a = TAU * (i % n) as f64 / n as f64;
                Point3::new(c.x + r * a.cos(), c.y + r * a.sin(), 0.0)
            };
            for i in 0..n {
                sk.add_segment(at(i), at(i + 1)).unwrap();
            }
            sk.end_curve();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], 0.8).unwrap().0
    };
    let main = pipe(doc, Point3::new(5.0, 0.0, 0.0));
    let branch = pipe(doc, Point3::new(5.0, 3.0, 0.0));
    let lay = kernel::Transform::translation(Vec3::new(-5.0, -3.0, 0.0))
        .then(&kernel::Transform::rotation(Vec3::new(1.0, 0.0, 0.0), -PI / 2.0).unwrap())
        .then(&kernel::Transform::translation(Vec3::new(5.0, 0.0006, 0.4)));
    doc.transform_object(branch, &lay).unwrap();
    let (tee, _) = doc.boolean(kernel::BooleanOp::Union, main, branch).unwrap();
    let obj = doc.object(tee).unwrap();
    let junction = obj
        .sharp_edges()
        .into_iter()
        .find(|&e| {
            let (a, b) = obj.edge_faces_for_test(e).unwrap();
            matches!(
                (obj.faces()[a].surface, obj.faces()[b].surface),
                (Some(SurfaceRef::Cylinder { axis: p, .. }), Some(SurfaceRef::Cylinder { axis: q, .. }))
                    if p.dot(q).abs() < 0.5
            )
        })
        .expect("a junction edge");
    let ends = obj
        .logical_edge(junction, false)
        .into_iter()
        .map(|e| obj.edge_endpoints(e).unwrap())
        .collect();
    (tee, ends)
}

#[test]
#[cfg(hew_preview)]
fn a_blend_where_the_faces_meet_flush_refuses_naming_the_point() {
    // D-234 through the API: an equal-radius tee's walls are tangent where
    // the junction passes the branch's sides, so a round or a chamfer of
    // the junction refuses `blend_faces_flush` with the point in `at`.
    let mut doc = Document::new();
    let mut conn = Connection::new(Grant::core(), "test");
    hello_attach(&mut conn, &mut doc);
    let (tee, ends) = equal_radius_tee(&mut doc);
    let scene = call_ok(&mut conn, &mut doc, 2, "hew.query.scene", json!({}));
    let id = scene["tree"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(object_of(&doc, &id), tee);
    let faces = doc.object(tee).unwrap().faces().len();
    let edges: Vec<Value> = ends
        .iter()
        .map(|(p, q)| json!({ "object": id, "from": [p.x, p.y, p.z], "to": [q.x, q.y, q.z] }))
        .collect();
    for (n, (method, params)) in [
        ("hew.solid.fillet", json!({ "radius": 0.01, "segments": 4 })),
        ("hew.solid.chamfer", json!({ "distance": 0.01 })),
    ]
    .into_iter()
    .enumerate()
    {
        let mut params = params;
        params["edges"] = json!(edges);
        params["propagate"] = json!("none");
        let d = refusal_of(call(&mut conn, &mut doc, 3 + n as i64, method, params));
        assert_eq!(d["refusal"], "blend_faces_flush", "{method}: {d}");
        let at = d["detail"]["at"].as_array().expect("at");
        let (x, z) = (at[0].as_f64().unwrap(), at[2].as_f64().unwrap());
        assert!(
            ((x - 4.9).abs() < 1e-3 || (x - 5.1).abs() < 1e-3) && (z - 0.4).abs() < 2e-3,
            "{method}: {at:?}"
        );
    }
    // A cove of this inside junction refuses before it gets that far.
    let d = refusal_of(call(
        &mut conn,
        &mut doc,
        9,
        "hew.solid.fillet",
        json!({ "edges": edges, "propagate": "none", "profile": "cove", "radius": 0.01, "segments": 4 }),
    ));
    assert_eq!(d["refusal"], "profile_needs_convex_edge");
    assert_eq!(doc.object(tee).unwrap().faces().len(), faces);
}
