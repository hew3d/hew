//! The live desktop transport's WASM half (docs/agents/HEW_API.md §11.2): a
//! per-connection [`api::Connection`] dispatched against the SAME live
//! [`kernel::Document`] the viewport renders. The Tauri shell
//! (`shells/tauri`) owns the socket, the discovery file, and the token
//! check; it never sees the document. This module owns dispatch and the
//! [`api::Host`] effects that are cleanly reachable from inside the
//! webview's WASM sandbox — nothing more.
//!
//! [`Scene::api_dispatch`](crate::Scene::api_dispatch) is the entry
//! point; it lives in `lib.rs` because it needs `&mut Scene` (to
//! reconcile derived caches after a mutation), not just `&mut
//! kernel::Document`.

use api::{
    ContextEntry, ContextKind, DocFormat, Host, IsolationEntry, LibraryListing, LibraryReadResult,
    LibraryWriteResult, LibraryWriteTarget, Refusal, SaveOutcome, SelectMode, SelectionEntry,
    SelectionState, SnapshotProjection, StandardView, ViewCameraSpec,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slotmap::{Key, KeyData};

/// A kernel handle as the app holds it: the slotmap key's FFI value, carried
/// on the wire as a decimal string so JavaScript reads it as a `BigInt`
/// without losing bits.
mod decimal {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse::<u64>().map_err(serde::de::Error::custom)
    }
}

/// One entry of the app's selection as the app mirrors it into the `Scene`
/// ([`crate::Scene::set_ui_selection`]) and as a
/// [`ViewDirective::Selection`] hands it back: a kind and a kernel handle
/// (a decimal string on the wire). The app's draft kinds have no public id
/// and are never mirrored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiSelectionEntry {
    Object {
        #[serde(with = "decimal")]
        id: u64,
    },
    Group {
        #[serde(with = "decimal")]
        id: u64,
    },
    Instance {
        #[serde(with = "decimal")]
        id: u64,
    },
    Sketch {
        #[serde(with = "decimal")]
        id: u64,
    },
    Guide {
        #[serde(with = "decimal")]
        id: u64,
    },
    Annotation {
        #[serde(with = "decimal")]
        id: u64,
    },
}

impl UiSelectionEntry {
    /// The entry with kernel ids, for the [`api::Host`] boundary.
    pub fn to_entry(&self) -> SelectionEntry {
        let key = |id: u64| KeyData::from_ffi(id);
        match *self {
            UiSelectionEntry::Object { id } => SelectionEntry::Object(key(id).into()),
            UiSelectionEntry::Group { id } => SelectionEntry::Group(key(id).into()),
            UiSelectionEntry::Instance { id } => SelectionEntry::Instance(key(id).into()),
            UiSelectionEntry::Sketch { id } => SelectionEntry::Sketch(key(id).into()),
            UiSelectionEntry::Guide { id } => SelectionEntry::Guide(key(id).into()),
            UiSelectionEntry::Annotation { id } => SelectionEntry::Annotation(key(id).into()),
        }
    }

    /// The app's form of a kernel entry.
    pub fn from_entry(entry: SelectionEntry) -> UiSelectionEntry {
        match entry {
            SelectionEntry::Object(id) => UiSelectionEntry::Object {
                id: id.data().as_ffi(),
            },
            SelectionEntry::Group(id) => UiSelectionEntry::Group {
                id: id.data().as_ffi(),
            },
            SelectionEntry::Instance(id) => UiSelectionEntry::Instance {
                id: id.data().as_ffi(),
            },
            SelectionEntry::Sketch(id) => UiSelectionEntry::Sketch {
                id: id.data().as_ffi(),
            },
            SelectionEntry::Guide(id) => UiSelectionEntry::Guide {
                id: id.data().as_ffi(),
            },
            SelectionEntry::Annotation(id) => UiSelectionEntry::Annotation {
                id: id.data().as_ffi(),
            },
        }
    }
}

/// One isolation target as the app mirrors it ([`crate::Scene::set_ui_isolation`])
/// and as a [`ViewDirective::Isolate`] hands it back: the app's
/// `IsolationTarget`, handles as decimal strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiIsolationEntry {
    Object {
        #[serde(with = "decimal")]
        id: u64,
    },
    Group {
        #[serde(with = "decimal")]
        id: u64,
    },
    Instance {
        #[serde(with = "decimal")]
        id: u64,
    },
    Member {
        #[serde(with = "decimal")]
        object: u64,
        #[serde(with = "decimal")]
        instance: u64,
    },
}

impl UiIsolationEntry {
    /// The entry with kernel ids, for the [`api::Host`] boundary.
    pub fn to_entry(&self) -> IsolationEntry {
        let key = |id: u64| KeyData::from_ffi(id);
        match *self {
            UiIsolationEntry::Object { id } => {
                IsolationEntry::Node(kernel::NodeId::Object(key(id).into()))
            }
            UiIsolationEntry::Group { id } => {
                IsolationEntry::Node(kernel::NodeId::Group(key(id).into()))
            }
            UiIsolationEntry::Instance { id } => {
                IsolationEntry::Node(kernel::NodeId::Instance(key(id).into()))
            }
            UiIsolationEntry::Member { object, instance } => IsolationEntry::Member {
                object: key(object).into(),
                instance: key(instance).into(),
            },
        }
    }

    /// The app's form of a kernel entry.
    pub fn from_entry(entry: IsolationEntry) -> UiIsolationEntry {
        match entry {
            IsolationEntry::Node(kernel::NodeId::Object(id)) => UiIsolationEntry::Object {
                id: id.data().as_ffi(),
            },
            IsolationEntry::Node(kernel::NodeId::Group(id)) => UiIsolationEntry::Group {
                id: id.data().as_ffi(),
            },
            IsolationEntry::Node(kernel::NodeId::Instance(id)) => UiIsolationEntry::Instance {
                id: id.data().as_ffi(),
            },
            IsolationEntry::Member { object, instance } => UiIsolationEntry::Member {
                object: object.data().as_ffi(),
                instance: instance.data().as_ffi(),
            },
        }
    }
}

/// One of the app's in-context entries as it mirrors them
/// ([`crate::Scene::set_ui_context`]): the kinds its fallback can hold, with
/// the handle as a decimal string. A group always opens a session, so it is
/// never one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiContextEntry {
    Object {
        #[serde(with = "decimal")]
        id: u64,
    },
    Instance {
        #[serde(with = "decimal")]
        id: u64,
    },
}

impl UiContextEntry {
    /// The entry with kernel ids, for the [`api::Host`] boundary.
    pub fn to_entry(&self) -> ContextEntry {
        let key = |id: u64| KeyData::from_ffi(id);
        match *self {
            UiContextEntry::Object { id } => ContextEntry {
                kind: ContextKind::Object,
                node: kernel::NodeId::Object(key(id).into()),
            },
            UiContextEntry::Instance { id } => ContextEntry {
                kind: ContextKind::Instance,
                node: kernel::NodeId::Instance(key(id).into()),
            },
        }
    }
}

/// A tree node on the wire of a [`ViewDirective::EnterContext`]: an object,
/// group, or instance, the handle a decimal string (the app's `NodeRef`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiNode {
    Object {
        #[serde(with = "decimal")]
        id: u64,
    },
    Group {
        #[serde(with = "decimal")]
        id: u64,
    },
    Instance {
        #[serde(with = "decimal")]
        id: u64,
    },
}

impl UiNode {
    /// The app's form of a kernel node.
    pub fn from_node(node: kernel::NodeId) -> UiNode {
        match node {
            kernel::NodeId::Object(id) => UiNode::Object {
                id: id.data().as_ffi(),
            },
            kernel::NodeId::Group(id) => UiNode::Group {
                id: id.data().as_ffi(),
            },
            kernel::NodeId::Instance(id) => UiNode::Instance {
                id: id.data().as_ffi(),
            },
        }
    }
}

/// A viewport/app-settings effect requested by `hew.view.camera`,
/// `hew.view.zoom_extents`, or `hew.view.units` that `LiveHost` cannot
/// perform itself — this WASM module has no DOM access to move the
/// viewport's three.js camera or call `app/src/settings/units.ts`'s
/// setter directly. `LiveHost`'s matching trait method (below) just
/// records the intent here and answers success; `Scene` (`lib.rs`)
/// captures it off `LiveHost` right after dispatch and hands it to the
/// webview's live bridge (`app/src/api/liveBridge.ts`) through
/// [`crate::Scene::take_pending_view_directives`] — a second, JS-side step
/// exactly like how `Scene::api_method_mutates` already tells the bridge
/// "should you resync" outside the JSON-RPC reply itself.
///
/// Deliberately NOT folded into the JSON-RPC reply `api_dispatch` returns
/// (the `reply` frame inside its tagged answer): that frame is the actual
/// wire response sent back over the socket to
/// whatever dialed in (`hew-cli --live`, an MCP agent) per docs/agents/HEW_API.md
/// §4 — those clients have no viewport or Settings window of their own to
/// act on a directive with, and JSON-RPC responses are contractually
/// `{jsonrpc, id, result|error}`, not a place to smuggle side-channel
/// instructions. This type instead travels a WASM-internal side channel
/// local to the one process that owns both the dispatcher and the UI the
/// directive is for — the desktop app itself — and is read out
/// synchronously in the same JS task that called `api_dispatch`, so there
/// is never a JS round trip for `LiveHost`'s trait methods to block on.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewDirective {
    /// `hew.view.camera`'s effect: the live bridge applies it through the
    /// SAME `Viewport.setCamera`/`Viewport.setStandardView` the app's own
    /// Camera menu and toolbar already call (`app/src/viewport/
    /// Viewport.tsx`) — there is no second camera-control path.
    Camera {
        #[serde(skip_serializing_if = "Option::is_none")]
        camera: Option<CameraJson>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    /// `hew.view.zoom_extents`'s effect: the bridge calls
    /// `Viewport.zoomExtents()`.
    ZoomExtents,
    /// `hew.view.units`'s effect: the bridge calls
    /// `app/src/settings/units.ts`'s `setLengthUnit(format)` directly —
    /// NOT a viewport call, since display units are an app-level setting,
    /// not a camera property. Going through that module's own setter
    /// (rather than writing `localStorage` directly) is what keeps the
    /// cross-window `'settings-changed'` broadcast firing, so a live
    /// desktop's separate Settings window stays in sync.
    Units { format: String },
    /// `hew.scenes.apply`'s host notification (`Host::scene_applied`):
    /// the kernel-side state (tag/node hidden flags, section plane) is
    /// already written by the time this fires — `sid` alone is enough
    /// for the bridge to drive whatever app-side activation logic
    /// `useScenesController`'s `scenes.activate` path already has
    /// (camera tween, panel/outliner sync), rather than duplicating that
    /// logic here from raw numbers. See `applyPendingViewDirectives`
    /// (`app/src/api/liveBridge.ts`) for the JS side.
    ActivateScene { sid: u64 },
    /// `hew.selection.set`/`clear`'s effect: the merged selection, which the
    /// bridge applies wholesale through the app's own selection handlers
    /// (`handleSelectMany(nodes, 'replace')`, `handleSelectGuide`,
    /// `handleSelectAnnotation`). `mode` is always `"replace"`: the merge
    /// already happened here.
    Selection {
        entries: Vec<UiSelectionEntry>,
        mode: String,
    },
    /// `hew.view.hide`/`show`/`show_all`'s notification: the kernel's
    /// user-hidden flags changed, so the app re-reads them into its hidden
    /// set and pushes the union to the renderer and the pick/snap exclusion.
    HiddenChanged,
    /// `hew.view.isolate`'s effect: the bridge calls `ViewportApi.isolate`
    /// with these targets, framing them when `frame` is set (an aborted
    /// plugin command putting an isolation back does not reframe).
    Isolate {
        entries: Vec<UiIsolationEntry>,
        frame: bool,
    },
    /// `hew.view.clear_isolation`'s effect: `ViewportApi.clearIsolation`.
    ClearIsolation,
    /// `hew.view.zoom_to`'s effect: `ViewportApi.zoomToWorldBounds` over
    /// the box, which already carries the padding.
    ZoomTo {
        min: [f64; 3],
        max: [f64; 3],
        padding: f64,
    },
    /// `hew.view.enter`'s effect: the bridge converges the user's window on
    /// `leaf` the way an Outliner double click does, calling the app's
    /// `enterNode` for each instance of `path` (outermost first) and then
    /// for `leaf`. The outcome is `context_changed`, not a reply.
    EnterContext { path: Vec<UiNode>, leaf: UiNode },
    /// `hew.view.exit`'s effect: the app pops its innermost entry or
    /// session frame (`setPathDepth(total - 1)`).
    ExitContext,
    /// `hew.view.exit_all`'s effect: the app closes everything
    /// (`setPathDepth(0)`).
    ExitAllContexts,
}

/// The wire shape `ViewDirective::Camera` hands the live bridge for an
/// explicit camera — mirrors `hew.view.snapshot`/`hew.view.camera`'s own
/// `camera` params shape (docs/agents/HEW_API.md §7) so the bridge's JSON
/// parsing on the JS side needs exactly one shape to know, matching what
/// it already receives on the wire from an actual `--live` client.
#[derive(Debug, Clone, Serialize)]
pub struct CameraJson {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fov_deg: Option<f64>,
    /// The height the view spans at the target, meters, when the camera
    /// says (what a parallel view frames).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_height: Option<f64>,
}

fn standard_view_name(view: StandardView) -> String {
    match view {
        StandardView::Iso => "iso",
        StandardView::Front => "front",
        StandardView::Back => "back",
        StandardView::Left => "left",
        StandardView::Right => "right",
        StandardView::Top => "top",
        StandardView::Bottom => "bottom",
    }
    .to_string()
}

impl From<&ViewCameraSpec> for ViewDirective {
    fn from(spec: &ViewCameraSpec) -> ViewDirective {
        match spec {
            ViewCameraSpec::Explicit(cam) => ViewDirective::Camera {
                camera: Some(CameraJson {
                    eye: cam.eye,
                    target: cam.target,
                    up: cam.up,
                    projection: cam.projection.map(|p| match p {
                        SnapshotProjection::Perspective => "perspective".to_string(),
                        SnapshotProjection::Parallel => "parallel".to_string(),
                    }),
                    fov_deg: cam.fov_deg,
                    parallel_height: cam.parallel_height,
                }),
                view: None,
            },
            ViewCameraSpec::Standard(view) => ViewDirective::Camera {
                camera: None,
                view: Some(standard_view_name(*view)),
            },
        }
    }
}

/// The `api::Host` for a live desktop connection. The webview's WASM
/// runtime has no filesystem, so most host-implemented commands
/// (docs/agents/HEW_API.md §3: `hew.doc.new/open/import`, a snapshot's
/// `path` write) are honestly refused here with `host_capability_missing` rather than
/// faked (the registry declares that exact refusal name for every one of
/// these — see `crates/api/src/registry.rs`'s Wave D block).
/// `hew.doc.save` and `hew.doc.export` are the two exceptions: neither
/// needs a filesystem to produce its bytes, only the final write does, so
/// both hand the bytes back for the caller to write and refuse only a
/// `path` specifically (see [`save_document`](Self::save_document) and
/// [`export_document`](Self::export_document) below). The much larger
/// kernel-served command surface — sketch, solid, structure, entity,
/// style, attrs, history: everything that is NOT one of the above —
/// never touches this trait at all and runs for real against the live
/// document (`registry.rs`'s `Served::Kernel` commands execute directly
/// inside `crates/api`), which is what makes `--live` actually useful
/// today even before counting `save`/`export`.
///
/// `hew.doc.new`/`hew.doc.open` are a deliberate design call, not a
/// missing feature: docs/agents/HEW_API.md §10 names exactly this posture — "a
/// live host that keeps document lifecycle user-driven simply withholds
/// `hew.doc.new`/`open` from its `app` grant" — because a remote
/// connection silently replacing the user's live, unsaved document is
/// precisely the "astonishing" case §12 warns `--live` must never be.
/// `Grant::app()` grants unconditionally today (`grant.rs`'s
/// `Grant::grants` — not this crate's to narrow, and doing so is a
/// `crates/api` change outside this effort's ownership), so the
/// equivalent is enforced here instead: both refuse typed, pointing at
/// the app's own File menu, rather than either faking the swap or
/// reaching past this crate's boundary to narrow the grant.
///
/// `set_camera`/`zoom_extents`/`set_display_units`, unlike every method
/// above, are cleanly reachable from inside the WASM sandbox — not
/// through this trait's own effect (which still has no DOM access), but
/// by recording a [`ViewDirective`] into `directives` and answering
/// success; see that type's doc comment for the full JS hand-off story.
/// The selection and isolation commands work the same way over a copy of
/// the app's mirrors that `Scene::api_dispatch` seeds before the dispatch
/// and writes back only after it succeeds, so a refused command (or an
/// aborted transaction) leaves the mirrors and the app untouched.
#[derive(Debug, Default)]
pub struct LiveHost {
    /// Every directive this dispatch left, in the order its commands ran:
    /// a transaction can carry several view-state commands.
    /// `Scene::api_dispatch` (`lib.rs`) constructs a fresh `LiveHost` per
    /// dispatch, so a directive never leaks from an earlier command into a
    /// later one.
    pub directives: Vec<ViewDirective>,
    /// The app's selection as mirrored into the `Scene`, merged into by
    /// `hew.selection.set`.
    pub selection: SelectionState,
    /// The live viewport's isolation as mirrored into the `Scene`.
    pub isolation: Option<Vec<IsolationEntry>>,
    /// The app's display length format (`app/src/settings/units.ts`'s
    /// `LengthFormat`), which a SketchUp export opens in unless the caller
    /// names one; `None` writes SketchUp's own defaults.
    pub display_units: Option<String>,
    /// The app's in-context entries as mirrored into the `Scene`.
    pub fallback: Vec<ContextEntry>,
    /// Whether a tool gesture is armed in the user's window.
    pub gesture_armed: bool,
    /// Whether the user's window is at the top level (no session frame, no
    /// fallback entry): `hew.view.exit` has nothing to pop then.
    pub at_top_level: bool,
}

/// Builds the `host_capability_missing` refusal every method below
/// answers with — same refusal name the registry declares for these
/// commands on any host lacking the capability (`crates/api/src/host.rs`'s
/// own `unsupported()` helper mints the identical name; this is a local
/// copy so the explanation can be specific to what a *live* WASM
/// connection actually lacks, not the generic "connect through a host
/// that can" text, which is misleading here — this IS the desktop app).
fn refuse(what: &str, use_instead: &str) -> Refusal {
    Refusal::api(
        "host_capability_missing",
        &format!("The live desktop connection cannot {what} yet. {use_instead}"),
    )
    .with_detail(serde_json::json!({ "capability": what }))
}

/// Maps a [`mesh_export::ExportError`] to an [`api::Refusal`] through the
/// error's own name/message accessors, so the two can never drift apart —
/// `mesh-export` has no dependency on `crates/api` (DEVELOPMENT.md rule
/// 1), so this boundary-crossing step lives here instead, the same way
/// `crates/hew-cli`'s `CliHost` maps it on its own side.
fn export_refusal(e: mesh_export::ExportError) -> Refusal {
    Refusal::api(e.name(), &e.message())
}

impl Host for LiveHost {
    fn new_document(&mut self, _doc: &mut kernel::Document) -> Result<(), Refusal> {
        Err(refuse(
            "create a new document",
            "This window cannot replace its own document. The desktop app answers hew.doc.new itself by opening a new window; a connection that reaches this host directly should use the app's File > New.",
        ))
    }

    fn open_document(&mut self, _doc: &mut kernel::Document, _path: &str) -> Result<(), Refusal> {
        Err(refuse(
            "open a document",
            "This window cannot replace its own document. The desktop app answers hew.doc.open itself by opening the file in a new window; a connection that reaches this host directly should use the app's File > Open.",
        ))
    }

    /// Serializing the document needs no filesystem — only WRITING the
    /// result does. So a live save produces the bytes and hands them
    /// back; whoever asked (a CLI, an MCP client) writes them on the
    /// side that actually has a disk. A `path` is the one part this host
    /// genuinely cannot honor, and it refuses that specifically rather
    /// than refusing the whole command. A `.skp` format is refused too:
    /// the live document's file belongs to the app (its own Save keeps
    /// the SketchUp source and container), so an API save only ever
    /// produces `.hew` bytes; `hew.doc.export` with `skp` / `skp2017`
    /// hands back a SketchUp file without touching the document's file.
    fn save_document(
        &mut self,
        doc: &kernel::Document,
        path: Option<&str>,
        format: Option<DocFormat>,
        _units: Option<&str>,
    ) -> Result<SaveOutcome, Refusal> {
        if path.is_some() {
            return Err(refuse(
                "write a file",
                "The WASM boundary has no filesystem of its own. Omit `path` to receive the \
                 bytes and write them yourself, or use the app's File > Save.",
            ));
        }
        if format.is_some_and(DocFormat::is_skp) {
            return Err(refuse(
                "save a SketchUp file",
                "The live document's file belongs to the app. Use the app's File > Save, or \
                 hew.doc.export with format \"skp\" / \"skp2017\" to receive a SketchUp file.",
            ));
        }
        Ok(SaveOutcome {
            format: Some(DocFormat::Hew),
            ..SaveOutcome::with_bytes(doc.save_for_persistence())
        })
    }

    /// Mirrors [`save_document`](Self::save_document)'s posture exactly:
    /// the STL/3MF/glTF/USDZ writers (`crates/mesh-export`, shared with
    /// `crates/hew-cli`'s `CliHost`) need no filesystem to produce bytes,
    /// only a `path` would — so a live export produces the bytes and
    /// hands them back for the caller to write, and refuses only that one
    /// specific thing this boundary genuinely cannot do.
    fn export_document(
        &mut self,
        doc: &kernel::Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
        units: Option<&str>,
        selection: Option<&[api::locate::RenderRoot]>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        if path.is_some() {
            return Err(refuse(
                "write a file",
                "The WASM boundary has no filesystem of its own. Omit `path` to receive the \
                 bytes and write them yourself, or use the app's File > Export.",
            ));
        }
        // `true`: this is the `hew.doc.export` dispatch command, the same
        // non-interactive posture `hew-cli`'s `CliHost` has — a leaky
        // object is dropped rather than silently handed back
        // (crates/mesh-export's module doc, "Non-solid inclusion"). The
        // desktop app's own interactive File > Export goes through
        // `Scene::export` instead, which passes `false`.
        let bytes = match format {
            "skp" | "skp2017" => {
                let target = if format == "skp" {
                    skp_export::Target::Skp2026
                } else {
                    skp_export::Target::Skp2017
                };
                let units = units
                    .or(self.display_units.as_deref())
                    .and_then(skp_export::DisplayUnits::from_name);
                skp_export::export_in(doc, target, crate::write_env(), units)
                    .map_err(|e| Refusal::api("export_failed", &e.to_string()))?
                    .bytes
            }
            _ => match selection {
                Some(roots) => {
                    mesh_export::export_occurrences(doc, roots, format, segments_per_turn, true)
                        .map_err(export_refusal)?
                }
                None => mesh_export::export(doc, format, segments_per_turn, true)
                    .map_err(export_refusal)?,
            },
        };
        Ok(Some(bytes))
    }

    /// Merges a file the CLIENT read — this boundary has no filesystem —
    /// into the live document, additively, through the same importers the
    /// app's own File > Import uses. `hew-cli` (or any client) sends the
    /// file as `bytes_base64` (plus COLLADA `images`); a request with only
    /// a path cannot be served here and says so. Records the import the way
    /// [`crate::Scene::import_dae`] and friends do (embedded bytes, subject
    /// to the same size cap) so a bug-report recording replays it; the
    /// dispatcher therefore does not also record the envelope.
    ///
    /// The logic mirrors `crates/hew-cli`'s `CliHost::import_document` over
    /// the same importers — that crate cannot be shared with this one (it
    /// has a filesystem; this has none), so the two must stay in step.
    fn import_document(
        &mut self,
        doc: &mut kernel::Document,
        path: &str,
        options: &api::ImportOptions,
    ) -> Result<serde_json::Value, Refusal> {
        let Some(bytes) = options.bytes.as_deref() else {
            return Err(refuse(
                "import from a path",
                "The WASM boundary has no filesystem access of its own — send the file's bytes as \
                 `bytes_base64` (hew-cli does this for you).",
            ));
        };
        let file = std::path::Path::new(path);
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        // Recording waits until the merge has actually succeeded: a refused
        // import must leave no trace in a replay.
        let record: Box<dyn FnOnce() + '_>;
        let load_failed =
            |e: &dyn std::fmt::Display| Refusal::api("load_failed", &format!("{path}: {e}"));
        if let Some("png" | "jpg" | "jpeg") = ext.as_deref() {
            return import_picture(doc, path, bytes, options);
        }
        let (scene, missing, warnings) = match ext.as_deref() {
            Some("stl") => {
                let units = options.units.as_deref();
                let scale = match units {
                    Some("m") => 1.0,
                    Some("mm") => 0.001,
                    Some("cm") => 0.01,
                    Some("in") => 0.0254,
                    Some(other) => {
                        return Err(Refusal::api(
                            "units_required",
                            &format!("Unknown units \"{other}\". Use one of m, mm, cm, in."),
                        ));
                    }
                    None => {
                        return Err(Refusal::api(
                            "units_required",
                            "STL carries no units of its own; pass units: \"m\" | \"mm\" | \"cm\" | \"in\".",
                        ));
                    }
                };
                let name = file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                let out = stl_import::import(bytes, scale, name.as_deref())
                    .map_err(|e| load_failed(&e))?;
                record = Box::new(move || {
                    crate::recording::record_import("import_stl", bytes, |b| {
                        crate::recording::RecordedCall::ImportStl {
                            bytes: b,
                            unit_scale: scale,
                            name,
                        }
                    });
                });
                (out.scene, out.missing, out.warnings)
            }
            Some("glb") | Some("gltf") => {
                let out = gltf_import::import(bytes).map_err(|e| load_failed(&e))?;
                record = Box::new(move || {
                    crate::recording::record_import("import_gltf", bytes, |b| {
                        crate::recording::RecordedCall::ImportGltf { bytes: b }
                    });
                });
                (out.scene, out.missing, out.warnings)
            }
            Some("dae") => {
                let mut map = dae_import::ImageMap::new();
                for (uri, image) in &options.images {
                    let format = match image.format.as_deref() {
                        Some("jpeg") | Some("jpg") => kernel::ImageFormat::Jpeg,
                        _ => kernel::ImageFormat::Png,
                    };
                    map.insert(uri.clone(), (image.bytes.clone(), format));
                }
                let out = dae_import::import(bytes, &map).map_err(|e| load_failed(&e))?;
                let total = bytes.len() + map.values().map(|(b, _)| b.len()).sum::<usize>();
                let recorded_map = map;
                record = Box::new(move || {
                    crate::recording::record_import_sized("import_dae", total, || {
                        crate::recording::RecordedCall::ImportDae {
                            bytes: bytes.to_vec(),
                            images: recorded_map
                                .iter()
                                .map(|(uri, (b, format))| crate::recording::RecordedImage {
                                    uri: uri.clone(),
                                    bytes: b.clone(),
                                    format: match format {
                                        kernel::ImageFormat::Jpeg => 1,
                                        _ => 0,
                                    },
                                })
                                .collect(),
                        }
                    });
                });
                (out.scene, out.textures_missing, out.warnings)
            }
            Some("skp") => {
                let out = skp_import::import(bytes).map_err(|e| load_failed(&e))?;
                record = Box::new(move || {
                    crate::recording::record_import("import_skp", bytes, |b| {
                        crate::recording::RecordedCall::ImportSkp { bytes: b }
                    });
                });
                (out.scene, out.textures_missing, out.warnings)
            }
            _ => {
                return Err(Refusal::api(
                    "unsupported_format",
                    &format!(
                        "Unrecognized import extension for \"{path}\" — expected .stl, .gltf/.glb, .dae, .skp, or a .png/.jpg picture."
                    ),
                ));
            }
        };
        let (report, _change) = doc
            .ingest(scene, missing)
            .map_err(|e| Refusal::from_document_error(&e))?;
        record();
        Ok(serde_json::json!({
            "objects_created": report.objects_created,
            "watertight": report.watertight,
            "leaky": report.leaky,
            "health": api::health_counts_json(&report.health),
            "skipped": report.skipped.iter().map(|s| serde_json::json!({
                "name": s.name,
                "reason": s.reason,
            })).collect::<Vec<_>>(),
            "textures_missing": report.textures_missing,
            "warnings": warnings,
        }))
    }

    /// Rendered here by the same software rasterizer `hew-cli` uses
    /// (`crates/headless-render`), synchronously on the calling thread; the
    /// request's size clamp is the only bound. Read-only: it touches
    /// nothing but the document it is handed.
    fn snapshot(
        &mut self,
        doc: &kernel::Document,
        params: &api::SnapshotParams,
    ) -> Result<api::SnapshotResult, Refusal> {
        headless_render::snapshot(doc, params)
    }

    fn line_drawing(
        &mut self,
        doc: &kernel::Document,
        params: &api::LineDrawingParams,
    ) -> Result<api::LineDrawingResult, Refusal> {
        headless_render::line_drawing(doc, params)
    }

    fn write_snapshot(&mut self, _path: &str, _bytes: &[u8]) -> Result<(), Refusal> {
        Err(refuse(
            "write files",
            "The WASM boundary has no filesystem access of its own.",
        ))
    }

    /// The browser's clock: `std::time::SystemTime::now()` panics on
    /// `wasm32-unknown-unknown`, which used to wedge the whole app the
    /// first time a live client called `hew.library.save`.
    fn unix_time_ms(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::now() as u64
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::time::SystemTime::now() // wasm-guard: cfg(not(wasm32)) branch only
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        }
    }

    /// `crypto.getRandomValues` ([`random_16`]), the source a document id
    /// needs: `random_u64` keeps `Math.random()`.
    fn random_16(&self) -> [u8; 16] {
        random_16()
    }

    /// `Math.random()` — 53 bits twice over — in place of the native
    /// default's pid and OS entropy, neither of which exists here.
    fn random_u64(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            let hi = (js_sys::Math::random() * 4_294_967_296.0) as u64;
            let lo = (js_sys::Math::random() * 4_294_967_296.0) as u64;
            (hi << 32) | lo
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0x1234_5678_9ABC_DEF1);
            COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        }
    }

    fn set_camera(&mut self, spec: &ViewCameraSpec) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::from(spec));
        Ok(())
    }

    fn zoom_extents(&mut self) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::ZoomExtents);
        Ok(())
    }

    fn set_display_units(&mut self, format: &str) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::Units {
            format: format.to_string(),
        });
        Ok(())
    }

    fn selection(&self) -> Result<SelectionState, Refusal> {
        Ok(self.selection.clone())
    }

    /// Merges into this dispatch's copy of the mirror and leaves the merged
    /// result for the app to apply wholesale.
    fn set_selection(
        &mut self,
        entries: Vec<SelectionEntry>,
        mode: SelectMode,
    ) -> Result<SelectionState, Refusal> {
        self.selection.entries = api::selection::merge(&self.selection.entries, &entries, mode);
        self.directives.push(ViewDirective::Selection {
            entries: self
                .selection
                .entries
                .iter()
                .map(|e| UiSelectionEntry::from_entry(*e))
                .collect(),
            mode: "replace".to_string(),
        });
        Ok(self.selection.clone())
    }

    fn hidden_changed(&mut self) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::HiddenChanged);
        Ok(())
    }

    fn isolation(&self) -> Result<Option<Vec<IsolationEntry>>, Refusal> {
        Ok(self.isolation.clone())
    }

    fn isolate(&mut self, entries: Vec<IsolationEntry>) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::Isolate {
            entries: entries
                .iter()
                .map(|e| UiIsolationEntry::from_entry(*e))
                .collect(),
            frame: true,
        });
        self.isolation = Some(entries);
        Ok(())
    }

    fn clear_isolation(&mut self) -> Result<(), Refusal> {
        self.isolation = None;
        self.directives.push(ViewDirective::ClearIsolation);
        Ok(())
    }

    fn fallback_context(&self) -> Vec<ContextEntry> {
        self.fallback.clone()
    }

    fn gesture_armed(&self) -> bool {
        self.gesture_armed
    }

    fn enter_user_context(
        &mut self,
        path: &[kernel::InstanceId],
        leaf: kernel::NodeId,
    ) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::EnterContext {
            path: path
                .iter()
                .map(|i| UiNode::from_node(kernel::NodeId::Instance(*i)))
                .collect(),
            leaf: UiNode::from_node(leaf),
        });
        Ok(())
    }

    fn exit_user_context(&mut self) -> Result<(), Refusal> {
        if self.at_top_level {
            return Err(Refusal::api(
                "nothing_to_exit",
                "Nothing is open for editing; the model is already at the top level.",
            ));
        }
        self.directives.push(ViewDirective::ExitContext);
        Ok(())
    }

    fn exit_all_user_contexts(&mut self) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::ExitAllContexts);
        Ok(())
    }

    fn zoom_to(
        &mut self,
        min: kernel::Point3,
        max: kernel::Point3,
        padding: f64,
    ) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::ZoomTo {
            min: [min.x, min.y, min.z],
            max: [max.x, max.y, max.z],
            padding,
        });
        Ok(())
    }

    /// `hew.scenes.apply`'s host notification: unlike the three effects
    /// above, this one is reachable through the SAME kernel-served
    /// dispatch path (`Document::apply_scene` already wrote the kernel
    /// state before `crates/api`'s handler calls this) — it never itself
    /// refuses, it only records the directive for the bridge to pick up.
    fn scene_applied(&mut self, sid: u64) -> Result<(), Refusal> {
        self.directives.push(ViewDirective::ActivateScene { sid });
        Ok(())
    }

    /// `hew.library.list`/`describe`/`remove`/`update_meta` need the
    /// library folder itself — this WASM boundary has no filesystem
    /// access of its own to list, read, or delete from (the library lives
    /// wherever the CONNECTING client resolves it, not inside the
    /// browser/webview sandbox). Refuses pointing specifically at
    /// `hew-cli --live`'s own answer to this: it pre-resolves exactly
    /// these four commands on ITS side before they ever reach the wire
    /// (docs/agents/HEW_API.md §12.1), so a real `--live` session never
    /// actually calls this — only a raw client bypassing that pre-resolve
    /// would ever see this refusal.
    fn library_list(&self) -> Result<LibraryListing, Refusal> {
        Err(refuse(
            "list the library",
            "The live desktop connection has no filesystem of its own here — hew-cli --live \
             answers hew.library.list/describe/remove/update_meta locally instead of forwarding \
             them; a raw client should do the same.",
        ))
    }

    fn library_read(&self, path: &str) -> Result<LibraryReadResult, Refusal> {
        let _ = path;
        Err(refuse(
            "read library items",
            "The live desktop connection has no filesystem of its own here — hew-cli --live \
             pre-resolves hew.library.insert's item to bytes_base64 locally before forwarding; a \
             raw client should do the same rather than sending item.",
        ))
    }

    /// `hew.library.save`'s write step — the ONE `library_*` effect this
    /// boundary CAN perform without a filesystem, the same way
    /// [`Self::save_document`] and [`Self::export_document`] answer their
    /// own bytes-only requests: it has no disk to put the bytes on, but
    /// hashing them needs none either. `path` is a placeholder — nothing
    /// on this side of the connection is a meaningful filesystem location,
    /// and `hew-cli --live` already knows to discard it and mint its OWN
    /// local path from the returned item's `hew.library` id/name/category
    /// after decoding the bytes (docs/agents/HEW_API.md §12.1) — this
    /// method's real job is letting `hew.library.save`'s handler
    /// (`crates/api/src/commands/library.rs`) complete `extract_item` +
    /// `stamp_library_source` against the LIVE document and hand the bytes
    /// back, which is the whole point of a live save.
    fn library_write(
        &mut self,
        target: LibraryWriteTarget,
        bytes: &[u8],
    ) -> Result<LibraryWriteResult, Refusal> {
        let _ = target;
        Ok(LibraryWriteResult {
            path: "(no filesystem on the live desktop connection)".to_string(),
            content_hash: sha256_hex(bytes),
        })
    }

    fn library_remove(&mut self, path: &str) -> Result<(), Refusal> {
        let _ = path;
        Err(refuse(
            "remove library items",
            "The live desktop connection has no filesystem of its own here — hew-cli --live \
             answers hew.library.remove locally instead of forwarding it; a raw client should do \
             the same.",
        ))
    }
}

/// SHA-256 of `bytes`, lowercase hex — identical scheme to
/// `crates/library::sha256_hex` (every host that computes a
/// `LibraryProvenance.content_hash` must agree, or a live-saved item's
/// stamped hash would mismatch what `hew-cli --live` computes for the same
/// bytes when it writes them to disk).
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `hew.doc.import` of a picture: placed exactly as [`crate::Scene::place_image`]
/// places one (`image_import`), at the request's `origin`/`u`/`v`/`width`
/// (each defaulting to [`image_import::Placement::from_parts`]'s), named
/// after the file, and recorded as that call so a bug-report recording
/// replays it. `crates/hew-cli`'s headless host does the same over the
/// same crate.
fn import_picture(
    doc: &mut kernel::Document,
    path: &str,
    bytes: &[u8],
    options: &api::ImportOptions,
) -> Result<serde_json::Value, Refusal> {
    let placement =
        image_import::Placement::from_parts(options.origin, options.u, options.v, options.width);
    let name = image_import::picture_name(path);
    let out = image_import::import(bytes, None, &name, &placement)
        .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
    let (format, width, height) = (out.info.format, out.width, out.height);
    let report = ingest_picture(doc, path, out.scene)?;
    let p = |q: kernel::Point3| [q.x, q.y, q.z];
    let d = |q: kernel::Vec3| [q.x, q.y, q.z];
    crate::recording::record_import("place_image", bytes, |b| {
        crate::recording::RecordedCall::PlaceImage {
            image: b,
            format: match format {
                kernel::ImageFormat::Jpeg => 1,
                kernel::ImageFormat::Png => 0,
            },
            name,
            origin: p(placement.origin),
            u: d(placement.u),
            v: d(placement.v),
            width,
        }
    });
    Ok(picture_report(&report, width, height))
}

/// Ingests a picture's recipe, checked first ([`image_import::check`]), so
/// a recipe the kernel would not build is refused before anything changes:
/// the document, its undo and redo history and its palette stay as they
/// were. Past the check, a face the kernel skipped would be a kernel bug;
/// it is still refused, never reported as a success, and the refusal says
/// the document may hold part of the import.
fn ingest_picture(
    doc: &mut kernel::Document,
    path: &str,
    scene: kernel::ImportScene,
) -> Result<kernel::ImportReport, Refusal> {
    image_import::check(&scene)
        .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
    let (report, _change) = doc
        .ingest(scene, Vec::new())
        .map_err(|e| Refusal::from_document_error(&e))?;
    if image_import::placed(&report).is_none() {
        let why = report
            .skipped
            .first()
            .map_or("nothing was placed", |s| s.reason.as_str());
        return Err(Refusal::api(
            "load_failed",
            &format!(
                "{path}: the picture was not placed whole ({why}); the document may hold part of the import"
            ),
        ));
    }
    Ok(report)
}

/// The import report of a placed picture: the usual counts plus the
/// picture's placed size in meters.
pub(crate) fn picture_report(
    report: &kernel::ImportReport,
    width: f64,
    height: f64,
) -> serde_json::Value {
    serde_json::json!({
        "objects_created": report.objects_created,
        "watertight": report.watertight,
        "leaky": report.leaky,
        "health": api::health_counts_json(&report.health),
        "skipped": report.skipped.iter().map(|s| serde_json::json!({
            "name": s.name,
            "reason": s.reason,
        })).collect::<Vec<_>>(),
        "textures_missing": report.textures_missing,
        "warnings": [],
        "picture": { "width": width, "height": height },
    })
}

/// 16 bytes from the platform's cryptographic source, for minting a
/// document id (`kernel::Document::ensure_document_id`). In the page and in
/// a Worker alike, `crypto.getRandomValues` reached through the global
/// object (`window` does not exist in a Worker); should an engine lack it,
/// `Math.random()` fills the bytes instead rather than failing a save.
/// Native builds (tests only) use a deterministic counter.
pub(crate) fn random_16() -> [u8; 16] {
    let mut out = [0u8; 16];
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let buf = js_sys::Uint8Array::new_with_length(16);
        let filled = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))
            .ok()
            .filter(|c| c.is_object())
            .and_then(|crypto| {
                let f =
                    js_sys::Reflect::get(&crypto, &JsValue::from_str("getRandomValues")).ok()?;
                let f: js_sys::Function = f.dyn_into().ok()?;
                f.call1(&crypto, &buf).ok()
            })
            .is_some();
        if filled {
            buf.copy_to(&mut out);
        } else {
            for b in &mut out {
                *b = (js_sys::Math::random() * 256.0) as u8;
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        out[..8].copy_from_slice(&n.to_le_bytes());
        out[8..].copy_from_slice(&n.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{Connection, DispatchOutcome, Grant, Request, RequestId};

    fn req(id: i64, method: &str, params: serde_json::Value) -> Request {
        Request {
            document: None,
            jsonrpc: "2.0".to_string(),
            id: Some(RequestId::Number(id)),
            method: method.to_string(),
            params: Some(params),
        }
    }

    /// A document with exactly one watertight box object, built through
    /// `Document::ingest` — the same fixture `crates/mesh-export`'s and
    /// `crates/hew-cli`'s own tests use.
    fn box_document() -> kernel::Document {
        let positions = vec![
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 1.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
            kernel::Point3::new(0.0, 0.0, 1.0),
            kernel::Point3::new(1.0, 0.0, 1.0),
            kernel::Point3::new(1.0, 1.0, 1.0),
            kernel::Point3::new(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let face_count = faces.len();
        let mesh = kernel::MeshRecipe {
            name: "Box".to_string(),
            positions,
            faces,
            face_materials: vec![kernel::serialize::NO_MATERIAL; face_count],
            face_uv_frames: vec![None; face_count],
            face_holes: vec![Vec::new(); face_count],
            base_material: kernel::serialize::NO_MATERIAL,
            tags: Vec::new(),
            extra: Default::default(),
        };
        let mut doc = kernel::Document::new();
        let scene = kernel::ImportScene {
            materials: Vec::new(),
            defs: Vec::new(),
            roots: vec![kernel::ImportNode::Mesh(mesh)],
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            extra: Default::default(),
        };
        doc.ingest(scene, Vec::new()).expect("ingest a plain box");
        doc
    }

    /// A live export of the SketchUp formats hands back real files in the
    /// requested container.
    #[test]
    fn export_document_hands_back_both_skp_containers() {
        let doc = box_document();
        let mut host = LiveHost::default();
        for (format, container) in [
            ("skp", skp_import::openskp::Container::Zip),
            ("skp2017", skp_import::openskp::Container::Carchive2017),
        ] {
            let bytes = host
                .export_document(&doc, format, None, 0, None, None)
                .expect("a box exports as a .skp live")
                .expect("bytes come back");
            assert_eq!(skp_import::openskp::detect_container(&bytes), container);
        }
    }

    /// A live SketchUp export opens in the app's display units, as File >
    /// Export writes it, unless the caller names units of its own.
    #[test]
    fn a_live_skp_export_takes_the_apps_units_unless_named() {
        let doc = box_document();
        let mut host = LiveHost {
            display_units: Some("mm".to_string()),
            ..LiveHost::default()
        };
        let unit = |host: &mut LiveHost, units: Option<&str>| {
            let bytes = host
                .export_document(&doc, "skp", None, 0, units, None)
                .unwrap()
                .unwrap();
            skp_import::openskp::Model::parse(&bytes)
                .unwrap()
                .units
                .length_unit
        };
        assert_eq!(unit(&mut host, None), 2, "mm");
        assert_eq!(unit(&mut host, Some("cm")), 3, "cm");
    }

    /// A live save never produces a SketchUp file (the app owns the live
    /// document's file); it refuses typed, and a plain `.hew` save still
    /// hands bytes back.
    #[test]
    fn save_document_refuses_a_skp_format_but_serves_hew() {
        let doc = box_document();
        let mut host = LiveHost::default();
        for format in [DocFormat::Skp, DocFormat::Skp2017] {
            let err = host
                .save_document(&doc, None, Some(format), None)
                .unwrap_err();
            assert_eq!(err.name, "host_capability_missing");
        }
        let out = host
            .save_document(&doc, None, Some(DocFormat::Hew), None)
            .expect("hew saves");
        assert!(out.bytes.is_some());
    }

    /// A live export needs no filesystem to produce bytes, only the write
    /// does — mirrors `save_document`'s already-covered posture (see
    /// `crates/hew-cli/tests/cli.rs`'s `cli_host_exports_a_box_to_...`
    /// tests for the headless side of the same contract) and confirms it
    /// reaches the exact same shared writer `crates/mesh-export` gives
    /// every host, byte for byte.
    #[test]
    fn export_document_hands_back_bytes_matching_the_shared_writer() {
        let doc = box_document();
        let mut host = LiveHost::default();
        let bytes = host
            .export_document(&doc, "stl", None, 0, None, None)
            .expect("a solid box exports live")
            .expect("bytes come back since there is no filesystem to write to");
        let direct =
            mesh_export::export_stl(&doc, 0, true).expect("the same writer, called directly");
        assert_eq!(
            bytes, direct,
            "live export matches mesh-export byte for byte"
        );
    }

    /// A `path` is the one part of export this boundary genuinely cannot
    /// honor — refused specifically, not the whole command.
    #[test]
    fn export_document_refuses_a_path_since_the_wasm_boundary_has_no_filesystem() {
        let doc = box_document();
        let mut host = LiveHost::default();
        let err = host
            .export_document(&doc, "stl", Some("/tmp/out.stl"), 0, None, None)
            .unwrap_err();
        assert_eq!(err.name, "host_capability_missing");
    }

    #[test]
    fn export_document_refuses_nothing_to_export_on_an_empty_document() {
        let doc = kernel::Document::new();
        let mut host = LiveHost::default();
        let err = host
            .export_document(&doc, "stl", None, 0, None, None)
            .unwrap_err();
        assert_eq!(err.name, "nothing_to_export");
    }

    /// A live connection's `hew.doc.new` refuses typed instead of wiping
    /// the document out from under the app — the whole point of
    /// `LiveHost` withholding document lifecycle.
    #[test]
    fn live_host_refuses_new_document_typed() {
        let mut conn = Connection::new(Grant::app(), "test");
        let mut doc = kernel::Document::new();
        let mut host = LiveHost::default();
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(2, "hew.doc.new", serde_json::json!({})),
        ) else {
            panic!("dispatch replies")
        };
        let err = r.error.expect("hew.doc.new refuses on a live host");
        let data = err.data.expect("refusal carries data");
        assert_eq!(data["refusal"], "host_capability_missing");
    }

    /// `hew.library.save` over a LIVE connection (docs/agents/HEW_API.md
    /// §12.1): `LiveHost::library_write` has no filesystem, but it must
    /// still let the command complete `extract_item`/`stamp_library_source`
    /// against the real live document and hand real bytes back — a
    /// regression test for the bug where `LiveHost` implemented none of
    /// the four `library_*` methods at all, so this refused
    /// `host_capability_missing` before `crates/api`'s handler ever got to
    /// build anything. Dispatches through a REAL `api::Connection`, not a
    /// unit call on the handler — the whole point is proving the
    /// dispatcher's `hew.doc.transact` wrapping, the kernel-side
    /// extract/stamp, and this host's write all compose correctly, exactly
    /// as a real `--live` save would exercise them.
    #[test]
    fn library_save_over_a_live_connection_extracts_stamps_and_returns_bytes() {
        let mut conn = Connection::new(Grant::app(), "test");
        let mut doc = box_document();
        let mut host = LiveHost::default();
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );

        // The box object's own public id, to save it as a selection (not
        // the whole-document path) — the shape that ALSO exercises
        // `Document::stamp_library_source`, which the whole-document save
        // has nothing to stamp.
        let kernel::NodeId::Object(box_id) = doc
            .top_level_nodes()
            .into_iter()
            .next()
            .expect("box_document has one top-level object")
        else {
            panic!("box_document's root is an object");
        };
        let public_id = api::IdResolver::new(&doc)
            .public_of(&doc, &kernel::EntityRef::Object(box_id))
            .expect("a live object carries a stable id");

        let DispatchOutcome::Reply(r) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(
                2,
                "hew.library.save",
                serde_json::json!({
                    "selection": [public_id],
                    "name": "Box",
                    "return_bytes": true,
                }),
            ),
        ) else {
            panic!("dispatch replies")
        };
        assert!(r.error.is_none(), "live save should succeed: {:?}", r.error);
        let saved = r.result.expect("a successful reply carries a result");
        // `hew.library.save` is `ReadOnly` class (records no undo entry —
        // docs/agents/HEW_API.md's Library semantics note), so a plain
        // request runs BARE, unlike a `ModelMutating` command — no
        // `results[0]` transaction wrapper to unwrap here.
        assert!(saved["id"].is_string(), "{saved}");
        let b64 = saved["bytes_base64"]
            .as_str()
            .unwrap_or_else(|| panic!("return_bytes: true must hand bytes back — {saved}"));
        assert!(!b64.is_empty());

        // `Document::stamp_library_source` ran against the LIVE document:
        // the saved object now carries `hew.library` provenance — the
        // effect `LiveHost::library_write`'s real (not placeholder) content
        // hash makes possible.
        let dict = doc
            .attr_get(&kernel::AttrTarget::Entity(kernel::EntityRef::Object(
                box_id,
            )))
            .expect("the object is still live")
            .expect("provenance was stamped");
        assert!(
            dict.contains_key("hew.library"),
            "the saved object should carry hew.library provenance"
        );
    }

    /// Dispatches one command on a fresh live host holding a one-box
    /// document, after the hello/attach handshake.
    fn dispatch_on_box(method: &str, params: serde_json::Value) -> api::Response {
        let mut conn = Connection::new(Grant::app(), "test");
        let mut doc = kernel::Document::new();
        let ground = kernel::Plane::from_polygon(&[
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(ground);
        let c = [
            kernel::Point3::new(0.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 0.0, 0.0),
            kernel::Point3::new(1.0, 1.0, 0.0),
            kernel::Point3::new(0.0, 1.0, 0.0),
        ];
        for k in 0..4 {
            doc.sketch_mut(s)
                .unwrap()
                .add_segment(c[k], c[(k + 1) % 4])
                .unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], 1.0).unwrap();
        let mut host = LiveHost::default();
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(2, method, params),
        ) else {
            panic!("dispatch replies")
        };
        r
    }

    /// The app renders `hew.view.snapshot` itself: a PNG, no id palette
    /// unless asked for.
    #[test]
    fn live_host_renders_a_snapshot() {
        let r = dispatch_on_box("hew.view.snapshot", serde_json::json!({}));
        assert!(r.error.is_none(), "{:?}", r.error);
        let result = r.result.expect("a result");
        assert_eq!(
            (result["width"].as_u64(), result["height"].as_u64()),
            (Some(512), Some(512))
        );
        let b64 = result["png_base64"].as_str().expect("inline png");
        assert!(b64.starts_with("iVBORw0KGgo"), "PNG signature: {b64:.16}");
        assert!(
            result
                .get("id_palette")
                .is_none_or(|p| p.as_array().is_some_and(Vec::is_empty))
        );
    }

    #[test]
    fn live_host_renders_a_line_drawing() {
        let r = dispatch_on_box(
            "hew.view.line_drawing",
            serde_json::json!({ "format": "segments", "view": "iso" }),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let result = r.result.expect("a result");
        assert!(result["count"].as_u64().unwrap() > 0);
        assert!(!result["segments"].as_array().unwrap().is_empty());
    }

    /// There is still no filesystem at the wasm boundary: a `path` is
    /// refused after the render, by `write_snapshot`.
    #[test]
    fn live_host_still_refuses_a_snapshot_path() {
        let r = dispatch_on_box(
            "hew.view.snapshot",
            serde_json::json!({ "path": "/tmp/x.png" }),
        );
        let err = r.error.expect("a path is refused");
        assert_eq!(err.data.unwrap()["refusal"], "host_capability_missing");
    }

    /// A kernel-served command (not a `Host` effect at all) actually runs
    /// against the document — this is the whole reason `--live` is
    /// useful despite every `Host` method above refusing.
    #[test]
    fn kernel_served_commands_run_for_real_on_a_live_host() {
        let mut conn = Connection::new(Grant::app(), "test");
        let mut doc = kernel::Document::new();
        let mut host = LiveHost::default();
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(1, "hew.doc.attach", serde_json::json!({})),
        );
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(2, "hew.query.scene", serde_json::json!({})),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            r.error.is_none(),
            "a read-only kernel command succeeds live: {:?}",
            r.error
        );
    }

    /// `hew.scenes.apply` — a kernel-served command, unlike `hew.view.*` —
    /// still leaves a `ViewDirective::ActivateScene` for the bridge, via
    /// `LiveHost::scene_applied` (`Host::scene_applied`'s live impl):
    /// `crates/api`'s handler writes the kernel-side state for real AND
    /// notifies the host, both in the same dispatch.
    #[test]
    fn scene_apply_leaves_an_activate_scene_directive() {
        let mut conn = Connection::new(Grant::app(), "test");
        let mut doc = kernel::Document::new();
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut LiveHost::default(),
            req(0, "hew.meta.hello", serde_json::json!({"protocol": 1})),
        );
        conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut LiveHost::default(),
            req(1, "hew.doc.attach", serde_json::json!({})),
        );

        // Add a Scene through the same live connection first.
        let mut host = LiveHost::default();
        let DispatchOutcome::Reply(add_reply) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(2, "hew.scenes.add", serde_json::json!({ "name": "Front" })),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            add_reply.error.is_none(),
            "add succeeds: {:?}",
            add_reply.error
        );
        // A plain (non-`hew.doc.transact`) mutating dispatch is exactly a
        // one-command transaction (docs/agents/HEW_API.md §6.1): the reply is
        // `{"results": [...], "label": ...}`, the add's own result at
        // `results[0]`, not the bare command result.
        let add_body = add_reply.result.expect("add succeeds");
        let add_result = &add_body["results"][0];
        let sid = add_result["sid"].as_u64().expect("sid is a number");
        let id = add_result["id"]
            .as_str()
            .expect("id is a string")
            .to_string();

        // A directive from `add` (registry-state, ModelMutating, no
        // Host effect) must not leak into the NEXT dispatch's directive.
        assert!(host.directives.is_empty());

        let mut host = LiveHost::default();
        let DispatchOutcome::Reply(apply_reply) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(3, "hew.scenes.apply", serde_json::json!({ "id": id })),
        ) else {
            panic!("dispatch replies")
        };
        assert!(
            apply_reply.error.is_none(),
            "apply succeeds: {:?}",
            apply_reply.error
        );
        match host.directives.as_slice() {
            [ViewDirective::ActivateScene { sid: got }] => assert_eq!(*got, sid),
            other => panic!("expected an ActivateScene directive, got {other:?}"),
        }
    }

    /// The box above, as binary STL bytes — a real file for the import
    /// tests below to merge.
    fn box_stl() -> Vec<u8> {
        mesh_export::export(&box_document(), "stl", 0, true).expect("export the box to STL")
    }

    fn import_params(path: &str, bytes: Option<&[u8]>, units: Option<&str>) -> serde_json::Value {
        let mut p = serde_json::json!({ "path": path });
        if let Some(bytes) = bytes {
            p["bytes_base64"] = serde_json::Value::String(api::commands::doc::encode_base64(bytes));
        }
        if let Some(units) = units {
            p["units"] = serde_json::Value::String(units.to_string());
        }
        p
    }

    fn app_connection(doc: &mut kernel::Document) -> Connection {
        let mut conn = Connection::new(Grant::app(), "t");
        let mut host = LiveHost::default();
        for (id, method, params) in [
            (1, "hew.meta.hello", serde_json::json!({ "protocol": 1 })),
            (2, "hew.doc.attach", serde_json::json!({})),
        ] {
            conn.dispatch(
                &api::Registry::protocol_1(),
                doc,
                &mut host,
                req(id, method, params),
            );
        }
        conn
    }

    #[test]
    fn a_live_import_merges_the_clients_bytes_additively_and_reports_warnings() {
        let mut doc = box_document();
        let before = doc.undo_depth();
        let mut conn = app_connection(&mut doc);
        let mut host = LiveHost::default();
        let stl = box_stl();
        let DispatchOutcome::Reply(r) = conn.dispatch(
            &api::Registry::protocol_1(),
            &mut doc,
            &mut host,
            req(
                3,
                "hew.doc.import",
                import_params("part.stl", Some(&stl), Some("m")),
            ),
        ) else {
            panic!("a request is answered");
        };
        let report = r.result.expect("import succeeds")["report"].clone();
        assert_eq!(report["objects_created"], 1, "{report}");
        assert!(
            report["warnings"].is_array(),
            "the report carries warnings: {report}"
        );
        assert!(doc.undo_depth() > before, "the merge is one undoable entry");
    }

    #[test]
    fn a_live_import_without_bytes_or_units_refuses_typed_and_changes_nothing() {
        let mut doc = box_document();
        let before = doc.undo_depth();
        let mut conn = app_connection(&mut doc);
        let mut host = LiveHost::default();
        let stl = box_stl();
        for (params, refusal) in [
            (
                import_params("part.stl", None, Some("m")),
                "host_capability_missing",
            ),
            (
                import_params("part.stl", Some(&stl), None),
                "units_required",
            ),
            (
                import_params("part.stl", Some(&stl), Some("furlongs")),
                "units_required",
            ),
            (
                import_params("part.xyz", Some(&stl), None),
                "unsupported_format",
            ),
            (
                import_params("part.stl", Some(b"not an stl"), Some("m")),
                "load_failed",
            ),
        ] {
            let DispatchOutcome::Reply(r) = conn.dispatch(
                &api::Registry::protocol_1(),
                &mut doc,
                &mut host,
                req(9, "hew.doc.import", params.clone()),
            ) else {
                panic!("a request is answered");
            };
            let data = r
                .error
                .unwrap_or_else(|| panic!("{params} should refuse"))
                .data;
            assert_eq!(data.expect("refusal data")["refusal"], refusal, "{params}");
        }
        assert_eq!(
            doc.undo_depth(),
            before,
            "a refused import leaves the document alone"
        );
    }

    #[test]
    fn import_is_registered_as_a_mutation_so_the_live_bridge_resyncs() {
        let registry = api::Registry::protocol_1();
        assert!(
            registry
                .get("hew.doc.import")
                .expect("declared")
                .mutates_document
        );
    }

    /// A picture recipe the kernel would not build is refused before the
    /// document is touched: the saved bytes, the undo depth, the redo
    /// history (an earlier undone picture still redoes) and the palette
    /// are all as they were, rather than an ingest taken back by an undo
    /// that wipes the redo and leaves the picture's material behind.
    #[test]
    fn a_picture_the_kernel_would_not_build_is_refused_before_anything_changes() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let recipe = || {
            image_import::import(&png, None, "speck", &image_import::Placement::ground(1.0))
                .unwrap()
        };
        let mut doc = kernel::Document::new();
        ingest_picture(&mut doc, "kept.png", recipe().scene).unwrap();
        ingest_picture(&mut doc, "undone.png", recipe().scene).unwrap();
        doc.undo().unwrap();
        let before = doc.save();
        let (undo, redo) = (doc.undo_depth(), doc.redo_depth());
        let materials = doc.material_ids().len();
        assert_eq!(redo, 1);

        let mut out = recipe();
        let kernel::ImportNode::Mesh(mesh) = &mut out.scene.defs[0].children[0] else {
            panic!("the definition holds the picture's mesh");
        };
        for p in &mut mesh.positions {
            *p = kernel::Point3::new(p.x * 1e-7, p.y * 1e-7, 0.0);
        }
        let Err(refusal) = ingest_picture(&mut doc, "speck.png", out.scene) else {
            panic!("a face the kernel would skip is refused");
        };
        assert_eq!(refusal.name, "load_failed");
        assert!(
            refusal.explanation.contains("can't be built"),
            "{}",
            refusal.explanation
        );
        assert_eq!(doc.save(), before, "the document is unchanged");
        assert_eq!((doc.undo_depth(), doc.redo_depth()), (undo, redo));
        assert_eq!(doc.material_ids().len(), materials, "no material left");
        doc.redo().unwrap();
        assert_eq!(doc.instance_ids().len(), 2, "the undone picture redoes");
    }
}
