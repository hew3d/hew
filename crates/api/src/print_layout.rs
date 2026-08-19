//! Print page layout for the headless `hew.print.pdf` (docs/design/printing.md
//! §9b) — a Rust mirror of the app's `app/src/print/layout.ts` (§6): paper
//! sizes, margins, the drawing area, the tile grid with overlap bands, the
//! scale bar, and the page furniture (title block, crop/trim marks,
//! neighbour labels, scale bar) as plain drawing items in millimetres from
//! the page's top-left. Pure: no I/O, no rendering; a host renders the
//! drawings and hands both to `pdfwrite`. The two implementations are held
//! to the same numbers by shared fixtures (`tests/print_layout_fixtures.rs`
//! ↔ `layout.test.ts`).
//!
//! Coordinate frames: paper mm, y down; the tile's `model_rect` is in
//! view-plane metres, y up, relative to the print camera centre — exactly
//! `hlr::Seg` coordinates.

pub const MM_PER_INCH: f64 = 25.4;
pub const DEFAULT_OVERLAP_MM: f64 = 10.0;
pub const DEFAULT_TITLE_BLOCK_MM: f64 = 10.0;
pub const DEFAULT_MARGIN_MM: f64 = 12.7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectMm {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A rectangle in the view plane, metres, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectM {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Auto,
    Portrait,
    Landscape,
}

/// Portrait paper size in mm; `named` resolves the built-in list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    pub w: f64,
    pub h: f64,
}

impl Paper {
    pub fn named(name: &str) -> Option<Paper> {
        Some(match name {
            "letter" => Paper { w: 215.9, h: 279.4 },
            "legal" => Paper { w: 215.9, h: 355.6 },
            "tabloid" => Paper { w: 279.4, h: 431.8 },
            "a5" => Paper { w: 148.0, h: 210.0 },
            "a4" => Paper { w: 210.0, h: 297.0 },
            "a3" => Paper { w: 297.0, h: 420.0 },
            _ => return None,
        })
    }
    pub fn label(&self) -> String {
        for name in ["letter", "legal", "tabloid", "a5", "a4", "a3"] {
            let p = Paper::named(name).unwrap();
            if (p.w - self.w).abs() < 0.5 && (p.h - self.h).abs() < 0.5 {
                let mut c = name.chars();
                let mut s = String::new();
                if let Some(f) = c.next() {
                    s.push(f.to_ascii_uppercase());
                }
                s.push_str(c.as_str());
                return if s == "A5" || s == "A4" || s == "A3" {
                    s.to_uppercase()
                } else {
                    s
                };
            }
        }
        format!("Custom {} × {} mm", self.w, self.h)
    }
    pub fn oriented(&self, landscape: bool) -> (f64, f64) {
        if landscape {
            (self.h, self.w)
        } else {
            (self.w, self.h)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageGeometry {
    pub landscape: bool,
    pub paper_w: f64,
    pub paper_h: f64,
    pub margin_mm: f64,
    pub drawing: RectMm,
    pub title_block: Option<RectMm>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: String,
    pub row: usize,
    pub col: usize,
    pub page: usize,
    pub image_rect_mm: RectMm,
    pub image_px: (u32, u32),
    /// Scaled only.
    pub model_rect: Option<RectM>,
    pub overlap_right: bool,
    pub overlap_bottom: bool,
    pub left: Option<String>,
    pub right: Option<String>,
    pub up: Option<String>,
    pub down: Option<String>,
}

/// A graphic (architectural) scale: four alternating black/white segments
/// of `segment_mm` each, tick labels in round MODEL units at every boundary
/// ("0 5 10 15 20 cm"), plus the bar's own paper length for the ruler check.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleBar {
    /// Total bar length on paper, mm (= 4 × segment_mm).
    pub paper_mm: f64,
    /// One segment on paper, mm.
    pub segment_mm: f64,
    /// Model length of one segment, metres.
    pub segment_meters: f64,
    /// Five tick labels, 0 … 4 segments; the unit rides on the last one.
    pub labels: Vec<String>,
    /// The bar's own paper length: "40 mm" or "2 in".
    pub paper_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub scaled: bool,
    pub page: PageGeometry,
    pub dpi: u32,
    pub tiles: Vec<Tile>,
    pub rows: usize,
    pub cols: usize,
    pub scale_bar: Option<ScaleBar>,
    /// The overlap band actually reserved (mm): the requested band on a
    /// tiled layout, 0 on a single page.
    pub overlap_mm: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutInput {
    pub scaled: bool,
    pub paper: Paper,
    pub orientation: Orientation,
    pub margin_mm: f64,
    pub title_block_mm: f64,
    pub overlap_mm: f64,
    pub dpi: u32,
    /// Standard: viewport aspect (w/h) to letterbox.
    pub viewport_aspect: f64,
    /// Scaled: paper/model ratio.
    pub ratio: f64,
    /// Scaled: extent, view-plane metres.
    pub extent: RectM,
    /// Scale-bar unit family; None = no bar.
    pub scale_bar_metric: Option<bool>,
}

pub fn mm_to_px(mm: f64, dpi: u32) -> u32 {
    (mm / MM_PER_INCH * dpi as f64).round().max(0.0) as u32
}

pub fn px_to_mm(px: u32, dpi: u32) -> f64 {
    px as f64 / dpi as f64 * MM_PER_INCH
}

pub fn row_letter(row: usize) -> String {
    let mut s = String::new();
    let mut n = row as i64;
    loop {
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n = n / 26 - 1;
        if n < 0 {
            break;
        }
    }
    s
}

pub fn tile_id(row: usize, col: usize) -> String {
    format!("{}{}", row_letter(row), col + 1)
}

/// Page geometry. The drawing area is the printable area minus the title
/// block, minus — on a tiled layout — the overlap band, which is reserved
/// INSIDE the printable area on the right and bottom (design decision #12).
/// Uniform for every tile; the last column/row leave it empty.
/// `drawing = paper − 2·margin − title_block − overlap`.
fn page_geometry(
    paper: &Paper,
    landscape: bool,
    margin_mm: f64,
    title_block_mm: f64,
    overlap_mm: f64,
) -> PageGeometry {
    let (pw, ph) = paper.oriented(landscape);
    let inner = RectMm {
        x: margin_mm,
        y: margin_mm,
        w: pw - 2.0 * margin_mm,
        h: ph - 2.0 * margin_mm,
    };
    let title_block = if title_block_mm > 0.0 {
        Some(RectMm {
            x: inner.x,
            y: inner.y + inner.h - title_block_mm,
            w: inner.w,
            h: title_block_mm,
        })
    } else {
        None
    };
    let drawing = RectMm {
        x: inner.x,
        y: inner.y,
        w: inner.w - overlap_mm,
        h: inner.h
            - if title_block.is_some() {
                title_block_mm
            } else {
                0.0
            }
            - overlap_mm,
    };
    PageGeometry {
        landscape,
        paper_w: pw,
        paper_h: ph,
        margin_mm,
        drawing,
        title_block,
    }
}

/// Hard ceiling on tiles per axis: past this the request is nonsense (a zero
/// drawing area, a scale off by orders of magnitude) and must not allocate.
pub const MAX_TILES_PER_AXIS: usize = 500;

fn tile_count(extent_mm: f64, step_mm: f64) -> usize {
    if step_mm <= 0.0 || step_mm.is_nan() || !extent_mm.is_finite() {
        return 1;
    }
    (((extent_mm / step_mm - 1e-9).ceil().max(1.0)) as usize).min(MAX_TILES_PER_AXIS)
}

fn trim_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Inches as "1", "¼", "1½", "2¾" (quarter steps only).
fn inch_label(inches: f64) -> String {
    let whole = (inches + 1e-9).floor();
    let frac = ((inches - whole) * 4.0).round() / 4.0;
    let f = match (frac * 4.0).round() as i64 {
        1 => "¼",
        2 => "½",
        3 => "¾",
        0 => return trim_num(inches, 2),
        _ => return trim_num(inches, 2),
    };
    if whole == 0.0 {
        f.to_string()
    } else {
        format!("{}{}", whole as i64, f)
    }
}

/// The graphic scale bar for a drawing `drawing_w_mm` wide at `ratio`
/// (mirrors `pickScaleBar`): the largest ROUND model step — {1, 2, 5} × 10ᵏ
/// mm, or ¼″ ½″ 1″ 2″ 3″ 6″ 1′ 2′ 3′ 6′ 10′ — whose paper length p = step ×
/// ratio is ≥ 6 mm with 4·p ≤ min(40 % of the drawing width, 80 mm).
pub fn pick_scale_bar(drawing_w_mm: f64, ratio: f64, metric: bool) -> ScaleBar {
    let limit = (drawing_w_mm * 0.4).min(80.0);
    let mut steps: Vec<f64> = Vec::new();
    if metric {
        for k in -1..=4 {
            for m in [1.0, 2.0, 5.0] {
                steps.push(m * 10f64.powi(k));
            }
        }
    } else {
        for inch in [0.25, 0.5, 1.0, 2.0, 3.0, 6.0, 12.0, 24.0, 36.0, 72.0, 120.0] {
            steps.push(inch * MM_PER_INCH);
        }
    }
    let paper_of = |st: f64| st * ratio;
    let step = steps
        .iter()
        .rev()
        .copied()
        .find(|st| paper_of(*st) >= 6.0 && 4.0 * paper_of(*st) <= limit)
        .or_else(|| {
            steps
                .iter()
                .rev()
                .copied()
                .find(|st| 4.0 * paper_of(*st) <= limit)
        })
        .unwrap_or(steps[0]);
    let segment_mm = paper_of(step);
    let mut labels: Vec<String> = Vec::with_capacity(5);
    if metric {
        let (div, unit) = if step >= 1000.0 {
            (1000.0, "m")
        } else if step >= 10.0 {
            (10.0, "cm")
        } else {
            (1.0, "mm")
        };
        for i in 0..=4 {
            labels.push(trim_num(i as f64 * step / div, 3));
        }
        labels[4] = format!("{} {}", labels[4], unit);
    } else {
        let inches = step / MM_PER_INCH;
        if inches >= 12.0 {
            for i in 0..=4 {
                labels.push(trim_num(i as f64 * inches / 12.0, 2));
            }
            labels[4] = format!("{} ft", labels[4]);
        } else {
            for i in 0..=4 {
                labels.push(inch_label(i as f64 * inches));
            }
            labels[4] = format!("{} in", labels[4]);
        }
    }
    let paper_mm = 4.0 * segment_mm;
    let paper_label = if metric {
        format!("{} mm", trim_num(paper_mm, 1))
    } else {
        format!("{} in", trim_num(paper_mm / MM_PER_INCH, 2))
    };
    ScaleBar {
        paper_mm,
        segment_mm,
        segment_meters: step / 1000.0,
        labels,
        paper_label,
    }
}

fn layout_for(input: &LayoutInput, landscape: bool) -> Layout {
    let mut page = page_geometry(
        &input.paper,
        landscape,
        input.margin_mm,
        input.title_block_mm,
        0.0,
    );
    let dpi = input.dpi;
    if !input.scaled {
        let d = page.drawing;
        let aspect = if input.viewport_aspect > 0.0 {
            input.viewport_aspect
        } else {
            4.0 / 3.0
        };
        let mut w_px = mm_to_px(d.w, dpi);
        let mut h_px = (w_px as f64 / aspect).round() as u32;
        if px_to_mm(h_px, dpi) > d.h {
            h_px = mm_to_px(d.h, dpi);
            w_px = (h_px as f64 * aspect).round() as u32;
        }
        let w_mm = px_to_mm(w_px, dpi);
        let h_mm = px_to_mm(h_px, dpi);
        let tile = Tile {
            id: "A1".into(),
            row: 0,
            col: 0,
            page: 0,
            image_rect_mm: RectMm {
                x: d.x + (d.w - w_mm) / 2.0,
                y: d.y + (d.h - h_mm) / 2.0,
                w: w_mm,
                h: h_mm,
            },
            image_px: (w_px, h_px),
            model_rect: None,
            overlap_right: false,
            overlap_bottom: false,
            left: None,
            right: None,
            up: None,
            down: None,
        };
        return Layout {
            scaled: false,
            page,
            dpi,
            tiles: vec![tile],
            rows: 1,
            cols: 1,
            scale_bar: None,
            overlap_mm: 0.0,
        };
    }
    let ratio = input.ratio;
    let extent = input.extent;
    let mm_per_m = ratio * 1000.0;
    let ext_w = (extent.w * mm_per_m).max(1e-6);
    let ext_h = (extent.h * mm_per_m).max(1e-6);
    // First on the full drawing area: one page needs no band. Only a tiled
    // print reserves the overlap inside the printable area (decision #12) —
    // and then every tile steps by the reduced drawing area, uniformly.
    let requested = input.overlap_mm.max(0.0);
    let mut cols = tile_count(ext_w, page.drawing.w);
    let mut rows = tile_count(ext_h, page.drawing.h);
    let mut overlap = 0.0;
    if requested > 0.0 && (cols > 1 || rows > 1) {
        // Never eat more than half the drawing area (a tiny custom sheet).
        overlap = requested.min((page.drawing.w.min(page.drawing.h) / 2.0).max(0.0));
        page = page_geometry(
            &input.paper,
            landscape,
            input.margin_mm,
            input.title_block_mm,
            overlap,
        );
        cols = tile_count(ext_w, page.drawing.w);
        rows = tile_count(ext_h, page.drawing.h);
    }
    let d = page.drawing;
    let start_x = (cols as f64 * d.w - ext_w) / 2.0;
    let start_y = (rows as f64 * d.h - ext_h) / 2.0;
    let mut tiles = Vec::with_capacity(rows * cols);
    let mut page_idx = 0;
    for r in 0..rows {
        for c in 0..cols {
            let overlap_right = overlap > 0.0 && c < cols - 1;
            let overlap_bottom = overlap > 0.0 && r < rows - 1;
            let band_r = if overlap_right { overlap } else { 0.0 };
            let band_b = if overlap_bottom { overlap } else { 0.0 };
            let w_px = mm_to_px(d.w + band_r, dpi);
            let h_px = mm_to_px(d.h + band_b, dpi);
            let w_mm = px_to_mm(w_px, dpi);
            let h_mm = px_to_mm(h_px, dpi);
            let grid_x = c as f64 * d.w;
            let grid_y = r as f64 * d.h;
            let model_x = extent.x + (grid_x - start_x) / mm_per_m;
            let model_top = extent.y + extent.h - (grid_y - start_y) / mm_per_m;
            let model_w = w_mm / mm_per_m;
            let model_h = h_mm / mm_per_m;
            tiles.push(Tile {
                id: tile_id(r, c),
                row: r,
                col: c,
                page: page_idx,
                image_rect_mm: RectMm {
                    x: d.x,
                    y: d.y,
                    w: w_mm,
                    h: h_mm,
                },
                image_px: (w_px, h_px),
                model_rect: Some(RectM {
                    x: model_x,
                    y: model_top - model_h,
                    w: model_w,
                    h: model_h,
                }),
                overlap_right,
                overlap_bottom,
                left: if c > 0 { Some(tile_id(r, c - 1)) } else { None },
                right: if c + 1 < cols {
                    Some(tile_id(r, c + 1))
                } else {
                    None
                },
                up: if r > 0 { Some(tile_id(r - 1, c)) } else { None },
                down: if r + 1 < rows {
                    Some(tile_id(r + 1, c))
                } else {
                    None
                },
            });
            page_idx += 1;
        }
    }
    let scale_bar = input
        .scale_bar_metric
        .map(|metric| pick_scale_bar(d.w, ratio, metric));
    Layout {
        scaled: true,
        page,
        dpi,
        tiles,
        rows,
        cols,
        scale_bar,
        overlap_mm: overlap,
    }
}

/// Lay out a print. `Auto` orientation: Standard follows the viewport
/// aspect; Scaled takes whichever needs fewer tiles (ties → portrait).
pub fn layout(input: &LayoutInput) -> Layout {
    match input.orientation {
        Orientation::Portrait => layout_for(input, false),
        Orientation::Landscape => layout_for(input, true),
        Orientation::Auto => {
            if !input.scaled {
                return layout_for(input, input.viewport_aspect > 1.0);
            }
            let p = layout_for(input, false);
            let l = layout_for(input, true);
            if l.tiles.len() < p.tiles.len() { l } else { p }
        }
    }
}

/// The ratio at which `extent` fills one page in the best orientation.
pub fn fit_ratio(input: &LayoutInput) -> f64 {
    let orientations: Vec<bool> = match input.orientation {
        Orientation::Auto => vec![false, true],
        Orientation::Portrait => vec![false],
        Orientation::Landscape => vec![true],
    };
    let mut best = 0.0f64;
    for l in orientations {
        let page = page_geometry(&input.paper, l, input.margin_mm, input.title_block_mm, 0.0);
        let r = (page.drawing.w / 1000.0 / input.extent.w.max(1e-9))
            .min(page.drawing.h / 1000.0 / input.extent.h.max(1e-9));
        best = best.max(r);
    }
    best
}

// -------------------------------------------------------------- furniture

/// A page-furniture primitive, mm from the top-left (mirrors
/// `furniture.ts`; consumed by the host's PDF composer).
#[derive(Debug, Clone, PartialEq)]
pub enum Furniture {
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        width_mm: f64,
        gray: f64,
        dash: Option<Vec<f64>>,
    },
    Text {
        x: f64,
        y: f64,
        size_mm: f64,
        text: String,
        gray: f64,
        bold: bool,
        align: TextAlign,
        /// Clockwise rotation about the anchor, degrees (90 = reads top to
        /// bottom, glyph tops to the right); 0 for ordinary text.
        rotate_deg: f64,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        stroke_mm: Option<f64>,
        fill_gray: Option<f64>,
        gray: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FurnitureContext {
    /// Document name for the title block.
    pub document_name: String,
    /// Second line under the name: "Top view · Model", a Scene name,
    /// "Perspective view".
    pub subtitle: String,
    /// Scale text; None in Standard mode.
    pub scale_text: Option<String>,
    pub date_text: String,
    pub marks: bool,
    pub title_block: bool,
    pub scale_bar: bool,
}

/// Ink levels (0 = black … 1 = white) — `INK` in furniture.ts.
pub mod ink {
    /// #111 — primary text, bar fills.
    pub const TEXT: f64 = 0.067;
    /// #1a1a1a — rules and marks.
    pub const RULE: f64 = 0.1;
    /// #444 — secondary text.
    pub const SECONDARY: f64 = 0.267;
    /// #555 — neighbour labels.
    pub const TERTIARY: f64 = 0.333;
}

// The SPEC's numbers, named (`FURNITURE` in furniture.ts).
const TITLE_RULE_MM: f64 = 0.25;
const TITLE_TOP_PAD_MM: f64 = 0.9;
const DOC_NAME_MM: f64 = 3.2;
const SUBTITLE_MM: f64 = 2.3;
const SCALE_TEXT_MM: f64 = 2.6;
const NOT_TO_SCALE_MM: f64 = 2.8;
const SUB_NOTE_MM: f64 = 2.1;
const TILE_ID_MM: f64 = 4.6;
const PAGE_LINE_MM: f64 = 2.2;
const PAGE_LINE_SINGLE_MM: f64 = 2.3;
const BAR_HEIGHT_MM: f64 = 1.8;
const BAR_STROKE_MM: f64 = 0.25;
const BAR_LABEL_MM: f64 = 2.0;
const BAR_CAPTION_MM: f64 = 2.1;
const CROP_LEN_MM: f64 = 5.0;
const CROP_GAP_MM: f64 = 1.5;
const CROP_WIDTH_MM: f64 = 0.3;
const TRIM_WIDTH_MM: f64 = 0.3;
const TRIM_CAPTION_MM: f64 = 2.3;
const NEIGHBOR_LABEL_MM: f64 = 2.6;
const NEIGHBOR_INSET_MM: f64 = 2.0;
/// Estimated advance of "HEW" at 700 weight, ems.
const BRAND_EM: f64 = 2.333;

/// Baseline y for a text box whose top is `top` (ascender ≈ 0.78 em).
fn baseline(top: f64, size_mm: f64) -> f64 {
    top + size_mm * 0.78
}

fn text(x: f64, y: f64, t: &str, size: f64, align: TextAlign, gray: f64) -> Furniture {
    Furniture::Text {
        x,
        y,
        size_mm: size,
        text: t.to_string(),
        gray,
        bold: false,
        align,
        rotate_deg: 0.0,
    }
}

fn bold(x: f64, y: f64, t: &str, size: f64, align: TextAlign, gray: f64) -> Furniture {
    Furniture::Text {
        x,
        y,
        size_mm: size,
        text: t.to_string(),
        gray,
        bold: true,
        align,
        rotate_deg: 0.0,
    }
}

fn filled(x: f64, y: f64, w: f64, h: f64, gray: f64) -> Furniture {
    Furniture::Rect {
        x,
        y,
        w,
        h,
        stroke_mm: None,
        fill_gray: Some(gray),
        gray,
    }
}

/// Content of the title block, already resolved to strings (mirrors
/// `TitleBlockContent`).
pub struct TitleBlockContent<'a> {
    pub document_name: &'a str,
    pub subtitle: &'a str,
    pub center_main: Option<&'a str>,
    pub center_sub: Option<&'a str>,
    pub show_scale_bar: bool,
    pub tile_id: Option<&'a str>,
    pub page_text: &'a str,
    pub date_text: &'a str,
    /// Horizontal inset of the strip's content at both ends, mm — room for
    /// the bottom crop marks, which run down into the strip at its ends.
    pub inset_mm: f64,
}

/// The title block strip: rule, three columns, brand mark (mirrors
/// `titleBlockItems`).
pub fn title_block_items(layout: &Layout, content: &TitleBlockContent<'_>) -> Vec<Furniture> {
    let mut items = Vec::new();
    let Some(tb) = &layout.page.title_block else {
        return items;
    };
    let top = tb.y + TITLE_TOP_PAD_MM;
    let left = tb.x + content.inset_mm;
    let right = tb.x + tb.w - content.inset_mm;
    items.push(Furniture::Line {
        x1: tb.x,
        y1: tb.y,
        x2: tb.x + tb.w,
        y2: tb.y,
        width_mm: TITLE_RULE_MM,
        gray: ink::RULE,
        dash: None,
    });
    // Left: name over subtitle.
    items.push(bold(
        left,
        baseline(top, DOC_NAME_MM),
        content.document_name,
        DOC_NAME_MM,
        TextAlign::Left,
        ink::TEXT,
    ));
    items.push(text(
        left,
        baseline(top + DOC_NAME_MM * 1.2 + 0.8, SUBTITLE_MM),
        content.subtitle,
        SUBTITLE_MM,
        TextAlign::Left,
        ink::SECONDARY,
    ));
    // Centre: scale (+ graphic bar) or a count; nothing on a Standard page.
    let cx = tb.x + tb.w / 2.0;
    match (
        content.show_scale_bar,
        &layout.scale_bar,
        content.center_main,
    ) {
        (true, Some(sb), Some(main)) => {
            items.push(bold(
                cx,
                baseline(top, SCALE_TEXT_MM),
                main,
                SCALE_TEXT_MM,
                TextAlign::Center,
                ink::TEXT,
            ));
            // The bar's paper length, for the ruler check.
            let caption = sb.paper_label.clone();
            let caption_w = caption.chars().count() as f64 * 0.5 * BAR_CAPTION_MM;
            let last_label_w = sb.labels[4].chars().count() as f64 * 0.5 * BAR_LABEL_MM;
            let gap = (last_label_w / 2.0 + 1.0).max(2.5);
            let block_top = top + SCALE_TEXT_MM * 1.15 + 0.5;
            let x0 = cx - (sb.paper_mm + gap + caption_w) / 2.0;
            let label_base = baseline(block_top, BAR_LABEL_MM);
            for (i, label) in sb.labels.iter().enumerate() {
                items.push(text(
                    x0 + i as f64 * sb.segment_mm,
                    label_base,
                    label,
                    BAR_LABEL_MM,
                    TextAlign::Center,
                    ink::TEXT,
                ));
            }
            let bar_y = block_top + 2.7;
            for i in [0usize, 2] {
                items.push(filled(
                    x0 + i as f64 * sb.segment_mm,
                    bar_y,
                    sb.segment_mm,
                    BAR_HEIGHT_MM,
                    ink::TEXT,
                ));
            }
            items.push(Furniture::Rect {
                x: x0,
                y: bar_y,
                w: sb.paper_mm,
                h: BAR_HEIGHT_MM,
                stroke_mm: Some(BAR_STROKE_MM),
                fill_gray: None,
                gray: ink::TEXT,
            });
            items.push(text(
                x0 + sb.paper_mm + gap,
                bar_y + BAR_HEIGHT_MM - 0.2,
                &caption,
                BAR_CAPTION_MM,
                TextAlign::Left,
                ink::SECONDARY,
            ));
        }
        (_, _, Some(main)) => {
            let main_size = if content.center_sub.is_some() {
                NOT_TO_SCALE_MM
            } else {
                SCALE_TEXT_MM
            };
            items.push(bold(
                cx,
                baseline(top, main_size),
                main,
                main_size,
                TextAlign::Center,
                ink::TEXT,
            ));
            if let Some(sub) = content.center_sub {
                items.push(text(
                    cx,
                    baseline(top + main_size * 1.2 + 0.8, SUB_NOTE_MM),
                    sub,
                    SUB_NOTE_MM,
                    TextAlign::Center,
                    ink::SECONDARY,
                ));
            }
        }
        _ => {}
    }
    // Right: tile id over the page line, or page over date; "HEW" in bold.
    let brand_w = BRAND_EM * PAGE_LINE_MM;
    match content.tile_id {
        Some(id) => {
            items.push(bold(
                right,
                top + TILE_ID_MM * 0.78,
                id,
                TILE_ID_MM,
                TextAlign::Right,
                ink::TEXT,
            ));
            let line_base = baseline(top + TILE_ID_MM + 0.9, PAGE_LINE_MM);
            items.push(bold(
                right,
                line_base,
                "HEW",
                PAGE_LINE_MM,
                TextAlign::Right,
                ink::RULE,
            ));
            let line = if content.date_text.is_empty() {
                format!("{} ·", content.page_text)
            } else {
                format!("{} · {} ·", content.page_text, content.date_text)
            };
            items.push(text(
                right - brand_w - 0.6,
                line_base,
                &line,
                PAGE_LINE_MM,
                TextAlign::Right,
                ink::SECONDARY,
            ));
        }
        None => {
            items.push(text(
                right,
                baseline(top, PAGE_LINE_SINGLE_MM),
                content.page_text,
                PAGE_LINE_SINGLE_MM,
                TextAlign::Right,
                ink::SECONDARY,
            ));
            let line_base = baseline(top + PAGE_LINE_SINGLE_MM * 1.2 + 0.8, PAGE_LINE_MM);
            items.push(bold(
                right,
                line_base,
                "HEW",
                PAGE_LINE_MM,
                TextAlign::Right,
                ink::RULE,
            ));
            if !content.date_text.is_empty() {
                items.push(text(
                    right - brand_w - 0.6,
                    line_base,
                    &format!("{} ·", content.date_text),
                    PAGE_LINE_MM,
                    TextAlign::Right,
                    ink::SECONDARY,
                ));
            }
        }
    }
    items
}

/// Everything on one page besides the drawing (mirrors `pageFurniture`).
pub fn page_furniture(layout: &Layout, tile: &Tile, ctx: &FurnitureContext) -> Vec<Furniture> {
    let mut items = Vec::new();
    let d = layout.page.drawing;
    let total = layout.tiles.len();
    let tiled = total > 1;
    let title_on = ctx.title_block && layout.page.title_block.is_some();

    // Marks are a Scaled-mode thing: a single scaled sheet gets corner crop
    // marks (a squareness check); Standard is a picture and gets none.
    if ctx.marks && layout.scaled {
        let ov = layout.overlap_mm;
        let x2 = d.x + d.w;
        let y2 = d.y + d.h;
        let band_r = if tile.overlap_right { ov } else { 0.0 };
        let band_b = if tile.overlap_bottom { ov } else { 0.0 };
        // Crop marks at the drawing-area corners: 5 × 0.3 bars, 1.5 gap; a
        // bar is skipped where it would run into a band. The bottom vertical
        // bars run down into the title-block strip's ends (its content is
        // inset to leave them room) so a corner always shows both.
        // Bars are clipped to the sheet: with Narrow margins the 6.5 mm reach
        // (gap + length) exceeds the 6.35 mm margin by a hair.
        let pw = layout.page.paper_w;
        let ph = layout.page.paper_h;
        let hbar = |items: &mut Vec<Furniture>, x: f64, y: f64| {
            let x0 = x.max(0.0);
            let x1 = (x + CROP_LEN_MM).min(pw);
            if x1 - x0 > 0.5 {
                items.push(filled(
                    x0,
                    y - CROP_WIDTH_MM / 2.0,
                    x1 - x0,
                    CROP_WIDTH_MM,
                    ink::RULE,
                ));
            }
        };
        let vbar = |items: &mut Vec<Furniture>, x: f64, y: f64| {
            let y0 = y.max(0.0);
            let y1 = (y + CROP_LEN_MM).min(ph);
            if y1 - y0 > 0.5 {
                items.push(filled(
                    x - CROP_WIDTH_MM / 2.0,
                    y0,
                    CROP_WIDTH_MM,
                    y1 - y0,
                    ink::RULE,
                ));
            }
        };
        let out_l = d.x - CROP_GAP_MM - CROP_LEN_MM;
        let out_t = d.y - CROP_GAP_MM - CROP_LEN_MM;
        let out_r = x2 + CROP_GAP_MM;
        let out_b = y2 + CROP_GAP_MM;
        hbar(&mut items, out_l, d.y);
        vbar(&mut items, d.x, out_t);
        vbar(&mut items, x2, out_t);
        if !tile.overlap_right {
            hbar(&mut items, out_r, d.y);
        }
        hbar(&mut items, out_l, y2);
        if !tile.overlap_bottom {
            vbar(&mut items, d.x, out_b);
        }
        if !tile.overlap_right {
            hbar(&mut items, out_r, y2);
        }
        // With a right band the dashed trim line already defines that edge,
        // and a bar there would land in the title strip's page line.
        if !tile.overlap_bottom && !tile.overlap_right {
            vbar(&mut items, x2, out_b);
        }
        // Trim lines at the drawing edge on sides with a neighbour, spanning
        // the drawing plus the band; "✂ trim" at each line's start.
        if tile.right.is_some() {
            items.push(Furniture::Line {
                x1: x2,
                y1: d.y,
                x2,
                y2: y2 + band_b,
                width_mm: TRIM_WIDTH_MM,
                gray: ink::RULE,
                dash: Some(vec![2.0, 1.5]),
            });
            items.push(Furniture::Text {
                x: x2 + 0.8 + TRIM_CAPTION_MM * 0.25,
                y: d.y + 1.3,
                size_mm: TRIM_CAPTION_MM,
                text: "✂ trim".to_string(),
                gray: ink::SECONDARY,
                bold: false,
                align: TextAlign::Left,
                rotate_deg: 90.0,
            });
        }
        if tile.down.is_some() {
            items.push(Furniture::Line {
                x1: d.x,
                y1: y2,
                x2: x2 + band_r,
                y2,
                width_mm: TRIM_WIDTH_MM,
                gray: ink::RULE,
                dash: Some(vec![2.0, 1.5]),
            });
            items.push(text(
                d.x + 1.3,
                baseline(y2 + 0.8, TRIM_CAPTION_MM),
                "✂ trim",
                TRIM_CAPTION_MM,
                TextAlign::Left,
                ink::SECONDARY,
            ));
        }
        // Neighbour labels 2 mm inside the drawing edge, mid-edge.
        let ls = NEIGHBOR_LABEL_MM;
        let mid_y = d.y + d.h / 2.0 + ls * 0.35;
        let mid_x = d.x + d.w / 2.0;
        if let Some(r) = &tile.right {
            items.push(text(
                x2 - NEIGHBOR_INSET_MM,
                mid_y,
                &format!("→ {r}"),
                ls,
                TextAlign::Right,
                ink::TERTIARY,
            ));
        }
        if let Some(l) = &tile.left {
            items.push(text(
                d.x + NEIGHBOR_INSET_MM,
                mid_y,
                &format!("← {l}"),
                ls,
                TextAlign::Left,
                ink::TERTIARY,
            ));
        }
        if let Some(u) = &tile.up {
            items.push(text(
                mid_x,
                baseline(d.y + NEIGHBOR_INSET_MM - 0.7, ls),
                &format!("↑ {u}"),
                ls,
                TextAlign::Center,
                ink::TERTIARY,
            ));
        }
        if let Some(dn) = &tile.down {
            items.push(text(
                mid_x,
                y2 - NEIGHBOR_INSET_MM - 0.3,
                &format!("↓ {dn}"),
                ls,
                TextAlign::Center,
                ink::TERTIARY,
            ));
        }
    }

    if title_on {
        let page_text = if tiled {
            format!("Page {} of {} · Tile {}", tile.page + 1, total, tile.id)
        } else {
            "Page 1 of 1".to_string()
        };
        let scaled = layout.scaled;
        let content = TitleBlockContent {
            document_name: &ctx.document_name,
            subtitle: &ctx.subtitle,
            // Standard pages have no scale and say nothing about it.
            center_main: if scaled {
                ctx.scale_text.as_deref()
            } else {
                None
            },
            center_sub: None,
            show_scale_bar: scaled && ctx.scale_bar,
            tile_id: if scaled && tiled {
                Some(&tile.id)
            } else {
                None
            },
            page_text: &page_text,
            date_text: &ctx.date_text,
            inset_mm: if ctx.marks && scaled {
                CROP_WIDTH_MM + 1.5
            } else {
                0.0
            },
        };
        items.extend(title_block_items(layout, &content));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> LayoutInput {
        LayoutInput {
            scaled: true,
            paper: Paper::named("letter").unwrap(),
            orientation: Orientation::Portrait,
            margin_mm: 12.7,
            title_block_mm: 10.0,
            overlap_mm: 10.0,
            dpi: 300,
            viewport_aspect: 4.0 / 3.0,
            ratio: 1.0,
            extent: RectM {
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 0.45,
            },
            scale_bar_metric: Some(false),
        }
    }

    /// The same cases `layout.test.ts` pins — the two engines must agree.
    #[test]
    fn matches_the_app_layout_fixtures() {
        let l = layout(&base());
        // 500 × 450 mm at 1:1 on Letter portrait: the 10 mm band is reserved
        // inside the printable area (decision #12), so tiles step 180.5 × 234.
        assert_eq!((l.cols, l.rows), (3, 2));
        assert!((l.page.drawing.w - 180.5).abs() < 1e-9);
        assert!((l.page.drawing.h - 234.0).abs() < 1e-9);
        assert_eq!(l.overlap_mm, 10.0);
        assert_eq!(
            l.tiles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["A1", "A2", "A3", "B1", "B2", "B3"]
        );
        let a1 = &l.tiles[0];
        assert!(a1.overlap_right && a1.overlap_bottom);
        assert!((a1.image_rect_mm.w - (180.5 + 10.0)).abs() < 0.1);
        assert_eq!(a1.image_px.0, mm_to_px(190.5, 300));
        // The last column leaves its band empty.
        let a3 = &l.tiles[2];
        assert!(!a3.overlap_right);
        assert!((a3.image_rect_mm.w - 180.5).abs() < 0.1);
        // px-exact: model rect width == image mm / 1000 at 1:1
        assert!((a1.model_rect.unwrap().w * 1000.0 - a1.image_rect_mm.w).abs() < 1e-9);
        // A2 starts one drawing width to the right; B1 one drawing height down.
        let a2 = l.tiles[1].model_rect.unwrap();
        assert!((a2.x - a1.model_rect.unwrap().x - 180.5 / 1000.0).abs() < 1e-9);
        let b1 = l.tiles[3].model_rect.unwrap();
        let a1m = a1.model_rect.unwrap();
        assert!(((a1m.y + a1m.h) - (b1.y + b1.h) - 234.0 / 1000.0).abs() < 1e-9);
        // Centred: grid centre == extent centre.
        let grid_w = 3.0 * 180.5 / 1000.0;
        assert!((a1m.x + grid_w / 2.0 - 0.25).abs() < 1e-9);
        assert_eq!(l.tiles[0].right.as_deref(), Some("A2"));
        assert_eq!(l.tiles[4].left.as_deref(), Some("B1"));
        // 500 × 300 slab: 3 × 2 either way with the band; auto keeps portrait
        // on the tie. Overlap off: 2 × 2 landscape wins.
        let slab = LayoutInput {
            extent: RectM {
                x: -0.25,
                y: -0.15,
                w: 0.5,
                h: 0.3,
            },
            ..base()
        };
        assert_eq!(
            {
                let p = layout(&slab);
                (p.cols, p.rows)
            },
            (3, 2)
        );
        let land = layout(&LayoutInput {
            orientation: Orientation::Landscape,
            ..slab.clone()
        });
        assert_eq!((land.cols, land.rows), (3, 2));
        let auto = layout(&LayoutInput {
            orientation: Orientation::Auto,
            ..slab.clone()
        });
        assert!(!auto.page.landscape);
        let no_band = layout(&LayoutInput {
            orientation: Orientation::Auto,
            overlap_mm: 0.0,
            ..slab
        });
        assert!(no_band.page.landscape);
        assert_eq!((no_band.cols, no_band.rows), (2, 2));
        // 100 mm cube on one Letter page at 1:1: 1181 px per 100 mm, no band.
        let cube = layout(&LayoutInput {
            extent: RectM {
                x: -0.05,
                y: -0.05,
                w: 0.1,
                h: 0.1,
            },
            ..base()
        });
        assert_eq!(cube.tiles.len(), 1);
        assert_eq!(cube.overlap_mm, 0.0);
        assert!((cube.page.drawing.w - 190.5).abs() < 1e-9);
        let t = &cube.tiles[0];
        let px_per_m = t.image_px.0 as f64 / t.model_rect.unwrap().w;
        assert!((px_per_m * 0.1 - 1181.1).abs() < 0.5);
        // Graphic scale bar: imperial 1:1 on Letter portrait → ½″ segments, 2 in bar.
        let sb = cube.scale_bar.as_ref().unwrap();
        assert_eq!(sb.paper_label, "2 in");
        assert_eq!(sb.labels, ["0", "½", "1", "1½", "2 in"]);
        // Standard letterbox.
        let std = layout(&LayoutInput {
            scaled: false,
            orientation: Orientation::Auto,
            viewport_aspect: 16.0 / 9.0,
            ..base()
        });
        assert!(std.page.landscape);
        let t = &std.tiles[0];
        assert!((t.image_px.0 as f64 / t.image_px.1 as f64 - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn scale_bar_matches_the_app_cases() {
        let m = pick_scale_bar(190.5, 1.0, true);
        assert_eq!(m.segment_mm, 10.0);
        assert_eq!(m.paper_mm, 40.0);
        assert_eq!(m.labels, ["0", "1", "2", "3", "4 cm"]);
        assert_eq!(m.paper_label, "40 mm");
        assert_eq!(
            pick_scale_bar(190.5, 0.2, true).labels,
            ["0", "5", "10", "15", "20 cm"]
        );
        assert_eq!(
            pick_scale_bar(190.5, 0.01, true).labels,
            ["0", "1", "2", "3", "4 m"]
        );
        assert_eq!(
            pick_scale_bar(190.5, 0.5, true).labels,
            ["0", "2", "4", "6", "8 cm"]
        );
        assert_eq!(
            pick_scale_bar(190.5, 1.0 / 12.0, false).labels,
            ["0", "6", "12", "18", "24 in"]
        );
        assert_eq!(
            pick_scale_bar(190.5, 1.0 / 48.0, false).labels,
            ["0", "3", "6", "9", "12 ft"]
        );
        let narrow = pick_scale_bar(70.0, 1.0, true);
        assert_eq!(narrow.segment_mm, 5.0);
        // 1:10 on an A4 landscape drawing (271.6): 20 cm segments of 20 mm.
        let a4 = pick_scale_bar(271.6, 0.1, true);
        assert_eq!(a4.labels, ["0", "20", "40", "60", "80 cm"]);
        assert_eq!(a4.paper_label, "80 mm");
    }

    #[test]
    fn fit_ratio_puts_the_extent_on_one_page() {
        let input = LayoutInput {
            orientation: Orientation::Auto,
            overlap_mm: 0.0,
            ..base()
        };
        let r = fit_ratio(&input);
        assert_eq!(
            layout(&LayoutInput {
                ratio: r,
                ..input.clone()
            })
            .tiles
            .len(),
            1
        );
        assert!(
            layout(&LayoutInput {
                ratio: r * 1.01,
                ..input
            })
            .tiles
            .len()
                > 1
        );
    }

    #[test]
    fn furniture_and_labels() {
        let l = layout(&base());
        let ctx = FurnitureContext {
            document_name: "T".into(),
            subtitle: "Top view · Model".into(),
            scale_text: Some("1:1 (1 in = 1 in)".into()),
            date_text: "17 Aug 2026".into(),
            marks: true,
            title_block: true,
            scale_bar: true,
        };
        let f = page_furniture(&l, &l.tiles[0], &ctx);
        let texts: Vec<String> = f
            .iter()
            .filter_map(|i| {
                if let Furniture::Text { text, .. } = i {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(texts.iter().any(|t| t == "→ A2"));
        assert!(texts.iter().any(|t| t == "↓ B1"));
        assert!(texts.iter().any(|t| t == "A1"));
        assert!(
            texts
                .iter()
                .any(|t| t == "Page 1 of 6 · Tile A1 · 17 Aug 2026 ·")
        );
        assert!(texts.iter().any(|t| t == "HEW"));
        assert!(texts.iter().any(|t| t == "2 in"));
        assert_eq!(texts.iter().filter(|t| *t == "✂ trim").count(), 2);
        assert!(
            f.iter()
                .any(|i| matches!(i, Furniture::Text { rotate_deg, .. } if *rotate_deg == 90.0))
        );
        // Crop marks: 8 corner bars minus the four that would run into the
        // right band, the bottom band, or the title block.
        let crop = f
            .iter()
            .filter(|i| matches!(i, Furniture::Rect { w, h, .. } if (*w == CROP_LEN_MM && *h == CROP_WIDTH_MM) || (*h == CROP_LEN_MM && *w == CROP_WIDTH_MM)))
            .count();
        assert_eq!(crop, 4);
        // Standard: no marks, and no scale slot at all.
        let std = layout(&LayoutInput {
            scaled: false,
            ..base()
        });
        let sf = page_furniture(
            &std,
            &std.tiles[0],
            &FurnitureContext {
                scale_text: None,
                subtitle: "Perspective view".into(),
                ..ctx.clone()
            },
        );
        assert!(
            !sf.iter()
                .any(|i| matches!(i, Furniture::Text { text, .. } if text.contains("scale")))
        );
        assert!(
            !sf.iter()
                .any(|i| matches!(i, Furniture::Line { dash: Some(_), .. }))
        );
        assert_eq!(row_letter(26), "AA");
    }
}
