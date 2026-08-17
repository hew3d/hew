//! The typed host trait (docs/HEW_API.md §3): the narrow seam through
//! which host-implemented commands (file paths, importers, rendering)
//! reach the outside world. `crates/api` stays pure — it validates,
//! enforces profiles, and shapes responses; the host supplies only the
//! effect, to its ability. A host lacking a capability answers a typed
//! refusal, advertised via capabilities, never a protocol error.

use crate::refusal::Refusal;

/// The refusal every unsupported host effect answers.
fn unsupported(what: &str) -> Refusal {
    Refusal::api(
        "host_capability_missing",
        &format!("This host cannot {what}. Connect through a host that can (hew-cli has file access; the desktop app renders)."),
    )
    .with_detail(serde_json::json!({ "capability": what }))
}

// ------------------------------------------------------------ hew.view.snapshot

/// A named standard view (docs/HEW_API.md §7; mirrors
/// `softrender::StandardView` one-to-one). Kept as its own copy here
/// rather than a dependency on `softrender` — `crates/api` stays typed
/// against the host boundary without pulling in a renderer (§3's layering:
/// `api` depends on `kernel`, `inference`, `tessellate` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl StandardView {
    /// Parses a `view` parameter's name; `None` for anything else, which
    /// `commands/doc.rs` turns into a `CmdError::Params`.
    pub fn from_name(name: &str) -> Option<StandardView> {
        Some(match name {
            "iso" => StandardView::Iso,
            "front" => StandardView::Front,
            "back" => StandardView::Back,
            "left" => StandardView::Left,
            "right" => StandardView::Right,
            "top" => StandardView::Top,
            "bottom" => StandardView::Bottom,
            _ => return None,
        })
    }
}

/// An explicit camera projection, as given to `hew.view.snapshot`'s
/// `camera.projection` — `None` (absent) defaults to `Perspective`, a
/// host's job to resolve (docs/design/headless-snapshot.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotProjection {
    Perspective,
    Parallel,
}

/// An explicit camera for `hew.view.snapshot` (docs/HEW_API.md §7):
/// `up`, `projection`, and `fov_deg` are optional in the wire params and
/// stay optional here — a host resolves their defaults (identity up
/// `[0,0,1]`, perspective, 35°) when building its own renderer's camera,
/// per `docs/design/headless-snapshot.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapshotCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: Option<[f64; 3]>,
    pub projection: Option<SnapshotProjection>,
    pub fov_deg: Option<f64>,
}

/// Typed, validated parameters for `hew.view.snapshot`
/// (docs/design/headless-snapshot.md). `crates/api` owns parsing this out
/// of the wire JSON (`commands/doc.rs`) — width/height are already
/// defaulted (512) and clamped to `[16, 2048]`; `camera` and `view` are
/// already checked mutually exclusive; an unresolvable `view` name never
/// reaches a host. A host sees only a fully validated request.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotParams {
    pub width: u32,
    pub height: u32,
    /// An explicit camera. Mutually exclusive with `view` and `scene`.
    pub camera: Option<SnapshotCamera>,
    /// A named standard view, fitted to the scene. Mutually exclusive
    /// with `camera` and `scene`.
    pub view: Option<StandardView>,
    /// A Scene's stable id (`crates/api/src/commands/scenes.rs` has
    /// already resolved and validated the public id into this). Mutually
    /// exclusive with `camera` and `view`: renders through the Scene's
    /// resolved camera and hidden sets (`kernel::Document::resolve_scene`)
    /// instead of the document's own. When the Scene captures no camera,
    /// a host falls back to its usual cameraless resolution — the
    /// document's saved working camera, else a fitted isometric view —
    /// fitted to what the SCENE leaves visible, not the whole document.
    /// The Scene's section plane, if any, is NOT rendered headlessly at
    /// 1.0 (docs/HEW_API.md's Scenes section).
    pub scene: Option<u64>,
    /// When true, the result also carries a per-pixel id-buffer and the
    /// palette of public ids it indexes.
    pub include_ids: bool,
    /// When given, the PNG is written here instead of returned inline
    /// (docs/HEW_API.md §7, mirroring `hew.doc.export`'s posture):
    /// honored by hosts with filesystem access, refused typed elsewhere.
    /// `commands/doc.rs` still calls [`Host::snapshot`] to render the
    /// bytes; it is [`Host::write_snapshot`] that a host lacking
    /// filesystem access refuses.
    pub path: Option<String>,
}

/// What `hew.view.snapshot` returns (docs/design/headless-snapshot.md).
/// `id_buffer`/`id_palette` are only meaningful (and `id_buffer` only
/// `Some`) when the request's `include_ids` was true; `commands/doc.rs`
/// shapes the JSON result accordingly.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotResult {
    /// A complete PNG file's bytes.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Per-pixel index into `id_palette`, `u16` little-endian, 0 =
    /// background — present only when the request asked for it.
    pub id_buffer: Option<Vec<u8>>,
    /// `id_palette[i]` is the public id `id_buffer` reports as index
    /// `i + 1`. Empty when `id_buffer` is `None`.
    pub id_palette: Vec<String>,
}

/// The camera `hew.view.camera` sets — the SAME vocabulary
/// `hew.view.snapshot`'s `camera`/`view` accept ([`SnapshotCamera`],
/// [`StandardView`] above), parsed by the one shared implementation in
/// `crates/api/src/commands/camera.rs` so a live camera move and a
/// rendered snapshot's camera can never drift into two dialects of
/// "where is the camera" in one protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewCameraSpec {
    /// An explicit eye/target/up/projection/fov — exactly `hew.view.snapshot`'s `camera`.
    Explicit(SnapshotCamera),
    /// A named standard view, fitted to the scene — exactly `hew.view.snapshot`'s `view`.
    Standard(StandardView),
}

/// What a host can do for the dispatcher. Every method has a refusing
/// default, so a host implements exactly what it supports and the
/// dispatcher's contract never depends on which host is behind it.
pub trait Host {
    /// Replace the working document with a fresh one (`hew.doc.new`).
    fn new_document(&mut self, doc: &mut kernel::Document) -> Result<(), Refusal> {
        let _ = doc;
        Err(unsupported("create documents"))
    }

    /// Load `path` into the working document (`hew.doc.open`).
    fn open_document(&mut self, doc: &mut kernel::Document, path: &str) -> Result<(), Refusal> {
        let _ = (doc, path);
        Err(unsupported("open files"))
    }

    /// Persist the working document (`hew.doc.save`). `path` is required
    /// on the first save of a fresh document; `None` re-saves in place.
    /// Save the document (`hew.doc.save`): returns the `.hew` bytes
    /// (base64-encoded by the command) unless `path` was given and the
    /// host wrote it directly — the shape [`Host::export_document`]
    /// already had, for the same reason. A host with no filesystem of
    /// its own (the live WASM boundary) can still serve a save by
    /// handing the bytes back for whoever asked to write, rather than
    /// refusing an operation it can perform all but the last step of.
    fn save_document(
        &mut self,
        doc: &kernel::Document,
        path: Option<&str>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let _ = (doc, path);
        Err(unsupported("save files"))
    }

    /// Export the document (`hew.doc.export`): returns the exported bytes
    /// (base64-encoded by the command) unless `path` was given and the
    /// host wrote it directly. `segments_per_turn` is the STL curve
    /// re-facet resolution (0 = stored facets, tessellate::export_triangles's
    /// own convention); hosts exporting formats with no curve resolution
    /// concept ignore it.
    fn export_document(
        &mut self,
        doc: &kernel::Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let _ = (doc, format, path, segments_per_turn);
        Err(unsupported("export"))
    }

    /// Merge a foreign-format file into the document (`hew.doc.import`),
    /// returning the kernel's import report as JSON.
    fn import_document(
        &mut self,
        doc: &mut kernel::Document,
        path: &str,
        options: &serde_json::Value,
    ) -> Result<serde_json::Value, Refusal> {
        let _ = (doc, path, options);
        Err(unsupported("import"))
    }

    /// Render the document to PNG (`hew.view.snapshot`): a headless host
    /// renders through a software rasterizer (`hew-cli` uses
    /// `crates/softrender`); a live host may render through its actual
    /// viewport instead. Either way `crates/api` has already validated
    /// `params` (width/height defaulted and clamped, `camera`/`view`
    /// mutually exclusive, an unknown view name rejected) — a host
    /// implementing this sees only a well-formed request and answers
    /// `nothing_to_render` when the document has no visible geometry.
    fn snapshot(
        &mut self,
        doc: &kernel::Document,
        params: &SnapshotParams,
    ) -> Result<SnapshotResult, Refusal> {
        let _ = (doc, params);
        Err(unsupported("render snapshots"))
    }

    /// Writes bytes produced by [`Host::snapshot`] to `path`
    /// (`hew.view.snapshot`'s `path` parameter, docs/HEW_API.md §7):
    /// mirrors `export_document`'s path posture, split into its own
    /// method because `crates/api` stays pure and cannot write the file
    /// itself — the host does, to its ability. `commands/doc.rs` calls
    /// this once for the PNG and, when `include_ids` also asked for the
    /// id-buffer, a second time for its `<path>.ids.bin` sidecar; either
    /// way `bytes` is just the blob to write, and this method is
    /// otherwise agnostic to which one it is.
    fn write_snapshot(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        let _ = (path, bytes);
        Err(unsupported("write snapshot files"))
    }

    /// The host's open documents (`hew.meta.documents` and the `hello`
    /// response). A headless host has exactly its working document.
    fn documents(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }

    /// Set the live viewport's camera (`hew.view.camera`, docs/HEW_API.md
    /// §7): the same camera vocabulary `hew.view.snapshot` accepts,
    /// applied to the actual on-screen viewport instead of rendered to
    /// bytes. This is a host effect on the VIEW, not a document edit —
    /// the registry declares `mutates_document = false` for this command,
    /// so it is never recorded and never triggers a resync (unlike a
    /// kernel-served command, a refused or successful call here leaves
    /// the document, and the undo log, exactly as it found them). A
    /// headless host has no viewport and refuses `host_capability_missing`;
    /// a headless client passes a camera per `hew.view.snapshot` call
    /// instead.
    fn set_camera(&mut self, spec: &ViewCameraSpec) -> Result<(), Refusal> {
        let _ = spec;
        Err(unsupported("move the viewport camera"))
    }

    /// Frame all visible geometry in the live viewport
    /// (`hew.view.zoom_extents`, docs/HEW_API.md §7) — the API
    /// counterpart of the app's View > Zoom Extents. Also a view
    /// effect, not a document edit: same `mutates_document = false`
    /// posture as [`Host::set_camera`]. A headless host refuses
    /// `host_capability_missing` — there is no viewport to frame.
    fn zoom_extents(&mut self) -> Result<(), Refusal> {
        Err(unsupported("frame the viewport to fit the model"))
    }

    /// Set the app's displayed length-unit format (`hew.view.units`,
    /// docs/HEW_API.md §7). `format` is one of
    /// `app/src/settings/units.ts`'s `LengthFormat` values (`"m"`,
    /// `"cm"`, `"mm"`, `"arch"`, `"frac_in"`, `"dec_in"`), already
    /// validated against that exact enum by `crates/api` before this is
    /// called. This is an APP-LEVEL DISPLAY PREFERENCE, not document
    /// state: it changes no kernel data, is never serialized into
    /// `.hew`, and — like `set_camera`/`zoom_extents` — is never
    /// recorded and never resyncs the document. A headless host has no
    /// display preference to set and refuses `host_capability_missing`.
    fn set_display_units(&mut self, format: &str) -> Result<(), Refusal> {
        let _ = format;
        Err(unsupported("set the display unit format"))
    }

    /// Notifies the host that a Scene just finished applying
    /// (`hew.scenes.apply`): called AFTER `kernel::Document::apply_scene`
    /// has already written the kernel-side state (tag/node hidden flags,
    /// section plane) — this is a best-effort signal for a host that
    /// wants to react further (drive a live viewport's camera, tell a
    /// Scene-tray UI which Scene is now active), never a second veto
    /// point: the document mutation already happened whether or not any
    /// host implements this. The default is a silent no-op success —
    /// unlike every other method on this trait, there is nothing
    /// "unsupported" about a host that simply has nothing extra to do
    /// (`NoHost`, `hew-cli`'s `CliHost`).
    fn scene_applied(&mut self, sid: u64) -> Result<(), Refusal> {
        let _ = sid;
        Ok(())
    }
}

/// The capability-free host: pure in-process dispatch with no file
/// system and no renderer. Every host effect refuses typed.
#[derive(Debug, Default)]
pub struct NoHost;

impl Host for NoHost {}
