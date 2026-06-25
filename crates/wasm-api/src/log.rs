//! The wasm→shell logging seam (docs/DEVELOPMENT.md).
//!
//! The kernel runs as WASM and rule 1 forbids it any I/O, so **the shell owns the
//! sink** — exactly like the `FileHost` / `RecoveryStore` seams. This
//! module is the kernel-side half of that seam:
//!
//! ```text
//! kernel/inference `tracing` events
//!   → DrainSubscriber (here) → one JSON LogRecord per event
//!     → drain closure → JS callback (set_log_drain)  → TS sink → rolling file
//!                     ↘ in-memory ring buffer (drain_buffer)  → web download
//! ```
//!
//! The record schema is the **stub-first handshake** for  (the TS sink) and
//! is documented in `docs/DIAGNOSTICS.md`. The kernel emits plain `tracing` events
//! (op name + params); this layer stamps the cross-stream **monotonic seq#** and
//! the per-gesture **correlation id** that let the kernel stream and the TS stream
//! merge into one timeline (docs/DEVELOPMENT.md). Correlation ids are managed here
//! (`begin_gesture`/`end_gesture`) rather than via `tracing` spans, so a minimal
//! event-only `Subscriber` suffices and the WASM bundle stays lean.

use std::cell::{Cell, RefCell};

use serde::Serialize;
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

/// One structured diagnostic record — the JSON shape the TS sink consumes
/// (`docs/DIAGNOSTICS.md`). One per `tracing` event.
#[derive(Debug, Clone, Serialize)]
pub struct LogRecord {
    /// Monotonic sequence number across the whole session — the merge key shared
    /// with the TS-side stream. Strictly increasing, never reused.
    pub seq: u64,
    /// Correlation id of the gesture this event belongs to, or `null` outside any
    /// gesture. Lets the log be filtered to a single user gesture.
    pub corr: Option<u64>,
    /// Severity: `"TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR"`.
    pub level: &'static str,
    /// The event target — the emitting module path, e.g. `"kernel::cmd"`.
    pub target: String,
    /// Structured key/value payload. Well-known keys (see docs/DIAGNOSTICS.md):
    /// `message`, `op` (command name), `state_hash` (post-op [`Document`] digest).
    /// Keys are sorted (serde_json default `Map` = `BTreeMap`) so a record
    /// serializes deterministically.
    pub fields: Map<String, Value>,
}

/// Default cap on the in-memory ring buffer when no JS drain is installed (web
/// path holds the tail until downloaded). Oldest records drop first.
const RING_CAP: usize = 50_000;

/// A drain: receives each finished record's JSON string (the wasm wrapper
/// forwards to a JS callback).
type Drain = Box<dyn FnMut(&str)>;

// All drain state is thread-local. WASM in the webview is single-threaded, so a
// thread-local is exactly session-global there; in the native test harness it
// additionally isolates each (parallel) test, so seq/correlation assertions are
// deterministic. The `DrainSubscriber` is a ZST regardless, so it stays
// `Send + Sync` as `set_global_default` requires.
thread_local! {
    /// The installed drain. When present, every record's JSON is handed to it
    /// (the wasm wrapper forwards to a JS callback). When absent, records fall
    /// into [`BUFFER`].
    static SINK: RefCell<Option<Drain>> = const { RefCell::new(None) };
    /// In-memory ring buffer used when no `SINK` is installed.
    static BUFFER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Monotonic sequence counter — the merge key shared with the TS stream.
    static SEQ: Cell<u64> = const { Cell::new(0) };
    /// Current correlation id; 0 means "outside any gesture" (→ `corr: None`).
    static CORR: Cell<u64> = const { Cell::new(0) };
    /// Allocates the next gesture id; never 0.
    static NEXT_CORR: Cell<u64> = const { Cell::new(1) };
    /// Runtime capture level (see [`level_to_usize`]); events stricter than this
    /// are dropped at `enabled()`. Default = INFO.
    static MAX_LEVEL: Cell<usize> = const { Cell::new(3) };
    /// Dummy span-id source (spans are not tracked; ids must be non-zero).
    static SPAN_IDS: Cell<u64> = const { Cell::new(1) };
}

fn level_to_usize(level: &Level) -> usize {
    match *level {
        Level::ERROR => 1,
        Level::WARN => 2,
        Level::INFO => 3,
        Level::DEBUG => 4,
        Level::TRACE => 5,
    }
}

fn level_to_str(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

/// Sets the maximum captured severity by name (`"trace"|"debug"|"info"|"warn"|
/// "error"`); unknown names leave it unchanged. Default INFO.
pub fn set_capture_level(name: &str) {
    let lvl = match name.to_ascii_lowercase().as_str() {
        "error" => 1,
        "warn" => 2,
        "info" => 3,
        "debug" => 4,
        "trace" => 5,
        _ => return,
    };
    MAX_LEVEL.with(|m| m.set(lvl));
}

/// Opens a new correlation scope (a user gesture) and returns its id. Subsequent
/// events carry this id until [`end_gesture`] (or another `begin_gesture`).
pub fn begin_gesture() -> u64 {
    let id = NEXT_CORR.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    CORR.with(|c| c.set(id));
    id
}

/// Closes the current correlation scope; later events carry `corr: null`.
pub fn end_gesture() {
    CORR.with(|c| c.set(0));
}

/// Installs the drain closure (the wasm wrapper forwards to a JS callback).
pub fn set_drain(drain: Drain) {
    SINK.with(|s| *s.borrow_mut() = Some(drain));
}

/// Removes the drain; later records fall into the ring buffer.
pub fn clear_drain() {
    SINK.with(|s| *s.borrow_mut() = None);
}

/// Takes and clears the buffered JSON records (web on-demand download path).
pub fn drain_buffer() -> Vec<String> {
    BUFFER.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

/// Test/utility: reset sequence + correlation + buffer to a pristine state.
#[cfg(test)]
pub fn reset() {
    SEQ.with(|s| s.set(0));
    CORR.with(|c| c.set(0));
    NEXT_CORR.with(|n| n.set(1));
    MAX_LEVEL.with(|m| m.set(3));
    clear_drain();
    BUFFER.with(|b| b.borrow_mut().clear());
}

/// Routes one finished record's JSON to the installed drain, or the ring buffer.
fn emit(json: String) {
    SINK.with(|s| {
        if let Some(drain) = s.borrow_mut().as_mut() {
            drain(&json);
            return;
        }
        BUFFER.with(|b| {
            let mut buf = b.borrow_mut();
            if buf.len() >= RING_CAP {
                buf.remove(0);
            }
            buf.push(json);
        });
    });
}

/// Collects a `tracing` event's fields into a JSON object.
struct FieldVisitor(Map<String, Value>);

impl FieldVisitor {
    fn new() -> Self {
        FieldVisitor(Map::new())
    }
    fn put(&mut self, field: &Field, value: Value) {
        self.0.insert(field.name().to_string(), value);
    }
}

impl Visit for FieldVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        // JSON has no NaN/Inf; fall back to a string so the record stays valid.
        match serde_json::Number::from_f64(value) {
            Some(n) => self.put(field, Value::Number(n)),
            None => self.put(field, Value::String(value.to_string())),
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::Number(value.into()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::Bool(value));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, Value::String(value.to_string()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, Value::String(format!("{value:?}")));
    }
}

/// The event-only `Subscriber` that turns each `tracing` event into a
/// [`LogRecord`] and forwards its JSON. A ZST: all state is in module statics /
/// thread-locals, so it is trivially `Send + Sync` (required by
/// `set_global_default`).
pub struct DrainSubscriber;

impl Subscriber for DrainSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        level_to_usize(metadata.level()) <= MAX_LEVEL.with(|m| m.get())
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        // Spans are not tracked; hand back a unique non-zero id so the contract
        // (`Id` must be non-zero) holds.
        span::Id::from_u64(SPAN_IDS.with(|s| {
            let id = s.get();
            s.set(id + 1);
            id
        }))
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if level_to_usize(metadata.level()) > MAX_LEVEL.with(|m| m.get()) {
            return;
        }
        let mut visitor = FieldVisitor::new();
        event.record(&mut visitor);
        let corr = match CORR.with(|c| c.get()) {
            0 => None,
            n => Some(n),
        };
        let record = LogRecord {
            seq: SEQ.with(|s| {
                let v = s.get();
                s.set(v + 1);
                v
            }),
            corr,
            level: level_to_str(metadata.level()),
            target: metadata.target().to_string(),
            fields: visitor.0,
        };
        // Serialization of a Map<String, Value> over plain JSON values cannot
        // fail; fall back to an empty object on the impossible error.
        let json = serde_json::to_string(&record).unwrap_or_else(|_| "{}".to_string());
        emit(json);
    }

    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::with_default;

    /// Capture every record emitted while running `body` under the drain
    /// subscriber, returned as parsed JSON values (deterministic, no JS).
    fn capture(body: impl FnOnce()) -> Vec<Value> {
        reset();
        with_default(DrainSubscriber, body);
        drain_buffer()
            .into_iter()
            .map(|s| serde_json::from_str(&s).expect("record is valid JSON"))
            .collect()
    }

    #[test]
    fn event_becomes_a_record_with_seq_and_fields() {
        let records = capture(|| {
            tracing::info!(target: "kernel::cmd", op = "extrude_region", height = 2.5_f64);
        });
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r["seq"], 0);
        assert_eq!(r["corr"], Value::Null);
        assert_eq!(r["level"], "INFO");
        assert_eq!(r["target"], "kernel::cmd");
        assert_eq!(r["fields"]["op"], "extrude_region");
        assert_eq!(r["fields"]["height"], 2.5);
    }

    #[test]
    fn seq_is_monotonic_across_events() {
        let records = capture(|| {
            tracing::info!(target: "kernel::cmd", op = "a");
            tracing::info!(target: "kernel::cmd", op = "b");
            tracing::info!(target: "kernel::cmd", op = "c");
        });
        let seqs: Vec<u64> = records.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, vec![0, 1, 2], "seq is strictly increasing, no reuse");
    }

    #[test]
    fn correlation_id_scopes_a_gesture() {
        let records = capture(|| {
            tracing::info!(target: "kernel::cmd", op = "before");
            let g = begin_gesture();
            assert!(g > 0);
            tracing::info!(target: "kernel::cmd", op = "during1");
            tracing::info!(target: "kernel::cmd", op = "during2");
            end_gesture();
            tracing::info!(target: "kernel::cmd", op = "after");
        });
        assert_eq!(records[0]["corr"], Value::Null, "before the gesture");
        let g = records[1]["corr"].as_u64().unwrap();
        assert!(g > 0);
        assert_eq!(records[2]["corr"].as_u64().unwrap(), g, "same gesture id");
        assert_eq!(records[3]["corr"], Value::Null, "after end_gesture");
    }

    #[test]
    fn capture_level_filters_below_threshold() {
        reset();
        with_default(DrainSubscriber, || {
            // Default INFO: debug/trace are dropped.
            tracing::debug!(target: "kernel::cmd", op = "noisy");
            tracing::info!(target: "kernel::cmd", op = "kept");
        });
        let records = drain_buffer();
        assert_eq!(records.len(), 1, "debug event filtered out at default INFO");

        reset();
        set_capture_level("trace");
        with_default(DrainSubscriber, || {
            tracing::debug!(target: "kernel::cmd", op = "now_kept");
        });
        assert_eq!(drain_buffer().len(), 1, "raising the level captures debug");
    }

    #[test]
    fn installed_drain_receives_records_instead_of_buffer() {
        use std::cell::RefCell;
        use std::rc::Rc;
        reset();
        let sink = Rc::new(RefCell::new(Vec::<String>::new()));
        let sink2 = sink.clone();
        set_drain(Box::new(move |s: &str| {
            sink2.borrow_mut().push(s.to_string())
        }));
        with_default(DrainSubscriber, || {
            tracing::info!(target: "kernel::cmd", op = "to_drain");
        });
        assert_eq!(sink.borrow().len(), 1, "record went to the installed drain");
        assert!(drain_buffer().is_empty(), "and not to the ring buffer");
    }
}
