//! Conformance coverage for faces and solids from drawing through the API
//! (wireframe faces and solids §7): `hew.solid.from_faces` (Make Solid on
//! drawn faces) with its three refusals, and the optional `faces_closed` /
//! `solid` result fields the drawing commands gained — the automatic rule
//! applies to API drawing as to pointer drawing, so an agent that draws a
//! rectangle and four lines to an apex gets a pyramid.

use api::{Connection, DispatchOutcome, NoHost, Profile, Request, RequestId, Response, codes};
use kernel::{Document, EntityRef, Plane, Point3, SketchId, WatertightState};
use serde_json::{Value, json};

// ----------------------------------------------------------------- fixtures

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
    for (id, method, params) in [
        (0, "hew.meta.hello", json!({ "protocol": 1 })),
        (1, "hew.doc.attach", json!({})),
    ] {
        let DispatchOutcome::Reply(r) = conn.dispatch(doc, &mut NoHost, req(id, method, params))
        else {
            panic!("{method} replies")
        };
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
}

fn session() -> (Connection, Document) {
    let mut conn = Connection::new(Profile::Core, "test");
    let mut doc = Document::new();
    hello_attach(&mut conn, &mut doc);
    (conn, doc)
}

fn call(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(doc, &mut NoHost, req(7, method, params)) else {
        panic!("{method} replies")
    };
    r
}

/// A plain model-mutating request is a one-command transaction; this
/// asserts it added EXACTLY one undo entry that undoes byte-identically,
/// redoes it, and answers the bare command result (`results[0]`).
fn call_ok(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Value {
    let before = doc.save();
    let depth = doc.undo_depth();
    let r = call(conn, doc, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    assert_eq!(doc.undo_depth(), depth + 1, "{method}: one undo entry");
    doc.undo().expect("undo");
    assert_eq!(doc.save(), before, "{method}: undo is byte-identical");
    doc.redo().expect("redo");
    let result = r.result.expect("a result");
    match result.get("results") {
        Some(results) => results[0].clone(),
        None => result,
    }
}

/// A refusal: the canonical §4.4 `error.data`, the document untouched.
fn call_err(conn: &mut Connection, doc: &mut Document, method: &str, params: Value) -> Value {
    let before = doc.save();
    let r = call(conn, doc, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(err.code, codes::REFUSED, "{method}: a typed refusal");
    assert_eq!(
        doc.save(),
        before,
        "a refusal leaves the document untouched"
    );
    err.data.expect("a refusal carries data")
}

fn public(doc: &Document, entity: EntityRef) -> String {
    api::ids::public_id(&entity, doc.sid_of(&entity).expect("a stable id"))
}

fn object_handle(doc: &Document, public_id: &str) -> kernel::ObjectId {
    match api::IdResolver::new(doc).resolve(public_id) {
        Some(EntityRef::Object(o)) => o,
        other => panic!("{public_id} is no object: {other:?}"),
    }
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A closed polygon as one raw kernel sketch gesture: no close pass and no
/// cells search, so a set of faces that would close on its own when drawn
/// stays open (five faces of a cube cannot otherwise exist).
fn raw_face(doc: &mut Document, corners: &[Point3]) -> SketchId {
    let s = doc.add_sketch(Plane::from_polygon(corners).unwrap());
    doc.begin_sketch_gesture(s).unwrap();
    let sk = doc.sketch_mut(s).unwrap();
    for i in 0..corners.len() {
        sk.add_segment(corners[i], corners[(i + 1) % corners.len()])
            .unwrap();
    }
    doc.end_sketch_gesture(s).unwrap();
    s
}

/// The unit cube's floor and four walls, the walls' top corners at `tops`.
fn open_box(doc: &mut Document, tops: [Point3; 4]) -> Vec<String> {
    let (a, b, c, d) = (p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.));
    let [a1, b1, c1, d1] = tops;
    [
        raw_face(doc, &[a, b, c, d]),
        raw_face(doc, &[a, b, b1, a1]),
        raw_face(doc, &[b, c, c1, b1]),
        raw_face(doc, &[c, d, d1, c1]),
        raw_face(doc, &[d, a, a1, d1]),
    ]
    .into_iter()
    .map(|s| public(doc, EntityRef::Sketch(s)))
    .collect()
}

fn cube_tops() -> [Point3; 4] {
    [p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)]
}

/// The vertical plane through `a`–`b` as an `{origin, normal}` plane spec.
fn vertical_plane(a: [f64; 3], b: [f64; 3]) -> Value {
    let d = [b[0] - a[0], b[1] - a[1]];
    json!({ "origin": a, "normal": [d[1], -d[0], 0.0] })
}

fn ground_rect(conn: &mut Connection, doc: &mut Document) -> Value {
    call_ok(
        conn,
        doc,
        "hew.sketch.draw_rect",
        json!({ "plane": { "ground": true }, "corner_a": [0, 0, 0], "corner_b": [1, 1, 0] }),
    )
}

const APEX: [f64; 3] = [0.5, 0.5, 1.0];
const CORNERS: [[f64; 3]; 4] = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];

fn assert_solid(doc: &Document, object_id: &str) {
    let obj = doc
        .object(object_handle(doc, object_id))
        .expect("a visible object");
    assert_eq!(obj.watertight(), WatertightState::Watertight);
    obj.validate().expect("the solid validates");
}

// ======================================================= hew.solid.from_faces

/// §16 R26/R27: `from_faces` first fills the faces the named sketches'
/// lines can close (the close pass), then searches. Five faces of a cube
/// (floor and four walls, raw: no pass ran) close their lid on the way and
/// make one solid, every named sketch consumed. A floor and three walls
/// close nothing: the gap's rim is one loop of six corners on no one plane,
/// refused `loop_not_planar` with the document untouched; with the fourth
/// wall drawn, one named wall is enough.
#[test]
fn from_faces_closes_the_lid_of_five_cube_faces_and_refuses_four() {
    let (mut conn, mut doc) = session();
    let five = open_box(&mut doc, cube_tops());
    let out = call_ok(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": five }),
    );
    assert_eq!(out["grew"], false);
    let id = out["object_id"].as_str().unwrap();
    assert_solid(&doc, id);
    assert_eq!(doc.visible_object_ids().len(), 1);
    let obj = doc.object(object_handle(&doc, id)).unwrap();
    assert_eq!(obj.faces().len(), 6);
    assert!((volume(obj) - 1.0).abs() <= 1e-12, "volume {}", volume(obj));
    let faced = doc
        .sketch_ids()
        .into_iter()
        .filter(|&s| {
            doc.sketch(s)
                .unwrap()
                .regions()
                .values()
                .any(|r| !r.faceless)
        })
        .count();
    assert_eq!(faced, 0, "every face went into the solid");

    let (mut conn, mut doc) = session();
    let (a, b, c, d) = (p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.));
    let [a1, b1, c1, d1] = cube_tops();
    let four: Vec<String> = [
        raw_face(&mut doc, &[a, b, c, d]),
        raw_face(&mut doc, &[a, b, b1, a1]),
        raw_face(&mut doc, &[b, c, c1, b1]),
        raw_face(&mut doc, &[c, d, d1, c1]),
    ]
    .into_iter()
    .map(|s| public(&doc, EntityRef::Sketch(s)))
    .collect();
    let data = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": four }),
    );
    assert_eq!(data["refusal"], "loop_not_planar", "{data}");
    let loops = data["detail"]["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 1);
    let corners = loops[0].as_array().unwrap();
    assert_eq!(corners.len(), 6, "d-a, a-a1, a1-b1, b1-c1, c1-d1, d1-d");
    for q in [d, a, a1, b1, c1, d1] {
        assert!(
            corners.iter().any(|v| v == &json!([q.x, q.y, q.z])),
            "{q:?} in {corners:?}"
        );
    }

    // One named wall is enough: its top edge seeds the lid, and the search
    // gathers the rest through shared edges. (The floor alone would not do:
    // none of its lines meets the lid.)
    raw_face(&mut doc, &[d, a, a1, d1]);
    let out = call_ok(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": [four[1].clone()] }),
    );
    assert_eq!(out["grew"], false);
    assert_solid(&doc, out["object_id"].as_str().unwrap());
}

/// The signed volume of `obj` (divergence theorem over every loop).
fn volume(obj: &kernel::Object) -> f64 {
    let mut six = 0.0;
    for f in obj.faces().values() {
        for l in std::iter::once(f.outer_loop).chain(f.inner_loops.iter().copied()) {
            let q: Vec<kernel::Vec3> = obj.loop_positions(l).map(|x| x.to_vec()).collect();
            for i in 1..q.len().saturating_sub(1) {
                six += q[0].dot(q[i].cross(q[i + 1]));
            }
        }
    }
    six / 6.0
}

/// A gap whose rim lies on no one plane refuses `loop_not_planar`, with the
/// loop's corners.
#[test]
fn from_faces_refuses_a_non_planar_gap() {
    let (mut conn, mut doc) = session();
    let mut tops = cube_tops();
    tops[3] = p(0., 1., 2.);
    let five = open_box(&mut doc, tops);
    let data = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": five }),
    );
    assert_eq!(data["refusal"], "loop_not_planar", "{data}");
    let loops = data["detail"]["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].as_array().unwrap().len(), 4);
}

/// One flat face encloses nothing: its four edges are open.
#[test]
fn from_faces_refuses_a_single_flat_face() {
    let (mut conn, mut doc) = session();
    let rect = ground_rect(&mut conn, &mut doc);
    let data = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": [rect["sketch"]] }),
    );
    assert_eq!(data["refusal"], "faces_not_closed");
    assert_eq!(data["detail"]["open_edges"], 4);
}

/// §16 R22: the same face drawn twice is one face (the solid forms, the
/// later copy is left over); faces lying over each other otherwise refuse
/// `faces_not_solid` with `detail.reason: overlapping_faces` and
/// `detail.segments`, the edges where they do.
#[test]
fn from_faces_takes_one_of_a_twin_and_refuses_an_overlap() {
    let (mut conn, mut doc) = session();
    let mut ids = open_box(&mut doc, cube_tops());
    let top = cube_tops();
    let lid = raw_face(&mut doc, &top);
    ids.push(public(&doc, EntityRef::Sketch(lid)));
    let twin = raw_face(&mut doc, &top);
    ids.push(public(&doc, EntityRef::Sketch(twin)));
    let r = call_ok(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": ids }),
    );
    assert_eq!(r["grew"], false, "{r}");
    assert!(doc.sketch(twin).is_some(), "the later twin is left over");

    // The front wall in two halves, and a whole front wall over them.
    let (mut conn, mut doc) = session();
    let (a, b) = (p(0., 0., 0.), p(1., 0., 0.));
    let (m, m1) = (p(0.5, 0., 0.), p(0.5, 0., 1.));
    let [a1, b1, c1, d1] = cube_tops();
    let (c, d) = (p(1., 1., 0.), p(0., 1., 0.));
    let ids: Vec<String> = [
        raw_face(&mut doc, &[a, b, c, d]),
        raw_face(&mut doc, &[a, m, m1, a1]),
        raw_face(&mut doc, &[m, b, b1, m1]),
        raw_face(&mut doc, &[a, b, b1, a1]),
        raw_face(&mut doc, &[b, c, c1, b1]),
        raw_face(&mut doc, &[c, d, d1, c1]),
        raw_face(&mut doc, &[d, a, a1, d1]),
        raw_face(&mut doc, &[a1, b1, c1, d1]),
    ]
    .into_iter()
    .collect::<Vec<_>>()
    .into_iter()
    .map(|s| public(&doc, EntityRef::Sketch(s)))
    .collect();
    let err = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": ids }),
    );
    assert_eq!(err["refusal"], "faces_not_solid", "{err}");
    assert_eq!(err["detail"]["reason"], "overlapping_faces");
    let segs = err["detail"]["segments"].as_array().expect("segments");
    assert!(!segs.is_empty());
    for seg in segs {
        assert_eq!(seg[0][1], 0.0, "on the front: {seg}");
        assert_eq!(seg[1][1], 0.0, "on the front: {seg}");
    }
}

/// §16 R22: faces lying over each other are named even while open edges
/// remain elsewhere among the faces: `faces_not_closed` carries
/// `detail.reason: overlapping_faces` and `detail.overlapping_segments`
/// beside its open edges.
#[test]
fn from_faces_names_an_overlap_beside_open_edges() {
    let (mut conn, mut doc) = session();
    let (a, b) = (p(0., 0., 0.), p(1., 0., 0.));
    let (m, m1) = (p(0.5, 0., 0.), p(0.5, 0., 1.));
    let [a1, b1, c1, d1] = cube_tops();
    let (c, d) = (p(1., 1., 0.), p(0., 1., 0.));
    let sketches = [
        raw_face(&mut doc, &[a, b, c, d]),
        raw_face(&mut doc, &[a, m, m1, a1]),
        raw_face(&mut doc, &[m, b, b1, m1]),
        raw_face(&mut doc, &[a, b, b1, a1]),
        raw_face(&mut doc, &[b, c, c1, b1]),
        raw_face(&mut doc, &[c, d, d1, c1]),
        raw_face(&mut doc, &[d, a, a1, d1]),
        raw_face(&mut doc, &[a1, b1, c1, d1]),
        // A lone square well away: four open edges.
        raw_face(
            &mut doc,
            &[p(3., 0., 0.), p(4., 0., 0.), p(4., 1., 0.), p(3., 1., 0.)],
        ),
    ];
    let ids: Vec<String> = sketches
        .iter()
        .map(|&s| public(&doc, EntityRef::Sketch(s)))
        .collect();
    let err = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": ids }),
    );
    assert_eq!(err["refusal"], "faces_not_closed", "{err}");
    assert_eq!(err["detail"]["open_edges"], 4);
    assert_eq!(err["detail"]["reason"], "overlapping_faces");
    let segs = err["detail"]["overlapping_segments"]
        .as_array()
        .expect("overlapping_segments");
    assert!(!segs.is_empty());
    for seg in segs {
        assert_eq!(seg[0][1], 0.0, "on the front: {seg}");
        assert_eq!(seg[1][1], 0.0, "on the front: {seg}");
    }
}

#[test]
fn from_faces_refuses_an_unknown_sketch() {
    let (mut conn, mut doc) = session();
    let data = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": ["skt_00000000000000ff"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    // An object id is not a sketch.
    let five = open_box(&mut doc, cube_tops());
    raw_face(&mut doc, &cube_tops());
    let made = call_ok(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": [five[0].clone()] }),
    );
    let data = call_err(
        &mut conn,
        &mut doc,
        "hew.solid.from_faces",
        json!({ "sketches": [made["object_id"]] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
}

// ============================================ drawing: faces_closed / solid

/// §7 / §11 stage 5: an API-drawn rectangle and four lines to an apex make
/// a pyramid, reported as `solid` on the closing `draw_line`; the lines
/// before it report the faces they closed.
#[test]
fn an_api_drawn_pyramid_reports_solid_on_the_closing_line() {
    let (mut conn, mut doc) = session();
    let rect = ground_rect(&mut conn, &mut doc);
    assert!(rect.get("faces_closed").is_none() && rect.get("solid").is_none());
    let mut results = Vec::new();
    for q in CORNERS {
        results.push(call_ok(
            &mut conn,
            &mut doc,
            "hew.sketch.draw_line",
            json!({ "plane": vertical_plane(q, APEX), "points": [q, APEX] }),
        ));
    }
    assert!(results[0].get("faces_closed").is_none(), "{}", results[0]);
    for r in &results[1..3] {
        assert_eq!(r["faces_closed"].as_array().map(Vec::len), Some(1), "{r}");
        assert!(r.get("solid").is_none(), "{r}");
    }
    let last = &results[3];
    assert_eq!(last["solid"]["grew"], false, "{last}");
    let id = last["solid"]["object_id"].as_str().unwrap();
    assert_solid(&doc, id);
    assert_eq!(
        doc.object(object_handle(&doc, id)).unwrap().faces().len(),
        5
    );
}

/// The same with each line drawn on the plane of the face it bounds (an
/// `{origin, normal}` spec through the triangle).
#[test]
fn a_pyramid_drawn_on_its_faces_own_planes_reports_solid() {
    let (mut conn, mut doc) = session();
    ground_rect(&mut conn, &mut doc);
    let mut last = Value::Null;
    for i in 0..4 {
        let (q, r) = (CORNERS[i], CORNERS[(i + 1) % 4]);
        let tri = Plane::from_polygon(&[
            p(q[0], q[1], q[2]),
            p(r[0], r[1], r[2]),
            p(APEX[0], APEX[1], APEX[2]),
        ])
        .unwrap();
        let n = tri.normal();
        last = call_ok(
            &mut conn,
            &mut doc,
            "hew.sketch.draw_line",
            json!({ "plane": { "origin": q, "normal": [n.x, n.y, n.z] }, "points": [q, APEX] }),
        );
    }
    assert_eq!(last["solid"]["grew"], false, "{last}");
    assert_solid(&doc, last["solid"]["object_id"].as_str().unwrap());
}

/// A roof drawn on a box through the API grows the box: `grew: true`, the
/// new id a solid carrying the box's name.
#[test]
fn a_roof_drawn_on_a_box_reports_grew() {
    let (mut conn, mut doc) = session();
    let tx = call(
        &mut conn,
        &mut doc,
        "hew.doc.transact",
        json!({ "label": "Box", "commands": [
            { "method": "hew.sketch.draw_rect", "as": "r", "params": { "plane": { "ground": true }, "corner_a": [0, 0, 0], "corner_b": [1, 1, 0] } },
            { "method": "hew.solid.extrude", "params": { "region": { "$ref": "r#/region_id" }, "distance": 1.0 } }
        ]}),
    );
    let bx = tx.result.unwrap()["results"][1]["object_id"]
        .as_str()
        .unwrap()
        .to_string();
    call_ok(
        &mut conn,
        &mut doc,
        "hew.entity.rename",
        json!({ "id": bx, "name": "House" }),
    );
    let ridge = [0.5, 0.5, 1.5];
    let mut last = Value::Null;
    for q in CORNERS {
        let top = [q[0], q[1], 1.0];
        last = call_ok(
            &mut conn,
            &mut doc,
            "hew.sketch.draw_line",
            json!({ "plane": vertical_plane(top, ridge), "points": [ridge, top] }),
        );
    }
    assert_eq!(last["solid"]["grew"], true, "{last}");
    let grown = last["solid"]["object_id"].as_str().unwrap();
    assert_ne!(grown, bx);
    assert_solid(&doc, grown);
    assert_eq!(
        doc.object_name(object_handle(&doc, grown)),
        Some("House"),
        "the grown solid keeps the name"
    );
    assert_eq!(doc.visible_object_ids().len(), 1);
}

/// `hew.sketch.move` carries the fields too: the last line drawn a meter
/// too high and moved down onto the apex closes the pyramid.
#[test]
fn moving_the_last_line_into_place_reports_solid() {
    let (mut conn, mut doc) = session();
    ground_rect(&mut conn, &mut doc);
    for q in &CORNERS[..3] {
        call_ok(
            &mut conn,
            &mut doc,
            "hew.sketch.draw_line",
            json!({ "plane": vertical_plane(*q, APEX), "points": [q, APEX] }),
        );
    }
    let d = CORNERS[3];
    let high = |v: [f64; 3]| [v[0], v[1], v[2] + 1.0];
    let line = call_ok(
        &mut conn,
        &mut doc,
        "hew.sketch.draw_line",
        json!({ "plane": vertical_plane(high(d), high(APEX)), "points": [high(d), high(APEX)] }),
    );
    assert!(line.get("solid").is_none());
    let spec = json!({ "sketch": line["sketch"] });
    let units = call(
        &mut conn,
        &mut doc,
        "hew.sketch.units",
        json!({ "plane": spec }),
    )
    .result
    .unwrap();
    let key = units["units"][0]["key"].as_str().unwrap().to_string();
    let moved = call_ok(
        &mut conn,
        &mut doc,
        "hew.sketch.move",
        json!({ "plane": spec, "units": [key], "offset": [0, 0, -1] }),
    );
    assert_eq!(moved["solid"]["grew"], false, "{moved}");
    assert_solid(&doc, moved["solid"]["object_id"].as_str().unwrap());
}

/// The sixth face of a cube drawn through the API closes the cube on its
/// own: the automatic rule, no Make Solid needed.
#[test]
fn drawing_the_sixth_face_reports_solid() {
    let (mut conn, mut doc) = session();
    open_box(&mut doc, cube_tops());
    let top = call_ok(
        &mut conn,
        &mut doc,
        "hew.sketch.draw_rect",
        json!({ "plane": { "origin": [0, 0, 1], "normal": [0, 0, 1] }, "corner_a": [0, 0, 1], "corner_b": [1, 1, 1] }),
    );
    assert_eq!(top["solid"]["grew"], false, "{top}");
    assert_solid(&doc, top["solid"]["object_id"].as_str().unwrap());
}
