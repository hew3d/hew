//! A small, dependency-light PDF 1.4 writer (docs/design/printing.md §9b).
//!
//! Scope is exactly what a Hew page needs: images (JPEG passthrough, raw
//! RGB Flate), stroked line paths with optional dashes and a clip rectangle,
//! filled/stroked rectangles, and text in Helvetica / Helvetica-Bold
//! (standard-14 fonts, WinAnsi — no embedding; characters outside WinAnsi
//! print as `?`). Coordinates in the spec are **millimetres from the page's
//! top-left corner** — the print layout's own frame — and are converted to
//! PDF points (bottom-left origin) here, so callers never think about PDF.
//!
//! Output is deterministic for a given spec: objects are numbered in a fixed
//! order, the xref table is built from real byte offsets, and no timestamps
//! or ids are written.

use std::fmt::Write as _;

pub const PT_PER_MM: f64 = 72.0 / 25.4;

/// A rectangle in mm from the top-left of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A baseline JPEG, drawn into `rect` (blob index into `build`'s `blobs`).
    Jpeg {
        data: usize,
        w: u32,
        h: u32,
        rect: Rect,
    },
    /// Raw 8-bit RGB rows, top row first, `w*h*3` bytes.
    Rgb {
        data: usize,
        w: u32,
        h: u32,
        rect: Rect,
    },
    /// Stroked line segments `[ax, ay, bx, by]` (mm), optional dash pattern
    /// (mm), optional clip rectangle.
    Path {
        segs: Vec<[f64; 4]>,
        width_mm: f64,
        dash: Option<Vec<f64>>,
        gray: f64,
        clip: Option<Rect>,
    },
    /// Text at (x, baseline y) mm. `rotate_deg` turns the run clockwise on
    /// the page about the anchor (90 = reads top-to-bottom, glyph tops to
    /// the right — CSS `writing-mode: vertical-rl`); 0 for ordinary text.
    Text {
        x: f64,
        y: f64,
        size_mm: f64,
        bold: bool,
        text: String,
        gray: f64,
        align: Align,
        rotate_deg: f64,
    },
    /// A rectangle: stroked (`stroke_mm`), filled (`fill_gray`), or both.
    Rect {
        rect: Rect,
        stroke_mm: Option<f64>,
        fill_gray: Option<f64>,
        gray: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSpec {
    pub w_mm: f64,
    pub h_mm: f64,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PdfSpec {
    pub title: String,
    pub pages: Vec<PageSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// A blob index that `blobs` does not have.
    MissingBlob(usize),
    /// An RGB image whose blob length is not `w*h*3`.
    BadRgbLength {
        index: usize,
        expected: usize,
        actual: usize,
    },
    NoPages,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfError::MissingBlob(i) => write!(f, "pdf: blob {i} is missing"),
            PdfError::BadRgbLength {
                index,
                expected,
                actual,
            } => write!(
                f,
                "pdf: rgb blob {index} has {actual} bytes, expected {expected}"
            ),
            PdfError::NoPages => write!(f, "pdf: no pages"),
        }
    }
}

impl std::error::Error for PdfError {}

// ------------------------------------------------------------ helpers

fn fmt(v: f64) -> String {
    // PDF has no exponent notation; four decimals of a point is 3.5 µm.
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" || s.is_empty() {
        "0".to_string()
    } else {
        s
    }
}

fn gray_str(g: f64) -> String {
    fmt(g.clamp(0.0, 1.0))
}

/// WinAnsi encoding of one char, or None when it has no code.
fn winansi(c: char) -> Option<u8> {
    let u = c as u32;
    if (0x20..0x7F).contains(&u) {
        return Some(u as u8);
    }
    if (0xA0..=0xFF).contains(&u) {
        return Some(u as u8);
    }
    Some(match c {
        '€' => 0x80,
        '‚' => 0x82,
        'ƒ' => 0x83,
        '„' => 0x84,
        '…' => 0x85,
        '†' => 0x86,
        '‡' => 0x87,
        'ˆ' => 0x88,
        '‰' => 0x89,
        'Š' => 0x8A,
        '‹' => 0x8B,
        'Œ' => 0x8C,
        'Ž' => 0x8E,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '˜' => 0x98,
        '™' => 0x99,
        'š' => 0x9A,
        '›' => 0x9B,
        'œ' => 0x9C,
        'ž' => 0x9E,
        'Ÿ' => 0x9F,
        _ => return None,
    })
}

/// Common typographic characters that WinAnsi lacks, mapped to a readable
/// stand-in rather than `?` (arrows and primes are the print title block's).
fn substitute(c: char) -> Option<&'static str> {
    Some(match c {
        '→' => "->",
        '←' => "<-",
        '↑' => "^",
        '↓' => "v",
        '″' => "\"",
        '′' => "'",
        '≈' => "~",
        '≤' => "<=",
        '≥' => ">=",
        '−' => "-",
        _ => return None,
    })
}

/// Encode a string to WinAnsi bytes with substitutions; unknowns become `?`.
/// (Used for the document title, where only one font applies.)
fn encode_text(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        if let Some(b) = winansi(c) {
            out.push(b);
        } else if let Some(s) = substitute(c) {
            out.extend(s.bytes());
        } else {
            out.push(b'?');
        }
    }
    out
}

/// The core-14 fonts a text run can use. Symbol carries the arrows and
/// primes WinAnsi lacks; ZapfDingbats carries the scissors ("✂ trim").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Font {
    Regular,
    Bold,
    Symbol,
    Dingbats,
}

impl Font {
    fn resource(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
            Font::Symbol => "F3",
            Font::Dingbats => "F4",
        }
    }
}

/// Symbol-font code and advance width (1/1000 em) for the glyphs the page
/// furniture uses (Adobe Symbol AFM).
fn symbol_glyph(c: char) -> Option<(u8, u16)> {
    Some(match c {
        '←' => (0xAC, 987),
        '↑' => (0xAD, 603),
        '→' => (0xAE, 987),
        '↓' => (0xAF, 603),
        '′' => (0xA2, 247),
        '″' => (0xB2, 411),
        '≈' => (0xBB, 549),
        '≤' => (0xA3, 549),
        '≥' => (0xB3, 549),
        _ => return None,
    })
}

/// ZapfDingbats code and advance width for the glyphs the furniture uses.
fn dingbat_glyph(c: char) -> Option<(u8, u16)> {
    Some(match c {
        '✂' => (0x22, 961),
        '✁' => (0x21, 974),
        '✃' => (0x23, 974),
        '✄' => (0x24, 980),
        _ => return None,
    })
}

/// A run of same-font bytes with its advance width in 1/1000 em.
struct Run {
    font: Font,
    bytes: Vec<u8>,
    width_units: f64,
}

/// Split text into font runs: WinAnsi in Helvetica(-Bold), arrows/primes in
/// Symbol, scissors in ZapfDingbats, readable ASCII stand-ins for anything
/// else those three lack, `?` last.
fn encode_runs(text: &str, bold: bool) -> Vec<Run> {
    let base = if bold { Font::Bold } else { Font::Regular };
    let table = if bold {
        &HELVETICA_BOLD_WIDTHS
    } else {
        &HELVETICA_WIDTHS
    };
    let base_width = |b: u8| -> f64 {
        if (32..=126).contains(&b) {
            table[(b - 32) as usize] as f64
        } else {
            556.0
        }
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut push = |font: Font, bytes: &[u8], width: f64| {
        if let Some(last) = runs.last_mut().filter(|last| last.font == font) {
            last.bytes.extend_from_slice(bytes);
            last.width_units += width;
            return;
        }
        runs.push(Run {
            font,
            bytes: bytes.to_vec(),
            width_units: width,
        });
    };
    for c in text.chars() {
        if let Some(b) = winansi(c) {
            push(base, &[b], base_width(b));
        } else if let Some((code, w)) = symbol_glyph(c) {
            push(Font::Symbol, &[code], w as f64);
        } else if let Some((code, w)) = dingbat_glyph(c) {
            push(Font::Dingbats, &[code], w as f64);
        } else if let Some(sub) = substitute(c) {
            let w: f64 = sub.bytes().map(base_width).sum();
            push(base, sub.as_bytes(), w);
        } else {
            push(base, b"?", base_width(b'?'));
        }
    }
    runs
}

/// Helvetica / Helvetica-Bold advance widths (1/1000 em) for codes 32..=126,
/// from the Adobe core-14 AFMs.
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELVETICA_BOLD_WIDTHS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Width of a string in mm at a font size (mm).
pub fn text_width_mm(text: &str, size_mm: f64, bold: bool) -> f64 {
    encode_runs(text, bold)
        .iter()
        .map(|r| r.width_units)
        .sum::<f64>()
        / 1000.0
        * size_mm
}

fn pdf_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() + 2);
    s.push('(');
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(b as char);
            }
            0x20..=0x7E => s.push(b as char),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

// -------------------------------------------------------- content stream

struct PageBuild {
    content: String,
    /// XObject names → object numbers, in order.
    xobjects: Vec<(String, usize)>,
}

fn build_page(
    page: &PageSpec,
    blobs: &[&[u8]],
    image_objects: &mut Vec<(usize, Vec<u8>, String)>,
    next_obj: &mut usize,
) -> Result<PageBuild, PdfError> {
    let h = page.h_mm * PT_PER_MM;
    let pt = |mm: f64| mm * PT_PER_MM;
    // top-left mm → PDF pt (bottom-left origin)
    let px = |x_mm: f64| pt(x_mm);
    let py = |y_mm: f64| h - pt(y_mm);
    let mut c = String::new();
    let mut xobjects: Vec<(String, usize)> = Vec::new();
    for item in &page.items {
        match item {
            Item::Jpeg {
                data,
                w,
                h: ih,
                rect,
            }
            | Item::Rgb {
                data,
                w,
                h: ih,
                rect,
            } => {
                let bytes = *blobs.get(*data).ok_or(PdfError::MissingBlob(*data))?;
                let is_jpeg = matches!(item, Item::Jpeg { .. });
                if !is_jpeg {
                    let expected = (*w as usize) * (*ih as usize) * 3;
                    if bytes.len() != expected {
                        return Err(PdfError::BadRgbLength {
                            index: *data,
                            expected,
                            actual: bytes.len(),
                        });
                    }
                }
                let obj = *next_obj;
                *next_obj += 1;
                let name = format!("Im{obj}");
                let (stream, dict) = if is_jpeg {
                    (
                        bytes.to_vec(),
                        format!(
                            "/Type /XObject /Subtype /Image /Width {w} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"
                        ),
                    )
                } else {
                    (
                        deflate(bytes),
                        format!(
                            "/Type /XObject /Subtype /Image /Width {w} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"
                        ),
                    )
                };
                image_objects.push((obj, stream, dict));
                xobjects.push((name.clone(), obj));
                let _ = writeln!(
                    c,
                    "q {} 0 0 {} {} {} cm /{} Do Q",
                    fmt(pt(rect.w)),
                    fmt(pt(rect.h)),
                    fmt(px(rect.x)),
                    fmt(py(rect.y + rect.h)),
                    name
                );
            }
            Item::Path {
                segs,
                width_mm,
                dash,
                gray,
                clip,
            } => {
                if segs.is_empty() {
                    continue;
                }
                let _ = write!(c, "q ");
                if let Some(r) = clip {
                    let _ = write!(
                        c,
                        "{} {} {} {} re W n ",
                        fmt(px(r.x)),
                        fmt(py(r.y + r.h)),
                        fmt(pt(r.w)),
                        fmt(pt(r.h))
                    );
                }
                let _ = write!(c, "{} G {} w 1 J 1 j ", gray_str(*gray), fmt(pt(*width_mm)));
                match dash {
                    Some(d) if !d.is_empty() => {
                        let arr: Vec<String> = d.iter().map(|v| fmt(pt(*v))).collect();
                        let _ = write!(c, "[{}] 0 d ", arr.join(" "));
                    }
                    _ => {
                        let _ = write!(c, "[] 0 d ");
                    }
                }
                for s in segs {
                    let _ = write!(
                        c,
                        "{} {} m {} {} l ",
                        fmt(px(s[0])),
                        fmt(py(s[1])),
                        fmt(px(s[2])),
                        fmt(py(s[3]))
                    );
                }
                let _ = writeln!(c, "S Q");
            }
            Item::Rect {
                rect,
                stroke_mm,
                fill_gray,
                gray,
            } => {
                let _ = write!(c, "q ");
                let op = match (stroke_mm, fill_gray) {
                    (Some(sw), Some(fg)) => {
                        let _ = write!(
                            c,
                            "{} G {} g {} w ",
                            gray_str(*gray),
                            gray_str(*fg),
                            fmt(pt(*sw))
                        );
                        "B"
                    }
                    (Some(sw), None) => {
                        let _ = write!(c, "{} G {} w ", gray_str(*gray), fmt(pt(*sw)));
                        "S"
                    }
                    (None, Some(fg)) => {
                        let _ = write!(c, "{} g ", gray_str(*fg));
                        "f"
                    }
                    (None, None) => "n",
                };
                let _ = writeln!(
                    c,
                    "{} {} {} {} re {} Q",
                    fmt(px(rect.x)),
                    fmt(py(rect.y + rect.h)),
                    fmt(pt(rect.w)),
                    fmt(pt(rect.h)),
                    op
                );
            }
            Item::Text {
                x,
                y,
                size_mm,
                bold,
                text,
                gray,
                align,
                rotate_deg,
            } => {
                let runs = encode_runs(text, *bold);
                if runs.iter().all(|r| r.bytes.is_empty()) {
                    continue;
                }
                let width_mm = runs.iter().map(|r| r.width_units).sum::<f64>() / 1000.0 * size_mm;
                let shift = match align {
                    Align::Left => 0.0,
                    Align::Center => width_mm / 2.0,
                    Align::Right => width_mm,
                };
                let size_pt = fmt(pt(*size_mm));
                let mut shown = String::new();
                for r in &runs {
                    let _ = write!(
                        shown,
                        " /{} {} Tf {} Tj",
                        r.font.resource(),
                        size_pt,
                        pdf_string(&r.bytes)
                    );
                }
                if rotate_deg.abs() < 1e-9 {
                    let _ = writeln!(
                        c,
                        "BT {} g {} {} Td{} ET",
                        gray_str(*gray),
                        fmt(px(x - shift)),
                        fmt(py(*y)),
                        shown
                    );
                } else {
                    // Screen-space rotation (y down, clockwise positive) →
                    // PDF text matrix (y up): baseline direction (cos r,
                    // -sin r), glyph-up direction (sin r, cos r). The align
                    // shift moves the start back along the baseline.
                    let r = rotate_deg.to_radians();
                    let (sr, cr) = r.sin_cos();
                    let x0 = x - shift * cr;
                    let y0 = y - shift * sr;
                    let _ = writeln!(
                        c,
                        "BT {} g {} {} {} {} {} {} Tm{} ET",
                        gray_str(*gray),
                        fmt(cr),
                        fmt(-sr),
                        fmt(sr),
                        fmt(cr),
                        fmt(px(x0)),
                        fmt(py(y0)),
                        shown
                    );
                }
            }
        }
    }
    Ok(PageBuild {
        content: c,
        xobjects,
    })
}

/// Build the PDF bytes. `blobs[i]` backs `Item::Jpeg`/`Item::Rgb` with
/// `data == i`.
pub fn build(spec: &PdfSpec, blobs: &[&[u8]]) -> Result<Vec<u8>, PdfError> {
    if spec.pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    // Object numbering: 1 catalog, 2 pages, 3 info, 4 Helvetica, 5 Helvetica-Bold,
    // 6 Symbol, 7 ZapfDingbats, then per page: page object, content stream,
    // then image objects.
    let mut objects: Vec<(usize, Vec<u8>)> = Vec::new(); // (obj number, full body incl. "N 0 obj ... endobj")
    let mut next_obj = 8usize;
    let mut page_obj_numbers: Vec<usize> = Vec::new();

    let mut page_bodies: Vec<(usize, Vec<u8>)> = Vec::new();
    for page in &spec.pages {
        let page_obj = next_obj;
        next_obj += 1;
        let content_obj = next_obj;
        next_obj += 1;
        let mut image_objects: Vec<(usize, Vec<u8>, String)> = Vec::new();
        let built = build_page(page, blobs, &mut image_objects, &mut next_obj)?;
        let compressed = deflate(built.content.as_bytes());
        let mut xo = String::new();
        for (name, num) in &built.xobjects {
            let _ = write!(xo, "/{name} {num} 0 R ");
        }
        let w_pt = fmt(page.w_mm * PT_PER_MM);
        let h_pt = fmt(page.h_mm * PT_PER_MM);
        let page_dict = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w_pt} {h_pt}] /Contents {content_obj} 0 R /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R /F4 7 0 R >> /XObject << {xo}>> /ProcSet [/PDF /Text /ImageC] >> >>"
        );
        page_bodies.push((page_obj, obj_body(page_obj, page_dict.as_bytes(), None)));
        page_bodies.push((
            content_obj,
            obj_body(
                content_obj,
                format!("<< /Length {} /Filter /FlateDecode >>", compressed.len()).as_bytes(),
                Some(&compressed),
            ),
        ));
        for (num, stream, dict) in image_objects {
            page_bodies.push((
                num,
                obj_body(
                    num,
                    format!("<< {dict} /Length {} >>", stream.len()).as_bytes(),
                    Some(&stream),
                ),
            ));
        }
        page_obj_numbers.push(page_obj);
    }

    let kids: Vec<String> = page_obj_numbers
        .iter()
        .map(|n| format!("{n} 0 R"))
        .collect();
    objects.push((1, obj_body(1, b"<< /Type /Catalog /Pages 2 0 R >>", None)));
    objects.push((
        2,
        obj_body(
            2,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                kids.len()
            )
            .as_bytes(),
            None,
        ),
    ));
    let title = pdf_string(&encode_text(&spec.title));
    objects.push((
        3,
        obj_body(
            3,
            format!("<< /Title {title} /Producer (Hew) /Creator (Hew) >>").as_bytes(),
            None,
        ),
    ));
    objects.push((
        4,
        obj_body(
            4,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
            None,
        ),
    ));
    objects.push((5, obj_body(5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>", None)));
    // Symbol and ZapfDingbats use their built-in encodings (no /Encoding).
    objects.push((
        6,
        obj_body(
            6,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>",
            None,
        ),
    ));
    objects.push((
        7,
        obj_body(
            7,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>",
            None,
        ),
    ));
    objects.extend(page_bodies);
    objects.sort_by_key(|(n, _)| *n);

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let count = objects.len() + 1;
    let mut offsets = vec![0usize; count];
    for (n, body) in &objects {
        offsets[*n] = out.len();
        out.extend_from_slice(body);
    }
    let xref_at = out.len();
    let mut xref = format!("xref\n0 {count}\n0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        let _ = writeln!(xref, "{off:010} 00000 n ");
    }
    // Trailing space + newline per entry: exactly 20 bytes ("nnnnnnnnnn ggggg n \n").
    xref = xref.replace(" \n", " \r\n").replace(" \r\n", " \n");
    out.extend_from_slice(xref.as_bytes());
    let trailer = format!(
        "trailer\n<< /Size {count} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref_at}\n%%EOF\n"
    );
    out.extend_from_slice(trailer.as_bytes());
    Ok(out)
}

fn obj_body(num: usize, dict: &[u8], stream: Option<&[u8]>) -> Vec<u8> {
    let mut b = Vec::with_capacity(dict.len() + stream.map_or(0, |s| s.len()) + 32);
    b.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
    b.extend_from_slice(dict);
    b.push(b'\n');
    if let Some(s) = stream {
        b.extend_from_slice(b"stream\n");
        b.extend_from_slice(s);
        b.extend_from_slice(b"\nendstream\n");
    }
    b.extend_from_slice(b"endobj\n");
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_page(items: Vec<Item>) -> PdfSpec {
        PdfSpec {
            title: "Test — “quotes” → arrows".into(),
            pages: vec![PageSpec {
                w_mm: 215.9,
                h_mm: 279.4,
                items,
            }],
        }
    }

    #[test]
    fn structure_is_well_formed_and_deterministic() {
        let spec = one_page(vec![
            Item::Rect {
                rect: Rect {
                    x: 10.0,
                    y: 10.0,
                    w: 100.0,
                    h: 100.0,
                },
                stroke_mm: Some(0.35),
                fill_gray: None,
                gray: 0.0,
            },
            Item::Path {
                segs: vec![[0.0, 0.0, 10.0, 10.0]],
                width_mm: 0.2,
                dash: Some(vec![1.5, 1.0]),
                gray: 0.35,
                clip: Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 50.0,
                    h: 50.0,
                }),
            },
            Item::Text {
                x: 100.0,
                y: 200.0,
                size_mm: 3.0,
                bold: true,
                text: "Page 1 of 6 · Tile B2".into(),
                gray: 0.0,
                align: Align::Center,
                rotate_deg: 0.0,
            },
        ]);
        let a = build(&spec, &[]).unwrap();
        let b = build(&spec, &[]).unwrap();
        assert_eq!(a, b);
        let text = String::from_utf8_lossy(&a);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Type /Catalog"));
        assert!(text.contains("/MediaBox [0 0 612 792]"));
        assert!(text.contains("/Count 1"));
        assert!(text.ends_with("%%EOF\n"));
        // xref offsets are BYTE offsets that land on "N 0 obj" (the lossy
        // string above is only for the ASCII checks — streams are binary).
        let tail = String::from_utf8_lossy(&a[a.len() - 64..]).to_string();
        let xref_at: usize = tail
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .trim_end_matches("\n%%EOF\n")
            .trim()
            .parse()
            .unwrap();
        assert!(a[xref_at..].starts_with(b"xref"));
        let table = String::from_utf8_lossy(&a[xref_at..]).to_string();
        for (i, line) in table.lines().skip(2).enumerate() {
            if line.starts_with("trailer") {
                break;
            }
            if i == 0 {
                continue; // the free entry
            }
            let off: usize = line[..10].parse().unwrap();
            let head = format!("{i} 0 obj");
            assert!(a[off..].starts_with(head.as_bytes()), "object {i} at {off}");
        }
    }

    #[test]
    fn images_need_matching_blobs() {
        let spec = one_page(vec![Item::Rgb {
            data: 0,
            w: 2,
            h: 2,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        }]);
        assert_eq!(build(&spec, &[]).unwrap_err(), PdfError::MissingBlob(0));
        assert!(matches!(
            build(&spec, &[&[0u8; 5]]).unwrap_err(),
            PdfError::BadRgbLength { .. }
        ));
        let ok = build(&spec, &[&[255u8; 12]]).unwrap();
        assert!(String::from_utf8_lossy(&ok).contains("/Filter /FlateDecode"));
        let jpeg = one_page(vec![Item::Jpeg {
            data: 0,
            w: 2,
            h: 2,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
        }]);
        let ok = build(&jpeg, &[b"\xFF\xD8fake"]).unwrap();
        assert!(String::from_utf8_lossy(&ok).contains("/Filter /DCTDecode"));
    }

    #[test]
    fn text_encodes_winansi_with_substitutions() {
        assert_eq!(encode_text("½ × é"), vec![0xBD, b' ', 0xD7, b' ', 0xE9]);
        assert_eq!(encode_text("→ B3"), b"-> B3".to_vec());
        assert_eq!(encode_text("日本"), b"??".to_vec());
        // Furniture runs: arrows go to Symbol, scissors to ZapfDingbats,
        // the rest stays Helvetica; adjacent same-font chars merge.
        let runs = encode_runs("→ B3", false);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            (runs[0].font, runs[0].bytes.as_slice()),
            (Font::Symbol, &[0xAEu8][..])
        );
        assert_eq!(
            (runs[1].font, runs[1].bytes.as_slice()),
            (Font::Regular, &b" B3"[..])
        );
        let runs = encode_runs("✂ trim", true);
        assert_eq!(runs[0].font, Font::Dingbats);
        assert_eq!(runs[0].bytes, vec![0x22]);
        assert_eq!(runs[1].font, Font::Bold);
        // 1′-0″ keeps its primes as Symbol glyphs, and the width counts them.
        let w = text_width_mm("1′", 1.0, false);
        assert!((w - (556.0 + 247.0) / 1000.0).abs() < 1e-9);
        assert_eq!(pdf_string(b"a(b)\\"), "(a\\(b\\)\\\\)");
        // Helvetica "Hew" = H 722 + e 556 + w 722 = 2000 units → 2 mm at 1 mm.
        assert!((text_width_mm("Hew", 1.0, false) - 2.0).abs() < 1e-9);
    }

    /// `PDFWRITE_DUMP=/path/out.pdf cargo test -p pdfwrite -- --ignored` writes
    /// a sample for an external validator (`qpdf --check`, a viewer).
    #[test]
    #[ignore]
    fn dump_sample() {
        let Ok(path) = std::env::var("PDFWRITE_DUMP") else {
            return;
        };
        let mut rgb = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                let v = if (x / 8 + y / 8) % 2 == 0 {
                    40u8
                } else {
                    220u8
                };
                rgb.extend_from_slice(&[v, v, v]);
            }
        }
        let spec = PdfSpec {
            title: "pdfwrite sample".into(),
            pages: vec![
                PageSpec {
                    w_mm: 215.9,
                    h_mm: 279.4,
                    items: vec![
                        Item::Rgb {
                            data: 0,
                            w: 64,
                            h: 64,
                            rect: Rect {
                                x: 20.0,
                                y: 20.0,
                                w: 60.0,
                                h: 60.0,
                            },
                        },
                        Item::Rect {
                            rect: Rect {
                                x: 12.7,
                                y: 12.7,
                                w: 190.5,
                                h: 241.0,
                            },
                            stroke_mm: Some(0.2),
                            fill_gray: None,
                            gray: 0.35,
                        },
                        Item::Path {
                            segs: vec![[57.95, 100.0, 157.95, 100.0], [57.95, 100.0, 57.95, 200.0]],
                            width_mm: 0.35,
                            dash: None,
                            gray: 0.0,
                            clip: None,
                        },
                        Item::Path {
                            segs: vec![[57.95, 200.0, 157.95, 200.0]],
                            width_mm: 0.25,
                            dash: Some(vec![1.5, 1.0]),
                            gray: 0.0,
                            clip: None,
                        },
                        Item::Text {
                            x: 107.95,
                            y: 260.0,
                            size_mm: 3.0,
                            bold: true,
                            text: "1:1 — 100 mm square, → arrows, ½ in".into(),
                            gray: 0.0,
                            align: Align::Center,
                            rotate_deg: 0.0,
                        },
                        Item::Text {
                            x: 12.7,
                            y: 270.0,
                            size_mm: 2.4,
                            bold: false,
                            text: "Page 1 of 1 · Hew".into(),
                            gray: 0.35,
                            align: Align::Left,
                            rotate_deg: 0.0,
                        },
                    ],
                },
                PageSpec {
                    w_mm: 210.0,
                    h_mm: 297.0,
                    items: vec![Item::Text {
                        x: 105.0,
                        y: 148.5,
                        size_mm: 5.0,
                        bold: false,
                        text: "A4 second page".into(),
                        gray: 0.0,
                        align: Align::Center,
                        rotate_deg: 0.0,
                    }],
                },
            ],
        };
        std::fs::write(path, build(&spec, &[&rgb]).unwrap()).unwrap();
    }

    #[test]
    fn no_pages_is_an_error() {
        assert_eq!(
            build(&PdfSpec::default(), &[]).unwrap_err(),
            PdfError::NoPages
        );
    }
}
