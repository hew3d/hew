//! Pure computations (docs/agents/HEW_API.md §7 "Compute"): `hew.compute.*`
//! on `SolidSpec` operands, read-only, never touching the document; and the
//! transient cutter `hew.solid.subtract` takes beside a path `a`
//! (Addendum B6).

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId};
use kernel::Document;
use serde_json::{Value, json};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

struct Session {
    doc: Document,
    conn: Connection,
}

impl Session {
    fn new() -> Session {
        let mut s = Session {
            doc: Document::new(),
            conn: Connection::new(Grant::core(), "compute"),
        };
        for (m, p) in [
            ("hew.meta.hello", json!({ "protocol": 1 })),
            ("hew.doc.attach", json!({})),
        ] {
            let r = s.call(m, p);
            assert!(r.get("error").is_none(), "{m}: {r}");
        }
        s
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let req = Request {
            document: None,
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(3)),
            method: method.to_string(),
            params: Some(params),
        };
        match self
            .conn
            .dispatch(&REGISTRY, &mut self.doc, &mut NoHost, req)
        {
            DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => {
                serde_json::to_value(r).unwrap()
            }
            other => panic!("{method}: {other:?}"),
        }
    }

    /// A compute call's result; the document must be exactly as it was.
    fn compute(&mut self, method: &str, params: Value) -> Value {
        let before = self.clock();
        let r = self.call(method, params);
        assert_eq!(self.clock(), before, "{method} touched the document");
        assert!(r.get("error").is_none(), "{method}: {r}");
        r["result"].clone()
    }

    fn refusal(&mut self, method: &str, params: Value) -> String {
        let before = self.clock();
        let r = self.call(method, params);
        assert_eq!(self.clock(), before, "{method} touched the document");
        r["error"]["data"]["refusal"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} refused: {r}"))
            .to_string()
    }

    fn clock(&self) -> (u64, u64, usize, u64) {
        (
            self.doc.history_generation(),
            self.doc.document_epoch(),
            self.doc.undo_depth(),
            self.doc.state_hash(),
        )
    }

    /// One committed command's result.
    fn commit(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params);
        assert!(r.get("error").is_none(), "{method}: {r}");
        r["result"]["results"][0].clone()
    }

    /// Draws `ring` on the plane at height `z` and extrudes it by `h`.
    fn prism(&mut self, z: f64, ring: &[[f64; 3]], h: f64) -> String {
        let r = self.call(
            "hew.doc.transact",
            json!({ "commands": [
                { "method": "hew.sketch.draw_line", "as": "p", "params": {
                    "plane": { "origin": [0.0, 0.0, z], "normal": [0.0, 0.0, 1.0] },
                    "points": ring.iter().chain(ring.first()).collect::<Vec<_>>() } },
                { "method": "hew.solid.extrude", "params": {
                    "region": { "$ref": "p#/region_id" }, "distance": h } }
            ] }),
        );
        assert!(r.get("error").is_none(), "{r}");
        r["result"]["results"][1]["object_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn mesh_digest(&mut self, id: &str) -> (Value, Value) {
        let r = self.call("hew.query.mesh", json!({ "id": id }));
        let m = &r["result"];
        (m["positions_base64"].clone(), m["indices_base64"].clone())
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> Vec<[f64; 3]> {
    vec![[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]]
}

fn unit_box() -> Value {
    json!({ "box": { "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0] } })
}

fn close(a: &Value, b: f64, tol: f64) {
    let a = a.as_f64().unwrap_or_else(|| panic!("{a} is a number"));
    assert!((a - b).abs() <= tol, "{a} vs {b}");
}

// ------------------------------------------------------------ boolean

/// Addendum B3's equal-hash row: the computed subtract of two prisms is the
/// geometry a committed `hew.solid.subtract` of the same two leaves.
#[test]
fn boolean_two_prisms_equals_committed_subtract() {
    let tool = rect(0.5, 0.25, 1.5, 0.75, 0.5);
    let mut real = Session::new();
    let a = real.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let b = real.prism(0.5, &tool, 1.0);
    let cut = real.commit("hew.solid.subtract", json!({ "a": a, "b": b }));
    let committed = real.mesh_digest(cut["result"].as_str().unwrap());

    let mut pure = Session::new();
    let r = pure.compute(
        "hew.compute.boolean",
        json!({
            "op": "subtract",
            "a": { "prism": { "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
                              "loops": [rect(0.0, 0.0, 1.0, 1.0, 0.0)], "distance": 1.0 } },
            "b": { "prism": { "plane": { "origin": [0.0, 0.0, 0.5], "normal": [0.0, 0.0, 1.0] },
                              "loops": [tool], "distance": 1.0 } },
            "want": ["mesh", "measure"]
        }),
    );
    assert_eq!(r["solid"], true);
    assert_eq!(
        (
            r["mesh"]["positions_base64"].clone(),
            r["mesh"]["indices_base64"].clone()
        ),
        committed,
        "the computed mesh hashes equal to the committed one"
    );
    close(&r["measure"]["volume"], 1.0 - 0.125, 1e-12);
    assert_eq!(r["mesh"]["parts"][0]["id"], Value::Null);
    assert!(r["mesh"].get("generation").is_none());

    // The same subtract through boxes, measured: the box sugar is the same
    // prism.
    let r = pure.compute(
        "hew.compute.boolean",
        json!({
            "op": "subtract",
            "a": unit_box(),
            "b": { "box": { "min": [0.5, 0.25, 0.5], "max": [1.5, 0.75, 1.5] } },
            "want": ["mesh"]
        }),
    );
    assert_eq!(
        (
            r["mesh"]["positions_base64"].clone(),
            r["mesh"]["indices_base64"].clone()
        ),
        committed
    );
}

#[test]
fn boolean_by_id_and_by_box_agree() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let by_id = s.compute(
        "hew.compute.boolean",
        json!({ "op": "union", "a": { "id": a }, "b": { "box": { "min": [0.5, 0.0, 0.0], "max": [2.0, 1.0, 0.5] } } }),
    );
    close(&by_id["measure"]["volume"], 1.0 + 0.5, 1e-12);
    assert_eq!(by_id["measure"]["bbox"]["max"], json!([2.0, 1.0, 1.0]));
}

#[test]
fn boolean_degenerate_contact_refuses_by_name() {
    let mut s = Session::new();
    let name = s.refusal(
        "hew.compute.boolean",
        json!({ "op": "subtract", "a": { "id": "obj_9" }, "b": unit_box() }),
    );
    assert_eq!(name, "unknown_entity");
    // Two boxes touching only along an edge.
    let name = s.refusal(
        "hew.compute.boolean",
        json!({ "op": "union", "a": unit_box(), "b": { "box": { "min": [1.0, 1.0, 0.0], "max": [2.0, 2.0, 1.0] } } }),
    );
    assert_eq!(name, "degenerate_contact");
    let name = s.refusal(
        "hew.compute.boolean",
        json!({ "op": "subtract", "a": unit_box(), "b": { "box": { "min": [3.0, 0.0, 0.0], "max": [4.0, 1.0, 1.0] } } }),
    );
    assert_eq!(name, "nothing_removed");
}

#[test]
fn boolean_pose_replaces_world_pose() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let b = s.prism(0.0, &rect(0.5, 0.5, 1.5, 1.5, 0.0), 1.0);
    let r = s.compute(
        "hew.compute.boolean",
        json!({ "op": "intersect", "a": { "id": a }, "b": { "id": b } }),
    );
    assert_eq!(r["solid"], true);
    close(&r["measure"]["volume"], 0.25, 1e-12);
    // Posed ten meters away, b no longer meets a: nothing is left.
    let away = [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let r = s.compute(
        "hew.compute.boolean",
        json!({ "op": "intersect", "a": { "id": a }, "b": { "id": b, "pose": away } }),
    );
    assert_eq!(r, json!({ "solid": false }));
}

#[test]
fn mirrored_pose_refuses() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let mirror = [-1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    assert_eq!(
        s.refusal(
            "hew.compute.measure",
            json!({ "solid": { "id": a, "pose": mirror } })
        ),
        "mirrored_pose"
    );
}

#[test]
fn mirrored_placement_refuses_mirrored_pose() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let made = s.commit("hew.component.create", json!({ "members": [a] }));
    let ins = made["instance"].as_str().unwrap().to_string();
    s.commit(
        "hew.component.flip_instance",
        json!({ "instance": ins, "axis": "x" }),
    );
    let rows = s.call("hew.query.occurrences", json!({}));
    let member = rows["result"]["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "object")
        .unwrap()
        .clone();
    assert_eq!(member["mirrored"], true, "{member}");
    let path: Vec<Value> = member["path"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .chain([member["id"].clone()])
        .collect();
    assert_eq!(
        s.refusal(
            "hew.compute.measure",
            json!({ "solid": { "path": path.clone() } })
        ),
        "mirrored_pose"
    );
    // The un-mirrored frame passed as the pose measures it.
    let r = s.compute(
        "hew.compute.measure",
        json!({ "solid": { "path": path, "pose": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] } }),
    );
    close(&r["volume"], 1.0, 1e-12);
}

#[test]
fn group_or_instance_solid_refuses_not_an_object() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let g = s.commit("hew.group.create", json!({ "members": [a] }));
    let gid = g["group"]
        .as_str()
        .unwrap_or_else(|| panic!("{g}"))
        .to_string();
    assert_eq!(
        s.refusal("hew.compute.measure", json!({ "solid": { "id": gid } })),
        "not_an_object"
    );
}

// ------------------------------------------------------------ slice, shells

#[test]
fn slice_splits_a_box_in_two_with_volumes_summing() {
    let mut s = Session::new();
    let r = s.compute(
        "hew.compute.slice",
        json!({ "solid": unit_box(), "plane": { "origin": [0.3, 0.0, 0.0], "normal": [1.0, 0.0, 0.0] } }),
    );
    let pos = r["positive"]["measure"]["volume"].as_f64().unwrap();
    let neg = r["negative"]["measure"]["volume"].as_f64().unwrap();
    assert!((pos + neg - 1.0).abs() <= 1e-9);
    assert!(
        (pos - 0.7).abs() <= 1e-9,
        "positive is the side the normal points to"
    );
    close(&r["positive"]["measure"]["bbox"]["min"][0], 0.3, 1e-12);
}

#[test]
fn slice_plane_misses_solid_refuses() {
    let mut s = Session::new();
    assert_eq!(
        s.refusal(
            "hew.compute.slice",
            json!({ "solid": unit_box(), "plane": { "origin": [5.0, 0.0, 0.0], "normal": [1.0, 0.0, 0.0] } })
        ),
        "plane_misses_solid"
    );
}

#[test]
fn shells_counts_two_after_a_severing_subtract() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let slab = s.prism(-0.5, &rect(0.4, -0.5, 0.6, 1.5, -0.5), 2.0);
    let cut = s.commit("hew.solid.subtract", json!({ "a": a, "b": slab }));
    // The committed result lands as two solids in a group; their union
    // computed back is one Object of two shells.
    let rows = s.call("hew.query.occurrences", json!({ "kinds": ["object"] }));
    let ids: Vec<String> = rows["result"]["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["hidden"] == false)
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 2, "{cut}");
    let r = s.compute(
        "hew.compute.boolean",
        json!({ "op": "subtract", "a": unit_box(), "b": { "box": { "min": [0.4, -0.5, -0.5], "max": [0.6, 1.5, 1.5] } }, "want": ["shells", "measure"] }),
    );
    assert_eq!(r["shells"], 2);
    let shells = s.compute(
        "hew.compute.shells",
        json!({ "solid": { "prism": { "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] }, "loops": [rect(0.0, 0.0, 1.0, 1.0, 0.0)], "distance": 1.0 } }, "want_mesh": true }),
    );
    assert_eq!(shells["count"], 1);
    assert_eq!(shells["shells"][0]["mesh"]["triangle_count"], 12);
    for id in ids {
        let r = s.compute("hew.compute.shells", json!({ "solid": { "id": id } }));
        assert_eq!(r["count"], 1);
        close(&r["shells"][0]["measure"]["volume"], 0.4, 1e-9);
    }
}

// ------------------------------------------------------------ measure, prism

#[test]
fn measure_matches_query_mesh_volume() {
    let mut s = Session::new();
    let r = s.compute("hew.compute.measure", json!({ "solid": unit_box() }));
    close(&r["volume"], 1.0, 1e-12);
    close(&r["area"], 6.0, 1e-12);
    for k in 0..3 {
        close(&r["centroid"][k], 0.5, 1e-12);
    }
    assert_eq!(
        r["bbox"],
        json!({ "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0] })
    );
    let a = s.prism(0.0, &rect(0.0, 0.0, 2.0, 1.0, 0.0), 0.5);
    let r = s.compute("hew.compute.measure", json!({ "solid": { "id": a } }));
    close(&r["volume"], 1.0, 1e-12);
    close(&r["centroid"][0], 1.0, 1e-12);
}

#[test]
fn prism_with_a_hole_has_the_hole() {
    let mut s = Session::new();
    let r = s.compute(
        "hew.compute.measure",
        json!({ "solid": { "prism": {
            "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "loops": [rect(0.0, 0.0, 2.0, 2.0, 0.0), rect(0.5, 0.5, 1.0, 1.0, 0.0)],
            "distance": 1.0 } } }),
    );
    close(&r["volume"], 4.0 - 0.25, 1e-12);
    // A hole outside the outer loop is a params error.
    let r = s.call(
        "hew.compute.measure",
        json!({ "solid": { "prism": {
            "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "loops": [rect(0.0, 0.0, 1.0, 1.0, 0.0), rect(2.0, 2.0, 3.0, 3.0, 0.0)],
            "distance": 1.0 } } }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = s.call(
        "hew.compute.measure",
        json!({ "solid": { "box": { "min": [0, 0, 0], "max": [1, 1, 1] }, "id": "obj_1" } }),
    );
    assert_eq!(r["error"]["code"], -32602, "one form only: {r}");
}

/// A prism measured on the ground plane, or its `-32602` message.
fn prism_measure(s: &mut Session, loops: Value, distance: f64) -> Result<Value, String> {
    let r = s.call(
        "hew.compute.measure",
        json!({ "solid": { "prism": {
            "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "loops": loops, "distance": distance } } }),
    );
    match r.get("error") {
        None => Ok(r["result"].clone()),
        Some(e) => {
            assert_eq!(e["code"], -32602, "a params error, not a refusal: {r}");
            Err(e["message"].as_str().unwrap().to_string())
        }
    }
}

/// Loops that cross or touch are `-32602`, never a silently kept part:
/// a bowtie outer loop, a hole crossing the outer loop, two holes crossing
/// each other, a hole touching the outer loop at a vertex, and a loop that
/// revisits one of its own points.
#[test]
fn crossing_loops_are_params_errors() {
    let mut s = Session::new();
    let cases = [
        (
            "bowtie",
            json!([[
                [0.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 0.0]
            ]]),
        ),
        (
            "hole across the outer loop",
            json!([rect(0.0, 0.0, 2.0, 2.0, 0.0), rect(1.0, 0.5, 3.0, 1.5, 0.0)]),
        ),
        (
            "holes across each other",
            json!([
                rect(0.0, 0.0, 4.0, 4.0, 0.0),
                rect(0.5, 0.5, 2.0, 2.0, 0.0),
                rect(1.5, 1.5, 3.0, 3.0, 0.0)
            ]),
        ),
        (
            "a loop through its own point twice",
            json!([[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
                [1.0, 1.0, 0.0]
            ]]),
        ),
    ];
    for (what, loops) in cases {
        let msg = prism_measure(&mut s, loops, 1.0).expect_err(what);
        assert!(
            msg.contains("cross") || msg.contains("touch"),
            "{what}: {msg}"
        );
    }
    // The same through hew.compute.offset.
    let r = s.call(
        "hew.compute.offset",
        json!({ "loops": [[[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]]], "distance": 0.1 }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    // A loop with three collinear points in a row is sound.
    let ok = prism_measure(
        &mut s,
        json!([[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [0.0, 1.0, 0.0]
        ]]),
        1.0,
    )
    .unwrap();
    close(&ok["volume"], 2.0, 1e-12);
}

/// A point off the plane and two consecutive points that coincide are
/// `-32602` with a message saying which, not a kernel sketch refusal.
#[test]
fn malformed_loops_are_params_errors() {
    let mut s = Session::new();
    let msg = prism_measure(
        &mut s,
        json!([[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0]
        ]]),
        1.0,
    )
    .unwrap_err();
    assert!(msg.contains("coincide"), "{msg}");
    let msg = prism_measure(
        &mut s,
        json!([[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5]]]),
        1.0,
    )
    .unwrap_err();
    assert!(msg.contains("off the plane"), "{msg}");
    let r = s.call(
        "hew.compute.offset",
        json!({ "loops": [[[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 1.0]]], "distance": 0.1 }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

/// A distance so large the derived area or centroid overflows is `-32602`:
/// nothing serializes as `null` that a caller would read as an open shell.
#[test]
fn an_overflowing_prism_is_a_params_error() {
    let mut s = Session::new();
    let msg = prism_measure(&mut s, json!([rect(0.0, 0.0, 1.0, 1.0, 0.0)]), 1e200).unwrap_err();
    assert!(msg.contains("too large"), "{msg}");
    // A large but representable distance still measures.
    let r = prism_measure(&mut s, json!([rect(0.0, 0.0, 1.0, 1.0, 0.0)]), 1e5).unwrap();
    close(&r["volume"], 1e5, 1e-6);
}

/// An oversized loop is refused from its point count before any sketch is
/// built: a million points answer at once, not after a quadratic build.
#[test]
fn an_oversized_prism_is_refused_before_it_is_built() {
    let mut s = Session::new();
    let n = 1_000_000;
    let ring: Vec<[f64; 3]> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            [t.cos(), t.sin(), 0.0]
        })
        .collect();
    let started = std::time::Instant::now();
    let r = s.call(
        "hew.compute.measure",
        json!({ "solid": { "prism": {
            "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "loops": [ring], "distance": 1.0 } } }),
    );
    assert_eq!(r["error"]["data"]["refusal"], "compute_too_large", "{r}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "refused from the count, took {:?}",
        started.elapsed()
    );
}

/// A finely faceted loop builds and measures correctly (no timing
/// assertion: the cost is bounded by the point cap, not by a clock).
#[test]
fn a_finely_faceted_prism_builds_correctly() {
    let mut s = Session::new();
    let n = 1_500;
    let ring: Vec<[f64; 3]> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            [t.cos(), t.sin(), 0.0]
        })
        .collect();
    let r = prism_measure(&mut s, json!([ring]), 1.0).unwrap();
    close(&r["volume"], std::f64::consts::PI, 1e-3);
}

/// Past the point cap a loop is refused before it is built.
#[test]
fn a_loop_past_the_point_cap_is_a_params_error() {
    let mut s = Session::new();
    let n = 20_001;
    let ring: Vec<[f64; 3]> = (0..n)
        .map(|k| {
            let t = k as f64 / n as f64 * std::f64::consts::TAU;
            [t.cos(), t.sin(), 0.0]
        })
        .collect();
    let r = s.call(
        "hew.compute.measure",
        json!({ "solid": { "prism": {
            "plane": { "origin": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0] },
            "loops": [ring], "distance": 1.0 } } }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

// ------------------------------------------------------------ outline

#[test]
fn outline_largest_face_of_a_rotated_board() {
    let mut s = Session::new();
    let board = s.prism(0.0, &rect(0.0, 0.0, 0.4, 0.2, 0.0), 0.02);
    s.commit(
        "hew.entity.rotate",
        json!({ "ids": [board], "axis": [1.0, 1.0, 0.0], "angle": 0.6, "pivot": [0.0, 0.0, 0.0] }),
    );
    let r = s.compute(
        "hew.compute.outline",
        json!({ "id": board, "which": "largest_face" }),
    );
    let loops = r["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 1, "{r}");
    assert_eq!(loops[0]["outer"], true);
    let edges = loops[0]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 4);
    let mut lengths: Vec<f64> = edges
        .iter()
        .map(|e| {
            assert_eq!(e["kind"], "line");
            let (a, b) = (&e["a"], &e["b"]);
            let d = |k: usize| b[k].as_f64().unwrap() - a[k].as_f64().unwrap();
            (d(0).powi(2) + d(1).powi(2)).sqrt()
        })
        .collect();
    lengths.sort_by(f64::total_cmp);
    for (got, want) in lengths.iter().zip([0.2, 0.2, 0.4, 0.4]) {
        assert!((got - want).abs() <= 1e-9, "{lengths:?}");
    }
    // The silhouette along the part's thickness is the same rectangle.
    let r = s.compute(
        "hew.compute.outline",
        json!({ "id": board, "which": "projection" }),
    );
    assert_eq!(r["loops"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["loops"][0]["outer"], true);
}

fn drilled_board(s: &mut Session) -> String {
    // A rectangle with a circle drawn inside it on the same sketch, the
    // ring between them (the rectangle's region, now holed) extruded: the
    // hole's rims carry the circle.
    let r = s.call(
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.sketch.draw_rect", "as": "r", "params": {
                "plane": { "ground": true }, "corner_a": [0.0, 0.0, 0.0], "corner_b": [0.4, 0.2, 0.0] } },
            { "method": "hew.sketch.draw_circle", "as": "c", "params": {
                "plane": { "sketch": { "$ref": "r#/sketch" } }, "center": [0.1, 0.1, 0.0], "radius": 0.03 } },
            { "method": "hew.solid.extrude", "as": "b", "params": {
                "region": { "$ref": "r#/region_id" }, "distance": 0.02 } }
        ] }),
    );
    assert!(r.get("error").is_none(), "{r}");
    r["result"]["results"][2]["object_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn outline_keeps_a_circle_analytic() {
    let mut s = Session::new();
    let board = drilled_board(&mut s);
    let r = s.compute(
        "hew.compute.outline",
        json!({ "id": board, "which": "largest_face", "frame": {
            "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0], "z": [0.0, 0.0, 1.0] } }),
    );
    let loops = r["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 2, "{r}");
    let hole = loops.iter().find(|l| l["outer"] == false).unwrap();
    let edges = hole["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "{hole}");
    assert_eq!(edges[0]["kind"], "circle");
    close(&edges[0]["center"][0], 0.1, 1e-9);
    close(&edges[0]["center"][1], 0.1, 1e-9);
    close(&edges[0]["radius"], 0.03, 1e-9);
    // Seen along z, the silhouette keeps the hole and its circle.
    let r = s.compute(
        "hew.compute.outline",
        json!({ "id": board, "which": "projection", "frame": {
            "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0], "z": [0.0, 0.0, 1.0] } }),
    );
    let loops = r["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 2, "{r}");
    assert_eq!(loops[0]["outer"], true);
    let hole = &loops[1];
    assert_eq!(hole["outer"], false);
    assert_eq!(hole["edges"].as_array().unwrap().len(), 1, "{hole}");
    assert_eq!(hole["edges"][0]["kind"], "circle");
    close(&hole["edges"][0]["radius"], 0.03, 1e-9);
}

/// A part from one closed arc on the ground, extruded.
fn arc_part(s: &mut Session, sweep: f64, segments: u32, close: &str, height: f64) -> String {
    let r = s.call(
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.sketch.draw_arc", "as": "a", "params": {
                "plane": { "ground": true }, "center": [0.0, 0.0, 0.0], "radius": 0.1,
                "start_angle": 0.0, "end_angle": sweep, "segments": segments, "close": close } },
            { "method": "hew.solid.extrude", "params": {
                "region": { "$ref": "a#/region_id" }, "distance": height } }
        ] }),
    );
    assert!(r.get("error").is_none(), "{r}");
    r["result"]["results"][1]["object_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn edge_kinds(r: &Value) -> Vec<String> {
    let mut kinds: Vec<String> = r["loops"][0]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    kinds.sort();
    kinds
}

/// The silhouette keeps an arc an arc by what its facets are, not by how
/// short they are: a coarse arc of four facets (each 0.39 of the radius)
/// stays one arc, and a short straight chord whose ends sit on the circle
/// stays a line. Both agree with the largest face.
#[test]
fn outline_projection_reads_arcs_from_their_facets() {
    let mut s = Session::new();
    let frame = json!({ "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0], "z": [0.0, 0.0, 1.0] });
    let pie = arc_part(&mut s, std::f64::consts::FRAC_PI_2, 4, "pie", 0.02);
    // Thin, so its cap is its largest face.
    let sliver = arc_part(&mut s, 10f64.to_radians(), 4, "segment", 1e-4);
    for (id, expected) in [
        (&pie, vec!["arc", "line", "line"]),
        (&sliver, vec!["arc", "line"]),
    ] {
        for which in ["largest_face", "projection"] {
            let r = s.compute(
                "hew.compute.outline",
                json!({ "id": id, "which": which, "frame": frame }),
            );
            assert_eq!(edge_kinds(&r), expected, "{which}: {r}");
        }
    }
}

#[test]
fn outline_refuses_an_invalid_frame() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    assert_eq!(
        s.refusal(
            "hew.compute.outline",
            json!({ "id": a, "which": "projection", "frame": {
                "origin": [0.0, 0.0, 0.0], "x": [1.0, 0.0, 0.0], "y": [0.0, 1.0, 0.0], "z": [0.0, 0.0, -1.0] } })
        ),
        "invalid_frame"
    );
}

// ------------------------------------------------------------ offset

#[test]
fn offset_grows_and_shrinks() {
    let mut s = Session::new();
    let square = json!([[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]]);
    let grown = s.compute(
        "hew.compute.offset",
        json!({ "loops": square, "distance": 0.1 }),
    );
    let shrunk = s.compute(
        "hew.compute.offset",
        json!({ "loops": square, "distance": -0.1 }),
    );
    let extent = |r: &Value| -> (f64, f64) {
        let xs: Vec<f64> = r["loops"][0]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["a"][0].as_f64().unwrap())
            .collect();
        (
            xs.iter().copied().fold(f64::MAX, f64::min),
            xs.iter().copied().fold(f64::MIN, f64::max),
        )
    };
    let (lo, hi) = extent(&grown);
    assert!(
        (lo + 0.1).abs() <= 1e-9 && (hi - 1.1).abs() <= 1e-9,
        "{grown}"
    );
    let (lo, hi) = extent(&shrunk);
    assert!(
        (lo - 0.1).abs() <= 1e-9 && (hi - 0.9).abs() <= 1e-9,
        "{shrunk}"
    );
    assert_eq!(grown["loops"][0]["outer"], true);
    let r = s.call(
        "hew.compute.offset",
        json!({ "loops": square, "distance": 0.1, "join": "round" }),
    );
    assert_eq!(r["error"]["code"], -32602, "round is reserved: {r}");
}

#[test]
fn offset_collapse_refuses_by_name() {
    let mut s = Session::new();
    assert_eq!(
        s.refusal(
            "hew.compute.offset",
            json!({ "loops": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]], "distance": -0.6 })
        ),
        "offset_collapsed"
    );
    assert_eq!(
        s.refusal(
            "hew.compute.offset",
            json!({ "loops": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]], "distance": 0.0 })
        ),
        "offset_too_small"
    );
}

// ------------------------------------------------------------ gate, reserved rows

#[test]
fn compute_busy_when_the_gate_is_held() {
    let mut s = Session::new();
    let held = s.conn.compute_gate().try_acquire().expect("free");
    let r = s.call("hew.compute.measure", json!({ "solid": unit_box() }));
    assert_eq!(r["error"]["code"], -32000);
    assert_eq!(r["error"]["data"]["refusal"], "compute_busy", "{r}");
    assert_eq!(r["error"]["data"]["detail"], json!({ "limit": 1 }));
    drop(held);
    let r = s.call("hew.compute.measure", json!({ "solid": unit_box() }));
    assert!(r.get("error").is_none(), "{r}");
    assert!(!s.conn.compute_gate().is_held(), "released after the call");
    // A refused call releases the gate too.
    s.refusal(
        "hew.compute.measure",
        json!({ "solid": { "id": "obj_404" } }),
    );
    assert!(!s.conn.compute_gate().is_held());
}

#[test]
fn project_and_stretch_refuse_unimplemented() {
    let mut s = Session::new();
    for method in ["hew.compute.project", "hew.compute.stretch"] {
        assert_eq!(s.refusal(method, json!({})), "unimplemented");
    }
    let caps = s.call("hew.meta.capabilities", json!({}));
    let commands = caps["result"]["commands"].as_array().unwrap();
    for method in ["hew.compute.project", "hew.compute.stretch"] {
        let entry = commands.iter().find(|c| c["name"] == method).unwrap();
        assert_eq!(entry["implemented"], false);
        assert_eq!(entry["class"], "read_only");
    }
}

/// Every compute command leaves the document exactly as it was
/// (`Session::compute` checks each call); this runs them all once more
/// against a document with something in it.
#[test]
fn compute_does_not_touch_the_document() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let before = s.doc.save();
    s.compute("hew.compute.measure", json!({ "solid": { "id": a } }));
    s.compute(
        "hew.compute.boolean",
        json!({ "op": "subtract", "a": { "id": a }, "b": { "box": { "min": [0.5, 0.5, 0.5], "max": [2, 2, 2] } }, "want": ["mesh", "topology", "measure", "shells"] }),
    );
    s.compute(
        "hew.compute.slice",
        json!({ "solid": { "id": a }, "plane": { "origin": [0.5, 0.5, 0.5], "normal": [0, 0, 1] }, "want": ["topology"] }),
    );
    s.compute("hew.compute.shells", json!({ "solid": { "id": a } }));
    s.compute(
        "hew.compute.outline",
        json!({ "id": a, "which": "largest_face" }),
    );
    s.compute(
        "hew.compute.outline",
        json!({ "id": a, "which": "projection" }),
    );
    s.compute(
        "hew.compute.offset",
        json!({ "loops": [[[0, 0], [1, 0], [1, 1]]], "distance": 0.05 }),
    );
    assert_eq!(s.doc.save(), before);
}

// ------------------------------------------------- a transient cutter (B6)

#[test]
fn a_prism_cutter_machines_every_placement_in_one_call() {
    let mut s = Session::new();
    let a = s.prism(0.0, &rect(0.0, 0.0, 1.0, 1.0, 0.0), 1.0);
    let made = s.commit("hew.component.create", json!({ "members": [a] }));
    let ins = made["instance"].as_str().unwrap().to_string();
    let objects_before = s.doc.live_object_ids().len();
    let depth = s.doc.undo_depth();
    let cut = s.commit(
        "hew.solid.subtract",
        json!({
            "a": { "path": [ins, a] },
            "b": { "prism": { "plane": { "origin": [0.0, 0.0, 0.5], "normal": [0.0, 0.0, 1.0] },
                              "loops": [rect(0.25, 0.25, 0.75, 0.75, 0.5)], "distance": 1.0 } }
        }),
    );
    assert_eq!(s.doc.undo_depth(), depth + 1, "one undo entry");
    assert_eq!(
        s.doc.live_object_ids().len(),
        objects_before,
        "no temporary object, the member replaced"
    );
    let result = cut["result"].as_str().unwrap().to_string();
    let r = s.call("hew.query.entity", json!({ "id": result }));
    assert!(
        r.get("error").is_none() || r["error"]["data"]["refusal"] == "unknown_occurrence",
        "{r}"
    );
    let rows = s.call("hew.query.occurrences", json!({ "kinds": ["object"] }));
    let member = rows["result"]["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == result.as_str())
        .unwrap_or_else(|| panic!("{rows}"))
        .clone();
    let mut path: Vec<Value> = member["path"].as_array().unwrap().clone();
    path.push(member["id"].clone());
    let m = s.compute("hew.compute.measure", json!({ "solid": { "path": path } }));
    close(&m["volume"], 1.0 - 0.125, 1e-12);

    // Union and a world a refuse as params errors; nothing changes.
    let r = s.call(
        "hew.solid.union",
        json!({ "a": { "path": [ins, result] }, "b": unit_box() }),
    );
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let w = s.prism(5.0, &rect(0.0, 0.0, 1.0, 1.0, 5.0), 1.0);
    let r = s.call("hew.solid.subtract", json!({ "a": w, "b": unit_box() }));
    assert_eq!(r["error"]["code"], -32602, "{r}");
}
