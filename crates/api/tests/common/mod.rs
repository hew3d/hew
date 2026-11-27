//! Frame-driving pieces shared by the curves scoreboard and the
//! determinism golden: the corpus model format, the length and `$ref`
//! substitution a frame goes through, and a session that dispatches
//! frames through the Hew API into a fresh document.
#![allow(dead_code)]

use api::{Connection, DispatchOutcome, Grant, NoHost, Request, RequestId, Response};
use kernel::{Document, EntityRef, ObjectId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub static REGISTRY: std::sync::LazyLock<api::Registry> =
    std::sync::LazyLock::new(api::Registry::protocol_1);

pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/curves")
}

// ------------------------------------------------------------------ rows

/// One API call of a row: `{"method", "params", "as"?}`. `as` labels the
/// result so a later frame can name a value in it with
/// `{"$ref": "<label>#<json pointer>"}`, the form `hew.doc.transact` uses.
#[derive(serde::Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, rename = "as")]
    pub label: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    /// What the case is for, in a sentence. Not read by the runner.
    #[serde(default)]
    #[allow(dead_code)]
    pub note: Option<String>,
    /// Calls made before the swept one, at fixed sizes (an earlier fillet,
    /// for one). Each must succeed, or the case reports its refusal.
    #[serde(default)]
    pub before: Vec<Frame>,
    /// The swept call.
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// The parameter the sweep sets. `radius` for a fillet and `distance`
    /// for a chamfer unless given.
    #[serde(default)]
    pub sweep: Option<String>,
    /// The sizes, as lengths with a unit: `"3mm"`, `"1/8in"`.
    pub sizes: Vec<String>,
    /// Every size up to this one must build. A refusal at or below it is
    /// counted as open work in the summary.
    #[serde(default)]
    pub fits_to: Option<String>,
    /// No size above this one can exist on the shape, so a refusal that
    /// names a limit there is the right answer and a built result is wrong.
    #[serde(default)]
    pub never_above: Option<String>,
    /// `"less"` or `"more"`: which way the volume must move. Left out, any
    /// change within the bound passes.
    #[serde(default)]
    pub volume: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// One line: what the model is.
    #[allow(dead_code)]
    pub title: String,
    /// Where the row came from (a tutorial, a defect, real work).
    #[serde(default)]
    #[allow(dead_code)]
    pub from: Option<String>,
    pub build: Vec<Frame>,
    /// The cases the scoreboard sweeps. A model that is only built (the
    /// determinism golden's extras) has none.
    #[serde(default)]
    pub cases: Vec<Case>,
}

/// A length with a unit, in meters: `3mm`, `2.5cm`, `0.1m`, `1/8in`, `0.5in`.
pub fn parse_length(s: &str) -> Option<f64> {
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
pub fn substitute(v: &Value, labels: &BTreeMap<String, Value>) -> Result<Value, String> {
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
pub struct Session {
    pub doc: Document,
    pub conn: Connection,
    pub next_id: i64,
    pub labels: BTreeMap<String, Value>,
}

/// How a call ended.
pub enum Called {
    Ok(Value),
    /// A typed refusal: its name and detail.
    Refused(String, Value),
    /// Anything else (bad params, an unknown method): a defect in the row.
    Broken(String),
}

impl Session {
    pub fn new() -> Session {
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

    pub fn dispatch(&mut self, method: &str, params: Value) -> Response {
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

    pub fn call(&mut self, method: &str, params: Value) -> Called {
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
    pub fn frame(&mut self, frame: &Frame) -> Called {
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

    pub fn build(model: &Model, name: &str) -> Session {
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

    pub fn object(&self, public: &str) -> Option<ObjectId> {
        match api::IdResolver::new(&self.doc).resolve(public) {
            Some(EntityRef::Object(id)) => Some(id),
            _ => None,
        }
    }
}
