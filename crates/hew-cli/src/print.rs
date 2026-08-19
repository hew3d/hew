//! Headless printing for `CliHost` (docs/design/printing.md §9b):
//! `hew.view.line_drawing` through `crates/hlr`, and `hew.print.pdf` through
//! `api::print_layout` + `hlr` (line art) / `softrender` (shaded) +
//! `pdfwrite`. Mirrors the app's print pipeline page for page: the same
//! layout numbers, the same furniture, the same stroke weights.

use api::print_layout::{
    self as pl, Furniture, FurnitureContext, LayoutInput, Orientation, Paper, RectM,
};
use api::{
    LineDrawingFormat, LineDrawingParams, LineDrawingResult, PrintPdfParams, PrintPdfResult,
    Refusal, SnapshotCamera, SnapshotProjection, StandardView,
};
use kernel::{Document, EntityRef, Point3, Vec3};
use std::collections::BTreeMap;

/// Print stroke weights (mm) — the app's `LINE_ART_WEIGHTS_MM`.
const HARD_MM: f64 = 0.35;
const SOFT_MM: f64 = 0.18;
const HIDDEN_MM: f64 = 0.25;
const SECTION_MM: f64 = 0.5;
const PRINT_DPI: u32 = 300;

/// A resolved print view: unit direction (eye → target), up, and whether
/// the request was perspective.
struct View {
    dir: Vec3,
    up: Vec3,
    perspective: bool,
    /// Explicit eye/target when the caller gave a camera (Standard mode
    /// keeps them; Scaled only keeps the direction).
    eye: Point3,
    target: Point3,
    fov_deg: f64,
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// The view-plane basis `hlr::Frame` and the app's `viewPlaneBasis` use.
fn basis(dir: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let mut right = cross(dir, up);
    if right.length() < 1e-9 {
        // Same degenerate rule as hlr::Frame / the app's print pass.
        let seed = if dir.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        right = seed - dir * dot(seed, dir);
    }
    let right = right * (1.0 / right.length());
    let upv = cross(right, dir);
    (right, upv)
}

fn resolve_view(
    doc: &Document,
    camera: &Option<SnapshotCamera>,
    view: Option<StandardView>,
    scene: Option<&kernel::ResolvedScene>,
    bbox: (Point3, Point3),
) -> Result<View, Refusal> {
    let from_points = |eye: Point3,
                       target: Point3,
                       up: Vec3,
                       perspective: bool,
                       fov: f64|
     -> Result<View, Refusal> {
        let d = target - eye;
        if d.length() < 1e-12 {
            return Err(Refusal::api(
                "nothing_to_render",
                "The camera's eye sits on its target.",
            ));
        }
        let dir = d * (1.0 / d.length());
        Ok(View {
            dir,
            up,
            perspective,
            eye,
            target,
            fov_deg: fov,
        })
    };
    if let Some(cam) = camera {
        let eye = Point3::new(cam.eye[0], cam.eye[1], cam.eye[2]);
        let target = Point3::new(cam.target[0], cam.target[1], cam.target[2]);
        let up = cam
            .up
            .map(|u| Vec3::new(u[0], u[1], u[2]))
            .unwrap_or(Vec3::new(0.0, 0.0, 1.0));
        let perspective = !matches!(cam.projection, Some(SnapshotProjection::Parallel));
        return from_points(eye, target, up, perspective, cam.fov_deg.unwrap_or(35.0));
    }
    if let Some(v) = view {
        // Standard views: the app's print directions (world up +Z; top and
        // bottom are exact axis views — the basis rule reads x → right).
        let (eye_dir, up): ([f64; 3], [f64; 3]) = match v {
            StandardView::Top => ([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
            StandardView::Bottom => ([0.0, 0.0, -1.0], [0.0, 0.0, 1.0]),
            StandardView::Front => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Back => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Right => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Left => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            StandardView::Iso => ([1.0, -1.0, 1.0], [0.0, 0.0, 1.0]),
        };
        let center = Point3::new(
            (bbox.0.x + bbox.1.x) / 2.0,
            (bbox.0.y + bbox.1.y) / 2.0,
            (bbox.0.z + bbox.1.z) / 2.0,
        );
        let ed = Vec3::new(eye_dir[0], eye_dir[1], eye_dir[2]);
        let extent = (bbox.1 - bbox.0).length().max(1e-6);
        let eye = center + ed * (1.0 / ed.length()) * (extent * 1.8);
        return from_points(eye, center, Vec3::new(up[0], up[1], up[2]), false, 35.0);
    }
    if let Some(resolved) = scene
        && let Some(cam) = resolved.camera
    {
        return from_points(
            cam.eye,
            cam.target,
            cam.up,
            matches!(cam.projection, kernel::CameraProjection::Perspective),
            cam.fov_deg,
        );
    }
    match doc.camera_state() {
        Some(state) => from_points(
            state.eye,
            state.target,
            state.up,
            matches!(state.projection, kernel::CameraProjection::Perspective),
            state.fov_deg,
        ),
        None => {
            let center = Point3::new(
                (bbox.0.x + bbox.1.x) / 2.0,
                (bbox.0.y + bbox.1.y) / 2.0,
                (bbox.0.z + bbox.1.z) / 2.0,
            );
            let ed = Vec3::new(1.0, -1.0, 1.0);
            let extent = (bbox.1 - bbox.0).length().max(1e-6);
            let eye = center + ed * (1.0 / ed.length()) * (extent * 1.8);
            from_points(eye, center, Vec3::new(0.0, 0.0, 1.0), false, 35.0)
        }
    }
}

/// Scene resolution shared with `render_snapshot`: the Scene's hidden sets
/// (when it captures them) else the document's own.
fn scene_and_hidden(
    doc: &Document,
    scene: Option<u64>,
) -> Result<(Option<kernel::ResolvedScene>, softrender::HiddenLeaves), Refusal> {
    let scene = match scene {
        Some(sid) => Some(
            doc.resolve_scene(sid)
                .map_err(|e| Refusal::from_document_error(&e))?,
        ),
        None => None,
    };
    let hidden = match &scene {
        Some(resolved) => match (&resolved.hidden_object_ids, &resolved.hidden_instance_ids) {
            (Some(objects), Some(instances)) => {
                softrender::HiddenLeaves::from_lists(objects, instances)
            }
            _ => softrender::HiddenLeaves::of_document(doc),
        },
        None => softrender::HiddenLeaves::of_document(doc),
    };
    Ok((scene, hidden))
}

fn hlr_section(doc: &Document, scene: Option<&kernel::ResolvedScene>) -> Option<hlr::Section> {
    // A Scene's captured section plane (Some(None) = captured as "no
    // plane") wins; else the document's live one.
    let plane = match scene.and_then(|s| s.section) {
        Some(captured) => captured,
        None => doc.section_plane(),
    };
    match plane {
        Some(p) if p.active => Some(hlr::Section {
            origin: p.origin,
            normal: p.normal,
        }),
        _ => None,
    }
}

fn hlr_error(e: hlr::HlrError) -> Refusal {
    match e {
        hlr::HlrError::TooComplex { pairs, budget } => Refusal::api("too_complex", &format!("This model is too complex for a vector line drawing ({pairs} candidate/occluder pairs, budget {budget}). Use a shaded print instead."))
            .with_detail(serde_json::json!({ "pairs": pairs, "budget": budget })),
        hlr::HlrError::DegenerateCamera => Refusal::api("nothing_to_render", "The camera is degenerate (eye on target)."),
    }
}

fn kind_name(k: hlr::Kind) -> &'static str {
    match k {
        hlr::Kind::Hard => "hard",
        hlr::Kind::Silhouette => "silhouette",
        hlr::Kind::Soft => "soft",
        hlr::Kind::Section => "section",
        hlr::Kind::Hidden => "hidden",
    }
}

pub fn line_drawing(
    doc: &Document,
    params: &LineDrawingParams,
) -> Result<LineDrawingResult, Refusal> {
    let (scene, hidden) = scene_and_hidden(doc, params.scene)?;
    let items = softrender::document_items_hiding(doc, &hidden);
    if items.is_empty() {
        return Err(Refusal::api(
            "nothing_to_render",
            "This document has no visible solids to draw. Check that something is unhidden and un-tagged-away.",
        ));
    }
    let bbox = softrender::document_bbox_hiding(doc, &hidden);
    let view = resolve_view(doc, &params.camera, params.view, scene.as_ref(), bbox)?;
    let camera = hlr::Camera {
        eye: view.eye,
        target: view.target,
        up: view.up,
        projection: if view.perspective {
            hlr::Projection::Perspective
        } else {
            hlr::Projection::Parallel
        },
    };
    let hlr_items: Vec<hlr::Item> = items
        .iter()
        .map(|it| hlr::Item {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();
    let opts = hlr::Options {
        section: hlr_section(doc, scene.as_ref()),
        include_hidden: params.include_hidden,
        include_soft: params.include_soft,
        budget: hlr::DEFAULT_BUDGET,
    };
    let d = hlr::line_drawing(&hlr_items, &camera, &opts).map_err(hlr_error)?;
    let by_sid: BTreeMap<u64, EntityRef> = doc.sids().map(|(e, s)| (s, e.clone())).collect();
    let ids: Vec<String> = d
        .segs
        .iter()
        .map(|s| {
            by_sid
                .get(&s.sid)
                .map(|e| api::ids::public_id(e, s.sid))
                .unwrap_or_else(|| format!("sid_{:x}", s.sid))
        })
        .collect();
    let bounds = d.bounds.map(|(mn, mx)| [mn[0], mn[1], mx[0], mx[1]]);
    let count = d.segs.len();
    match params.format {
        LineDrawingFormat::Svg => {
            let svg = hlr::svg::write(
                &d,
                &hlr::svg::SvgStyle {
                    ratio: params.scale,
                    ..Default::default()
                },
            );
            Ok(LineDrawingResult {
                svg: Some(svg),
                segments: Vec::new(),
                kinds: Vec::new(),
                ids: Vec::new(),
                bounds,
                count,
            })
        }
        LineDrawingFormat::Segments => Ok(LineDrawingResult {
            svg: None,
            segments: d
                .segs
                .iter()
                .map(|s| [s.a[0], s.a[1], s.b[0], s.b[1]])
                .collect(),
            kinds: d.segs.iter().map(|s| kind_name(s.kind)).collect(),
            ids,
            bounds,
            count,
        }),
    }
}

/// Project the world bbox onto the view plane about `center`; returns the
/// rect (metres, y up) and the depth range along `dir`.
fn projected_extent(
    bbox: (Point3, Point3),
    center: Point3,
    dir: Vec3,
    right: Vec3,
    upv: Vec3,
) -> (RectM, (f64, f64)) {
    let (mut minx, mut maxx, mut miny, mut maxy, mut mind, mut maxd) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for i in 0..8 {
        let p = Point3::new(
            if i & 1 == 1 { bbox.1.x } else { bbox.0.x },
            if i & 2 == 2 { bbox.1.y } else { bbox.0.y },
            if i & 4 == 4 { bbox.1.z } else { bbox.0.z },
        );
        let rel = p - center;
        let x = dot(rel, right);
        let y = dot(rel, upv);
        let d = dot(rel, dir);
        minx = minx.min(x);
        maxx = maxx.max(x);
        miny = miny.min(y);
        maxy = maxy.max(y);
        mind = mind.min(d);
        maxd = maxd.max(d);
    }
    (
        RectM {
            x: minx,
            y: miny,
            w: (maxx - minx).max(1e-6),
            h: (maxy - miny).max(1e-6),
        },
        (mind, maxd),
    )
}

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
