//! SVG writer for a `LineDrawing`: real-size millimetres at a drawing scale,
//! so laser/CNC software and browsers read the file at true size.

use crate::{Kind, LineDrawing};

/// Stroke weights in mm and the drawing scale.
#[derive(Debug, Clone, Copy)]
pub struct SvgStyle {
    /// paper : model (0.1 = 1:10).
    pub ratio: f64,
    pub hard_mm: f64,
    pub soft_mm: f64,
    pub hidden_mm: f64,
    /// Margin around the drawing, mm.
    pub margin_mm: f64,
}

impl Default for SvgStyle {
    fn default() -> Self {
        SvgStyle {
            ratio: 1.0,
            hard_mm: 0.35,
            soft_mm: 0.18,
            hidden_mm: 0.25,
            margin_mm: 5.0,
        }
    }
}

fn fmt(v: f64) -> String {
    // Three decimals of a millimetre is a micron: plenty, and stable.
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".to_string() } else { s }
}

/// Write the drawing as an SVG document. Coordinates are mm (y down), the
/// `viewBox` spans the drawing's bounds plus the margin; `width`/`height` are
/// physical mm so the file opens at true size.
pub fn write(d: &LineDrawing, style: &SvgStyle) -> String {
    let k = style.ratio * 1000.0; // model m → paper mm
    let (min, max) = d.bounds.unwrap_or(([0.0, 0.0], [0.0, 0.0]));
    let x0 = min[0] * k - style.margin_mm;
    let y0 = -max[1] * k - style.margin_mm; // y flips
    let w = (max[0] - min[0]) * k + 2.0 * style.margin_mm;
    let h = (max[1] - min[1]) * k + 2.0 * style.margin_mm;
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"{} {} {} {}\">\n",
        fmt(w),
        fmt(h),
        fmt(x0),
        fmt(y0),
        fmt(w),
        fmt(h)
    ));
    // One <path> per kind keeps files small and lets editors restyle a class.
    for (kind, class, width, dash) in [
        (Kind::Hard, "hard", style.hard_mm, None),
        (Kind::Silhouette, "silhouette", style.hard_mm, None),
        (Kind::Section, "section", style.hard_mm * 1.4, None),
        (Kind::Soft, "soft", style.soft_mm, None),
        (Kind::Hidden, "hidden", style.hidden_mm, Some("1.5 1")),
    ] {
        let mut dpath = String::new();
        for s in d.segs.iter().filter(|s| s.kind == kind) {
            dpath.push_str(&format!(
                "M{} {}L{} {}",
                fmt(s.a[0] * k),
                fmt(-s.a[1] * k),
                fmt(s.b[0] * k),
                fmt(-s.b[1] * k)
            ));
        }
        if dpath.is_empty() {
            continue;
        }
        let dash_attr = dash
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        out.push_str(&format!(
            "  <path class=\"{class}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{}\" stroke-linecap=\"round\"{dash_attr} d=\"{dpath}\"/>\n",
            fmt(width)
        ));
    }
    out.push_str("</svg>\n");
    out
}
