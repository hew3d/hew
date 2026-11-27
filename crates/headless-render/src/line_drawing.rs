//! `hew.view.line_drawing` through `crates/hlr`.

use crate::view::{hlr_error, hlr_section, kind_name, resolve_view, scene_and_hidden};
use api::{LineDrawingFormat, LineDrawingParams, LineDrawingResult, Refusal};
use kernel::{Document, EntityRef};
use std::collections::BTreeMap;

pub fn line_drawing(
    doc: &Document,
    params: &LineDrawingParams,
) -> Result<LineDrawingResult, Refusal> {
    let (scene, hidden) = scene_and_hidden(doc, params.scene)?;
    let items = match &params.selection {
        Some(roots) => softrender::selection_items_hiding(doc, roots, &hidden)
            .map_err(|e| Refusal::from_document_error(&e))?,
        None => softrender::document_items_hiding(doc, &hidden),
    };
    if items.is_empty() {
        return Err(Refusal::api(
            "nothing_to_render",
            "This document has no visible solids to draw. Check that something is unhidden and un-tagged-away.",
        ));
    }
    let bbox = match &params.selection {
        Some(_) => softrender::items_bbox(&items)
            .unwrap_or_else(|| softrender::document_bbox_hiding(doc, &hidden)),
        None => softrender::document_bbox_hiding(doc, &hidden),
    };
    // A selection never takes the saved working camera (it frames the whole
    // model, not the part): without a camera or view it gets the fitted
    // isometric, as a snapshot does.
    let standard = match (&params.selection, &params.camera, params.view) {
        (Some(_), None, None) => Some(api::StandardView::Iso),
        _ => params.view,
    };
    let view = resolve_view(doc, &params.camera, standard, scene.as_ref(), bbox)?;
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
