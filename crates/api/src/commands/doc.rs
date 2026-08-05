//! Host-effect document lifecycle: hew.doc.new/open/save/export/import, hew.view.snapshot (docs/HEW_API.md §7). Implemented by its wave; see
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
use crate::host::SnapshotParams;
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.doc.new" => Some(new_doc),
        "hew.doc.open" => Some(open_doc),
        "hew.doc.save" => Some(save_doc),
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewParams {}

fn new_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let NewParams {} = parse(params)?;
    ctx.host.new_document(ctx.doc).map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ------------------------------------------------------------- hew.doc.open

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenParams {
    path: String,
}

fn open_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let OpenParams { path } = parse(params)?;
    ctx.host
        .open_document(ctx.doc, &path)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ------------------------------------------------------------- hew.doc.save

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    #[serde(default)]
    path: Option<String>,
}

fn save_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let SaveParams { path } = parse(params)?;
    let written = ctx
        .host
        .save_document(ctx.doc, path.as_deref())
        .map_err(CmdError::Refusal)?;
    // A host that wrote the file itself returns nothing; one with no
    // filesystem hands the bytes back for the caller to write, exactly
    // as `hew.doc.export` already does.
    match written {
        Some(bytes) => Ok(serde_json::json!({ "bytes_base64": encode_base64(&bytes) })),
        None => Ok(serde_json::json!({})),
    }
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
}

fn export_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ExportParams {
        format,
        path,
        segments_per_turn,
    } = parse(params)?;
    let bytes = ctx
        .host
        .export_document(
            ctx.doc,
            &format,
            path.as_deref(),
            segments_per_turn.unwrap_or(0),
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
    /// own (STL) refuse typed without it (docs/HEW_API.md §7's semantics
    /// note); formats that do (glTF, COLLADA, `.skp`) ignore it.
    #[serde(default)]
    units: Option<String>,
}

fn import_doc(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ImportParams { path, units } = parse(params)?;
    let options = serde_json::json!({ "units": units });
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
/// spec in the protocol (docs/HEW_API.md §7).
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
    include_ids: Option<bool>,
    #[serde(default)]
    path: Option<String>,
}

/// `hew.view.snapshot`'s default and clamped size bounds
/// (docs/design/headless-snapshot.md): a request outside `[MIN, MAX]` is
/// clamped rather than refused — a client asking for an absurd size still
/// gets a usable image, not a protocol error.
const SNAPSHOT_DEFAULT_SIZE: u32 = 512;
const SNAPSHOT_MIN_SIZE: u32 = 16;
const SNAPSHOT_MAX_SIZE: u32 = 2048;

fn snapshot(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let raw: RawSnapshotParams = parse(params)?;
    let (camera, view) = parse_camera_or_view(raw.camera, raw.view)?;
    let width = raw
        .width
        .unwrap_or(SNAPSHOT_DEFAULT_SIZE)
        .clamp(SNAPSHOT_MIN_SIZE, SNAPSHOT_MAX_SIZE);
    let height = raw
        .height
        .unwrap_or(SNAPSHOT_DEFAULT_SIZE)
        .clamp(SNAPSHOT_MIN_SIZE, SNAPSHOT_MAX_SIZE);

    let snapshot_params = SnapshotParams {
        width,
        height,
        camera,
        view,
        include_ids: raw.include_ids.unwrap_or(false),
        path: raw.path,
    };
    let result = ctx
        .host
        .snapshot(ctx.doc, &snapshot_params)
        .map_err(CmdError::Refusal)?;

    // Mirrors `hew.doc.export`'s posture (docs/HEW_API.md §7): bytes
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
            Ok(out)
        }
    }
}

// ------------------------------------------------------------------- base64

/// RFC 4648 §4 standard-alphabet base64, WITH padding. No new dependency —
/// the whole point of `hew.doc.export`'s inline-bytes path and
/// `hew.view.snapshot`'s PNG result is a few dozen lines nobody needs a
/// crate for.
fn encode_base64(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
