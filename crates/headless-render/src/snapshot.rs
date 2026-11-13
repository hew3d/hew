//! `hew.view.snapshot`: the software-rasterized picture of a document, or
//! of a selection of it (docs/design/headless-snapshot.md).

use api::{
    Refusal, SnapshotCamera, SnapshotParams, SnapshotProjection, SnapshotResult, StandardView,
};
use kernel::{Document, EntityRef, Point3};
use std::collections::BTreeMap;

use crate::view::scene_and_hidden;

/// Renders `doc` through [`softrender`]: the shared
/// [`softrender::document_items_hiding`] traversal builds the scene (world
/// objects at identity pose; instance members at the instance's own pose,
/// tagged with the INSTANCE's stable id so pixels report the instance, not
/// the shared definition; user-hidden nodes skipped; an object that fails to
/// tessellate skipped, not fatal). With `selection`, only those roots'
/// leaves render ([`softrender::selection_items_hiding`]; a definition root
/// at the identity pose, tagged with the definition's id) and a camera the
/// caller did not give is fitted to them. If nothing renders at all, refuses
/// typed rather than returning a background-only image.
pub fn snapshot(doc: &Document, params: &SnapshotParams) -> Result<SnapshotResult, Refusal> {
    // `scene` renders through the Scene's OWN resolved hidden set (the
    // Scenes section of HEW_API.md) instead of the document's live one;
    // a Scene that captures neither hidden property renders with the
    // DOCUMENT's own hidden state, exactly as activating it in the app
    // would show.
    let (scene, hidden) = scene_and_hidden(doc, params.scene)?;
    let items = match &params.selection {
        Some(roots) => softrender::selection_items_hiding(doc, roots, &hidden)
            .map_err(|e| Refusal::from_document_error(&e))?,
        None => softrender::document_items_hiding(doc, &hidden),
    };

    if items.is_empty() {
        return Err(Refusal::api(
            "nothing_to_render",
            "This document has no visible solids to render. Check that something is unhidden and un-tagged-away.",
        ));
    }

    let render_items: Vec<softrender::RenderItem> = items
        .iter()
        .map(|it| softrender::RenderItem {
            mesh: &it.mesh,
            pose: it.pose,
            sid: it.sid,
        })
        .collect();

    let camera = resolve_camera(doc, params, scene.as_ref(), &hidden, &items);
    let rendered = softrender::render_with_background(
        &render_items,
        &camera,
        params.width,
        params.height,
        params.background,
    )
    .map_err(|e| {
        Refusal::api("too_many_objects", &e.to_string())
            .with_detail(serde_json::json!({ "max_items": u16::MAX }))
    })?;
    let png = softrender::png::encode(&rendered.rgba, rendered.width, rendered.height);

    let (id_buffer, id_palette) = if params.include_ids {
        let by_sid: BTreeMap<u64, EntityRef> = doc.sids().map(|(e, s)| (s, e.clone())).collect();
        let mut buffer = Vec::with_capacity(rendered.ids.len() * 2);
        for id in &rendered.ids {
            buffer.extend_from_slice(&id.to_le_bytes());
        }
        let palette = rendered
            .id_palette_sids
            .iter()
            .map(|sid| match by_sid.get(sid) {
                Some(entity) => api::ids::public_id(entity, *sid),
                // A sid whose entity has since been retired (a tombstone)
                // still needs a stable-shaped palette entry — this never
                // happens for a sid this render just used, but stay
                // typed rather than panic on a future edge case.
                None => format!("sid_{sid:x}"),
            })
            .collect();
        (Some(buffer), palette)
    } else {
        (None, Vec::new())
    };

    let depth = if params.include_depth {
        let d = rendered.depth_image(params.depth_bits);
        Some(api::DepthPass {
            png: d.png,
            bits: params.depth_bits,
            near: d.near,
            far: d.far,
        })
    } else {
        None
    };
    let normals_png = params.include_normals.then(|| rendered.normals_png());

    Ok(SnapshotResult {
        png,
        width: rendered.width,
        height: rendered.height,
        id_buffer,
        id_palette,
        depth,
        normals_png,
    })
}

/// The camera resolution `docs/design/headless-snapshot.md` calls for,
/// extended with `scene` (mutually exclusive with `camera`/`view`, already
/// enforced by `crates/api/src/commands/doc.rs`) and `selection`: an
/// explicit `camera` param, a named `view` fitted to what renders, a
/// Scene's own captured camera, or, given none of those and no selection,
/// the document's saved working camera. A selection never uses the saved
/// camera (it frames the whole model, not the part): with no camera or
/// view it gets a fitted isometric over the selection. The fit always
/// accounts for what is actually rendered: the items' posed bounds under a
/// selection, else the visible document under `hidden`.
fn resolve_camera(
    doc: &Document,
    params: &SnapshotParams,
    scene: Option<&kernel::ResolvedScene>,
    hidden: &softrender::HiddenLeaves,
    items: &[softrender::DocumentItem],
) -> softrender::Camera {
    let fitted = || match &params.selection {
        Some(_) => softrender::items_bbox(items)
            .unwrap_or_else(|| softrender::document_bbox_hiding(doc, hidden)),
        None => softrender::document_bbox_hiding(doc, hidden),
    };
    if let Some(cam) = &params.camera {
        return build_camera(cam);
    }
    if let Some(view) = params.view {
        return softrender::Camera::standard_view(to_softrender_view(view), fitted());
    }
    if params.selection.is_some() {
        return softrender::Camera::standard_view(softrender::StandardView::Iso, fitted());
    }
    if let Some(resolved) = scene
        && let Some(cam) = resolved.camera
    {
        return softrender::Camera::from_kernel(&cam);
    }
    match doc.camera_state() {
        Some(state) => softrender::Camera::from_kernel(&state),
        None => softrender::Camera::standard_view(softrender::StandardView::Iso, fitted()),
    }
}

/// An explicit `camera` param resolved to a `softrender::Camera`
/// (docs/design/headless-snapshot.md): `up` defaults to +Z, `projection`
/// to perspective, `fov_deg` to 35°; under `parallel`, the half-height is
/// derived from the eye-target distance and that same fov, mirroring
/// `softrender::Camera::from_kernel`'s own parallel derivation so a
/// parallel snapshot frames the same way a parallel document camera would.
fn build_camera(cam: &SnapshotCamera) -> softrender::Camera {
    let eye = Point3::new(cam.eye[0], cam.eye[1], cam.eye[2]);
    let target = Point3::new(cam.target[0], cam.target[1], cam.target[2]);
    let up = cam.up.unwrap_or([0.0, 0.0, 1.0]);
    let fov_deg = cam.fov_deg.unwrap_or(35.0);
    let projection = match cam.projection.unwrap_or(SnapshotProjection::Perspective) {
        SnapshotProjection::Perspective => {
            softrender::Projection::Perspective { fov_y_deg: fov_deg }
        }
        // The height the view spans at the target when the camera says,
        // else what the perspective view spans there.
        SnapshotProjection::Parallel => {
            let distance = (target - eye).length();
            let height = cam
                .parallel_height
                .filter(|h| h.is_finite() && *h > 0.0)
                .unwrap_or_else(|| 2.0 * distance * (fov_deg.to_radians() * 0.5).tan());
            softrender::Projection::Parallel {
                half_height: height * 0.5,
            }
        }
    };
    softrender::Camera {
        eye,
        target,
        up: kernel::Vec3::new(up[0], up[1], up[2]),
        projection,
    }
}

fn to_softrender_view(view: StandardView) -> softrender::StandardView {
    match view {
        StandardView::Iso => softrender::StandardView::Iso,
        StandardView::Front => softrender::StandardView::Front,
        StandardView::Back => softrender::StandardView::Back,
        StandardView::Left => softrender::StandardView::Left,
        StandardView::Right => softrender::StandardView::Right,
        StandardView::Top => softrender::StandardView::Top,
        StandardView::Bottom => softrender::StandardView::Bottom,
    }
}
