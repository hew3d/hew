//! Headless printing for `CliHost` (docs/design/printing.md §9b):
//! `hew.view.line_drawing` through `crates/hlr`, and `hew.print.pdf` through
//! `api::print_layout` + `hlr` (line art) / `softrender` (shaded) +
//! `pdfwrite`. Mirrors the app's print pipeline page for page: the same
//! layout numbers, the same furniture, the same stroke weights.

use api::print_layout::{
    self as pl, Furniture, FurnitureContext, LayoutInput, Orientation, Paper, RectM,
};
use api::{PrintPdfParams, PrintPdfResult, Refusal};
use headless_render::{
    basis, hlr_error, hlr_section, projected_extent, resolve_view, scene_and_hidden,
};
use kernel::{Document, Point3};
use std::collections::BTreeMap;

/// Print stroke weights (mm) — the app's `LINE_ART_WEIGHTS_MM`.
const HARD_MM: f64 = 0.35;
const SOFT_MM: f64 = 0.18;
const HIDDEN_MM: f64 = 0.25;
const SECTION_MM: f64 = 0.5;
const PRINT_DPI: u32 = 300;

fn furniture_to_pdf(items: &[Furniture]) -> Vec<pdfwrite::Item> {
    items
        .iter()
        .map(|f| match f {
            Furniture::Line {
                x1,
                y1,
                x2,
                y2,
                width_mm,
                gray,
                dash,
            } => pdfwrite::Item::Path {
                segs: vec![[*x1, *y1, *x2, *y2]],
                width_mm: *width_mm,
                dash: dash.clone(),
                gray: *gray,
                clip: None,
            },
            Furniture::Text {
                x,
                y,
                size_mm,
                text,
                gray,
                bold,
                align,
                rotate_deg,
            } => pdfwrite::Item::Text {
                x: *x,
                y: *y,
                size_mm: *size_mm,
                bold: *bold,
                text: text.clone(),
                gray: *gray,
                align: match align {
                    pl::TextAlign::Left => pdfwrite::Align::Left,
                    pl::TextAlign::Center => pdfwrite::Align::Center,
                    pl::TextAlign::Right => pdfwrite::Align::Right,
                },
                rotate_deg: *rotate_deg,
            },
            Furniture::Rect {
                x,
                y,
                w,
                h,
                stroke_mm,
                fill_gray,
                gray,
            } => pdfwrite::Item::Rect {
                rect: pdfwrite::Rect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                },
                stroke_mm: *stroke_mm,
                fill_gray: *fill_gray,
                gray: *gray,
            },
        })
        .collect()
}

pub fn print_pdf(
    doc: &Document,
    params: &PrintPdfParams,
    document_name: &str,
) -> Result<PrintPdfResult, Refusal> {
    let (scene, hidden) = scene_and_hidden(doc, params.scene)?;
    let items = softrender::document_items_hiding(doc, &hidden);
    if items.is_empty() {
        return Err(Refusal::api(
            "nothing_to_render",
            "This document has no visible solids to print. Check that something is unhidden and un-tagged-away.",
        ));
    }
    let bbox = softrender::document_bbox_hiding(doc, &hidden);
    let view = resolve_view(doc, &params.camera, params.view, scene.as_ref(), bbox)?;
    let (right, upv) = basis(view.dir, view.up);
    let bbox_center = Point3::new(
        (bbox.0.x + bbox.1.x) / 2.0,
        (bbox.0.y + bbox.1.y) / 2.0,
        (bbox.0.z + bbox.1.z) / 2.0,
    );
    let (ext0, depth0) = projected_extent(bbox, bbox_center, view.dir, right, upv);
    // Re-centre so the extent is symmetric about the print camera centre.
    let cx = ext0.x + ext0.w / 2.0;
    let cy = ext0.y + ext0.h / 2.0;
    let cd = (depth0.0 + depth0.1) / 2.0;
    let center = bbox_center + right * cx + upv * cy + view.dir * cd;
    let extent = RectM {
        x: -ext0.w / 2.0,
        y: -ext0.h / 2.0,
        w: ext0.w,
        h: ext0.h,
    };
    let depth = (depth0.0 - cd, depth0.1 - cd);

    let paper = Paper {
        w: params.paper_w_mm,
        h: params.paper_h_mm,
    };
    let input = LayoutInput {
        scaled: params.scaled,
        paper,
        orientation: match params.landscape {
            None => Orientation::Auto,
            Some(false) => Orientation::Portrait,
            Some(true) => Orientation::Landscape,
        },
        margin_mm: params.margin_mm,
        title_block_mm: if params.title_block {
            pl::DEFAULT_TITLE_BLOCK_MM
        } else {
            0.0
        },
        overlap_mm: params.overlap_mm,
        dpi: PRINT_DPI,
        // Headless Standard mode has no viewport: frame the extent's aspect.
        viewport_aspect: extent.w / extent.h,
        ratio: params.ratio,
        extent,
        scale_bar_metric: if params.scale_bar {
            Some(params.metric)
        } else {
            None
        },
    };
    let layout = pl::layout(&input);
    let title = params
        .title
        .clone()
        .unwrap_or_else(|| document_name.to_string());
    let view_label = match params.view {
        Some(v) => format!("{v:?} view"),
        None => "Current view".to_string(),
    };
    let ctx = FurnitureContext {
        document_name: title.clone(),
        subtitle: if params.scaled {
            format!("{view_label} · Model")
        } else if view.perspective {
            "Perspective view".to_string()
        } else {
            "Parallel view".to_string()
        },
        scale_text: if params.scaled {
            Some(params.scale_label.clone())
        } else {
            None
        },
        date_text: String::new(),
        marks: params.marks,
        title_block: params.title_block,
        scale_bar: params.scale_bar,
    };

    let hlr_items: Vec<hlr::Item> = items
        .iter()
        .map(|it| hlr::Item {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    let render_items: Vec<softrender::RenderItem> = items
        .iter()
        .map(|it| softrender::RenderItem {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    let section = hlr_section(doc, scene.as_ref());
    let range = (depth.1 - depth.0).max(1e-3);
    let pad = (range * 0.1).max(0.05);
    let eye = center + view.dir * (depth.0 - pad);

    // One drawing for every tile (Scaled), or the single Standard page.
    let drawing = if params.line_art {
        let camera = if params.scaled {
            hlr::Camera {
                eye,
                target: center,
                up: view.up,
                projection: hlr::Projection::Parallel,
            }
        } else {
            hlr::Camera {
                eye: view.eye,
                target: view.target,
                up: view.up,
                projection: if view.perspective {
                    hlr::Projection::Perspective
                } else {
                    hlr::Projection::Parallel
                },
            }
        };
        Some(
            hlr::line_drawing(
                &hlr_items,
                &camera,
                &hlr::Options {
                    section,
                    include_hidden: params.include_hidden,
                    include_soft: false,
                    budget: hlr::DEFAULT_BUDGET,
                },
            )
            .map_err(hlr_error)?,
        )
    } else {
        None
    };

    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let mut pages: Vec<pdfwrite::PageSpec> = Vec::with_capacity(layout.tiles.len());
    for tile in &layout.tiles {
        let mut items_pdf: Vec<pdfwrite::Item> = Vec::new();
        let img = tile.image_rect_mm;
        let clip = pdfwrite::Rect {
            x: img.x,
            y: img.y,
            w: img.w,
            h: img.h,
        };
        match (&drawing, tile.model_rect) {
            (Some(d), Some(mr)) => {
                // Scaled vector: view-plane metres → page mm through the tile's model rect.
                let k = params.ratio * 1000.0;
                push_vector(&mut items_pdf, d, params.include_hidden, clip, |x, y| {
                    (img.x + (x - mr.x) * k, img.y + (mr.y + mr.h - y) * k)
                });
            }
            (Some(d), None) => {
                // Standard vector: fit the drawing's bounds into the image rect.
                if let Some((mn, mx)) = d.bounds {
                    let w = (mx[0] - mn[0]).max(1e-9);
                    let h = (mx[1] - mn[1]).max(1e-9);
                    let k = (img.w / w).min(img.h / h);
                    let ox = img.x + (img.w - w * k) / 2.0;
                    let oy = img.y + (img.h - h * k) / 2.0;
                    push_vector(&mut items_pdf, d, params.include_hidden, clip, |x, y| {
                        (ox + (x - mn[0]) * k, oy + (mx[1] - y) * k)
                    });
                }
            }
            (None, model_rect) => {
                // Shaded raster through softrender.
                let (w_px, h_px) = tile.image_px;
                let camera = match model_rect {
                    Some(mr) => {
                        let tile_center =
                            center + right * (mr.x + mr.w / 2.0) + upv * (mr.y + mr.h / 2.0);
                        softrender::Camera {
                            eye: tile_center + view.dir * (depth.0 - pad),
                            target: tile_center,
                            up: view.up,
                            projection: softrender::Projection::Parallel {
                                half_height: mr.h / 2.0,
                            },
                        }
                    }
                    None => {
                        if view.perspective {
                            softrender::Camera {
                                eye: view.eye,
                                target: view.target,
                                up: view.up,
                                projection: softrender::Projection::Perspective {
                                    fov_y_deg: view.fov_deg,
                                },
                            }
                        } else {
                            softrender::Camera {
                                eye,
                                target: center,
                                up: view.up,
                                projection: softrender::Projection::Parallel {
                                    half_height: extent.h / 2.0 * 1.05,
                                },
                            }
                        }
                    }
                };
                let rendered = softrender::render(&render_items, &camera, w_px.max(1), h_px.max(1))
                    .map_err(|e| Refusal::api("too_many_objects", &e.to_string()))?;
                // Paper is white: softrender's own background tint becomes white.
                let mut rgb = Vec::with_capacity((rendered.width * rendered.height * 3) as usize);
                for px in rendered.rgba.chunks_exact(4) {
                    if px[..4] == softrender::BACKGROUND_RGBA {
                        rgb.extend_from_slice(&[255, 255, 255]);
                    } else {
                        rgb.extend_from_slice(&px[..3]);
                    }
                }
                blobs.push(rgb);
                items_pdf.push(pdfwrite::Item::Rgb {
                    data: blobs.len() - 1,
                    w: rendered.width,
                    h: rendered.height,
                    rect: clip,
                });
            }
        }
        items_pdf.extend(furniture_to_pdf(&pl::page_furniture(&layout, tile, &ctx)));
        pages.push(pdfwrite::PageSpec {
            w_mm: layout.page.paper_w,
            h_mm: layout.page.paper_h,
            items: items_pdf,
        });
    }
    let refs: Vec<&[u8]> = blobs.iter().map(|b| b.as_slice()).collect();
    let pdf = pdfwrite::build(&pdfwrite::PdfSpec { title, pages }, &refs)
        .map_err(|e| Refusal::api("save_failed", &e.to_string()))?;
    Ok(PrintPdfResult {
        pdf,
        pages: layout.tiles.len(),
        cols: layout.cols,
        rows: layout.rows,
    })
}

fn push_vector(
    out: &mut Vec<pdfwrite::Item>,
    d: &hlr::LineDrawing,
    include_hidden: bool,
    clip: pdfwrite::Rect,
    to_mm: impl Fn(f64, f64) -> (f64, f64),
) {
    let mut by_kind: BTreeMap<u8, Vec<[f64; 4]>> = BTreeMap::new();
    for s in &d.segs {
        let k = match s.kind {
            hlr::Kind::Hidden if !include_hidden => continue,
            hlr::Kind::Hidden => 4,
            hlr::Kind::Soft => 2,
            hlr::Kind::Hard => 0,
            hlr::Kind::Silhouette => 1,
            hlr::Kind::Section => 3,
        };
        let a = to_mm(s.a[0], s.a[1]);
        let b = to_mm(s.b[0], s.b[1]);
        // Cheap cull: keep segments touching the clip rect (with a hair).
        let (minx, maxx) = (a.0.min(b.0), a.0.max(b.0));
        let (miny, maxy) = (a.1.min(b.1), a.1.max(b.1));
        if maxx < clip.x - 1.0
            || minx > clip.x + clip.w + 1.0
            || maxy < clip.y - 1.0
            || miny > clip.y + clip.h + 1.0
        {
            continue;
        }
        by_kind.entry(k).or_default().push([a.0, a.1, b.0, b.1]);
    }
    for (k, width, dash) in [
        (4u8, HIDDEN_MM, Some(vec![1.5, 1.0])),
        (2, SOFT_MM, None),
        (0, HARD_MM, None),
        (1, HARD_MM, None),
        (3, SECTION_MM, None),
    ] {
        if let Some(segs) = by_kind.remove(&k) {
            out.push(pdfwrite::Item::Path {
                segs,
                width_mm: width,
                dash,
                gray: 0.0,
                clip: Some(clip),
            });
        }
    }
}
