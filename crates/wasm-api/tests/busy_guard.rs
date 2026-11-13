//! The busy guard is complete (docs/agents/PLUGINS.md "Compounds"): while
//! a plugin command holds the document, every `Scene` method the UI can
//! reach that could change the document refuses `DocumentBusy`. The UI
//! reaches the kernel only through `Scene`, so a method that forgot the
//! guard is a hole in it; this test reads the source and finds any.
//!
//! The rule: every `pub fn` taking `&mut self` in a `#[wasm_bindgen] impl
//! Scene` block calls `self.ensure_not_busy()` first, unless its name is in
//! `wasm_api::BUSY_EXEMPT` (the bus, view mirrors, the compound controls,
//! and methods that touch only `Scene`'s own caches).

use std::path::PathBuf;

/// One `&mut self` method of an exported `impl Scene` block.
struct Method {
    file: String,
    name: String,
    body: String,
}

fn exported_mut_methods() -> Vec<Method> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("src is readable")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("source is readable");
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let exported = lines[i].starts_with("impl Scene")
                && i > 0
                && lines[i - 1].starts_with("#[wasm_bindgen");
            if !exported {
                i += 1;
                continue;
            }
            // The block ends at the closing brace in column 0.
            let end = (i + 1..lines.len())
                .find(|&j| lines[j] == "}")
                .expect("the impl block closes");
            let block = &lines[i + 1..end];
            let starts: Vec<usize> = (0..block.len())
                .filter(|&k| {
                    let t = block[k].trim_start();
                    t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ")
                })
                .collect();
            for (n, &k) in starts.iter().enumerate() {
                let stop = starts.get(n + 1).copied().unwrap_or(block.len());
                let text = block[k..stop].join("\n");
                // The signature runs to the body's opening brace; it may
                // span lines.
                let sig_end = text.find('{').expect("a body");
                let signature = &text[..sig_end];
                if !signature.contains("&mut self") {
                    continue;
                }
                let after_fn = signature.split(" fn ").nth(1).expect("fn name");
                let name = after_fn
                    .split(['(', '<'])
                    .next()
                    .expect("a name")
                    .trim()
                    .to_string();
                out.push(Method {
                    file: path.file_name().unwrap().to_string_lossy().into_owned(),
                    name,
                    body: text[sig_end..].to_string(),
                });
            }
            i = end + 1;
        }
    }
    out
}

#[test]
fn every_mutating_scene_method_is_guarded_or_exempt() {
    let methods = exported_mut_methods();
    assert!(
        methods.len() > 150,
        "the scan found only {} methods: is it reading the right blocks?",
        methods.len()
    );
    let missed: Vec<String> = methods
        .iter()
        .filter(|m| !wasm_api::BUSY_EXEMPT.contains(&m.name.as_str()))
        .filter(|m| !m.body.contains("ensure_not_busy()"))
        .map(|m| format!("{}::{}", m.file, m.name))
        .collect();
    assert!(
        missed.is_empty(),
        "these &mut self Scene methods neither call self.ensure_not_busy() first nor appear in BUSY_EXEMPT: {missed:?}"
    );
    // The guard is the FIRST statement: nothing reaches the document
    // before it.
    let late: Vec<String> = methods
        .iter()
        .filter(|m| m.body.contains("ensure_not_busy()"))
        .filter(|m| {
            let first = m.body[1..].trim_start();
            !first.starts_with("self.ensure_not_busy()")
        })
        .map(|m| format!("{}::{}", m.file, m.name))
        .collect();
    assert!(
        late.is_empty(),
        "the guard is not the first statement: {late:?}"
    );
    // Every exempt name is a real method, so the list cannot rot.
    for name in wasm_api::BUSY_EXEMPT {
        assert!(
            methods.iter().any(|m| m.name == *name),
            "BUSY_EXEMPT names {name}, which is no &mut self Scene method"
        );
    }
}

/// The runtime half, through the public surface: with a compound open
/// around a plugin command, the UI's undo and drawing refuse
/// `DocumentBusy`; after the abort they work again.
#[test]
fn ui_methods_refuse_while_a_compound_is_open() {
    let mut scene = wasm_api::Scene::new();
    scene
        .register_plugin_commands(
            "org.hew3d.test",
            r#"[{"id":"hold","class":"mutating","summary":"Holds.","permission":"model.write"}]"#,
        )
        .expect("registers");
    let conn = scene.api_connection_open();
    for frame in [
        r#"{"jsonrpc":"2.0","id":0,"method":"hew.meta.hello","params":{"protocol":1}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"hew.doc.attach","params":{}}"#,
    ] {
        scene.api_dispatch(conn, frame).expect("answers");
    }
    let tagged = scene
        .api_dispatch(
            conn,
            r#"{"jsonrpc":"2.0","id":2,"method":"org.hew3d.test.hold","params":{}}"#,
        )
        .expect("answers");
    let v: serde_json::Value = serde_json::from_str(&tagged).unwrap();
    let forward = v["pending"]["token"].as_u64().expect("forwarded");
    let kt = scene.compound_begin(conn, forward).expect("opens");
    assert!(scene.busy());
    let busy = |e: String| assert!(e.starts_with("DocumentBusy:"), "{e}");
    busy(scene.scene_undo().err().expect("refused").to_string());
    busy(scene.begin_ground_sketch().unwrap_err().to_string());
    busy(scene.replay("{}").unwrap_err().to_string());
    scene.compound_abort(kt).expect("aborts");
    assert!(!scene.busy());
    assert!(scene.begin_ground_sketch().is_ok());
}
