//! The Preview wall, Rust layer (docs/dev/DEVELOPMENT.md "Preview
//! features"). Every test here walks `preview-features.json` through
//! `api::preview`; none names a feature, so adding an entry puts it under
//! these tests and deleting one (graduating it) takes it out.
//!
//! Run in both configurations by the gate: plain `cargo test -p api` is a
//! production build (every preview command and parameter refuses
//! `preview_only` on every dispatch path and is listed nowhere), and
//! `HEW_PREVIEW=1 cargo test -p api` is a preview build (each one
//! dispatches). The checks that hold in both run in both.

use api::preview::{self, GATED_API_COPY, GATED_PARAMS, GATED_VALUES, PREVIEW_ONLY};
use api::{
    Connection, DispatchOutcome, Grant, NoHost, Registry, Request, RequestId, Response, codes,
};
use kernel::{Document, ObjectId, Plane, Point3, Vec3};
use serde_json::{Value, json};
use std::collections::BTreeSet;

// ------------------------------------------------------------ fixtures

fn req(id: i64, method: &str, params: Value) -> Request {
    Request {
        document: None,
        jsonrpc: "2.0".to_string(),
        id: Some(RequestId::Number(id)),
        method: method.to_string(),
        params: Some(params),
    }
}

/// The response a dispatch answered, a dry run's included.
fn answer(outcome: DispatchOutcome) -> Response {
    match outcome {
        DispatchOutcome::Reply(r) | DispatchOutcome::DryRun(r) => r,
        other => panic!("expected a reply, got {other:?}"),
    }
}

fn dispatch(
    conn: &mut Connection,
    registry: &Registry,
    doc: &mut Document,
    method: &str,
    params: Value,
) -> Response {
    answer(conn.dispatch(registry, doc, &mut NoHost, req(9, method, params)))
}

/// A 1 × 1 × 0.5 box extruded from a ground rectangle.
fn build_box(doc: &mut Document) -> ObjectId {
    let ground = Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
        .expect("ground plane");
    let s = doc.add_sketch(ground);
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

/// A document holding a box, and a connection hello'd and attached to it
/// under `grant`.
fn session(registry: &Registry, grant: Grant) -> (Connection, Document) {
    let mut doc = Document::new();
    build_box(&mut doc);
    let mut conn = Connection::new(grant, "preview-wall");
    for (method, params) in [
        ("hew.meta.hello", json!({ "protocol": 1 })),
        ("hew.doc.attach", json!({})),
    ] {
        let r = dispatch(&mut conn, registry, &mut doc, method, params);
        assert!(r.error.is_none(), "{method} failed: {:?}", r.error);
    }
    (conn, doc)
}

/// The refusal data of a `-32000` answer, if that is what it is.
fn refusal(r: &Response) -> Option<&Value> {
    let e = r.error.as_ref()?;
    (e.code == codes::REFUSED).then_some(())?;
    e.data.as_ref()
}

/// Every name a capabilities answer lists.
fn listed(r: &Response) -> BTreeSet<String> {
    r.result.as_ref().expect("capabilities answers")["commands"]
        .as_array()
        .expect("a command list")
        .iter()
        .map(|c| c["name"].as_str().expect("a name").to_string())
        .collect()
}

/// Every API command the registry gates, with its feature id.
fn gated_commands() -> Vec<(String, String)> {
    preview::features()
        .iter()
        .flat_map(|f| f.api_commands.iter().map(|c| (c.clone(), f.id.clone())))
        .collect()
}

/// Every preview parameter group whose behavior is in the registry:
/// `(command, params, feature id)`.
fn gated_param_groups() -> Vec<(&'static str, &'static [&'static str], String)> {
    GATED_PARAMS
        .iter()
        .filter_map(|&(behavior, command, params)| {
            preview::feature_of_behavior(behavior).map(|f| (command, params, f.id.clone()))
        })
        .collect()
}

/// Every preview value row whose behavior is in the registry:
/// `(command, field, values, feature id)`.
fn gated_value_rows() -> Vec<(&'static str, &'static str, &'static [&'static str], String)> {
    GATED_VALUES
        .iter()
        .filter_map(|&(behavior, command, field, values)| {
            preview::feature_of_behavior(behavior).map(|f| (command, field, values, f.id.clone()))
        })
        .collect()
}

/// Whether `command`'s params schema declares `field` at its top level
/// (the rows a request can set; the others are result fields only).
fn is_param(registry: &Registry, command: &str, field: &str) -> bool {
    registry
        .get(command)
        .and_then(|d| d.params_schema["properties"].get(field))
        .is_some()
}

/// Every value any enum of `field` lists in `command`'s params and result
/// schemas, as `registry` declares them.
fn listed_values(registry: &Registry, command: &str, field: &str) -> BTreeSet<String> {
    let decl = registry.get(command).expect("declared");
    [&decl.params_schema, &decl.result_schema]
        .into_iter()
        .flat_map(|schema| preview::enums_of_field(schema, field))
        .flatten()
        .collect()
}

// ---------------------------------------------- both configurations

/// Every name in the registry is real, so a typo cannot make an entry gate
/// nothing: each `apiCommands` name is a first-party command of the full
/// inventory, and each preview parameter row names a behavior some entry
/// lists, a real command, and parameters its schema declares.
#[test]
fn every_registry_name_is_a_real_command_or_parameter() {
    let reference = Registry::reference();
    for (command, feature) in gated_commands() {
        let decl = reference.get(&command).unwrap_or_else(|| {
            panic!("{feature}: apiCommands names {command}, which is not a command")
        });
        assert!(!decl.is_plugin(), "{command} is a plugin command");
    }
    for &(behavior, command, params) in GATED_PARAMS {
        assert!(
            preview::feature_of_behavior(behavior).is_some(),
            "GATED_PARAMS names behavior {behavior}, which no entry of preview-features.json lists \
             (a typo, or it graduated: delete the row)"
        );
        let decl = reference
            .get(command)
            .unwrap_or_else(|| panic!("GATED_PARAMS names {command}, which is not a command"));
        assert!(
            preview::feature_of_command(command).is_none(),
            "{command} is gated whole; its parameters need no row"
        );
        let properties = decl.params_schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{command} declares no params properties"));
        for p in params {
            assert!(
                properties.contains_key(*p),
                "{command} declares no parameter {p}"
            );
        }
    }
}

/// Every preview value row is real: its behavior is in an entry, its
/// command is a live first-party command, and every value it gates is one
/// an enum of that field declares (in the params schema, the result
/// schema, or both), so a typo cannot make a row gate nothing.
#[test]
fn every_preview_value_row_names_a_real_value() {
    let reference = Registry::reference();
    assert!(!GATED_VALUES.is_empty());
    for &(behavior, command, field, values) in GATED_VALUES {
        assert!(
            preview::feature_of_behavior(behavior).is_some(),
            "GATED_VALUES names behavior {behavior}, which no entry of preview-features.json lists \
             (a typo, or it graduated: delete the row)"
        );
        assert!(
            reference.get(command).is_some(),
            "GATED_VALUES names {command}, which is not a command"
        );
        assert!(
            preview::feature_of_command(command).is_none(),
            "{command} is gated whole; its values need no row"
        );
        let declared = listed_values(&reference, command, field);
        assert!(
            !declared.is_empty(),
            "{command} declares no enum for {field}"
        );
        for v in values {
            assert!(
                declared.contains(*v),
                "{command}.{field} declares no value {v:?}: {declared:?}"
            );
        }
        assert!(
            declared.iter().any(|d| !values.contains(&d.as_str())),
            "{command}.{field}: every value is gated; gate the parameter instead"
        );
    }
}

/// Every handler-copy row names a behavior some entry lists; its preview
/// copy does point at that feature (so the row is needed) and its
/// production copy does not, nor at any other.
#[test]
fn every_handler_copy_row_is_needed_and_its_production_copy_is_clean() {
    for &(behavior, name, preview_copy, production) in GATED_API_COPY {
        let feature = preview::feature_of_behavior(behavior).unwrap_or_else(|| {
            panic!("GATED_API_COPY names behavior {behavior}, which no entry lists (a typo, or it graduated: delete the row)")
        });
        assert!(
            feature_needles(feature)
                .iter()
                .any(|n| preview_copy.contains(n.as_str())),
            "{name}: the preview copy does not point at {}",
            feature.name
        );
        for f in preview::features() {
            for n in feature_needles(f) {
                assert!(
                    !production.contains(n.as_str()),
                    "{name}: the production copy names {n}"
                );
            }
        }
        let expected = if preview::PREVIEW_BUILD {
            preview_copy
        } else {
            production
        };
        assert_eq!(preview::api_copy(name), expected, "{name}");
    }
}

/// The names that point a reader at `feature`: its name, its tools, and its
/// tools as "the X tool".
fn feature_needles(feature: &preview::Feature) -> Vec<String> {
    let mut needles = vec![feature.name.clone()];
    needles.extend(feature.tools.iter().cloned());
    needles.extend(feature.tools.iter().map(|t| format!("{t} tool")));
    needles
}

/// Strings a handler writes itself, that point at a preview feature, and
/// the gated command each one serves: a production build never runs those
/// handlers, so they never reach a production user. Every other string
/// literal under `crates/api/src` (bar the tables that hold both copies
/// and the copy the runtime checks below cover) names no preview feature,
/// so a refusal written inline in a live handler cannot leak one: its copy
/// belongs in `GATED_API_COPY`.
const HANDLER_TEXT_OF_GATED_COMMANDS: &[(&str, &str, &str)] = &[
    (
        "commands/object.rs",
        "This object has no Smooth level to bake.",
        "hew.solid.bake",
    ),
    (
        "commands/shape.rs",
        "Loft needs closed shapes: name a region or a face, not a line or curve.",
        "hew.solid.loft",
    ),
    (
        "commands/shape.rs",
        "Loft takes faces of world objects; explode the instance first.",
        "hew.solid.loft",
    ),
    (
        "commands/shape.rs",
        "Revolve on a face inside a component definition isn't supported here",
        "hew.solid.revolve",
    ),
];

/// The files whose text the runtime checks cover instead: the gate's own
/// tables (both copies of every gated string), the registry (every
/// production summary and schema is checked below), and the generated UI
/// copy (checked through `explanation_for`).
const TEXT_CHECKED_ELSEWHERE: &[&str] = &["preview.rs", "registry.rs", "refusal_copy.gen.rs"];

/// Every string literal of a Rust source, with its line, comments
/// skipped. Plain and raw strings; escapes are kept as written.
fn string_literals(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'\'' {
            // A char literal ('"' included) or a lifetime: skip the quote
            // and an escaped or plain char with its closing quote.
            if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') {
                i += 4;
            } else if bytes.get(i + 2) == Some(&b'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if c == b'r'
            && matches!(bytes.get(i + 1), Some(b'"' | b'#'))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let mut hashes = 0;
            let mut j = i + 1;
            while bytes.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) != Some(&b'"') {
                i += 1;
                continue;
            }
            let close: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            let start = j + 1;
            let end = source[start..]
                .find(&close)
                .map_or(bytes.len(), |k| start + k);
            out.push((line, source[start..end].to_string()));
            line += source[start..end].matches('\n').count();
            i = end + close.len();
        } else if c == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let text = &source[start..j.min(bytes.len())];
            out.push((line, text.to_string()));
            line += text.matches('\n').count();
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// No string a handler writes names a preview feature or its tool, except
/// the listed strings of gated commands' own handlers; and each listed
/// string is still there and still serves a gated command (so the list
/// cannot outlive what it excuses). Source text is the same in both
/// builds, so this runs in both.
#[test]
fn no_handler_string_names_a_preview_feature() {
    for &(file, text, command) in HANDLER_TEXT_OF_GATED_COMMANDS {
        assert!(
            preview::feature_of_command(command).is_some(),
            "{command} is not a preview command: drop the {file} row (its text is public now)"
        );
        let source = std::fs::read_to_string(format!("{}/src/{file}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        assert!(
            source.contains(text),
            "{file} no longer says {text:?}: drop the row"
        );
    }
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("src is readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(files.len() > 10, "the scan found the sources");
    let needles: BTreeSet<String> = preview::features()
        .iter()
        .flat_map(feature_needles)
        .collect();
    let word = |text: &str, n: &str| {
        text.match_indices(n).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + n.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    };
    let mut leaks = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if TEXT_CHECKED_ELSEWHERE.iter().any(|f| rel.ends_with(f)) {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("readable");
        for (line, text) in string_literals(&source) {
            if HANDLER_TEXT_OF_GATED_COMMANDS
                .iter()
                .any(|&(f, t, _)| f == rel && text.contains(t))
            {
                continue;
            }
            for n in &needles {
                if word(&text, n) {
                    leaks.push(format!("{rel}:{line} names {n}: {text:?}"));
                }
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "handler text names a preview feature; move its copy into preview::GATED_API_COPY:\n{}",
        leaks.join("\n")
    );
}

/// No permission exists only for preview commands, so a production build
/// has nothing preview-specific a plugin could ask for or be granted.
#[test]
fn no_permission_belongs_to_preview_commands_alone() {
    let reference = Registry::reference();
    let gated: BTreeSet<String> = gated_commands().into_iter().map(|(c, _)| c).collect();
    for c in reference.commands().filter(|c| gated.contains(&c.name)) {
        assert!(
            reference
                .commands()
                .any(|o| !gated.contains(&o.name) && o.permission == c.permission),
            "{} is the only kind of command holding {:?}",
            c.name,
            c.permission
        );
    }
}

/// What the wall never gates (wall point 8): editing existing curve
/// geometry, and opening and saving files, are live in every build.
#[test]
fn what_is_never_gated_is_live_in_this_build() {
    let served = Registry::protocol_1();
    for name in [
        "hew.solid.resize_blend",
        "hew.solid.remove_blend",
        "hew.solid.follow_me",
        "hew.doc.new",
        "hew.doc.open",
        "hew.doc.save",
        "hew.doc.export",
        "hew.library.save",
        "hew.query.faces",
    ] {
        assert!(
            preview::feature_of_command(name).is_none(),
            "{name} must never be a preview command"
        );
        assert!(served.get(name).is_some(), "{name} is not served");
    }
}

/// Resize and Remove of a blend work in every build on a blend the kernel
/// made (as a file from a preview build brings one into a production
/// build).
#[test]
fn a_blend_from_a_preview_build_can_be_resized_and_removed_in_this_build() {
    let registry = Registry::protocol_1();
    let (mut conn, mut doc) = session(&registry, Grant::core());
    let object = doc.live_object_ids().into_iter().next().expect("the box");
    let obj = doc.object(object).expect("live");
    let edge = obj
        .edges()
        .keys()
        .find(|&e| {
            let (a, b) = obj.edge_endpoints(e).expect("an edge");
            a.y.abs() < 1e-9
                && b.y.abs() < 1e-9
                && (a.z - 0.5).abs() < 1e-9
                && (b.z - 0.5).abs() < 1e-9
        })
        .expect("the top front edge");
    doc.blend_edges(
        object,
        &kernel::BlendSpec {
            edges: vec![edge],
            profile: kernel::BlendProfile::Round {
                radius: 0.1,
                segments: 4,
            },
            primary: None,
            corners: kernel::CornerStyle::Default,
            stops: None,
            handles: Vec::new(),
        },
    )
    .expect("the kernel rounds the edge");

    let scene = dispatch(&mut conn, &registry, &mut doc, "hew.query.scene", json!({}));
    let id = scene.result.expect("scene")["tree"][0]["id"]
        .as_str()
        .expect("an object id")
        .to_string();
    let strip = |conn: &mut Connection, doc: &mut Document| {
        let q = dispatch(
            conn,
            &registry,
            doc,
            "hew.query.faces",
            json!({ "object": id }),
        );
        let faces = q.result.expect("faces");
        let f = faces["faces"]
            .as_array()
            .expect("a face list")
            .iter()
            .find(|f| f.get("blend").is_some())
            .expect("a blend face")
            .clone();
        json!({ "object": id, "at": f["centroid"] })
    };
    let face = strip(&mut conn, &mut doc);
    let r = dispatch(
        &mut conn,
        &registry,
        &mut doc,
        "hew.solid.resize_blend",
        json!({ "face": face, "radius": 0.2 }),
    );
    assert!(r.error.is_none(), "resize_blend refused: {:?}", r.error);
    let face = strip(&mut conn, &mut doc);
    let r = dispatch(
        &mut conn,
        &registry,
        &mut doc,
        "hew.solid.remove_blend",
        json!({ "face": face }),
    );
    assert!(r.error.is_none(), "remove_blend refused: {:?}", r.error);
    let obj = doc.object(object).expect("live");
    assert_eq!(obj.faces().len(), 6, "the sharp box is back");
}

// ------------------------------------------------ production build

/// Asserts `r` is the `preview_only` refusal for `feature`.
#[cfg(not(hew_preview))]
fn assert_preview_only(r: &Response, feature: &str, path: &str) {
    let data = refusal(r).unwrap_or_else(|| panic!("{path}: expected preview_only, got {r:?}"));
    assert_eq!(data["refusal"], PREVIEW_ONLY, "{path}: {data}");
    assert_eq!(data["detail"]["feature"], feature, "{path}: {data}");
    let explanation = data["explanation"].as_str().expect("an explanation");
    let name = &preview::features()
        .iter()
        .find(|f| f.id == feature)
        .expect("the feature")
        .name;
    assert!(
        explanation.contains(name.as_str()),
        "{path}: the explanation names the feature: {explanation}"
    );
}

/// A production build declares no preview command, so nothing built from
/// its registry lists one: capabilities for a first-party, a plugin and a
/// connected-app grant, and a registry rebuilt from a capabilities answer
/// that does list them (a preview app's, as a production `hew-cli --live`
/// would read it). Preview parameters are out of their command's schema.
#[cfg(not(hew_preview))]
#[test]
fn production_lists_no_preview_command_or_parameter() {
    let registry = Registry::protocol_1();
    let gated = gated_commands();
    for (command, _) in &gated {
        assert!(registry.get(command).is_none(), "{command} is declared");
    }
    use api::Permission;
    let everything: BTreeSet<Permission> = [
        Permission::ModelRead,
        Permission::ModelWrite,
        Permission::View,
        Permission::Library,
        Permission::Files,
        Permission::DocumentsSave,
        Permission::DocumentsOther,
    ]
    .into();
    for (label, grant) in [
        ("core", Grant::core()),
        ("app", Grant::app()),
        (
            "plugin",
            Grant::plugin("com.example.wall", None, everything.clone()),
        ),
        (
            "connected app",
            Grant::connected_app("pair", everything.clone()),
        ),
    ] {
        // Through the build's registry, and through the whole inventory a
        // host might wrongly dispatch with: neither lists one.
        for (which, reg) in [
            ("protocol_1", Registry::protocol_1()),
            ("reference", Registry::reference()),
        ] {
            let (mut conn, mut doc) = session(&reg, grant.clone());
            let r = dispatch(
                &mut conn,
                &reg,
                &mut doc,
                "hew.meta.capabilities",
                json!({}),
            );
            let names = listed(&r);
            for (command, _) in &gated {
                assert!(
                    !names.contains(command),
                    "{label}/{which}: {command} is listed"
                );
            }
            for (command, params, _) in gated_param_groups() {
                let entry = r.result.as_ref().expect("capabilities")["commands"]
                    .as_array()
                    .expect("a list")
                    .iter()
                    .find(|c| c["name"] == command)
                    .cloned();
                // The command may be outside this grant; when listed, its
                // schema carries none of the preview parameters.
                if let (Some(entry), "protocol_1") = (entry, which) {
                    for p in params {
                        assert!(
                            entry["params"]["properties"].get(*p).is_none(),
                            "{label}: {command} lists the preview parameter {p}"
                        );
                    }
                    assert!(
                        entry["refusals"]
                            .as_array()
                            .expect("refusals")
                            .iter()
                            .any(|r| r == PREVIEW_ONLY),
                        "{label}: {command} does not declare preview_only"
                    );
                }
            }
        }
    }
    // A capabilities answer listing them (first-party entries, as a preview
    // app reports them) adds nothing back.
    let full: Vec<Value> = Registry::reference()
        .commands()
        .map(|c| {
            json!({
                "name": c.name, "version": c.version, "summary": c.summary,
                "class": "read_only", "params": c.params_schema, "result": c.result_schema,
                "refusals": c.refusals, "implemented": c.implemented,
            })
        })
        .collect();
    let rebuilt = Registry::from_capabilities(&json!({ "commands": full }));
    for (command, _) in &gated {
        assert!(
            rebuilt.get(command).is_none(),
            "{command} came back from capabilities"
        );
    }
}

/// Every preview command refuses `preview_only`, naming its feature, on
/// every way a command is dispatched: a plain request, a one-command
/// transaction, a transaction where it is not first (nothing before it
/// runs), a dry run, a frame a plugin sends while serving a forward, a
/// restricted plugin connection, a connection before `hello`, and a host
/// that wrongly dispatches through the whole inventory. The handler table
/// itself has nothing for it.
#[cfg(not(hew_preview))]
#[test]
fn every_preview_command_refuses_on_every_dispatch_path() {
    let draw = json!({
        "method": "hew.sketch.draw_rect",
        "params": { "plane": { "ground": true }, "corner_a": [3, 3, 0], "corner_b": [4, 4, 0] }
    });
    use api::Permission;
    let plugin_perms: BTreeSet<Permission> = [
        Permission::ModelRead,
        Permission::ModelWrite,
        Permission::View,
    ]
    .into();
    for (command, feature) in gated_commands() {
        for (which, registry) in [
            ("protocol_1", Registry::protocol_1()),
            ("reference", Registry::reference()),
        ] {
            let path = |p: &str| format!("{command} via {which}: {p}");
            let (mut conn, mut doc) = session(&registry, Grant::core());
            let before = doc.history_generation();

            let r = dispatch(&mut conn, &registry, &mut doc, &command, json!({}));
            assert_preview_only(&r, &feature, &path("plain request"));

            let r = dispatch(
                &mut conn,
                &registry,
                &mut doc,
                "hew.doc.transact",
                json!({ "commands": [{ "method": command, "params": {} }] }),
            );
            assert_preview_only(&r, &feature, &path("one-command transaction"));

            let r = dispatch(
                &mut conn,
                &registry,
                &mut doc,
                "hew.doc.transact",
                json!({ "commands": [draw, { "method": command, "params": {} }] }),
            );
            assert_preview_only(&r, &feature, &path("second in a transaction"));
            assert_eq!(
                refusal(&r).expect("refused")["failed_index"],
                1,
                "{}",
                path("failed index")
            );

            let r = dispatch(
                &mut conn,
                &registry,
                &mut doc,
                "hew.doc.transact",
                json!({ "dry_run": true, "commands": [{ "method": command, "params": {} }] }),
            );
            assert_preview_only(&r, &feature, &path("dry run"));

            let r = answer(conn.dispatch_in_forward(
                &registry,
                &mut doc,
                &mut NoHost,
                req(10, &command, json!({})),
                7,
            ));
            assert_preview_only(&r, &feature, &path("inside a forward"));

            assert_eq!(
                doc.history_generation(),
                before,
                "{}",
                path("the document is untouched")
            );

            let (mut plugin, mut pdoc) = session(
                &registry,
                Grant::plugin("com.example.wall", None, plugin_perms.clone()),
            );
            let r = dispatch(&mut plugin, &registry, &mut pdoc, &command, json!({}));
            assert_preview_only(&r, &feature, &path("plugin connection"));

            let mut fresh = Connection::new(Grant::core(), "no-hello");
            let mut fdoc = Document::new();
            let r = dispatch(&mut fresh, &registry, &mut fdoc, &command, json!({}));
            assert_preview_only(&r, &feature, &path("before hello"));
        }
        assert!(
            api::commands::handler(&command).is_none(),
            "{command} has a handler in a production build"
        );
        match api::commands::resolve(&command, &json!({})) {
            Err(r) => assert_eq!(r.name, PREVIEW_ONLY, "{command}"),
            Ok(_) => panic!("{command} resolves in a production build"),
        }
    }
}

/// A command that stays live but has preview parameters refuses
/// `preview_only` when any of them is set, on every dispatch path, and
/// runs as before without them.
#[cfg(not(hew_preview))]
#[test]
fn every_preview_parameter_refuses_while_its_command_stays_live() {
    let groups = gated_param_groups();
    assert!(!groups.is_empty());
    for (command, params, feature) in groups {
        for p in params {
            let with = json!({ *p: 1 });
            for (which, registry) in [
                ("protocol_1", Registry::protocol_1()),
                ("reference", Registry::reference()),
            ] {
                let path = |s: &str| format!("{command}.{p} via {which}: {s}");
                let (mut conn, mut doc) = session(&registry, Grant::core());
                let before = doc.history_generation();
                let r = dispatch(&mut conn, &registry, &mut doc, command, with.clone());
                assert_preview_only(&r, &feature, &path("plain request"));
                assert_eq!(
                    refusal(&r).expect("refused")["detail"]["params"],
                    json!([p])
                );
                let r = dispatch(
                    &mut conn,
                    &registry,
                    &mut doc,
                    "hew.doc.transact",
                    json!({ "commands": [{ "method": command, "params": with }] }),
                );
                assert_preview_only(&r, &feature, &path("transaction"));
                let r = answer(conn.dispatch_in_forward(
                    &registry,
                    &mut doc,
                    &mut NoHost,
                    req(10, command, with.clone()),
                    7,
                ));
                assert_preview_only(&r, &feature, &path("inside a forward"));
                assert_eq!(doc.history_generation(), before, "{}", path("untouched"));
            }
            match api::commands::resolve(command, &with) {
                Err(r) => assert_eq!(r.name, PREVIEW_ONLY, "{command}.{p}"),
                Ok(_) => panic!("{command}.{p} resolves in a production build"),
            }
        }
        // Without them the command is live: whatever it answers to empty
        // params, it is not the wall.
        let registry = Registry::protocol_1();
        let (mut conn, mut doc) = session(&registry, Grant::core());
        let r = dispatch(&mut conn, &registry, &mut doc, command, json!({}));
        if let Some(data) = refusal(&r) {
            assert_ne!(
                data["refusal"], PREVIEW_ONLY,
                "{command} without preview parameters"
            );
        }
        assert!(
            api::commands::resolve(command, &json!({})).is_ok(),
            "{command}"
        );
    }
}

/// Every preview value of a live field is out of the production schemas
/// (params and result, in the build's registry and in capabilities), and
/// a request setting a parameter to one refuses `preview_only`, naming the
/// feature, on every dispatch path: a plain request, a one-command
/// transaction, a transaction where it is not first, a dry run, a frame
/// inside a forward, a plugin connection, a host dispatching through the
/// whole inventory, and `resolve` itself. The field's other values pass
/// the wall.
#[cfg(not(hew_preview))]
#[test]
fn every_preview_value_is_unlisted_and_refuses_on_every_dispatch_path() {
    let draw = json!({
        "method": "hew.sketch.draw_rect",
        "params": { "plane": { "ground": true }, "corner_a": [3, 3, 0], "corner_b": [4, 4, 0] }
    });
    use api::Permission;
    let plugin_perms: BTreeSet<Permission> = [
        Permission::ModelRead,
        Permission::ModelWrite,
        Permission::View,
        Permission::Library,
    ]
    .into();
    let rows = gated_value_rows();
    assert!(!rows.is_empty());
    let production = Registry::protocol_1();
    let reference = Registry::reference();
    for (command, field, values, feature) in rows {
        // Unlisted: the build's registry and its capabilities answer.
        let left = listed_values(&production, command, field);
        for v in values {
            assert!(!left.contains(*v), "{command}.{field} lists {v:?}");
        }
        assert!(!left.is_empty(), "{command}.{field} keeps its other values");
        assert!(
            production
                .get(command)
                .unwrap()
                .refusals
                .iter()
                .any(|r| r == PREVIEW_ONLY),
            "{command} does not declare preview_only"
        );
        let (mut conn, mut doc) = session(&production, Grant::core());
        let caps = dispatch(
            &mut conn,
            &production,
            &mut doc,
            "hew.meta.capabilities",
            json!({}),
        );
        let entry = caps.result.as_ref().expect("capabilities")["commands"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|c| c["name"] == command)
            .cloned()
            .expect("listed");
        for schema in [&entry["params"], &entry["result"]] {
            for e in preview::enums_of_field(schema, field) {
                for v in values {
                    assert!(
                        !e.iter().any(|x| x == v),
                        "capabilities: {command}.{field} lists {v:?}"
                    );
                }
            }
        }

        if !is_param(&reference, command, field) {
            continue;
        }
        for v in values {
            let with = json!({ field: v });
            for (which, registry) in [("protocol_1", &production), ("reference", &reference)] {
                let path = |s: &str| format!("{command}.{field}={v} via {which}: {s}");
                let (mut conn, mut doc) = session(registry, Grant::core());
                let before = doc.history_generation();
                let r = dispatch(&mut conn, registry, &mut doc, command, with.clone());
                assert_preview_only(&r, &feature, &path("plain request"));
                let data = refusal(&r).expect("refused");
                assert_eq!(
                    data["detail"]["params"],
                    json!([field]),
                    "{}",
                    path("detail")
                );
                assert_eq!(data["detail"]["value"], json!(v), "{}", path("detail"));
                let one = json!({ "method": command, "params": with });
                for (label, params) in [
                    ("one-command transaction", json!({ "commands": [one] })),
                    (
                        "second in a transaction",
                        json!({ "commands": [draw, one] }),
                    ),
                    ("dry run", json!({ "dry_run": true, "commands": [one] })),
                ] {
                    let r = dispatch(&mut conn, registry, &mut doc, "hew.doc.transact", params);
                    assert_preview_only(&r, &feature, &path(label));
                }
                let r = answer(conn.dispatch_in_forward(
                    registry,
                    &mut doc,
                    &mut NoHost,
                    req(10, command, with.clone()),
                    7,
                ));
                assert_preview_only(&r, &feature, &path("inside a forward"));
                assert_eq!(doc.history_generation(), before, "{}", path("untouched"));
                let (mut plugin, mut pdoc) = session(
                    registry,
                    Grant::plugin("com.example.wall", None, plugin_perms.clone()),
                );
                let r = dispatch(&mut plugin, registry, &mut pdoc, command, with.clone());
                assert_preview_only(&r, &feature, &path("plugin connection"));
            }
            match api::commands::resolve(command, &with) {
                Err(r) => assert_eq!(r.name, PREVIEW_ONLY, "{command}.{field}={v}"),
                Ok(_) => panic!("{command}.{field}={v} resolves in a production build"),
            }
        }
        // The field's other values are not the wall.
        for other in left {
            assert!(
                api::commands::resolve(command, &json!({ field: other })).is_ok(),
                "{command}.{field}={other}"
            );
        }
    }
}

// --------------------------------------------------- preview build

/// In a preview build every preview command is declared, listed, and
/// dispatches: whatever it answers to empty params, it is the command's
/// own answer, never the wall and never an unknown method.
#[cfg(hew_preview)]
#[test]
fn every_preview_command_dispatches_in_a_preview_build() {
    let registry = Registry::protocol_1();
    let (mut conn, mut doc) = session(&registry, Grant::core());
    let names = listed(&dispatch(
        &mut conn,
        &registry,
        &mut doc,
        "hew.meta.capabilities",
        json!({}),
    ));
    for (command, _) in gated_commands() {
        assert!(
            registry.get(&command).is_some(),
            "{command} is not declared"
        );
        assert!(names.contains(&command), "{command} is not listed");
        assert!(
            api::commands::resolve(&command, &json!({})).is_ok(),
            "{command}"
        );
        for r in [
            dispatch(&mut conn, &registry, &mut doc, &command, json!({})),
            dispatch(
                &mut conn,
                &registry,
                &mut doc,
                "hew.doc.transact",
                json!({ "commands": [{ "method": command, "params": {} }] }),
            ),
        ] {
            if let Some(e) = &r.error {
                assert_ne!(e.code, codes::METHOD_NOT_FOUND, "{command}: {e:?}");
                let name = e.data.as_ref().and_then(|d| d.get("refusal"));
                assert_ne!(name, Some(&json!(PREVIEW_ONLY)), "{command}: {e:?}");
            }
        }
    }
}

/// In a preview build every preview parameter is in its command's schema
/// and reaches the command.
#[cfg(hew_preview)]
#[test]
fn every_preview_parameter_reaches_its_command_in_a_preview_build() {
    let registry = Registry::protocol_1();
    for (command, params, _) in gated_param_groups() {
        let decl = registry.get(command).expect("declared");
        for p in params {
            assert!(
                decl.params_schema["properties"].get(*p).is_some(),
                "{command} does not declare {p}"
            );
            let with = json!({ *p: 1 });
            assert!(api::commands::resolve(command, &with).is_ok());
            let (mut conn, mut doc) = session(&registry, Grant::core());
            let r = dispatch(&mut conn, &registry, &mut doc, command, with);
            if let Some(data) = refusal(&r) {
                assert_ne!(data["refusal"], PREVIEW_ONLY, "{command}.{p}");
            }
        }
    }
}

/// Every production-copy row names a behavior some entry lists, and the
/// copy it replaces does point at that feature (so the row is needed).
#[test]
fn every_copy_row_replaces_text_that_points_at_its_feature() {
    for &(behavior, code, production) in preview::GATED_COPY {
        let feature = preview::feature_of_behavior(behavior).unwrap_or_else(|| {
            panic!("GATED_COPY names behavior {behavior}, which no entry lists (a typo, or it graduated: delete the row)")
        });
        let reference = api::refusal::reference_explanation_for(&api::refusal::snake_case(code))
            .unwrap_or_else(|| panic!("{code} has no UI copy"));
        assert!(
            reference.contains(&feature.name),
            "{code}'s copy does not name {}",
            feature.name
        );
        assert!(
            !production.contains(&feature.name),
            "{code}'s production copy names {}",
            feature.name
        );
    }
}

/// Nothing a production build lists (summaries, schemas, refusal names)
/// names a preview command, feature or tool, and no refusal it can
/// explain points at one.
#[cfg(not(hew_preview))]
#[test]
fn production_text_points_at_no_preview_feature() {
    let mut needles: Vec<String> = Vec::new();
    for f in preview::features() {
        needles.extend(f.api_commands.iter().cloned());
        needles.push(f.name.clone());
        needles.extend(f.tools.iter().map(|t| format!("{t} tool")));
    }
    for c in Registry::protocol_1().commands() {
        let text = format!(
            "{} {} {} {:?}",
            c.summary, c.params_schema, c.result_schema, c.refusals
        );
        for n in &needles {
            assert!(!text.contains(n.as_str()), "{} mentions {n}", c.name);
        }
        for r in &c.refusals {
            if let Some(copy) = api::refusal::explanation_for(r) {
                for f in preview::features() {
                    assert!(
                        !copy.contains(&f.name),
                        "{}: refusal {r} points at {}",
                        c.name,
                        f.name
                    );
                }
            }
        }
    }
    for &(_, code, production) in preview::GATED_COPY {
        assert_eq!(
            api::refusal::explanation_for(&api::refusal::snake_case(code)),
            Some(production),
            "{code}"
        );
    }
    let r = api::Refusal::from_document_error(&kernel::DocumentError::Boolean(
        kernel::BooleanError::DegenerateInput {
            at: Point3::new(0.0, 0.0, 0.0),
        },
    ));
    for f in preview::features() {
        assert!(
            !r.explanation.contains(&f.name),
            "{} points at {}",
            r.name,
            f.name
        );
    }
}

/// In a preview build every preview value is in its field's enums, and a
/// request setting it reaches the command.
#[cfg(hew_preview)]
#[test]
fn every_preview_value_reaches_its_command_in_a_preview_build() {
    let registry = Registry::protocol_1();
    for (command, field, values, _) in gated_value_rows() {
        let declared = listed_values(&registry, command, field);
        for v in values {
            assert!(
                declared.contains(*v),
                "{command}.{field} does not list {v:?}"
            );
            if !is_param(&registry, command, field) {
                continue;
            }
            let with = json!({ field: v });
            assert!(api::commands::resolve(command, &with).is_ok());
            let (mut conn, mut doc) = session(&registry, Grant::core());
            let r = dispatch(&mut conn, &registry, &mut doc, command, with);
            if let Some(data) = refusal(&r) {
                assert_ne!(data["refusal"], PREVIEW_ONLY, "{command}.{field}={v}");
            }
        }
    }
}

// ------------------------------------------------- Eraser and Smooth

/// The Eraser tool is public: no entry gates it. Its crease half belongs to
/// Smooth, so `hew.object.crease` and the app's `eraser-crease` behavior sit
/// on that one entry, and a production build refuses the command.
#[test]
fn the_eraser_is_public_and_its_crease_half_belongs_to_smooth() {
    assert!(
        preview::features()
            .iter()
            .all(|f| f.tools.iter().all(|t| t != "Eraser") && f.id != "eraser"),
        "no entry gates the Eraser tool"
    );
    let owner =
        preview::feature_of_command("hew.object.crease").expect("hew.object.crease is gated");
    assert_eq!(owner.id, "smooth");
    let behavior = preview::feature_of_behavior("eraser-crease").expect("eraser-crease is gated");
    assert_eq!(behavior.id, "smooth");
    // A production build refuses it; a preview build lets it through.
    assert_eq!(
        preview::command_allowed("hew.object.crease").is_ok(),
        preview::PREVIEW_BUILD
    );
}
