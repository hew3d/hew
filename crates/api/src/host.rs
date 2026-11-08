//! The typed host trait (docs/agents/HEW_API.md §3): the narrow seam through
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

/// A named standard view (docs/agents/HEW_API.md §7; mirrors
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

/// An explicit camera for `hew.view.snapshot` (docs/agents/HEW_API.md §7):
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
    /// The height the view spans at `target`, meters (what a parallel
    /// view frames); `None` takes what the perspective view spans there.
    pub parallel_height: Option<f64>,
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
    /// 1.0 (docs/agents/HEW_API.md's Scenes section).
    pub scene: Option<u64>,
    /// When true, the result also carries a per-pixel id-buffer and the
    /// palette of public ids it indexes.
    pub include_ids: bool,
    /// When true, the result also carries a depth pass (a grayscale PNG,
    /// near = white, plus the metric range it encodes).
    pub include_depth: bool,
    /// Bits per pixel of the depth PNG: 8 or 16 (`commands/doc.rs` has
    /// already validated it; 8 when depth was not asked for).
    pub depth_bits: u8,
    /// When true, the result also carries a camera-space normals pass (an
    /// RGB PNG).
    pub include_normals: bool,
    /// When given, the PNG is written here instead of returned inline
    /// (docs/agents/HEW_API.md §7, mirroring `hew.doc.export`'s posture):
    /// honored by hosts with filesystem access, refused typed elsewhere.
    /// `commands/doc.rs` still calls [`Host::snapshot`] to render the
    /// bytes; it is [`Host::write_snapshot`] that a host lacking
    /// filesystem access refuses.
    pub path: Option<String>,
    /// Render only these roots, with the camera fitted to them (the
    /// command refuses an empty list, so a host sees `None` or at least
    /// one root). `None` renders the whole visible document. With
    /// neither `camera` nor `view`, the camera is a fitted isometric over
    /// the selection, never the document's saved working camera.
    pub selection: Option<Vec<crate::locate::RenderRoot>>,
    /// RGBA fill for pixels no surface touches; `[0, 0, 0, 0]` is a
    /// transparent snapshot. The id, depth, and normals passes are
    /// unaffected.
    pub background: [u8; 4],
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
    /// The depth pass — present only when the request asked for it.
    pub depth: Option<DepthPass>,
    /// The camera-space normals pass as a complete RGB PNG — present only
    /// when the request asked for it.
    pub normals_png: Option<Vec<u8>>,
}

/// A rendered depth pass (`hew.view.snapshot {include_depth}`).
#[derive(Debug, Clone, PartialEq)]
pub struct DepthPass {
    /// A complete grayscale PNG, `bits` bits per pixel. NEAR is WHITE.
    /// Background pixels are 0; every drawn surface maps linearly into
    /// `1..=max` (`max` = 255 or 65535) between `far` (1) and `near`
    /// (`max`), so `depth = far - (v - 1) / (max - 1) * (far - near)`; a
    /// scene at a single depth is a flat `max`.
    pub png: Vec<u8>,
    pub bits: u8,
    /// Metres from the eye, along the view axis (not radial), of the
    /// nearest and farthest drawn surface.
    pub near: f64,
    pub far: f64,
}

// ------------------------------------------ hew.view.line_drawing / hew.print.pdf

/// What `hew.view.line_drawing` returns (docs/design/printing.md §9b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineDrawingFormat {
    /// A standalone SVG document at true size (`scale`), mm units.
    Svg,
    /// Raw segments in view-plane metres.
    Segments,
}

/// Typed, validated parameters for `hew.view.line_drawing`: the same
/// camera/view/scene vocabulary as `hew.view.snapshot`, plus the drawing
/// options. `crates/api` parses and validates; a host renders through its
/// hidden-line engine (`hew-cli` uses `crates/hlr`).
#[derive(Debug, Clone, PartialEq)]
pub struct LineDrawingParams {
    pub camera: Option<SnapshotCamera>,
    pub view: Option<StandardView>,
    pub scene: Option<u64>,
    /// Also return hidden pieces (kind `hidden`) — dashed hidden lines.
    pub include_hidden: bool,
    /// Also return non-silhouette curved-wall seams (kind `soft`).
    pub include_soft: bool,
    pub format: LineDrawingFormat,
    /// Drawing scale for `Svg` (paper/model; 1.0 = full size).
    pub scale: f64,
    /// When given (SVG only), the file is written here instead of inline.
    pub path: Option<String>,
    /// Draw only these (`hew.view.line_drawing`'s `selection`), resolved
    /// by the command; `None` draws the whole visible document.
    pub selection: Option<Vec<crate::locate::RenderRoot>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineDrawingResult {
    /// The SVG document, for `LineDrawingFormat::Svg`.
    pub svg: Option<String>,
    /// `[ax, ay, bx, by]` in view-plane metres (y up, origin at the camera
    /// target), for `LineDrawingFormat::Segments`.
    pub segments: Vec<[f64; 4]>,
    /// One per segment: "hard" | "silhouette" | "soft" | "section" | "hidden".
    pub kinds: Vec<&'static str>,
    /// One per segment: the public id of the entity the line belongs to.
    pub ids: Vec<String>,
    /// `[min_x, min_y, max_x, max_y]`, or None when empty.
    pub bounds: Option<[f64; 4]>,
    pub count: usize,
}

/// Typed, validated parameters for `hew.print.pdf` (docs/design/printing.md
/// §9b): the page setup and drawing scale the app's Print dialog offers,
/// for a headless host.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintPdfParams {
    /// Portrait paper size, mm.
    pub paper_w_mm: f64,
    pub paper_h_mm: f64,
    /// `None` = auto (fewer tiles / follows the aspect).
    pub landscape: Option<bool>,
    pub margin_mm: f64,
    /// Scaled (parallel projection at `ratio`) vs Standard (one page).
    pub scaled: bool,
    /// paper / model, e.g. 0.1 for 1:10.
    pub ratio: f64,
    /// Scale text for the title block ("1:10", "1 in = 1 ft").
    pub scale_label: String,
    pub camera: Option<SnapshotCamera>,
    pub view: Option<StandardView>,
    pub scene: Option<u64>,
    /// Vector line art (hidden lines removed) vs a shaded raster.
    pub line_art: bool,
    pub include_hidden: bool,
    pub title_block: bool,
    pub scale_bar: bool,
    pub marks: bool,
    pub overlap_mm: f64,
    /// Scale bar / caption unit family.
    pub metric: bool,
    /// Title-block document name; defaults to the host's document name.
    pub title: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrintPdfResult {
    pub pdf: Vec<u8>,
    pub pages: usize,
    pub cols: usize,
    pub rows: usize,
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

// ------------------------------------------------------------ hew.library

/// One entry of `hew.library.list`'s listing (docs/agents/HEW_API.md §8.1;
/// docs/design/v1.1-cycle.md's Lane B) — already fully resolved by the
/// host: parsed `hew.library` metadata, the category (explicit or derived),
/// and the manifest summary. `crates/api` never touches the filesystem or
/// the `crates/library` crate that backs this (that crate does real I/O —
/// see its module doc — which is exactly what this trait exists to keep
/// out of this crate); a host that offers a library (`hew-cli`'s
/// `CliHost`, the Tauri shell) builds these by wrapping `crates/library`'s
/// own `list()`/`ListedItem`.
#[derive(Debug, Clone)]
pub struct LibraryItemEntry {
    /// Relative to the library folder, forward-slash, including the
    /// category subfolder when there is one.
    pub path: String,
    /// The item's stable library identity, if its metadata carries one.
    pub id: Option<String>,
    pub name: String,
    /// `"component"` | `"material"` | `"model"`.
    pub category: String,
    pub keywords: Vec<String>,
    pub collection: Option<String>,
    /// ISO-8601 save timestamp, if the metadata carries one.
    pub saved_at: Option<String>,
    pub size: u64,
    pub mtime_ms: u64,
    /// The manifest-only summary, or `None` for a file that failed to
    /// parse (see `error`) — never dropped from the listing either way.
    pub summary: Option<kernel::ItemSummary>,
    pub error: Option<String>,
}

/// [`Host::library_list`]'s whole answer: the resolved folder's display
/// path (`hew.library.list`'s `folder` result field) alongside every item
/// in it.
#[derive(Debug, Clone)]
pub struct LibraryListing {
    pub folder: String,
    pub items: Vec<LibraryItemEntry>,
}

/// What [`Host::library_read`] hands back: an item's bytes, the resolved
/// path they were read from, and a content-fingerprint the host computed
/// (`crates/library::sha256_hex`) — `crates/api` has no hashing dependency
/// of its own (this crate stays std + kernel + serde; see its module doc),
/// so a host with one supplies the hash rather than the caller deriving it.
#[derive(Debug, Clone)]
pub struct LibraryReadResult {
    pub path: String,
    pub bytes: Vec<u8>,
    pub content_hash: String,
}

/// What [`Host::library_write`] writes to: a brand-new item (the host
/// mints the on-disk file name from `category`/`name`/`id`, mirroring
/// `crates/library::item_file_name`), or an overwrite of an already-known
/// path (`hew.library.update_meta`'s re-save — the file name a `save`
/// minted does not change just because the item's display name later
/// does).
#[derive(Debug, Clone)]
pub enum LibraryWriteTarget<'a> {
    New {
        category: &'a str,
        name: &'a str,
        id: &'a str,
    },
    Existing {
        path: &'a str,
    },
}

/// What [`Host::library_write`] hands back: the relative path it wrote
/// (identical to the `Existing` target's path, or the freshly-minted one
/// for `New`), and the content hash of the exact bytes it just wrote — a
/// host with a hashing dependency (`crates/library::sha256_hex`) computes
/// this once here rather than `crates/api` needing one of its own (see
/// this module's doc); `hew.library.save` uses it to stamp
/// `LibraryProvenance` on the saved selection's source nodes.
#[derive(Debug, Clone)]
pub struct LibraryWriteResult {
    pub path: String,
    pub content_hash: String,
}

/// Resolve a client's document reference against `documents` (the shape
/// [`Host::documents`] returns): an exact `id` (`doc_3`), else an exact
/// `path`, else an exact `label`, else an exact `title`. The last two
/// must be unambiguous — two windows both titled "Untitled" refuse
/// `ambiguous_document` listing the candidates' ids rather than guess,
/// since a guess is exactly how an edit lands in the wrong window.
/// Nothing matching refuses `unknown_document`, with the open documents
/// as detail so the client can correct itself in one round trip.
pub fn resolve_document<'a>(
    documents: &'a [serde_json::Value],
    query: &str,
) -> Result<&'a serde_json::Value, Refusal> {
    let field = |d: &serde_json::Value, key: &str| -> Option<String> {
        d.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    if let Some(d) = documents
        .iter()
        .find(|d| field(d, "id").as_deref() == Some(query))
    {
        return Ok(d);
    }
    for key in ["path", "label", "title"] {
        let hits: Vec<&serde_json::Value> = documents
            .iter()
            .filter(|d| field(d, key).as_deref() == Some(query))
            .collect();
        match hits.as_slice() {
            [] => {}
            [one] => return Ok(one),
            many => {
                let ids: Vec<String> = many.iter().filter_map(|d| field(d, "id")).collect();
                return Err(Refusal::api(
                    "ambiguous_document",
                    &format!(
                        "\"{query}\" matches {} documents by {key} ({}); pass one of their ids instead.",
                        many.len(),
                        ids.join(", ")
                    ),
                )
                .with_detail(serde_json::json!({ "matches": ids, "documents": documents })));
            }
        }
    }
    Err(Refusal::api(
        "unknown_document",
        &format!("No open document is identified by \"{query}\" (match is by id, path, label, or title); hew.meta.documents lists them."),
    )
    .with_detail(serde_json::json!({ "documents": documents })))
}

/// What `hew.doc.import` hands a host beyond the path.
///
/// `bytes` and `images` exist for hosts with no filesystem of their own
/// (the desktop app's WASM boundary): the client reads the file — and the
/// texture images a COLLADA file refers to — and sends them along, so the
/// host imports from memory. A host with a filesystem uses `bytes` when
/// given and otherwise reads `path` (and looks for textures beside it).
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Meters-per-source-unit hint name (`m`/`mm`/`cm`/`in`); required by
    /// formats that carry no units (STL).
    pub units: Option<String>,
    /// The file's bytes, if the client supplied them.
    pub bytes: Option<Vec<u8>>,
    /// COLLADA texture images by the uri the file uses.
    pub images: std::collections::BTreeMap<String, ImportImage>,
    /// Where a picture (`.png`/`.jpg`/`.jpeg`) goes: its lower-left
    /// corner, world meters (default the origin).
    pub origin: Option<[f64; 3]>,
    /// A picture's width direction (default +X).
    pub u: Option<[f64; 3]>,
    /// Toward a picture's top (default +Y; need only not be parallel to
    /// `u`).
    pub v: Option<[f64; 3]>,
    /// A picture's width in meters (default 1); its height follows from
    /// its proportions.
    pub width: Option<f64>,
    /// Whether a host that records its own imports (the live app, for
    /// replay) records this one. `true` by default; an import into a scratch
    /// document (`hew.component.import_definition`) passes `false`, because
    /// replaying it as a world merge would be wrong: the dispatcher records
    /// the whole command frame instead.
    pub record: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            units: None,
            bytes: None,
            images: std::collections::BTreeMap::new(),
            origin: None,
            u: None,
            v: None,
            width: None,
            record: true,
        }
    }
}

/// One texture image of an [`ImportOptions`].
#[derive(Debug, Clone)]
pub struct ImportImage {
    pub bytes: Vec<u8>,
    /// `png` or `jpeg`; `None` means png.
    pub format: Option<String>,
}

/// The display length formats (`app/src/settings/units.ts`'s
/// `LengthFormat`): metric `m`, `cm`, `mm`; imperial `arch` (feet and
/// inches), `frac_in`, `dec_in`. `hew.view.units` sets the app's, and
/// `hew.doc.save`/`hew.doc.export` write a SketchUp file in one.
pub const LENGTH_FORMATS: &[&str] = &["m", "cm", "mm", "arch", "frac_in", "dec_in"];

/// The file format of a document (`hew.doc.save`'s `format`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
pub enum DocFormat {
    /// The native `.hew` format.
    #[default]
    #[serde(rename = "hew")]
    Hew,
    /// A SketchUp file in the 2026 container, what current SketchUp saves.
    #[serde(rename = "skp")]
    Skp,
    /// A SketchUp file in the 2017 format, which opens in SketchUp Make
    /// 2017 and every later release.
    #[serde(rename = "skp2017")]
    Skp2017,
}

impl DocFormat {
    /// The wire name: `"hew"`, `"skp"` or `"skp2017"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DocFormat::Hew => "hew",
            DocFormat::Skp => "skp",
            DocFormat::Skp2017 => "skp2017",
        }
    }

    /// Whether this is one of the SketchUp containers.
    pub fn is_skp(self) -> bool {
        !matches!(self, DocFormat::Hew)
    }
}

/// What [`Host::save_document`] did.
#[derive(Debug, Clone, Default)]
pub struct SaveOutcome {
    /// The file's bytes, when the host did not write it itself.
    pub bytes: Option<Vec<u8>>,
    /// The format the host resolved and saved in, when it reports one.
    pub format: Option<DocFormat>,
    /// What a SketchUp file could not hold exactly (empty for a `.hew`).
    pub simplified: Vec<String>,
    /// What the 2017 writer dropped (empty otherwise).
    pub writer_notes: Vec<String>,
}

impl SaveOutcome {
    /// The host wrote the file itself.
    pub fn written() -> SaveOutcome {
        SaveOutcome::default()
    }

    /// The host has no filesystem and hands the bytes back.
    pub fn with_bytes(bytes: Vec<u8>) -> SaveOutcome {
        SaveOutcome {
            bytes: Some(bytes),
            ..SaveOutcome::default()
        }
    }
}

// ------------------------------------------------- selection and view state

/// One entry of the app's selection (docs/agents/HEW_API.md §7, "Selection
/// and view state"): a tree node or sketch, or exactly one guide or one
/// annotation. Equality is kind plus id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SelectionEntry {
    Object(kernel::ObjectId),
    Group(kernel::GroupId),
    Instance(kernel::InstanceId),
    Sketch(kernel::SketchId),
    Guide(kernel::GuideId),
    Annotation(kernel::AnnotationId),
}

impl SelectionEntry {
    /// The document entity this entry names.
    pub fn entity(self) -> kernel::EntityRef {
        match self {
            SelectionEntry::Object(id) => kernel::EntityRef::Object(id),
            SelectionEntry::Group(id) => kernel::EntityRef::Group(id),
            SelectionEntry::Instance(id) => kernel::EntityRef::Instance(id),
            SelectionEntry::Sketch(id) => kernel::EntityRef::Sketch(id),
            SelectionEntry::Guide(id) => kernel::EntityRef::Guide(id),
            SelectionEntry::Annotation(id) => kernel::EntityRef::Annotation(id),
        }
    }

    /// The entry naming `entity`, for the six selectable kinds.
    pub fn from_entity(entity: &kernel::EntityRef) -> Option<SelectionEntry> {
        Some(match entity {
            kernel::EntityRef::Object(id) => SelectionEntry::Object(*id),
            kernel::EntityRef::Group(id) => SelectionEntry::Group(*id),
            kernel::EntityRef::Instance(id) => SelectionEntry::Instance(*id),
            kernel::EntityRef::Sketch(id) => SelectionEntry::Sketch(*id),
            kernel::EntityRef::Guide(id) => SelectionEntry::Guide(*id),
            kernel::EntityRef::Annotation(id) => SelectionEntry::Annotation(*id),
            kernel::EntityRef::Component(_)
            | kernel::EntityRef::Material(_)
            | kernel::EntityRef::Tag(_) => return None,
        })
    }
}

/// The app's selection, ordered: index 0 is the primary pick.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionState {
    pub entries: Vec<SelectionEntry>,
}

/// How a pick combines with the current selection — the app's
/// `SelectMode` (`app/src/panels/treeModel.ts`); [`crate::selection::merge`]
/// is the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Subtract,
    Toggle,
}

/// One isolation target: a node, or one definition member shown through
/// one placing instance (the app's `IsolationTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationEntry {
    Node(kernel::NodeId),
    Member {
        object: kernel::ObjectId,
        instance: kernel::InstanceId,
    },
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

    /// Save the document (`hew.doc.save`). `path` is required on the first
    /// save of a fresh document; `None` re-saves in place. `format` is the
    /// caller's explicit choice, `None` when the caller left it to the
    /// path's extension and the working file's own format — the host
    /// resolves it (a `.skp` path saves a SketchUp file, anything else a
    /// `.hew`) and refuses one that contradicts the path rather than write
    /// one format's bytes to the other's file.
    ///
    /// The outcome's `bytes` carry the file (base64-encoded by the
    /// command) unless the host wrote it directly — the shape
    /// [`Host::export_document`] already had, for the same reason. A host
    /// with no filesystem of its own (the live WASM boundary) can still
    /// serve a save by handing the bytes back for whoever asked to write,
    /// rather than refusing an operation it can perform all but the last
    /// step of.
    ///
    /// `units` is one of [`LENGTH_FORMATS`], asked for by name: a SketchUp
    /// file opens in it, a save back over a `.skp` included. `None` leaves
    /// the choice to the host, as the app's File > Save does: a save back
    /// keeps the file's own units, a new file takes the display units. A
    /// `.hew` has no display units and ignores it.
    fn save_document(
        &mut self,
        doc: &kernel::Document,
        path: Option<&str>,
        format: Option<DocFormat>,
        units: Option<&str>,
    ) -> Result<SaveOutcome, Refusal> {
        let _ = (doc, path, format, units);
        Err(unsupported("save files"))
    }

    /// Save a COPY of the document to `path` (`hew.doc.save {copy: true}`):
    /// the file is written, but the document is neither bound to it nor
    /// marked saved — the working document's file association and unsaved
    /// mark stay exactly as they were. The safe way to snapshot someone
    /// else's open document to disk.
    ///
    /// `format` and `units` resolve as [`Host::save_document`]'s do: a
    /// `.skp` path writes a SketchUp file, anything else a `.hew`, and a
    /// copy never moves the working document's SketchUp source.
    fn save_copy(
        &mut self,
        doc: &kernel::Document,
        path: &str,
        format: Option<DocFormat>,
        units: Option<&str>,
    ) -> Result<SaveOutcome, Refusal> {
        let _ = (doc, path, format, units);
        Err(unsupported("save a copy of a document"))
    }

    /// Export the document (`hew.doc.export`): returns the exported bytes
    /// (base64-encoded by the command) unless `path` was given and the
    /// host wrote it directly. `segments_per_turn` is the STL curve
    /// re-facet resolution (0 = stored facets, tessellate::export_triangles's
    /// own convention); hosts exporting formats with no curve resolution
    /// concept ignore it. `units` (one of [`LENGTH_FORMATS`], SketchUp
    /// formats only) is the display units a `.skp` opens in; `None` means
    /// the host's display units.
    #[allow(clippy::too_many_arguments)]
    fn export_document(
        &mut self,
        doc: &kernel::Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
        units: Option<&str>,
        selection: Option<&[crate::locate::RenderRoot]>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let _ = (doc, format, path, segments_per_turn, units, selection);
        Err(unsupported("export"))
    }

    /// Read a texture image file for `hew.material.create {texture: {path}}`.
    /// A host with no filesystem of its own (the live WASM boundary) never
    /// sees a `path` from `hew-cli`, which reads the file and sends
    /// `bytes_base64` instead (docs/agents/HEW_API.md §12.2).
    fn read_texture(&mut self, path: &str) -> Result<Vec<u8>, Refusal> {
        let _ = path;
        Err(unsupported("read texture files"))
    }

    /// Merge a foreign-format file into the document (`hew.doc.import`),
    /// returning the kernel's import report as JSON.
    fn import_document(
        &mut self,
        doc: &mut kernel::Document,
        path: &str,
        options: &ImportOptions,
    ) -> Result<serde_json::Value, Refusal> {
        let _ = (doc, path, options);
        Err(unsupported("import"))
    }

    /// Render the document to PNG (`hew.view.snapshot`): every host that owns
    /// a document renders through the same software rasterizer
    /// (`crates/headless-render`, over `crates/softrender`), so `hew-cli`
    /// and the app return the same bytes. `crates/api` has already validated
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
    /// (`hew.view.snapshot`'s `path` parameter, docs/agents/HEW_API.md §7):
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

    /// Hidden-line drawing (`hew.view.line_drawing`, docs/design/printing.md
    /// §9b): the visible line work from a camera, as SVG or segments. A
    /// headless host renders through `crates/hlr`; refuses `nothing_to_render`
    /// on an empty document and `too_complex` past the engine's budget.
    fn line_drawing(
        &mut self,
        doc: &kernel::Document,
        params: &LineDrawingParams,
    ) -> Result<LineDrawingResult, Refusal> {
        let _ = (doc, params);
        Err(unsupported("render line drawings"))
    }

    /// Print pages to a PDF (`hew.print.pdf`): the app's page layout
    /// (`crate::print_layout`) with vector line art or a shaded raster per
    /// page, written by `pdfwrite`. Bytes come back; `path` goes through
    /// [`Host::write_snapshot`] like every other file this trait writes.
    fn print_pdf(
        &mut self,
        doc: &kernel::Document,
        params: &PrintPdfParams,
    ) -> Result<PrintPdfResult, Refusal> {
        let _ = (doc, params);
        Err(unsupported("print to PDF"))
    }

    /// The host's open documents (`hew.meta.documents` and the `hello`
    /// response). A headless host has exactly its working document.
    ///
    /// Each entry is an object `{id, title, path, dirty, document_id}`
    /// (docs/agents/HEW_API.md §4.2): `id` is the host-lifetime-unique
    /// `doc_N` a client passes to `hew.doc.attach`/`close`/`focus`, `path`
    /// is the file the document was last opened from or saved to (`null`
    /// while unsaved), `dirty` is whether it has edits since its last save,
    /// and `document_id` is the document's persistent UUID
    /// (`kernel::Document::document_id`, `null` until the first save mints
    /// one). A multi-window host adds `label`, `front`, and `attached`.
    fn documents(&self, doc: &kernel::Document) -> Vec<serde_json::Value> {
        let _ = doc;
        Vec::new()
    }

    /// Close a document (`hew.doc.close`). `document` is a resolved
    /// document id, or `None` for the connection's own attached document;
    /// `discard` closes it even with unsaved changes (without it, a dirty
    /// document refuses `unsaved_changes`). Only a multi-document host
    /// with windows has anything to close; a headless host refuses
    /// `host_capability_missing`.
    fn close_document(&mut self, document: Option<&str>, discard: bool) -> Result<(), Refusal> {
        let _ = (document, discard);
        Err(unsupported("close documents"))
    }

    /// Raise a document's window to the front (`hew.doc.focus`) — the
    /// opt-in counterpart of every other command's promise not to steal
    /// the user's focus. `None` is the connection's own attached document.
    fn focus_document(&mut self, document: Option<&str>) -> Result<(), Refusal> {
        let _ = document;
        Err(unsupported("focus a document window"))
    }

    /// Milliseconds since the Unix epoch — `hew.library.save`'s `savedAt`
    /// and the entropy of a fresh item id. A host supplies its own clock
    /// because `std::time::SystemTime::now()` PANICS on
    /// `wasm32-unknown-unknown`: the live app's WASM kernel has to read the
    /// browser's. The default is the native clock.
    fn unix_time_ms(&self) -> u64 {
        std::time::SystemTime::now() // wasm-guard: native default; LiveHost overrides
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// 64 bits that differ call to call (a fresh library item id's raw
    /// material). Native default: the OS-seeded `RandomState` mixed with
    /// the clock, pid, and a counter. A WASM host overrides it
    /// (`std::process::id()` is unavailable there).
    fn random_u64(&self) -> u64 {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut h = RandomState::new().build_hasher();
        h.write_u64(self.unix_time_ms());
        h.write_u32(std::process::id()); // wasm-guard: native default; LiveHost overrides
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.finish()
    }

    /// 16 random bytes for minting a document id
    /// (`kernel::Document::ensure_document_id`), which `hew.doc.save` does
    /// before its first write. Default: [`Host::random_u64`] twice. A host
    /// with a real entropy source overrides it (`hew-cli` reads the OS's,
    /// the live WASM host `crypto.getRandomValues`).
    fn random_16(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.random_u64().to_le_bytes());
        out[8..].copy_from_slice(&self.random_u64().to_le_bytes());
        out
    }

    /// The path the working document was last opened from or saved to, if
    /// any — `hew.library.save`'s default for `source_doc` (docs/agents/HEW_API.md
    /// §8.1) when the caller doesn't supply one, mirroring the UI's own
    /// `documentName(docSessionRef.current)` at the Save-to-Library flow.
    /// `hew-cli`'s `CliHost` answers this from the same `working_path` its
    /// `hew.doc.save`/`open` already track; a host with no notion of a
    /// "current file" (a live connection identifies its document some
    /// other way, through its own window/session, not through this trait)
    /// answers `None`, which `hew.library.save` treats as "omit
    /// `sourceDoc`" rather than a refusal — a missing source-document name
    /// is display-only bookkeeping, never a hard requirement.
    fn working_document_path(&self) -> Option<&str> {
        None
    }

    /// Set the live viewport's camera (`hew.view.camera`, docs/agents/HEW_API.md
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
    /// (`hew.view.zoom_extents`, docs/agents/HEW_API.md §7) — the API
    /// counterpart of the app's View > Zoom Extents. Also a view
    /// effect, not a document edit: same `mutates_document = false`
    /// posture as [`Host::set_camera`]. A headless host refuses
    /// `host_capability_missing` — there is no viewport to frame.
    fn zoom_extents(&mut self) -> Result<(), Refusal> {
        Err(unsupported("frame the viewport to fit the model"))
    }

    /// Set the app's displayed length-unit format (`hew.view.units`,
    /// docs/agents/HEW_API.md §7). `format` is one of
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

    /// Every library item, fully resolved (`hew.library.list`,
    /// `hew.library.describe`, and the id/path resolution `insert`/
    /// `remove`/`update_meta` need — `crates/api`'s command handlers find
    /// the entry matching a client's `item` id-or-path in this listing
    /// rather than asking the host to resolve it separately, so this is
    /// the ONE library read every one of those commands starts from). A
    /// host with no library folder configured, or a `--live` desktop
    /// backend with no filesystem, refuses `host_capability_missing`.
    fn library_list(&self) -> Result<LibraryListing, Refusal> {
        Err(unsupported("list the library"))
    }

    /// An item's raw bytes plus a content hash, by its RESOLVED relative
    /// path (`hew.library.describe`'s bytes, `hew.library.insert {item}`'s
    /// source, `hew.library.update_meta`'s read-modify-write) — `path` is
    /// always one [`Host::library_list`] already reported, never a
    /// client-supplied string reaching the filesystem unvalidated.
    fn library_read(&self, path: &str) -> Result<LibraryReadResult, Refusal> {
        let _ = path;
        Err(unsupported("read library items"))
    }

    /// Writes an item's bytes to `target` (`hew.library.save`'s fresh
    /// item, `hew.library.update_meta`'s in-place re-save) — a
    /// caller-minted [`LibraryWriteTarget::New`] name can collide in
    /// principle (two saves in the same instant with the same slug+id,
    /// astronomically unlikely with a fresh random id per save) but never
    /// silently overwrites a DIFFERENT item: the path is derived from the
    /// id this very save minted.
    fn library_write(
        &mut self,
        target: LibraryWriteTarget,
        bytes: &[u8],
    ) -> Result<LibraryWriteResult, Refusal> {
        let _ = (target, bytes);
        Err(unsupported("write library items"))
    }

    /// Deletes an item at a resolved relative path (`hew.library.remove`).
    /// Deleting an already-absent item is not an error — the caller's goal
    /// ("this item is gone") is already satisfied.
    fn library_remove(&mut self, path: &str) -> Result<(), Refusal> {
        let _ = path;
        Err(unsupported("remove library items"))
    }

    /// The app's selection (`hew.selection.get`). Only a host with a
    /// selection to mirror has one; a headless host refuses.
    fn selection(&self) -> Result<SelectionState, Refusal> {
        Err(unsupported("read the selection"))
    }

    /// Combines `entries` with the selection by `mode`
    /// ([`crate::selection::merge`]) and answers the result
    /// (`hew.selection.set`/`clear`). `crates/api` has already validated
    /// every entry: it resolves, the set is one kind family, and nothing in
    /// it is hidden.
    fn set_selection(
        &mut self,
        entries: Vec<SelectionEntry>,
        mode: SelectMode,
    ) -> Result<SelectionState, Refusal> {
        let _ = (entries, mode);
        Err(unsupported("change the selection"))
    }

    /// Notifies the host that the kernel's user-hidden flags changed
    /// (`hew.view.hide`/`show`/`show_all`): the document already holds the
    /// change, so this is a signal for a host that mirrors hidden state,
    /// never a veto. A silent success by default, like
    /// [`Host::scene_applied`].
    fn hidden_changed(&mut self) -> Result<(), Refusal> {
        Ok(())
    }

    /// The live viewport's isolation, `None` when nothing is isolated
    /// (`hew.view.hidden`). Isolation is viewport-only view state.
    fn isolation(&self) -> Result<Option<Vec<IsolationEntry>>, Refusal> {
        Err(unsupported("read the isolation"))
    }

    /// Isolates `entries` in the live viewport and frames them
    /// (`hew.view.isolate`), replacing any isolation already on.
    fn isolate(&mut self, entries: Vec<IsolationEntry>) -> Result<(), Refusal> {
        let _ = entries;
        Err(unsupported("isolate"))
    }

    /// Ends the live viewport's isolation (`hew.view.clear_isolation`); a
    /// no-op when nothing is isolated.
    fn clear_isolation(&mut self) -> Result<(), Refusal> {
        Err(unsupported("clear isolation"))
    }

    /// The app's in-context entries sitting logically inside the innermost
    /// open session frame, outermost first: the editing model the app falls
    /// back on for an instance a session cannot take (mirrored, unevenly
    /// scaled, or placed in a group elsewhere). Empty headless.
    fn fallback_context(&self) -> Vec<ContextEntry> {
        Vec::new()
    }

    /// Whether a tool gesture is armed in the user's window (a Move drag in
    /// flight). `hew.view.enter`/`exit`/`exit_all` refuse
    /// `gesture_in_progress` while it is, because the app's own entry
    /// silently does nothing then.
    fn gesture_armed(&self) -> bool {
        false
    }

    /// `hew.view.enter`: converges the user's window onto `leaf` reached
    /// through `path` (instances only, outermost first), the way a double
    /// click on the node in the Outliner would. The host queues the effect;
    /// the outcome is reported by `context_changed`.
    fn enter_user_context(
        &mut self,
        path: &[kernel::InstanceId],
        leaf: kernel::NodeId,
    ) -> Result<(), Refusal> {
        let _ = (path, leaf);
        Err(unsupported("enter the user's context"))
    }

    /// `hew.view.exit`: pops the user's innermost entry, a fallback entry
    /// or a session frame.
    fn exit_user_context(&mut self) -> Result<(), Refusal> {
        Err(unsupported("leave the user's context"))
    }

    /// `hew.view.exit_all`: back to the top level in the user's window.
    fn exit_all_user_contexts(&mut self) -> Result<(), Refusal> {
        Err(unsupported("leave the user's context"))
    }

    /// Frames the world box `min`..`max` in the live viewport
    /// (`hew.view.zoom_to`). The box already carries `padding`, which is
    /// passed along for information only.
    fn zoom_to(
        &mut self,
        min: kernel::Point3,
        max: kernel::Point3,
        padding: f64,
    ) -> Result<(), Refusal> {
        let _ = (min, max, padding);
        Err(unsupported("frame the viewport"))
    }
}

/// What an in-context entry holds (`Host::fallback_context`): the kinds the
/// app's fallback can hold. A group always opens a session, so it never
/// appears here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextKind {
    Object,
    Instance,
}

/// One in-context entry the app keeps beside the kernel's session stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextEntry {
    pub kind: ContextKind,
    pub node: kernel::NodeId,
}

/// The capability-free host: pure in-process dispatch with no file
/// system and no renderer. Every host effect refuses typed.
#[derive(Debug, Default)]
pub struct NoHost;

impl Host for NoHost {}
