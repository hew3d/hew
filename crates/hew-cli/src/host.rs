//! `CliHost`: the real, file-I/O-backed implementation of `api::Host`
//! (docs/HEW_API.md §12) — headless `hew-cli` embeds `crates/api` and the
//! kernel directly, so this is the only place in the binary that touches a
//! filesystem or an importer crate. Kernel-class purity (DEVELOPMENT.md
//! rule 1) stops at `crates/api`'s door; this crate is the host on the
//! other side of it.

use api::{
    Host, Refusal, SnapshotCamera, SnapshotParams, SnapshotProjection, SnapshotResult, StandardView,
};
use kernel::{Document, EntityRef, Point3};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Headless host: real file I/O, plus `hew.view.snapshot` rendered through
/// `crates/softrender` — the pure software rasterizer
/// (docs/design/headless-snapshot.md). No GPU, no viewport: bytes in
/// (tessellated meshes), bytes out (a PNG).
#[derive(Debug, Default)]
pub struct CliHost {
    /// The path the working document was last opened from or saved to —
    /// `hew.doc.save` with no `path` re-saves here (§7's "path required on
    /// first save" semantics).
    working_path: Option<PathBuf>,
}

impl CliHost {
    pub fn new() -> CliHost {
        CliHost::default()
    }

    /// The path a prior open/save recorded, if any. Exposed for the CLI's
    /// `run --out` step, which wants to know whether a bare `save` (no
    /// path) is even possible before trying.
    pub fn working_path(&self) -> Option<&Path> {
        self.working_path.as_deref()
    }
}

impl Host for CliHost {
    fn new_document(&mut self, doc: &mut Document) -> Result<(), Refusal> {
        *doc = Document::new();
        self.working_path = None;
        Ok(())
    }

    fn open_document(&mut self, doc: &mut Document, path: &str) -> Result<(), Refusal> {
        let bytes = std::fs::read(path)
            .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
        let loaded = Document::load(&bytes)
            .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
        *doc = loaded;
        self.working_path = Some(PathBuf::from(path));
        Ok(())
    }

    fn save_document(
        &mut self,
        doc: &Document,
        path: Option<&str>,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        let target = match path {
            Some(p) => {
                let target = PathBuf::from(p);
                self.working_path = Some(target.clone());
                target
            }
            None => self.working_path.clone().ok_or_else(|| {
                Refusal::api(
                    "path_required",
                    "This document has never been saved. Pass a path.",
                )
            })?,
        };
        let bytes = doc.save_for_persistence();
        std::fs::write(&target, bytes)
            .map_err(|e| Refusal::api("save_failed", &format!("{}: {e}", target.display())))?;
        // This host has a filesystem and used it, so there is nothing to
        // hand back — the file IS the result.
        Ok(None)
    }

    fn export_document(
        &mut self,
        doc: &Document,
        format: &str,
        path: Option<&str>,
        segments_per_turn: u32,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        // `true`: a headless dispatch has no dialog of its own to warn about
        // non-solid objects first, so a leaky object is dropped rather than
        // silently handed back in a file no human reviewed
        // (crates/mesh-export's module doc, "Non-solid inclusion").
        let bytes =
            mesh_export::export(doc, format, segments_per_turn, true).map_err(export_refusal)?;
        match path {
            Some(p) => {
                std::fs::write(p, &bytes)
                    .map_err(|e| Refusal::api("save_failed", &format!("{p}: {e}")))?;
                Ok(None)
            }
            None => Ok(Some(bytes)),
        }
    }

    fn import_document(
        &mut self,
        doc: &mut Document,
        path: &str,
        options: &serde_json::Value,
    ) -> Result<serde_json::Value, Refusal> {
        import_by_extension(doc, path, options)
    }

    fn snapshot(
        &mut self,
        doc: &Document,
        params: &SnapshotParams,
    ) -> Result<SnapshotResult, Refusal> {
        render_snapshot(doc, params)
    }

    fn write_snapshot(&mut self, path: &str, bytes: &[u8]) -> Result<(), Refusal> {
        std::fs::write(path, bytes)
            .map_err(|e| Refusal::api("save_failed", &format!("{path}: {e}")))
    }
}

/// Maps a [`mesh_export::ExportError`] to an [`api::Refusal`] through the
/// error's own name/message accessors, so the two can never drift apart —
/// `mesh-export` has no dependency on `crates/api` (DEVELOPMENT.md rule
/// 1), so this boundary-crossing step lives here instead.
fn export_refusal(e: mesh_export::ExportError) -> Refusal {
    Refusal::api(e.name(), &e.message())
}

// --------------------------------------------------------------- snapshot

/// Renders `doc` headlessly through `crates/softrender`
/// (docs/design/headless-snapshot.md): the shared
/// [`softrender::document_items`] traversal builds the scene (world objects
/// at identity pose; instance members at the instance's own pose, tagged
/// with the INSTANCE's stable id so pixels report the instance, not the
/// shared definition; user-hidden nodes skipped; an object that fails to
/// tessellate skipped, not fatal). If nothing renders at all, refuses
/// typed rather than returning a background-only image.
fn render_snapshot(doc: &Document, params: &SnapshotParams) -> Result<SnapshotResult, Refusal> {
    // `scene` renders through the Scene's OWN resolved hidden set
    // (docs/HEW_API.md's Scenes section) instead of the document's live
    // one — `crates/api/src/commands/scenes.rs::resolve_scene_id` already
    // validated the public id shape; an sid that no longer names a live
    // Scene answers the same `unknown_scene` refusal here, through
    // `Document::resolve_scene`.
    let scene = match params.scene {
        Some(sid) => Some(
            doc.resolve_scene(sid)
                .map_err(|e| Refusal::from_document_error(&e))?,
        ),
        None => None,
    };
    // A Scene that captures neither hidden property resolves to `None` leaf
    // sets — "leave the current hidden state alone" — so a camera-only
    // Scene renders with the DOCUMENT's own hidden state, exactly as
    // activating it in the app would show, never with everything visible.
    let hidden = match &scene {
        Some(resolved) => match (&resolved.hidden_object_ids, &resolved.hidden_instance_ids) {
            (Some(objects), Some(instances)) => {
                softrender::HiddenLeaves::from_lists(objects, instances)
            }
            _ => softrender::HiddenLeaves::of_document(doc),
        },
        None => softrender::HiddenLeaves::of_document(doc),
    };
    let items = softrender::document_items_hiding(doc, &hidden);

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

    let camera = resolve_camera(doc, params, scene.as_ref(), &hidden);
    let rendered = softrender::render(&render_items, &camera, params.width, params.height)
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

    Ok(SnapshotResult {
        png,
        width: rendered.width,
        height: rendered.height,
        id_buffer,
        id_palette,
    })
}

/// The camera resolution `docs/design/headless-snapshot.md` calls for,
/// extended with `scene` (docs/HEW_API.md's Scenes section — mutually
/// exclusive with `camera`/`view`, already enforced by
/// `crates/api/src/commands/doc.rs` before this is ever called): an
/// explicit `camera` param, a named `view` fitted to the scene bbox, a
/// Scene's own captured camera when `scene` was given and it captured
/// one, or — given none of those — the document's saved working camera,
/// falling back to a fitted isometric view. `hidden`/`scene`'s bbox
/// fitting always accounts for what is actually visible (the Scene's
/// resolved hidden set when one applies, the document's live one
/// otherwise) — `render_snapshot` already computed both, so this stays
/// infallible.
fn resolve_camera(
    doc: &Document,
    params: &SnapshotParams,
    scene: Option<&kernel::ResolvedScene>,
    hidden: &softrender::HiddenLeaves,
) -> softrender::Camera {
    if let Some(cam) = &params.camera {
        return build_camera(cam);
    }
    if let Some(view) = params.view {
        return softrender::Camera::standard_view(
            to_softrender_view(view),
            fitted_bbox(doc, hidden),
        );
    }
    if let Some(resolved) = scene
        && let Some(cam) = resolved.camera
    {
        return softrender::Camera::from_kernel(&cam);
    }
    match doc.camera_state() {
        Some(state) => softrender::Camera::from_kernel(&state),
        None => softrender::Camera::standard_view(
            softrender::StandardView::Iso,
            fitted_bbox(doc, hidden),
        ),
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
        SnapshotProjection::Parallel => {
            let distance = (target - eye).length();
            softrender::Projection::Parallel {
                half_height: distance * (fov_deg.to_radians() * 0.5).tan(),
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

/// The document's bounding box under `hidden`, world space, poses applied
/// — every visible object's vertices plus every instance member's
/// vertices transformed by its instance pose, skipping whatever `hidden`
/// says isn't visible (the document's own live hidden set, or a Scene's
/// resolved one — `render_snapshot` computes the right one and passes it
/// through, so a Scene's fitted standard view frames what the SCENE
/// shows, not the whole document). Falls back to a unit box centered on
/// the origin so a standard-view camera always has something to fit even
/// in the degenerate case (unreachable in practice: `render_snapshot`
/// already refuses `nothing_to_render` before this is called with
/// nothing visible).
fn fitted_bbox(doc: &Document, hidden: &softrender::HiddenLeaves) -> (Point3, Point3) {
    softrender::document_bbox_hiding(doc, hidden)
}

// -------------------------------------------------------- import unit scale

/// meters-per-source-unit, keyed by the `units` param (docs/HEW_API.md §7:
/// STL carries no units of its own). The STL/3MF/glTF/USDZ *writers*
/// `export_document` calls above live in `crates/mesh-export` now, shared
/// with `crates/wasm-api`'s `LiveHost`; this scale table is import-only
/// and stays here.
fn unit_scale(units: &str) -> Option<f64> {
    match units {
        "m" => Some(1.0),
        "mm" => Some(0.001),
        "cm" => Some(0.01),
        "in" => Some(0.0254),
        _ => None,
    }
}

// ------------------------------------------------------------------ import

/// Merges `path` into `doc` by file extension, honest about the one thing
/// no importer can guess: STL units (docs/HEW_API.md §7's semantics note —
/// STL carries none, so `hew.doc.import` refuses `units_required` rather
/// than assume).
fn import_by_extension(
    doc: &mut Document,
    path: &str,
    options: &serde_json::Value,
) -> Result<serde_json::Value, Refusal> {
    let bytes =
        std::fs::read(path).map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    let (scene, textures_missing) = match ext.as_deref() {
        Some("stl") => {
            let units = options.get("units").and_then(|v| v.as_str());
            let scale = match units.map(unit_scale) {
                Some(Some(scale)) => scale,
                Some(None) => {
                    return Err(Refusal::api(
                        "units_required",
                        &format!(
                            "Unknown units \"{}\". Use one of m, mm, cm, in.",
                            units.unwrap_or("")
                        ),
                    ));
                }
                None => {
                    return Err(Refusal::api(
                        "units_required",
                        "STL carries no units of its own; pass units: \"m\" | \"mm\" | \"cm\" | \"in\".",
                    ));
                }
            };
            let name_hint = Path::new(path).file_stem().and_then(|s| s.to_str());
            let parsed = stl_import::import(&bytes, scale, name_hint)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.missing)
        }
        Some("glb") | Some("gltf") => {
            let parsed = gltf_import::import(&bytes)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.missing)
        }
        Some("dae") => {
            let images = dae_import::ImageMap::new();
            let parsed = dae_import::import(&bytes, &images)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.textures_missing)
        }
        Some("skp") => {
            let parsed = skp_import::import(&bytes)
                .map_err(|e| Refusal::api("load_failed", &format!("{path}: {e}")))?;
            (parsed.scene, parsed.textures_missing)
        }
        _ => {
            return Err(Refusal::api(
                "unsupported_format",
                &format!(
                    "Unrecognized import extension for \"{path}\" — expected .stl, .gltf/.glb, .dae, or .skp."
                ),
            ));
        }
    };

    let (report, _change) = doc
        .ingest(scene, textures_missing)
        .map_err(|e| api::Refusal::from_document_error(&e))?;
    Ok(report_to_json(&report))
}

fn report_to_json(report: &kernel::ImportReport) -> serde_json::Value {
    serde_json::json!({
        "objects_created": report.objects_created,
        "watertight": report.watertight,
        "leaky": report.leaky,
        "skipped": report.skipped.iter().map(|s| serde_json::json!({
            "name": s.name,
            "reason": s.reason,
        })).collect::<Vec<_>>(),
        "textures_missing": report.textures_missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_by_extension_refuses_units_required_for_stl_without_units() {
        // A minimal binary STL: 80-byte header + 0 triangles is legal for
        // the format but `StlError::Empty`s in this importer — use a real
        // one-triangle fixture instead so the units gate is what's tested.
        let stl = tetrahedron_stl_bytes();
        let dir = std::env::temp_dir().join(format!("hew-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tet.stl");
        std::fs::write(&path, &stl).unwrap();

        let mut doc = Document::new();
        let err = import_by_extension(&mut doc, path.to_str().unwrap(), &serde_json::json!({}))
            .unwrap_err();
        assert_eq!(err.name, "units_required");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A single-triangle (degenerate as a solid, but perfectly valid STL)
    /// binary fixture, built by hand at the byte level. Public so
    /// `crates/hew-cli/tests/cli.rs` can build a richer one (a tetrahedron)
    /// the same way for its own import test.
    pub(crate) fn tetrahedron_stl_bytes() -> Vec<u8> {
        let tris: &[[[f32; 3]; 3]] = &[
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        ];
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for tri in tris {
            for _ in 0..3 {
                out.extend_from_slice(&0f32.to_le_bytes());
            }
            for v in tri {
                for c in v {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }
}
