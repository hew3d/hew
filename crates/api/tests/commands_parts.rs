//! The buffers behind `hew.query.mesh` and `hew.query.topology`
//! (docs/agents/HEW_API.md §7.4): winding under plain and mirrored
//! placements, transformed normals, per-part offsets, face indices that
//! land on the faces topology lists, loop orientation, and the cap
//! refusals the part reads share.

use api::{Connection, DispatchOutcome, Grant, Host, NoHost, Refusal, Request, RequestId};
use kernel::{Document, EntityRef, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .unwrap()
}

fn extrude_box(doc: &mut Document, x0: f64, x1: f64, y1: f64, h: f64) -> ObjectId {
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).unwrap();
    let c = [
        Point3::new(x0, 0.0, 0.0),
        Point3::new(x1, 0.0, 0.0),
        Point3::new(x1, y1, 0.0),
        Point3::new(x0, y1, 0.0),
    ];
    for k in 0..4 {
        sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
    }
    let regions = doc.extrudable_regions(s).unwrap();
    doc.extrude_region(s, regions[0], h).unwrap().0
}

fn public(doc: &Document, e: EntityRef) -> String {
    api::ids::public_id(&e, doc.sid_of(&e).unwrap())
}

/// A host whose only effect is exporting through the real writers.
struct ExportHost;

impl Host for ExportHost {
    fn export_document(
        &mut self,
        doc: &Document,
        format: &str,
        _path: Option<&str>,
        segments_per_turn: u32,
        _units: Option<&str>,
        selection: Option<&[api::locate::RenderRoot]>,
        _compat: bool,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let bytes = match selection {
            Some(roots) => {
                mesh_export::export_occurrences(doc, roots, format, segments_per_turn, true)
            }
            None => mesh_export::export(doc, format, segments_per_turn, true),
        };
        bytes
            .map(Some)
            .map_err(|e| Refusal::api(e.name(), &e.message()))
    }
}

fn call(doc: &mut Document, host: &mut dyn Host, method: &str, params: Value) -> Value {
    let mut conn = Connection::new(Grant::core(), "parts");
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

fn decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        let n = chunk.iter().fold(0u32, |acc, &c| (acc << 6) | val(c));
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        out.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
    }
    out
}

fn f32s(v: &Value) -> Vec<f32> {
    decode(v.as_str().unwrap())
        .chunks(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn u32s(v: &Value) -> Vec<u32> {
    decode(v.as_str().unwrap())
        .chunks(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// The decoded mesh: positions, normals, indices, face indices.
struct Mesh {
    p: Vec<Vec3>,
    n: Vec<Vec3>,
    idx: Vec<u32>,
    faces: Vec<u32>,
    parts: Vec<Value>,
}

fn mesh(doc: &mut Document, params: Value) -> Mesh {
    let r = ok(doc, "hew.query.mesh", params);
    let v3 = |f: Vec<f32>| -> Vec<Vec3> {
        f.chunks(3)
            .map(|c| Vec3::new(f64::from(c[0]), f64::from(c[1]), f64::from(c[2])))
            .collect()
    };
    let m = Mesh {
        p: v3(f32s(&r["positions_base64"])),
        n: v3(f32s(&r["normals_base64"])),
        idx: u32s(&r["indices_base64"]),
        faces: u32s(&r["face_indices_base64"]),
        parts: r["parts"].as_array().unwrap().clone(),
    };
    assert_eq!(m.p.len(), r["vertex_count"].as_u64().unwrap() as usize);
    assert_eq!(
        m.idx.len() / 3,
        r["triangle_count"].as_u64().unwrap() as usize
    );
    assert_eq!(m.faces.len(), m.idx.len() / 3);
    m
}

/// Every triangle winds counter-clockwise from outside: its geometric
/// normal agrees with its vertex normals and points away from `inside`.
fn assert_outward(m: &Mesh, inside: Vec3) {
    for t in m.idx.chunks(3) {
        let (a, b, c) = (m.p[t[0] as usize], m.p[t[1] as usize], m.p[t[2] as usize]);
        let g = (b - a).cross(c - a);
        for &k in t {
            let n = m.n[k as usize];
            assert!((n.length() - 1.0).abs() < 1e-5, "unit normal");
            assert!(g.dot(n) > 0.0, "winding agrees with the normal");
        }
        let centroid = (a + b + c) * (1.0 / 3.0);
        assert!(g.dot(centroid - inside) > 0.0, "faces outward");
    }
}

#[test]
fn mesh_winds_outward_under_plain_mirrored_and_scaled_placements() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.1);
    let (_, plain, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0))
        .then(&Transform::translation(Vec3::new(3.0, 0.0, 0.0)));
    let def = doc.instance_def(plain).unwrap();
    let (mirrored, _) = doc.place_instance(def, mirror).unwrap();
    let skew = Transform::scale(Vec3::new(2.0, 0.5, 3.0))
        .then(&Transform::rotation(Vec3::new(1.0, 1.0, 0.0), 0.7).unwrap());
    let (scaled, _) = doc.place_instance(def, skew).unwrap();
    for (inst, pose) in [
        (plain, Transform::IDENTITY),
        (mirrored, mirror),
        (scaled, skew),
    ] {
        let id = public(&doc, EntityRef::Instance(inst));
        let m = mesh(&mut doc, json!({ "id": id }));
        assert_eq!(m.idx.len() / 3, 12);
        let inside = pose.apply_point(Point3::new(0.2, 0.1, 0.05)) - Point3::ORIGIN;
        assert_outward(&m, inside);
        // Local space: the definition's own box, outward too.
        let local = mesh(&mut doc, json!({ "id": id, "space": "local" }));
        assert_outward(&local, Vec3::new(0.2, 0.1, 0.05));
    }
}

#[test]
fn a_group_mesh_offsets_its_parts_and_face_indices_land_on_topology_faces() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 3.0, 0.5, 0.25);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let gid = public(&doc, EntityRef::Group(g));
    let m = mesh(&mut doc, json!({ "id": gid }));
    assert_eq!(m.parts.len(), 2);
    let mut index_base = 0u64;
    let mut face_base = 0u64;
    for (k, part) in m.parts.iter().enumerate() {
        assert_eq!(part["index_base"].as_u64().unwrap(), index_base);
        assert_eq!(part["face_index_base"].as_u64().unwrap(), face_base);
        let count = part["index_count"].as_u64().unwrap();
        assert_eq!(count, 36);
        let id = part["id"].as_str().unwrap().to_string();
        let topo = ok(&mut doc, "hew.query.topology", json!({ "id": id }));
        let faces = topo["faces"].as_array().unwrap();
        let verts = topo["vertices"].as_array().unwrap();
        let vtx = |i: &Value| -> Point3 {
            let v = &verts[i.as_u64().unwrap() as usize];
            Point3::new(
                v[0].as_f64().unwrap(),
                v[1].as_f64().unwrap(),
                v[2].as_f64().unwrap(),
            )
        };
        for t in (index_base / 3)..((index_base + count) / 3) {
            let fi = u64::from(m.faces[t as usize]) - face_base;
            let face = &faces[fi as usize];
            let n = &face["plane"]["normal"];
            let normal = Vec3::new(
                n[0].as_f64().unwrap(),
                n[1].as_f64().unwrap(),
                n[2].as_f64().unwrap(),
            );
            let on = vtx(&face["outer"][0]) - Point3::ORIGIN;
            for &v in &m.idx[(t * 3) as usize..(t * 3 + 3) as usize] {
                let p = m.p[v as usize];
                assert!(
                    (p - on).dot(normal).abs() < 1e-5,
                    "part {k}: triangle {t} lies on face {fi}"
                );
            }
        }
        index_base += count;
        face_base += faces.len() as u64;
    }
}

/// Newell's normal of a loop of topology vertex indices.
fn loop_normal(verts: &[Value], ring: &[Value]) -> Vec3 {
    let p = |i: &Value| {
        let v = &verts[i.as_u64().unwrap() as usize];
        Vec3::new(
            v[0].as_f64().unwrap(),
            v[1].as_f64().unwrap(),
            v[2].as_f64().unwrap(),
        )
    };
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for k in 0..ring.len() {
        let (a, b) = (p(&ring[k]), p(&ring[(k + 1) % ring.len()]));
        n = n + a.cross(b);
    }
    n
}

#[test]
fn topology_loops_run_counter_clockwise_about_their_normals_even_mirrored() {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.1);
    let (_, plain, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let def = doc.instance_def(plain).unwrap();
    let (mirrored, _) = doc
        .place_instance(def, Transform::scale(Vec3::new(1.0, -1.0, 1.0)))
        .unwrap();
    let obj = public(&doc, EntityRef::Object(b));
    for inst in [plain, mirrored] {
        let path = json!([public(&doc, EntityRef::Instance(inst)), obj]);
        for space in ["world", "local"] {
            let topo = ok(
                &mut doc,
                "hew.query.topology",
                json!({ "path": path, "space": space }),
            );
            let verts = topo["vertices"].as_array().unwrap().clone();
            for face in topo["faces"].as_array().unwrap() {
                let n = &face["plane"]["normal"];
                let normal = Vec3::new(
                    n[0].as_f64().unwrap(),
                    n[1].as_f64().unwrap(),
                    n[2].as_f64().unwrap(),
                );
                let ring = face["outer"].as_array().unwrap();
                assert!(
                    loop_normal(&verts, ring).dot(normal) > 0.0,
                    "{space}: outer loop CCW about its normal"
                );
            }
        }
    }
}

/// A doubling ladder past the occurrence cap.
fn ladder() -> (Document, String) {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let (mut def, mut inst, _) = doc.make_component(&[NodeId::Object(a)]).unwrap();
    for _ in 0..17 {
        let (second, _) = doc
            .place_instance(def, Transform::translation(Vec3::new(2.0, 0.0, 0.0)))
            .unwrap();
        let (d, i, _) = doc
            .make_component(&[NodeId::Instance(inst), NodeId::Instance(second)])
            .unwrap();
        def = d;
        inst = i;
    }
    let id = public(&doc, EntityRef::Instance(inst));
    (doc, id)
}

#[test]
fn past_the_cap_part_reads_and_selection_export_refuse_too_many_occurrences() {
    let (mut doc, id) = ladder();
    for (method, params) in [
        ("hew.query.part_frame", json!({ "id": id })),
        ("hew.query.mesh", json!({ "id": id })),
    ] {
        let r = call(&mut doc, &mut NoHost, method, params);
        assert_eq!(
            r["error"]["data"]["refusal"], "too_many_occurrences",
            "{method}: {r}"
        );
    }
    let r = call(
        &mut doc,
        &mut ExportHost,
        "hew.doc.export",
        json!({ "format": "stl", "selection": [id] }),
    );
    assert_eq!(r["error"]["data"]["refusal"], "too_many_occurrences", "{r}");
}

fn stl_triangles(r: &Value) -> u32 {
    let bytes = decode(r["result"]["bytes_base64"].as_str().unwrap());
    u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]])
}

#[test]
fn an_export_selection_reaching_one_part_twice_exports_it_once() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 3.0, 1.0, 1.0);
    let (g, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    let gid = public(&doc, EntityRef::Group(g));
    let aid = public(&doc, EntityRef::Object(a));
    for selection in [
        json!([gid, aid]),
        json!([aid, gid, aid]),
        json!([gid, { "path": [aid] }]),
    ] {
        let r = call(
            &mut doc,
            &mut ExportHost,
            "hew.doc.export",
            json!({ "format": "stl", "selection": selection }),
        );
        assert_eq!(stl_triangles(&r), 24, "{selection}");
    }
}

// ------------------------------------------------------- instance materials

/// A component holding one box, placed twice inside an outer component
/// (`outer`), so a leaf is reached through two levels of placements.
fn nested_doc() -> (Document, kernel::InstanceId, kernel::InstanceId, ObjectId) {
    let mut doc = Document::new();
    let b = extrude_box(&mut doc, 0.0, 0.4, 0.2, 0.1);
    let (_, inner, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
    let (_, outer, _) = doc.make_component(&[NodeId::Instance(inner)]).unwrap();
    (doc, outer, inner, b)
}

#[test]
fn faces_report_the_effective_material_through_a_path() {
    let (mut doc, outer, inner, b) = nested_doc();
    let red = doc.add_material(kernel::Material::solid(
        "Red",
        kernel::Rgba8::rgb(200, 0, 0),
    ));
    let blue = doc.add_material(kernel::Material::solid(
        "Blue",
        kernel::Rgba8::rgb(0, 0, 200),
    ));
    doc.set_instance_material(outer, Some(blue)).unwrap();
    doc.set_instance_material(inner, Some(red)).unwrap();
    let face = doc.object(b).unwrap().faces().keys().next().unwrap();
    doc.paint_face(b, face, Some(blue)).unwrap();
    let path = json!([
        public(&doc, EntityRef::Instance(outer)),
        public(&doc, EntityRef::Instance(inner)),
        public(&doc, EntityRef::Object(b)),
    ]);
    let r = ok(&mut doc, "hew.query.faces", json!({ "path": path }));
    let red_id = public(&doc, EntityRef::Material(red));
    let blue_id = public(&doc, EntityRef::Material(blue));
    let faces = r["faces"].as_array().unwrap();
    assert_eq!(
        faces[0]["material_effective"],
        json!(blue_id),
        "own paint wins"
    );
    assert!(
        faces[1..]
            .iter()
            .all(|f| f["material_effective"] == json!(red_id)),
        "the nearest painted placement fills the rest"
    );

    // By object id alone there is no path, so no effective material.
    let b_id = public(&doc, EntityRef::Object(b));
    let by_id = ok(&mut doc, "hew.query.faces", json!({ "object": b_id }));
    assert!(by_id["faces"][0].get("material_effective").is_none());
}

#[test]
fn entity_scene_and_paint_scopes_cover_instances_and_definitions() {
    let (mut doc, outer, inner, b) = nested_doc();
    let red = doc.add_material(kernel::Material::solid(
        "Red",
        kernel::Rgba8::rgb(200, 0, 0),
    ));
    let red_id = public(&doc, EntityRef::Material(red));
    let outer_id = public(&doc, EntityRef::Instance(outer));

    ok(
        &mut doc,
        "hew.material.paint",
        json!({ "id": outer_id, "material": red_id }),
    );
    let e = ok(&mut doc, "hew.query.entity", json!({ "id": outer_id }));
    assert_eq!(e["material"], json!(red_id));
    assert_eq!(e["material_effective"], json!(red_id));
    let scene = ok(&mut doc, "hew.query.scene", json!({}));
    let node = scene["tree"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == json!(outer_id))
        .unwrap()
        .clone();
    assert_eq!(node["material"], json!(red_id));

    // `scope: "definition"` on an instance paints its definition's objects.
    let inner_def = doc.instance_def(inner).unwrap();
    let outer_def = doc.instance_def(outer).unwrap();
    assert_ne!(inner_def, outer_def);
    let inner_id = public(&doc, EntityRef::Instance(inner));
    ok(
        &mut doc,
        "hew.material.paint",
        json!({ "id": inner_id, "material": red_id, "scope": "definition" }),
    );
    assert_eq!(doc.object(b).unwrap().default_material(), Some(red));

    // An object takes no scope; a definition refuses the instance scope.
    let b_id = public(&doc, EntityRef::Object(b));
    let r = call(
        &mut doc,
        &mut NoHost,
        "hew.material.paint",
        json!({ "id": b_id, "material": red_id, "scope": "definition" }),
    );
    assert_eq!(r["error"]["code"], json!(-32602));
    let def_id = public(&doc, EntityRef::Component(outer_def));
    let r = call(
        &mut doc,
        &mut NoHost,
        "hew.material.paint",
        json!({ "id": def_id, "material": null, "scope": "instance" }),
    );
    assert_eq!(r["error"]["code"], json!(-32602));

    // Unpainting the instance clears it.
    ok(
        &mut doc,
        "hew.material.paint",
        json!({ "id": outer_id, "material": null }),
    );
    let e = ok(&mut doc, "hew.query.entity", json!({ "id": outer_id }));
    assert_eq!(e["material"], json!(null));
}

#[test]
fn document_replace_switches_instance_materials() {
    let (mut doc, outer, _, _) = nested_doc();
    let red = doc.add_material(kernel::Material::solid(
        "Red",
        kernel::Rgba8::rgb(200, 0, 0),
    ));
    let blue = doc.add_material(kernel::Material::solid(
        "Blue",
        kernel::Rgba8::rgb(0, 0, 200),
    ));
    doc.set_instance_material(outer, Some(red)).unwrap();
    let (from, to) = (
        public(&doc, EntityRef::Material(red)),
        public(&doc, EntityRef::Material(blue)),
    );
    ok(
        &mut doc,
        "hew.material.replace",
        json!({ "from": from, "to": to }),
    );
    assert_eq!(doc.instance_material(outer), Some(Some(blue)));
}
