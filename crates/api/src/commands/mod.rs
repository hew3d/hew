//! Command implementations, one module per namespace (docs/HEW_API.md
//! §7). The handler table below is the dispatch surface; the registry's
//! `implemented` flags must agree with it (a sync test enforces that),
//! so implementing a command means adding its handler here, flipping its
//! registry flag, tightening its schemas, and un-gating its conformance
//! tests — §14's burn-down step.

use crate::host::Host;
use crate::ids::IdResolver;
use crate::refusal::Refusal;
use std::collections::BTreeMap;

/// Why a command did not produce a result (docs/HEW_API.md §4.4's three
/// classes, minus protocol-envelope concerns handled by dispatch).
#[derive(Debug)]
pub enum CmdError {
    /// Malformed params — `-32602` for the envelope (static defect).
    Params(String),
    /// The kernel (or host) declined — `-32000`, document untouched.
    Refusal(Refusal),
    /// A kernel invariant failed — `-32003`, a bug to report.
    Internal(String),
}

impl From<kernel::DocumentError> for CmdError {
    fn from(e: kernel::DocumentError) -> CmdError {
        CmdError::Refusal(Refusal::from_document_error(&e))
    }
}

/// Face tokens minted inside one transaction (docs/HEW_API.md §5.2,
/// §5.4): `label` → `key` → the face it names. Transaction-scoped;
/// they expire with the envelope.
pub type FaceTokens = BTreeMap<String, BTreeMap<String, (kernel::ObjectId, kernel::FaceId)>>;

/// Everything a command executes against.
pub struct Ctx<'a> {
    pub doc: &'a mut kernel::Document,
    pub host: &'a mut dyn Host,
    /// Face tokens minted so far in this envelope (empty outside
    /// transactions).
    pub face_tokens: &'a mut FaceTokens,
    /// The label the executing command was given with `"as"`, if any —
    /// where its face tokens land.
    pub current_label: Option<String>,
}

impl Ctx<'_> {
    /// A fresh id resolver over the CURRENT document state — rebuilt per
    /// command so entities created earlier in the same transaction
    /// resolve (§5.3's dispatch-time semantics).
    pub fn resolver(&self) -> IdResolver {
        IdResolver::new(self.doc)
    }

    /// Records a face token under the executing command's label.
    pub fn mint_face_token(&mut self, key: &str, object: kernel::ObjectId, face: kernel::FaceId) {
        if let Some(label) = &self.current_label {
            self.face_tokens
                .entry(label.clone())
                .or_default()
                .insert(key.to_string(), (object, face));
        }
    }
}

/// One command implementation.
pub type Handler = fn(&mut Ctx, &serde_json::Value) -> Result<serde_json::Value, CmdError>;

/// Brackets a sketch mutation in one `begin_sketch_gesture` /
/// `end_sketch_gesture` pair, so everything `body` does lands as a
/// single undo entry.
///
/// The gesture closes either way — that is the kernel's contract — so a
/// failing body still ends cleanly and the command's own error is what
/// the caller sees. A body that changed nothing (the refusal paths,
/// where the kernel leaves the sketch untouched) records no undo entry,
/// so a refused command never leaves a phantom step in the history.
pub(super) fn run_sketch_gesture<T>(
    ctx: &mut Ctx,
    sketch: kernel::SketchId,
    body: impl FnOnce(&mut kernel::Sketch) -> Result<T, CmdError>,
) -> Result<T, CmdError> {
    ctx.doc.begin_sketch_gesture(sketch)?;
    let outcome = match ctx.doc.sketch_mut(sketch) {
        Some(s) => body(s),
        None => Err(CmdError::Internal("sketch vanished mid-gesture".into())),
    };
    let end = ctx.doc.end_sketch_gesture(sketch);
    match outcome {
        Ok(v) => {
            end?;
            Ok(v)
        }
        Err(e) => {
            let _ = end;
            Err(e)
        }
    }
}

pub mod attrs;
/// Shared camera-spec parsing for `hew.view.snapshot` (doc.rs) and
/// `hew.view.camera` (view.rs) — not a command namespace of its own, so
/// it carries no `handler`.
mod camera;
pub mod doc;
pub mod entity;
pub mod history;
pub mod query;
pub mod scenes;
pub mod sketch;
pub mod solid;
pub mod structure;
pub mod style;
pub mod view;

/// The handler table, partitioned per namespace so each module owns its
/// own match — a `None` answers the `unimplemented` refusal (§14).
pub fn handler(name: &str) -> Option<Handler> {
    query::handler(name)
        .or_else(|| sketch::handler(name))
        .or_else(|| solid::handler(name))
        .or_else(|| entity::handler(name))
        .or_else(|| structure::handler(name))
        .or_else(|| style::handler(name))
        .or_else(|| attrs::handler(name))
        .or_else(|| history::handler(name))
        .or_else(|| doc::handler(name))
        .or_else(|| view::handler(name))
        .or_else(|| scenes::handler(name))
}
