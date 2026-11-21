//! The curves scoreboard (docs/dev/DEVELOPMENT.md, "The curves
//! scoreboard"): every model under `corpus/curves/` is rebuilt through the
//! Hew API, every case is run at every size of its sweep, and each run is
//! classed as
//!
//! - `correct`: a valid, watertight solid that passed the checks below;
//! - `limit`: refused, and the refusal names what is in the way;
//! - `generic`: refused without a cause the user could act on;
//! - `wrong`: a result that fails a check, or a panic.
//!
//! The table is checked in as `corpus/curves/SCOREBOARD.tsv`. This test
//! fails when a fresh run differs from it, and says which cases got worse.
//! `scripts/scoreboard.sh` rewrites the table and prints the same
//! comparison.
//!
//! The blend commands are Preview features, so the whole file is compiled
//! into a preview build only.
#![cfg(hew_preview)]

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response};
use kernel::{Document, EntityRef, ObjectId, Point3, WatertightState};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/curves")
}

// ------------------------------------------------------------------ rows

/// One API call of a row: `{"method", "params", "as"?}`. `as` labels the
/// result so a later frame can name a value in it with
/// `{"$ref": "<label>#<json pointer>"}`, the form `hew.doc.transact` uses.
#[derive(serde::Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Frame {
    method: String,
    #[serde(default)]
    params: Value,
    #[serde(default, rename = "as")]
    label: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    /// What the case is for, in a sentence. Not read by the runner.
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
    /// Calls made before the swept one, at fixed sizes (an earlier fillet,
    /// for one). Each must succeed, or the case reports its refusal.
    #[serde(default)]
    before: Vec<Frame>,
    /// The swept call.
    method: String,
    #[serde(default)]
    params: Value,
    /// The parameter the sweep sets. `radius` for a fillet and `distance`
    /// for a chamfer unless given.
    #[serde(default)]
    sweep: Option<String>,
    /// The sizes, as lengths with a unit: `"3mm"`, `"1/8in"`.
    sizes: Vec<String>,
    /// Every size up to this one must build. A refusal at or below it is
    /// counted as open work in the summary.
    #[serde(default)]
    fits_to: Option<String>,
    /// No size above this one can exist on the shape, so a refusal that
    /// names a limit there is the right answer and a built result is wrong.
    #[serde(default)]
    never_above: Option<String>,
    /// `"less"` or `"more"`: which way the volume must move. Left out, any
    /// change within the bound passes.
    #[serde(default)]
    volume: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    /// One line: what the model is.
    #[allow(dead_code)]
    title: String,
    /// Where the row came from (a tutorial, a defect, real work).
    #[serde(default)]
    #[allow(dead_code)]
    from: Option<String>,
    build: Vec<Frame>,
    cases: Vec<Case>,
}

/// A length with a unit, in meters: `3mm`, `2.5cm`, `0.1m`, `1/8in`, `0.5in`.
fn parse_length(s: &str) -> Option<f64> {
    let s = s.trim();
    let (number, scale) = if let Some(n) = s.strip_suffix("mm") {
        (n, 1e-3)
    } else if let Some(n) = s.strip_suffix("cm") {
        (n, 1e-2)
    } else if let Some(n) = s.strip_suffix("in") {
        (n, 0.0254)
    } else {
        (s.strip_suffix('m')?, 1.0)
    };
    let number = number.trim();
    let value = match number.split_once('/') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
            if b == 0.0 {
                return None;
            }
            a / b
        }
        None => number.parse::<f64>().ok()?,
    };
    (value.is_finite()).then_some(value * scale)
}

/// Replaces every `{"$ref": "label#/pointer"}` with the labeled result's
/// value, and every string that is a length with a unit with meters.
fn substitute(v: &Value, labels: &BTreeMap<String, Value>) -> Result<Value, String> {
    Ok(match v {
        Value::Object(map) => {
            if let (1, Some(Value::String(r))) = (map.len(), map.get("$ref")) {
                let (label, pointer) = r
                    .split_once('#')
                    .ok_or_else(|| format!("{r:?} is not \"label#/pointer\""))?;
                let base = labels
                    .get(label)
                    .ok_or_else(|| format!("no earlier frame is labeled {label:?}"))?;
                return base
                    .pointer(pointer)
                    .cloned()
                    .ok_or_else(|| format!("{r:?} names nothing in {base}"));
            }
            let mut out = serde_json::Map::new();
            for (k, x) in map {
                out.insert(k.clone(), substitute(x, labels)?);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|x| substitute(x, labels))
                .collect::<Result<_, _>>()?,
        ),
        Value::String(s) => match parse_length(s) {
            Some(meters) if s.starts_with(|c: char| c.is_ascii_digit() || c == '-') => {
                json!(meters)
            }
            _ => v.clone(),
        },
        other => other.clone(),
    })
}

// --------------------------------------------------------------- session

/// One document and the connection driving it.
struct Session {
    doc: Document,
    conn: Connection,
    next_id: i64,
    labels: BTreeMap<String, Value>,
}

/// A defect in a row, as a panic payload: the run fails with its message
/// where an engine panic becomes a `wrong` line of the table.
struct RowDefect(String);

fn row_defect(message: String) -> ! {
    std::panic::panic_any(RowDefect(message))
}

/// Refusals that say the row is wrong (a locator that finds nothing, a
/// name that does not exist), not what the engine can do.
fn is_row_defect(code: &str) -> bool {
    matches!(
        code,
        "locator_missed"
            | "ambiguous_locator"
            | "unknown_entity"
            | "unknown_profile"
            | "edges_span_objects"
            | "unknown_edge"
            | "empty_spec"
    )
}

/// How a call ended.
enum Called {
    Ok(Value),
    /// A typed refusal: its name and detail.
    Refused(String, Value),
    /// Anything else (bad params, an unknown method): a defect in the row.
    Broken(String),
}

impl Session {
    fn new() -> Session {
        let mut s = Session {
            doc: Document::new(),
            conn: Connection::new(Grant::core(), "scoreboard"),
            next_id: 0,
            labels: BTreeMap::new(),
        };
        for (method, params) in [
            ("hew.meta.hello", json!({ "protocol": 1 })),
            ("hew.doc.attach", json!({})),
        ] {
            match s.call(method, params) {
                Called::Ok(_) => {}
                _ => panic!("{method} failed"),
            }
        }
        s
    }

    fn dispatch(&mut self, method: &str, params: Value) -> Response {
        self.next_id += 1;
        let request = Request {
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(self.next_id)),
            method: method.to_string(),
            params: Some(params),
        };
        match self
            .conn
            .dispatch(&REGISTRY, &mut self.doc, &mut NoHost, request)
        {
            DispatchOutcome::Reply(r) => r,
            _ => panic!("{method} did not reply"),
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Called {
        let r = self.dispatch(method, params);
        match r.error {
            None => {
                let result = r.result.unwrap_or(Value::Null);
                // A mutating command's own result sits inside its envelope.
                Called::Ok(match result.get("results") {
                    Some(inner) => inner[0].clone(),
                    None => result,
                })
            }
            Some(e) => match e.data.as_ref().and_then(|d| d.get("refusal")) {
                Some(Value::String(name)) => Called::Refused(
                    name.clone(),
                    e.data
                        .as_ref()
                        .and_then(|d| d.get("detail"))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                _ => Called::Broken(format!("{}: {}", e.code, e.message)),
            },
        }
    }

    /// Runs one frame, recording its result under its label.
    fn frame(&mut self, frame: &Frame) -> Called {
        let params = match substitute(&frame.params, &self.labels) {
            Ok(p) => p,
            Err(e) => return Called::Broken(e),
        };
        let out = self.call(&frame.method, params);
        if let (Called::Ok(result), Some(label)) = (&out, &frame.label) {
            self.labels.insert(label.clone(), result.clone());
        }
        out
    }

    fn build(model: &Model, name: &str) -> Session {
        let mut s = Session::new();
        for (i, frame) in model.build.iter().enumerate() {
            match s.frame(frame) {
                Called::Ok(_) => {}
                Called::Refused(code, detail) => {
                    panic!(
                        "{name}: build frame {i} ({}) refused {code} {detail}",
                        frame.method
                    )
                }
                Called::Broken(why) => {
                    panic!("{name}: build frame {i} ({}) failed: {why}", frame.method)
                }
            }
        }
        s
    }

    fn object(&self, public: &str) -> Option<ObjectId> {
        match api::IdResolver::new(&self.doc).resolve(public) {
            Some(EntityRef::Object(id)) => Some(id),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- checks

/// What the checks read from an object before and after.
#[derive(Clone, Copy)]
struct Shape {
    volume: f64,
    min: Point3,
    max: Point3,
    edge_length: f64,
}

fn shape_of(doc: &Document, id: ObjectId) -> Option<Shape> {
    let obj = doc.object(id)?;
    let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for v in obj.vertices().values() {
        let p = v.position;
        min = Point3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Point3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    let edge_length = obj
        .edges()
        .keys()
        .filter_map(|e| obj.edge_endpoints(e))
        .map(|(a, b)| (b - a).length())
        .sum();
    Some(Shape {
        volume: obj.volume()?,
        min,
        max,
        edge_length,
    })
}

/// The checks a built result must pass. `None` is a pass; `Some` says which
/// check failed.
fn check_result(
    doc: &Document,
    id: ObjectId,
    before: Option<Shape>,
    size: f64,
    volume: Option<&str>,
) -> Option<String> {
    let Some(obj) = doc.object(id) else {
        return Some("the object is gone".into());
    };
    if let Err(e) = obj.validate() {
        return Some(format!("invalid: {e:?}"));
    }
    if obj.watertight() != WatertightState::Watertight {
        return Some("not watertight".into());
    }
    let (Some(before), Some(after)) = (before, shape_of(doc, id)) else {
        return Some("no volume".into());
    };
    // A blend never reaches outside the shape it was cut from.
    let eps = 1e-9;
    let grew = after.min.x < before.min.x - eps
        || after.min.y < before.min.y - eps
        || after.min.z < before.min.z - eps
        || after.max.x > before.max.x + eps
        || after.max.y > before.max.y + eps
        || after.max.z > before.max.z + eps;
    if grew {
        return Some("grew past its bounds".into());
    }
    // It moves some volume, and no more than a square of the size along
    // every edge the object had (a chamfer moves half of that on its own
    // chain, a round less).
    let moved = after.volume - before.volume;
    if moved == 0.0 {
        return Some("volume unchanged".into());
    }
    if moved.abs() > before.edge_length * size * size {
        return Some(format!(
            "moved {} of volume, more than its edges allow",
            sig(moved.abs() * 1e9, "mm³")
        ));
    }
    match volume {
        Some("less") if moved > 0.0 => return Some("volume grew".into()),
        Some("more") if moved < 0.0 => return Some("volume shrank".into()),
        _ => {}
    }
    None
}

// --------------------------------------------------------------- classes

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Wrong,
    Generic,
    Limit,
    Correct,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Wrong => "wrong",
            Class::Generic => "generic",
            Class::Limit => "limit",
            Class::Correct => "correct",
        }
    }
    fn parse(s: &str) -> Option<Class> {
        Some(match s {
            "wrong" => Class::Wrong,
            "generic" => Class::Generic,
            "limit" => Class::Limit,
            "correct" => Class::Correct,
            _ => return None,
        })
    }
}

/// Three significant digits: enough to read, and steady across platforms.
fn sig(v: f64, unit: &str) -> String {
    if v == 0.0 {
        return format!("0{unit}");
    }
    if !v.is_finite() {
        return format!("{v}{unit}");
    }
    let digits = (2 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.digits$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    format!("{s}{unit}")
}

fn point(v: &Value) -> String {
    // Below a nanometer a coordinate is zero: noise there would print
    // differently from one machine to the next.
    let c = |i: usize| {
        let mm = v[i].as_f64().unwrap_or(f64::NAN) * 1e3;
        sig(if mm.abs() < 1e-6 { 0.0 } else { mm }, "")
    };
    format!("({}, {}, {})mm", c(0), c(1), c(2))
}

/// Classes a refusal: `limit` when it names what is in the way, `generic`
/// when it only says the engine cannot do it. A refusal this table does not
/// know is `generic`, so a new variant has to earn its place here.
fn class_of_refusal(code: &str, detail: &Value, asked: Option<f64>) -> (Class, String) {
    match code {
        "radius_too_large" => {
            let by = detail["limited_by"].as_str().unwrap_or("unknown");
            let largest = detail["max"].as_f64().unwrap_or(f64::NAN);
            let max = sig(largest * 1e3, "mm");
            // A refusal that says a larger size than the one asked for
            // would fit contradicts itself.
            if asked.is_some_and(|size| largest > size * (1.0 + 1e-6)) {
                return (
                    Class::Wrong,
                    format!("{code} max={max} by={by}, above the size refused"),
                );
            }
            let class = if by == "unknown" {
                Class::Generic
            } else {
                Class::Limit
            };
            (class, format!("{code} max={max} by={by}"))
        }
        "profile_too_wide" => (
            Class::Limit,
            format!(
                "{code} needed={} available={}",
                sig(detail["needed"].as_f64().unwrap_or(f64::NAN) * 1e3, "mm"),
                sig(detail["available"].as_f64().unwrap_or(f64::NAN) * 1e3, "mm"),
            ),
        ),
        "angle_does_not_fit" => (
            Class::Limit,
            format!(
                "{code} max={}",
                sig(detail["max"].as_f64().unwrap_or(f64::NAN), "rad")
            ),
        ),
        "blend_faces_flush" | "obstructed" => {
            (Class::Limit, format!("{code} at={}", point(&detail["at"])))
        }
        "profile_needs_convex_edge"
        | "bullnose_needs_parallel_sides"
        | "edge_is_smooth"
        | "stop_out_of_range"
        | "segments_out_of_range"
        | "blend_needs_solid" => (Class::Limit, code.to_string()),
        // Which corner is named first is not steady across platforms: the
        // refusal comes out of a size search that lands within rounding of
        // a seam threshold, so identical corners build or refuse by the last
        // bit. The class is what the board judges.
        "unsupported_vertex" => (Class::Generic, code.to_string()),
        _ => (Class::Generic, code.to_string()),
    }
}

// ------------------------------------------------------------------- run

/// One line of the table.
#[derive(Clone, PartialEq, Debug)]
struct Row {
    model: String,
    case: String,
    size: String,
    /// `fits` (must build), `over` (cannot exist), or empty.
    expect: String,
    class: Class,
    detail: String,
}

impl Row {
    fn key(&self) -> (String, String, String) {
        (self.model.clone(), self.case.clone(), self.size.clone())
    }
    /// How good the answer is, expectation counted in: a named limit where
    /// nothing can exist is as good as a build.
    fn rank(&self) -> u8 {
        match (self.class, self.expect.as_str()) {
            (Class::Limit, "over") => 3,
            (Class::Correct, _) => 3,
            (Class::Limit, _) => 2,
            (Class::Generic, _) => 1,
            (Class::Wrong, _) => 0,
        }
    }
    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.model,
            self.case,
            self.size,
            self.expect,
            self.class.name(),
            self.detail.replace(['\t', '\r', '\n'], " ")
        )
    }
    fn parse(line: &str) -> Option<Row> {
        let f: Vec<&str> = line.split('\t').collect();
        (f.len() == 6 && Class::parse(f[4]).is_some()).then(|| Row {
            model: f[0].into(),
            case: f[1].into(),
            size: f[2].into(),
            expect: f[3].into(),
            class: Class::parse(f[4]).unwrap_or(Class::Wrong),
            detail: f[5].into(),
        })
    }
}

fn length(name: &str, what: &str, s: &str) -> f64 {
    parse_length(s).unwrap_or_else(|| panic!("{name}: {what} {s:?} is not a length with a unit"))
}

/// Undoes back to the built model and says whether that restored it.
fn rewind(s: &mut Session, depth: usize, hash: u64) -> bool {
    while s.doc.undo_depth() > depth {
        if s.doc.undo().is_err() {
            return false;
        }
    }
    s.doc.undo_depth() == depth && s.doc.state_hash() == hash
}

fn run_model(name: &str, model: &Model, rows: &mut Vec<Row>) {
    let mut session = Session::build(model, name);
    let depth = session.doc.undo_depth();
    let hash = session.doc.state_hash();
    let labels = session.labels.clone();

    for case in &model.cases {
        let sweep = case.sweep.clone().unwrap_or_else(|| {
            match case.method.as_str() {
                "hew.solid.chamfer" => "distance",
                _ => "radius",
            }
            .to_string()
        });
        let fits_to = case.fits_to.as_deref().map(|s| length(name, "fits_to", s));
        let never_above = case
            .never_above
            .as_deref()
            .map(|s| length(name, "never_above", s));
        for size_label in &case.sizes {
            let size = length(name, "size", size_label);
            let expect = if never_above.is_some_and(|m| size > m * (1.0 + 1e-9)) {
                "over"
            } else if fits_to.is_some_and(|m| size <= m * (1.0 + 1e-9)) {
                "fits"
            } else {
                ""
            };
            // Each run gets a thread of its own: a panic anywhere in it is
            // caught at the join, and nothing the engine keeps per thread
            // outlives it.
            let ran = std::thread::scope(|scope| {
                let session = &mut session;
                let sweep = sweep.as_str();
                scope
                    .spawn(move || run_case(session, name, case, sweep, size))
                    .join()
            });
            let (mut class, mut detail, panicked) = match ran {
                Ok((class, detail)) => (class, detail, false),
                Err(payload) => {
                    if let Some(RowDefect(message)) = payload.downcast_ref::<RowDefect>() {
                        panic!("{message}");
                    }
                    // The message without its source location, so the
                    // table does not change when a line number does.
                    let message = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("no message");
                    let line = message.lines().next().unwrap_or_default();
                    let short: String = line.chars().take(120).collect();
                    (Class::Wrong, format!("panic: {short}"), true)
                }
            };
            if class == Class::Correct && expect == "over" {
                class = Class::Wrong;
                detail = "built at a size the shape cannot hold".into();
            }
            // A panic can leave the document half changed: start over
            // rather than trust an undo of it.
            if panicked || !rewind(&mut session, depth, hash) {
                if class != Class::Wrong {
                    class = Class::Wrong;
                    detail = "undo did not restore the model".into();
                }
                session = Session::build(model, name);
                assert!(
                    session.doc.undo_depth() == depth && session.doc.state_hash() == hash,
                    "{name}: the model did not rebuild the same"
                );
            }
            session.labels = labels.clone();
            rows.push(Row {
                model: name.to_string(),
                case: case.id.clone(),
                size: size_label.clone(),
                expect: expect.to_string(),
                class,
                detail,
            });
        }
    }
}

fn run_case(s: &mut Session, name: &str, case: &Case, sweep: &str, size: f64) -> (Class, String) {
    for (i, frame) in case.before.iter().enumerate() {
        match s.frame(frame) {
            Called::Ok(_) => {}
            Called::Refused(code, detail) => {
                if is_row_defect(&code) {
                    row_defect(format!("{name}/{}: before[{i}]: {code} {detail}", case.id));
                }
                let (class, text) = class_of_refusal(&code, &detail, None);
                return (class, format!("before[{i}]: {text}"));
            }
            Called::Broken(why) => {
                row_defect(format!("{name}/{}: before[{i}] failed: {why}", case.id))
            }
        }
    }
    let mut params = substitute(&case.params, &s.labels)
        .unwrap_or_else(|e| row_defect(format!("{name}/{}: {e}", case.id)));
    params
        .as_object_mut()
        .unwrap_or_else(|| row_defect(format!("{name}/{}: params must be an object", case.id)))
        .insert(sweep.to_string(), json!(size));

    // The object is not known until the call answers, so every live object
    // is measured first.
    let before: BTreeMap<ObjectId, Shape> = s
        .doc
        .live_object_ids()
        .into_iter()
        .filter_map(|id| shape_of(&s.doc, id).map(|shape| (id, shape)))
        .collect();

    match s.call(&case.method, params) {
        Called::Broken(why) => row_defect(format!("{name}/{}: {why}", case.id)),
        Called::Refused(code, detail) => {
            if is_row_defect(&code) {
                row_defect(format!("{name}/{}: {code} {detail}", case.id));
            }
            class_of_refusal(&code, &detail, Some(size))
        }
        Called::Ok(result) => {
            let Some(id) = result["object_id"].as_str().and_then(|p| s.object(p)) else {
                return (Class::Wrong, "the result names no object".into());
            };
            // The engine says what size it built. Only the plain forms
            // report the swept number back.
            let plain = matches!(sweep, "radius" | "distance")
                && case.params.get("profile").is_none()
                && case.params.get("angle").is_none();
            if let (true, Some(built)) = (plain, result["size"].as_f64())
                && (built - size).abs() > 1e-9 * size.max(1.0)
            {
                return (
                    Class::Wrong,
                    format!(
                        "built at {}, not the size asked for",
                        sig(built * 1e3, "mm")
                    ),
                );
            }
            match check_result(
                &s.doc,
                id,
                before.get(&id).copied(),
                size,
                case.volume.as_deref(),
            ) {
                Some(why) => (Class::Wrong, why),
                None => (Class::Correct, String::new()),
            }
        }
    }
}

fn load_models() -> Vec<(String, Model)> {
    let dir = corpus_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let model: Model =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut seen = std::collections::BTreeSet::new();
            for case in &model.cases {
                assert!(
                    seen.insert(case.id.clone()),
                    "{name}: two cases are named {:?}",
                    case.id
                );
                assert!(!case.sizes.is_empty(), "{name}/{}: no sizes", case.id);
                let mut sizes = std::collections::BTreeSet::new();
                for size in &case.sizes {
                    assert!(
                        sizes.insert(size),
                        "{name}/{}: size {size:?} is listed twice",
                        case.id
                    );
                }
            }
            (name, model)
        })
        .collect()
}

// ----------------------------------------------------------------- table

const HEADER: &str = "model\tcase\tsize\texpect\tclass\tdetail";

fn render(rows: &[Row]) -> String {
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# The curves scoreboard. Written by scripts/scoreboard.sh; do not edit."
    );
    let _ = writeln!(
        out,
        "# {} cases: {} correct, {} limit, {} generic, {} wrong.",
        rows.len(),
        count(&|r| r.class == Class::Correct),
        count(&|r| r.class == Class::Limit),
        count(&|r| r.class == Class::Generic),
        count(&|r| r.class == Class::Wrong),
    );
    let _ = writeln!(
        out,
        "# Open: {} refused or wrong at a size that must build; {} limits where nothing larger can exist.",
        count(&|r| r.expect == "fits" && r.class != Class::Correct),
        count(&|r| r.expect == "over" && r.class == Class::Limit),
    );
    let _ = writeln!(out, "{HEADER}");
    for r in rows {
        let _ = writeln!(out, "{}", r.line());
    }
    out
}

fn parse_table(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.starts_with('#') && *l != HEADER && !l.is_empty())
        .map(|l| Row::parse(l).unwrap_or_else(|| panic!("not a line of the table: {l:?}")))
        .collect()
}

/// The comparison, worst news first. Empty when the tables agree.
fn compare(old: &[Row], new: &[Row]) -> String {
    let old_by: BTreeMap<_, _> = old.iter().map(|r| (r.key(), r)).collect();
    let new_by: BTreeMap<_, _> = new.iter().map(|r| (r.key(), r)).collect();
    let mut worse = Vec::new();
    let mut better = Vec::new();
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut gone = Vec::new();
    let show = |r: &Row| {
        if r.detail.is_empty() {
            r.class.name().to_string()
        } else {
            format!("{} ({})", r.class.name(), r.detail)
        }
    };
    for r in new {
        let id = format!("{}/{} at {}", r.model, r.case, r.size);
        match old_by.get(&r.key()) {
            None => added.push(format!("  {id}: {}", show(r))),
            Some(o) if **o == *r => {}
            Some(o) => {
                let line = format!("  {id}: {} -> {}", show(o), show(r));
                match r.rank().cmp(&o.rank()) {
                    std::cmp::Ordering::Less => worse.push(line),
                    std::cmp::Ordering::Greater => better.push(line),
                    std::cmp::Ordering::Equal => changed.push(line),
                }
            }
        }
    }
    for o in old {
        if !new_by.contains_key(&o.key()) {
            gone.push(format!(
                "  {}/{} at {}: {}",
                o.model,
                o.case,
                o.size,
                show(o)
            ));
        }
    }
    let mut out = String::new();
    for (title, lines) in [
        ("WORSE", worse),
        ("BETTER", better),
        ("CHANGED (same class of answer)", changed),
        ("NEW", added),
        ("GONE", gone),
    ] {
        if !lines.is_empty() {
            let _ = writeln!(out, "{title}: {}", lines.len());
            for l in lines {
                let _ = writeln!(out, "{l}");
            }
        }
    }
    out
}

#[test]
fn the_scoreboard_matches_the_checked_in_table() {
    // Models are independent, so each runs on its own thread; the table
    // keeps the models in name order whatever finishes first.
    let models = load_models();
    let rows: Vec<Row> = std::thread::scope(|scope| {
        let running: Vec<_> = models
            .iter()
            .map(|(name, model)| {
                scope.spawn(move || {
                    let started = std::time::Instant::now();
                    let mut rows = Vec::new();
                    run_model(name, model, &mut rows);
                    eprintln!("{name}: {:.1?}", started.elapsed());
                    rows
                })
            })
            .collect();
        running
            .into_iter()
            .flat_map(|thread| match thread.join() {
                Ok(rows) => rows,
                // A defect in a row: fail with its message.
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    });

    let path = corpus_dir().join("SCOREBOARD.tsv");
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    let old = parse_table(&on_disk);
    let fresh = render(&rows);
    let diff = compare(&old, &rows);
    let summary: String = fresh.lines().take(3).skip(1).collect::<Vec<_>>().join("\n");

    if std::env::var_os("UPDATE_SCOREBOARD").is_some_and(|v| v == "1") {
        std::fs::write(&path, &fresh).unwrap();
        println!("{summary}");
        if diff.is_empty() {
            println!("# No case changed since the last run.");
        } else {
            println!("{diff}");
        }
        return;
    }
    assert!(
        diff.is_empty(),
        "the curves scoreboard changed. Run scripts/scoreboard.sh, read the comparison, and \
         commit corpus/curves/SCOREBOARD.tsv with the change.\n{diff}"
    );
    // The same cases in another order, or an edited summary: the file must
    // be exactly what the script writes.
    assert!(
        on_disk == fresh,
        "corpus/curves/SCOREBOARD.tsv is not what a run writes (its order or its summary \
         differs). Run scripts/scoreboard.sh and commit the result."
    );
}

#[test]
fn lengths_parse_with_their_units() {
    let close = |s: &str, want: f64| {
        let got = parse_length(s).unwrap_or_else(|| panic!("{s} parses"));
        assert!((got - want).abs() < 1e-12, "{s}: {got} vs {want}");
    };
    close("3mm", 0.003);
    close("2.5cm", 0.025);
    close("0.1m", 0.1);
    close("1/8in", 0.003175);
    close("0.5in", 0.0127);
    close("-2mm", -0.002);
    assert_eq!(parse_length("round"), None);
    assert_eq!(parse_length("1/0in"), None);
    assert_eq!(parse_length("12"), None);
}

#[test]
fn a_worse_case_is_named_first() {
    let row = |size: &str, class, detail: &str| Row {
        model: "box".into(),
        case: "top".into(),
        size: size.into(),
        expect: String::new(),
        class,
        detail: detail.into(),
    };
    let old = [
        row("1mm", Class::Correct, ""),
        row("2mm", Class::Generic, "unsupported_edge"),
        row(
            "3mm",
            Class::Limit,
            "radius_too_large max=2mm by=adjacent_face",
        ),
    ];
    let new = [
        row(
            "1mm",
            Class::Limit,
            "radius_too_large max=0.5mm by=short_edge",
        ),
        row("2mm", Class::Correct, ""),
        row(
            "3mm",
            Class::Limit,
            "radius_too_large max=2.5mm by=adjacent_face",
        ),
        row("4mm", Class::Wrong, "not watertight"),
    ];
    let diff = compare(&old, &new);
    assert!(diff.starts_with("WORSE: 1\n  box/top at 1mm: correct -> limit"));
    assert!(diff.contains("BETTER: 1\n  box/top at 2mm"));
    assert!(diff.contains("CHANGED (same class of answer): 1\n  box/top at 3mm"));
    assert!(diff.contains("NEW: 1\n  box/top at 4mm: wrong (not watertight)"));
    assert_eq!(compare(&new, &new), "");
    // The table round-trips through its file form.
    assert_eq!(parse_table(&render(&new)), new);
    // A tab in a detail cannot split its line into more fields.
    let tabbed = row("5mm", Class::Wrong, "panic: a\tb");
    let read = parse_table(&render(std::slice::from_ref(&tabbed)));
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].detail, "panic: a b");
    // A line that is not a row is refused, not skipped.
    assert!(Row::parse("box\ttop\t1mm\t\tCorrect\t").is_none());
    assert!(Row::parse("box\ttop\t1mm\tcorrect").is_none());
}

#[test]
fn a_refusal_that_contradicts_itself_is_wrong() {
    let detail = json!({ "max": 0.004, "limited_by": "adjacent_face" });
    // Refused at 5 mm with 4 mm as the largest that fits: a named limit.
    assert_eq!(
        class_of_refusal("radius_too_large", &detail, Some(0.005)).0,
        Class::Limit
    );
    // Refused at 3 mm while saying 4 mm fits.
    assert_eq!(
        class_of_refusal("radius_too_large", &detail, Some(0.003)).0,
        Class::Wrong
    );
    let unknown = json!({ "max": 0.004, "limited_by": "unknown" });
    assert_eq!(
        class_of_refusal("radius_too_large", &unknown, Some(0.005)).0,
        Class::Generic
    );
    assert_eq!(
        class_of_refusal("unsupported_edge", &json!({}), Some(0.005)).0,
        Class::Generic
    );
    assert!(is_row_defect("ambiguous_locator"));
    assert!(!is_row_defect("radius_too_large"));
}
