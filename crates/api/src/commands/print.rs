//! Printing: `hew.view.line_drawing` and `hew.print.pdf`
//! (docs/design/printing.md §9b, docs/agents/HEW_API.md §7). Both are
//! `Served::Host` like `hew.view.snapshot`: this module owns parameter
//! validation and result shaping; the host renders (`hew-cli` through
//! `crates/hlr`, `crates/softrender`, and `crates/pdfwrite`) or answers
//! `host_capability_missing`. Neither changes document state.

use super::camera::{RawCamera, parse_camera_or_view};
use super::doc::encode_base64;
use super::{CmdError, Ctx, Handler};
use crate::host::{LineDrawingFormat, LineDrawingParams, PrintPdfParams};
use crate::print_layout::Paper;
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.view.line_drawing" => Some(line_drawing),
        "hew.print.pdf" => Some(print_pdf),
        _ => None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLineDrawingParams {
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    scene: Option<String>,
    #[serde(default)]
    include_hidden: Option<bool>,
    #[serde(default)]
    include_soft: Option<bool>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    path: Option<String>,
}

fn line_drawing(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let raw: RawLineDrawingParams = parse(params)?;
    if raw.scene.is_some() && (raw.camera.is_some() || raw.view.is_some()) {
        return Err(CmdError::Params(
            "scene is mutually exclusive with camera and view".to_string(),
        ));
    }
    let (camera, view) = parse_camera_or_view(raw.camera, raw.view)?;
    let scene = raw
        .scene
        .as_deref()
        .map(super::scenes::resolve_scene_id)
        .transpose()?;
    let format = match raw.format.as_deref() {
        None | Some("svg") => LineDrawingFormat::Svg,
        Some("segments") => LineDrawingFormat::Segments,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "format must be \"svg\" or \"segments\", not {other:?}"
            )));
        }
    };
    let scale = raw.scale.unwrap_or(1.0);
    if !(scale.is_finite() && scale > 0.0) {
        return Err(CmdError::Params(
            "scale must be a positive number (paper / model)".to_string(),
        ));
    }
    if raw.path.is_some() && format != LineDrawingFormat::Svg {
        return Err(CmdError::Params(
            "path applies to format \"svg\" only".to_string(),
        ));
    }
    let p = LineDrawingParams {
        camera,
        view,
        scene,
        include_hidden: raw.include_hidden.unwrap_or(false),
        include_soft: raw.include_soft.unwrap_or(false),
        format,
        scale,
        path: raw.path,
    };
    let result = ctx
        .host
        .line_drawing(ctx.doc, &p)
        .map_err(CmdError::Refusal)?;
    let bounds = result.bounds.map(|b| serde_json::json!(b));
    match p.format {
        LineDrawingFormat::Svg => {
            let svg = result.svg.unwrap_or_default();
            match &p.path {
                Some(path) => {
                    ctx.host
                        .write_snapshot(path, svg.as_bytes())
                        .map_err(CmdError::Refusal)?;
                    Ok(serde_json::json!({ "path": path, "count": result.count, "bounds": bounds }))
                }
                None => {
                    Ok(serde_json::json!({ "svg": svg, "count": result.count, "bounds": bounds }))
                }
            }
        }
        LineDrawingFormat::Segments => Ok(serde_json::json!({
            "segments": result.segments,
            "kinds": result.kinds,
            "ids": result.ids,
            "count": result.count,
            "bounds": bounds,
        })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrintPdfParams {
    #[serde(default)]
    paper: Option<Value>,
    #[serde(default)]
    orientation: Option<String>,
    #[serde(default)]
    margin_mm: Option<f64>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    scale_label: Option<String>,
    #[serde(default)]
    camera: Option<RawCamera>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    scene: Option<String>,
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    include_hidden: Option<bool>,
    #[serde(default)]
    title_block: Option<bool>,
    #[serde(default)]
    scale_bar: Option<bool>,
    #[serde(default)]
    marks: Option<bool>,
    #[serde(default)]
    overlap_mm: Option<f64>,
    #[serde(default)]
    units: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

fn print_pdf(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let raw: RawPrintPdfParams = parse(params)?;
    if raw.scene.is_some() && (raw.camera.is_some() || raw.view.is_some()) {
        return Err(CmdError::Params(
            "scene is mutually exclusive with camera and view".to_string(),
        ));
    }
    let (camera, view) = parse_camera_or_view(raw.camera, raw.view)?;
    let scene = raw
        .scene
        .as_deref()
        .map(super::scenes::resolve_scene_id)
        .transpose()?;
    let paper = match &raw.paper {
        None => Paper::named("a4").expect("a4 is built in"),
        Some(Value::String(name)) => Paper::named(name).ok_or_else(|| {
            CmdError::Params(format!(
                "paper must be one of letter, legal, tabloid, a5, a4, a3, or {{w_mm, h_mm}}; not {name:?}"
            ))
        })?,
        Some(Value::Object(o)) => {
            let w = o.get("w_mm").and_then(Value::as_f64);
            let h = o.get("h_mm").and_then(Value::as_f64);
            match (w, h) {
                (Some(w), Some(h))
                    if w.is_finite()
                        && h.is_finite()
                        && (50.0..=2000.0).contains(&w)
                        && (50.0..=2000.0).contains(&h) =>
                {
                    Paper { w: w.min(h), h: w.max(h) }
                }
                _ => {
                    return Err(CmdError::Params(
                        "paper {w_mm, h_mm} must be finite numbers between 50 and 2000".to_string(),
                    ));
                }
            }
        }
        Some(_) => {
            return Err(CmdError::Params(
                "paper must be a name or {w_mm, h_mm}".to_string(),
            ));
        }
    };
    let landscape = match raw.orientation.as_deref() {
        None | Some("auto") => None,
        Some("portrait") => Some(false),
        Some("landscape") => Some(true),
        Some(other) => {
            return Err(CmdError::Params(format!(
                "orientation must be auto, portrait, or landscape; not {other:?}"
            )));
        }
    };
    let scaled = match raw.mode.as_deref() {
        None | Some("scaled") => true,
        Some("standard") => false,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "mode must be \"scaled\" or \"standard\", not {other:?}"
            )));
        }
    };
    let ratio = raw.scale.unwrap_or(0.1);
    if !(ratio.is_finite() && (0.001..=20.0).contains(&ratio)) {
        return Err(CmdError::Params(
            "scale must be paper/model between 0.001 and 20".to_string(),
        ));
    }
    let line_art = match raw.style.as_deref() {
        None | Some("line_art") => true,
        Some("shaded") => false,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "style must be \"line_art\" or \"shaded\", not {other:?}"
            )));
        }
    };
    let metric = match raw.units.as_deref() {
        None | Some("metric") => true,
        Some("imperial") => false,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "units must be \"metric\" or \"imperial\", not {other:?}"
            )));
        }
    };
    let margin_mm = raw
        .margin_mm
        .unwrap_or(crate::print_layout::DEFAULT_MARGIN_MM);
    if !(margin_mm.is_finite() && (0.0..=50.0).contains(&margin_mm)) {
        return Err(CmdError::Params(
            "margin_mm must be between 0 and 50".to_string(),
        ));
    }
    // The two are valid alone but must leave a drawing area together.
    let inner_w = paper.w - 2.0 * margin_mm;
    let inner_h = paper.h - 2.0 * margin_mm - crate::print_layout::DEFAULT_TITLE_BLOCK_MM;
    if inner_w < 20.0 || inner_h < 20.0 {
        return Err(CmdError::Params(format!(
            "margin_mm {margin_mm} leaves no drawing area on a {} × {} mm sheet (need at least 20 mm each way)",
            paper.w, paper.h
        )));
    }
    let scale_label = raw.scale_label.unwrap_or_else(|| ratio_label(ratio));
    let p = PrintPdfParams {
        paper_w_mm: paper.w,
        paper_h_mm: paper.h,
        landscape,
        margin_mm,
        scaled,
        ratio,
        scale_label,
        camera,
        view,
        scene,
        line_art,
        include_hidden: raw.include_hidden.unwrap_or(false),
        title_block: raw.title_block.unwrap_or(true),
        scale_bar: raw.scale_bar.unwrap_or(true),
        marks: raw.marks.unwrap_or(true),
        overlap_mm: raw
            .overlap_mm
            .unwrap_or(crate::print_layout::DEFAULT_OVERLAP_MM),
        metric,
        title: raw.title,
        path: raw.path,
    };
    let result = ctx.host.print_pdf(ctx.doc, &p).map_err(CmdError::Refusal)?;
    match &p.path {
        Some(path) => {
            ctx.host
                .write_snapshot(path, &result.pdf)
                .map_err(CmdError::Refusal)?;
            Ok(
                serde_json::json!({ "path": path, "pages": result.pages, "cols": result.cols, "rows": result.rows }),
            )
        }
        None => Ok(serde_json::json!({
            "pdf_base64": encode_base64(&result.pdf),
            "pages": result.pages,
            "cols": result.cols,
            "rows": result.rows,
        })),
    }
}

/// "1:10" / "2:1" from a ratio.
pub fn ratio_label(ratio: f64) -> String {
    let near = |v: f64| {
        if (v - v.round()).abs() < 1e-6 {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.2}")
        }
    };
    if ratio >= 1.0 {
        format!("{}:1", near(ratio))
    } else {
        format!("1:{}", near(1.0 / ratio))
    }
}
