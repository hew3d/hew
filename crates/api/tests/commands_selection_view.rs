//! Conformance coverage for the selection and view-state commands
//! (docs/agents/HEW_API.md §7, "Selection and view state"):
//! `hew.selection.get/set/clear` and `hew.view.hide/show/show_all/hidden/
//! isolate/clear_isolation/zoom_to`.
//!
//! Two layers, like `commands_view.rs`: the `NoHost` refusals (the host
//! commands answer `host_capability_missing`; hide, show, and hidden are
//! kernel-served and run anywhere), and a `FakeHost` that holds a selection
//! and an isolation the way the live app's host does, merging picks through
//! the one shared rule (`api::selection::merge`).

use api::{
    Connection, DispatchOutcome, Grant, Host, IdResolver, IsolationEntry, NoHost, Refusal, Request,
    RequestId, SelectMode, SelectionEntry, SelectionState, codes,
};
use kernel::{Document, EntityRef, NodeId, Plane, Point3, Vec3};
use serde_json::{Value, json};

/// The registry every dispatch in this file runs against.
static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

// ------------------------------------------------------------- test host

/// Holds a selection and an isolation, and records the hidden-state
/// notifications and the boxes it was asked to frame.
#[derive(Default)]
struct FakeHost {
    selection: SelectionState,
    isolation: Option<Vec<IsolationEntry>>,
    hidden_changed: u32,
    zooms: Vec<([f64; 3], [f64; 3], f64)>,
}

impl Host for FakeHost {
    fn selection(&self) -> Result<SelectionState, Refusal> {
        Ok(self.selection.clone())
    }

    fn set_selection(
        &mut self,
        entries: Vec<SelectionEntry>,
        mode: SelectMode,
    ) -> Result<SelectionState, Refusal> {
        self.selection.entries = api::selection::merge(&self.selection.entries, &entries, mode);
        Ok(self.selection.clone())
    }

    fn hidden_changed(&mut self) -> Result<(), Refusal> {
        self.hidden_changed += 1;
        Ok(())
    }

    fn isolation(&self) -> Result<Option<Vec<IsolationEntry>>, Refusal> {
        Ok(self.isolation.clone())
    }

    fn isolate(&mut self, entries: Vec<IsolationEntry>) -> Result<(), Refusal> {
        self.isolation = Some(entries);
        Ok(())
    }

    fn clear_isolation(&mut self) -> Result<(), Refusal> {
        self.isolation = None;
        Ok(())
    }

    fn zoom_to(&mut self, min: Point3, max: Point3, padding: f64) -> Result<(), Refusal> {
        self.zooms
            .push(([min.x, min.y, min.z], [max.x, max.y, max.z], padding));
        Ok(())
    }
}

// ----------------------------------------------------------------- fixtures

fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// A rectangle at `x`, one meter square, as one gesture.
fn rect(doc: &mut Document, x: f64) -> kernel::SketchId {
    let s = doc.add_sketch(ground());
    doc.begin_sketch_gesture(s).expect("gesture");
    let sk = doc.sketch_mut(s).expect("sketch is live");
    let c = [
        Point3::new(x, 0.0, 0.0),
        Point3::new(x + 1.0, 0.0, 0.0),
        Point3::new(x + 1.0, 1.0, 0.0),
        Point3::new(x, 1.0, 0.0),
    ];
    for i in 0..4 {
        sk.add_segment(c[i], c[(i + 1) % 4]).expect("segment");
    }
    doc.end_sketch_gesture(s).expect("end gesture");
    s
}

/// A box at `x`: a rectangle extruded half a meter.
fn boxed(doc: &mut Document, x: f64) -> kernel::ObjectId {
    let s = rect(doc, x);
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    doc.extrude_region(s, regions[0], 0.5).expect("extrude");
    *doc.live_object_ids().last().expect("an object")
}

fn public(doc: &Document, entity: EntityRef) -> String {
    IdResolver::new(doc)
        .public_of(doc, &entity)
        .expect("a live entity carries a stable id")
}

/// Two boxes (`a` at x 0, `b` at x 3), a free sketch at x 6, a guide point,
/// and a guide line.
struct Fixture {
    doc: Document,
    a: String,
    b: String,
    a_id: kernel::ObjectId,
    sketch: String,
    point: String,
    line: String,
}

fn fixture() -> Fixture {
    let mut doc = Document::new();
    let a_id = boxed(&mut doc, 0.0);
    let b_id = boxed(&mut doc, 3.0);
    let sk = rect(&mut doc, 6.0);
    let point = doc
        .add_guide_point(Point3::new(0.0, 0.0, 5.0))
        .expect("guide point");
    let line = doc
        .add_guide_line(Point3::ORIGIN, Vec3::new(1.0, 0.0, 0.0))
        .expect("guide line");
    Fixture {
        a: public(&doc, EntityRef::Object(a_id)),
        b: public(&doc, EntityRef::Object(b_id)),
        sketch: public(&doc, EntityRef::Sketch(sk)),
        point: public(&doc, EntityRef::Guide(point)),
        line: public(&doc, EntityRef::Guide(line)),
        a_id,
        doc,
    }
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

fn call(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> api::Response {
    let DispatchOutcome::Reply(r) = conn.dispatch(&REGISTRY, doc, host, req(9, method, params))
    else {
        panic!("{method} replies")
    };
    r
}

fn connect(doc: &mut Document, host: &mut dyn Host) -> Connection {
    let mut conn = Connection::new(Grant::app(), "test");
    for (method, params) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        let r = call(&mut conn, doc, host, method, params);
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
    conn
}

fn ok(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, method, params);
    assert!(r.error.is_none(), "{method} refused: {:?}", r.error);
    r.result.expect("a successful reply carries a result")
}

/// The refusal's canonical data, asserting the `-32000` code.
fn refused(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) -> Value {
    let r = call(conn, doc, host, method, params);
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} was expected to refuse"));
    assert_eq!(err.code, codes::REFUSED, "{method}: {err:?}");
    err.data.expect("a refusal carries the canonical data")
}

fn params_error(
    conn: &mut Connection,
    doc: &mut Document,
    host: &mut dyn Host,
    method: &str,
    params: Value,
) {
    let r = call(conn, doc, host, method, params.clone());
    let err = r
        .error
        .unwrap_or_else(|| panic!("{method} {params} was expected to fail"));
    assert_eq!(
        err.code,
        codes::INVALID_PARAMS,
        "{method} {params}: {err:?}"
    );
}

// ------------------------------------------------------- the merge algebra

/// The four modes against `mergeSelection`'s doc comment
/// (`app/src/panels/treeModel.ts`): replace → the picked nodes; toggle →
/// the picked ones already selected leave, the rest join; add → the ones
/// not yet selected join, in pick order, no duplicates; subtract → the
/// picked ones leave; survivors keep their order (index 0 stays primary).
#[test]
fn merge_matches_the_app_rule() {
    let mut doc = Document::new();
    let ids: Vec<kernel::ObjectId> = (0..5).map(|i| boxed(&mut doc, 3.0 * i as f64)).collect();
    let e = |i: usize| SelectionEntry::Object(ids[i]);
    let g = |list: &[usize]| list.iter().map(|&i| e(i)).collect::<Vec<_>>();
    use SelectMode::*;
    type Row = (
        &'static [usize],
        &'static [usize],
        SelectMode,
        &'static [usize],
    );
    let table: &[Row] = &[
        // replace: the pick, whatever was selected.
        (&[0, 1], &[2, 3], Replace, &[2, 3]),
        (&[0, 1], &[], Replace, &[]),
        // add: the new ones join at the end in pick order; nothing leaves.
        (&[0, 1], &[3, 1, 2], Add, &[0, 1, 3, 2]),
        (&[0, 1], &[1], Add, &[0, 1]),
        (&[], &[4, 2], Add, &[4, 2]),
        // subtract: the picked ones leave; the order of the rest holds.
        (&[0, 1, 2, 3], &[2, 0], Subtract, &[1, 3]),
        (&[0, 1], &[4], Subtract, &[0, 1]),
        // toggle: the selected picks leave, the rest join in pick order.
        (&[0, 1, 2], &[3, 1], Toggle, &[0, 2, 3]),
        (&[0], &[0], Toggle, &[]),
        (&[], &[2, 1], Toggle, &[2, 1]),
        // a pick naming an entry twice counts it once.
        (&[0], &[2, 2], Add, &[0, 2]),
        (&[0], &[2, 2], Toggle, &[0, 2]),
        (&[], &[3, 3, 1], Replace, &[3, 1]),
    ];
    for (current, picked, mode, expected) in table {
        assert_eq!(
            api::selection::merge(&g(current), &g(picked), *mode),
            g(expected),
            "{current:?} {mode:?} {picked:?}"
        );
    }
    // Kind is part of equality: an object and a group never match.
    let (grp, _) = doc
        .group_nodes(&[NodeId::Object(ids[3]), NodeId::Object(ids[4])])
        .expect("group");
    let merged = api::selection::merge(&[e(0)], &[SelectionEntry::Group(grp)], Add);
    assert_eq!(merged, vec![e(0), SelectionEntry::Group(grp)]);
}

// ------------------------------------------------------- hew.selection.*

#[test]
fn set_replaces_adds_subtracts_and_toggles() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.a] }),
    );
    assert_eq!(got["ids"], json!([f.a]));
    assert_eq!(got["primary"], json!(f.a));
    assert!(got["generation"].is_u64() && got["epoch"].is_u64(), "{got}");

    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.b, f.sketch], "mode": "add" }),
    );
    assert_eq!(got["ids"], json!([f.a, f.b, f.sketch]));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.a], "mode": "subtract" }),
    );
    assert_eq!(got["ids"], json!([f.b, f.sketch]));
    assert_eq!(got["primary"], json!(f.b));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.b, f.a], "mode": "toggle" }),
    );
    assert_eq!(got["ids"], json!([f.sketch, f.a]));

    // get answers the host's state; clear empties it.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.get",
        json!({}),
    );
    assert_eq!(got["ids"], json!([f.sketch, f.a]));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.clear",
        json!({}),
    );
    assert_eq!(got["ids"], json!([]));
    assert_eq!(got["primary"], Value::Null);
    assert!(host.selection.entries.is_empty());

    // One guide or one annotation alone is a selection too.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.point] }),
    );
    assert_eq!(got["ids"], json!([f.point]));
}

#[test]
fn set_refuses_unknown_mixed_and_hidden_targets_and_changes_nothing() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.b] }),
    );
    let before = host.selection.clone();

    for bad in ["obj_ffffff", "nonsense"] {
        let data = refused(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.selection.set",
            json!({ "ids": [f.a, bad] }),
        );
        assert_eq!(data["refusal"], "unknown_entity");
        assert_eq!(data["detail"]["id"], bad);
    }
    // A material resolves, but nothing selects one.
    let mat = f.doc.add_material(kernel::Material {
        name: "Oak".into(),
        color: kernel::Rgba8 {
            r: 200,
            g: 150,
            b: 100,
            a: 255,
        },
        texture: None,
    });
    let mat = public(&f.doc, EntityRef::Material(mat));
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [mat] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");

    for ids in [json!([f.a, f.point]), json!([f.point, f.line])] {
        let data = refused(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.selection.set",
            json!({ "ids": ids }),
        );
        assert_eq!(data["refusal"], "selection_mixed_kinds");
        assert_eq!(
            data["explanation"],
            "A selection holds nodes, or one guide, or one annotation, never a mix."
        );
    }

    f.doc.set_node_user_hidden(NodeId::Object(f.a_id), true);
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.a], "mode": "add" }),
    );
    assert_eq!(data["refusal"], "selection_target_not_visible");
    assert_eq!(data["detail"], json!({ "id": f.a }));
    assert_eq!(
        data["explanation"],
        "That entity is hidden. Show it first, or isolate it."
    );
    assert_eq!(
        host.selection, before,
        "a refused set leaves the selection alone"
    );

    // Over 10 000 ids is a params error.
    let many: Vec<String> = (0..10_001).map(|_| f.b.clone()).collect();
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": many }),
    );
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.b], "mode": "xor" }),
    );
}

/// A node hidden through a hidden tag on its group refuses as well; a
/// sketch on a hidden tag does too.
#[test]
fn set_refuses_what_a_tag_or_a_group_hides() {
    let mut f = fixture();
    let b = IdResolver::new(&f.doc).resolve(&f.b).unwrap();
    let EntityRef::Object(b_id) = b else { panic!() };
    let (grp, _) = f.doc.group_nodes(&[NodeId::Object(b_id)]).expect("group");
    f.doc
        .add_node_tag(NodeId::Group(grp), vec!["Walls".into(), "North".into()])
        .unwrap();
    f.doc.set_tag_hidden(vec!["Walls".into()], true);
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.b] }),
    );
    assert_eq!(data["refusal"], "selection_target_not_visible");

    // A sketch on a hidden tag refuses too (sketch tags arrive with an
    // import, a `.skp` edge's layer).
    let scene = kernel::ImportScene {
        materials: Vec::new(),
        defs: Vec::new(),
        roots: Vec::new(),
        guides: Vec::new(),
        tags: vec![kernel::ImportTag {
            path: vec!["Walls".into(), "Sketches".into()],
            hidden: false,
        }],
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: kernel::ImportSceneExtra {
            sketches: vec![kernel::ImportSketch {
                plane: ground(),
                def: None,
                group: None,
                segments: vec![
                    [Point3::new(9.0, 0.0, 0.0), Point3::new(10.0, 0.0, 0.0)],
                    [Point3::new(10.0, 0.0, 0.0), Point3::new(10.0, 1.0, 0.0)],
                ],
                curves: Vec::new(),
                tags: vec![vec!["Walls".into(), "Sketches".into()]],
            }],
            ..Default::default()
        },
    };
    let (report, _) = f
        .doc
        .ingest(scene, Vec::new())
        .expect("ingest a tagged sketch");
    let tagged = report.sketches[0].expect("the sketch was built");
    let tagged = public(&f.doc, EntityRef::Sketch(tagged));
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [tagged] }),
    );
    assert_eq!(data["refusal"], "selection_target_not_visible");
    assert_eq!(data["detail"], json!({ "id": tagged }));
    // Shown again, it selects.
    f.doc.set_tag_hidden(vec!["Walls".into()], false);
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [tagged] }),
    );
}

/// The one-kind rule holds for the merged result, not only the pick: add
/// or toggle onto a selection of the other family refuses
/// `selection_mixed_kinds` and leaves the selection as it was.
#[test]
fn add_and_toggle_never_mix_kinds() {
    let mut f = fixture();
    // A second guide point, for "a second guide".
    let other = f
        .doc
        .add_guide_point(Point3::new(1.0, 1.0, 5.0))
        .expect("guide point");
    let other = public(&f.doc, EntityRef::Guide(other));
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let cases: [(Value, Value); 6] = [
        // Nodes selected: a guide added or toggled in would mix.
        (
            json!([f.a, f.b]),
            json!({ "ids": [f.point], "mode": "add" }),
        ),
        (json!([f.a]), json!({ "ids": [f.point], "mode": "toggle" })),
        // A guide selected: a node or a second guide added would mix.
        (json!([f.point]), json!({ "ids": [f.a], "mode": "add" })),
        (json!([f.point]), json!({ "ids": [other], "mode": "add" })),
        (
            json!([f.point]),
            json!({ "ids": [f.sketch], "mode": "toggle" }),
        ),
        (
            json!([f.point]),
            json!({ "ids": [other], "mode": "toggle" }),
        ),
    ];
    for (start, params) in cases {
        ok(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.selection.set",
            json!({ "ids": start }),
        );
        let before = host.selection.clone();
        let data = refused(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.selection.set",
            params.clone(),
        );
        assert_eq!(data["refusal"], "selection_mixed_kinds", "{start} {params}");
        assert_eq!(host.selection, before, "{start} {params} changed nothing");
    }
    // Toggling the lone guide off, adding it again, and subtracting stay legal.
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.point] }),
    );
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.point], "mode": "toggle" }),
    );
    assert_eq!(got["ids"], json!([]));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.point], "mode": "add" }),
    );
    assert_eq!(got["ids"], json!([f.point]));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.a], "mode": "subtract" }),
    );
    assert_eq!(got["ids"], json!([f.point]));
    // Replace always may switch families.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.selection.set",
        json!({ "ids": [f.a] }),
    );
    assert_eq!(got["ids"], json!([f.a]));
}

#[test]
fn set_inside_a_transaction_is_visible_to_a_later_get() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let undo = f.doc.undo_depth();
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.selection.set", "params": { "ids": [f.a, f.b] } },
            { "method": "hew.selection.set", "params": { "ids": [f.a], "mode": "subtract" } },
            { "method": "hew.selection.get", "params": {} }
        ] }),
    );
    assert_eq!(got["results"][2]["ids"], json!([f.b]), "{got}");
    assert_eq!(
        f.doc.undo_depth(),
        undo,
        "the selection records no undo entry"
    );
}

// ---------------------------------------------- hew.view.hide / show / hidden

#[test]
fn hide_then_hidden_lists_it() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let undo = f.doc.undo_depth();
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hide",
        json!({ "ids": [f.b, f.a] }),
    );
    // Listed in stable-id order: a was made first.
    assert_eq!(got, json!({ "hidden": [f.a, f.b] }));
    assert_eq!(host.hidden_changed, 1);
    assert!(f.doc.node_user_hidden(NodeId::Object(f.a_id)));
    assert_eq!(f.doc.undo_depth(), undo, "hiding records no undo entry");

    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["hidden"], json!([f.a, f.b]));
    assert_eq!(got["isolated"], Value::Null);

    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.show",
        json!({ "ids": [f.a] }),
    );
    assert_eq!(got, json!({ "hidden": [f.b] }));
    assert_eq!(host.hidden_changed, 2);

    // A hidden tag is listed by its public id.
    f.doc.set_tag_hidden(vec!["Walls".into()], true);
    let tag = public(&f.doc, EntityRef::Tag(vec!["Walls".into()]));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["tags_hidden"], json!([tag]));
}

#[test]
fn hide_refuses_what_is_not_a_node_and_changes_nothing() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    for not_node in [f.sketch.clone(), f.point.clone()] {
        let data = refused(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.view.hide",
            json!({ "ids": [f.a, not_node] }),
        );
        assert_eq!(data["refusal"], "not_a_node");
        assert_eq!(data["detail"]["id"], json!(not_node));
    }
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.show",
        json!({ "ids": ["obj_ffffff"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    assert!(
        !f.doc.node_user_hidden(NodeId::Object(f.a_id)),
        "a refused hide hides nothing"
    );
    assert_eq!(host.hidden_changed, 0);
}

#[test]
fn show_all_clears() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hide",
        json!({ "ids": [f.a, f.b] }),
    );
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.show_all",
        json!({}),
    );
    assert_eq!(got, json!({ "hidden": [] }));
    assert!(f.doc.user_hidden_nodes().is_empty());
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["hidden"], json!([]));
}

/// Hide is view state, but it rides a transaction like any read and an
/// aborted envelope takes it back.
#[test]
fn a_hide_in_an_aborted_transaction_rolls_back() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let r = call(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.doc.transact",
        json!({ "commands": [
            { "method": "hew.view.hide", "params": { "ids": [f.a] } },
            { "method": "hew.view.hide", "params": { "ids": ["obj_ffffff"] } }
        ] }),
    );
    assert_eq!(
        r.error.expect("the envelope aborts").data.unwrap()["refusal"],
        "unknown_entity"
    );
    assert!(!f.doc.node_user_hidden(NodeId::Object(f.a_id)));
}

// --------------------------------------------- hew.view.isolate / zoom_to

#[test]
fn isolate_then_hidden_reports_it_and_clear_ends_it() {
    let mut f = fixture();
    // A component instance of box b, to name one of its members.
    let EntityRef::Object(b_id) = IdResolver::new(&f.doc).resolve(&f.b).unwrap() else {
        panic!()
    };
    let (_, instance, _) = f
        .doc
        .make_component(&[NodeId::Object(b_id)])
        .expect("make a component");
    let ins = public(&f.doc, EntityRef::Instance(instance));
    let def = f.doc.instance_def(instance).unwrap();
    let (member, _) = f.doc.expanded_def_placements(def)[0];
    let member = public(&f.doc, EntityRef::Object(member));

    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    let target = json!({ "object": member, "instance": ins });
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.isolate",
        json!({ "ids": [f.a, target] }),
    );
    assert_eq!(got, json!({ "isolated": [f.a, target] }));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["isolated"], json!([f.a, target]));

    // A member must be placed by the instance it is named with.
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.isolate",
        json!({ "ids": [{ "object": f.a, "instance": ins }] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    assert_eq!(data["detail"]["id"], json!(f.a));
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.isolate",
        json!({ "ids": [f.sketch] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    params_error(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.isolate",
        json!({ "ids": [] }),
    );

    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.clear_isolation",
        json!({}),
    );
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["isolated"], Value::Null);
}

#[test]
fn zoom_to_pads_the_union_box() {
    let mut f = fixture();
    let mut host = FakeHost::default();
    let mut conn = connect(&mut f.doc, &mut host);
    // a spans x 0..1, b spans x 3..4, both y 0..1 and z 0..0.5: the union
    // is 4 wide, so padding 0.5 widens every side by 2.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.zoom_to",
        json!({ "ids": [f.a, f.b], "padding": 0.5 }),
    );
    assert_eq!(
        got,
        json!({ "min": [0.0, 0.0, 0.0], "max": [4.0, 1.0, 0.5] })
    );
    assert_eq!(host.zooms, vec![([-2.0, -2.0, -2.0], [6.0, 3.0, 2.5], 0.5)]);

    // The default padding is a tenth; a guide line has no box but does not
    // spoil the others.
    ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.zoom_to",
        json!({ "ids": [f.a, f.line] }),
    );
    let (min, max, padding) = host.zooms[1];
    assert_eq!(padding, 0.1);
    assert!(
        (min[0] + 0.1).abs() < 1e-12 && (max[0] - 1.1).abs() < 1e-12,
        "{min:?} {max:?}"
    );

    // A lone guide point has no extent: it frames as a 1 m cube around
    // it, padded by a tenth of that, and answers the point itself.
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.zoom_to",
        json!({ "ids": [f.point] }),
    );
    assert_eq!(
        got,
        json!({ "min": [0.0, 0.0, 5.0], "max": [0.0, 0.0, 5.0] })
    );
    let (min, max, _) = *host.zooms.last().unwrap();
    for k in 0..3 {
        assert!((max[k] - min[k] - 1.2).abs() < 1e-12, "{min:?} {max:?}");
    }
    assert!((min[2] - 4.4).abs() < 1e-12 && (max[2] - 5.6).abs() < 1e-12);
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.zoom_to",
        json!({ "ids": [f.line] }),
    );
    assert_eq!(data["refusal"], "nothing_to_frame");
    let data = refused(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.zoom_to",
        json!({ "ids": ["obj_ffffff"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    for padding in [-0.1, 2.5] {
        params_error(
            &mut conn,
            &mut f.doc,
            &mut host,
            "hew.view.zoom_to",
            json!({ "ids": [f.a], "padding": padding }),
        );
    }
    assert_eq!(host.zooms.len(), 3, "a refused zoom frames nothing");
}

// ---------------------------------------------------------- NoHost pins

/// A `core` grant with no host reaches the four kernel-served visibility
/// commands; the host-served `hew.view.isolate` stays outside the ceiling.
#[test]
fn hide_show_hidden_work_with_a_core_grant_and_no_host() {
    let mut f = fixture();
    let mut host = NoHost;
    let mut conn = Connection::new(Grant::core(), "core");
    for (method, params) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        let r = call(&mut conn, &mut f.doc, &mut host, method, params);
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hide",
        json!({ "ids": [f.b, f.a] }),
    );
    assert_eq!(got, json!({ "hidden": [f.a, f.b] }));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.hidden",
        json!({}),
    );
    assert_eq!(got["hidden"], json!([f.a, f.b]));
    assert_eq!(got["isolated"], Value::Null);
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.show",
        json!({ "ids": [f.a] }),
    );
    assert_eq!(got, json!({ "hidden": [f.b] }));
    let got = ok(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.show_all",
        json!({}),
    );
    assert_eq!(got["hidden"], json!([]));
    let r = call(
        &mut conn,
        &mut f.doc,
        &mut host,
        "hew.view.isolate",
        json!({ "ids": [f.a] }),
    );
    let err = r.error.expect("isolate is outside the core ceiling");
    assert_eq!(err.code, -32001, "{err:?}");
}

/// The selection commands are host-served, so a headless host has none; the
/// kernel-served hide and hidden still answer.
#[test]
fn no_host_refusals() {
    let mut doc = Document::new();
    let mut host = NoHost;
    let mut conn = connect(&mut doc, &mut host);
    let r = call(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.selection.get",
        json!({}),
    );
    let err = r.error.expect("refused");
    assert_eq!(err.code, codes::REFUSED);
    let data = err.data.expect("data");
    let keys: Vec<&str> = data
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "detail",
            "explanation",
            "failed_index",
            "failed_method",
            "refusal"
        ],
        "the five canonical fields"
    );
    assert_eq!(data["refusal"], "host_capability_missing");
    assert_eq!(data["failed_method"], "hew.selection.get");

    for (method, params) in [
        ("hew.selection.set", json!({ "ids": [] })),
        ("hew.selection.clear", json!({})),
        ("hew.view.clear_isolation", json!({})),
    ] {
        let data = refused(&mut conn, &mut doc, &mut host, method, params);
        assert_eq!(data["refusal"], "host_capability_missing", "{method}");
    }

    let data = refused(
        &mut conn,
        &mut doc,
        &mut host,
        "hew.view.hide",
        json!({ "ids": ["obj_1"] }),
    );
    assert_eq!(data["refusal"], "unknown_entity");
    let got = ok(&mut conn, &mut doc, &mut host, "hew.view.hidden", json!({}));
    assert_eq!(
        got,
        json!({ "hidden": [], "tags_hidden": [], "isolated": null })
    );
}

/// All ten are ReadOnly, never mutate the document, and need `view`.
#[test]
fn the_selection_and_view_commands_are_read_only_view_commands() {
    for name in [
        "hew.selection.get",
        "hew.selection.set",
        "hew.selection.clear",
        "hew.view.hide",
        "hew.view.show",
        "hew.view.show_all",
        "hew.view.hidden",
        "hew.view.isolate",
        "hew.view.clear_isolation",
        "hew.view.zoom_to",
    ] {
        let decl = REGISTRY.get(name).expect("declared");
        assert_eq!(decl.class, api::CommandClass::ReadOnly, "{name}");
        assert!(!decl.mutates_document, "{name}");
        assert!(decl.implemented, "{name}");
        assert_eq!(decl.permission, api::Permission::View, "{name}");
    }
}
