//! Replayable command recording (docs/DEVELOPMENT.md,  +).
//!
//! A recording is the ordered stream of committed `Scene` mutations as **typed,
//! replayable calls**. Replaying them verbatim into a fresh [`Scene`] reproduces
//! the session, and the final [`state_hash`](crate::Scene::state_hash) is
//! asserted against the recorded golden — the regression guarantee (docs/DEVELOPMENT.md).
//!
//! ## Why verbatim replay needs no handle-remapping
//!
//! Recorded calls carry literal slotmap handles (a `sketch`/`object`/`region`
//! id). Naively those wouldn't survive into a fresh document — except the kernel
//! is now **deterministic**: the same op sequence does the same
//! insert/remove sequence, and `slotmap` assigns keys deterministically from
//! that, so a replay that re-issues the identical calls reproduces the identical
//! handles. The recorded ids are therefore valid verbatim — no remap table. This
//! is a direct payoff of the determinism lane, and the replay test empirically
//! confirms it (a divergence would fail to resolve a handle, or break the hash).
//!
//! A recording replays into a **fresh** `Scene` (`golden_hash` is captured
//! relative to the empty document the recording began on). The artifact JSON
//! shape is frozen in `docs/DIAGNOSTICS.md` — the handshake for the
//! Node runner and the M17 bug-report bundle.

use std::cell::{Cell, RefCell};

use serde::{Deserialize, Serialize};

/// Bump on any breaking change to the [`Recording`] JSON shape. v2 = typed
/// replayable calls (v1 was the log-tap hash-chain). See
/// `docs/DIAGNOSTICS.md`.
pub const RECORDING_FORMAT_VERSION: u32 = 2;

/// One committed `Scene` mutation, captured with the exact arguments needed to
/// re-issue it. `#[serde(tag = "method")]` gives a self-describing JSON object
/// per call (`{"method":"extrude_region","sketch":…,"region":…,"distance":…}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RecordedCall {
    /// `begin_ground_sketch()`.
    BeginGroundSketch,
    /// `sketch_add_segment(sketch, a, b)`.
    SketchAddSegment {
        sketch: u64,
        a: [f64; 3],
        b: [f64; 3],
    },
    /// `extrude_region(sketch, region, distance)`.
    ExtrudeRegion {
        sketch: u64,
        region: u64,
        distance: f64,
    },
    /// `boolean(op, a, b)`.
    Boolean { op: u8, a: u64, b: u64 },
    /// `slice_object(object, plane)`.
    SliceObject { object: u64, plane: [f64; 6] },
    /// `transform_object(object, affine)`.
    TransformObject { object: u64, affine: [f64; 12] },
    /// `delete_node(kind, id)`.
    DeleteNode { kind: u8, id: u64 },
}

/// A complete recorded session: the committed call stream plus the canonical
/// `state_hash` it produced. Replaying `calls` verbatim into a fresh
/// `Scene` must reproduce `golden_hash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Format version (`RECORDING_FORMAT_VERSION`).
    pub version: u32,
    /// The committed mutations, in application order.
    pub calls: Vec<RecordedCall>,
    /// The document `state_hash` after the last recorded call — the replay oracle.
    pub golden_hash: u64,
}

thread_local! {
    /// Whether capture is active (toggled by [`start`]/[`stop`]). Held off during
    /// replay so re-issued calls are not re-recorded.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    /// Calls captured so far.
    static CALLS: RefCell<Vec<RecordedCall>> = const { RefCell::new(Vec::new()) };
}

/// Begins capture, discarding any prior in-progress recording. The caller should
/// be on a fresh/empty document for the golden to be replayable from `Scene::new`.
pub fn start() {
    CALLS.with(|c| c.borrow_mut().clear());
    ENABLED.with(|e| e.set(true));
}

/// Stops capture; the accumulated calls remain available to [`take_calls`].
pub fn stop() {
    ENABLED.with(|e| e.set(false));
}

/// Whether capture is active.
pub fn is_active() -> bool {
    ENABLED.with(|e| e.get())
}

/// Appends one committed call — a no-op unless capture is active. Call this
/// **after** the mutation succeeds, so failed/refused ops are never recorded.
pub fn record(call: RecordedCall) {
    if is_active() {
        CALLS.with(|c| c.borrow_mut().push(call));
    }
}

/// Takes the captured calls, clearing the buffer.
pub fn take_calls() -> Vec<RecordedCall> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Runs `body` with capture suppressed (used during replay so re-issued calls
/// don't re-record), restoring the prior state after.
pub fn without_capture<R>(body: impl FnOnce() -> R) -> R {
    let prev = ENABLED.with(|e| e.replace(false));
    let out = body();
    ENABLED.with(|e| e.set(prev));
    out
}

#[cfg(test)]
pub fn reset() {
    ENABLED.with(|e| e.set(false));
    CALLS.with(|c| c.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_while_active() {
        reset();
        record(RecordedCall::BeginGroundSketch); // ignored — not started
        start();
        record(RecordedCall::BeginGroundSketch);
        record(RecordedCall::ExtrudeRegion {
            sketch: 1,
            region: 2,
            distance: 3.0,
        });
        stop();
        record(RecordedCall::BeginGroundSketch); // ignored — stopped
        let calls = take_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], RecordedCall::BeginGroundSketch);
    }

    #[test]
    fn without_capture_suppresses_recording() {
        reset();
        start();
        without_capture(|| record(RecordedCall::BeginGroundSketch));
        assert!(is_active(), "capture state restored after the closure");
        assert!(
            take_calls().is_empty(),
            "the suppressed call was not recorded"
        );
    }

    #[test]
    fn calls_round_trip_through_json() {
        let rec = Recording {
            version: RECORDING_FORMAT_VERSION,
            calls: vec![
                RecordedCall::BeginGroundSketch,
                RecordedCall::SketchAddSegment {
                    sketch: 5,
                    a: [0.0, 0.0, 0.0],
                    b: [1.0, 0.0, 0.0],
                },
                RecordedCall::ExtrudeRegion {
                    sketch: 5,
                    region: 9,
                    distance: 2.0,
                },
            ],
            golden_hash: 0xABCD,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"method\":\"extrude_region\""));
        let back: Recording = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, rec.version);
        assert_eq!(back.golden_hash, rec.golden_hash);
        assert_eq!(back.calls, rec.calls);
    }
}
