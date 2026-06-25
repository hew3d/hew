//! Command-stream recording (docs/DEVELOPMENT.md).
//!
//! "Once Layer 1 is structured, the log essentially *is* a recording"
//! (docs/DEVELOPMENT.md). So this module is a **tap on the log stream** rather than
//! a parallel capture path: every [`LogRecord`] the `DrainSubscriber` builds is
//! also offered to [`observe`], and when recording is enabled it pairs each
//! `kernel::op` event (the command name + its params) with the following
//! `kernel::cmd` event (the post-op `state_hash`) into one
//! [`RecordedCommand`], threading a **pre/post `state_hash` chain** through the
//! whole session.
//!
//! The resulting [`Recording`] is simultaneously a **bug reproducer** (attach to
//! an issue) and, once a bug is fixed, the seed of a **permanent regression
//! test** — replay it and assert the final `post_hash` matches the golden
//!. The artifact JSON shape is frozen in `docs/DIAGNOSTICS.md`, the
//! stub-first handshake for the replay runner and bug-report bundle.
//!
//! Only **committed** commands are recorded: a `kernel::op` whose op then errors
//! emits no `kernel::cmd`, so its pending entry is simply overwritten by the next
//! op and never lands in the recording (it changed no state). A committed
//! mutation that has no `kernel::op` instrumentation yet (e.g. `paint_face`) still
//! lands as a command with `op: null` — the `state_hash` chain stays complete
//! either way, which is what replay/golden assertions rely on.

use std::cell::{Cell, RefCell};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::log::LogRecord;

/// Bump on any breaking change to the [`Recording`] JSON shape (mirrors the
/// `.hew` `MANIFEST` version discipline). See `docs/DIAGNOSTICS.md`.
pub const RECORDING_FORMAT_VERSION: u32 = 1;

/// One committed Document command: its name + params, bracketed by the canonical
/// `state_hash` immediately before and after it applied.
#[derive(Debug, Clone, Serialize)]
pub struct RecordedCommand {
    /// The `seq` of the originating `kernel::op` event (or the `kernel::cmd`
    /// event when the op was not instrumented) — ties back into the full log.
    pub seq: u64,
    /// Correlation id of the user gesture, or `null` outside any gesture.
    pub corr: Option<u64>,
    /// Command name (the `Document` method), or `null` for a committed mutation
    /// whose op is not yet instrumented with a `kernel::op` event.
    pub op: Option<String>,
    /// The command's structured params (the `kernel::op` fields minus `op`).
    pub params: Map<String, Value>,
    /// Canonical `state_hash` of the document *before* this command applied
    /// (= the previous command's `post_hash`, or the recording's baseline).
    pub pre_hash: u64,
    /// Canonical `state_hash` of the document *after* this command applied.
    pub post_hash: u64,
}

/// A complete recorded session: an ordered command stream with a `state_hash`
/// chain anchored at `baseline_hash`. Replaying the commands into a document
/// that starts at `baseline_hash` must reproduce each `post_hash`.
#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    /// Format version (`RECORDING_FORMAT_VERSION`).
    pub version: u32,
    /// `state_hash` of the document when recording started — the chain anchor
    /// and the first command's `pre_hash`.
    pub baseline_hash: u64,
    /// The committed commands, in application order.
    pub commands: Vec<RecordedCommand>,
}

/// A command awaiting its `kernel::cmd` (post-hash) partner.
struct Pending {
    seq: u64,
    corr: Option<u64>,
    op: Option<String>,
    params: Map<String, Value>,
}

thread_local! {
    /// Whether observation is active (toggled by [`start`]/[`stop`]).
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    /// Recorded commands so far.
    static COMMANDS: RefCell<Vec<RecordedCommand>> = const { RefCell::new(Vec::new()) };
    /// The `kernel::op` event awaiting its `kernel::cmd` partner.
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    /// The baseline (recording start) hash — first command's `pre_hash`.
    static BASELINE: Cell<u64> = const { Cell::new(0) };
    /// The most recent `post_hash` — the next command's `pre_hash`.
    static LAST_HASH: Cell<u64> = const { Cell::new(0) };
}

/// Begins a recording anchored at `baseline_hash` (the document's current
/// `state_hash`). Discards any prior in-progress recording.
pub fn start(baseline_hash: u64) {
    COMMANDS.with(|c| c.borrow_mut().clear());
    PENDING.with(|p| *p.borrow_mut() = None);
    BASELINE.with(|b| b.set(baseline_hash));
    LAST_HASH.with(|h| h.set(baseline_hash));
    ENABLED.with(|e| e.set(true));
}

/// Stops observing. The accumulated commands remain available to [`take`].
pub fn stop() {
    ENABLED.with(|e| e.set(false));
}

/// Whether a recording is currently active.
pub fn is_active() -> bool {
    ENABLED.with(|e| e.get())
}

/// Takes the recording built so far, clearing the buffer (and any pending op).
pub fn take() -> Recording {
    let commands = COMMANDS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    PENDING.with(|p| *p.borrow_mut() = None);
    Recording {
        version: RECORDING_FORMAT_VERSION,
        baseline_hash: BASELINE.with(|b| b.get()),
        commands,
    }
}

/// Offered every [`LogRecord`] the subscriber builds; a no-op unless recording
/// is active. Pairs `kernel::op` → `kernel::cmd` into a [`RecordedCommand`].
pub fn observe(record: &LogRecord) {
    if !ENABLED.with(|e| e.get()) {
        return;
    }
    match record.target.as_str() {
        "kernel::op" => {
            let mut params = record.fields.clone();
            let op = params
                .remove("op")
                .and_then(|v| v.as_str().map(str::to_string));
            PENDING.with(|p| {
                *p.borrow_mut() = Some(Pending {
                    seq: record.seq,
                    corr: record.corr,
                    op,
                    params,
                });
            });
        }
        "kernel::cmd" => {
            // The post-op state_hash is required to extend the chain; without it
            // there is nothing to record.
            let Some(post) = record.fields.get("state_hash").and_then(Value::as_u64) else {
                return;
            };
            let pre = LAST_HASH.with(|h| h.get());
            let pending = PENDING.with(|p| p.borrow_mut().take());
            let cmd = match pending {
                Some(p) => RecordedCommand {
                    seq: p.seq,
                    corr: p.corr,
                    op: p.op,
                    params: p.params,
                    pre_hash: pre,
                    post_hash: post,
                },
                // A committed mutation with no instrumented op event: keep the
                // chain complete with a null-op entry.
                None => RecordedCommand {
                    seq: record.seq,
                    corr: record.corr,
                    op: None,
                    params: Map::new(),
                    pre_hash: pre,
                    post_hash: post,
                },
            };
            COMMANDS.with(|c| c.borrow_mut().push(cmd));
            LAST_HASH.with(|h| h.set(post));
        }
        _ => {}
    }
}

/// Test/utility: clear all recorder state.
#[cfg(test)]
pub fn reset() {
    ENABLED.with(|e| e.set(false));
    COMMANDS.with(|c| c.borrow_mut().clear());
    PENDING.with(|p| *p.borrow_mut() = None);
    BASELINE.with(|b| b.set(0));
    LAST_HASH.with(|h| h.set(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op_record(seq: u64, op: &str, key: &str, val: f64) -> LogRecord {
        let mut fields = Map::new();
        fields.insert("op".into(), Value::String(op.into()));
        fields.insert(
            key.into(),
            Value::Number(serde_json::Number::from_f64(val).unwrap()),
        );
        LogRecord {
            seq,
            corr: Some(1),
            level: "INFO",
            target: "kernel::op".into(),
            fields,
        }
    }

    fn cmd_record(seq: u64, hash: u64) -> LogRecord {
        let mut fields = Map::new();
        fields.insert("state_hash".into(), Value::Number(hash.into()));
        LogRecord {
            seq,
            corr: Some(1),
            level: "INFO",
            target: "kernel::cmd".into(),
            fields,
        }
    }

    #[test]
    fn pairs_op_and_cmd_into_one_command_with_params() {
        reset();
        start(100);
        observe(&op_record(0, "extrude_region", "distance", 2.5));
        observe(&cmd_record(1, 200));
        let rec = take();
        assert_eq!(rec.version, RECORDING_FORMAT_VERSION);
        assert_eq!(rec.baseline_hash, 100);
        assert_eq!(rec.commands.len(), 1);
        let c = &rec.commands[0];
        assert_eq!(c.op.as_deref(), Some("extrude_region"));
        assert_eq!(c.params["distance"], 2.5);
        assert!(
            !c.params.contains_key("op"),
            "op key is lifted out of params"
        );
        assert_eq!(c.pre_hash, 100, "first command's pre_hash is the baseline");
        assert_eq!(c.post_hash, 200);
    }

    #[test]
    fn chains_pre_and_post_hashes_across_commands() {
        reset();
        start(10);
        observe(&op_record(0, "a", "d", 1.0));
        observe(&cmd_record(1, 20));
        observe(&op_record(2, "b", "d", 2.0));
        observe(&cmd_record(3, 30));
        let rec = take();
        assert_eq!(rec.commands.len(), 2);
        assert_eq!(
            (rec.commands[0].pre_hash, rec.commands[0].post_hash),
            (10, 20)
        );
        assert_eq!(
            (rec.commands[1].pre_hash, rec.commands[1].post_hash),
            (20, 30),
            "each command's pre_hash is the previous post_hash"
        );
    }

    #[test]
    fn a_failed_op_with_no_cmd_is_not_recorded() {
        reset();
        start(0);
        // An op that errors: emits kernel::op but no kernel::cmd.
        observe(&op_record(0, "boolean", "d", 0.0));
        // The next successful op overwrites the abandoned pending entry.
        observe(&op_record(1, "extrude_region", "distance", 1.0));
        observe(&cmd_record(2, 50));
        let rec = take();
        assert_eq!(rec.commands.len(), 1, "only the committed op is recorded");
        assert_eq!(rec.commands[0].op.as_deref(), Some("extrude_region"));
    }

    #[test]
    fn committed_mutation_without_op_event_keeps_the_chain() {
        reset();
        start(5);
        // A kernel::cmd with no preceding kernel::op (un-instrumented mutation).
        observe(&cmd_record(0, 15));
        let rec = take();
        assert_eq!(rec.commands.len(), 1);
        assert_eq!(rec.commands[0].op, None);
        assert_eq!(
            (rec.commands[0].pre_hash, rec.commands[0].post_hash),
            (5, 15)
        );
    }

    #[test]
    fn observe_is_a_noop_when_not_recording() {
        reset();
        observe(&op_record(0, "extrude_region", "distance", 1.0));
        observe(&cmd_record(1, 99));
        // take() with no active recording yields an empty command list.
        assert!(take().commands.is_empty());
    }

    #[test]
    fn recording_serializes_to_stable_json() {
        reset();
        start(1);
        observe(&op_record(0, "extrude_region", "distance", 2.0));
        observe(&cmd_record(1, 2));
        let json = serde_json::to_string(&take()).unwrap();
        // version + baseline + the command's hash chain are all present.
        assert!(json.contains("\"version\":1"));
        assert!(json.contains("\"baseline_hash\":1"));
        assert!(json.contains("\"pre_hash\":1"));
        assert!(json.contains("\"post_hash\":2"));
        assert!(json.contains("\"op\":\"extrude_region\""));
    }
}
