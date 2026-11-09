//! The part-model reads (docs/agents/PLUGINS.md "What a row says about a
//! part", "Reading a container's own Objects", "Same part, mirrored part"):
//! `members`, `solid`, `family`, `family_closed`, and `linked` on
//! `hew.query.occurrences` rows; `scope: "own"` on `hew.query.part_frame`,
//! `hew.query.measure`, `hew.view.snapshot`, and `hew.view.line_drawing`;
//! `orient: "part"` on snapshots; and `hew.query.signature`.

use api::{
    Connection, DispatchOutcome, Grant, Host, LineDrawingParams, LineDrawingResult, NoHost,
    Refusal, Request, RequestId, SnapshotParams, SnapshotResult,
};
use kernel::{
    AttrTarget, AttrValue, Document, EntityRef, ImportNode, ImportScene, MeshRecipe, NodeId,
    ObjectId, Plane, Point3, Transform, Vec3,
};
use proptest::prelude::*;
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

/// An extrusion of `pts` (counter-clockwise from above) by `h`.
fn extrude_poly(doc: &mut Document, pts: &[(f64, f64)], z0: f64, h: f64) -> ObjectId {
    let plane = Plane::from_polygon(&[
        Point3::new(0.0, 0.0, z0),
        Point3::new(1.0, 0.0, z0),
        Point3::new(0.0, 1.0, z0),
    ])
    .unwrap();
    let s = doc.add_sketch(plane);
    let sk = doc.sketch_mut(s).unwrap();
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        sk.add_segment(Point3::new(a.0, a.1, z0), Point3::new(b.0, b.1, z0))
            .unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], h).unwrap().0
}

/// The box `[x0, x0 + dx] × [y0, y0 + dy] × [z0, z0 + dz]`.
fn cuboid(doc: &mut Document, at: [f64; 3], size: [f64; 3]) -> ObjectId {
    let [x0, y0, z0] = at;
    let [dx, dy, dz] = size;
    extrude_poly(
        doc,
        &[(x0, y0), (x0 + dx, y0), (x0 + dx, y0 + dy), (x0, y0 + dy)],
        z0,
        dz,
    )
}

fn named(doc: &mut Document, o: ObjectId, name: &str) -> ObjectId {
    doc.set_node_name(NodeId::Object(o), Some(name.into()))
        .unwrap();
    o
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

fn obj(doc: &Document, o: ObjectId) -> String {
    public(doc, EntityRef::Object(o))
}

fn call(doc: &mut Document, host: &mut dyn Host, method: &str, params: Value) -> Value {
    let mut conn = Connection::new(Grant::core(), "part-model");
    for (id, (m, p)) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
        (method, params),
    ]
    .into_iter()
    .enumerate()
    {
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(id as i64)),
            method: m.to_string(),
            params: Some(p),
        };
        let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, host, req) else {
            panic!("{m} replies");
        };
        if m == method {
            return serde_json::to_value(r).unwrap();
        }
    }
    unreachable!()
}

fn ok(doc: &mut Document, method: &str, params: Value) -> Value {
    let r = call(doc, &mut NoHost, method, params);
    assert!(r.get("error").is_none(), "{method}: {r}");
    r["result"].clone()
}

fn refusal(doc: &mut Document, method: &str, params: Value) -> String {
    let r = call(doc, &mut NoHost, method, params);
    r["error"]["data"]["refusal"]
        .as_str()
        .unwrap_or_else(|| panic!("{method} refuses: {r}"))
        .to_string()
}

fn params_error(doc: &mut Document, host: &mut dyn Host, method: &str, params: Value) {
    let r = call(doc, host, method, params.clone());
    assert_eq!(r["error"]["code"], -32602, "{method} {params}: {r}");
}

/// The rows by id.
fn rows(doc: &mut Document, params: Value) -> std::collections::BTreeMap<String, Value> {
    ok(doc, "hew.query.occurrences", params)["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["id"].as_str().unwrap().to_string(), r.clone()))
        .collect()
}

/// A health-report fixture mesh: the box `[x0, x1] × [0, 1] × [0, 1]`
/// without the faces in `drops` (3 is its x = x1 side, 5 its x = x0 side,
/// 1 its top). `bend` moves the x = x1 rim's corner `(x1, 1, 0)` and, on
/// the matching half, `(x0, 1, 0)` along +x, warping the shared rim.
fn half(name: &str, x0: f64, x1: f64, drops: &[usize], bend: f64) -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(x0, 0.0, 0.0),
        p(x1, 0.0, 0.0),
        p(x1 + if drops.contains(&3) { bend } else { 0.0 }, 1.0, 0.0),
        p(x0 + if drops.contains(&5) { bend } else { 0.0 }, 1.0, 0.0),
        p(x0, 0.0, 1.0),
        p(x1, 0.0, 1.0),
        p(x1, 1.0, 1.0),
        p(x0, 1.0, 1.0),
    ];
    let faces: Vec<Vec<usize>> = [
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
    .into_iter()
    .enumerate()
    .filter(|(i, _)| !drops.contains(i))
    .map(|(_, f)| f)
    .collect();
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

fn ingest(doc: &mut Document, meshes: Vec<MeshRecipe>) {
    let scene = ImportScene {
        geo: None,
        materials: Vec::new(),
        defs: Vec::new(),
        roots: meshes.into_iter().map(ImportNode::Mesh).collect(),
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
    };
    doc.ingest(scene, Vec::new()).unwrap();
}

/// The object named `name`.
fn by_name(doc: &Document, name: &str) -> ObjectId {
    doc.live_object_ids()
        .into_iter()
        .find(|&o| doc.object_name(o) == Some(name))
        .unwrap_or_else(|| panic!("no object named {name}"))
}

struct Dowel {
    doc: Document,
    door: kernel::GroupId,
    panel: ObjectId,
    hinge: kernel::InstanceId,
    plate: ObjectId,
    pin: ObjectId,
    shelf: ObjectId,
    left: ObjectId,
    right: ObjectId,
    open_box: ObjectId,
}

/// A Group "Door" holding the Object "Panel" (0.4 × 0.2 × 0.018 m) and a
/// placement of "Hinge" (its members "Plate" 0.04 × 0.03 × 0.002 and "Pin"
/// 0.01 × 0.01 × 0.03); a loose "Shelf" (0.3 × 0.2 × 0.018); two touching
/// half-boxes sharing a rim (one seam family); one open-topped box.
fn nested_dowel() -> Dowel {
    let mut doc = Document::new();
    let panel = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    named(&mut doc, panel, "Panel");
    let plate = cuboid(&mut doc, [0.0, 0.0, 0.018], [0.04, 0.03, 0.002]);
    named(&mut doc, plate, "Plate");
    let pin = cuboid(&mut doc, [0.0, 0.0, 0.02], [0.01, 0.01, 0.03]);
    named(&mut doc, pin, "Pin");
    let (def, hinge, _) = doc
        .make_component(&[NodeId::Object(plate), NodeId::Object(pin)])
        .unwrap();
    doc.set_component_name(def, Some("Hinge".into())).unwrap();
    let (door, _) = doc
        .group_nodes(&[NodeId::Object(panel), NodeId::Instance(hinge)])
        .unwrap();
    doc.set_node_name(NodeId::Group(door), Some("Door".into()))
        .unwrap();
    let shelf = cuboid(&mut doc, [1.0, 0.0, 0.0], [0.3, 0.2, 0.018]);
    named(&mut doc, shelf, "Shelf");
    ingest(
        &mut doc,
        vec![
            half("Left", 3.0, 4.0, &[3], 0.0),
            half("Right", 4.0, 5.0, &[5], 0.0),
            half("Open", 7.0, 8.0, &[1], 0.0),
        ],
    );
    let (left, right, open_box) = (
        by_name(&doc, "Left"),
        by_name(&doc, "Right"),
        by_name(&doc, "Open"),
    );
    Dowel {
        doc,
        door,
        panel,
        hinge,
        plate,
        pin,
        shelf,
        left,
        right,
        open_box,
    }
}

fn close(a: &Value, want: [f64; 3], tol: f64) {
    for k in 0..3 {
        let got = a[k].as_f64().unwrap();
        assert!((got - want[k]).abs() <= tol, "{a} vs {want:?}");
    }
}

// ------------------------------------------------------------------ rows

#[test]
fn occurrence_rows_carry_members() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let hinge = public(&f.doc, EntityRef::Instance(f.hinge));
    let (panel, plate, pin) = (
        obj(&f.doc, f.panel),
        obj(&f.doc, f.plate),
        obj(&f.doc, f.pin),
    );
    let r = rows(&mut f.doc, json!({}));
    assert_eq!(
        r[&door]["members"],
        json!({ "objects": [panel], "groups": [], "instances": [hinge] })
    );
    assert_eq!(r[&hinge]["members"]["objects"], json!([plate, pin]));
    assert_eq!(
        r[&panel]["members"],
        json!({ "objects": [], "groups": [], "instances": [] })
    );
    f.doc.set_node_user_hidden(NodeId::Object(f.pin), true);
    let r = rows(&mut f.doc, json!({}));
    assert_eq!(r[&hinge]["members"]["objects"], json!([plate]));
    let r = rows(&mut f.doc, json!({ "include_hidden": true }));
    assert_eq!(r[&hinge]["members"]["objects"], json!([plate, pin]));
}

#[test]
fn object_row_reports_solid_family_and_family_closed() {
    let mut f = nested_dowel();
    let health = ok(&mut f.doc, "hew.doc.health", json!({}));
    let r = rows(&mut f.doc, json!({}));
    let panel = &r[&obj(&f.doc, f.panel)];
    assert_eq!(panel["solid"], true);
    assert_eq!(panel["family"], Value::Null);
    assert_eq!(panel["family_closed"], Value::Null);
    assert_eq!(r[&obj(&f.doc, f.open_box)]["solid"], false);
    let (left, right) = (&r[&obj(&f.doc, f.left)], &r[&obj(&f.doc, f.right)]);
    assert_eq!(left["solid"], false);
    assert!(left["family"].is_u64());
    assert_eq!(left["family"], right["family"]);
    let family = left["family"].as_u64().unwrap() as usize;
    let members = &health["families"][family]["members"];
    assert!(
        members
            .as_array()
            .unwrap()
            .contains(&json!(obj(&f.doc, f.left)))
    );
    assert_eq!(left["family_closed"], true);
    assert_eq!(right["family_closed"], true);

    // A warped seam cannot take a cap: the family is not closed.
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![
            half("Left", 0.0, 1.0, &[3], 0.1),
            half("Right", 1.0, 2.0, &[5], 0.1),
        ],
    );
    let r = rows(&mut doc, json!({}));
    let left = &r[&obj(&doc, by_name(&doc, "Left"))];
    assert!(left["family"].is_u64(), "{left}");
    assert_eq!(left["family_closed"], false);
}

#[test]
fn solid_agrees_with_the_health_verdict() {
    let mut f = nested_dowel();
    let health = ok(&mut f.doc, "hew.doc.health", json!({}));
    let open: Vec<&str> = health["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["verdict"]["kind"] != "solid")
        .map(|row| row["object"].as_str().unwrap())
        .collect();
    let r = rows(&mut f.doc, json!({ "include_hidden": true }));
    let mut objects = 0;
    for row in r.values().filter(|row| row["kind"] == "object") {
        objects += 1;
        let id = row["id"].as_str().unwrap();
        assert_eq!(row["solid"], !open.contains(&id), "{id}");
        let o = api::ids::IdResolver::new(&f.doc).resolve(id).unwrap();
        let EntityRef::Object(o) = o else { panic!() };
        let verdict = kernel::health::diagnose(f.doc.object(o).unwrap()).verdict;
        assert_eq!(
            row["solid"],
            verdict == kernel::health::Verdict::Solid,
            "{id}"
        );
    }
    assert_eq!(objects, 7);
}

#[test]
fn instance_row_reports_linked() {
    let mut f = nested_dowel();
    let hinge = public(&f.doc, EntityRef::Instance(f.hinge));
    let r = rows(&mut f.doc, json!({}));
    assert_eq!(r[&hinge]["linked"], false);
    let def = f.doc.instance_def(f.hinge).unwrap();
    f.doc
        .attr_set(
            AttrTarget::Entity(EntityRef::Component(def)),
            kernel::HEW_ATTR_NS,
            kernel::LINKED_ATTR_KEY,
            AttrValue::Bool(true),
        )
        .unwrap();
    let r = rows(&mut f.doc, json!({}));
    assert_eq!(r[&hinge]["linked"], true);
    for row in r.values().filter(|row| row["kind"] != "instance") {
        assert!(row.get("linked").is_none(), "{row}");
    }
}

#[test]
fn group_and_instance_rows_carry_no_object_facts() {
    let mut f = nested_dowel();
    let r = rows(&mut f.doc, json!({}));
    for row in r.values().filter(|row| row["kind"] != "object") {
        for key in ["solid", "family", "family_closed"] {
            assert!(row.get(key).is_none(), "{key} on {row}");
        }
        assert!(row["members"].is_object());
    }
    // `hew.query.resolve`'s occurrence keeps its M1 shape.
    let door = public(&f.doc, EntityRef::Group(f.door));
    let resolved = ok(
        &mut f.doc,
        "hew.query.resolve",
        json!({ "occurrence": { "path": [door] } }),
    );
    assert!(resolved["occurrence"].get("members").is_none());
}

// ---------------------------------------------------------------- scope

#[test]
fn scope_defaults_to_all_and_unknown_scope_is_invalid_params() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let a = ok(&mut f.doc, "hew.query.part_frame", json!({ "id": door }));
    let b = ok(
        &mut f.doc,
        "hew.query.part_frame",
        json!({ "id": door, "scope": "all" }),
    );
    assert_eq!(a["extents"], b["extents"]);
    for (method, params) in [
        (
            "hew.query.part_frame",
            json!({ "id": door, "scope": "mine" }),
        ),
        ("hew.query.measure", json!({ "id": door, "scope": "mine" })),
        ("hew.query.measure", json!({ "id": door, "scope": 1 })),
        (
            "hew.query.measure",
            json!({ "from": [0.0, 0.0, 0.0], "to": [1.0, 0.0, 0.0], "scope": "own" }),
        ),
        ("hew.query.mesh", json!({ "id": door, "scope": "own" })),
        ("hew.query.topology", json!({ "id": door, "scope": "own" })),
        ("hew.view.snapshot", json!({ "scope": "own" })),
    ] {
        params_error(&mut f.doc, &mut NoHost, method, params);
    }
}

#[test]
fn part_frame_scope_own_ignores_nested_instance_members() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let own = ok(
        &mut f.doc,
        "hew.query.part_frame",
        json!({ "id": door, "scope": "own" }),
    );
    close(&own["extents"], [0.4, 0.2, 0.018], 1e-9);
    let all = ok(&mut f.doc, "hew.query.part_frame", json!({ "id": door }));
    assert_ne!(all["extents"], own["extents"], "the hinge enlarges the box");
}

#[test]
fn part_frame_scope_own_on_an_object_equals_all() {
    let mut f = nested_dowel();
    let shelf = obj(&f.doc, f.shelf);
    let own = ok(
        &mut f.doc,
        "hew.query.part_frame",
        json!({ "id": shelf, "scope": "own" }),
    );
    let all = ok(&mut f.doc, "hew.query.part_frame", json!({ "id": shelf }));
    assert_eq!(own, all);
}

#[test]
fn part_frame_scope_own_on_an_instance_measures_its_direct_objects() {
    let mut doc = Document::new();
    let plate = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.01]);
    let pin = cuboid(&mut doc, [0.0, 0.0, 0.01], [0.01, 0.01, 0.5]);
    let (_, inner, _) = doc.make_component(&[NodeId::Object(pin)]).unwrap();
    let (_, outer, _) = doc
        .make_component(&[NodeId::Object(plate), NodeId::Instance(inner)])
        .unwrap();
    let id = public(&doc, EntityRef::Instance(outer));
    let own = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": id, "scope": "own" }),
    );
    assert_eq!(own["source"], "definition");
    close(&own["extents"], [0.4, 0.2, 0.01], 1e-9);
    let all = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
    close(&all["extents"], [0.4, 0.2, 0.51], 1e-9);
}

fn set_frame(doc: &mut Document, node: NodeId) {
    let list = |v: [f64; 3]| AttrValue::List(v.iter().map(|&c| AttrValue::Float(c)).collect());
    let mut m = std::collections::BTreeMap::new();
    m.insert("origin".to_string(), list([0.0, 0.0, 0.0]));
    m.insert("x".to_string(), list([0.0, 1.0, 0.0]));
    m.insert("y".to_string(), list([1.0, 0.0, 0.0]));
    let entity = match node {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    };
    doc.attr_set(
        AttrTarget::Entity(entity),
        api::part_frame::FRAME_NAMESPACE,
        api::part_frame::FRAME_KEY,
        AttrValue::Map(m),
    )
    .unwrap();
}

#[test]
fn part_frame_through_a_placement_honors_a_member_override() {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    set_frame(&mut doc, NodeId::Object(board));
    let board_id = obj(&doc, board);
    let before = ok(&mut doc, "hew.query.part_frame", json!({ "id": board_id }));
    assert_eq!(before["source"], "attribute");
    let (_, inst, _) = doc.wrap_linked(NodeId::Object(board)).unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let after = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
    assert_eq!(after["source"], "attribute");
    for k in ["origin", "x", "y", "z", "extents"] {
        assert_eq!(after[k], before[k], "{k}");
    }
    // A second placement, a quarter turn away, reports the same frame
    // carried through its pose.
    let def = doc.instance_def(inst).unwrap();
    let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2)
        .unwrap()
        .then(&Transform::translation(Vec3::new(2.0, 0.0, 0.0)));
    let (second, _) = doc.place_instance(def, turn).unwrap();
    let id = public(&doc, EntityRef::Instance(second));
    let other = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
    assert_eq!(other["source"], "attribute");
    close(
        &other["extents"],
        [
            before["extents"][0].as_f64().unwrap(),
            before["extents"][1].as_f64().unwrap(),
            before["extents"][2].as_f64().unwrap(),
        ],
        1e-9,
    );
    let x = turn.apply_vector(Vec3::new(
        before["x"][0].as_f64().unwrap(),
        before["x"][1].as_f64().unwrap(),
        before["x"][2].as_f64().unwrap(),
    ));
    close(&other["x"], [x.x, x.y, x.z], 1e-9);
}

#[test]
fn part_frame_through_a_placement_without_an_override_is_unchanged() {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let (_, inst, _) = doc.wrap_linked(NodeId::Object(board)).unwrap();
    let id = public(&doc, EntityRef::Instance(inst));
    let f = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
    assert_eq!(f["source"], "definition");
    close(&f["x"], [1.0, 0.0, 0.0], 1e-12);
    close(&f["extents"], [0.4, 0.2, 0.018], 1e-12);
}

#[test]
fn part_frame_scope_own_with_no_own_leaves_refuses_no_part_frame() {
    let mut f = nested_dowel();
    // A group holding only a placement of the hinge.
    let def = f.doc.instance_def(f.hinge).unwrap();
    let (i, _) = f.doc.place_instance(def, Transform::IDENTITY).unwrap();
    let (holder, _) = f.doc.group_nodes(&[NodeId::Instance(i)]).unwrap();
    let id = public(&f.doc, EntityRef::Group(holder));
    assert_eq!(
        refusal(
            &mut f.doc,
            "hew.query.part_frame",
            json!({ "id": id, "scope": "own" })
        ),
        "no_part_frame"
    );
    let r = call(
        &mut f.doc,
        &mut NoHost,
        "hew.query.measure",
        json!({ "id": id, "scope": "own" }),
    );
    assert_eq!(r["error"]["code"], -32602);
    assert!(ok(&mut f.doc, "hew.query.part_frame", json!({ "id": id }))["extents"].is_array());
}

#[test]
fn measure_scope_own_sums_direct_objects_only() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let own = ok(
        &mut f.doc,
        "hew.query.measure",
        json!({ "id": door, "scope": "own" }),
    );
    assert!((own["volume"].as_f64().unwrap() - 0.00144).abs() < 1e-12);
    assert_eq!(own["leaf_count"], 1);
    let all = ok(
        &mut f.doc,
        "hew.query.measure",
        json!({ "id": door, "scope": "all" }),
    );
    assert!((all["volume"].as_f64().unwrap() - 0.0014454).abs() < 1e-12);
    assert_eq!(all["leaf_count"], 3);
}

/// A host that records what the commands asked it to render.
#[derive(Default)]
struct Recording {
    snapshot: Option<SnapshotParams>,
    line_drawing: Option<LineDrawingParams>,
}

impl Host for Recording {
    fn snapshot(&mut self, _doc: &Document, p: &SnapshotParams) -> Result<SnapshotResult, Refusal> {
        self.snapshot = Some(p.clone());
        Ok(SnapshotResult {
            png: Vec::new(),
            width: p.width,
            height: p.height,
            id_buffer: None,
            id_palette: Vec::new(),
            depth: None,
            normals_png: None,
        })
    }
    fn line_drawing(
        &mut self,
        _doc: &Document,
        p: &LineDrawingParams,
    ) -> Result<LineDrawingResult, Refusal> {
        self.line_drawing = Some(p.clone());
        Ok(LineDrawingResult {
            svg: Some(String::new()),
            segments: Vec::new(),
            kinds: Vec::new(),
            ids: Vec::new(),
            bounds: None,
            count: 0,
        })
    }
}

/// The objects the host was asked to draw, as Object occurrence roots.
fn drawn(selection: &[api::locate::RenderRoot]) -> Vec<NodeId> {
    selection
        .iter()
        .map(|r| match r {
            api::locate::RenderRoot::Occurrence(o) => o.node,
            api::locate::RenderRoot::Definition(_) => panic!("a definition root"),
        })
        .collect()
}

#[test]
fn snapshot_scope_own_renders_only_own_leaves() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let mut host = Recording::default();
    let r = call(
        &mut f.doc,
        &mut host,
        "hew.view.snapshot",
        json!({ "selection": [door], "scope": "own" }),
    );
    assert!(r.get("error").is_none(), "{r}");
    let sel = host.snapshot.take().unwrap().selection.unwrap();
    assert_eq!(drawn(&sel), vec![NodeId::Object(f.panel)]);
    call(
        &mut f.doc,
        &mut host,
        "hew.view.snapshot",
        json!({ "selection": [door] }),
    );
    let sel = host.snapshot.take().unwrap().selection.unwrap();
    assert_eq!(sel.len(), 1, "scope all keeps the root as given");
}

#[test]
fn line_drawing_scope_own_draws_only_own_leaves() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    let mut host = Recording::default();
    let r = call(
        &mut f.doc,
        &mut host,
        "hew.view.line_drawing",
        json!({ "selection": [door], "scope": "own", "format": "segments" }),
    );
    assert!(r.get("error").is_none(), "{r}");
    let sel = host.line_drawing.take().unwrap().selection.unwrap();
    assert_eq!(drawn(&sel), vec![NodeId::Object(f.panel)]);
    params_error(
        &mut f.doc,
        &mut host,
        "hew.view.line_drawing",
        json!({ "scope": "own" }),
    );
}

#[test]
fn snapshot_orient_part_requires_one_root() {
    let mut f = nested_dowel();
    let (panel, shelf) = (obj(&f.doc, f.panel), obj(&f.doc, f.shelf));
    let mut host = Recording::default();
    for params in [
        json!({ "orient": "part" }),
        json!({ "orient": "part", "selection": [shelf.clone(), panel] }),
        json!({ "orient": "sideways", "selection": [shelf] }),
    ] {
        params_error(&mut f.doc, &mut host, "hew.view.snapshot", params);
    }
}

#[test]
fn snapshot_orient_part_refuses_explicit_camera() {
    let mut f = nested_dowel();
    let shelf = obj(&f.doc, f.shelf);
    params_error(
        &mut f.doc,
        &mut Recording::default(),
        "hew.view.snapshot",
        json!({
            "orient": "part",
            "selection": [shelf],
            "camera": { "eye": [1.0, 1.0, 1.0], "target": [0.0, 0.0, 0.0] }
        }),
    );
}

/// `front` in the part frame looks along the frame's +y: the camera the
/// host is handed turns with the board, so the picture does not.
#[test]
fn snapshot_orient_part_front_looks_along_the_frame_y() {
    let mut cams = Vec::new();
    for yaw in [0.0_f64, 0.7, 2.1] {
        let mut doc = Document::new();
        let b = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
        let turn = Transform::rotation(Vec3::new(0.0, 0.0, 1.0), yaw).unwrap();
        doc.transform_object(b, &turn).unwrap();
        let id = obj(&doc, b);
        let frame = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
        let mut host = Recording::default();
        let r = call(
            &mut doc,
            &mut host,
            "hew.view.snapshot",
            json!({ "selection": [id], "orient": "part", "view": "front" }),
        );
        assert!(r.get("error").is_none(), "{r}");
        let p = host.snapshot.unwrap();
        assert!(p.view.is_none());
        let cam = p.camera.unwrap();
        let dir = Vec3::new(
            cam.target[0] - cam.eye[0],
            cam.target[1] - cam.eye[1],
            cam.target[2] - cam.eye[2],
        )
        .normalized()
        .unwrap();
        let fy = Vec3::new(
            frame["y"][0].as_f64().unwrap(),
            frame["y"][1].as_f64().unwrap(),
            frame["y"][2].as_f64().unwrap(),
        );
        assert!((dir - fy).length() < 1e-9, "front looks along +y");
        // The camera in the board's own coordinates is the same at every yaw.
        let inv = turn.inverse().unwrap();
        let eye = inv.apply_point(Point3::new(cam.eye[0], cam.eye[1], cam.eye[2]));
        cams.push(eye);
    }
    for e in &cams[1..] {
        assert!((*e - cams[0]).length() < 1e-9, "{e:?} vs {:?}", cams[0]);
    }
}

// ------------------------------------------------------------ signature

fn signature(doc: &mut Document, params: Value) -> Value {
    ok(doc, "hew.query.signature", params)
}

fn sig_of(doc: &mut Document, o: ObjectId) -> Value {
    let id = obj(doc, o);
    signature(doc, json!({ "id": id }))
}

/// Three arms of different lengths from one corner, along +x, +y, +z
/// (`sx` = -1 mirrors it in x): a chiral part.
fn tripod(doc: &mut Document, sx: f64, at: f64) -> ObjectId {
    let x0 = if sx > 0.0 { at } else { at - 0.5 };
    let arm_x = cuboid(doc, [x0, 0.0, 0.0], [0.5, 0.1, 0.1]);
    let bx = if sx > 0.0 { at } else { at - 0.1 };
    let arm_y = cuboid(doc, [bx, 0.1, 0.0], [0.1, 0.3, 0.1]);
    let arm_z = cuboid(doc, [bx, 0.0, 0.1], [0.1, 0.1, 0.2]);
    let (xy, _) = doc
        .boolean_nodes(
            kernel::BooleanOp::Union,
            NodeId::Object(arm_x),
            NodeId::Object(arm_y),
        )
        .unwrap();
    let (all, _) = doc
        .boolean_nodes(kernel::BooleanOp::Union, xy, NodeId::Object(arm_z))
        .unwrap();
    let NodeId::Object(o) = all else {
        panic!("one solid")
    };
    o
}

#[test]
fn signature_of_identical_boards_matches() {
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let b = cuboid(&mut doc, [3.0, 1.0, 0.0], [0.4, 0.2, 0.018]);
    let turn = Transform::rotation(Vec3::new(0.3, 1.0, 0.2), 1.1)
        .unwrap()
        .then(&Transform::translation(Vec3::new(5.0, -2.0, 7.0)));
    doc.transform_object(b, &turn).unwrap();
    let (sa, sb) = (sig_of(&mut doc, a), sig_of(&mut doc, b));
    assert_eq!(sa["shape"], sb["shape"]);
    assert_eq!(sa["mirror"], sb["mirror"]);
    assert_eq!(sa["shape"].as_str().unwrap().len(), 64);
    let c = cuboid(&mut doc, [9.0, 0.0, 0.0], [0.4, 0.21, 0.018]);
    assert_ne!(sig_of(&mut doc, c)["shape"], sa["shape"]);
}

#[test]
fn signature_of_a_mirrored_pair_swaps_shape_and_mirror() {
    let mut doc = Document::new();
    let a = tripod(&mut doc, 1.0, 0.0);
    let b = tripod(&mut doc, -1.0, 5.0);
    let (sa, sb) = (sig_of(&mut doc, a), sig_of(&mut doc, b));
    assert_eq!(sa["shape"], sb["mirror"]);
    assert_eq!(sa["mirror"], sb["shape"]);
    assert_eq!(sa["symmetric"], false);
    assert_ne!(sa["shape"], sb["shape"]);
}

#[test]
fn signature_of_a_symmetric_board_is_symmetric() {
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.4, 0.2, 0.018]);
    let s = sig_of(&mut doc, a);
    assert_eq!(s["symmetric"], true);
    assert_eq!(s["shape"], s["mirror"]);
}

#[test]
fn signature_is_invariant_under_a_save_load_round_trip() {
    let mut doc = Document::new();
    let a = tripod(&mut doc, 1.0, 0.0);
    let before = sig_of(&mut doc, a);
    let mut re = Document::load(&doc.save()).unwrap();
    let id = re.visible_object_ids()[0];
    let after = sig_of(&mut re, id);
    assert_eq!(before["shape"], after["shape"]);
    assert_eq!(before["mirror"], after["mirror"]);
}

#[test]
fn signature_ties_choose_the_smallest_hash() {
    // A square-section stick: the tie in extents adds the quarter turns
    // about its length, so turning it a quarter turn changes nothing.
    let mut doc = Document::new();
    let a = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.5, 0.05, 0.05]);
    let b = cuboid(&mut doc, [0.0, 1.0, 0.0], [0.5, 0.05, 0.05]);
    let turn = Transform::rotation(Vec3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2).unwrap();
    doc.transform_object(b, &turn).unwrap();
    let (sa, sb) = (sig_of(&mut doc, a), sig_of(&mut doc, b));
    assert_eq!(sa["shape"], sb["shape"]);
    // A notch on one long face breaks the square symmetry; the answer is
    // still a minimum over every candidate, so it is pose-independent.
    let n1 = extrude_poly(
        &mut doc,
        &[
            (0.0, 3.0),
            (0.5, 3.0),
            (0.5, 3.05),
            (0.3, 3.05),
            (0.3, 3.04),
            (0.2, 3.04),
            (0.2, 3.05),
            (0.0, 3.05),
        ],
        0.0,
        0.05,
    );
    let n2 = extrude_poly(
        &mut doc,
        &[
            (0.0, 5.0),
            (0.5, 5.0),
            (0.5, 5.05),
            (0.3, 5.05),
            (0.3, 5.04),
            (0.2, 5.04),
            (0.2, 5.05),
            (0.0, 5.05),
        ],
        0.0,
        0.05,
    );
    doc.transform_object(n2, &turn).unwrap();
    assert_eq!(sig_of(&mut doc, n1)["shape"], sig_of(&mut doc, n2)["shape"]);
}

#[test]
fn signature_refuses_a_group_and_an_instance_with_not_an_object() {
    let mut f = nested_dowel();
    let door = public(&f.doc, EntityRef::Group(f.door));
    assert_eq!(
        refusal(&mut f.doc, "hew.query.signature", json!({ "id": door })),
        "not_an_object"
    );
    let hinge = public(&f.doc, EntityRef::Instance(f.hinge));
    assert_eq!(
        refusal(&mut f.doc, "hew.query.signature", json!({ "id": hinge })),
        "not_an_object"
    );
}

#[test]
fn signature_frame_equals_part_frame() {
    let mut doc = Document::new();
    let a = tripod(&mut doc, 1.0, 0.0);
    let turn = Transform::rotation(Vec3::new(1.0, 2.0, 0.5), 0.8).unwrap();
    doc.transform_object(a, &turn).unwrap();
    let id = obj(&doc, a);
    let s = sig_of(&mut doc, a);
    let f = ok(&mut doc, "hew.query.part_frame", json!({ "id": id }));
    for k in ["origin", "x", "y", "z", "extents"] {
        assert_eq!(s["frame"][k], f[k], "{k}");
    }
}

#[test]
fn signature_by_path_reaches_a_definition_member() {
    let mut f = nested_dowel();
    let plate_alone = {
        let mut doc = Document::new();
        let p = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.04, 0.03, 0.002]);
        sig_of(&mut doc, p)
    };
    let hinge = public(&f.doc, EntityRef::Instance(f.hinge));
    let plate = obj(&f.doc, f.plate);
    let s = signature(&mut f.doc, json!({ "path": [hinge, plate] }));
    assert_eq!(s["shape"], plate_alone["shape"]);
    assert!(s["generation"].is_u64(), "the read carries the clock");
}

#[test]
fn registry_declares_signature_readonly_with_modelread() {
    let reg = api::Registry::protocol_1();
    let c = reg.get("hew.query.signature").unwrap();
    assert_eq!(c.class, api::CommandClass::ReadOnly);
    assert_eq!(c.permission, api::Permission::ModelRead);
    assert!(c.implemented && !c.mutates_document);
    for r in [
        "unknown_entity",
        "unknown_occurrence",
        "not_an_object",
        "no_part_frame",
        "too_many_occurrences",
    ] {
        assert!(c.refusals.iter().any(|x| x == r), "{r}");
    }
}

/// A star-shaped polygon: `radii` at evenly spaced angles.
fn star(radii: &[f64]) -> Vec<(f64, f64)> {
    let n = radii.len() as f64;
    radii
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let t = std::f64::consts::TAU * k as f64 / n;
            (r * t.cos(), r * t.sin())
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn signature_shape_is_invariant_under_random_rigid_motion_and_mirror_swaps_it(
        radii in prop::collection::vec(0.05f64..0.5, 5..9),
        h in 0.01f64..0.3,
        axis in (-1.0f64..1.0, -1.0f64..1.0, 0.1f64..1.0),
        angle in 0.0f64..6.2,
        shift in (-5.0f64..5.0, -5.0f64..5.0, -5.0f64..5.0),
    ) {
        let pts = star(&radii);
        let mut doc = Document::new();
        let a = extrude_poly(&mut doc, &pts, 0.0, h);
        let b = extrude_poly(&mut doc, &pts, 0.0, h);
        let motion = Transform::rotation(Vec3::new(axis.0, axis.1, axis.2), angle)
            .unwrap()
            .then(&Transform::translation(Vec3::new(shift.0, shift.1, shift.2)));
        doc.transform_object(b, &motion).unwrap();
        // The mirror image: x negated, the loop reversed to stay
        // counter-clockwise.
        let mirrored: Vec<(f64, f64)> = pts.iter().rev().map(|&(x, y)| (-x, y)).collect();
        let m = extrude_poly(&mut doc, &mirrored, 0.0, h);
        let (sa, sb, sm) = (sig_of(&mut doc, a), sig_of(&mut doc, b), sig_of(&mut doc, m));
        prop_assert_eq!(&sa["shape"], &sb["shape"]);
        prop_assert_eq!(&sa["mirror"], &sb["mirror"]);
        prop_assert_eq!(&sa["shape"], &sm["mirror"]);
        prop_assert_eq!(&sa["mirror"], &sm["shape"]);
    }
}

// ---------------------------------------------------------------- rejoin

/// Left (1 wide), Right (2 wide, the larger), and the flat wall between
/// them, one seam family.
fn split_box() -> Document {
    let mut doc = Document::new();
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let mut wall = half("Wall", 0.0, 1.0, &[], 0.0);
    wall.positions = vec![
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(1.0, 1.0, 1.0),
        p(1.0, 0.0, 1.0),
    ];
    wall.faces = vec![vec![0, 1, 2, 3]];
    wall.face_materials.truncate(1);
    wall.face_uv_frames.truncate(1);
    wall.face_holes.truncate(1);
    ingest(
        &mut doc,
        vec![
            half("Left", 0.0, 1.0, &[3], 0.0),
            half("Right", 1.0, 3.0, &[5], 0.0),
            wall,
        ],
    );
    doc
}

#[test]
fn rejoin_joins_a_closed_family_into_one_solid() {
    let mut doc = split_box();
    let depth = doc.undo_depth();
    let r = ok(
        &mut doc,
        "hew.solid.rejoin",
        json!({ "family": 0, "as": "one_solid" }),
    );
    let joined = r["results"][0]["object"].as_str().unwrap().to_string();
    assert_eq!(
        doc.live_object_ids().len(),
        1,
        "the seam pair is one Object"
    );
    let o = doc.live_object_ids()[0];
    assert_eq!(obj(&doc, o), joined);
    assert!(doc.health_report(None).rows.is_empty(), "nothing is open");
    assert_eq!(doc.undo_depth(), depth + 1, "one undo entry");
    // Closed, and exactly the two pieces' volume (1 + 2): nothing lost to
    // the deleted wall, nothing doubled.
    let volume = doc
        .object(o)
        .unwrap()
        .volume()
        .expect("the result is closed");
    assert!((volume - 3.0).abs() < 1e-9, "volume {volume}");
    assert_eq!(doc.object_name(o), Some("Right"), "the larger piece's name");
    doc.undo().unwrap();
    assert_eq!(doc.live_object_ids().len(), 3);
}

#[test]
fn rejoin_refuses_unknown_family() {
    let mut doc = split_box();
    let bytes = doc.save();
    assert_eq!(
        refusal(
            &mut doc,
            "hew.solid.rejoin",
            json!({ "family": 7, "as": "one_solid" })
        ),
        "unknown_family"
    );
    assert_eq!(doc.save(), bytes);
    for bad in [
        json!({ "family": 0 }),
        json!({ "family": 0, "as": "two_solids" }),
        json!({ "family": 0, "as": "one_solid", "extra": 1 }),
    ] {
        params_error(&mut doc, &mut NoHost, "hew.solid.rejoin", bad);
        assert_eq!(doc.save(), bytes, "a params error changes nothing");
    }
}

#[test]
fn rejoin_refuses_a_family_that_is_not_closed() {
    let mut doc = Document::new();
    ingest(
        &mut doc,
        vec![
            half("Left", 0.0, 1.0, &[3], 0.1),
            half("Right", 1.0, 2.0, &[5], 0.1),
        ],
    );
    let bytes = doc.save();
    assert_eq!(
        refusal(
            &mut doc,
            "hew.solid.rejoin",
            json!({ "family": 0, "as": "one_solid" })
        ),
        "not_cappable"
    );
    assert_eq!(doc.save(), bytes);
}

/// One open piece and the flat wall that closes it: a family of one
/// cell, which the kernel refuses as `rejoin_not_possible` (the family
/// the handler passes is whole, so the kernel's structural refusals are
/// what a bus caller meets).
#[test]
fn rejoin_surfaces_the_kernels_refusal_for_a_family_of_one_piece() {
    let mut doc = Document::new();
    let mut wall = half("Wall", 0.0, 1.0, &[], 0.0);
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    wall.positions = vec![
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(1.0, 1.0, 1.0),
        p(1.0, 0.0, 1.0),
    ];
    wall.faces = vec![vec![0, 1, 2, 3]];
    wall.face_materials.truncate(1);
    wall.face_uv_frames.truncate(1);
    wall.face_holes.truncate(1);
    ingest(&mut doc, vec![half("Left", 0.0, 1.0, &[3], 0.0), wall]);
    let bytes = doc.save();
    assert_eq!(
        refusal(
            &mut doc,
            "hew.solid.rejoin",
            json!({ "family": 0, "as": "one_solid" })
        ),
        "rejoin_not_possible"
    );
    assert_eq!(doc.save(), bytes);
}

/// `family` is numbered over the whole-model report, whatever scope a
/// `hew.doc.health` call was given.
#[test]
fn rejoin_family_is_numbered_over_the_whole_model() {
    let mut doc = split_box();
    // A second, separate family far away, so a scoped report can renumber.
    ingest(
        &mut doc,
        vec![
            half("FarLeft", 10.0, 11.0, &[3], 0.0),
            half("FarRight", 11.0, 12.0, &[5], 0.0),
        ],
    );
    let whole = doc.health_report(None).families;
    assert_eq!(whole.len(), 2);
    let far = by_name(&doc, "FarLeft");
    let scoped = doc
        .health_report(Some(&[far, by_name(&doc, "FarRight")]))
        .families;
    assert_eq!(
        scoped.len(),
        1,
        "the scoped report renumbers its family to 0"
    );
    // Index 0 joins the whole report's family 0 (Left/Right/Wall), not the
    // scoped one.
    ok(
        &mut doc,
        "hew.solid.rejoin",
        json!({ "family": 0, "as": "one_solid" }),
    );
    assert!(
        doc.live_object_ids().contains(&far),
        "the far pair is untouched"
    );
}

/// The refusals the registry lists for `hew.solid.rejoin` are the codes the
/// command emits, and each is a name the bus can return.
#[test]
fn rejoin_listed_refusals_include_what_it_returns() {
    let listed = &REGISTRY.get("hew.solid.rejoin").unwrap().refusals;
    for code in [
        "unknown_family",
        "not_cappable",
        "rejoin_not_possible",
        "degenerate_contact",
        "boolean_operand_not_solid",
        "boolean_operand_empty",
        "invalid",
    ] {
        assert!(listed.iter().any(|r| r == code), "{code} is listed");
    }
    let mut doc = split_box();
    let mut seen = Vec::new();
    seen.push(refusal(
        &mut doc,
        "hew.solid.rejoin",
        json!({ "family": 9, "as": "one_solid" }),
    ));
    for code in &seen {
        assert!(listed.iter().any(|r| r == code), "{code} is listed");
    }
}

#[test]
fn rejoin_label_and_class_match_union() {
    let (rejoin, union) = (
        REGISTRY.get("hew.solid.rejoin").unwrap(),
        REGISTRY.get("hew.solid.union").unwrap(),
    );
    assert_eq!(rejoin.class, union.class);
    assert_eq!(rejoin.served, union.served);
    assert_eq!(rejoin.permission, union.permission);
    assert!(rejoin.implemented);
    let mut doc = split_box();
    let r = ok(
        &mut doc,
        "hew.solid.rejoin",
        json!({ "family": 0, "as": "one_solid" }),
    );
    assert_eq!(r["label"], "hew.solid.rejoin");
}

/// A group of a 0.7 x 0.45 x 0.018 board and a 0.3 x 0.02 x 0.036 machining
/// body that sticks out of it; the group's public id, the board's, the
/// machining body's.
fn machined_board() -> (Document, String, String, String) {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.7, 0.45, 0.018]);
    let cut = cuboid(&mut doc, [0.2, 0.2, -0.009], [0.3, 0.02, 0.036]);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(board), NodeId::Object(cut)])
        .unwrap();
    let ids = (
        public(&doc, EntityRef::Group(g)),
        public(&doc, EntityRef::Object(board)),
        public(&doc, EntityRef::Object(cut)),
    );
    (doc, ids.0, ids.1, ids.2)
}

#[test]
fn part_frame_exclude_leaves_the_named_own_objects_out() {
    let (mut doc, g, board, cut) = machined_board();
    let with = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": g, "scope": "own" }),
    );
    assert!((with["extents"][2].as_f64().unwrap() - 0.018).abs() > 1e-6);
    let without = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": g, "scope": "own", "exclude": [cut] }),
    );
    close(&without["extents"], [0.7, 0.45, 0.018], 1e-9);
    // An empty list is no exclusion.
    let empty = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": g, "scope": "own", "exclude": [] }),
    );
    assert_eq!(empty, with);
    // Excluding every own Object leaves nothing to measure.
    assert_eq!(
        refusal(
            &mut doc,
            "hew.query.part_frame",
            json!({ "id": g, "scope": "own", "exclude": [board, cut] })
        ),
        "no_part_frame"
    );
}

#[test]
fn part_frame_exclude_is_invalid_without_own_or_for_a_stranger() {
    let (mut doc, g, board, cut) = machined_board();
    for params in [
        json!({ "id": g, "exclude": [cut.clone()] }),
        json!({ "id": g, "scope": "all", "exclude": [cut.clone()] }),
        json!({ "id": g, "scope": "own", "exclude": [g.clone()] }),
        json!({ "id": g, "scope": "own", "exclude": ["obj_nope"] }),
        json!({ "id": board, "scope": "own", "exclude": [cut.clone()] }),
    ] {
        for cmd in ["hew.query.part_frame", "hew.query.measure"] {
            let r = call(&mut doc, &mut NoHost, cmd, params.clone());
            assert_eq!(r["error"]["code"], -32602, "{cmd} {params}");
        }
    }
    // The buffer reads and the signature take no exclude.
    for cmd in [
        "hew.query.mesh",
        "hew.query.topology",
        "hew.query.signature",
    ] {
        let r = call(
            &mut doc,
            &mut NoHost,
            cmd,
            json!({ "id": board, "exclude": [cut.clone()] }),
        );
        assert_eq!(r["error"]["code"], -32602, "{cmd}");
    }
}

#[test]
fn measure_exclude_leaves_the_named_own_objects_out() {
    let (mut doc, g, board, cut) = machined_board();
    let with = ok(
        &mut doc,
        "hew.query.measure",
        json!({ "id": g, "scope": "own" }),
    );
    assert_eq!(with["leaf_count"], 2);
    let without = ok(
        &mut doc,
        "hew.query.measure",
        json!({ "id": g, "scope": "own", "exclude": [cut.clone()] }),
    );
    assert_eq!(without["leaf_count"], 1);
    assert!((without["volume"].as_f64().unwrap() - 0.7 * 0.45 * 0.018).abs() < 1e-9);
    close(&without["extents"], [0.7, 0.45, 0.018], 1e-9);
    let r = call(
        &mut doc,
        &mut NoHost,
        "hew.query.measure",
        json!({ "id": g, "scope": "own", "exclude": [board, cut] }),
    );
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn exclude_on_an_instance_leaves_a_definition_member_out() {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.7, 0.45, 0.018]);
    let cut = cuboid(&mut doc, [0.2, 0.2, -0.009], [0.3, 0.02, 0.036]);
    let (_, inst, _) = doc
        .make_component(&[NodeId::Object(board), NodeId::Object(cut)])
        .unwrap();
    let i = public(&doc, EntityRef::Instance(inst));
    let cut_id = public(&doc, EntityRef::Object(cut));
    let frame = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": i, "scope": "own", "exclude": [cut_id.clone()] }),
    );
    close(&frame["extents"], [0.7, 0.45, 0.018], 1e-9);
    let m = ok(
        &mut doc,
        "hew.query.measure",
        json!({ "id": i, "scope": "own", "exclude": [cut_id] }),
    );
    assert_eq!(m["leaf_count"], 1);
}

#[test]
fn exclude_by_path_ignores_duplicates_and_an_empty_list_needs_no_scope() {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.7, 0.45, 0.018]);
    let cut = cuboid(&mut doc, [0.2, 0.2, -0.009], [0.3, 0.02, 0.036]);
    let (_, inst, _) = doc
        .make_component(&[NodeId::Object(board), NodeId::Object(cut)])
        .unwrap();
    let i = public(&doc, EntityRef::Instance(inst));
    let cut_id = public(&doc, EntityRef::Object(cut));
    let f = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "path": [i.clone()], "scope": "own", "exclude": [cut_id.clone(), cut_id] }),
    );
    close(&f["extents"], [0.7, 0.45, 0.018], 1e-9);
    let plain = ok(
        &mut doc,
        "hew.query.measure",
        json!({ "id": i, "exclude": [] }),
    );
    assert_eq!(plain["leaf_count"], 2);
}

#[test]
fn exclude_with_a_member_override_on_a_group_and_an_instance() {
    let mut doc = Document::new();
    let board = cuboid(&mut doc, [0.0, 0.0, 0.0], [0.7, 0.45, 0.018]);
    let cut = cuboid(&mut doc, [0.2, 0.2, -0.009], [0.3, 0.02, 0.036]);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(board), NodeId::Object(cut)])
        .unwrap();
    set_frame(&mut doc, NodeId::Group(g));
    let gid = public(&doc, EntityRef::Group(g));
    let cut_id = public(&doc, EntityRef::Object(cut));
    let f = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": gid, "scope": "own", "exclude": [cut_id.clone()] }),
    );
    assert_eq!(f["source"], "attribute");
    let all = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": gid, "scope": "own" }),
    );
    assert_ne!(f["extents"], all["extents"]);
    // The same group as the sole member of a definition.
    let (_, inst, _) = doc.make_component(&[NodeId::Group(g)]).unwrap();
    let i = public(&doc, EntityRef::Instance(inst));
    let fi = ok(
        &mut doc,
        "hew.query.part_frame",
        json!({ "id": i, "scope": "own", "exclude": [cut_id] }),
    );
    assert_eq!(fi["extents"], f["extents"]);
}
