//! `CliHost`: the real, file-I/O-backed implementation of `api::Host`
//! (docs/agents/HEW_API.md §12) — headless `hew-cli` embeds `crates/api` and the
//! kernel directly, so this is the only place in the binary that touches a
//! filesystem or an importer crate. Kernel-class purity (DEVELOPMENT.md
//! rule 1) stops at `crates/api`'s door; this crate is the host on the
//! other side of it.

use api::{
    DocFormat, Host, LibraryItemEntry, LibraryListing, LibraryReadResult, LibraryWriteResult,
    LibraryWriteTarget, LineDrawingParams, LineDrawingResult, PrintPdfParams, PrintPdfResult,
    Refusal, SaveOutcome, SnapshotParams, SnapshotResult,
};
use kernel::Document;
use std::path::{Path, PathBuf};

/// Headless host: real file I/O, plus `hew.view.snapshot` rendered through
/// `crates/softrender` — the pure software rasterizer
/// (docs/design/headless-snapshot.md). No GPU, no viewport: bytes in
/// (tessellated meshes), bytes out (a PNG).
#[derive(Debug, Default)]
pub struct CliHost {
    /// The path the working document was last opened from or saved to —
    /// `hew.doc.save` with no `path` re-saves here (§7's "path required on
    /// first save" semantics).
    working_path: Option<PathBuf>,
    /// The format the working document is in: what a bare `hew.doc.save`
    /// writes back, and which `.skp` container a `.skp` path takes when no
    /// `format` is given.
    working_format: DocFormat,
    /// The `.skp` the working document was opened from or last saved as,
    /// which the next `.skp` save builds on so unchanged content stays as
    /// SketchUp wrote it. `None` for a document that did not come from a
    /// `.skp` (a `.hew`, or a new one).
    skp_source: Option<skp_export::SkpSource>,
    /// The working document came from, or was last saved as, revision 17 of
    /// the 2021-and-later container: a save with no `format` keeps to it.
    working_compat: bool,
}

impl CliHost {
    pub fn new() -> CliHost {
        CliHost::default()
    }

    /// A host that already knows which file its document came from — the
    /// live client's render fallback, which renders a copy of the app's
    /// document and wants the title block to name the file the app does.
    pub fn with_working_path(path: Option<PathBuf>) -> CliHost {
        CliHost {
            working_path: path,
            ..CliHost::default()
        }
    }

    /// The path a prior open/save recorded, if any. Exposed for the CLI's
    /// `run --out` step, which wants to know whether a bare `save` (no
    /// path) is even possible before trying.
    pub fn working_path(&self) -> Option<&Path> {
        self.working_path.as_deref()
    }
}

impl Host for CliHost {
    fn new_document(&mut self, doc: &mut Document) -> Result<(), Refusal> {
        *doc = Document::new();
        self.working_path = None;
        self.working_format = DocFormat::Hew;
        self.skp_source = None;
        self.working_compat = false;
        Ok(())
    }

    fn open_document(&mut self, doc: &mut Document, path: &str) -> Result<(), Refusal> {
        let bytes = std::fs::read(path)
            .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
        if has_skp_extension(Path::new(path)) {
            let (loaded, format, source) = open_skp(&bytes, path)?;
            *doc = loaded;
            self.working_format = format;
            self.working_compat =
                skp_export::target_of(&bytes) == skp_export::Target::Skp2026Compat;
            self.skp_source = Some(source);
        } else {
            let loaded = Document::load(&bytes)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            *doc = loaded;
            self.working_format = DocFormat::Hew;
            self.skp_source = None;
            self.working_compat = false;
        }
        self.working_path = Some(PathBuf::from(path));
        Ok(())
    }

    fn save_document(
        &mut self,
        doc: &Document,
        path: Option<&str>,
        format: Option<DocFormat>,
        units: Option<&str>,
        compat: bool,
    ) -> Result<SaveOutcome, Refusal> {
        let target = match path {
            Some(p) => PathBuf::from(p),
            None => self.working_path.clone().ok_or_else(|| {
                Refusal::api(
                    "path_required",
                    "This document has never been saved. Pass a path.",
                )
            })?,
        };
        let explicit_compat = compat;
        let compat = compat || (format.is_none() && self.working_compat);
        let format = resolve_save_format(&target, format, self.working_format)?;
        refuse_compat_off_container(explicit_compat, format)?;
        let mut outcome = SaveOutcome {
            format: Some(format),
            ..SaveOutcome::default()
        };
        // Write first and record the new state only once the bytes landed,
        // so a failed write leaves the working path, format and source as
        // they were.
        let new_source = match format {
            DocFormat::Hew => {
                write_file(&target, &doc.save_for_persistence())?;
                None
            }
            DocFormat::Skp | DocFormat::SkpStream(_) => {
                // Asked for by name, the units go over the file's own too;
                // otherwise a save back keeps them and a new file takes
                // this host's display units, as the app's File > Save does.
                let units = match units {
                    Some(u) => skp_export::SaveUnits::Set(display_units(Some(u))),
                    None => skp_export::SaveUnits::New(display_units(None)),
                };
                let saved = skp_export::save_in(
                    doc,
                    skp_target(format, compat),
                    skp_export::WriteEnv::system(),
                    self.skp_source.as_ref(),
                    units,
                )
                .map_err(|e| Refusal::api("save_failed", &format!("{}: {e}", target.display())))?;
                write_file(&target, &saved.export.bytes)?;
                outcome.simplified = saved.export.simplified;
                outcome.writer_notes = saved.export.writer_notes;
                Some(saved.source)
            }
        };
        self.skp_source = new_source;
        self.working_compat = compat && format == DocFormat::Skp;
        self.working_format = format;
        self.working_path = Some(target);
        // This host has a filesystem and used it, so there are no bytes to
        // hand back — the file IS the result.
        Ok(outcome)
    }

    fn save_copy(
        &mut self,
        doc: &Document,
        path: &str,
        format: Option<DocFormat>,
        units: Option<&str>,
        compat: bool,
    ) -> Result<SaveOutcome, Refusal> {
        let target = Path::new(path);
        let explicit_compat = compat;
        let compat = compat || (format.is_none() && self.working_compat);
        let format = resolve_save_format(target, format, self.working_format)?;
        refuse_compat_off_container(explicit_compat, format)?;
        let mut outcome = SaveOutcome {
            format: Some(format),
            ..SaveOutcome::default()
        };
        match format {
            DocFormat::Hew => write_file(target, &doc.save_for_persistence())?,
            DocFormat::Skp | DocFormat::SkpStream(_) => {
                // Built on the working document's source, as a save is, so
                // unchanged content keeps SketchUp's records; a copy never
                // becomes the source itself.
                let units = match units {
                    Some(u) => skp_export::SaveUnits::Set(display_units(Some(u))),
                    None => skp_export::SaveUnits::New(display_units(None)),
                };
                let saved = skp_export::save_in(
                    doc,
                    skp_target(format, compat),
                    skp_export::WriteEnv::system(),
                    self.skp_source.as_ref(),
                    units,
                )
                .map_err(|e| Refusal::api("save_failed", &format!("{path}: {e}")))?;
                write_file(target, &saved.export.bytes)?;
                outcome.simplified = saved.export.simplified;
                outcome.writer_notes = saved.export.writer_notes;
            }
        }
        Ok(outcome)
    }

    fn export_document(
        &mut self,
        doc: &Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
        units: Option<&str>,
        selection: Option<&[api::locate::RenderRoot]>,
        compat: bool,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        // `true`: a headless dispatch has no dialog of its own to warn about
        // non-solid objects first, so a leaky object is dropped rather than
        // silently handed back in a file no human reviewed
        // (crates/mesh-export's module doc, "Non-solid inclusion").
        let bytes = match format {
            _ if DocFormat::skp_from_name(format).is_some() => {
                let target =
                    skp_target(DocFormat::skp_from_name(format).unwrap_or_default(), compat);
                skp_export::export_in(
                    doc,
                    target,
                    skp_export::WriteEnv::system(),
                    Some(display_units(units)),
                )
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
        match path {
            Some(p) => {
                std::fs::write(p, &bytes)
                    .map_err(|e| Refusal::api("save_failed", &format!("{p}: {e}")))?;
                Ok(None)
            }
            None => Ok(Some(bytes)),
        }
    }

    fn import_document(
        &mut self,
        doc: &mut Document,
        path: &str,
        options: &api::ImportOptions,
    ) -> Result<serde_json::Value, Refusal> {
        import_by_extension(doc, path, options)
    }

    fn read_texture(&mut self, path: &str) -> Result<Vec<u8>, Refusal> {
        read_texture_file(path)
    }

    fn snapshot(
        &mut self,
        doc: &Document,
        params: &SnapshotParams,
    ) -> Result<SnapshotResult, Refusal> {
        headless_render::snapshot(doc, params)
    }

    fn write_snapshot(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        std::fs::write(path, bytes)
            .map_err(|e| Refusal::api("save_failed", &format!("{path}: {e}")))
    }

    fn line_drawing(
        &mut self,
        doc: &Document,
        params: &LineDrawingParams,
    ) -> Result<LineDrawingResult, Refusal> {
        headless_render::line_drawing(doc, params)
    }

    fn print_pdf(
        &mut self,
        doc: &Document,
        params: &PrintPdfParams,
    ) -> Result<PrintPdfResult, Refusal> {
        let name = self
            .working_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        crate::print::print_pdf(doc, params, &name)
    }

    fn library_list(&self) -> Result<LibraryListing, Refusal> {
        let dir = library::LibraryDir::resolve(None);
        Ok(LibraryListing {
            folder: dir.root().to_string_lossy().into_owned(),
            items: library::list(&dir)
                .into_iter()
                .map(listed_to_entry)
                .collect(),
        })
    }

    fn library_read(&self, path: &str) -> Result<LibraryReadResult, Refusal> {
        let dir = library::LibraryDir::resolve(None);
        let bytes = library::read(&dir, path)
            .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
        let content_hash = library::sha256_hex(&bytes);
        Ok(LibraryReadResult {
            path: path.to_string(),
            bytes,
            content_hash,
        })
    }

    fn library_write(
        &mut self,
        target: LibraryWriteTarget,
        bytes: &[u8],
    ) -> Result<LibraryWriteResult, Refusal> {
        let dir = library::LibraryDir::resolve(None);
        let path = match target {
            LibraryWriteTarget::New { category, name, id } => {
                let category = library::Category::parse(category).ok_or_else(|| {
                    Refusal::api(
                        "bad_library_category",
                        &format!("unknown library category {category:?}"),
                    )
                })?;
                library::item_file_name(name, id, category)
            }
            LibraryWriteTarget::Existing { path } => path.to_string(),
        };
        library::write(&dir, &path, bytes)
            .map_err(|e| Refusal::api("save_failed", &format!("{path}: {e}")))?;
        Ok(LibraryWriteResult {
            content_hash: library::sha256_hex(bytes),
            path,
        })
    }

    fn library_remove(&mut self, path: &str) -> Result<(), Refusal> {
        let dir = library::LibraryDir::resolve(None);
        library::remove(&dir, path)
            .map_err(|e| Refusal::api("save_failed", &format!("{path}: {e}")))
    }

    fn working_document_path(&self) -> Option<&str> {
        self.working_path.as_deref().and_then(Path::to_str)
    }

    /// A headless host has exactly its working document, always `doc_1`
    /// (docs/agents/HEW_API.md §4.2).
    fn documents(&self, doc: &Document) -> Vec<serde_json::Value> {
        let path = self.working_path.as_deref();
        let title = path
            .and_then(Path::file_stem)
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("Untitled");
        vec![serde_json::json!({
            "id": "doc_1",
            "title": title,
            "path": path.and_then(Path::to_str),
            "dirty": !doc.at_saved_mark(),
            "document_id": doc.document_id(),
        })]
    }

    fn random_16(&self) -> [u8; 16] {
        random_16()
    }
}

/// 16 bytes from the operating system's random source, for minting a
/// document id on save. Falls back to the trait's default mix (clock,
/// pid, a counter, a seeded hasher) in the unlikely case the OS source
/// fails, rather than failing the save.
fn random_16() -> [u8; 16] {
    let mut out = [0u8; 16];
    if getrandom::fill(&mut out).is_err() {
        struct Fallback;
        impl Host for Fallback {}
        out = Fallback.random_16();
    }
    out
}

/// Converts a `crates/library` listing row into the plain shape
/// `crates/api`'s `Host` trait speaks — `crates/api` has no dependency on
/// `crates/library` (that crate does real filesystem I/O; `crates/api`
/// stays free of it), so this host is where the two meet. An item whose
/// manifest failed to parse gets no derived category (there is nothing to
/// derive it FROM) — `"model"` is the same harmless default the web app's
/// `erroredItem` uses for the identical case.
fn listed_to_entry(item: library::ListedItem) -> LibraryItemEntry {
    LibraryItemEntry {
        path: item.rel_path,
        id: item.meta.id,
        name: item.display_name,
        category: item
            .category
            .map(|c| c.as_str().to_string())
            .unwrap_or_else(|| "model".to_string()),
        keywords: item.meta.keywords,
        collection: item.meta.collection,
        saved_at: item.meta.saved_at,
        size: item.size,
        mtime_ms: item.mtime_ms,
        summary: item.summary,
        error: item.error,
    }
}

/// Maps a [`mesh_export::ExportError`] to an [`api::Refusal`] through the
/// error's own name/message accessors, so the two can never drift apart —
/// `mesh-export` has no dependency on `crates/api` (DEVELOPMENT.md rule
/// 1), so this boundary-crossing step lives here instead.
fn export_refusal(e: mesh_export::ExportError) -> Refusal {
    Refusal::api(e.name(), &e.message())
}

// -------------------------------------------------------- import unit scale

/// meters-per-source-unit, keyed by the `units` param (docs/agents/HEW_API.md §7:
/// STL carries no units of its own). The STL/3MF/glTF/USDZ *writers*
/// `export_document` calls above live in `crates/mesh-export` now, shared
/// with `crates/wasm-api`'s `LiveHost`; this scale table is import-only
/// and stays here.
fn unit_scale(units: &str) -> Option<f64> {
    match units {
        "m" => Some(1.0),
        "mm" => Some(0.001),
        "cm" => Some(0.01),
        "in" => Some(0.0254),
        _ => None,
    }
}

// ------------------------------------------------------------------ texture

/// Reads a texture image for `hew.material.create {texture: {path}}`: the
/// headless host's read, and what `hew-cli` sends a live app as
/// `bytes_base64` (the app has no filesystem). Whether the bytes are an image
/// is the command's check, the same for both.
pub(crate) fn read_texture_file(path: &str) -> Result<Vec<u8>, Refusal> {
    // Held in the document and, live, sent whole over a socket as base64.
    const MAX_TEXTURE_BYTES: u64 = 64 * 1024 * 1024;
    read_input_file(
        path,
        MAX_TEXTURE_BYTES,
        "a texture image may be; scale the image down",
    )
}

/// Reads a file a command names, as `load_failed` on any failure: only a
/// regular file (a FIFO would block the open and a device never ends), and
/// at most `cap` bytes however the file reports its size. `limit` finishes
/// the too-large message: "{n} MiB is more than the {cap} MiB {limit}".
fn read_input_file(path: &str, cap: u64, limit: &str) -> Result<Vec<u8>, Refusal> {
    use std::io::Read;
    let failed =
        |why: &dyn std::fmt::Display| Refusal::api("load_failed", &format!("{path}: {why}"));
    let meta = std::fs::metadata(path).map_err(|e| failed(&e))?;
    if !meta.is_file() {
        return Err(failed(&"not a regular file"));
    }
    let too_big = |len: u64| {
        failed(&format!(
            "{} MiB is more than the {} MiB {limit}.",
            len / (1024 * 1024),
            cap / (1024 * 1024)
        ))
    };
    if meta.len() > cap {
        return Err(too_big(meta.len()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(cap + 1).read_to_end(&mut bytes))
        .map_err(|e| failed(&e))?;
    if bytes.len() as u64 > cap {
        return Err(too_big(bytes.len() as u64));
    }
    Ok(bytes)
}

// -------------------------------------------------------------- .skp files

fn has_skp_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("skp"))
}

/// A length format by name (already validated by `crates/api`), or this
/// host's display units when none is given: meters, a new app's default
/// (`app/src/settings/units.ts`). A headless session has no display
/// preference of its own to read.
fn display_units(name: Option<&str>) -> skp_export::DisplayUnits {
    name.and_then(skp_export::DisplayUnits::from_name)
        .unwrap_or(skp_export::DisplayUnits::Meters)
}

fn skp_target(format: DocFormat, compat: bool) -> skp_export::Target {
    match format {
        DocFormat::SkpStream(n) => skpwright_release(n)
            .map(skp_export::Target::Stream)
            .unwrap_or(skp_export::Target::Skp2026),
        _ => skp_export::Target::Skp2026,
    }
    .with_compat(compat)
}

/// The Skpwright release a stream format's number names.
fn skpwright_release(number: u8) -> Option<skp_import::skpwright::Release> {
    skp_import::skpwright::Release::of_name(u32::from(number))
}

fn write_file(target: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    std::fs::write(target, bytes)
        .map_err(|e| Refusal::api("save_failed", &format!("{}: {e}", target.display())))
}

/// Opens a `.skp` as a working document: imports it, makes opening not an
/// undo step, and keeps the file as the source the next save builds on.
/// Returns the format its own container is in.
fn open_skp(
    bytes: &[u8],
    path: &str,
) -> Result<(Document, DocFormat, skp_export::SkpSource), Refusal> {
    let load_failed =
        |e: &dyn std::fmt::Display| Refusal::api("load_failed", &format!("{path}: {e}"));
    let parsed = skp_import::import(bytes).map_err(|e| load_failed(&e))?;
    let mut doc = Document::new();
    let (report, _change) = doc
        .ingest(parsed.scene, parsed.textures_missing)
        .map_err(|e| load_failed(&e))?;
    doc.forget_history().map_err(|e| load_failed(&e))?;
    let source =
        skp_export::SkpSource::after_open(bytes.to_vec(), &doc, &parsed.provenance, &report);
    let format = match skp_export::target_of(bytes) {
        skp_export::Target::Skp2026 | skp_export::Target::Skp2026Compat => DocFormat::Skp,
        skp_export::Target::Stream(r) => DocFormat::SkpStream(r.number()),
    };
    Ok((doc, format, source))
}

/// The format a save writes. An explicit `format` must agree with the
/// path: a `.skp` path takes only a SketchUp format, and a SketchUp format
/// takes only a `.skp` path, so one format's bytes never land in the
/// other's file. Without one, a `.skp` path keeps the working document's
/// own `.skp` container (the 2026 one when it did not come from a `.skp`)
/// and any other path writes `.hew`.
/// `compat` asks for revision 17 of the 2021-and-later container; asked for
/// any other resolved format (a `.hew` path, a stream release) it would be
/// silently dropped, so it is refused. An explicit `false` is the same as
/// absent, as in the app.
fn refuse_compat_off_container(compat: bool, format: DocFormat) -> Result<(), Refusal> {
    if compat && format != DocFormat::Skp {
        return Err(Refusal::api(
            "format_mismatch",
            "`compat` applies to the 2021-and-later container (format \"skp\"); this save resolves to another format.",
        ));
    }
    Ok(())
}

fn resolve_save_format(
    target: &Path,
    explicit: Option<DocFormat>,
    working: DocFormat,
) -> Result<DocFormat, Refusal> {
    let skp_path = has_skp_extension(target);
    match explicit {
        Some(f) if f.is_skp() != skp_path => Err(Refusal::api(
            "format_mismatch",
            &format!(
                "format \"{}\" contradicts the path \"{}\": a .skp path takes format \"skp\", \"skp3\" to \"skp8\" or \"skp2013\" to \"skp2020\", \
                 any other path takes \"hew\". Change the extension or drop the format.",
                f.as_str(),
                target.display()
            ),
        )),
        Some(f) => Ok(f),
        None if skp_path => Ok(if working.is_skp() {
            working
        } else {
            DocFormat::Skp
        }),
        None => Ok(DocFormat::Hew),
    }
}

// ------------------------------------------------------------------ import

/// Reads what `hew.doc.import` needs from disk, for a client whose host has
/// no filesystem of its own (the live desktop app): the file's bytes and,
/// for COLLADA, the texture images it refers to — found the way the app's
/// own import finds them (the file's folder, plus `<stem>/`,
/// `<stem>_textures/`, `textures/`, `Textures/`), keyed by bare name and by
/// folder-relative name so `wood.png` and `textures/wood.png` both resolve.
/// Best-effort for the images, like the app: an unreadable one just ends up
/// in the report's `textures_missing`.
pub(crate) fn read_import_inputs(
    path: &str,
) -> Result<
    (
        Vec<u8>,
        std::collections::BTreeMap<String, api::ImportImage>,
    ),
    Refusal,
> {
    // Sent whole over a socket as base64 (and held in the app's memory), so
    // a file this large is refused up front rather than wedging a connection.
    const MAX_IMPORT_BYTES: u64 = 128 * 1024 * 1024;
    let bytes = read_input_file(
        path,
        MAX_IMPORT_BYTES,
        "a live import carries; import it in the app (File > Import)",
    )?;
    let mut images = std::collections::BTreeMap::new();
    let is_dae = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dae"));
    if !is_dae {
        return Ok((bytes, images));
    }
    let file = Path::new(path);
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let mut add = |folder: &Path, relative_prefix: Option<&str>| {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            let format = if lower.ends_with(".png") {
                "png"
            } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                "jpeg"
            } else {
                continue;
            };
            let Ok(image) = std::fs::read(entry.path()) else {
                continue;
            };
            let image = api::ImportImage {
                bytes: image,
                format: Some(format.to_string()),
            };
            images.insert(name.clone(), image.clone());
            if let Some(prefix) = relative_prefix {
                images.insert(format!("{prefix}/{name}"), image);
            }
        }
    };
    add(dir, None);
    for sub in [
        stem.clone(),
        format!("{stem}_textures"),
        "textures".to_string(),
        "Textures".to_string(),
    ] {
        add(&dir.join(&sub), Some(&sub));
    }
    Ok((bytes, images))
}

/// Ingests a picture's recipe, checked first ([`image_import::check`]), so
/// a recipe the kernel would not build is refused before anything changes:
/// the document, its undo and redo history and its palette stay as they
/// were. Past the check, a face the kernel skipped would be a kernel bug;
/// it is still refused, and the refusal says the document may hold part of
/// the import. The live host in `crates/wasm-api` does the same.
fn ingest_picture(
    doc: &mut Document,
    path: &str,
    scene: kernel::ImportScene,
) -> Result<kernel::ImportReport, Refusal> {
    image_import::check(&scene)
        .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
    let (report, _change) = doc
        .ingest(scene, Vec::new())
        .map_err(|e| api::Refusal::from_document_error(&e))?;
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

/// Merges a file into `doc` by extension, honest about the one thing the
/// path alone cannot say: STL carries no units and refuses without them
/// (`units_required`). The bytes (and COLLADA images) come from `options`
/// when the client supplied them, else from `path` and the files beside it.
///
/// `crates/wasm-api`'s live host has the same logic over the same
/// importers (it cannot share this: that crate has no filesystem and this
/// one is not a dependency of it); the two must stay in step.
fn import_by_extension(
    doc: &mut Document,
    path: &str,
    options: &api::ImportOptions,
) -> Result<serde_json::Value, Refusal> {
    let (bytes, images) = match &options.bytes {
        Some(bytes) => (bytes.clone(), options.images.clone()),
        None => read_import_inputs(path)?,
    };
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    if let Some("png" | "jpg" | "jpeg") = ext.as_deref() {
        // A picture: placed the way the app's Place Image places one
        // (`image_import`), at the request's frame and width.
        let placement = image_import::Placement::from_parts(
            options.origin,
            options.u,
            options.v,
            options.width,
        );
        let out = image_import::import(&bytes, None, &image_import::picture_name(path), &placement)
            .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
        let (width, height) = (out.width, out.height);
        let report = ingest_picture(doc, path, out.scene)?;
        let mut json = report_to_json(&report, &[]);
        json["picture"] = serde_json::json!({ "width": width, "height": height });
        return Ok(json);
    }

    let (scene, textures_missing, warnings) = match ext.as_deref() {
        Some("stl") => {
            let units = options.units.as_deref();
            let scale = match units.map(unit_scale) {
                Some(Some(scale)) => scale,
                Some(None) => {
                    return Err(Refusal::api(
                        "units_required",
                        &format!(
                            "Unknown units \"{}\". Use one of m, mm, cm, in.",
                            units.unwrap_or("")
                        ),
                    ));
                }
                None => {
                    return Err(Refusal::api(
                        "units_required",
                        "STL carries no units of its own; pass units: \"m\" | \"mm\" | \"cm\" | \"in\".",
                    ));
                }
            };
            let name_hint = Path::new(path).file_stem().and_then(|s| s.to_str());
            let parsed = stl_import::import(&bytes, scale, name_hint)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.missing, parsed.warnings)
        }
        Some("glb") | Some("gltf") => {
            let parsed = gltf_import::import(&bytes)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.missing, parsed.warnings)
        }
        Some("dae") => {
            let mut map = dae_import::ImageMap::new();
            for (uri, image) in &images {
                let format = match image.format.as_deref() {
                    Some("jpeg") | Some("jpg") => kernel::ImageFormat::Jpeg,
                    _ => kernel::ImageFormat::Png,
                };
                map.insert(uri.clone(), (image.bytes.clone(), format));
            }
            let parsed = dae_import::import(&bytes, &map)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.textures_missing, parsed.warnings)
        }
        Some("skp") => {
            let parsed = skp_import::import(&bytes)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.textures_missing, parsed.warnings)
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
        .ingest(scene, textures_missing)
        .map_err(|e| api::Refusal::from_document_error(&e))?;
    Ok(report_to_json(&report, &warnings))
}

fn report_to_json(report: &kernel::ImportReport, warnings: &[String]) -> serde_json::Value {
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
        "warnings": warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hew-cli-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_collada_files_textures_are_found_beside_it_like_the_apps_import_does() {
        let dir = scratch("dae-textures");
        std::fs::write(dir.join("chair.dae"), b"<COLLADA/>").unwrap();
        std::fs::write(dir.join("wood.png"), b"png-bytes").unwrap();
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        std::fs::write(dir.join("textures").join("oak.JPG"), b"jpg-bytes").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let (bytes, images) = read_import_inputs(dir.join("chair.dae").to_str().unwrap()).unwrap();
        assert_eq!(bytes, b"<COLLADA/>");
        assert_eq!(images["wood.png"].format.as_deref(), Some("png"));
        assert_eq!(images["oak.JPG"].format.as_deref(), Some("jpeg"));
        // Both the bare name and the folder-relative one resolve.
        assert!(images.contains_key("textures/oak.JPG"));
        assert!(!images.contains_key("notes.txt"));
        // A non-COLLADA file never scans for textures.
        std::fs::write(dir.join("part.stl"), b"x").unwrap();
        let (_, none) = read_import_inputs(dir.join("part.stl").to_str().unwrap()).unwrap();
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_import_from_supplied_bytes_needs_no_file_and_reports_warnings() {
        let mut doc = Document::new();
        let options = api::ImportOptions {
            units: Some("m".to_string()),
            bytes: Some(tetrahedron_stl_bytes()),
            ..Default::default()
        };
        let report = import_by_extension(&mut doc, "/no/such/dir/tet.stl", &options).unwrap();
        assert!(report["objects_created"].as_u64().unwrap() >= 1, "{report}");
        assert!(
            report["warnings"].is_array(),
            "the report carries warnings: {report}"
        );
    }

    /// `hew.doc.import` of a picture places it, as Place Image does, at the
    /// given frame and width (its height from its proportions).
    #[test]
    fn importing_a_picture_places_it_at_the_given_frame() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&200u32.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut doc = Document::new();
        let options = api::ImportOptions {
            bytes: Some(png),
            origin: Some([1.0, 2.0, 0.0]),
            u: Some([0.0, 1.0, 0.0]),
            v: Some([0.0, 0.0, 1.0]),
            width: Some(4.0),
            ..Default::default()
        };
        let report = import_by_extension(&mut doc, "/pics/Front wall.PNG", &options).unwrap();
        assert_eq!(report["picture"]["width"], 4.0);
        assert_eq!(report["picture"]["height"], 2.0);
        let [inst] = doc.instance_ids()[..] else {
            panic!("one instance");
        };
        let def = doc.instance_def(inst).unwrap();
        assert_eq!(doc.component_name(def), Some("Front wall"));
        let a = doc.instance_pose(inst).unwrap().to_affine();
        assert_eq!((a[3], a[7], a[11]), (1.0, 2.0, 0.0));
        // Not a picture: the usual refusal names the picture formats.
        let err = import_by_extension(&mut doc, "/pics/a.gif", &options).unwrap_err();
        assert_eq!(err.name, "unsupported_format");
    }

    /// A picture too small for its face to build is refused, naming the
    /// minimum, and the document is left as it was.
    #[test]
    fn importing_a_picture_too_small_to_place_is_refused() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut doc = Document::new();
        let options = api::ImportOptions {
            bytes: Some(png),
            width: Some(1.1e-6),
            ..Default::default()
        };
        let err = import_by_extension(&mut doc, "/pics/speck.png", &options).unwrap_err();
        assert_eq!(err.name, "load_failed");
        assert!(err.explanation.contains("at least"), "{}", err.explanation);
        assert!(doc.instance_ids().is_empty());
        assert!(!doc.can_undo());
    }

    /// A picture the kernel would not build is refused before the document
    /// is touched: the saved bytes, the undo depth, the redo history (an
    /// earlier undone picture still redoes) and the palette all stay as
    /// they were. Reached two ways: a width so large the face's area
    /// overflows, through the ordinary import, and a recipe shrunk past
    /// what the kernel can find a plane for, through `ingest_picture`.
    #[test]
    fn a_picture_the_kernel_would_not_build_is_refused_before_anything_changes() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&100u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let at = |width: f64| api::ImportOptions {
            bytes: Some(png.clone()),
            width: Some(width),
            ..Default::default()
        };
        let mut doc = Document::new();
        import_by_extension(&mut doc, "/pics/kept.png", &at(1.0)).unwrap();
        import_by_extension(&mut doc, "/pics/undone.png", &at(1.0)).unwrap();
        doc.undo().unwrap();
        let before = doc.save();
        let (undo, redo) = (doc.undo_depth(), doc.redo_depth());
        let materials = doc.material_ids().len();
        assert_eq!(redo, 1);
        let unchanged = |doc: &Document| {
            assert_eq!(doc.save(), before, "the document is unchanged");
            assert_eq!((doc.undo_depth(), doc.redo_depth()), (undo, redo));
            assert_eq!(doc.material_ids().len(), materials, "no material left");
        };

        // A vast width is refused as too big before the kernel is asked to
        // build it (`image_import::MAX_SIDE`), with its own reason.
        let err = import_by_extension(&mut doc, "/pics/vast.png", &at(1e200)).unwrap_err();
        assert_eq!(err.name, "load_failed");
        assert!(
            err.explanation.contains("each side must be at most"),
            "{}",
            err.explanation
        );
        unchanged(&doc);

        let mut out =
            image_import::import(&png, None, "speck", &image_import::Placement::ground(1.0))
                .unwrap();
        let kernel::ImportNode::Mesh(mesh) = &mut out.scene.defs[0].children[0] else {
            panic!("the definition holds the picture's mesh");
        };
        for p in &mut mesh.positions {
            *p = kernel::Point3::new(p.x * 1e-7, p.y * 1e-7, 0.0);
        }
        let Err(err) = ingest_picture(&mut doc, "/pics/speck.png", out.scene) else {
            panic!("a face the kernel would skip is refused");
        };
        assert_eq!(err.name, "load_failed");
        assert!(
            err.explanation.contains("can't be built"),
            "{}",
            err.explanation
        );
        unchanged(&doc);

        doc.redo().unwrap();
        assert_eq!(doc.instance_ids().len(), 2, "the undone picture redoes");
    }

    /// `hew.doc.new`, `open`, and `revert` replace the document, and the
    /// epoch every query reports keeps rising across each
    /// (docs/agents/HEW_API.md §7).
    #[test]
    fn document_replacement_never_lowers_the_epoch() {
        let dir = scratch("epoch");
        let path = dir.join("doc.hew");
        let mut host = CliHost::new();
        let mut doc = Document::new();
        let registry = api::Registry::protocol_1();
        let mut conn = api::Connection::new(api::Grant::core(), "test");
        let mut call = |doc: &mut Document,
                        host: &mut CliHost,
                        id: i64,
                        method: &str,
                        params: serde_json::Value| {
            let request: api::Request = serde_json::from_value(serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            }))
            .unwrap();
            match conn.dispatch(&registry, doc, host, request) {
                api::DispatchOutcome::Reply(r) => {
                    assert!(r.error.is_none(), "{method}: {:?}", r.error);
                    r.result.unwrap()
                }
                _ => panic!("{method} replies"),
            }
        };
        let epoch = |doc: &mut Document,
                     host: &mut CliHost,
                     call: &mut dyn FnMut(
            &mut Document,
            &mut CliHost,
            i64,
            &str,
            serde_json::Value,
        ) -> serde_json::Value| {
            call(doc, host, 90, "hew.query.context", serde_json::json!({}))["epoch"]
                .as_u64()
                .unwrap()
        };
        call(
            &mut doc,
            &mut host,
            0,
            "hew.meta.hello",
            serde_json::json!({"protocol": 1}),
        );
        call(
            &mut doc,
            &mut host,
            1,
            "hew.doc.attach",
            serde_json::json!({}),
        );
        let attr =
            serde_json::json!({"target": "document", "ns": "com.example", "key": "k", "value": 1});
        call(&mut doc, &mut host, 2, "hew.attr.set", attr.clone());
        call(
            &mut doc,
            &mut host,
            3,
            "hew.doc.save",
            serde_json::json!({"path": path.to_str().unwrap()}),
        );
        let mut last = epoch(&mut doc, &mut host, &mut call);
        assert!(last > 0);
        call(&mut doc, &mut host, 4, "hew.attr.set", attr);
        last = last.max(epoch(&mut doc, &mut host, &mut call));
        call(
            &mut doc,
            &mut host,
            5,
            "hew.doc.revert",
            serde_json::json!({}),
        );
        let after_revert = epoch(&mut doc, &mut host, &mut call);
        assert!(after_revert > last, "revert: {after_revert} after {last}");
        call(
            &mut doc,
            &mut host,
            6,
            "hew.doc.open",
            serde_json::json!({"path": path.to_str().unwrap()}),
        );
        let after_open = epoch(&mut doc, &mut host, &mut call);
        assert!(
            after_open > after_revert,
            "open: {after_open} after {after_revert}"
        );
        call(&mut doc, &mut host, 7, "hew.doc.new", serde_json::json!({}));
        let after_new = epoch(&mut doc, &mut host, &mut call);
        assert!(
            after_new > after_open,
            "new: {after_new} after {after_open}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_save_mints_a_document_id_that_save_as_copies_and_documents_keep() {
        let dir = scratch("document-id");
        let (first, second, copy) = (dir.join("a.hew"), dir.join("b.hew"), dir.join("c.hew"));
        let mut host = CliHost::new();
        let mut doc = Document::new();
        let registry = api::Registry::protocol_1();
        let mut conn = api::Connection::new(api::Grant::core(), "test");
        let mut call = |doc: &mut Document, host: &mut CliHost, method: &str, params| {
            let request: api::Request = serde_json::from_value(serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
            }))
            .unwrap();
            match conn.dispatch(&registry, doc, host, request) {
                api::DispatchOutcome::Reply(r) => {
                    assert!(r.error.is_none(), "{method}: {:?}", r.error);
                    r.result.unwrap()
                }
                _ => panic!("{method} replies"),
            }
        };
        call(
            &mut doc,
            &mut host,
            "hew.meta.hello",
            serde_json::json!({"protocol": 1}),
        );
        call(&mut doc, &mut host, "hew.doc.attach", serde_json::json!({}));
        let listed = |doc: &mut Document,
                      host: &mut CliHost,
                      call: &mut dyn FnMut(
            &mut Document,
            &mut CliHost,
            &str,
            serde_json::Value,
        ) -> serde_json::Value| {
            call(doc, host, "hew.meta.documents", serde_json::json!({}))["documents"][0]
                ["document_id"]
                .clone()
        };
        assert_eq!(
            listed(&mut doc, &mut host, &mut call),
            serde_json::Value::Null
        );
        let path = |p: &PathBuf| serde_json::json!(p.to_str().unwrap());
        call(
            &mut doc,
            &mut host,
            "hew.doc.save",
            serde_json::json!({"path": path(&first)}),
        );
        let id = doc.document_id().expect("the first save mints an id");
        assert_eq!(
            listed(&mut doc, &mut host, &mut call),
            serde_json::json!(id)
        );
        call(
            &mut doc,
            &mut host,
            "hew.doc.save",
            serde_json::json!({"path": path(&second)}),
        );
        call(
            &mut doc,
            &mut host,
            "hew.doc.save",
            serde_json::json!({"path": path(&copy), "copy": true}),
        );
        for file in [&first, &second, &copy] {
            let loaded = Document::load(&std::fs::read(file).unwrap()).unwrap();
            assert_eq!(
                loaded.document_id().as_deref(),
                Some(id.as_str()),
                "{file:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn random_16_differs_call_to_call() {
        assert_ne!(random_16(), random_16());
    }

    #[test]
    fn save_copy_writes_the_file_but_leaves_the_working_document_unbound() {
        let dir = scratch("save-copy");
        let mut host = CliHost::new();
        let doc = Document::new();
        let target = dir.join("copy.hew");
        host.save_copy(&doc, target.to_str().unwrap(), None, None, false)
            .unwrap();
        assert!(target.exists());
        assert_eq!(
            host.working_path(),
            None,
            "a copy does not bind the document"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_by_extension_refuses_units_required_for_stl_without_units() {
        // A minimal binary STL: 80-byte header + 0 triangles is legal for
        // the format but `StlError::Empty`s in this importer — use a real
        // one-triangle fixture instead so the units gate is what's tested.
        let stl = tetrahedron_stl_bytes();
        let dir = std::env::temp_dir().join(format!("hew-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tet.stl");
        std::fs::write(&path, &stl).unwrap();

        let mut doc = Document::new();
        let err = import_by_extension(
            &mut doc,
            path.to_str().unwrap(),
            &api::ImportOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.name, "units_required");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A single-triangle (degenerate as a solid, but perfectly valid STL)
    /// binary fixture, built by hand at the byte level. Public so
    /// `crates/hew-cli/tests/cli.rs` can build a richer one (a tetrahedron)
    /// the same way for its own import test.
    pub(crate) fn tetrahedron_stl_bytes() -> Vec<u8> {
        let tris: &[[[f32; 3]; 3]] = &[
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        ];
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for tri in tris {
            for _ in 0..3 {
                out.extend_from_slice(&0f32.to_le_bytes());
            }
            for v in tri {
                for c in v {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }
}
