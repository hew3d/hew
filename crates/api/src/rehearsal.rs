//! The host a dry run executes against (docs/agents/HEW_API.md §6.6): no
//! effect leaves it. A rehearsal runs on a clone of the document, so the
//! document side is already contained; this host contains the other side.
//!
//! Reads pass through. Every `&self` question a Kernel-served command asks
//! its host (the user's in-context fallback, whether a gesture is armed,
//! the selection, the isolation, the open documents, the working path, the
//! clock and entropy) is answered by the LIVE host, so a rehearsal answers
//! what the commit would: `hew.query.context` reports the real `fallback`
//! and `armed`, and `hew.view.enter` refuses `gesture_in_progress` exactly
//! when the commit would. The library reads are the one exception: a dry
//! run never touches the library (§6.6), so they refuse like a write.
//!
//! Writes never pass. Every effect (hidden-state and scene signals, the
//! selection, isolation, the user's context, the camera, files, the
//! library) refuses `host_capability_missing`, which the dry-run executor
//! reports as `dry_run_host_command`: a view or selection effect cannot be
//! rehearsed, only run for real. The two signals the trait answers with a
//! silent success by default ([`Host::hidden_changed`],
//! [`Host::scene_applied`]) refuse too, so nothing a rehearsal does reaches
//! a viewport.

use crate::host::{ContextEntry, Host, IsolationEntry, SelectionState};
use crate::refusal::Refusal;

/// The host of a dry run: the live host's reads, none of its writes.
pub struct RehearsalHost<'a> {
    live: &'a dyn Host,
}

impl<'a> RehearsalHost<'a> {
    /// A rehearsal host reading through to `live`.
    pub fn new(live: &'a dyn Host) -> RehearsalHost<'a> {
        RehearsalHost { live }
    }
}

impl std::fmt::Debug for RehearsalHost<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RehearsalHost")
    }
}

fn not_rehearsable(what: &str) -> Refusal {
    Refusal::api(
        "host_capability_missing",
        &format!("A dry run cannot {what}."),
    )
    .with_detail(serde_json::json!({ "capability": what }))
}

// Every write method keeps the trait default, which refuses
// `host_capability_missing`; only the two silent-success signals need an
// override. The reads below forward.
impl Host for RehearsalHost<'_> {
    fn hidden_changed(&mut self) -> Result<(), Refusal> {
        Err(not_rehearsable("change what is hidden"))
    }

    fn scene_applied(&mut self, sid: u64) -> Result<(), Refusal> {
        let _ = sid;
        Err(not_rehearsable("apply a scene"))
    }

    fn documents(&self, doc: &kernel::Document) -> Vec<serde_json::Value> {
        self.live.documents(doc)
    }

    fn unix_time_ms(&self) -> u64 {
        self.live.unix_time_ms()
    }

    fn random_u64(&self) -> u64 {
        self.live.random_u64()
    }

    fn random_16(&self) -> [u8; 16] {
        self.live.random_16()
    }

    fn working_document_path(&self) -> Option<&str> {
        self.live.working_document_path()
    }

    fn selection(&self) -> Result<SelectionState, Refusal> {
        self.live.selection()
    }

    fn isolation(&self) -> Result<Option<Vec<IsolationEntry>>, Refusal> {
        self.live.isolation()
    }

    fn fallback_context(&self) -> Vec<ContextEntry> {
        self.live.fallback_context()
    }

    fn gesture_armed(&self) -> bool {
        self.live.gesture_armed()
    }
}

/// `dry_run_host_command`: a command a dry run cannot rehearse, named in
/// `detail.method` (docs/agents/HEW_API.md §6.6).
pub fn dry_run_host_command(method: &str) -> Refusal {
    Refusal::api(
        "dry_run_host_command",
        &format!(
            "A dry run can only rehearse edits to the model. {method} touches files, the library, or the screen, so run it for real."
        ),
    )
    .with_detail(serde_json::json!({ "method": method }))
}
