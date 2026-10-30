//! Host-effect document lifecycle: hew.doc.new/open/save/export/import, hew.view.snapshot (docs/agents/HEW_API.md §7). Implemented by its wave; see
//! docs/design/api-implementation-conventions.md.
//!
//! Every command here is `Served::Host` (registry.rs): this module owns no
//! effect of its own — it parses params, calls the matching `ctx.host`
//! method, and shapes the result. A host lacking the capability answers
//! `host_capability_missing` through the normal `CmdError::Refusal` path
//! (host.rs's `unsupported()`); nothing here special-cases that — it just
//! propagates like any other refusal.

use super::camera::{RawCamera, parse_camera_or_view};
use super::{CmdError, Ctx, Handler};
use crate::Refusal;
use crate::host::{DocFormat, Host, ImportImage, ImportOptions, SaveOutcome, SnapshotParams};
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.doc.new" => Some(new_doc),
        "hew.doc.open" => Some(open_doc),
        "hew.doc.close" => Some(close_doc),
        "hew.doc.focus" => Some(focus_doc),
        "hew.doc.save" => Some(save_doc),
        "hew.doc.revert" => Some(revert_doc),
        "hew.doc.export" => Some(export_doc),
        "hew.doc.import" => Some(import_doc),
        "hew.view.snapshot" => Some(snapshot),
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

// -------------------------------------------------------------- hew.doc.new

/// `focus` and `label` steer a multi-window host's new window (the desktop
/// app handles them before the command reaches the dispatcher); a
/// single-document host has no window to place and ignores them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // validated here, acted on by a multi-window host
struct NewParams {
    #[serde(default)]
    focus: Option<WindowFocus>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WindowFocus {
    Background,
    Front,
}

fn new_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NewParams { .. } = parse(params)?;
    replace_document(ctx, |host, doc| host.new_document(doc))?;
    Ok(document_result(ctx))
}

/// Runs a host's document replacement (`new`, `open`, `revert`) and starts
/// the new document's epoch above the old one's, whatever the host, so a
/// reader keyed on `epoch` never sees it go back (docs/agents/HEW_API.md
/// §7).
fn replace_document(
    ctx: &mut Ctx,
    replace: impl FnOnce(&mut dyn Host, &mut kernel::Document) -> Result<(), Refusal>,
) -> Result<(), CmdError> {
    let epoch = ctx.doc.document_epoch();
    replace(ctx.host, ctx.doc).map_err(CmdError::Refusal)?;
    ctx.doc.raise_document_epoch_to(epoch + 1);
    // One `reset` record: everything since any generation is `full`.
    ctx.doc.record_reset();
    Ok(())
}

/// `{document: <the host's only/working document>}` — what a
/// single-document host answers `new`/`open` with; the multi-window
/// desktop host answers these itself with the new window's entry.
fn document_result(ctx: &Ctx) -> Value {
    match ctx.host.documents(ctx.doc).into_iter().next() {
        Some(document) => serde_json::json!({ "document": document }),
        None => serde_json::json!({}),
    }
}

// ------------------------------------------------------------- hew.doc.open

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // `focus`/`label`: validated here, acted on by a multi-window host
struct OpenParams {
    path: String,
    #[serde(default)]
    focus: Option<WindowFocus>,
    #[serde(default)]
    label: Option<String>,
}

fn open_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let OpenParams { path, .. } = parse(params)?;
    replace_document(ctx, |host, doc| host.open_document(doc, &path))?;
    Ok(document_result(ctx))
}

// ------------------------------------------------------ hew.doc.close / focus

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseParams {
    #[serde(default)]
    document: Option<String>,
    #[serde(default)]
    discard: bool,
}

fn close_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let CloseParams { document, discard } = parse(params)?;
    ctx.host
        .close_document(document.as_deref(), discard)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FocusParams {
    #[serde(default)]
    document: Option<String>,
}

fn focus_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let FocusParams { document } = parse(params)?;
    ctx.host
        .focus_document(document.as_deref())
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ------------------------------------------------------------- hew.doc.save

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    #[serde(default)]
    path: Option<String>,
    /// Write a copy: the document is not rebound to `path` and keeps its
    /// unsaved mark. Requires `path`.
    #[serde(default)]
    copy: bool,
    /// Serialize and hand the bytes back — nothing is written anywhere, the
    /// document is not bound to a file and is not marked saved. Works the
    /// same on every host; it is how a client reads a document it does not
    /// own (hew-cli's live render fallback). Always `.hew` bytes.
    #[serde(default)]
    return_bytes: bool,
    /// `"hew"`, `"skp"` or `"skp2017"`; absent means the path's extension
    /// (or, with no path, the working file's own format) decides.
    #[serde(default)]
    format: Option<DocFormat>,
    /// The display units a SketchUp file opens in, by name; absent, the
    /// host decides as the app's File > Save does.
    #[serde(default)]
    units: Option<String>,
}

/// `app/src/settings/units.ts`'s `LengthFormat` union
/// ([`crate::LENGTH_FORMATS`]): an unrecognized value is refused
/// statically (`-32602`) rather than reach the host at all.
pub(crate) fn check_length_format(format: &str) -> Result<(), CmdError> {
    if crate::LENGTH_FORMATS.contains(&format) {
        Ok(())
    } else {
        Err(CmdError::Params(format!(
            "unknown display unit format \"{format}\" (expected one of {:?})",
            crate::LENGTH_FORMATS
        )))
    }
}

/// A save's `format` and, for a SketchUp file, what it could not hold.
fn outcome_fields(outcome: &SaveOutcome) -> Value {
    let mut out = serde_json::json!({});
    if let Some(f) = outcome.format {
        out["format"] = Value::String(f.as_str().to_string());
        if f.is_skp() {
            out["simplified"] = serde_json::json!(outcome.simplified);
            out["writer_notes"] = serde_json::json!(outcome.writer_notes);
        }
    }
    out
}

fn save_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let SaveParams {
        path,
        copy,
        return_bytes,
        format,
        units,
    } = parse(params)?;
    if let Some(u) = &units {
        check_length_format(u)?;
    }
    if return_bytes
        && (path.is_some() || copy || format.is_some_and(DocFormat::is_skp) || units.is_some())
    {
        return Err(CmdError::Params(
            "`return_bytes` hands back .hew bytes and takes no `path`, `copy`, SketchUp `format` or `units`".into(),
        ));
    }
    // Every save mints the document id first, if the document has none
    // (docs/dev/HEW_FILE_FORMAT.md §2 `document_id`): a `.hew` write
    // carries it, and a SketchUp write, which has no slot for it, leaves
    // it on the document so the next `.hew` save keeps the same id. A
    // copy carries it too. An id never changes once minted, and a save the
    // host refuses or fails keeps no freshly minted one (an errored
    // envelope leaves the document untouched).
    let random = ctx.host.random_16();
    if return_bytes {
        ctx.doc.ensure_document_id(random);
        return Ok(serde_json::json!({
            "bytes_base64": encode_base64(&ctx.doc.save_for_persistence()),
            "format": "hew",
        }));
    }
    if copy {
        let Some(path) = path else {
            return Err(CmdError::Params("`copy` needs a `path`".into()));
        };
        let host = &mut *ctx.host;
        let outcome = ctx
            .doc
            .with_document_id_for_save(random, |doc| {
                host.save_copy(doc, &path, format, units.as_deref())
            })
            .map_err(CmdError::Refusal)?;
        return Ok(outcome_fields(&outcome));
    }
    let host = &mut *ctx.host;
    let outcome = ctx
        .doc
        .with_document_id_for_save(random, |doc| {
            host.save_document(doc, path.as_deref(), format, units.as_deref())
        })
        .map_err(CmdError::Refusal)?;
    let mut out = outcome_fields(&outcome);
    // A host that wrote the file itself (`CliHost`) returns no bytes — the
    // save is complete the instant `save_document` returns `Ok`, so this
    // is the right place to mark the document clean (Lane C's saved
    // mark, docs/agents/HEW_API.md §7). A host with no filesystem of its
    // own (`LiveHost`) hands the bytes back for the caller to write,
    // exactly as `hew.doc.export` already does — the write hasn't
    // happened yet at this point, so marking clean here would be a lie;
    // the live app calls `Scene::mark_saved` itself once ITS write
    // (Tauri fs, or a browser download the user confirmed) actually
    // lands.
    match outcome.bytes {
        Some(bytes) => out["bytes_base64"] = Value::String(encode_base64(&bytes)),
        None => ctx.doc.mark_saved(),
    }
    Ok(out)
}

// ---------------------------------------------------------- hew.doc.revert

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // the desktop app routes by it before the command gets here
struct RevertParams {
    #[serde(default)]
    document: Option<String>,
}

/// File > Revert to Saved: reload the working document from its file. The
/// confirmation the app asks of a person is not repeated here; the caller
/// asked for it by name. A fresh load starts with empty undo and redo
/// stacks and reads clean.
fn revert_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let RevertParams { .. } = parse(params)?;
    let nothing = |why: &str| {
        CmdError::Refusal(Refusal::api(
            "nothing_to_revert",
            &format!("There is nothing to revert: {why}."),
        ))
    };
    let Some(path) = ctx.host.working_document_path().map(str::to_string) else {
        return Err(nothing("the document has no saved file"));
    };
    if ctx.doc.at_saved_mark() {
        return Err(nothing("the document has no changes since the last save"));
    }
    replace_document(ctx, |host, doc| host.open_document(doc, &path))?;
    Ok(serde_json::json!({}))
}

// ----------------------------------------------------------- hew.doc.export

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    format: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    segments_per_turn: Option<u32>,
    /// The display units a SketchUp file opens in; absent, the host's.
    #[serde(default)]
    units: Option<String>,
    /// Export only these placements (or definitions at identity).
    #[serde(default)]
    selection: Option<Vec<Value>>,
}

fn export_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ExportParams {
        format,
        path,
        segments_per_turn,
        units,
        selection,
    } = parse(params)?;
    if let Some(u) = &units {
        check_length_format(u)?;
        if !matches!(format.as_str(), "skp" | "skp2017") {
            return Err(CmdError::Params(format!(
                "units applies to SketchUp files (\"skp\", \"skp2017\"); \"{format}\" has none to set"
            )));
        }
    }
    validate_selection(&selection)?;
    let selection = match &selection {
        Some(list) => {
            if matches!(format.as_str(), "skp" | "skp2017") {
                return Err(CmdError::Params(
                    "selection applies to the mesh formats (stl, 3mf, glb, gltf, usdz); a SketchUp export writes the whole document".into(),
                ));
            }
            Some(crate::locate::resolve_render_roots(ctx, list)?)
        }
        None => None,
    };
    let bytes = ctx
        .host
        .export_document(
            ctx.doc,
            &format,
            path.as_deref(),
            segments_per_turn.unwrap_or(0),
            units.as_deref(),
            selection.as_deref(),
        )
        .map_err(CmdError::Refusal)?;
    Ok(match bytes {
        Some(bytes) => serde_json::json!({
            "bytes_base64": encode_base64(&bytes),
            "format": format,
        }),
        None => serde_json::json!({ "format": format }),
    })
}

// ----------------------------------------------------------- hew.doc.import

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    path: String,
    /// Meters-per-source-unit hint. Formats that carry no units of their
    /// own (STL) refuse typed without it (docs/agents/HEW_API.md §7's semantics
    /// note); formats that do (glTF, COLLADA, `.skp`) ignore it.
    #[serde(default)]
    units: Option<String>,
    /// The file's bytes, for a host with no filesystem of its own (the
    /// desktop app); `path` then only names the file — its extension picks
    /// the format and its stem names STL objects. `hew-cli` fills this in
    /// for you. A host with a filesystem ignores it only if absent: given,
    /// it is used instead of reading `path`.
    #[serde(default)]
    bytes_base64: Option<String>,
    /// COLLADA texture images the file refers to, by the uri it uses.
    #[serde(default)]
    images: Option<std::collections::BTreeMap<String, ImportImageParam>>,
    /// A picture's (`.png`/`.jpg`/`.jpeg`) lower-left corner, its width
    /// direction, a direction toward its top, and its width in meters
    /// (defaults: the origin, +X, +Y, 1 m). Other formats ignore them.
    #[serde(default)]
    origin: Option<[f64; 3]>,
    #[serde(default)]
    u: Option<[f64; 3]>,
    #[serde(default)]
    v: Option<[f64; 3]>,
    #[serde(default)]
    width: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportImageParam {
    bytes_base64: String,
    #[serde(default)]
    format: Option<String>,
}

fn import_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ImportParams {
        path,
        units,
        bytes_base64,
        images,
        origin,
        u,
        v,
        width,
    } = parse(params)?;
    let mut options = ImportOptions {
        units,
        origin,
        u,
        v,
        width,
        ..ImportOptions::default()
    };
    if let Some(b64) = bytes_base64 {
        options.bytes = Some(
            decode_base64(&b64)
                .ok_or_else(|| CmdError::Params("bytes_base64 is not valid base64".into()))?,
        );
    }
    for (uri, image) in images.unwrap_or_default() {
        let bytes = decode_base64(&image.bytes_base64).ok_or_else(|| {
            CmdError::Params(format!("images[{uri}].bytes_base64 is not valid base64"))
        })?;
        options.images.insert(
            uri,
            ImportImage {
                bytes,
                format: image.format,
            },
        );
    }
    let report = ctx
        .host
        .import_document(ctx.doc, &path, &options)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({ "report": report }))
}

// --------------------------------------------------------- hew.view.snapshot

/// The wire shape of `hew.view.snapshot`'s params, before the validation
/// this handler applies (mutual exclusion, size clamping, view-name
/// resolution) turns it into a typed [`SnapshotParams`]. `camera`/`view`
/// are the shared vocabulary `camera.rs` parses — the same one
/// `hew.view.camera` (view.rs) accepts, so there is exactly one camera
/// spec in the protocol (docs/agents/HEW_API.md §7). `scene` is a Scene's public
/// id (docs/agents/HEW_API.md's Scenes section) — mutually exclusive with both.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotParams {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    scene: Option<String>,
    #[serde(default)]
    include_ids: Option<bool>,
    #[serde(default)]
    include_depth: Option<bool>,
    #[serde(default)]
    depth_bits: Option<u32>,
    #[serde(default)]
    include_normals: Option<bool>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    selection: Option<Vec<Value>>,
    #[serde(default)]
    background: Option<String>,
}

/// `hew.view.snapshot`'s default and clamped size bounds
/// (docs/design/headless-snapshot.md): a request outside `[MIN, MAX]` is
/// clamped rather than refused — a client asking for an absurd size still
/// gets a usable image, not a protocol error.
const SNAPSHOT_DEFAULT_SIZE: u32 = 512;
const SNAPSHOT_MIN_SIZE: u32 = 16;
const SNAPSHOT_MAX_SIZE: u32 = 2048;

/// The fill of a snapshot's untouched pixels when `background` is absent
/// (the software rasterizer's own backdrop).
pub const DEFAULT_SNAPSHOT_BACKGROUND: [u8; 4] = [245, 246, 248, 255];

/// The `selection` rule shared by `hew.doc.export`, `hew.view.snapshot`,
/// and `hew.view.line_drawing`: when present, it names at least one
/// entity. (What each entry resolves to is
/// [`crate::locate::resolve_render_roots`].)
pub(crate) fn validate_selection(selection: &Option<Vec<Value>>) -> Result<(), CmdError> {
    match selection {
        Some(list) if list.is_empty() => Err(CmdError::Params(
            "selection must name at least one entity".to_string(),
        )),
        _ => Ok(()),
    }
}

/// `background`: `"transparent"`, `#rrggbb`, or `#rrggbbaa` (hex digits in
/// either case). Anything else, `#rgb` and color names included, is
/// refused.
pub(crate) fn parse_background(text: &str) -> Result<[u8; 4], CmdError> {
    let bad = || {
        CmdError::Params(
            "background is \"transparent\" or a #rrggbb or #rrggbbaa color".to_string(),
        )
    };
    if text == "transparent" {
        return Ok([0, 0, 0, 0]);
    }
    let hex = text.strip_prefix('#').ok_or_else(bad)?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
    Ok([
        byte(0)?,
        byte(2)?,
        byte(4)?,
        if hex.len() == 8 { byte(6)? } else { 255 },
    ])
}

fn snapshot(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let raw: RawSnapshotParams = parse(params)?;
    if raw.scene.is_some() && (raw.camera.is_some() || raw.view.is_some()) {
        return Err(CmdError::Params(
            "scene is mutually exclusive with camera and view".to_string(),
        ));
    }
    if raw.selection.is_some() && raw.scene.is_some() {
        return Err(CmdError::Params(
            "selection is mutually exclusive with scene".to_string(),
        ));
    }
    validate_selection(&raw.selection)?;
    let background = match raw.background.as_deref() {
        None => DEFAULT_SNAPSHOT_BACKGROUND,
        Some(text) => parse_background(text)?,
    };
    let (camera, view) = parse_camera_or_view(raw.camera, raw.view)?;
    let scene = raw
        .scene
        .as_deref()
        .map(super::scenes::resolve_scene_id)
        .transpose()?;
    let selection = raw
        .selection
        .as_deref()
        .map(|list| crate::locate::resolve_render_roots(ctx, list))
        .transpose()?;
    let width = raw
        .width
        .unwrap_or(SNAPSHOT_DEFAULT_SIZE)
        .clamp(SNAPSHOT_MIN_SIZE, SNAPSHOT_MAX_SIZE);
    let height = raw
        .height
        .unwrap_or(SNAPSHOT_DEFAULT_SIZE)
        .clamp(SNAPSHOT_MIN_SIZE, SNAPSHOT_MAX_SIZE);

    let include_depth = raw.include_depth.unwrap_or(false);
    let depth_bits = match raw.depth_bits {
        None => 8,
        Some(_) if !include_depth => {
            return Err(CmdError::Params(
                "depth_bits only applies with include_depth".to_string(),
            ));
        }
        Some(bits @ (8 | 16)) => bits as u8,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "depth_bits is 8 or 16, not {other}"
            )));
        }
    };

    let snapshot_params = SnapshotParams {
        width,
        height,
        camera,
        view,
        scene,
        include_ids: raw.include_ids.unwrap_or(false),
        include_depth,
        depth_bits,
        include_normals: raw.include_normals.unwrap_or(false),
        path: raw.path,
        selection,
        background,
    };
    let result = ctx
        .host
        .snapshot(ctx.doc, &snapshot_params)
        .map_err(CmdError::Refusal)?;

    // Mirrors `hew.doc.export`'s posture (docs/agents/HEW_API.md §7): bytes
    // base64 by default; a `path` is honored by hosts with filesystem
    // access and refused typed elsewhere (`write_snapshot`'s own default).
    // The inline PNG (and, worse, the id-buffer) can exceed an MCP
    // client's tool-result budget at any useful resolution — `path` is
    // how a caller avoids that entirely.
    match &snapshot_params.path {
        Some(path) => {
            ctx.host
                .write_snapshot(path, &result.png)
                .map_err(CmdError::Refusal)?;
            let mut out = serde_json::json!({
                "path": path,
                "width": result.width,
                "height": result.height,
            });
            if let Some(id_buffer) = &result.id_buffer {
                let id_buffer_path = format!("{path}.ids.bin");
                ctx.host
                    .write_snapshot(&id_buffer_path, id_buffer)
                    .map_err(CmdError::Refusal)?;
                out["id_buffer_path"] = Value::String(id_buffer_path);
                out["id_palette"] = serde_json::json!(result.id_palette);
            }
            if let Some(depth) = &result.depth {
                let depth_path = format!("{path}.depth.png");
                ctx.host
                    .write_snapshot(&depth_path, &depth.png)
                    .map_err(CmdError::Refusal)?;
                out["depth_path"] = Value::String(depth_path);
                add_depth_range(&mut out, depth);
            }
            if let Some(normals) = &result.normals_png {
                let normals_path = format!("{path}.normals.png");
                ctx.host
                    .write_snapshot(&normals_path, normals)
                    .map_err(CmdError::Refusal)?;
                out["normals_path"] = Value::String(normals_path);
            }
            Ok(out)
        }
        None => {
            let mut out = serde_json::json!({
                "png_base64": encode_base64(&result.png),
                "width": result.width,
                "height": result.height,
            });
            if let Some(id_buffer) = &result.id_buffer {
                out["id_buffer_base64"] = Value::String(encode_base64(id_buffer));
                out["id_palette"] = serde_json::json!(result.id_palette);
            }
            if let Some(depth) = &result.depth {
                out["depth_png_base64"] = Value::String(encode_base64(&depth.png));
                add_depth_range(&mut out, depth);
            }
            if let Some(normals) = &result.normals_png {
                out["normals_png_base64"] = Value::String(encode_base64(normals));
            }
            Ok(out)
        }
    }
}

/// The metric range and depth of a depth pass, the same fields whether the
/// image is inline or in a file.
fn add_depth_range(out: &mut Value, depth: &crate::host::DepthPass) {
    out["depth_near"] = serde_json::json!(depth.near);
    out["depth_far"] = serde_json::json!(depth.far);
    out["depth_bits"] = serde_json::json!(depth.bits);
}

// ------------------------------------------------------------------- base64

/// RFC 4648 §4 standard-alphabet base64, WITH padding. No new dependency —
/// the whole point of `hew.doc.export`'s inline-bytes path and
/// `hew.view.snapshot`'s PNG result is a few dozen lines nobody needs a
/// crate for.
pub fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        let n =
            (u32::from(b0) << 16) | (u32::from(b1.unwrap_or(0)) << 8) | u32::from(b2.unwrap_or(0));
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if b1.is_some() {
            ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if b2.is_some() {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The inverse of [`encode_base64`] — `hew.library.insert`'s `bytes_base64`
/// param is the only caller today. `None` on any character outside the
/// alphabet, a bad length, or misplaced padding: a malformed payload is
/// refused typed (`CmdError::Params`), never guessed at.
pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s = s.trim().as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && chunk[3] != b'=') {
            return None;
        }
        let mut acc: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = if c == b'=' {
                if i < 4 - pad {
                    return None;
                }
                0
            } else {
                ALPHABET.iter().position(|&a| a == c)? as u32
            };
            acc = (acc << 6) | v;
        }
        let bytes = acc.to_be_bytes();
        out.push(bytes[1]);
        if pad < 2 {
            out.push(bytes[2]);
        }
        if pad < 1 {
            out.push(bytes[3]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_parses_transparent_and_hex_in_either_case() {
        assert_eq!(parse_background("transparent").unwrap(), [0, 0, 0, 0]);
        assert_eq!(
            parse_background("#1A2B3C").unwrap(),
            [0x1a, 0x2b, 0x3c, 255]
        );
        assert_eq!(
            parse_background("#1a2b3c80").unwrap(),
            [0x1a, 0x2b, 0x3c, 0x80]
        );
        for bad in [
            "", "#", "#fff", "#12345", "#1234567", "#gg0000", "red", "1a2b3c",
        ] {
            assert!(parse_background(bad).is_err(), "{bad:?}");
        }
    }

    /// RFC 4648 §10's own test vectors.
    #[test]
    fn base64_matches_rfc4648_test_vectors() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in cases {
            assert_eq!(encode_base64(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn decode_base64_inverts_encode_base64_at_every_padding_length() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(decode_base64(encoded).as_deref(), Some(*plain), "{encoded}");
        }
    }

    #[test]
    fn decode_base64_refuses_malformed_input() {
        for bad in ["Zg=", "Zm9vY", "Zg===", "Z!==", "=Zm8", "Zm=8"] {
            assert!(decode_base64(bad).is_none(), "should refuse {bad:?}");
        }
    }
}
