//! The determinism golden (docs/dev/DEVELOPMENT.md, "The determinism
//! golden"): a fixed set of models is rebuilt through the Hew API and, for
//! each, the document's `state_hash` and a hash of every object's
//! tessellation are compared with `corpus/determinism/golden.tsv`.
//!
//! The kernel must produce bit-identical geometry on every platform it
//! ships on, so a platform whose math library rounds differently shows up
//! here as differing rows. The test prints every differing row, expected
//! and actual, so the output of two platforms can be diffed.
//!
//! The set is:
//!
//! - every model under `corpus/curves/` and `corpus/determinism/`, as built;
//! - for each case listed in `corpus/determinism/cases.tsv`, that case run
//!   at its one listed size on a fresh build of its model.
//!
//! Known gap: `Document::state_hash` encodes at an old geometry version
//! (8, or 9 and 10 for an object with dictionaries or curve data), and a
//! face's stored plane is written only from version 11, so the hash does not
//! see it. The mesh hash reads normals only as f32. A platform whose stored
//! f64 plane differs while its vertices and f32 normals agree would pass.
//!
//! The curves cases are the blend commands, which are Preview features, so
//! the whole file is compiled into a preview build only.
//!
//! Environment:
//!
//! - `UPDATE_DETERMINISM=1` rewrites the golden from this run;
//! - `HEW_DETERMINISM_OUT=<path>` writes this run's table to `<path>` and
//!   passes, whatever the golden says, so a platform can produce a file to
//!   diff against the checked-in one.
//!
//! `scripts/determinism.sh` wraps both.
#![cfg(hew_preview)]

use serde_json::json;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

mod common;
use common::*;

const HEADER: &str = "id\toutcome\tstate_hash\tmesh_hash";

fn determinism_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/determinism")
}

// ------------------------------------------------------------------ hash

/// FNV-1a, 64 bits. Written inline: the table only needs a stable digest
/// of exact bits, and the kernel-class crates take no hashing dependency.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    /// A length-prefixed run of floats, as their exact bits.
    fn f32s(&mut self, values: &[f32]) {
        self.u64(values.len() as u64);
        for v in values {
            self.bytes(&v.to_bits().to_le_bytes());
        }
    }

    fn u32s(&mut self, values: &[u32]) {
        self.u64(values.len() as u64);
        for v in values {
            self.bytes(&v.to_le_bytes());
        }
    }
}

/// A digest of the tessellation of every visible object, in the document's
/// own order: the buffers a renderer and the exporters draw from. The flag
/// says an object was missing or failed to tessellate, which the row's
/// outcome then names.
fn mesh_hash(doc: &kernel::Document) -> (u64, bool) {
    let mut h = Fnv::new();
    let mut failed = false;
    let ids = doc.visible_object_ids();
    h.u64(ids.len() as u64);
    for id in ids {
        let Some(object) = doc.object(id) else {
            h.u64(u64::MAX);
            failed = true;
            continue;
        };
        match tessellate::tessellate(object, doc.materials()) {
            Ok(mesh) => {
                h.u64(1);
                h.f32s(&mesh.positions);
                h.f32s(&mesh.normals);
                h.u32s(&mesh.indices);
                h.f32s(&mesh.uvs);
                h.f32s(&mesh.edge_positions);
                h.f32s(&mesh.soft_edge_positions);
                h.f32s(&mesh.tangent_edge_positions);
            }
            Err(_) => {
                h.u64(0);
                failed = true;
            }
        }
    }
    (h.0, failed)
}

// ------------------------------------------------------------------ rows

#[derive(Clone, PartialEq, Debug)]
struct Row {
    id: String,
    /// `built` or `ok`, or what happened instead: `refused:<code>`,
    /// `build_refused:<code>`, `build_broken`, `broken`, `panic`. A
    /// document with an object that would not tessellate adds
    /// `+mesh_error`.
    outcome: String,
    state: u64,
    mesh: u64,
}

impl Row {
    fn of(id: String, outcome: String, doc: &kernel::Document) -> Row {
        let (mesh, failed) = mesh_hash(doc);
        Row {
            id,
            outcome: if failed {
                format!("{outcome}+mesh_error")
            } else {
                outcome
            },
            state: doc.state_hash(),
            mesh,
        }
    }

    /// A row with no document to hash: the run panicked.
    fn panicked(id: String) -> Row {
        Row {
            id,
            outcome: "panic".into(),
            state: 0,
            mesh: 0,
        }
    }

    fn line(&self) -> String {
        format!(
            "{}\t{}\t{:016x}\t{:016x}",
            self.id, self.outcome, self.state, self.mesh
        )
    }

    fn is_clean(&self) -> bool {
        self.outcome == "built" || self.outcome == "ok"
    }
}

/// One listed case: the model, the case id, and the size label.
struct Pick {
    model: String,
    case: String,
    size: String,
}

fn load_picks() -> Vec<Pick> {
    let path = determinism_dir().join("cases.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("model\t") && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                f.len(),
                3,
                "{}: not model, case, size: {l:?}",
                path.display()
            );
            Pick {
                model: f[0].into(),
                case: f[1].into(),
                size: f[2].into(),
            }
        })
        .collect()
}

fn load_dir(dir: &Path, out: &mut BTreeMap<String, Model>) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let model: Model =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            out.insert(name.clone(), model).is_none(),
            "two models are named {name:?}"
        );
    }
}

/// Builds a model. A frame that is refused or broken stops the build and
/// names it as the outcome; the session so far is returned with it.
fn build(model: &Model) -> (Session, Option<String>) {
    let mut s = Session::new();
    for (i, frame) in model.build.iter().enumerate() {
        match s.frame(frame) {
            Called::Ok(_) => {}
            Called::Refused(code, _) => {
                return (s, Some(format!("build_refused:{code}@{i}")));
            }
            Called::Broken(why) => {
                eprintln!("build frame {i} ({}) failed: {why}", frame.method);
                return (s, Some(format!("build_broken@{i}")));
            }
        }
    }
    (s, None)
}

/// The swept call of a case at one size, on the built model: how it ended.
fn run_case(s: &mut Session, case: &Case, size: f64) -> String {
    for (i, frame) in case.before.iter().enumerate() {
        match s.frame(frame) {
            Called::Ok(_) => {}
            Called::Refused(code, _) => return format!("before[{i}] refused:{code}"),
            Called::Broken(why) => {
                eprintln!("{}: before[{i}] failed: {why}", case.id);
                return "broken".into();
            }
        }
    }
    let sweep = case.sweep.clone().unwrap_or_else(|| {
        match case.method.as_str() {
            "hew.solid.chamfer" => "distance",
            _ => "radius",
        }
        .to_string()
    });
    let mut params = match substitute(&case.params, &s.labels) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e}", case.id);
            return "broken".into();
        }
    };
    let Some(map) = params.as_object_mut() else {
        eprintln!("{}: params must be an object", case.id);
        return "broken".into();
    };
    map.insert(sweep, json!(size));
    match s.call(&case.method, params) {
        Called::Ok(_) => "ok".into(),
        Called::Refused(code, _) => format!("refused:{code}"),
        Called::Broken(why) => {
            eprintln!("{}: {why}", case.id);
            "broken".into()
        }
    }
}

/// Runs `f` on a thread of its own, so nothing the engine keeps per thread
/// carries from one row to the next. A panic is a `panic` row, with its
/// message on stderr under the row's id.
fn isolated(id: &str, f: impl FnOnce() -> Row + Send) -> Row {
    let ran = std::thread::scope(|scope| scope.spawn(f).join());
    match ran {
        Ok(row) => row,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("no message");
            eprintln!("{id}: panic: {message}");
            Row::panicked(id.to_string())
        }
    }
}

/// Every row of one model: as built, then each listed case on a fresh
/// build. A model whose build is refused, broken or panics gives that
/// outcome on its own row and on each of its case rows.
fn run_model(name: &str, model: &Model, picks: &[&Pick]) -> Vec<Row> {
    let mut rows = vec![isolated(name, || {
        let (s, failed) = build(model);
        Row::of(
            name.to_string(),
            failed.unwrap_or_else(|| "built".into()),
            &s.doc,
        )
    })];
    for pick in picks {
        let case = model
            .cases
            .iter()
            .find(|c| c.id == pick.case)
            .unwrap_or_else(|| panic!("cases.tsv: {name} has no case {:?}", pick.case));
        assert!(
            case.sizes.contains(&pick.size),
            "cases.tsv: {name}/{} has no size {:?}",
            pick.case,
            pick.size
        );
        let size = parse_length(&pick.size)
            .unwrap_or_else(|| panic!("cases.tsv: {:?} is not a length", pick.size));
        let id = format!("{name}/{}@{}", pick.case, pick.size);
        rows.push(isolated(&id, || {
            let (mut s, failed) = build(model);
            let outcome = match failed {
                Some(outcome) => outcome,
                None => run_case(&mut s, case, size),
            };
            Row::of(id.clone(), outcome, &s.doc)
        }));
    }
    rows
}

fn run_all() -> Vec<Row> {
    let mut models = BTreeMap::new();
    load_dir(&common::corpus_dir(), &mut models);
    load_dir(&determinism_dir(), &mut models);
    let picks = load_picks();
    for p in &picks {
        assert!(
            models.contains_key(&p.model),
            "cases.tsv names no model {:?}",
            p.model
        );
    }
    let started = std::time::Instant::now();
    let rows: Vec<Row> = std::thread::scope(|scope| {
        let running: Vec<_> = models
            .iter()
            .map(|(name, model)| {
                let mine: Vec<&Pick> = picks.iter().filter(|p| &p.model == name).collect();
                scope.spawn(move || run_model(name, model, &mine))
            })
            .collect();
        running
            .into_iter()
            .flat_map(|t| match t.join() {
                Ok(rows) => rows,
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    });
    eprintln!(
        "determinism: {} rows in {:.1?}",
        rows.len(),
        started.elapsed()
    );
    rows
}

// ----------------------------------------------------------------- table

fn render(rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(
        "# The determinism golden. Written by scripts/determinism.sh; do not edit.\n\
         # state_hash is Document::state_hash; mesh_hash is FNV-1a 64 over the exact bits of every\n\
         # visible object's tessellation.\n",
    );
    let _ = writeln!(out, "{HEADER}");
    for r in rows {
        let _ = writeln!(out, "{}", r.line());
    }
    out
}

/// The text with CRLF line endings read as LF.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The rows that did not build or run cleanly, one per line; empty when
/// every row is `built` or `ok`.
fn unclean(rows: &[Row]) -> String {
    rows.iter()
        .filter(|r| !r.is_clean())
        .map(|r| format!("  {}\n", r.line()))
        .collect()
}

fn parse_table(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("id\t") && !l.trim().is_empty())
        .map(|l| {
            let id = l.split('\t').next().unwrap_or_default().to_string();
            (id, l.to_string())
        })
        .collect()
}

/// Every row that differs, expected and actual, one pair per row; empty
/// when the tables agree.
fn differences(expected: &BTreeMap<String, String>, rows: &[Row]) -> String {
    let mut out = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for r in rows {
        seen.insert(r.id.clone());
        let actual = r.line();
        match expected.get(&r.id) {
            Some(e) if *e == actual => {}
            Some(e) => {
                let _ = writeln!(out, "DIFF {}\n  expected {e}\n  actual   {actual}", r.id);
            }
            None => {
                let _ = writeln!(out, "NEW  {}\n  actual   {actual}", r.id);
            }
        }
    }
    for (id, e) in expected {
        if !seen.contains(id) {
            let _ = writeln!(out, "GONE {id}\n  expected {e}");
        }
    }
    out
}

#[test]
fn the_determinism_golden_matches_the_checked_in_table() {
    let rows = run_all();
    let fresh = render(&rows);

    if let Some(path) = std::env::var_os("HEW_DETERMINISM_OUT") {
        std::fs::write(&path, &fresh).unwrap();
        println!(
            "determinism: wrote {} rows to {}",
            rows.len(),
            Path::new(&path).display()
        );
        return;
    }
    let path = determinism_dir().join("golden.tsv");
    if std::env::var_os("UPDATE_DETERMINISM").is_some_and(|v| v == "1") {
        let bad = unclean(&rows);
        assert!(
            bad.is_empty(),
            "the golden is not rewritten while a row is not clean (built or ok), so a failure \
             is not recorded as the expected answer. Fix it, or capture this run with \
             HEW_DETERMINISM_OUT=<file>.\n{bad}"
        );
        std::fs::write(&path, &fresh).unwrap();
        println!(
            "determinism: wrote {} rows to {}",
            rows.len(),
            path.display()
        );
        return;
    }
    // A checkout that turned the table's line endings into CRLF is still
    // the same table.
    let on_disk = lf(&std::fs::read_to_string(&path).unwrap_or_default());
    let diff = differences(&parse_table(&on_disk), &rows);
    assert!(
        diff.is_empty(),
        "the determinism golden differs from corpus/determinism/golden.tsv. If this change moves \
         geometry on purpose, run scripts/determinism.sh and commit the table; if it is a \
         platform, set HEW_DETERMINISM_OUT=<file> to capture its table.\n{diff}"
    );
    assert!(
        on_disk == fresh,
        "corpus/determinism/golden.tsv is not what a run writes (its order or header differs). \
         Run scripts/determinism.sh and commit the result."
    );
}

#[test]
fn every_difference_is_listed_not_just_the_first() {
    let row = |id: &str, state: u64| Row {
        id: id.into(),
        outcome: "ok".into(),
        state,
        mesh: 7,
    };
    let expected = parse_table(&render(&[
        row("a", 1),
        row("b", 2),
        row("c", 3),
        row("d", 4),
    ]));
    let actual = [row("a", 1), row("b", 9), row("c", 8), row("e", 5)];
    let diff = differences(&expected, &actual);
    assert!(!diff.contains("DIFF a\n"));
    for id in ["DIFF b\n", "DIFF c\n", "NEW  e\n", "GONE d\n"] {
        assert!(diff.contains(id), "{id:?} missing from:\n{diff}");
    }
}

#[test]
fn the_hash_reads_exact_bits() {
    let mut a = Fnv::new();
    a.f32s(&[0.0]);
    let mut b = Fnv::new();
    b.f32s(&[-0.0]);
    assert_ne!(a.0, b.0, "signed zero is a different bit pattern");
    let mut c = Fnv::new();
    c.f32s(&[0.0]);
    assert_eq!(a.0, c.0);
}

#[test]
fn a_crlf_table_reads_as_the_same_table() {
    let rows = [Row {
        id: "a".into(),
        outcome: "ok".into(),
        state: 1,
        mesh: 2,
    }];
    let crlf = render(&rows).replace('\n', "\r\n");
    assert_eq!(lf(&crlf), render(&rows));
    assert!(differences(&parse_table(&crlf), &rows).is_empty());
}

#[test]
fn a_row_that_did_not_run_cleanly_is_listed_for_a_rewrite() {
    let row = |id: &str, outcome: &str| Row {
        id: id.into(),
        outcome: outcome.into(),
        state: 0,
        mesh: 0,
    };
    let rows = [
        row("a", "built"),
        row("b", "ok"),
        row("c", "panic"),
        row("d", "ok+mesh_error"),
        row("e", "build_refused:x@0"),
        row("f", "refused:radius_too_large"),
    ];
    let bad = unclean(&rows);
    assert!(!bad.contains("\ta\t") && !bad.contains("\tb\t"));
    for id in ["c", "d", "e", "f"] {
        assert!(
            bad.contains(&format!("  {id}\t")),
            "{id} missing from {bad}"
        );
    }
}

#[test]
fn a_model_that_will_not_build_is_a_row_not_a_failed_run() {
    let model: Model = serde_json::from_value(json!({
        "title": "x",
        "build": [{"method": "hew.solid.extrude", "params": {"region": "rgn_nope", "distance": 1.0}}],
        "cases": []
    }))
    .unwrap();
    let rows = run_model("bad", &model, &[]);
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].outcome.starts_with("build_") && !rows[0].is_clean(),
        "{}",
        rows[0].line()
    );
    let panicked = isolated("boom", || panic!("on purpose"));
    assert_eq!(panicked.outcome, "panic");
}
