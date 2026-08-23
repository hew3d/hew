//! Live-viewport and app-display effects: `hew.view.camera`,
//! `hew.view.zoom_extents`, `hew.view.units` (docs/agents/HEW_API.md §7).
//!
//! Like `hew.view.snapshot` (doc.rs), all three are `Served::Host` — this
//! module owns no effect of its own, only parameter validation and result
//! shaping; the host performs the actual effect (or answers
//! `host_capability_missing` through the normal `CmdError::Refusal` path,
//! host.rs's `unsupported()`).
//!
//! None of the three changes document state: the registry declares
//! `mutates_document = false` for all three (`registry.rs`'s `Solitary`
//! default, un-corrected — contrast `hew.doc.new`/`open`, which the
//! registry explicitly flips back to `true`). A camera move is view
//! state, not a modeled edit; the display-unit format is an app-level
//! preference (`app/src/settings/units.ts`), never serialized into
//! `.hew`. Neither rides the undo log, and `crates/wasm-api`'s
//! `Scene::api_dispatch` never resyncs the document for them (it only
//! resyncs when `mutates_document` is true).

use super::camera::{RawCamera, parse_camera_or_view};
use super::{CmdError, Ctx, Handler};
use crate::host::ViewCameraSpec;
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.view.camera" => Some(camera),
        "hew.view.zoom_extents" => Some(zoom_extents),
        "hew.view.units" => Some(units),
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

// ---------------------------------------------------------- hew.view.camera

/// `camera`/`view` are the exact vocabulary `hew.view.snapshot` accepts
/// (`camera.rs`), mutually exclusive; unlike snapshot, exactly one is
/// REQUIRED here — there is no "give neither" default because there is no
/// document camera to fall back to (snapshot's "no camera given" case
/// renders the document's saved working camera or a fitted isometric
/// view; a live camera-set command with neither would be a no-op with no
/// honest meaning).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraParams {
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    view: Option<String>,
}

fn camera(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let CameraParams { camera, view } = parse(params)?;
    let (camera, view) = parse_camera_or_view(camera, view)?;
    let spec = match (camera, view) {
        (Some(c), None) => ViewCameraSpec::Explicit(c),
        (None, Some(v)) => ViewCameraSpec::Standard(v),
        (None, None) => {
            return Err(CmdError::Params(
                "one of camera or view is required".to_string(),
            ));
        }
        (Some(_), Some(_)) => {
            unreachable!("parse_camera_or_view already enforces mutual exclusion")
        }
    };
    ctx.host.set_camera(&spec).map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ---------------------------------------------------- hew.view.zoom_extents

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoomExtentsParams {}

fn zoom_extents(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let ZoomExtentsParams {} = parse(params)?;
    ctx.host.zoom_extents().map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}

// ----------------------------------------------------------- hew.view.units

/// `app/src/settings/units.ts`'s `LengthFormat` union, mirrored here so an
/// unrecognized value is refused statically (`-32602`) rather than reach
/// the host at all — the same posture `hew.doc.export`'s `format` enum
/// and `hew.view.snapshot`'s `view` name get from their own JSON Schema
/// `enum` plus this crate's own validation.
const VALID_FORMATS: &[&str] = &["m", "cm", "mm", "arch", "frac_in", "dec_in"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitsParams {
    format: String,
}

fn units(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let UnitsParams { format } = parse(params)?;
    if !VALID_FORMATS.contains(&format.as_str()) {
        return Err(CmdError::Params(format!(
            "unknown display unit format \"{format}\" (expected one of {VALID_FORMATS:?})"
        )));
    }
    ctx.host
        .set_display_units(&format)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({}))
}
